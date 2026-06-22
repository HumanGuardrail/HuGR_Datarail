//! `datarail` — the command-line surface (SPEC `09-spec-cli.md`).
//!
//! Subcommands:
//! - `validate <rail.toml>` — parse + validate a route spec.
//! - `keygen` — emit a fresh Ed25519 seed + verifying key (hex).
//! - `run <rail.toml> [record …]` — board → loopback rail → offload a batch; report the outcome
//!   (records also read from stdin, one per line; a built-in demo if none).
//! - `verify <rail.toml> <hex>` — verify a wire-encoded cofre's seal against the route's source key.
//! - `ticket <rail.toml>` — emit a signed route-capability ticket (hex).
//!
//! Lean by design (Charter *leveza*): hand-rolled argument handling (no `clap`) and `/dev/urandom` for keygen
//! (no `rand`). The CLI holds no logic of its own beyond wiring the crates together.

#![forbid(unsafe_code)]

use std::io::Read;
use std::process::ExitCode;

use datarail_connectors::{LineFileSink, LineFileSource, Sink, SliceSource, Source, VecSink};
use datarail_core::{Cofre, Substrate};
use datarail_crypto::{ctx, sign_domain, verifying_key};
use datarail_rail::{LoopbackSubstrate, TcpSubstrate};
use datarail_spec::{RailSpec, SpecError};
use datarail_terminal::{DestTerminal, SourceTerminal, TerminalError};

const USAGE: &str = "\
datarail — provider-blind data rail (SPEC 09)

USAGE:
    datarail validate <rail.toml>
    datarail keygen
    datarail run <rail.toml> [record ...]    (records also read from stdin, one per line)
    datarail verify <rail.toml> <cofre-hex>
    datarail ticket <rail.toml>
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
        "keygen" => cmd_keygen(),
        "run" => cmd_run(rest),
        "verify" => cmd_verify(rest),
        "ticket" => cmd_ticket(rest),
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
    Io(String),
    Spec(SpecError),
    BadHex,
    Decode(String),
    SealInvalid(String),
    Terminal(TerminalError),
    Rail(String),
    NoCofre,
}

impl core::fmt::Display for CliError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Usage => write!(f, "no command given\n\n{USAGE}"),
            Self::UnknownCommand(c) => write!(f, "unknown command `{c}`\n\n{USAGE}"),
            Self::MissingArg(a) => write!(f, "missing argument <{a}>"),
            Self::Io(e) => write!(f, "io: {e}"),
            Self::Spec(e) => write!(f, "spec: {e}"),
            Self::BadHex => write!(f, "argument is not valid hex"),
            Self::Decode(e) => write!(f, "cofre decode: {e}"),
            Self::SealInvalid(e) => write!(f, "seal INVALID: {e}"),
            Self::Terminal(e) => write!(f, "terminal: {e}"),
            Self::Rail(e) => write!(f, "rail: {e}"),
            Self::NoCofre => write!(f, "no cofre came off the rail"),
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

fn cmd_keygen() -> Result<String, CliError> {
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

    // Optional `--source-file` / `--sink-file`; remaining args are inline records.
    let mut source_file: Option<String> = None;
    let mut sink_file: Option<String> = None;
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
            other => {
                inline.push(other.as_bytes().to_vec());
                i += 1;
            }
        }
    }

    // Source connector: `--source-file` > inline args > stdin lines > a built-in conforming demo.
    let mut source: Box<dyn Source> = if let Some(path) = source_file {
        Box::new(LineFileSource::open(&path).map_err(|e| CliError::Io(e.to_string()))?)
    } else {
        let mut records = inline;
        if records.is_empty() {
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .map_err(|e| CliError::Io(e.to_string()))?;
            records = buf.lines().filter(|l| !l.is_empty()).map(|l| l.as_bytes().to_vec()).collect();
        }
        if records.is_empty() {
            records = vec![b"evt:hello".to_vec(), b"evt:world".to_vec()];
        }
        Box::new(SliceSource::one(records))
    };

    // Sink connector: `--sink-file` (append as lines) > in-memory.
    let mut sink: Box<dyn Sink> = match sink_file {
        Some(path) => Box::new(LineFileSink::create(&path).map_err(|e| CliError::Io(e.to_string()))?),
        None => Box::new(VecSink::new()),
    };

    run_pipe(&spec, source.as_mut(), sink.as_mut())
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

/// Drive the route: for each source batch, board → move over the spec's substrate → offload → commit the
/// newly-landed records to the sink. The connectors **and** the real substrate are now wired end-to-end.
fn run_pipe(spec: &RailSpec, source: &mut dyn Source, sink: &mut dyn Sink) -> Result<String, CliError> {
    let cfg = spec.terminal_config();
    let source_vk = verifying_key(&spec.keys.source_seed);
    let mut src_term =
        SourceTerminal::new(cfg.clone(), spec.onboarding_contract(), spec.keys.source_seed);
    let mut dst_term = DestTerminal::new(
        cfg,
        spec.offloading_contract(),
        source_vk,
        spec.keys.dest_seed,
        spec.keys.dest_x25519_secret,
    );
    let mut rail = AnyRail::from_spec(&spec.route.substrate)?;

    let mut boarded = 0usize;
    let mut committed_to_sink = 0usize;
    let mut last_cofre_hex = String::new();
    let mut batch_no = 0u64;
    while let Some(batch) = source.next_batch().map_err(|e| CliError::Rail(e.to_string()))? {
        if batch.is_empty() {
            batch_no += 1;
            continue;
        }
        let refs: Vec<&[u8]> = batch.iter().map(Vec::as_slice).collect();
        let record_key = format!("datarail-run-{batch_no}");
        let cofre = src_term
            .board(&refs, record_key.as_bytes())
            .map_err(CliError::Terminal)?;
        boarded += refs.len();
        last_cofre_hex = to_hex(&datarail_cofre::encode(&cofre));

        rail.send(&cofre)?;
        let mut received = None;
        for _ in 0..1_000_000 {
            if let Some(c) = rail.recv()? {
                received = Some(c);
                break;
            }
        }
        let received = received.ok_or(CliError::NoCofre)?;
        let _ = dst_term.offload(&received).map_err(CliError::Terminal)?;
        rail.ack(received.etiqueta.cofre_id)?;

        // Only Delivered grows the terminal sink; commit those fresh records to the external sink.
        let total = dst_term.sink().committed().len();
        if total > committed_to_sink {
            let fresh: Vec<Vec<u8>> = dst_term.sink().committed()[committed_to_sink..].to_vec();
            sink.commit(&fresh).map_err(|e| CliError::Rail(e.to_string()))?;
            committed_to_sink = total;
        }
        batch_no += 1;
    }

    Ok(format!(
        "substrate   = {}\nboarded     = {boarded} record(s)\ncommitted   = {}\ndead-letter = {}\nlast cofre  = 0x{last_cofre_hex}\n",
        rail.name(),
        dst_term.sink().len(),
        dst_term.dead_letters().len(),
    ))
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

#[cfg(test)]
mod tests {
    use super::{dispatch, from_hex, run_pipe, to_hex, AnyRail, SliceSource, VecSink};
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
    fn run_pipe_delivers_a_conforming_batch_through_the_connectors() {
        let spec = RailSpec::parse(&sample()).unwrap();
        let mut source = SliceSource::one(vec![b"evt:a".to_vec(), b"evt:b".to_vec()]);
        let mut sink = VecSink::new();
        let out = run_pipe(&spec, &mut source, &mut sink).unwrap();
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
        assert!(run_pipe(&spec, &mut source, &mut sink).is_err());
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
}
