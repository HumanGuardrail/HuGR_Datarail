//! The Kafka ingest serve loop: accept TCP, read length-framed requests, dispatch `ApiVersions` / `Metadata` /
//! `Produce`, write length-framed responses, and emit each produced batch's record VALUES over an `mpsc` channel
//! (the integration layer seals them and lands them via a datarail `Sink`). Dependency-free: this crate speaks
//! only the wire protocol; it never sees datarail keys or cofres.

use std::collections::HashMap;
use std::io::{self, Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use crate::codec::{Reader, Writer};
use crate::handlers::{
    api_versions_response, metadata_response, parse_metadata_topics, API_METADATA, API_PRODUCE, API_VERSIONS,
};
use crate::produce::{parse_produce, produce_response};

/// A produced batch handed to the integration layer: `(topic, partition, record values)`.
pub type ProducedBatch = (String, i32, Vec<Vec<u8>>);

const MAX_FRAME: usize = 100 * 1024 * 1024;

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
    for stream in listener.incoming() {
        let stream = stream?;
        let shared = Arc::clone(&shared);
        std::thread::spawn(move || {
            let _ = handle_connection(stream, &shared);
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
                    let mut offsets =
                        shared.offsets.lock().map_err(|_| io::Error::other("offset lock poisoned"))?;
                    produce_response(api_version, correlation_id, &topics, &mut |name, part, count| {
                        let key = (name.to_owned(), part);
                        let base = *offsets.get(&key).unwrap_or(&0);
                        offsets.insert(key, base + i64::try_from(count).unwrap_or(i64::MAX));
                        base
                    })
                };
                for t in &topics {
                    for p in &t.partitions {
                        if !p.values.is_empty() {
                            let _ = shared.tx.send((t.name.clone(), p.partition, p.values.clone()));
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
