//! The Kafka ingest serve loop: accept TCP, read length-framed requests, dispatch `ApiVersions` / `Metadata` /
//! `Produce`, write length-framed responses, and emit each produced batch's record VALUES over an `mpsc` channel
//! (the integration layer seals them and lands them via a datarail `Sink`). Dependency-free: this crate speaks
//! only the wire protocol; it never sees datarail keys or cofres.

use std::collections::HashMap;
use std::io::{self, Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use crate::codec::{Reader, Writer};
use crate::handlers::{
    api_versions_response, metadata_response, parse_metadata_topics, API_METADATA, API_PRODUCE, API_VERSIONS,
};
use crate::produce::{parse_produce, produce_response, EosCoord};

/// A produced batch handed to the integration layer: `(topic, partition, record values, EOS coord)`. The
/// `EosCoord` (when present) carries the idempotent producer's stable sequence range — the basis for exactly-once
/// ingest (see `KAFKA-EOS-DESIGN.md`); `None` ⇒ the batch is not EOS-eligible (→ at-least-once).
pub type ProducedBatch = (String, i32, Vec<Vec<u8>>, Option<EosCoord>);

/// Max bytes in a single framed request — a hostile peer cannot make us allocate a giant buffer.
const MAX_FRAME: usize = 16 * 1024 * 1024;
/// Max concurrent connections — bounds thread/FD/memory under a connection flood (audit K2).
const MAX_CONNECTIONS: usize = 256;
/// Per-socket read/write deadline — a stalled/slowloris peer errors out instead of pinning a thread (audit K2).
const IO_TIMEOUT: Duration = Duration::from_secs(30);
/// Max distinct `(topic, partition)` offset entries retained — bounds the global map so a hostile producer
/// cannot mint unbounded permanent keys (audit K1). Beyond this, a new key simply restarts at offset 0.
const MAX_OFFSET_KEYS: usize = 100_000;

struct Shared {
    offsets: Mutex<HashMap<(String, i32), i64>>,
    tx: Sender<ProducedBatch>,
    host: String,
    port: i32,
}

/// Serve Kafka ingest on `listener`, advertising this broker as `advertised_host:advertised_port` (the address a
/// client must be able to reach us at). Produced record values are sent on `tx`. Blocks, one thread per connection.
///
/// # Errors
/// [`io::Error`] if accepting a connection fails.
pub fn serve(
    listener: &TcpListener,
    advertised_host: &str,
    advertised_port: i32,
    tx: Sender<ProducedBatch>,
) -> io::Result<()> {
    let shared = Arc::new(Shared {
        offsets: Mutex::new(HashMap::new()),
        tx,
        host: advertised_host.to_owned(),
        port: advertised_port,
    });
    let active = Arc::new(AtomicUsize::new(0));
    for stream in listener.incoming() {
        let stream = stream?;
        // Bound concurrency: refuse (and immediately drop) connections past the cap rather than spawn unboundedly.
        if active.load(Ordering::Relaxed) >= MAX_CONNECTIONS {
            drop(stream);
            continue;
        }
        // A stalled peer must not pin a thread forever — fail its blocking reads/writes after a deadline.
        let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
        let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
        active.fetch_add(1, Ordering::Relaxed);
        let shared = Arc::clone(&shared);
        let active = Arc::clone(&active);
        std::thread::spawn(move || {
            let _ = handle_connection(stream, &shared);
            active.fetch_sub(1, Ordering::Relaxed);
        });
    }
    Ok(())
}

/// Whether the REQUEST header for `(api_key, version)` is flexible (KIP-482 tagged fields present).
fn request_is_flexible(api_key: i16, version: i16) -> bool {
    match api_key {
        API_VERSIONS => version >= 3,
        API_METADATA | API_PRODUCE => version >= 9,
        _ => false,
    }
}

/// Read one length-framed request (`INT32` length prefix + payload), or `None` at a clean EOF.
fn read_frame(stream: &mut TcpStream) -> io::Result<Option<Vec<u8>>> {
    let mut len_buf = [0u8; 4];
    match stream.read_exact(&mut len_buf) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = i32::from_be_bytes(len_buf);
    let n = usize::try_from(len).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "negative frame length"))?;
    if n > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too large"));
    }
    let mut buf = vec![0u8; n];
    stream.read_exact(&mut buf)?;
    Ok(Some(buf))
}

fn handle_connection(mut stream: TcpStream, shared: &Shared) -> io::Result<()> {
    while let Some(frame) = read_frame(&mut stream)? {
        let mut reader = Reader::new(&frame);
        let api_key = reader.int16()?;
        let api_version = reader.int16()?;
        let correlation_id = reader.int32()?;
        let _client_id = reader.nullable_string()?;
        if request_is_flexible(api_key, api_version) {
            reader.skip_tagged_fields()?;
        }

        let response = match api_key {
            API_VERSIONS => api_versions_response(api_version, correlation_id),
            API_METADATA => {
                let topics = parse_metadata_topics(&mut reader)?;
                metadata_response(api_version, correlation_id, &shared.host, shared.port, &topics)
            }
            API_PRODUCE => {
                let topics = parse_produce(&mut reader, api_version)?;
                let response = {
                    // Recover from a poisoned lock (the protected state is plain data) so one panicked
                    // connection can't brick produce for the broker's lifetime (audit K5).
                    let mut offsets = shared.offsets.lock().unwrap_or_else(PoisonError::into_inner);
                    produce_response(api_version, correlation_id, &topics, &mut |name, part, count| {
                        let key = (name.to_owned(), part);
                        let base = *offsets.get(&key).unwrap_or(&0);
                        // Bound the map: past the cap, don't retain new keys (they restart at 0) — no unbounded
                        // growth from attacker-chosen (topic, partition) identities (audit K1). saturating add (K6).
                        if offsets.contains_key(&key) || offsets.len() < MAX_OFFSET_KEYS {
                            offsets.insert(key, base.saturating_add(i64::try_from(count).unwrap_or(i64::MAX)));
                        }
                        base
                    })
                };
                for t in &topics {
                    for p in &t.partitions {
                        if !p.values.is_empty() {
                            let _ = shared.tx.send((t.name.clone(), p.partition, p.values.clone(), p.eos));
                        }
                    }
                }
                response
            }
            other => {
                return Err(io::Error::other(format!("unsupported Kafka api_key {other}")));
            }
        };

        stream.write_all(&Writer::frame(&response))?;
        stream.flush()?;
    }
    Ok(())
}
