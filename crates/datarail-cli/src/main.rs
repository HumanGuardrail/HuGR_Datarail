//! `datarail` — the command-line surface (SPEC `09-spec-cli.md`).
//!
//! Subcommands:
//! - `validate <rail.toml>` — parse + validate a route spec.
//! - `keygen` — emit a fresh Ed25519 seed + verifying key (hex).
//! - `run <rail.toml> [record …] [--watch]` — board → loopback rail → offload a batch; report the outcome
//!   (records also read from stdin, one per line; a built-in demo if none). `--watch` prints a live speedometer.
//! - `replay <rail.toml> <range>` — re-ship a 0-based index range of the source's records through the full pipe.
//! - `verify <rail.toml> <hex>` — verify a wire-encoded cofre's seal against the route's source key.
//! - `ticket <rail.toml>` — emit a signed route-capability ticket (hex).
//! - `pair` — local rehearsal of the SPEC-08 identity layer (F3 SPAKE2 pairing + F2 `Noise_KK` session).
//! - `recv` / `send` — a genuine **two-process** transfer over a real TCP socket (separate OS processes):
//!   `recv` binds + offloads a shipment; `send` connects + boards + ships it. Cross-host at the product level.
//!
//! Lean by design (Charter *leveza*): hand-rolled argument handling (no `clap`) and `/dev/urandom` for keygen
//! (no `rand`). The CLI holds no logic of its own beyond wiring the crates together.

#![forbid(unsafe_code)]

use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use datarail_connectors::{
    HttpSource, LineFileSink, LineFileSource, PgConfig, PostgresSink, Sink, SliceSource, Source, VecSink,
};
use datarail_core::{Cofre, Disposition, Substrate};
use datarail_crypto::{blake3_256, ctx, sign_domain, verifying_key};
use datarail_identity::{NoiseSubstrate, Pairing, ShortCode, StaticKeypair};
use datarail_rail::{LoopbackSubstrate, TcpSubstrate};
use datarail_spec::{RailSpec, SpecError};
use datarail_terminal::{DestTerminal, SourceTerminal, TerminalError};

const USAGE: &str = "\
datarail — provider-blind data rail (SPEC 09)

USAGE:
    datarail validate <rail.toml>
    datarail keygen
    datarail run <rail.toml> [record ...] [--watch]    (records also read from stdin, one per line)
    datarail replay <rail.toml> <range>               (range: start..end | start..=end | all; 0-based)
    datarail verify <rail.toml> <cofre-hex>
    datarail ticket <rail.toml>
    datarail pair                                     (local F2/F3 identity rehearsal — see note)
    datarail pair --listen ADDR  --code <hex> [--my-static <hex>]    (F3 remote pairing: responder)
    datarail pair --connect ADDR --code <hex> [--my-static <hex>]    (F3 remote pairing: initiator)
    datarail recv <rail.toml> [--listen ADDR] [--sink-file F] [--count N]   (cross-process dest)
    datarail send <rail.toml> --connect ADDR [--source-file F | record ...] (cross-process source)

    keygen --noise   mint a Noise_KK static keypair (X25519) for the encrypted hop.
    send/recv --noise-secret <hex> --peer-public <hex>   wrap the TCP hop in a Noise_KK channel (F2):
                     mutual-static auth + on-wire encryption of the etiqueta metadata. Both flags or neither.

    run connectors (the v1 product flow — an HTTP API to a database, sealed end-to-end):
      --source-http URL          GET an endpoint; each non-empty line of the body is one record (one batch).
      --sink-postgres CONN       land each delivered record as a row, via the zero-dep Postgres wire protocol.
                                 CONN = comma-separated key=value: host,user,db,table,column[,port][,password]
                                 e.g. host=db,user=rail,db=events,table=raw,column=data,port=5432,password=secret
      (precedence: --source-http > --source-file > inline/stdin ; --sink-postgres > --sink-file > in-memory)
    --watch        on `run`: print a live one-line speedometer per batch (no TUI; raw stdout).
    replay reads from the configured source (--source-file > inline args > stdin) and re-ships only the
    selected index slice; the once-gate dedups an already-delivered range in-process (no re-commit). True
    cross-invocation time-travel needs the manifest/WAL store (future) — this replays from the source.
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match dispatch(&args) {
        Ok(output) => {
            print!("{output}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn dispatch(args: &[String]) -> Result<String, CliError> {
    let (cmd, rest) = args.split_first().ok_or(CliError::Usage)?;
    match cmd.as_str() {
        "validate" => cmd_validate(rest),
        "keygen" => cmd_keygen(rest),
        "run" => cmd_run(rest),
        "replay" => cmd_replay(rest),
        "verify" => cmd_verify(rest),
        "ticket" => cmd_ticket(rest),
        "pair" => cmd_pair(rest),
        "send" => cmd_send(rest),
        "recv" => cmd_recv(rest),
        "help" | "-h" | "--help" => Ok(USAGE.to_owned()),
        other => Err(CliError::UnknownCommand(other.to_owned())),
    }
}

// ----------------------------------------------------------------------------------------------------------
// Errors.
// ----------------------------------------------------------------------------------------------------------

#[derive(Debug)]
enum CliError {
    Usage,
    UnknownCommand(String),
    MissingArg(&'static str),
    Arg(String),
    Io(String),
    Spec(SpecError),
    BadHex,
    Decode(String),
    SealInvalid(String),
    Terminal(TerminalError),
    Rail(String),
    NoCofre,
    BadRange(String),
    Identity(String),
}

impl core::fmt::Display for CliError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Usage => write!(f, "no command given\n\n{USAGE}"),
            Self::UnknownCommand(c) => write!(f, "unknown command `{c}`\n\n{USAGE}"),
            Self::MissingArg(a) => write!(f, "missing argument <{a}>"),
            Self::Arg(e) => write!(f, "argument: {e}"),
            Self::Io(e) => write!(f, "io: {e}"),
            Self::Spec(e) => write!(f, "spec: {e}"),
            Self::BadHex => write!(f, "argument is not valid hex"),
            Self::Decode(e) => write!(f, "cofre decode: {e}"),
            Self::SealInvalid(e) => write!(f, "seal INVALID: {e}"),
            Self::Terminal(e) => write!(f, "terminal: {e}"),
            Self::Rail(e) => write!(f, "rail: {e}"),
            Self::NoCofre => write!(f, "no cofre came off the rail"),
            Self::BadRange(r) => write!(f, "bad range `{r}` (expected start..end, start..=end, or all)"),
            Self::Identity(e) => write!(f, "identity: {e}"),
        }
    }
}

impl std::error::Error for CliError {}

// ----------------------------------------------------------------------------------------------------------
// Small helpers (hex, file/spec loading) — Charter leveza: no hex/serde crates.
// ----------------------------------------------------------------------------------------------------------

fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(char::from(HEX[usize::from(b >> 4)]));
        s.push(char::from(HEX[usize::from(b & 0x0f)]));
    }
    s
}

fn from_hex(s: &str) -> Option<Vec<u8>> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok())
        .collect()
}

fn load_spec(path: &str) -> Result<RailSpec, CliError> {
    let src = std::fs::read_to_string(path).map_err(|e| CliError::Io(format!("{path}: {e}")))?;
    RailSpec::parse(&src).map_err(CliError::Spec)
}

fn require<'a>(rest: &'a [String], n: usize, name: &'static str) -> Result<&'a str, CliError> {
    rest.get(n)
        .map(String::as_str)
        .ok_or(CliError::MissingArg(name))
}

// ----------------------------------------------------------------------------------------------------------
// Subcommands.
// ----------------------------------------------------------------------------------------------------------

fn cmd_validate(rest: &[String]) -> Result<String, CliError> {
    let spec = load_spec(require(rest, 0, "rail.toml")?)?;
    Ok(format!(
        "ok\n  route_id   = 0x{}\n  stream_id  = 0x{}\n  aead       = {:?}\n  guarantee  = {}\n  onboarding = max {}B, prefix {}B\n  offloading = max {}B, prefix {}B\n",
        to_hex(&spec.route.route_id),
        to_hex(&spec.route.stream_id),
        spec.route.aead,
        spec.route.guarantee,
        spec.onboarding.max_record_len,
        spec.onboarding.required_prefix.len(),
        spec.offloading.max_record_len,
        spec.offloading.required_prefix.len(),
    ))
}

fn cmd_keygen(rest: &[String]) -> Result<String, CliError> {
    // `--noise` mints a Noise_KK static keypair (the SPEC-08 X25519 KEM key) for the encrypted-hop send/recv;
    // otherwise the Ed25519 signing identity.
    if rest.iter().any(|a| a == "--noise") {
        let kp = StaticKeypair::generate().map_err(|e| CliError::Identity(e.to_string()))?;
        return Ok(format!(
            "noise_secret = \"0x{}\"   # keep secret; pass to your endpoint as --noise-secret\nnoise_public = \"0x{}\"   # share; the peer pins it as --peer-public\n",
            to_hex(&kp.secret()),
            to_hex(&kp.public()),
        ));
    }
    let seed = random_seed()?;
    let vk = verifying_key(&seed);
    Ok(format!(
        "source_seed = \"0x{}\"\nverifying_key = \"0x{}\"\n",
        to_hex(&seed),
        to_hex(&vk),
    ))
}

fn random_seed() -> Result<[u8; 32], CliError> {
    let mut file =
        std::fs::File::open("/dev/urandom").map_err(|e| CliError::Io(e.to_string()))?;
    let mut seed = [0u8; 32];
    file.read_exact(&mut seed)
        .map_err(|e| CliError::Io(e.to_string()))?;
    Ok(seed)
}

fn cmd_run(rest: &[String]) -> Result<String, CliError> {
    let spec = load_spec(require(rest, 0, "rail.toml")?)?;

    // Optional `--source-file` / `--sink-file` / `--watch`; remaining args are inline records.
    let mut source_file: Option<String> = None;
    let mut sink_file: Option<String> = None;
    let mut source_http: Option<String> = None;
    let mut sink_pg: Option<String> = None;
    let mut watch = false;
    let mut inline: Vec<Vec<u8>> = Vec::new();
    let mut i = 1;
    while i < rest.len() {
        match rest[i].as_str() {
            "--source-file" => {
                source_file = Some(require(rest, i + 1, "path")?.to_string());
                i += 2;
            }
            "--sink-file" => {
                sink_file = Some(require(rest, i + 1, "path")?.to_string());
                i += 2;
            }
            "--source-http" => {
                source_http = Some(require(rest, i + 1, "url")?.to_string());
                i += 2;
            }
            "--sink-postgres" => {
                sink_pg = Some(require(rest, i + 1, "conn")?.to_string());
                i += 2;
            }
            "--watch" => {
                watch = true;
                i += 1;
            }
            other => {
                inline.push(other.as_bytes().to_vec());
                i += 1;
            }
        }
    }

    // Source connector: `--source-http` (one GET = one batch) > `--source-file` > inline > stdin > demo.
    let mut source: Box<dyn Source> = if let Some(url) = source_http {
        Box::new(HttpSource::get(&url).map_err(|e| CliError::Io(e.to_string()))?)
    } else if let Some(path) = source_file {
        Box::new(LineFileSource::open(&path).map_err(|e| CliError::Io(e.to_string()))?)
    } else {
        let mut records = resolve_inline_or_stdin(inline)?;
        if records.is_empty() {
            records = vec![b"evt:hello".to_vec(), b"evt:world".to_vec()];
        }
        Box::new(SliceSource::one(records))
    };

    // Sink connector: `--sink-postgres` (seal→land as rows) > `--sink-file` (append lines) > in-memory.
    let mut sink: Box<dyn Sink> = if let Some(conn) = sink_pg {
        Box::new(PostgresSink::connect(parse_pg_conn(&conn)?).map_err(|e| CliError::Io(e.to_string()))?)
    } else if let Some(path) = sink_file {
        Box::new(LineFileSink::create(&path).map_err(|e| CliError::Io(e.to_string()))?)
    } else {
        Box::new(VecSink::new())
    };

    run_pipe(&spec, source.as_mut(), sink.as_mut(), watch)
}

/// If `inline` already has records, use them; otherwise read stdin (one record per non-empty line). Shared by
/// `run` and `replay` so both resolve their source the same way (file handling differs and stays at the call site).
fn resolve_inline_or_stdin(inline: Vec<Vec<u8>>) -> Result<Vec<Vec<u8>>, CliError> {
    if !inline.is_empty() {
        return Ok(inline);
    }
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .map_err(|e| CliError::Io(e.to_string()))?;
    Ok(buf
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| l.as_bytes().to_vec())
        .collect())
}

/// Parse a `--sink-postgres` connection string: comma-separated `key=value` pairs. Required keys:
/// `host`, `user`, `db`, `table`, `column`; optional: `port` (default 5432), `password`.
/// Example: `host=db.internal,port=5432,user=rail,password=secret,db=events,table=raw,column=data`.
fn parse_pg_conn(conn: &str) -> Result<PgConfig, CliError> {
    let mut host = None;
    let mut user = None;
    let mut db = None;
    let mut table = None;
    let mut column = None;
    let mut port: u16 = 5432;
    let mut password = None;
    for pair in conn.split(',').filter(|p| !p.is_empty()) {
        let (key, value) = pair
            .split_once('=')
            .ok_or_else(|| CliError::Arg(format!("--sink-postgres: expected key=value, got `{pair}`")))?;
        match key.trim() {
            "host" => host = Some(value.to_owned()),
            "user" => user = Some(value.to_owned()),
            "db" => db = Some(value.to_owned()),
            "table" => table = Some(value.to_owned()),
            "column" => column = Some(value.to_owned()),
            "password" => password = Some(value.to_owned()),
            "port" => {
                port = value
                    .parse()
                    .map_err(|_| CliError::Arg(format!("--sink-postgres: bad port `{value}`")))?;
            }
            other => return Err(CliError::Arg(format!("--sink-postgres: unknown key `{other}`"))),
        }
    }
    let need = |opt: Option<String>, name: &str| {
        opt.ok_or_else(|| CliError::Arg(format!("--sink-postgres: missing required `{name}`")))
    };
    let mut cfg = PgConfig::new(
        need(host, "host")?,
        need(user, "user")?,
        need(db, "db")?,
        need(table, "table")?,
        need(column, "column")?,
    );
    cfg.port = port;
    cfg.password = password;
    Ok(cfg)
}

/// A parsed `replay` range over the source's records: a 0-based start (inclusive) and an end (exclusive).
struct RecordRange {
    start: usize,
    end: usize,
}

/// Parse a `replay` range argument against a source of `len` records. Accepts `start..end` (exclusive),
/// `start..=end` (inclusive), or the literal `all` (the whole source). Returns the resolved exclusive
/// `[start, end)` bounds, clamped only by validation: any out-of-range, reversed, or non-numeric input is a
/// clean [`CliError::BadRange`] (never a panic).
fn parse_range(arg: &str, len: usize) -> Result<RecordRange, CliError> {
    let bad = || CliError::BadRange(arg.to_owned());
    if arg == "all" {
        return Ok(RecordRange { start: 0, end: len });
    }
    let (lhs, rhs, inclusive) = if let Some((l, r)) = arg.split_once("..=") {
        (l, r, true)
    } else if let Some((l, r)) = arg.split_once("..") {
        (l, r, false)
    } else {
        return Err(bad());
    };
    let start: usize = lhs.parse().map_err(|_| bad())?;
    let raw_end: usize = rhs.parse().map_err(|_| bad())?;
    // Inclusive end maps to exclusive end+1; guard the +1 against overflow.
    let end = if inclusive {
        raw_end.checked_add(1).ok_or_else(bad)?
    } else {
        raw_end
    };
    if start > end || end > len {
        return Err(bad());
    }
    Ok(RecordRange { start, end })
}

/// `datarail replay <rail.toml> <range>` — re-ship a 0-based index range of the source's records through the
/// full board → substrate → offload → commit pipe.
///
/// HONEST SCOPE: there is no durable manifest/WAL yet, so "replay" is not cross-invocation time-travel — it
/// reads from the **configured source** (mirroring `run`: `--source-file` > inline args > stdin), materialises
/// every record, then re-ships only the selected index slice. True replay-from-history needs the manifest/WAL
/// store (future). The demonstrable value today is the once-gate: re-shipping an already-delivered slice **in
/// one process** offloads it as [`Disposition::Duplicate`] with zero re-commit (exactly-once at the sink).
fn cmd_replay(rest: &[String]) -> Result<String, CliError> {
    let spec = load_spec(require(rest, 0, "rail.toml")?)?;

    // Resolve the source exactly like `run`: `--source-file` > inline args > stdin (no built-in demo here —
    // replay needs a real, indexable source). Parse `--source-file` anywhere; the first remaining positional
    // is the range argument, any further positionals are inline records.
    let mut source_file: Option<String> = None;
    let mut positionals: Vec<&str> = Vec::new();
    let mut i = 1;
    while i < rest.len() {
        match rest[i].as_str() {
            "--source-file" => {
                source_file = Some(require(rest, i + 1, "path")?.to_string());
                i += 2;
            }
            other => {
                positionals.push(other);
                i += 1;
            }
        }
    }
    let (range_arg, inline_strs) = positionals
        .split_first()
        .ok_or(CliError::MissingArg("range"))?;
    let range_arg = (*range_arg).to_owned();
    let inline: Vec<Vec<u8>> = inline_strs.iter().map(|s| s.as_bytes().to_vec()).collect();

    let records: Vec<Vec<u8>> = if let Some(path) = source_file {
        let raw = std::fs::read(&path).map_err(|e| CliError::Io(format!("{path}: {e}")))?;
        raw.split(|&b| b == b'\n')
            .filter(|line| !line.is_empty())
            .map(<[u8]>::to_vec)
            .collect()
    } else {
        resolve_inline_or_stdin(inline)?
    };

    let range = parse_range(&range_arg, records.len())?;
    let slice: Vec<Vec<u8>> = records[range.start..range.end].to_vec();

    // Re-ship the slice through the shared pipeline (DRY: same board→rail→offload→commit as `run`). The
    // record-key is keyed on the index range, so replaying the identical range again **in this process** lands
    // as a once-gate `Duplicate` (no re-commit) — the demonstrable value of replay without a durable manifest.
    let mut pipe = Pipeline::from_spec(&spec)?;
    let mut sink = VecSink::new();
    let mut source = SliceSource::one(slice);
    let record_key = format!("datarail-replay-{}..{}", range.start, range.end);
    let mut delivered = 0usize;
    let mut duplicate = 0usize;
    while let Some(batch) = source.next_batch().map_err(|e| CliError::Rail(e.to_string()))? {
        if let Some(outcome) = pipe.ship_batch(&batch, record_key.as_bytes(), &mut sink)? {
            match outcome.disposition {
                Disposition::Delivered => delivered += outcome.fresh_committed,
                Disposition::Duplicate => duplicate += 1,
                Disposition::DeadLettered => {}
            }
        }
    }

    Ok(format!(
        "replay {}..{} of {} source record(s)\n{}re-committed = {delivered} record(s) (duplicate batches: {duplicate})\n",
        range.start,
        range.end,
        records.len(),
        pipe.report(),
    ))
}

/// The substrate `datarail run` moves cofres over, chosen from the spec's `substrate` field. Single-process
/// `run` uses each substrate's local/loopback flavour (it still exercises the real transport code path); a true
/// two-process run would be a `--source` / `--dest` split (future).
enum AnyRail {
    Loopback(LoopbackSubstrate),
    Tcp(TcpSubstrate),
    Shmem(datarail_substrate_shmem::ShmemRing),
    ObjectStore(datarail_substrate_objectstore::ObjectStoreSubstrate),
    #[cfg(feature = "quic")]
    Quic(Box<datarail_substrate_quic::QuicSubstrate>), // boxed: a QUIC substrate owns a Tokio runtime (large)
}

impl AnyRail {
    fn from_spec(substrate: &str) -> Result<Self, CliError> {
        match substrate {
            "auto" | "loopback" => Ok(Self::Loopback(LoopbackSubstrate::new())),
            "tcp" => Ok(Self::Tcp(TcpSubstrate::loopback_pair().map_err(|e| CliError::Rail(e.to_string()))?)),
            "shmem" => Ok(Self::Shmem(datarail_substrate_shmem::ShmemRing::pair().map_err(|e| CliError::Rail(e.to_string()))?)),
            s if s == "s3" || s == "object-store" || s.starts_with("s3://") => {
                let dir = std::env::temp_dir().join(format!("datarail-run-{}.store", std::process::id()));
                Ok(Self::ObjectStore(
                    datarail_substrate_objectstore::ObjectStoreSubstrate::open(&dir).map_err(|e| CliError::Rail(e.to_string()))?,
                ))
            }
            #[cfg(feature = "quic")]
            "quic" => Ok(Self::Quic(Box::new(
                datarail_substrate_quic::QuicSubstrate::loopback_pair()
                    .map_err(|e| CliError::Rail(e.to_string()))?,
            ))),
            #[cfg(not(feature = "quic"))]
            "quic" => Err(CliError::Rail(
                "substrate \"quic\" requires building the CLI with --features quic".to_owned(),
            )),
            other => Err(CliError::Rail(format!("unknown substrate \"{other}\""))),
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Loopback(_) => "loopback",
            Self::Tcp(_) => "tcp",
            Self::Shmem(_) => "shmem",
            Self::ObjectStore(_) => "object-store",
            #[cfg(feature = "quic")]
            Self::Quic(_) => "quic",
        }
    }

    fn send(&mut self, cofre: &Cofre) -> Result<(), CliError> {
        match self {
            Self::Loopback(s) => s.send(cofre).map_err(|e| match e {}),
            Self::Tcp(s) => s.send(cofre).map_err(|e| CliError::Rail(e.to_string())),
            Self::Shmem(s) => s.send(cofre).map_err(|e| CliError::Rail(e.to_string())),
            Self::ObjectStore(s) => s.send(cofre).map_err(|e| CliError::Rail(e.to_string())),
            #[cfg(feature = "quic")]
            Self::Quic(s) => s.send(cofre).map_err(|e| CliError::Rail(e.to_string())),
        }
    }

    fn recv(&mut self) -> Result<Option<Cofre>, CliError> {
        match self {
            Self::Loopback(s) => s.recv().map_err(|e| match e {}),
            Self::Tcp(s) => s.recv().map_err(|e| CliError::Rail(e.to_string())),
            Self::Shmem(s) => s.recv().map_err(|e| CliError::Rail(e.to_string())),
            Self::ObjectStore(s) => s.recv().map_err(|e| CliError::Rail(e.to_string())),
            #[cfg(feature = "quic")]
            Self::Quic(s) => s.recv().map_err(|e| CliError::Rail(e.to_string())),
        }
    }

    fn ack(&mut self, cofre_id: [u8; 32]) -> Result<(), CliError> {
        match self {
            Self::Loopback(s) => s.ack(cofre_id).map_err(|e| match e {}),
            Self::Tcp(s) => s.ack(cofre_id).map_err(|e| CliError::Rail(e.to_string())),
            Self::Shmem(s) => s.ack(cofre_id).map_err(|e| CliError::Rail(e.to_string())),
            Self::ObjectStore(s) => s.ack(cofre_id).map_err(|e| CliError::Rail(e.to_string())),
            #[cfg(feature = "quic")]
            Self::Quic(s) => s.ack(cofre_id).map_err(|e| CliError::Rail(e.to_string())),
        }
    }
}

/// The outcome of shipping one batch through the [`Pipeline`]: the disposition the offload terminal returned
/// and how many *fresh* records that batch committed to the external sink (0 for a `Duplicate`/`DeadLettered`).
struct BatchOutcome {
    disposition: Disposition,
    fresh_committed: usize,
}

/// A wired route — source + dest terminals over the spec's substrate — that ships one batch at a time.
///
/// This is the single home of the board → move-over-substrate → offload → commit-fresh-records pipeline.
/// Both `datarail run` (streaming source batches) and `datarail replay` (one re-shipped index slice) drive
/// it through [`Pipeline::ship_batch`], so the per-record cofre logic lives in exactly one place (DRY).
struct Pipeline {
    src_term: SourceTerminal,
    dst_term: DestTerminal,
    rail: AnyRail,
    /// Count of records already mirrored from the terminal sink to the external sink (commit cursor).
    committed_to_sink: usize,
    /// Total records handed to `board` (the rail's view; duplicates still board, they dedup at offload).
    boarded: usize,
    /// The wire-hex of the most recently boarded cofre (for the human report).
    last_cofre_hex: String,
}

impl Pipeline {
    /// Build the source + dest terminals and the substrate from `spec`.
    fn from_spec(spec: &RailSpec) -> Result<Self, CliError> {
        let cfg = spec.terminal_config();
        let source_vk = verifying_key(&spec.keys.source_seed);
        let src_term =
            SourceTerminal::new(cfg.clone(), spec.onboarding_contract(), spec.keys.source_seed);
        let dst_term = DestTerminal::new(
            cfg,
            spec.offloading_contract(),
            source_vk,
            spec.keys.dest_seed,
            spec.keys.dest_x25519_secret,
        );
        let rail = AnyRail::from_spec(&spec.route.substrate)?;
        Ok(Self {
            src_term,
            dst_term,
            rail,
            committed_to_sink: 0,
            boarded: 0,
            last_cofre_hex: String::new(),
        })
    }

    /// Board `batch` under the idempotency identity `record_key`, move it over the substrate, offload it, and
    /// commit any freshly-delivered records to `sink`. An empty batch is a no-op. Returns the offload
    /// [`Disposition`] and the number of records this batch committed (so callers can report dedup).
    ///
    /// `record_key` is the sole effectively-once identity (the dest hashes it into the dedup key): board the
    /// *same* `record_key` again and the offload terminal returns [`Disposition::Duplicate`] with no re-commit.
    fn ship_batch(
        &mut self,
        batch: &[Vec<u8>],
        record_key: &[u8],
        sink: &mut dyn Sink,
    ) -> Result<Option<BatchOutcome>, CliError> {
        if batch.is_empty() {
            return Ok(None);
        }
        let refs: Vec<&[u8]> = batch.iter().map(Vec::as_slice).collect();
        let cofre = self
            .src_term
            .board(&refs, record_key)
            .map_err(CliError::Terminal)?;
        self.boarded += refs.len();
        self.last_cofre_hex = to_hex(&datarail_cofre::encode(&cofre));

        self.rail.send(&cofre)?;
        let mut received = None;
        for _ in 0..1_000_000 {
            if let Some(c) = self.rail.recv()? {
                received = Some(c);
                break;
            }
        }
        let received = received.ok_or(CliError::NoCofre)?;
        let disposition = self.dst_term.offload(&received).map_err(CliError::Terminal)?;
        self.rail.ack(received.etiqueta.cofre_id)?;

        // Only Delivered grows the terminal sink; commit those fresh records to the external sink.
        let total = self.dst_term.sink().committed().len();
        let fresh_committed = total - self.committed_to_sink;
        if fresh_committed > 0 {
            let fresh: Vec<Vec<u8>> = self.dst_term.sink().committed()[self.committed_to_sink..].to_vec();
            sink.commit(&fresh).map_err(|e| CliError::Rail(e.to_string()))?;
            self.committed_to_sink = total;
        }
        Ok(Some(BatchOutcome {
            disposition,
            fresh_committed,
        }))
    }

    /// The human-readable end-of-run report (substrate, totals, last cofre).
    fn report(&self) -> String {
        format!(
            "substrate   = {}\nboarded     = {} record(s)\ncommitted   = {}\ndead-letter = {}\nlast cofre  = 0x{}\n",
            self.rail.name(),
            self.boarded,
            self.dst_term.sink().len(),
            self.dst_term.dead_letters().len(),
            self.last_cofre_hex,
        )
    }
}

/// Drive the route: for each source batch, board → move over the spec's substrate → offload → commit the
/// newly-landed records to the sink. The connectors **and** the real substrate are wired end-to-end.
///
/// With `watch`, a live one-line speedometer (elapsed ms, boarded, committed, dead-lettered, records/sec) is
/// printed to stdout after every batch and as a final summary. The flag is purely observational — delivery
/// semantics are byte-for-byte identical with and without it.
fn run_pipe(
    spec: &RailSpec,
    source: &mut dyn Source,
    sink: &mut dyn Sink,
    watch: bool,
) -> Result<String, CliError> {
    let mut pipe = Pipeline::from_spec(spec)?;
    let started = Instant::now();
    let mut batch_no = 0u64;
    while let Some(batch) = source.next_batch().map_err(|e| CliError::Rail(e.to_string()))? {
        // Each streamed batch is a distinct effectively-once identity (`datarail-run-<n>`).
        let record_key = format!("datarail-run-{batch_no}");
        pipe.ship_batch(&batch, record_key.as_bytes(), sink)?;
        batch_no += 1;
        if watch {
            print_speedometer("watch ", &pipe, started.elapsed());
        }
    }
    if watch {
        print_speedometer("final ", &pipe, started.elapsed());
    }
    Ok(pipe.report())
}

/// Print one speedometer line: elapsed ms, records boarded/committed/dead-lettered, and throughput
/// (records/sec). Raw `println!` only — Charter *leveza* forbids a TUI dependency. Throughput is integer
/// `records · 1000 / elapsed_ms`, so the line stays cast-free (no float precision lint to silence).
fn print_speedometer(tag: &str, pipe: &Pipeline, elapsed: std::time::Duration) {
    let elapsed_ms = elapsed.as_millis();
    let committed = pipe.dst_term.sink().len();
    let dead = pipe.dst_term.dead_letters().len();
    let boarded = u128::try_from(pipe.boarded).unwrap_or(u128::MAX);
    let rate = (boarded * 1000).checked_div(elapsed_ms).unwrap_or(0);
    println!(
        "{tag}| {elapsed_ms:>7} ms | boarded {:>6} | committed {committed:>6} | dead {dead:>4} | {rate:>9} rec/s",
        pipe.boarded,
    );
}

fn cmd_verify(rest: &[String]) -> Result<String, CliError> {
    let spec = load_spec(require(rest, 0, "rail.toml")?)?;
    let bytes = from_hex(require(rest, 1, "cofre-hex")?).ok_or(CliError::BadHex)?;
    let cofre = datarail_cofre::decode(&bytes).map_err(|e| CliError::Decode(e.to_string()))?;
    let source_vk = verifying_key(&spec.keys.source_seed);
    match datarail_cofre::verify(&cofre, &source_vk) {
        Ok(()) => Ok(format!(
            "verified ✓\n  cofre_id = 0x{}\n  seq      = {}\n",
            to_hex(&cofre.etiqueta.cofre_id),
            cofre.etiqueta.seq,
        )),
        Err(e) => Err(CliError::SealInvalid(e.to_string())),
    }
}

fn cmd_ticket(rest: &[String]) -> Result<String, CliError> {
    let spec = load_spec(require(rest, 0, "rail.toml")?)?;
    let mut msg = Vec::with_capacity(32);
    msg.extend_from_slice(&spec.route.route_id);
    msg.extend_from_slice(&spec.route.stream_id);
    let sig = sign_domain(ctx::TICKET, &spec.keys.source_seed, &msg);
    Ok(format!(
        "ticket   = \"0x{}\"\nroute_vk = \"0x{}\"\n",
        to_hex(&sig),
        to_hex(&verifying_key(&spec.keys.source_seed)),
    ))
}

/// `datarail pair` — SPEC-08 F3 short-code pairing. With `--connect ADDR` / `--listen ADDR` it runs a **real
/// two-process pairing** over TCP (each side a separate OS process; see [`cmd_pair_remote`]); with neither it
/// runs a local self-test rehearsal of the F2/F3 primitives ([`cmd_pair_local`]).
fn cmd_pair(rest: &[String]) -> Result<String, CliError> {
    let body = rest.to_vec();
    let (connect, r1) = take_flag(&body, "--connect");
    let (listen, r2) = take_flag(&r1, "--listen");
    match (connect, listen) {
        (None, None) => cmd_pair_local(),
        (Some(addr), None) => cmd_pair_remote(&r2, PairRole::Initiator, &addr),
        (None, Some(addr)) => cmd_pair_remote(&r2, PairRole::Responder, &addr),
        (Some(_), Some(_)) => Err(CliError::Rail("give --connect OR --listen, not both".to_owned())),
    }
}

/// Which side of a remote pairing this process is. The `--connect` side is the initiator (A); the `--listen`
/// side is the responder (B). Both bind the static keys in the **same** `(A, B)` order.
#[derive(Clone, Copy)]
enum PairRole {
    Initiator,
    Responder,
}

/// `datarail pair --connect ADDR | --listen ADDR --code <hex> [--my-static <hex>]` — a genuine **two-process**
/// SPEC-08 F3 pairing over TCP. Each side mints/loads its own Noise static keypair, the two exchange their
/// static **public** keys over the socket, bind both into the SPAKE2 transcript, run the PAKE + key-confirmation
/// from the shared short `--code`, and end up with the **authenticated** peer static + an agreed secret. A
/// man-in-the-middle that swaps a static changes the bound identity, so confirmation fails (MAJ-5). The
/// resulting peer static is exactly what you then pin as `--peer-public` for the Noise hop / a `rail.toml`.
fn cmd_pair_remote(rest: &[String], role: PairRole, addr: &str) -> Result<String, CliError> {
    let (code_hex, r1) = take_flag(rest, "--code");
    let code_bytes: [u8; 16] = from_hex(&code_hex.ok_or(CliError::MissingArg("--code <hex>"))?)
        .and_then(|v| v.try_into().ok())
        .ok_or(CliError::BadHex)?;
    let code = ShortCode::from_bytes(code_bytes);
    let (my_static, _r2) = take_flag(&r1, "--my-static");
    let local = match my_static {
        Some(h) => {
            let s: [u8; 32] = from_hex(&h).and_then(|v| v.try_into().ok()).ok_or(CliError::BadHex)?;
            StaticKeypair::from_secret(s)
        }
        None => StaticKeypair::generate().map_err(|e| CliError::Identity(e.to_string()))?,
    };
    let my_pub = local.public();

    // Establish the connection (initiator connects; responder binds + announces + accepts).
    let mut stream = match role {
        PairRole::Initiator => connect_with_retry(addr)?,
        PairRole::Responder => bind_announce_accept(addr)?.0,
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .map_err(|e| CliError::Rail(e.to_string()))?;

    // (0) Exchange static public keys (initiator writes first → no deadlock). Now both know (A_pub, B_pub).
    let (a_pub, b_pub) = match role {
        PairRole::Initiator => {
            write_framed_stream(&mut stream, &my_pub)?;
            let peer = read_framed_array::<32>(&mut stream)?;
            (my_pub, peer)
        }
        PairRole::Responder => {
            let peer = read_framed_array::<32>(&mut stream)?;
            write_framed_stream(&mut stream, &my_pub)?;
            (peer, my_pub)
        }
    };

    // (1) Build the pairing bound to both identities; exchange the PAKE messages.
    let (mut pairing, my_msg) = match role {
        PairRole::Initiator => Pairing::start_initiator(&code, a_pub, b_pub),
        PairRole::Responder => Pairing::start_responder(&code, a_pub, b_pub),
    }
    .map_err(|e| CliError::Identity(e.to_string()))?;
    let peer_msg = exchange(&mut stream, role, &my_msg)?;

    // (2) Derive (always succeeds for a well-formed message); exchange the confirmation tags.
    let my_tag = pairing.derive(&peer_msg).map_err(|e| CliError::Identity(e.to_string()))?;
    let peer_tag = exchange(&mut stream, role, &my_tag)?;

    // (3) Confirm: a wrong code or a swapped (MITM) static makes the tags disagree → failure.
    let secret = pairing.confirm(&peer_tag).map_err(|e| CliError::Identity(e.to_string()))?;

    let (role_name, peer_pub) = match role {
        PairRole::Initiator => ("initiator", b_pub),
        PairRole::Responder => ("responder", a_pub),
    };
    let fpr = blake3_256(&secret);
    Ok(format!(
        "paired ({role_name}) with {addr}\n  peer static = 0x{}\n  shared fpr  = 0x{}   (pin the peer static as --peer-public)\n",
        to_hex(&peer_pub),
        to_hex(&fpr[..8]),
    ))
}

/// One request/response message exchange over the pairing socket, ordered by role to avoid deadlock
/// (initiator writes then reads; responder reads then writes).
fn exchange(stream: &mut TcpStream, role: PairRole, mine: &[u8]) -> Result<Vec<u8>, CliError> {
    match role {
        PairRole::Initiator => {
            write_framed_stream(stream, mine)?;
            read_framed_vec(stream)
        }
        PairRole::Responder => {
            let peer = read_framed_vec(stream)?;
            write_framed_stream(stream, mine)?;
            Ok(peer)
        }
    }
}

/// The local self-test rehearsal of the F2/F3 primitives (no network) — kept so `datarail pair` with no
/// transport flag still demonstrates the identity layer end-to-end in one process.
fn cmd_pair_local() -> Result<String, CliError> {
    use datarail_identity::KkSession;

    // Two endpoints, each with a long-term Noise static keypair (the SPEC-08 X25519 KEM key).
    let a = StaticKeypair::generate().map_err(|e| CliError::Identity(e.to_string()))?;
    let b = StaticKeypair::generate().map_err(|e| CliError::Identity(e.to_string()))?;

    // --- F3: SPAKE2 short-code pairing, identities bound, with confirmation. ---
    let code = ShortCode::generate().map_err(|e| CliError::Identity(e.to_string()))?;
    let (mut pa, msg_a) = Pairing::start_initiator(&code, a.public(), b.public())
        .map_err(|e| CliError::Identity(e.to_string()))?;
    let (mut pb, msg_b) = Pairing::start_responder(&code, a.public(), b.public())
        .map_err(|e| CliError::Identity(e.to_string()))?;
    let tag_a = pa.derive(&msg_b).map_err(|e| CliError::Identity(e.to_string()))?;
    let tag_b = pb.derive(&msg_a).map_err(|e| CliError::Identity(e.to_string()))?;
    let secret_a = pa.confirm(&tag_b).map_err(|e| CliError::Identity(e.to_string()))?;
    let secret_b = pb.confirm(&tag_a).map_err(|e| CliError::Identity(e.to_string()))?;
    if secret_a != secret_b {
        return Err(CliError::Identity("pairing secrets disagreed".to_owned()));
    }

    // --- F2: Noise_KK handshake between the two pinned statics + a sealed round-trip. ---
    let mut init = KkSession::initiator(&a, b.public()).map_err(|e| CliError::Identity(e.to_string()))?;
    let mut resp = KkSession::responder(&b, a.public()).map_err(|e| CliError::Identity(e.to_string()))?;
    let h1 = init.write_handshake(&[]).map_err(|e| CliError::Identity(e.to_string()))?;
    resp.read_handshake(&h1).map_err(|e| CliError::Identity(e.to_string()))?;
    let h2 = resp.write_handshake(&[]).map_err(|e| CliError::Identity(e.to_string()))?;
    init.read_handshake(&h2).map_err(|e| CliError::Identity(e.to_string()))?;
    let mut at = init.into_transport().map_err(|e| CliError::Identity(e.to_string()))?;
    let mut bt = resp.into_transport().map_err(|e| CliError::Identity(e.to_string()))?;
    let ct = at.encrypt(b"datarail-noise-roundtrip").map_err(|e| CliError::Identity(e.to_string()))?;
    let pt = bt.decrypt(&ct).map_err(|e| CliError::Identity(e.to_string()))?;
    if pt != b"datarail-noise-roundtrip" {
        return Err(CliError::Identity("noise round-trip mismatch".to_owned()));
    }

    Ok(format!(
        "pair (local rehearsal — not a remote pairing)\n  endpoint A static = 0x{}\n  endpoint B static = 0x{}\n  F3 PAKE     = ok (shared secret agreed, identities bound)\n  F2 Noise_KK = ok (mutual-static handshake + sealed round-trip)\n",
        to_hex(&a.public()),
        to_hex(&b.public()),
    ))
}

/// Pull an optional `--flag <value>` from `rest`, returning the (owned) value and the remaining positionals.
/// Owned returns so successive calls can chain (`let (v, rest) = take_flag(&rest, …)`) without lifetime knots.
fn take_flag(rest: &[String], flag: &str) -> (Option<String>, Vec<String>) {
    let mut value = None;
    let mut positionals = Vec::new();
    let mut i = 0;
    while i < rest.len() {
        if rest[i] == flag {
            value = rest.get(i + 1).cloned();
            i += 2;
        } else {
            positionals.push(rest[i].clone());
            i += 1;
        }
    }
    (value, positionals)
}

/// `datarail send <rail.toml> --connect ADDR [--source-file F | record ...]` — the cross-process **source**.
///
/// Boards the resolved source records into one sealed cofre and ships it over a **real TCP socket** to a
/// `datarail recv` listening at `ADDR`. A separate OS process from the destination — this is the genuine
/// cross-host source side (loopback `ADDR` for a same-host demo; any reachable address otherwise). The cofre is
/// sealed end-to-end, so the TCP hop is a dumb pipe (`INV-OPAQUE-CARGO`).
fn cmd_send(rest: &[String]) -> Result<String, CliError> {
    let spec = load_spec(require(rest, 0, "rail.toml")?)?;
    let body: Vec<String> = rest.get(1..).unwrap_or(&[]).to_vec();
    let (connect, r1) = take_flag(&body, "--connect");
    let addr = connect.ok_or(CliError::MissingArg("--connect ADDR"))?;
    let (noise_secret, r2) = take_flag(&r1, "--noise-secret");
    let (peer_public, r3) = take_flag(&r2, "--peer-public");
    let (source_file, positionals) = take_flag(&r3, "--source-file");
    let noise = parse_noise(noise_secret, peer_public)?;

    let records: Vec<Vec<u8>> = if let Some(path) = source_file {
        let raw = std::fs::read(&path).map_err(|e| CliError::Io(format!("{path}: {e}")))?;
        raw.split(|&b| b == b'\n').filter(|l| !l.is_empty()).map(<[u8]>::to_vec).collect()
    } else {
        resolve_inline_or_stdin(positionals.iter().map(|s| s.as_bytes().to_vec()).collect())?
    };
    if records.is_empty() {
        return Err(CliError::Rail("nothing to send (empty source)".to_owned()));
    }

    let mut src = SourceTerminal::new(spec.terminal_config(), spec.onboarding_contract(), spec.keys.source_seed);
    let refs: Vec<&[u8]> = records.iter().map(Vec::as_slice).collect();
    let cofre = src.board(&refs, b"datarail-send").map_err(CliError::Terminal)?;

    // Connect with a short retry window so `send` can be launched ~concurrently with `recv`; then build the hop
    // (a Noise_KK-protected channel when --noise-secret/--peer-public are given, else plain TCP).
    let stream = connect_with_retry(&addr)?;
    let hop = if noise.is_some() { "noise" } else { "tcp" };
    let mut rail = build_hop(stream, noise, true)?;
    rail.send(&cofre).map_err(|e| CliError::Rail(e.to_string()))?;
    // Hold the connection briefly so the kernel flushes the framed cofre before the socket closes.
    std::thread::sleep(std::time::Duration::from_millis(100));

    Ok(format!(
        "sent {} record(s) in 1 sealed cofre to {addr} over {hop}\n  cofre_id = 0x{}\n",
        records.len(),
        to_hex(&cofre.etiqueta.cofre_id),
    ))
}

/// Parse the optional `--noise-secret`/`--peer-public` pair into a local static keypair + the pinned peer
/// public. Both flags must be present together (a Noise hop needs both), or neither (plain TCP).
fn parse_noise(
    secret: Option<String>,
    peer: Option<String>,
) -> Result<Option<(StaticKeypair, [u8; 32])>, CliError> {
    match (secret, peer) {
        (None, None) => Ok(None),
        (Some(s), Some(p)) => {
            let sk: [u8; 32] = from_hex(&s).and_then(|v| v.try_into().ok()).ok_or(CliError::BadHex)?;
            let pk: [u8; 32] = from_hex(&p).and_then(|v| v.try_into().ok()).ok_or(CliError::BadHex)?;
            Ok(Some((StaticKeypair::from_secret(sk), pk)))
        }
        _ => Err(CliError::Rail(
            "--noise-secret and --peer-public must be given together".to_owned(),
        )),
    }
}

/// Build the transport hop over `stream`: a `Noise_KK`-protected channel when `noise` is `Some`, else plain
/// TCP. `initiator` picks the handshake role (send = initiator, recv = responder). Boxed behind the
/// [`Substrate`] trait so the caller drives either uniformly.
fn build_hop(
    stream: TcpStream,
    noise: Option<(StaticKeypair, [u8; 32])>,
    initiator: bool,
) -> Result<Box<dyn Substrate<Error = std::io::Error>>, CliError> {
    match noise {
        Some((local, peer)) => {
            let sub = if initiator {
                NoiseSubstrate::initiator(stream, &local, peer)
            } else {
                NoiseSubstrate::responder(stream, &local, peer)
            };
            Ok(Box::new(sub.map_err(|e| CliError::Rail(e.to_string()))?))
        }
        None => Ok(Box::new(
            TcpSubstrate::from_stream(stream).map_err(|e| CliError::Rail(e.to_string()))?,
        )),
    }
}

/// Connect to `addr`, retrying briefly (the peer `recv` may still be binding). Returns the raw stream so the
/// caller can wrap it in the chosen hop.
fn connect_with_retry(addr: &str) -> Result<TcpStream, CliError> {
    let mut last = String::new();
    for _ in 0..50 {
        match TcpStream::connect(addr) {
            Ok(s) => return Ok(s),
            Err(e) => {
                last = e.to_string();
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }
    Err(CliError::Rail(format!("could not connect to {addr}: {last}")))
}

/// Bind `listen`, announce the bound address (`DATARAIL-LISTENING <addr>`, flushed, so a peer/orchestrator can
/// discover the OS-assigned port), and bounded-accept one connection (non-blocking poll + 30s deadline so it
/// never hangs). Shared by `datarail recv` and `datarail pair --listen`.
fn bind_announce_accept(listen: &str) -> Result<(TcpStream, std::net::SocketAddr), CliError> {
    use std::io::Write as _;
    let listener = TcpListener::bind(listen).map_err(|e| CliError::Rail(format!("{listen}: {e}")))?;
    let bound = listener.local_addr().map_err(|e| CliError::Rail(e.to_string()))?;
    println!("DATARAIL-LISTENING {bound}");
    std::io::stdout().flush().map_err(|e| CliError::Io(e.to_string()))?;
    listener.set_nonblocking(true).map_err(|e| CliError::Rail(e.to_string()))?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match listener.accept() {
            Ok((s, _peer)) => return Ok((s, bound)),
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err(CliError::Rail("timed out waiting for a peer".to_owned()));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(CliError::Rail(e.to_string())),
        }
    }
}

/// Write a `u32`-length-prefixed frame to a pairing socket.
fn write_framed_stream(s: &mut TcpStream, bytes: &[u8]) -> Result<(), CliError> {
    use std::io::Write as _;
    let len = u32::try_from(bytes.len()).map_err(|_| CliError::Rail("frame too large".to_owned()))?;
    s.write_all(&len.to_le_bytes())
        .and_then(|()| s.write_all(bytes))
        .and_then(|()| s.flush())
        .map_err(|e| CliError::Rail(e.to_string()))
}

/// Fill `buf` exactly from the pairing socket, tolerating short reads / timeouts within a 30s deadline.
fn read_exact_dl(s: &mut TcpStream, buf: &mut [u8]) -> Result<(), CliError> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut filled = 0;
    while filled < buf.len() {
        match s.read(&mut buf[filled..]) {
            Ok(0) => return Err(CliError::Rail("peer closed during pairing".to_owned())),
            Ok(n) => filled += n,
            Err(ref e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                if Instant::now() >= deadline {
                    return Err(CliError::Rail("pairing read timed out".to_owned()));
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(CliError::Rail(e.to_string())),
        }
    }
    Ok(())
}

/// Read one `u32`-length-prefixed frame from a pairing socket (pairing messages are small; cap at 4 KiB).
fn read_framed_vec(s: &mut TcpStream) -> Result<Vec<u8>, CliError> {
    let mut hdr = [0u8; 4];
    read_exact_dl(s, &mut hdr)?;
    let len = u32::from_le_bytes(hdr) as usize;
    if len > 4096 {
        return Err(CliError::Rail("oversized pairing frame".to_owned()));
    }
    let mut body = vec![0u8; len];
    read_exact_dl(s, &mut body)?;
    Ok(body)
}

/// Read one length-prefixed frame and require it to be exactly `N` bytes.
fn read_framed_array<const N: usize>(s: &mut TcpStream) -> Result<[u8; N], CliError> {
    read_framed_vec(s)?
        .try_into()
        .map_err(|_| CliError::Rail("unexpected pairing frame length".to_owned()))
}

/// `datarail recv <rail.toml> [--listen ADDR] [--sink-file F] [--count N]` — the cross-process **destination**.
///
/// Binds a TCP listener (default `127.0.0.1:0`, an OS-assigned port), prints its bound address as
/// `DATARAIL-LISTENING <addr>` (so a peer/orchestrator can discover it), accepts one `datarail send`
/// connection, then verifies → opens → offloads `N` cofres (default 1) and commits the delivered records to the
/// sink. A separate OS process from the source — the genuine cross-host destination side.
fn cmd_recv(rest: &[String]) -> Result<String, CliError> {
    let spec = load_spec(require(rest, 0, "rail.toml")?)?;
    let body: Vec<String> = rest.get(1..).unwrap_or(&[]).to_vec();
    let (listen, r1) = take_flag(&body, "--listen");
    let listen = listen.unwrap_or_else(|| "127.0.0.1:0".to_owned());
    let (noise_secret, r2) = take_flag(&r1, "--noise-secret");
    let (peer_public, r3) = take_flag(&r2, "--peer-public");
    let (sink_file, r4) = take_flag(&r3, "--sink-file");
    let (count, _r5) = take_flag(&r4, "--count");
    let noise = parse_noise(noise_secret, peer_public)?;
    let count: usize = match count {
        Some(c) => c.parse().map_err(|_| CliError::BadRange(c.clone()))?,
        None => 1,
    };

    let source_vk = verifying_key(&spec.keys.source_seed);
    let mut dst = DestTerminal::new(
        spec.terminal_config(),
        spec.offloading_contract(),
        source_vk,
        spec.keys.dest_seed,
        spec.keys.dest_x25519_secret,
    );
    let mut sink: Box<dyn Sink> = match sink_file {
        Some(path) => Box::new(LineFileSink::create(path).map_err(|e| CliError::Io(e.to_string()))?),
        None => Box::new(VecSink::new()),
    };

    let (stream, bound) = bind_announce_accept(&listen)?;
    let hop = if noise.is_some() { "noise" } else { "tcp" };
    let mut rail = build_hop(stream, noise, false)?;

    let mut committed_to_sink = 0usize;
    let mut received = 0usize;
    let recv_deadline = Instant::now() + std::time::Duration::from_secs(30);
    while received < count {
        match rail.recv().map_err(|e| CliError::Rail(e.to_string()))? {
            Some(cofre) => {
                let _ = dst.offload(&cofre).map_err(CliError::Terminal)?;
                rail.ack(cofre.etiqueta.cofre_id).map_err(|e| CliError::Rail(e.to_string()))?;
                received += 1;
                let total = dst.sink().committed().len();
                if total > committed_to_sink {
                    let fresh: Vec<Vec<u8>> = dst.sink().committed()[committed_to_sink..].to_vec();
                    sink.commit(&fresh).map_err(|e| CliError::Rail(e.to_string()))?;
                    committed_to_sink = total;
                }
            }
            None => {
                if Instant::now() >= recv_deadline {
                    return Err(CliError::Rail("timed out waiting for cofres".to_owned()));
                }
            }
        }
    }

    Ok(format!(
        "recv on {bound} over {hop}\n  received    = {received} cofre(s)\n  committed   = {}\n  dead-letter = {}\n",
        dst.sink().len(),
        dst.dead_letters().len(),
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        cmd_pair, dispatch, from_hex, parse_range, run_pipe, to_hex, AnyRail, Pipeline, SliceSource,
        VecSink,
    };
    use datarail_connectors::Source;
    use datarail_core::Disposition;
    use datarail_crypto::verifying_key;
    use datarail_spec::RailSpec;
    use datarail_terminal::SourceTerminal;

    const K32: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";

    fn sample() -> String {
        format!(
            "[route]\n\
             route_id = \"0x01010101010101010101010101010101\"\n\
             stream_id = \"0x02020202020202020202020202020202\"\n\
             aead = \"gcm-siv-256\"\n\
             guarantee = \"exactly-once\"\n\
             [onboarding]\n\
             max_record_len = 1024\n\
             required_prefix = \"evt:\"\n\
             [offloading]\n\
             max_record_len = 1024\n\
             required_prefix = \"evt:\"\n\
             [keys]\n\
             source_seed = \"{K32}\"\n\
             dest_seed = \"{K32}\"\n\
             dest_x25519_secret = \"{K32}\"\n\
             tenant_secret = \"{K32}\"\n"
        )
    }

    #[test]
    fn pair_local_rehearsal_succeeds_end_to_end() {
        // The identity layer (F2 Noise_KK + F3 PAKE) is reachable from the CLI and works end-to-end: the
        // command runs both sides locally and reports success for each. (This also keeps datarail-identity a
        // real, exercised dependency — not an orphan crate.)
        let out = cmd_pair(&[]).expect("local pairing rehearsal should succeed");
        assert!(out.contains("F3 PAKE     = ok"), "{out}");
        assert!(out.contains("F2 Noise_KK = ok"), "{out}");
    }

    #[test]
    fn run_pipe_delivers_a_conforming_batch_through_the_connectors() {
        let spec = RailSpec::parse(&sample()).unwrap();
        let mut source = SliceSource::one(vec![b"evt:a".to_vec(), b"evt:b".to_vec()]);
        let mut sink = VecSink::new();
        let out = run_pipe(&spec, &mut source, &mut sink, false).unwrap();
        assert!(out.contains("substrate   = loopback"), "{out}");
        assert!(out.contains("committed   = 2"), "{out}");
        assert!(out.contains("dead-letter = 0"), "{out}");
        // The records actually landed in the sink connector (end-to-end through board→rail→offload→commit).
        assert_eq!(sink.committed(), &[b"evt:a".to_vec(), b"evt:b".to_vec()]);
    }

    #[test]
    fn run_pipe_refuses_a_contract_violator_at_boarding() {
        let spec = RailSpec::parse(&sample()).unwrap();
        // A record without the "evt:" prefix never boards (AC-9 onboarding side).
        let mut source = SliceSource::one(vec![b"nope".to_vec()]);
        let mut sink = VecSink::new();
        assert!(run_pipe(&spec, &mut source, &mut sink, false).is_err());
    }

    #[test]
    fn substrate_field_selects_the_real_substrate() {
        assert!(matches!(AnyRail::from_spec("loopback").unwrap(), AnyRail::Loopback(_)));
        assert!(matches!(AnyRail::from_spec("auto").unwrap(), AnyRail::Loopback(_)));
        assert!(matches!(AnyRail::from_spec("tcp").unwrap(), AnyRail::Tcp(_)));
        assert!(matches!(AnyRail::from_spec("shmem").unwrap(), AnyRail::Shmem(_)));
        assert!(matches!(AnyRail::from_spec("s3").unwrap(), AnyRail::ObjectStore(_)));
        assert!(AnyRail::from_spec("bogus").is_err());
    }

    #[test]
    fn board_encode_hex_decode_verify_round_trips() {
        let spec = RailSpec::parse(&sample()).unwrap();
        let mut src = SourceTerminal::new(
            spec.terminal_config(),
            spec.onboarding_contract(),
            spec.keys.source_seed,
        );
        let recs: [&[u8]; 1] = [b"evt:x"];
        let cofre = src.board(&recs, b"k").unwrap();
        let hex = to_hex(&datarail_cofre::encode(&cofre));
        let decoded = datarail_cofre::decode(&from_hex(&hex).unwrap()).unwrap();
        let vk = verifying_key(&spec.keys.source_seed);
        assert!(datarail_cofre::verify(&decoded, &vk).is_ok());
    }

    #[test]
    fn hex_round_trips() {
        assert_eq!(from_hex("0xdeadbeef").unwrap(), vec![0xde, 0xad, 0xbe, 0xef]);
        assert_eq!(to_hex(&[0xde, 0xad, 0xbe, 0xef]), "deadbeef");
        assert!(from_hex("xyz").is_none());
        assert!(from_hex("0d0").is_none());
    }

    #[test]
    fn unknown_and_missing_commands_error() {
        assert!(dispatch(&["frobnicate".to_owned()]).is_err());
        assert!(dispatch(&[]).is_err());
        assert!(dispatch(&["validate".to_owned()]).is_err()); // missing path
    }

    /// Ship one batch of `records` under `key` through `pipe`/`sink` and return its disposition.
    fn ship_once(pipe: &mut Pipeline, sink: &mut VecSink, records: Vec<Vec<u8>>, key: &[u8]) -> Disposition {
        let mut source = SliceSource::one(records);
        let batch = source.next_batch().unwrap().unwrap();
        pipe.ship_batch(&batch, key, sink).unwrap().unwrap().disposition
    }

    #[test]
    fn parse_range_accepts_exclusive_inclusive_and_all() {
        assert!(matches!(parse_range("1..3", 5).unwrap(), super::RecordRange { start: 1, end: 3 }));
        assert!(matches!(parse_range("1..=3", 5).unwrap(), super::RecordRange { start: 1, end: 4 }));
        assert!(matches!(parse_range("all", 5).unwrap(), super::RecordRange { start: 0, end: 5 }));
        // Empty range is valid (start == end).
        assert!(matches!(parse_range("2..2", 5).unwrap(), super::RecordRange { start: 2, end: 2 }));
    }

    #[test]
    fn parse_range_rejects_garbage_reversed_and_out_of_bounds() {
        assert!(parse_range("garbage", 5).is_err());
        assert!(parse_range("1..", 5).is_err()); // empty rhs
        assert!(parse_range("..3", 5).is_err()); // empty lhs
        assert!(parse_range("3..1", 5).is_err()); // reversed
        assert!(parse_range("0..6", 5).is_err()); // end past len
        assert!(parse_range("0..=5", 5).is_err()); // inclusive end past last index
        assert!(parse_range("x..y", 5).is_err()); // non-numeric
    }

    #[test]
    fn replay_subrange_commits_exactly_that_slice() {
        // A 5-record source; replay [1..3) must land records 1 and 2 and nothing else.
        let spec = RailSpec::parse(&sample()).unwrap();
        let all: Vec<Vec<u8>> = (0..5).map(|n| format!("evt:r{n}").into_bytes()).collect();
        let range = parse_range("1..3", all.len()).unwrap();
        let slice = all[range.start..range.end].to_vec();

        let mut pipe = Pipeline::from_spec(&spec).unwrap();
        let mut sink = VecSink::new();
        let disp = ship_once(&mut pipe, &mut sink, slice, b"datarail-replay-1..3");

        assert_eq!(disp, Disposition::Delivered);
        assert_eq!(sink.committed(), &[b"evt:r1".to_vec(), b"evt:r2".to_vec()]);
        assert_eq!(pipe.dst_term.sink().len(), 2);
        assert_eq!(pipe.dst_term.dead_letters().len(), 0);
    }

    #[test]
    fn replaying_the_same_slice_twice_in_one_process_recommits_nothing() {
        // The once-gate: re-shipping an already-delivered slice through the SAME dest terminal is a Duplicate
        // (0 re-commit). Both passes go through one `Pipeline`, demonstrating in-process dedup.
        let spec = RailSpec::parse(&sample()).unwrap();
        let slice = vec![b"evt:r1".to_vec(), b"evt:r2".to_vec()];

        let mut pipe = Pipeline::from_spec(&spec).unwrap();
        let mut sink = VecSink::new();

        // SAME record-key both passes — that is the effectively-once identity the once-gate dedups on.
        let key = b"datarail-replay-0..2";
        let first = ship_once(&mut pipe, &mut sink, slice.clone(), key);
        assert_eq!(first, Disposition::Delivered);
        assert_eq!(sink.committed().len(), 2);

        let second = ship_once(&mut pipe, &mut sink, slice, key);
        assert_eq!(second, Disposition::Duplicate);
        // Nothing new committed on the replay: the sink connector and terminal sink both stay at 2.
        assert_eq!(sink.committed().len(), 2);
        assert_eq!(pipe.dst_term.sink().len(), 2);
    }

    #[test]
    fn cmd_replay_malformed_range_errors() {
        // Inline records so `replay` does not block on stdin; the bad range must surface as Err, not a panic.
        let toml = std::env::temp_dir().join(format!("datarail-replay-bad-{}.toml", std::process::id()));
        std::fs::write(&toml, sample()).unwrap();
        let path = toml.to_string_lossy().into_owned();
        let args = vec![path.clone(), "not-a-range".to_owned(), "evt:a".to_owned(), "evt:b".to_owned()];
        assert!(super::cmd_replay(&args).is_err());
        let _ = std::fs::remove_file(&toml);
    }

    #[test]
    fn cmd_replay_subrange_through_the_real_command() {
        // End-to-end through `cmd_replay`: inline 4 records, replay [1..3), report must show 2 re-committed.
        let toml = std::env::temp_dir().join(format!("datarail-replay-ok-{}.toml", std::process::id()));
        std::fs::write(&toml, sample()).unwrap();
        let path = toml.to_string_lossy().into_owned();
        let args = vec![
            path,
            "1..3".to_owned(),
            "evt:a".to_owned(),
            "evt:b".to_owned(),
            "evt:c".to_owned(),
            "evt:d".to_owned(),
        ];
        let out = super::cmd_replay(&args).unwrap();
        assert!(out.contains("replay 1..3 of 4 source record(s)"), "{out}");
        assert!(out.contains("re-committed = 2 record(s)"), "{out}");
        assert!(out.contains("committed   = 2"), "{out}");
        let _ = std::fs::remove_file(toml);
    }

    #[test]
    fn watch_run_delivers_identically_to_a_plain_run() {
        // `--watch` is observational: a watched run commits exactly the same records as an unwatched one.
        let spec = RailSpec::parse(&sample()).unwrap();
        let recs = vec![b"evt:a".to_vec(), b"evt:b".to_vec(), b"evt:c".to_vec()];

        let mut plain_src = SliceSource::one(recs.clone());
        let mut plain_sink = VecSink::new();
        run_pipe(&spec, &mut plain_src, &mut plain_sink, false).unwrap();

        let mut watch_src = SliceSource::one(recs);
        let mut watch_sink = VecSink::new();
        let watched = run_pipe(&spec, &mut watch_src, &mut watch_sink, true).unwrap();

        assert!(watched.contains("committed   = 3"), "{watched}");
        assert_eq!(watch_sink.committed().len(), 3);
        assert_eq!(watch_sink.committed(), plain_sink.committed());
    }
}
