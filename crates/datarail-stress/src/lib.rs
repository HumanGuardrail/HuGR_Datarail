//! `datarail-stress` — the **violent resilience / stress suite** for the sealed rail.
//!
//! This crate is a pure test harness. It does not add product behavior; it *assaults* the rail across its five
//! flagship use cases and asserts the three rail invariants hold under fire:
//!
//! - **0-loss** — every conforming, authentic record that should be delivered IS delivered (none dropped);
//! - **0-dup** — nothing is committed to a destination sink twice (the effectively-once gate holds);
//! - **0-leak** — nothing forged, tampered, garbage, or out-of-contract ever reaches a sink; every bad input is
//!   reason-coded onto the dead-letter siding (or rejected by the substrate's parse-before-verify), never committed.
//!
//! The flagship scenarios live one-per-file under `tests/` (Stream/CDC, cross-container shmem, untrusted
//! object-store, store-and-forward, WAN kill-mid-stream) plus a cross-cutting forgery flood. This `lib`
//! supplies the shared fixtures, the source/destination [`Rig`], the [`Tally`], and a few small assault
//! primitives (byte-tampering, forged-signer cofres) so each scenario file stays focused on its assault.
//!
//! Everything here is `#![forbid(unsafe_code)]` and lint-clean to the workspace charter; the helpers are
//! infallible or return `Result`, so the harness never panics in non-test code.

#![forbid(unsafe_code)]

use datarail_core::{AeadAlg, Cofre, Disposition};
use datarail_crypto::{verifying_key, x25519_public};
use datarail_terminal::{ContentContract, DestTerminal, SourceTerminal, TerminalConfig};

/// The route's Ed25519 **source** signing seed (the pinned, authentic sender identity).
pub const SOURCE_SEED: [u8; 32] = [11u8; 32];
/// The destination's Ed25519 watermark-signing seed (the effectively-once gate's key).
pub const DEST_SEED: [u8; 32] = [22u8; 32];
/// The destination's X25519 key-agreement **secret** (re-derives each cofre's per-cofre data key).
pub const DEST_X_SECRET: [u8; 32] = [5u8; 32];
/// A **forged** sender's Ed25519 seed — a wrong signer the destination does not pin (used by the forgery flood).
pub const FORGER_SEED: [u8; 32] = [99u8; 32];
/// The fixed A→B route id every cofre on this route carries.
pub const ROUTE: [u8; 16] = [1u8; 16];
/// The fixed ordering-domain (stream) id every cofre on this route carries.
pub const STREAM: [u8; 16] = [2u8; 16];
/// The per-tenant secret keying the idempotency MAC `HMAC(tenant_secret, record_key)`.
pub const TENANT: [u8; 32] = [6u8; 32];
/// The contract prefix every conforming record must start with (a minimal "required fields" stand-in).
pub const PREFIX: &[u8] = b"evt:";
/// The contract's maximum permitted single-record length, in bytes.
pub const MAX_RECORD_LEN: usize = 1024;

/// The shared route configuration both terminals are built from (same route/stream/tenant/keys).
#[must_use]
pub fn config() -> TerminalConfig {
    TerminalConfig {
        route_id: ROUTE,
        stream_id: STREAM,
        aead_alg: AeadAlg::Gcmsiv256,
        dest_x25519_pk: x25519_public(&DEST_X_SECRET),
        tenant_secret: TENANT,
    }
}

/// The content contract both terminals enforce: non-empty, `<= MAX_RECORD_LEN`, prefixed by [`PREFIX`].
#[must_use]
pub fn contract() -> ContentContract {
    ContentContract::new(MAX_RECORD_LEN, PREFIX.to_vec())
}

/// A matched source/destination terminal pair for one route — the unit under assault.
///
/// The [`SourceTerminal`] holds the route's signing seed; the [`DestTerminal`] pins the matching verifying key,
/// so an authentic cofre verifies and a forged one is dead-lettered. Build with [`Rig::new`].
#[derive(Debug)]
pub struct Rig {
    /// The onboarding terminal (seals + signs conforming records into cofres).
    pub source: SourceTerminal,
    /// The offloading terminal (verifies, opens, re-validates, and commits exactly once / dead-letters).
    pub dest: DestTerminal,
}

impl Rig {
    /// A fresh rig: a source signing under [`SOURCE_SEED`] and a destination pinning its verifying key.
    #[must_use]
    pub fn new() -> Self {
        let source_vk = verifying_key(&SOURCE_SEED);
        Self {
            source: SourceTerminal::new(config(), contract(), SOURCE_SEED),
            dest: DestTerminal::new(config(), contract(), source_vk, DEST_SEED, DEST_X_SECRET),
        }
    }
}

impl Default for Rig {
    fn default() -> Self {
        Self::new()
    }
}

/// A running count of every [`Disposition`] a destination returned, plus an explicit **lost** counter the
/// scenario increments when a record that *should* have been delivered was not.
///
/// A clean run ends with `lost == 0`, `duplicate`/`dead_lettered` matching the assault, and `delivered`
/// equal to the number of distinct authentic records. Print it with [`Tally::report`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Tally {
    /// Records committed to the sink exactly once.
    pub delivered: u64,
    /// Cofres the effectively-once gate dropped as already-seen (drop + re-ack, no re-commit).
    pub duplicate: u64,
    /// Cofres reason-coded onto the dead-letter siding (failed seal/contract/open), never delivered.
    pub dead_lettered: u64,
    /// Records that should have been delivered but were not (a 0-loss violation if non-zero).
    pub lost: u64,
}

impl Tally {
    /// A zeroed tally.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold one observed [`Disposition`] into the tally.
    pub fn observe(&mut self, disp: Disposition) {
        match disp {
            Disposition::Delivered => self.delivered += 1,
            Disposition::Duplicate => self.duplicate += 1,
            Disposition::DeadLettered => self.dead_lettered += 1,
        }
    }

    /// Record that one record that should have been delivered was lost.
    pub fn lost_one(&mut self) {
        self.lost += 1;
    }

    /// A one-line, human-readable tally for the test output (printed by each scenario on success).
    #[must_use]
    pub fn report(&self, scenario: &str) -> String {
        format!(
            "{scenario}: delivered={} duplicate={} dead_lettered={} lost={}",
            self.delivered, self.duplicate, self.dead_lettered, self.lost
        )
    }
}

/// Build a conforming record payload for index `i`: `evt:rec-<i>` (always passes the [`contract`]).
#[must_use]
pub fn conforming_record(i: usize) -> Vec<u8> {
    format!("evt:rec-{i:08}").into_bytes()
}

/// The record-key for index `i` — distinct per `i`, so each yields a distinct idempotency key (a genuinely
/// distinct record, not a duplicate).
#[must_use]
pub fn record_key(i: usize) -> Vec<u8> {
    format!("rk-{i:08}").into_bytes()
}

/// A forger source terminal: identical route config + contract, but signing under [`FORGER_SEED`] (not pinned
/// by the destination). Boarding a conforming record through it yields a *structurally valid* cofre that fails
/// the destination's `SignerMismatch` check — so it exercises the real forged-signer path, not merely a decode
/// failure. The test body boards through this terminal (test bodies may `expect`); a conforming payload always
/// boards, so the only error path (`ContractViolation`) cannot fire for the records this suite forges.
#[must_use]
pub fn forger_source() -> SourceTerminal {
    SourceTerminal::new(config(), contract(), FORGER_SEED)
}

/// Flip a single bit deep inside an authentic cofre's **ciphertext** (`carga`), returning a tampered clone.
///
/// The flip invalidates `cofre_id == BLAKE3(carga)` and the `lacre` over the region, so the destination's
/// parse-before-verify must reject it (`CofreIdMismatch`/`BadSignature`) — it is dead-lettered, never opened.
/// If the carga is empty (it never is for a boarded cofre), the cofre is returned unchanged.
#[must_use]
pub fn tamper_carga(cofre: &Cofre) -> Cofre {
    let mut c = cofre.clone();
    if let Some(byte) = c.carga.first_mut() {
        *byte ^= 0x01;
    }
    c
}

/// Tamper a cofre **on the wire**: encode it, flip one byte at `index % len`, and return the raw bytes. Used by
/// the untrusted-store assault to write corrupt objects whose `decode`+`verify` must fail. Returns an empty
/// vector only for an (impossible) empty encoding.
#[must_use]
pub fn tampered_wire_bytes(cofre: &Cofre, index: usize) -> Vec<u8> {
    let mut bytes = datarail_cofre::encode(cofre);
    if !bytes.is_empty() {
        let n = bytes.len();
        bytes[index % n] ^= 0x01;
    }
    bytes
}
