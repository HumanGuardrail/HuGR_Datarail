//! `datarail-omb-shim` — a mini-broker that lets the **`OpenMessaging` Benchmark** (OMB) drive datarail over
//! its REAL sealed terminals. It listens on two TCP ports (ingress for producers, egress for consumers) and,
//! per topic, runs datarail's full sealed datapath: `board` a batch into a cofre via a real
//! [`datarail_terminal::SourceTerminal`], move it to a real [`datarail_terminal::DestTerminal`]
//! (encode→decode→verify→open→admit→commit — no crypto shortcut), then fan the offloaded records out to every
//! subscribed egress consumer.
//!
//! THIS IS A BENCHMARK ADAPTER, **NOT A PRODUCT COMPONENT**. It maps OMB's pub/sub onto datarail's
//! point-to-point send/recv (a fair-but-partial fit; datarail has no native topic/partition/consumer-group).
//! The fixed, deterministic source/dest seeds and keys below exist only because both terminals run in **this
//! same process** — a real deployment derives and pins these via the identity layer (SPEC-08), never hardcodes
//! them.
//!
//! The wire protocol, config, batching rules, and honesty labels are the frozen contract in
//! `docs/design/OMB-PROTOCOL.md`; this binary implements it verbatim.
//!
//! ## Topology (std-only threading — Charter *leveza*, zero external deps)
//! - One **ingress acceptor** thread and one **egress acceptor** thread.
//! - One thread per accepted connection (producer or consumer).
//! - One **topic worker** thread per topic, created lazily on first reference. It owns that topic's
//!   `SourceTerminal` + `DestTerminal`, drains an [`std::sync::mpsc`] queue of ingested messages, boards them
//!   in batches (flush at `batch_max_records` OR `batch_max_micros`, whichever first), offloads through the
//!   real seal path, and fans the delivered records out to every subscriber's channel.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::io::{BufReader, BufWriter, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::ExitCode;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use datarail_core::{AeadAlg, Cofre, Disposition, Substrate};
use datarail_crypto::{verifying_key, x25519_public};
use datarail_rail::TcpSubstrate;
use datarail_substrate_shmem::ShmemRing;
use datarail_terminal::{ContentContract, DestTerminal, SourceTerminal, TerminalConfig};

// ----------------------------------------------------------------------------------------------------------
// Fixed, deterministic route keys — SAME-PROCESS BENCH SHIM ONLY (NOT a product component).
//
// Source and dest terminals live in this one process, so a fixed route id / stream id / AEAD key material is
// fine: there is no third party to pin against. A real datarail deployment derives and pins these via the
// SPEC-08 identity layer; it MUST NOT hardcode them. They mirror `datarail-bench`'s `head_to_head` anchor.
// ----------------------------------------------------------------------------------------------------------

/// Fixed A→B route id for the shim's single in-process route.
const ROUTE_ID: [u8; 16] = [1u8; 16];
/// Fixed ordering domain for the shim's single in-process route.
const STREAM_ID: [u8; 16] = [2u8; 16];
/// Fixed Ed25519 source signing seed (bench shim — see module note).
const SRC_SEED: [u8; 32] = [11u8; 32];
/// Fixed destination admission-gate seed (bench shim — see module note).
const DEST_SEED: [u8; 32] = [22u8; 32];
/// Fixed destination X25519 secret; `board` seals each per-cofre data key to its public key (bench shim).
const DEST_X25519_SECRET: [u8; 32] = [9u8; 32];
/// Fixed per-tenant idempotency-MAC secret (bench shim — see module note).
const TENANT_SECRET: [u8; 32] = [6u8; 32];

// ----------------------------------------------------------------------------------------------------------
// Config — TOML, all fields optional, defaults per OMB-PROTOCOL.md. Hand-rolled `key = value` parse (no deps).
// ----------------------------------------------------------------------------------------------------------

/// Which real transport the cofre traverses between source and dest. Default [`Substrate::Loopback`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubstrateKind {
    /// In-process handoff: `board` → `offload` directly (still the full seal/open path). The frozen default.
    Loopback,
    /// A real `127.0.0.1` TCP kernel hop ([`datarail_rail::TcpSubstrate`]) between board and offload.
    Tcp,
    /// A real shared-memory ring hop ([`datarail_substrate_shmem::ShmemRing`]) between board and offload.
    Shmem,
}

/// The shim's parsed configuration (`docs/design/OMB-PROTOCOL.md` §"Shim config").
#[derive(Debug, Clone)]
struct Config {
    /// Address producers connect to. Default `127.0.0.1:7701`.
    ingress_addr: String,
    /// Address consumers connect to. Default `127.0.0.1:7702`.
    egress_addr: String,
    /// Transport between the source and dest terminals. Default [`SubstrateKind::Loopback`].
    substrate: SubstrateKind,
    /// Flush a cofre once a batch reaches this many records. Default `128`.
    batch_max_records: usize,
    /// Flush a partial batch once it has been filling for this long. Default `1000` µs.
    batch_max_micros: u64,
    /// Reject any single frame whose payload exceeds this many bytes (parse-safety). Default `1_048_576`.
    max_record_bytes: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            ingress_addr: "127.0.0.1:7701".to_owned(),
            egress_addr: "127.0.0.1:7702".to_owned(),
            substrate: SubstrateKind::Loopback,
            batch_max_records: 128,
            batch_max_micros: 1000,
            max_record_bytes: 1_048_576,
        }
    }
}

impl Config {
    /// Parse a config from a TOML file at `path`, layering over [`Config::default`]. Only the documented flat
    /// `key = value` keys are recognised; any unknown key is a hard error (typo safety). All values optional.
    fn from_path(path: &str) -> Result<Self, ShimError> {
        let text = std::fs::read_to_string(path).map_err(|e| ShimError::Config(format!("{path}: {e}")))?;
        Self::from_str(&text)
    }

    /// Parse a config from already-loaded TOML `text`, layering over [`Config::default`].
    fn from_str(text: &str) -> Result<Self, ShimError> {
        let mut cfg = Self::default();
        for raw in text.lines() {
            // Strip a `#` comment, then surrounding whitespace; skip blank / comment-only lines.
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| ShimError::Config(format!("not `key = value`: {raw:?}")))?;
            let key = key.trim();
            let value = unquote(value.trim());
            match key {
                "ingress_addr" => value.clone_into(&mut cfg.ingress_addr),
                "egress_addr" => value.clone_into(&mut cfg.egress_addr),
                "substrate" => cfg.substrate = parse_substrate(value)?,
                "batch_max_records" => cfg.batch_max_records = parse_usize(key, value)?,
                "batch_max_micros" => cfg.batch_max_micros = parse_u64(key, value)?,
                "max_record_bytes" => cfg.max_record_bytes = parse_usize(key, value)?,
                other => return Err(ShimError::Config(format!("unknown config key `{other}`"))),
            }
        }
        if cfg.batch_max_records == 0 {
            return Err(ShimError::Config("batch_max_records must be >= 1".to_owned()));
        }
        Ok(cfg)
    }
}

/// Strip one matching pair of surrounding single or double quotes from a TOML scalar, if present.
fn unquote(value: &str) -> &str {
    let bytes = value.as_bytes();
    if bytes.len() >= 2 {
        let (first, last) = (bytes[0], bytes[bytes.len() - 1]);
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return &value[1..value.len() - 1];
        }
    }
    value
}

/// Parse the `substrate` enum value.
fn parse_substrate(value: &str) -> Result<SubstrateKind, ShimError> {
    match value {
        "loopback" => Ok(SubstrateKind::Loopback),
        "tcp" => Ok(SubstrateKind::Tcp),
        "shmem" => Ok(SubstrateKind::Shmem),
        other => Err(ShimError::Config(format!(
            "substrate must be loopback|tcp|shmem, got `{other}`"
        ))),
    }
}

/// Parse a `usize` config scalar with a key-qualified error.
fn parse_usize(key: &str, value: &str) -> Result<usize, ShimError> {
    value
        .parse()
        .map_err(|_| ShimError::Config(format!("`{key}` is not a non-negative integer: `{value}`")))
}

/// Parse a `u64` config scalar with a key-qualified error.
fn parse_u64(key: &str, value: &str) -> Result<u64, ShimError> {
    value
        .parse()
        .map_err(|_| ShimError::Config(format!("`{key}` is not a non-negative integer: `{value}`")))
}

// ----------------------------------------------------------------------------------------------------------
// Errors.
// ----------------------------------------------------------------------------------------------------------

/// A fatal shim error (config or socket bind/accept). Per-connection I/O errors are handled locally and end
/// only that connection's thread; they never reach here.
#[derive(Debug)]
enum ShimError {
    /// The config file could not be read or parsed.
    Config(String),
    /// An ingress/egress listener could not be bound.
    Bind(String),
}

impl core::fmt::Display for ShimError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Config(e) => write!(f, "config: {e}"),
            Self::Bind(e) => write!(f, "bind: {e}"),
        }
    }
}

impl core::error::Error for ShimError {}

// ----------------------------------------------------------------------------------------------------------
// Internal messages.
// ----------------------------------------------------------------------------------------------------------

/// One delivered message handed to an egress subscriber: the original `publish_ts_millis` plus the payload
/// (the `{publish_ts || payload}` record, split back apart after the seal round-trip).
#[derive(Clone)]
struct Delivered {
    /// Epoch-millis the producer stamped at `sendAsync` time (full E2E latency anchor). Travels as opaque
    /// record bytes through the sealed path.
    publish_ts: u64,
    /// The original message payload, byte-identical to what the producer sent.
    payload: Vec<u8>,
}

/// One ingested message queued to a topic worker: the assembled `{publish_ts(8B) || payload}` record bytes.
struct Ingested {
    /// `publish_ts_millis` (8 BE bytes) followed by the raw payload — exactly the record `board` seals.
    record: Vec<u8>,
}

/// A topic's shared handle: the channel into its worker (for producers) and its subscriber list (for
/// consumers). Created lazily on first reference and stored in the [`Registry`].
#[derive(Clone)]
struct TopicHandle {
    /// Sink for ingested records → the topic worker. **Bounded** (`SyncSender`): when the seal/open worker is
    /// the bottleneck, this fills and `send` blocks, so the ingress thread stops reading the producer socket →
    /// the producer's TCP buffer fills → the producer slows. That is datarail's backpressure (0-loss, no
    /// unbounded backlog), versus an unbounded queue that would grow until OOM under a firehose.
    ingress_tx: SyncSender<Ingested>,
    /// Every egress subscriber's delivery channel. The worker fans each offloaded record out to all of them.
    subscribers: Arc<Mutex<Vec<Sender<Delivered>>>>,
}

/// The lazy topic registry: topic name → handle, behind a mutex. Cloned (`Arc`) into every connection thread.
type Registry = Arc<Mutex<HashMap<String, TopicHandle>>>;

// ----------------------------------------------------------------------------------------------------------
// Entry point.
// ----------------------------------------------------------------------------------------------------------

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("datarail-omb-shim: error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Parse the optional config (argv[1]), bind both listeners, and serve forever. Returns only on a fatal bind
/// error (the accept loops never terminate normally).
fn run() -> Result<(), ShimError> {
    let cfg = match std::env::args().nth(1) {
        Some(path) => Config::from_path(&path)?,
        None => Config::default(),
    };

    let ingress = TcpListener::bind(&cfg.ingress_addr)
        .map_err(|e| ShimError::Bind(format!("ingress {}: {e}", cfg.ingress_addr)))?;
    let egress = TcpListener::bind(&cfg.egress_addr)
        .map_err(|e| ShimError::Bind(format!("egress {}: {e}", cfg.egress_addr)))?;

    // Report the actually-bound addresses (an OS-assigned `:0` port resolves here) so a test/orchestrator can
    // discover them; flush so the line is observable before the accept loops block.
    let ingress_bound = ingress.local_addr().map_err(|e| ShimError::Bind(e.to_string()))?;
    let egress_bound = egress.local_addr().map_err(|e| ShimError::Bind(e.to_string()))?;
    println!("DATARAIL-OMB-SHIM ingress={ingress_bound} egress={egress_bound} substrate={:?}", cfg.substrate);
    let _ = std::io::stdout().flush();

    let registry: Registry = Arc::new(Mutex::new(HashMap::new()));

    // Egress acceptor on its own thread; ingress acceptor on this (the main) thread. Both loop forever.
    let egress_registry = Arc::clone(&registry);
    let egress_cfg = cfg.clone();
    std::thread::spawn(move || accept_loop(&egress, &egress_registry, &egress_cfg, serve_egress));

    accept_loop(&ingress, &registry, &cfg, serve_ingress);
    Ok(())
}

/// Generic accept loop: for every accepted connection, spawn `handler(stream, &registry, &cfg)` on its own
/// thread (each thread owns its own clones). A failed `accept` is logged and skipped (the listener stays up).
/// Never returns.
fn accept_loop(
    listener: &TcpListener,
    registry: &Registry,
    cfg: &Config,
    handler: fn(TcpStream, &Registry, &Config),
) {
    for conn in listener.incoming() {
        match conn {
            Ok(stream) => {
                let registry = Arc::clone(registry);
                let cfg = cfg.clone();
                std::thread::spawn(move || handler(stream, &registry, &cfg));
            }
            Err(e) => eprintln!("datarail-omb-shim: accept failed: {e}"),
        }
    }
}

// ----------------------------------------------------------------------------------------------------------
// Lazy topic creation.
// ----------------------------------------------------------------------------------------------------------

/// Look up `topic`, creating its worker thread + handle on first reference (lazy topics — no control message).
/// Returns a clone of the handle. A poisoned registry mutex is reported as `None` (the caller ends its
/// connection); it cannot happen unless a worker panicked, which the no-unwrap datapath precludes.
fn topic_handle(registry: &Registry, cfg: &Config, topic: &str) -> Option<TopicHandle> {
    let mut map = registry.lock().ok()?;
    if let Some(handle) = map.get(topic) {
        return Some(handle.clone());
    }
    // Bounded ingress queue → backpressure (see `TopicHandle::ingress_tx`). Depth scales with the batch size
    // (a handful of batches in flight) and is clamped so worst-case buffered memory stays bounded.
    let depth = cfg.batch_max_records.saturating_mul(16).clamp(256, 65_536);
    let (ingress_tx, ingress_rx) = mpsc::sync_channel::<Ingested>(depth);
    let subscribers: Arc<Mutex<Vec<Sender<Delivered>>>> = Arc::new(Mutex::new(Vec::new()));
    let handle = TopicHandle { ingress_tx, subscribers: Arc::clone(&subscribers) };
    let worker_cfg = cfg.clone();
    let topic_name = topic.to_owned();
    std::thread::spawn(move || topic_worker(&topic_name, &worker_cfg, &ingress_rx, &subscribers));
    map.insert(topic.to_owned(), handle.clone());
    Some(handle)
}

// ----------------------------------------------------------------------------------------------------------
// Topic worker — the REAL sealed datapath (board → substrate → offload → fan-out), batched.
// ----------------------------------------------------------------------------------------------------------

/// Build the content contract for the shim's route: a **zero-length** required prefix (OMB sends random
/// payloads, so no prefix may be required) and a `max_record_len` of `max_record_bytes + 8` (the 8-byte
/// `publish_ts` rides ahead of every payload inside the record). Mirrors `OMB-PROTOCOL.md` §"Shim internals".
fn build_contract(cfg: &Config) -> ContentContract {
    ContentContract::new(cfg.max_record_bytes + 8, Vec::new())
}

/// Build the shared [`TerminalConfig`] for the shim's single in-process route (GCM-SIV default AEAD).
fn build_terminal_config() -> TerminalConfig {
    TerminalConfig {
        route_id: ROUTE_ID,
        stream_id: STREAM_ID,
        aead_alg: AeadAlg::Gcmsiv256,
        // The destination X25519 *public* key matching `DEST_X25519_SECRET` — the same primitive
        // `head_to_head` uses, so `board`'s per-cofre key-wrap targets a key the dest can open.
        dest_x25519_pk: x25519_public(&DEST_X25519_SECRET),
        tenant_secret: TENANT_SECRET,
    }
}

/// One topic's REAL sealed datapath: a real `SourceTerminal`, a real `DestTerminal`, the chosen substrate, and
/// the two cursors that make fan-out exactly-once. Bundled into one struct so the per-batch step is a method
/// (not an 8-argument free function) — and so the cohesive seal-path state lives in one place.
struct TopicEngine {
    /// Onboarding terminal — seals each batch into a cofre.
    source: SourceTerminal,
    /// Offloading terminal — verifies/opens/admits/commits each cofre; its sink holds the delivered records.
    dest: DestTerminal,
    /// The transport the cofre traverses between board and offload.
    transport: Transport,
    /// Monotonic record-key counter: a unique key per cofre ⇒ the once-gate never false-dedups a distinct
    /// batch (the same discipline as `datarail-bench`'s `head_to_head`).
    cofre_seq: u64,
}

impl TopicEngine {
    /// Build the engine for one topic from the shim config (fixed in-process route keys — bench shim only).
    fn new(cfg: &Config) -> Self {
        let term_cfg = build_terminal_config();
        let contract = build_contract(cfg);
        let src_vk = verifying_key(&SRC_SEED);
        let source = SourceTerminal::new(term_cfg.clone(), contract.clone(), SRC_SEED);
        let dest = DestTerminal::new(term_cfg, contract, src_vk, DEST_SEED, DEST_X25519_SECRET);
        Self {
            source,
            dest,
            transport: Transport::new(cfg.substrate),
            cofre_seq: 0,
        }
    }

    /// Board one batch into a sealed cofre, move it across the substrate, offload it (full verify→open→admit→
    /// commit), then split each newly-committed record back into `{publish_ts, payload}` and fan it out to
    /// every subscriber. A unique `record_key` per cofre keeps the once-gate from deduping distinct batches.
    ///
    /// The seal-path calls are infallible in practice for our own well-formed records, but the code never
    /// unwraps: a board/offload error or a non-`Delivered` disposition is logged and the batch is skipped (a
    /// regression would surface as missing deliveries in the round-trip test, never a panic).
    fn flush(&mut self, topic: &str, batch: &[Vec<u8>], subscribers: &Arc<Mutex<Vec<Sender<Delivered>>>>) {
        if batch.is_empty() {
            return;
        }
        let refs: Vec<&[u8]> = batch.iter().map(Vec::as_slice).collect();
        let record_key = self.cofre_seq.to_le_bytes();
        self.cofre_seq += 1;

        let cofre = match self.source.board(&refs, &record_key) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("datarail-omb-shim[{topic}]: board failed ({e}); dropping {} record(s)", batch.len());
                return;
            }
        };

        let received = match self.transport.relay(&cofre) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("datarail-omb-shim[{topic}]: substrate relay failed ({e}); dropping batch");
                return;
            }
        };

        match self.dest.offload(&received) {
            Ok(Disposition::Delivered) => {}
            Ok(other) => {
                // Distinct record-keys mean this is never Duplicate; a DeadLettered batch never delivers.
                eprintln!("datarail-omb-shim[{topic}]: offload returned {other:?} (not Delivered); skipping");
                return;
            }
            Err(e) => {
                eprintln!("datarail-omb-shim[{topic}]: offload failed ({e}); dropping batch");
                return;
            }
        }

        // DRAIN the records this offload committed (take, not borrow): the sink must not retain them, else a
        // long stream grows it until OOM. Each offload commits exactly this batch, and we drained last time, so
        // the drain yields exactly the fresh records. Split each back into {publish_ts, payload} and fan out.
        for record in self.dest.sink_mut().take_committed() {
            if let Some(delivered) = split_record(&record) {
                fan_out(subscribers, &delivered);
            } else {
                eprintln!("datarail-omb-shim[{topic}]: committed record shorter than 8-byte ts header; skip");
            }
        }
    }
}

/// The per-topic worker: owns the topic's [`TopicEngine`], drains the ingress channel in **batches** (flush on
/// `batch_max_records` OR `batch_max_micros`, whichever first), runs every batch through the full seal/open
/// path, and fans the delivered records out to every subscriber. Exits when the registry (hence every
/// producer) drops the sender.
fn topic_worker(
    topic: &str,
    cfg: &Config,
    ingress_rx: &Receiver<Ingested>,
    subscribers: &Arc<Mutex<Vec<Sender<Delivered>>>>,
) {
    let mut engine = TopicEngine::new(cfg);
    let batch_window = Duration::from_micros(cfg.batch_max_micros);

    loop {
        // Block for the first record of a batch; `None` means every producer hung up ⇒ the topic is done.
        let Some(first) = recv_blocking(ingress_rx) else {
            return;
        };
        let mut batch: Vec<Vec<u8>> = Vec::with_capacity(cfg.batch_max_records);
        batch.push(first.record);

        // Fill the batch until it is full OR the time window elapses, whichever first.
        let deadline = Instant::now() + batch_window;
        while batch.len() < cfg.batch_max_records {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            match ingress_rx.recv_timeout(deadline - now) {
                Ok(msg) => batch.push(msg.record),
                // Timeout: window elapsed. Disconnected: every producer hung up — flush what we have; the
                // next outer-loop `recv_blocking` then returns `None` and the worker exits.
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => break,
            }
        }

        engine.flush(topic, &batch, subscribers);
    }
}

/// Block on the next ingested record. Returns `None` only when the channel is disconnected (all producers
/// dropped). A periodic timeout keeps the worker responsive without busy-spinning.
fn recv_blocking(ingress_rx: &Receiver<Ingested>) -> Option<Ingested> {
    loop {
        match ingress_rx.recv_timeout(Duration::from_millis(200)) {
            Ok(msg) => return Some(msg),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return None,
        }
    }
}

/// Split a committed record `{publish_ts(8 BE) || payload}` back into its [`Delivered`] parts. Returns `None`
/// if the record is shorter than the 8-byte timestamp header (impossible for shim-boarded records).
fn split_record(record: &[u8]) -> Option<Delivered> {
    let ts_bytes: [u8; 8] = record.get(0..8)?.try_into().ok()?;
    Some(Delivered {
        publish_ts: u64::from_be_bytes(ts_bytes),
        payload: record[8..].to_vec(),
    })
}

/// Fan `delivered` out to every live subscriber on the topic, dropping any whose consumer has disconnected
/// (its egress thread closed the receiver). Each distinct subscription is an independent delivery copy, per
/// OMB semantics ("each subscription gets all messages").
fn fan_out(subscribers: &Arc<Mutex<Vec<Sender<Delivered>>>>, delivered: &Delivered) {
    let Ok(mut subs) = subscribers.lock() else {
        return; // poisoned only if an egress thread panicked; nothing to deliver to then.
    };
    subs.retain(|tx| tx.send(delivered.clone()).is_ok());
}

// ----------------------------------------------------------------------------------------------------------
// Substrate relay — board → (chosen transport) → offload. Loopback is a direct in-process handoff.
// ----------------------------------------------------------------------------------------------------------

/// The transport a cofre traverses between `board` and `offload`. `Loopback` returns the cofre directly (the
/// frozen default in-process handoff); `Tcp`/`Shmem` push it through a real substrate and drain it back, so
/// the full framed encode→decode round-trip is exercised before the dest opens it.
enum Transport {
    /// Direct hand-off — no transport object; the cofre is offloaded as boarded.
    Loopback,
    /// A real `127.0.0.1` TCP substrate (a genuine kernel hop on loopback).
    Tcp(TcpSubstrate),
    /// A real shared-memory ring substrate.
    Shmem(ShmemRing),
}

impl Transport {
    /// Build the transport for `kind`. A failure to create the real `tcp`/`shmem` substrate falls back to a
    /// loopback handoff (logged): the seal/open path still runs, only the extra hop is skipped — a benchmark
    /// adapter degrades to the faithful default rather than refusing to serve.
    fn new(kind: SubstrateKind) -> Self {
        match kind {
            SubstrateKind::Loopback => Self::Loopback,
            SubstrateKind::Tcp => match TcpSubstrate::loopback_pair() {
                Ok(s) => Self::Tcp(s),
                Err(e) => {
                    eprintln!("datarail-omb-shim: tcp substrate unavailable ({e}); using loopback handoff");
                    Self::Loopback
                }
            },
            SubstrateKind::Shmem => match ShmemRing::pair() {
                Ok(s) => Self::Shmem(s),
                Err(e) => {
                    eprintln!("datarail-omb-shim: shmem substrate unavailable ({e}); using loopback handoff");
                    Self::Loopback
                }
            },
        }
    }

    /// Move one cofre across the transport and return the received copy to offload. For `Loopback` this is the
    /// same cofre; for a real substrate it is the byte-identical cofre after a framed send/recv round-trip.
    ///
    /// # Errors
    /// Returns a transport error string if a real substrate fails to send/recv (loopback never fails).
    fn relay(&mut self, cofre: &Cofre) -> Result<Cofre, String> {
        match self {
            Self::Loopback => Ok(cofre.clone()),
            Self::Tcp(s) => relay_through(s, cofre),
            Self::Shmem(s) => relay_through(s, cofre),
        }
    }
}

/// Send `cofre` over a real substrate and drain it back (the substrate is a FIFO pipe; one in, one out). Used
/// for the `tcp`/`shmem` modes. Polls `recv` with a bounded spin so a slow kernel hop cannot hang the worker
/// forever, then acks the received cofre to keep any resumable substrate's window clear.
fn relay_through<S: Substrate>(sub: &mut S, cofre: &Cofre) -> Result<Cofre, String>
where
    S::Error: core::fmt::Display,
{
    sub.send(cofre).map_err(|e| format!("send: {e}"))?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match sub.recv() {
            Ok(Some(received)) => {
                sub.ack(received.etiqueta.cofre_id).map_err(|e| format!("ack: {e}"))?;
                return Ok(received);
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    return Err("substrate recv timed out".to_owned());
                }
                std::thread::yield_now();
            }
            Err(e) => return Err(format!("recv: {e}")),
        }
    }
}

// ----------------------------------------------------------------------------------------------------------
// Ingress connection — producer → shim. Header, then frames; one monotonic ack per accepted message.
// ----------------------------------------------------------------------------------------------------------

/// Serve one producer connection. Reads the `[u16 topic_len][topic]` header, resolves (lazily creates) the
/// topic, then loops reading `[u32 payload_len][u64 publish_ts][payload]` frames: for each, assemble the
/// `{publish_ts || payload}` record, hand it to the topic worker, and write back a monotonic `[u64 seq]` ack
/// (from 0, in receive order). The handoff to the worker IS the ack point (acks=1: boarded + handed to the
/// substrate, per the frozen contract) — the worker still seals every record on the real datapath. Ends the
/// thread cleanly on EOF or any socket/parse error.
fn serve_ingress(stream: TcpStream, registry: &Registry, cfg: &Config) {
    if let Err(e) = serve_ingress_inner(stream, registry, cfg) {
        // EOF/peer-close is the normal end of a producer; only note genuinely unexpected errors.
        if e.kind() != std::io::ErrorKind::UnexpectedEof && e.kind() != std::io::ErrorKind::ConnectionReset {
            eprintln!("datarail-omb-shim: ingress connection ended: {e}");
        }
    }
}

/// The fallible body of [`serve_ingress`]; every framing/socket error bubbles up as `io::Error` to end the
/// connection thread without panicking.
fn serve_ingress_inner(stream: TcpStream, registry: &Registry, cfg: &Config) -> std::io::Result<()> {
    // Buffered I/O on BOTH directions: an unbuffered per-message read+ack is ~3 syscalls/msg, which caps
    // throughput at the syscall rate (the real bottleneck, not the crypto). A BufReader coalesces frame reads
    // and a BufWriter coalesces acks; acks flush when we have caught up to the socket (so a streaming producer
    // still gets timely acks, but a burst is acked in one write).
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut acks = BufWriter::new(stream);
    let topic = read_topic_header(&mut reader)?;
    let handle = topic_handle(registry, cfg, &topic)
        .ok_or_else(|| std::io::Error::other("topic registry unavailable"))?;

    let mut seq: u64 = 0;
    let mut hdr = [0u8; 12]; // [u32 payload_len][u64 publish_ts]
    loop {
        // Read the 12-byte frame header; a clean EOF here ends the producer. Flush any pending acks first so a
        // producer that paused is not left waiting on buffered-but-unsent acks.
        if reader.buffer().is_empty() {
            acks.flush()?;
        }
        match read_exact_or_eof(&mut reader, &mut hdr)? {
            ReadEnd::Eof => {
                acks.flush()?;
                return Ok(());
            }
            ReadEnd::Full => {}
        }
        let payload_len = u32::from_be_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]) as usize;
        let publish_ts = u64::from_be_bytes([hdr[4], hdr[5], hdr[6], hdr[7], hdr[8], hdr[9], hdr[10], hdr[11]]);
        if payload_len > cfg.max_record_bytes {
            return Err(std::io::Error::other(format!(
                "frame payload_len {payload_len} exceeds max_record_bytes {}",
                cfg.max_record_bytes
            )));
        }

        // Assemble the record `{publish_ts(8 BE) || payload}` directly, reading the payload into place.
        let mut record = vec![0u8; 8 + payload_len];
        record[0..8].copy_from_slice(&publish_ts.to_be_bytes());
        reader.read_exact(&mut record[8..])?;

        // Hand to the worker (the accept point; the bounded queue applies backpressure here), then buffer the
        // ack in receive order. A worker-gone error ends us. Flush pending acks before a send that may block so
        // the producer is never stalled waiting on acks we are holding.
        if reader.buffer().is_empty() {
            acks.flush()?;
        }
        handle
            .ingress_tx
            .send(Ingested { record })
            .map_err(|_| std::io::Error::other("topic worker gone"))?;
        acks.write_all(&seq.to_be_bytes())?;
        seq += 1;
    }
}

// ----------------------------------------------------------------------------------------------------------
// Egress connection — consumer → shim. Header, then a stream of delivered frames.
// ----------------------------------------------------------------------------------------------------------

/// Serve one consumer connection. Reads the `[u16 topic_len][topic][u16 sub_len][sub]` header, registers a
/// fresh delivery channel on the topic (each distinct subscription is its own fan-out copy), then streams
/// `[u32 payload_len][u64 publish_ts][payload]` frames for every delivered message until the consumer
/// disconnects. Ends the thread cleanly on any socket error.
fn serve_egress(stream: TcpStream, registry: &Registry, cfg: &Config) {
    if let Err(e) = serve_egress_inner(stream, registry, cfg) {
        if e.kind() != std::io::ErrorKind::UnexpectedEof
            && e.kind() != std::io::ErrorKind::ConnectionReset
            && e.kind() != std::io::ErrorKind::BrokenPipe
        {
            eprintln!("datarail-omb-shim: egress connection ended: {e}");
        }
    }
}

/// The fallible body of [`serve_egress`]: register a subscriber channel, then write each delivered frame to
/// the socket. The `sub` name is read per the protocol and (intentionally) used only to model an independent
/// delivery copy — every subscription receives every message (OMB consumer-group semantics, v1).
fn serve_egress_inner(stream: TcpStream, registry: &Registry, cfg: &Config) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let topic = read_topic_header(&mut reader)?;
    let _sub = read_len_prefixed_string(&mut reader)?; // independent fan-out copy per subscription (v1).

    let handle = topic_handle(registry, cfg, &topic)
        .ok_or_else(|| std::io::Error::other("topic registry unavailable"))?;
    let (tx, rx) = mpsc::channel::<Delivered>();
    handle
        .subscribers
        .lock()
        .map_err(|_| std::io::Error::other("subscriber registry poisoned"))?
        .push(tx);

    // Buffered writer: a per-message `write_all` is one syscall per message and caps egress throughput at the
    // syscall rate. We coalesce a burst of delivered frames into the buffer and flush only when the delivery
    // channel momentarily drains — full throughput under load, still low-latency when idle.
    let mut out = BufWriter::new(stream);
    let mut frame = Vec::new();
    let mut write_frame = |w: &mut BufWriter<TcpStream>, d: &Delivered| -> std::io::Result<()> {
        let payload_len =
            u32::try_from(d.payload.len()).map_err(|_| std::io::Error::other("delivered payload exceeds u32"))?;
        frame.clear();
        frame.extend_from_slice(&payload_len.to_be_bytes());
        frame.extend_from_slice(&d.publish_ts.to_be_bytes());
        frame.extend_from_slice(&d.payload);
        w.write_all(&frame)
    };
    loop {
        // Block for the next delivery; `recv` errs only when the worker drops the sender (topic done).
        let Ok(delivered) = rx.recv() else {
            out.flush()?;
            return Ok(());
        };
        write_frame(&mut out, &delivered)?;
        // Drain whatever else is immediately ready into the same buffer, then flush once.
        while let Ok(more) = rx.try_recv() {
            write_frame(&mut out, &more)?;
        }
        out.flush()?;
    }
}

// ----------------------------------------------------------------------------------------------------------
// Wire-reading helpers (big-endian, length-prefixed). std-only.
// ----------------------------------------------------------------------------------------------------------

/// Whether a read filled the buffer or hit a clean EOF at a frame boundary.
enum ReadEnd {
    /// The buffer was fully filled.
    Full,
    /// EOF occurred with **zero** bytes read (a clean end between frames).
    Eof,
}

/// Read exactly `buf.len()` bytes, distinguishing a clean EOF at the start (no bytes yet) from a truncated
/// frame mid-read (which is an error). A frame header read uses this so a producer closing between frames ends
/// the connection cleanly rather than erroring.
fn read_exact_or_eof(stream: &mut impl Read, buf: &mut [u8]) -> std::io::Result<ReadEnd> {
    let mut filled = 0usize;
    while filled < buf.len() {
        match stream.read(&mut buf[filled..]) {
            Ok(0) => {
                if filled == 0 {
                    return Ok(ReadEnd::Eof);
                }
                return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof));
            }
            Ok(n) => filled += n,
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(ReadEnd::Full)
}

/// Read a `[u16 len][utf8]` length-prefixed string (big-endian). Used for the topic and the subscription name.
fn read_len_prefixed_string(stream: &mut impl Read) -> std::io::Result<String> {
    let mut len_bytes = [0u8; 2];
    stream.read_exact(&mut len_bytes)?;
    let len = u16::from_be_bytes(len_bytes) as usize;
    let mut bytes = vec![0u8; len];
    stream.read_exact(&mut bytes)?;
    String::from_utf8(bytes).map_err(|_| std::io::Error::other("topic/sub name is not valid UTF-8"))
}

/// Read the leading `[u16 topic_len][topic_utf8]` header common to both ingress and egress connections.
fn read_topic_header(stream: &mut impl Read) -> std::io::Result<String> {
    read_len_prefixed_string(stream)
}

#[cfg(test)]
mod tests {
    use super::{build_contract, split_record, unquote, Config, SubstrateKind};

    #[test]
    fn config_defaults_match_the_frozen_contract() {
        let cfg = Config::default();
        assert_eq!(cfg.ingress_addr, "127.0.0.1:7701");
        assert_eq!(cfg.egress_addr, "127.0.0.1:7702");
        assert_eq!(cfg.substrate, SubstrateKind::Loopback);
        assert_eq!(cfg.batch_max_records, 128);
        assert_eq!(cfg.batch_max_micros, 1000);
        assert_eq!(cfg.max_record_bytes, 1_048_576);
    }

    #[test]
    fn config_parses_documented_keys_and_layers_over_defaults() {
        let toml = r#"
            # a comment
            ingress_addr = "127.0.0.1:9001"
            egress_addr  = '127.0.0.1:9002'
            substrate    = "tcp"
            batch_max_records = 64
            batch_max_micros  = 500   # inline comment
        "#;
        let cfg = Config::from_str(toml).unwrap();
        assert_eq!(cfg.ingress_addr, "127.0.0.1:9001");
        assert_eq!(cfg.egress_addr, "127.0.0.1:9002");
        assert_eq!(cfg.substrate, SubstrateKind::Tcp);
        assert_eq!(cfg.batch_max_records, 64);
        assert_eq!(cfg.batch_max_micros, 500);
        // Untouched key keeps its default.
        assert_eq!(cfg.max_record_bytes, 1_048_576);
    }

    #[test]
    fn config_rejects_unknown_key_and_zero_batch() {
        assert!(Config::from_str("nope = 1").is_err());
        assert!(Config::from_str("batch_max_records = 0").is_err());
        assert!(Config::from_str("substrate = \"carrier-pigeon\"").is_err());
    }

    #[test]
    fn unquote_strips_one_matching_pair() {
        assert_eq!(unquote("\"x\""), "x");
        assert_eq!(unquote("'x'"), "x");
        assert_eq!(unquote("bare"), "bare");
        assert_eq!(unquote("\"mismatch'"), "\"mismatch'");
    }

    #[test]
    fn contract_admits_max_payload_plus_ts_and_zero_prefix() {
        let cfg = Config::default();
        let contract = build_contract(&cfg);
        assert!(contract.required_prefix.is_empty());
        // A full-size payload plus the 8-byte ts header must validate (boundary).
        let max_record = vec![0xABu8; cfg.max_record_bytes + 8];
        assert!(contract.validate(&max_record));
        // One byte over the cap must NOT validate.
        let too_big = vec![0xABu8; cfg.max_record_bytes + 9];
        assert!(!contract.validate(&too_big));
    }

    #[test]
    fn split_record_recovers_ts_and_payload() {
        let mut record = Vec::new();
        record.extend_from_slice(&0x0102_0304_0506_0708u64.to_be_bytes());
        record.extend_from_slice(b"hello");
        let d = split_record(&record).unwrap();
        assert_eq!(d.publish_ts, 0x0102_0304_0506_0708);
        assert_eq!(d.payload, b"hello");
        // A record shorter than the 8-byte header is rejected (no panic).
        assert!(split_record(&[0u8; 4]).is_none());
    }
}
