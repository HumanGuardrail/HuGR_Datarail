//! The Kafka ingest serve loop: accept TCP, read length-framed requests, dispatch `ApiVersions` / `Metadata` /
//! `Produce`, write length-framed responses, and emit each produced batch's record VALUES over an `mpsc` channel
//! (the integration layer seals them and lands them via a datarail `Sink`). Dependency-free: this crate speaks
//! only the wire protocol; it never sees datarail keys or cofres.

use std::collections::HashMap;
use std::io::{self, Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use crate::codec::{Reader, Writer};
use crate::consume::{
    fetch_response, list_offsets_response, parse_fetch, parse_list_offsets, FetchPartitionResult,
    FetchTopicResult, ListOffsetResult, ListOffsetTopicResult, API_FETCH, API_LIST_OFFSETS,
};
use crate::groups::{
    find_coordinator_response, offset_commit_response, offset_fetch_response, parse_find_coordinator,
    parse_offset_commit, parse_offset_fetch, OffsetCommitPartitionResult, OffsetCommitTopicResult,
    OffsetFetchPartitionResult, OffsetFetchTopicResult, API_FIND_COORDINATOR, API_OFFSET_COMMIT, API_OFFSET_FETCH,
};
use crate::handlers::{
    api_versions_response, init_producer_id_response, metadata_response, parse_metadata_topics,
    API_INIT_PRODUCER_ID, API_METADATA, API_PRODUCE, API_VERSIONS,
};
use crate::produce::{build_record_batch, parse_produce, produce_response, EosCoord};

/// A produced batch handed to the integration layer for **durable** landing. The producer is **not acked until
/// `done` reports the landing result** (ack-after-durable — audit A: a broker that acks before the record is
/// durable loses acked records on a crash, and an idempotent producer never resends an acked batch). `done`
/// carries `Ok(())` on a durable land or `Err(msg)` on a sink failure (the producer then gets a retriable error
/// code, never a false ack). The `EosCoord` (when present) carries the idempotent producer's stable sequence
/// range — the basis for exactly-once ingest (`KAFKA-EOS-DESIGN.md`); `None` ⇒ at-least-once.
pub struct ProducedBatch {
    /// Topic the batch was produced to.
    pub topic: String,
    /// Partition index.
    pub partition: i32,
    /// Record value payloads, in order.
    pub values: Vec<Vec<u8>>,
    /// The idempotent-producer EOS coordinate, if present.
    pub eos: Option<EosCoord>,
    /// The integration layer signals the durable-landing result here; the serve loop acks only after it arrives.
    pub done: Sender<Result<(), String>>,
}

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
    /// Monotonic source of `producer_id`s handed out by `InitProducerId` — each idempotent producer gets a
    /// distinct id, so distinct producers form distinct exactly-once substreams in the sink.
    next_producer_id: AtomicI64,
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
        next_producer_id: AtomicI64::new(1),
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
                // ACK-AFTER-DURABLE (audit A): land every partition batch through the integration layer and wait
                // for its durable-landing result BEFORE building the ack. A landing failure becomes a retriable
                // error code for that partition — never a false NONE ack that would lose the records on a crash.
                let mut codes: HashMap<(String, i32), i16> = HashMap::new();
                for t in &topics {
                    for p in &t.partitions {
                        if p.values.is_empty() {
                            continue; // nothing to land → NONE
                        }
                        let (done, done_rx) = std::sync::mpsc::channel();
                        let sent = shared.tx.send(ProducedBatch {
                            topic: t.name.clone(),
                            partition: p.partition,
                            values: p.values.clone(),
                            eos: p.eos,
                            done,
                        });
                        // 56 = KAFKA_STORAGE_ERROR (retriable): the integration layer is gone or the land failed,
                        // so the producer retries rather than treating the records as durably stored.
                        let code = match sent {
                            Err(_) => 56,
                            Ok(()) => match done_rx.recv() {
                                Ok(Ok(())) => 0,
                                Ok(Err(_)) | Err(_) => 56,
                            },
                        };
                        codes.insert((t.name.clone(), p.partition), code);
                    }
                }
                // Recover from a poisoned lock (the protected state is plain data) so one panicked
                // connection can't brick produce for the broker's lifetime (audit K5).
                let mut offsets = shared.offsets.lock().unwrap_or_else(PoisonError::into_inner);
                produce_response(api_version, correlation_id, &topics, &mut |name, part, count| {
                    let key = (name.to_owned(), part);
                    let base = *offsets.get(&key).unwrap_or(&0);
                    let code = codes.get(&key).copied().unwrap_or(0); // read before the insert moves `key`
                    // Bound the map: past the cap, don't retain new keys (they restart at 0) — no unbounded
                    // growth from attacker-chosen (topic, partition) identities (audit K1). saturating add (K6).
                    if offsets.contains_key(&key) || offsets.len() < MAX_OFFSET_KEYS {
                        offsets.insert(key, base.saturating_add(i64::try_from(count).unwrap_or(i64::MAX)));
                    }
                    (base, code)
                })
            }
            API_INIT_PRODUCER_ID => {
                // Hand out a fresh producer_id so the client can enable idempotence; the request body
                // (transactional_id / timeout) needs no parsing — each producer just needs a distinct id.
                let pid = shared.next_producer_id.fetch_add(1, Ordering::Relaxed);
                init_producer_id_response(correlation_id, pid)
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

/// The CONSUME-side broker backend (see `KAFKA-FETCH-DESIGN.md`). The `kafka-broker` mode wires this to a sealed
/// store: `produce` seals + stores the cofre, `fetch` un-seals at the edge. Shared across connection threads.
pub trait KafkaBroker: Send + Sync {
    /// Seal + store a produced batch for `(topic, partition)`; return the logical base offset assigned.
    ///
    /// # Errors
    /// Propagates a seal/store error (the producer then gets a retriable code, never a false ack).
    fn produce(&self, topic: &str, partition: i32, records: &[Vec<u8>]) -> io::Result<i64>;

    /// Un-seal + return the plaintext records for `(topic, partition)` from `offset`, bounded by `max_bytes`.
    ///
    /// # Errors
    /// Propagates a read/open error.
    fn fetch(&self, topic: &str, partition: i32, offset: i64, max_bytes: i32) -> io::Result<Vec<Vec<u8>>>;

    /// The logical `(earliest, latest)` offsets for `(topic, partition)` (latest = the next offset to be written).
    fn bounds(&self, topic: &str, partition: i32) -> (i64, i64);

    /// Durably commit a consumer group's offset for `(topic, partition)` (`OffsetCommit`). Default: no-op — a
    /// broker that does not persist consumer offsets (acked as NONE; the consumer simply gains no durability).
    ///
    /// # Errors
    /// Propagates a durable-store error (the consumer then gets a retriable code, never a false success).
    fn commit_offset(&self, _group: &str, _topic: &str, _partition: i32, _offset: i64) -> io::Result<()> {
        Ok(())
    }

    /// The last durably-committed offset for `(group, topic, partition)`, or `None` if none (`OffsetFetch`).
    /// Default: `None`.
    ///
    /// # Errors
    /// Propagates a durable-store read error.
    fn fetch_offset(&self, _group: &str, _topic: &str, _partition: i32) -> io::Result<Option<i64>> {
        Ok(None)
    }
}

/// Serve the bidirectional `datarail kafka-broker` on `listener`: `Produce` (seal+store), `Fetch` (un-seal+return),
/// `ListOffsets`, plus `Metadata`/`ApiVersions`/`InitProducerId`. Blocks; one bounded thread per connection (like
/// [`serve`]). The storage holds only sealed cofres — un-sealing happens here, at the serving edge.
///
/// # Errors
/// [`io::Error`] if accepting a connection fails.
pub fn serve_broker<B: KafkaBroker + 'static>(
    listener: &TcpListener,
    advertised_host: &str,
    advertised_port: i32,
    broker: &Arc<B>,
) -> io::Result<()> {
    let host = advertised_host.to_owned();
    let next_producer_id = Arc::new(AtomicI64::new(1));
    let active = Arc::new(AtomicUsize::new(0));
    for stream in listener.incoming() {
        let stream = stream?;
        if active.load(Ordering::Relaxed) >= MAX_CONNECTIONS {
            drop(stream);
            continue;
        }
        let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
        let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
        active.fetch_add(1, Ordering::Relaxed);
        let broker = Arc::clone(broker);
        let host = host.clone();
        let pid = Arc::clone(&next_producer_id);
        let active = Arc::clone(&active);
        std::thread::spawn(move || {
            let _ = handle_broker_connection(stream, broker.as_ref(), &host, advertised_port, &pid);
            active.fetch_sub(1, Ordering::Relaxed);
        });
    }
    Ok(())
}

fn handle_broker_connection<B: KafkaBroker>(
    mut stream: TcpStream,
    broker: &B,
    host: &str,
    port: i32,
    next_producer_id: &AtomicI64,
) -> io::Result<()> {
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
                metadata_response(api_version, correlation_id, host, port, &topics)
            }
            API_INIT_PRODUCER_ID => {
                let pid = next_producer_id.fetch_add(1, Ordering::Relaxed);
                init_producer_id_response(correlation_id, pid)
            }
            API_PRODUCE => {
                let topics = parse_produce(&mut reader, api_version)?;
                // Seal + store each partition batch BEFORE acking; the response carries the base offset + code.
                let mut results: HashMap<(String, i32), (i64, i16)> = HashMap::new();
                for t in &topics {
                    for p in &t.partitions {
                        let outcome = match broker.produce(&t.name, p.partition, &p.values) {
                            Ok(base) => (base, 0i16),
                            Err(_) => (-1, 56), // KAFKA_STORAGE_ERROR (retriable) — never a false ack
                        };
                        results.insert((t.name.clone(), p.partition), outcome);
                    }
                }
                produce_response(api_version, correlation_id, &topics, &mut |name, part, _count| {
                    results.get(&(name.to_owned(), part)).copied().unwrap_or((0, 0))
                })
            }
            API_FETCH => {
                let topics = parse_fetch(&mut reader, api_version)?;
                let out = fetch_results(broker, &topics);
                fetch_response(api_version, correlation_id, &out)
            }
            API_LIST_OFFSETS => {
                let topics = parse_list_offsets(&mut reader, api_version)?;
                let mut out = Vec::with_capacity(topics.len());
                for t in &topics {
                    let mut parts = Vec::with_capacity(t.partitions.len());
                    for p in &t.partitions {
                        let (earliest, latest) = broker.bounds(&t.name, p.partition);
                        let offset = if p.timestamp == -2 { earliest } else { latest };
                        parts.push(ListOffsetResult { partition: p.partition, offset });
                    }
                    out.push(ListOffsetTopicResult { name: t.name.clone(), partitions: parts });
                }
                list_offsets_response(api_version, correlation_id, &out)
            }
            API_FIND_COORDINATOR => {
                let _group = parse_find_coordinator(&mut reader, api_version)?;
                // Single-node: THIS broker is the coordinator (node 0, the advertised host/port).
                find_coordinator_response(api_version, correlation_id, 0, host, port)
            }
            API_OFFSET_COMMIT => {
                let req = parse_offset_commit(&mut reader, api_version)?;
                let out = offset_commit_results(broker, &req);
                offset_commit_response(correlation_id, &out)
            }
            API_OFFSET_FETCH => {
                let req = parse_offset_fetch(&mut reader, api_version)?;
                let out = offset_fetch_results(broker, &req);
                offset_fetch_response(api_version, correlation_id, &out)
            }
            other => return Err(io::Error::other(format!("unsupported Kafka api_key {other}"))),
        };
        stream.write_all(&Writer::frame(&response))?;
        stream.flush()?;
    }
    Ok(())
}

/// Resolve a parsed `Fetch` request against the broker: un-seal each partition's records at the edge into a v2
/// `RecordBatch` (or empty), carrying the high-watermark; a read/open error maps to `OFFSET_OUT_OF_RANGE` (1).
fn fetch_results<B: KafkaBroker>(broker: &B, topics: &[crate::consume::FetchTopic]) -> Vec<FetchTopicResult> {
    let mut out = Vec::with_capacity(topics.len());
    for t in topics {
        let mut parts = Vec::with_capacity(t.partitions.len());
        for p in &t.partitions {
            let (_, latest) = broker.bounds(&t.name, p.partition);
            let (error_code, records) = match broker.fetch(&t.name, p.partition, p.fetch_offset, p.max_bytes) {
                Ok(values) if values.is_empty() => (0, Vec::new()),
                Ok(values) => (0, build_record_batch(p.fetch_offset, &values)),
                Err(_) => (1, Vec::new()), // 1 = OFFSET_OUT_OF_RANGE
            };
            parts.push(FetchPartitionResult { partition: p.partition, error_code, high_watermark: latest, records });
        }
        out.push(FetchTopicResult { name: t.name.clone(), partitions: parts });
    }
    out
}

/// Durably commit a parsed `OffsetCommit` request, per `(topic, partition)`, BEFORE the response is sent. Each
/// partition carries its OWN error: 0 = NONE on a durable commit, else a retriable code (16) so the consumer
/// re-commits THAT partition — a failure on one never masks another's success, and never a false NONE.
fn offset_commit_results<B: KafkaBroker>(
    broker: &B,
    req: &crate::groups::OffsetCommitRequest,
) -> Vec<OffsetCommitTopicResult> {
    let mut out = Vec::with_capacity(req.topics.len());
    for t in &req.topics {
        let mut parts = Vec::with_capacity(t.partitions.len());
        for p in &t.partitions {
            let error_code = match broker.commit_offset(&req.group_id, &t.name, p.partition, p.offset) {
                Ok(()) => 0,
                Err(_) => 16, // COORDINATOR_NOT_AVAILABLE-class: retriable, never a false success
            };
            parts.push(OffsetCommitPartitionResult { partition: p.partition, error_code });
        }
        out.push(OffsetCommitTopicResult { name: t.name.clone(), partitions: parts });
    }
    out
}

/// Resolve a parsed `OffsetFetch` request against the broker's durable offset store. `-1` ("no committed offset",
/// the Kafka sentinel) on absence OR a store read error — the consumer then falls back to `auto.offset.reset`
/// rather than resuming at a wrong position.
fn offset_fetch_results<B: KafkaBroker>(
    broker: &B,
    req: &crate::groups::OffsetFetchRequest,
) -> Vec<OffsetFetchTopicResult> {
    let mut out = Vec::with_capacity(req.topics.len());
    for t in &req.topics {
        let mut parts = Vec::with_capacity(t.partitions.len());
        for &partition in &t.partitions {
            let offset = broker.fetch_offset(&req.group_id, &t.name, partition).ok().flatten().unwrap_or(-1);
            parts.push(OffsetFetchPartitionResult { partition, offset });
        }
        out.push(OffsetFetchTopicResult { name: t.name.clone(), partitions: parts });
    }
    out
}
