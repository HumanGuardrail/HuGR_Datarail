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

use datarail_core::Substrate;
use datarail_crypto::{ctx, sign_domain, verifying_key};
use datarail_rail::LoopbackSubstrate;
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

    // Records: trailing CLI args, else stdin lines, else a built-in conforming demo.
    let mut records: Vec<Vec<u8>> = rest[1..].iter().map(|s| s.as_bytes().to_vec()).collect();
    if records.is_empty() {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| CliError::Io(e.to_string()))?;
        records = buf
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| l.as_bytes().to_vec())
            .collect();
    }
    if records.is_empty() {
        records = vec![b"evt:hello".to_vec(), b"evt:world".to_vec()];
    }

    run_pipe(&spec, &records, b"datarail-run")
}

/// Board → loopback rail → offload one batch, returning a human-readable report (and the cofre hex, so the
/// `verify` subcommand can be demonstrated on the same artifact).
fn run_pipe(spec: &RailSpec, records: &[Vec<u8>], record_key: &[u8]) -> Result<String, CliError> {
    let cfg = spec.terminal_config();
    let source_vk = verifying_key(&spec.keys.source_seed);
    let mut source =
        SourceTerminal::new(cfg.clone(), spec.onboarding_contract(), spec.keys.source_seed);
    let mut dest = DestTerminal::new(
        cfg,
        spec.offloading_contract(),
        source_vk,
        spec.keys.dest_seed,
        spec.keys.dest_x25519_secret,
    );
    let mut rail = LoopbackSubstrate::new();

    let recs: Vec<&[u8]> = records.iter().map(Vec::as_slice).collect();
    let cofre = source.board(&recs, record_key).map_err(CliError::Terminal)?;
    let cofre_hex = to_hex(&datarail_cofre::encode(&cofre));

    rail.send(&cofre).map_err(|e| CliError::Rail(e.to_string()))?;
    let received = rail
        .recv()
        .map_err(|e| CliError::Rail(e.to_string()))?
        .ok_or(CliError::NoCofre)?;
    let disposition = dest.offload(&received).map_err(CliError::Terminal)?;

    Ok(format!(
        "boarded {} record(s)\n  cofre_id    = 0x{}\n  disposition = {:?}\n  committed   = {}\n  dead-letter = {}\n  cofre       = 0x{cofre_hex}\n",
        records.len(),
        to_hex(&cofre.etiqueta.cofre_id),
        disposition,
        dest.sink().len(),
        dest.dead_letters().len(),
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
    use super::{dispatch, from_hex, run_pipe, to_hex};
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
    fn run_pipe_delivers_a_conforming_batch() {
        let spec = RailSpec::parse(&sample()).unwrap();
        let out = run_pipe(&spec, &[b"evt:a".to_vec(), b"evt:b".to_vec()], b"k").unwrap();
        assert!(out.contains("disposition = Delivered"), "{out}");
        assert!(out.contains("committed   = 2"), "{out}");
        assert!(out.contains("dead-letter = 0"), "{out}");
    }

    #[test]
    fn run_pipe_refuses_a_contract_violator_at_boarding() {
        let spec = RailSpec::parse(&sample()).unwrap();
        // A record without the "evt:" prefix never boards (AC-9 onboarding side).
        assert!(run_pipe(&spec, &[b"nope".to_vec()], b"k").is_err());
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
