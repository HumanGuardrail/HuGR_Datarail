//! `datarail-terminal` — the onboarding / offloading terminals (SPEC `06-terminal-protocol.md`,
//! Capability D / AC-2,3,9). A terminal is **our mechanism running in the user's trust zone**: it holds the
//! keys, sees plaintext, and enforces the **user's content contract** (`INV-CONTRACT-SPLIT` — content
//! contract here, envelope contract at the rail).
//!
//! This crate wires the whole flow end-to-end against the **frozen** seam (`docs/design/FOOTER-FREEZE.md`):
//! it transcribes [`datarail_cofre::seal`] / [`datarail_cofre::verify`], the [`datarail_crypto`] primitives,
//! and the [`datarail_once`] admission gate; it does **not** redesign any of them.
//!
//! - **Onboarding** ([`SourceTerminal::board`]): enforce the content contract on every record (a failing
//!   record **never boards**, AC-9 boarding side), frame the conforming records into a `RECORD_BATCH`,
//!   AEAD-seal them into a [`Cofre`], and Ed25519-sign it.
//! - **Offloading** ([`DestTerminal::offload`]): parse-before-verify the seal against the **pinned** source
//!   key, check the `contract_fp`, AEAD-open, re-validate every record against the offloading contract, then
//!   take the [`datarail_once`] admission decision and commit exactly once to the sink. Any seal/contract
//!   failure is routed to a reason-coded **dead-letter** siding (AC-9 offload side), never delivered.
//!
//! ## Key-wrap & v1 notes
//!
//! - **Per-cofre X25519 key-wrap (SPEC A5 / `03`).** `board` generates a fresh ephemeral X25519 key and seals
//!   a fresh per-cofre data key to the route's destination public key ([`datarail_crypto::seal_key`]); the
//!   `eph_pk` rides in the etiqueta and only the holder of the destination secret re-derives the key
//!   ([`datarail_crypto::open_key`]). Forward-secure (the ephemeral is discarded) and provider-blind.
//! - **`aad = &[]`.** The SPEC names `aad = etiqueta` for the AEAD. v1 passes an **empty AAD**: the outer
//!   Ed25519 `lacre` already binds `etiqueta ⊗ carga` (`INV-SEAL-COMPLETE`), so integrity of the header is
//!   not lost. Binding `aad = etiqueta` is a **hardening TODO** with a real ordering cycle to resolve first:
//!   `cofre_id = BLAKE3(carga)` is set by [`datarail_cofre::seal`] *after* the carga exists, so the etiqueta
//!   is not final at AEAD-seal time. See [`AAD_V1`].

#![forbid(unsafe_code)]

use datarail_core::{AeadAlg, Cofre, Disposition, Etiqueta};
use datarail_cofre::CofreError;
use datarail_crypto::{aead_open, aead_seal, blake3_256, hmac_blake3, open_key, seal_key, AeadError};
use datarail_once::Once;

/// The AEAD associated data used in v1: **empty**.
///
/// The outer `lacre` (Ed25519 over `etiqueta ⊗ carga`) already authenticates the header, so an empty AAD does
/// not weaken integrity. Binding `aad = etiqueta` is a documented hardening TODO (it has a `cofre_id` ordering
/// cycle, since `cofre_id = BLAKE3(carga)` is only set inside [`datarail_cofre::seal`]).
pub const AAD_V1: &[u8] = &[];

/// Read 32 bytes of OS entropy for a fresh per-cofre ephemeral key. Zero-dep (`/dev/urandom`); the source
/// terminal calls this once per [`SourceTerminal::board`].
fn random_32() -> Result<[u8; 32], TerminalError> {
    use std::io::Read as _;
    let mut file = std::fs::File::open("/dev/urandom").map_err(|_| TerminalError::Entropy)?;
    let mut buf = [0u8; 32];
    file.read_exact(&mut buf).map_err(|_| TerminalError::Entropy)?;
    Ok(buf)
}

// ----------------------------------------------------------------------------------------------------------
// Content contract — a real, simple stand-in for schema / required-fields (D4: declarative policy).
// ----------------------------------------------------------------------------------------------------------

/// A content contract: the user's declarative validation policy for a single record (SPEC 06, D4).
///
/// This is a deliberately simple but *real* stand-in for a schema / required-fields rule set: a record is
/// valid iff it is non-empty, no longer than [`max_record_len`](Self::max_record_len), and begins with
/// [`required_prefix`](Self::required_prefix). The [`fingerprint`](Self::fingerprint) is what the destination
/// pins as the expected schema version — a mismatch is a *managed* schema-drift event (dead-letter), never
/// silent corruption.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentContract {
    /// `BLAKE3` fingerprint identifying this contract / schema version (stamped into the etiqueta's
    /// `contract_fp` and checked at offload).
    pub fingerprint: [u8; 32],
    /// Maximum permitted length of a single record, in bytes.
    pub max_record_len: usize,
    /// Bytes every conforming record must start with (a minimal "required fields" stand-in).
    pub required_prefix: Vec<u8>,
}

impl ContentContract {
    /// Build a contract whose [`fingerprint`](Self::fingerprint) is derived deterministically from its rules,
    /// so two terminals configured with the same rules agree on the same schema id.
    ///
    /// The fingerprint commits to `max_record_len ‖ required_prefix` under [`blake3_256`].
    #[must_use]
    pub fn new(max_record_len: usize, required_prefix: Vec<u8>) -> Self {
        let mut preimage = Vec::with_capacity(8 + required_prefix.len());
        preimage.extend_from_slice(&(max_record_len as u64).to_le_bytes());
        preimage.extend_from_slice(&required_prefix);
        Self {
            fingerprint: blake3_256(&preimage),
            max_record_len,
            required_prefix,
        }
    }

    /// Validate one record against this contract: non-empty, `len <= max_record_len`, and starting with
    /// [`required_prefix`](Self::required_prefix).
    #[must_use]
    pub fn validate(&self, record: &[u8]) -> bool {
        !record.is_empty()
            && record.len() <= self.max_record_len
            && record.starts_with(&self.required_prefix)
    }
}

// ----------------------------------------------------------------------------------------------------------
// RECORD_BATCH framing — length-prefixed: u32 count, then per-record (u32 len ‖ bytes).
// ----------------------------------------------------------------------------------------------------------

/// Encode records into a length-prefixed `RECORD_BATCH`: `u32 count`, then for each record `u32 len ‖ bytes`
/// (little-endian). This is the plaintext that gets AEAD-sealed into the cofre's `carga`.
fn frame_batch(records: &[&[u8]]) -> Result<Vec<u8>, TerminalError> {
    let count = u32::try_from(records.len()).map_err(|_| TerminalError::BatchTooLarge)?;
    let total: usize = records.iter().map(|r| 4 + r.len()).sum();
    let mut buf = Vec::with_capacity(4 + total);
    buf.extend_from_slice(&count.to_le_bytes());
    for record in records {
        let len = u32::try_from(record.len()).map_err(|_| TerminalError::BatchTooLarge)?;
        buf.extend_from_slice(&len.to_le_bytes());
        buf.extend_from_slice(record);
    }
    Ok(buf)
}

/// Decode a `RECORD_BATCH` produced by [`frame_batch`] back into its records. Every declared length is
/// bound-checked against the remaining buffer (a malformed batch is a [`TerminalError::MalformedBatch`], which
/// the caller turns into a dead-letter — never a panic).
fn unframe_batch(bytes: &[u8]) -> Result<Vec<Vec<u8>>, TerminalError> {
    let mut pos = 0usize;
    let count_bytes = bytes
        .get(pos..pos + 4)
        .ok_or(TerminalError::MalformedBatch)?;
    let count = u32::from_le_bytes(count_bytes.try_into().map_err(|_| TerminalError::MalformedBatch)?);
    pos += 4;
    let mut records = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let len_bytes = bytes
            .get(pos..pos + 4)
            .ok_or(TerminalError::MalformedBatch)?;
        let len = u32::from_le_bytes(len_bytes.try_into().map_err(|_| TerminalError::MalformedBatch)?)
            as usize;
        pos += 4;
        let end = pos.checked_add(len).ok_or(TerminalError::MalformedBatch)?;
        let record = bytes.get(pos..end).ok_or(TerminalError::MalformedBatch)?;
        records.push(record.to_vec());
        pos = end;
    }
    if pos != bytes.len() {
        return Err(TerminalError::MalformedBatch);
    }
    Ok(records)
}

// ----------------------------------------------------------------------------------------------------------
// Errors.
// ----------------------------------------------------------------------------------------------------------

/// An error from a terminal operation.
///
/// Note the split (SPEC 06): a content-contract violation **at onboarding** is an *error* ([`board`] never
/// produces a cofre), whereas a content/seal failure **at offloading** is a [`Disposition::DeadLettered`]
/// (the cofre is preserved on the siding), not an error. [`MalformedBatch`](Self::MalformedBatch) is reserved
/// for an internal framing inconsistency.
///
/// [`board`]: SourceTerminal::board
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalError {
    /// A record violated the onboarding content contract — the batch **never boards** (AC-9, boarding side).
    ContractViolation,
    /// Sealing the batch failed at the AEAD layer (e.g. a bad key length).
    Seal,
    /// A `RECORD_BATCH` could not be framed (more than `u32::MAX` records, or a record longer than `u32::MAX`).
    BatchTooLarge,
    /// A decoded `RECORD_BATCH` was internally inconsistent (truncated / trailing bytes).
    MalformedBatch,
    /// OS entropy for the per-cofre ephemeral key could not be read.
    Entropy,
}

impl core::fmt::Display for TerminalError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = match self {
            Self::ContractViolation => "record violates the onboarding content contract (never boards)",
            Self::Seal => "AEAD seal failed",
            Self::BatchTooLarge => "record batch exceeds framing limits",
            Self::MalformedBatch => "record batch is malformed",
            Self::Entropy => "could not read OS entropy for the per-cofre key",
        };
        f.write_str(s)
    }
}

impl core::error::Error for TerminalError {}

impl From<AeadError> for TerminalError {
    fn from(_: AeadError) -> Self {
        Self::Seal
    }
}

// ----------------------------------------------------------------------------------------------------------
// Dead-letter siding + sink.
// ----------------------------------------------------------------------------------------------------------

/// Why a cofre was diverted to the offloading dead-letter siding (D3 — reason-coded, preserved, never
/// delivered).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeadLetterReason {
    /// The seal failed parse-before-verify against the pinned source key (`INV-TAMPER-REJECT`, AC-2/3).
    SealFailed(CofreError),
    /// The cofre is addressed to a different route/stream than this terminal serves.
    RouteMismatch,
    /// The cofre's `contract_fp` did not match the destination's expected schema (AC-9 — managed drift).
    ContractFingerprintMismatch,
    /// The AEAD-open failed (tamper / wrong key / nonce), so the payload could not be read.
    OpenFailed,
    /// The decrypted `RECORD_BATCH` was malformed (truncated / trailing bytes).
    MalformedBatch,
    /// A decrypted record violated the offloading content contract (AC-9, offload side).
    ContractViolation,
}

impl core::fmt::Display for DeadLetterReason {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::SealFailed(e) => write!(f, "seal verification failed: {e}"),
            Self::RouteMismatch => f.write_str("cofre addressed to a different route/stream"),
            Self::ContractFingerprintMismatch => f.write_str("contract fingerprint mismatch (schema drift)"),
            Self::OpenFailed => f.write_str("AEAD open failed (tamper/wrong key)"),
            Self::MalformedBatch => f.write_str("decrypted record batch is malformed"),
            Self::ContractViolation => f.write_str("decrypted record violates offloading contract"),
        }
    }
}

/// One entry on the dead-letter siding: the preserved cofre plus the reason it was diverted (D3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetter {
    /// The full cofre, preserved verbatim (never dropped, never delivered).
    pub cofre: Cofre,
    /// The reason-code for the diversion.
    pub reason: DeadLetterReason,
}

/// The offloading dead-letter siding: a reason-coded, append-only list of diverted cofres (D3).
#[derive(Debug, Default, Clone)]
pub struct DeadLetterSiding {
    entries: Vec<DeadLetter>,
}

impl DeadLetterSiding {
    /// A fresh, empty siding.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of diverted cofres on the siding.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the siding is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The diverted entries, in arrival order.
    #[must_use]
    pub fn entries(&self) -> &[DeadLetter] {
        &self.entries
    }

    /// Divert a cofre to the siding with its reason-code.
    fn push(&mut self, cofre: Cofre, reason: DeadLetterReason) {
        self.entries.push(DeadLetter { cofre, reason });
    }
}

/// A trivial in-memory sink: the records committed (delivered exactly once) at the destination.
#[derive(Debug, Default, Clone)]
pub struct Sink {
    committed: Vec<Vec<u8>>,
}

impl Sink {
    /// A fresh, empty sink.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of records committed so far.
    #[must_use]
    pub fn len(&self) -> usize {
        self.committed.len()
    }

    /// Whether the sink holds no committed records.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.committed.is_empty()
    }

    /// The committed records, in commit order.
    #[must_use]
    pub fn committed(&self) -> &[Vec<u8>] {
        &self.committed
    }

    /// Commit a batch of records (idempotency is decided upstream by [`datarail_once`]).
    fn commit(&mut self, records: Vec<Vec<u8>>) {
        self.committed.extend(records);
    }
}

// ----------------------------------------------------------------------------------------------------------
// Shared route configuration (the v1 key simplification lives here).
// ----------------------------------------------------------------------------------------------------------

/// The static, shared parameters for one A→B route, held by **both** terminals.
///
/// Bundling the route parameters in one struct (rather than passing loose `[u8; _]` arguments) keeps the
/// terminal constructors within the Craft Charter without an `#[allow]`.
#[derive(Debug, Clone)]
pub struct TerminalConfig {
    /// Fixed A→B route id (copied into every etiqueta).
    pub route_id: [u8; 16],
    /// Ordering domain (copied into every etiqueta; drives the [`datarail_once`] gate keyed per stream).
    pub stream_id: [u8; 16],
    /// AEAD algorithm for the carga (default [`AeadAlg::Gcmsiv256`]).
    pub aead_alg: AeadAlg,
    /// The route **destination's X25519 public key**. `board` seals a fresh per-cofre data key to it; only the
    /// destination terminal (holding the matching secret) can open it — provider-blind and forward-secure.
    pub dest_x25519_pk: [u8; 32],
    /// Per-tenant secret keying the idempotency MAC `HMAC(tenant_secret, record_key)`.
    pub tenant_secret: [u8; 32],
}

// ----------------------------------------------------------------------------------------------------------
// Source terminal — onboarding.
// ----------------------------------------------------------------------------------------------------------

/// The **onboarding** (source) terminal: enforces the content contract, frames a `RECORD_BATCH`, and seals it
/// into a signed [`Cofre`] (SPEC 06, boarding side).
///
/// Holds the route config, the onboarding [`ContentContract`], the Ed25519 source signing seed, and the
/// monotonic per-stream `seq`. A contract-violating record makes [`board`](Self::board) return
/// [`TerminalError::ContractViolation`] — it **never boards** (AC-9).
#[derive(Debug, Clone)]
pub struct SourceTerminal {
    config: TerminalConfig,
    contract: ContentContract,
    /// Ed25519 source signing seed (the route's pinned identity).
    source_seed: [u8; 32],
    /// Monotonic per-stream sequence counter.
    seq: u64,
}

impl SourceTerminal {
    /// Build a source terminal for a route with its onboarding `contract` and Ed25519 `source_seed`.
    #[must_use]
    pub fn new(config: TerminalConfig, contract: ContentContract, source_seed: [u8; 32]) -> Self {
        Self {
            config,
            contract,
            source_seed,
            seq: 0,
        }
    }

    /// The next sequence number this terminal will assign (the count of cofres boarded so far).
    #[must_use]
    pub fn next_seq(&self) -> u64 {
        self.seq
    }

    /// **Onboard** a batch of records into a sealed cofre (SPEC 06, boarding side).
    ///
    /// Steps: validate **every** record against the onboarding contract (any failure ⇒
    /// [`TerminalError::ContractViolation`], and the batch never boards — AC-9); frame the records into a
    /// `RECORD_BATCH`; seal a fresh per-cofre data key to the dest's X25519 key, then AEAD-seal the batch;
    /// stamp the etiqueta (including `idempotency_key = HMAC(tenant_secret, record_key)` and the contract
    /// fingerprint); [`datarail_cofre::seal`] it under the source seed; bump `seq`; return the cofre.
    ///
    /// # Errors
    /// - [`TerminalError::ContractViolation`] if any record fails the onboarding contract (never boards).
    /// - [`TerminalError::BatchTooLarge`] if the batch exceeds the `u32` framing limits.
    /// - [`TerminalError::Seal`] if the AEAD seal fails (e.g. a bad key length).
    pub fn board(&mut self, records: &[&[u8]], record_key: &[u8]) -> Result<Cofre, TerminalError> {
        // (1) Enforce the content contract on EVERY record — a single failure means the batch never boards.
        if !records.iter().all(|r| self.contract.validate(r)) {
            return Err(TerminalError::ContractViolation);
        }

        // (2) Frame the conforming records into a length-prefixed RECORD_BATCH.
        let batch = frame_batch(records)?;

        // (3) Idempotency key: HMAC(tenant_secret, record_key) — the sole effectively-once dedup key.
        let idempotency_key = hmac_blake3(&self.config.tenant_secret, record_key);

        // (4) Per-cofre key-wrap (SPEC A5/03): a fresh ephemeral X25519 key seals a fresh data key to the
        // route's destination public key — only the dest can re-derive it (provider-blind, forward-secure).
        let eph_secret = random_32()?;
        let (eph_pk, data_key) = seal_key(&self.config.dest_x25519_pk, &eph_secret);

        // (5) AEAD-seal the batch under the per-cofre data key with a per-cofre nonce. aad = &[] (v1).
        let nonce = self.next_nonce();
        let carga = aead_seal(self.config.aead_alg, &data_key, &nonce, AAD_V1, &batch)?;

        // (6) Build the etiqueta; cofre_id/signer_key_id are stamped by `seal`.
        let etiqueta = Etiqueta {
            route_id: self.config.route_id,
            stream_id: self.config.stream_id,
            seq: self.seq,
            cofre_id: [0u8; 32],
            idempotency_key,
            contract_fp: self.contract.fingerprint,
            aead_alg: self.config.aead_alg,
            nonce,
            signer_key_id: [0u8; 32],
            eph_pk,
        };

        // (6) Seal (Ed25519 over etiqueta ⊗ carga) and advance the per-stream sequence.
        let cofre = datarail_cofre::seal(etiqueta, carga, &self.source_seed);
        self.seq += 1;
        Ok(cofre)
    }

    /// Derive a per-cofre nonce. Deterministic in v1 (the route's `seq`), which is safe under the GCM-SIV
    /// default (nonce-misuse-resistant, AUDIT-01 BLK-1). A CSPRNG nonce is the production choice.
    fn next_nonce(&self) -> [u8; 12] {
        let mut nonce = [0u8; 12];
        nonce[..8].copy_from_slice(&self.seq.to_le_bytes());
        nonce
    }
}

// ----------------------------------------------------------------------------------------------------------
// Destination terminal — offloading.
// ----------------------------------------------------------------------------------------------------------

/// The **offloading** (destination) terminal: verifies the seal, checks the contract, opens the payload,
/// re-validates every record, and commits exactly once (SPEC 06, offload side).
///
/// Owns the route config, the offloading [`ContentContract`], the **pinned** source verifying key, the
/// [`datarail_once`] admission gate, the [`Sink`], and the [`DeadLetterSiding`]. Any seal/contract failure is
/// reason-coded onto the siding and reported as [`Disposition::DeadLettered`] — never delivered (AC-9).
#[derive(Debug)]
pub struct DestTerminal {
    config: TerminalConfig,
    contract: ContentContract,
    /// The route-pinned source Ed25519 verifying key (BLK-4).
    pinned_source_vk: [u8; 32],
    /// This destination's X25519 secret — unwraps the per-cofre data key from the cofre's `eph_pk`.
    dest_x25519_secret: [u8; 32],
    once: Once,
    sink: Sink,
    dead_letters: DeadLetterSiding,
}

impl DestTerminal {
    /// Build a destination terminal for a route with its offloading `contract`, the **pinned** source
    /// verifying key, and a [`datarail_once`] gate signing watermarks under `dest_seed`.
    #[must_use]
    pub fn new(
        config: TerminalConfig,
        contract: ContentContract,
        pinned_source_vk: [u8; 32],
        dest_seed: [u8; 32],
        dest_x25519_secret: [u8; 32],
    ) -> Self {
        Self {
            config,
            contract,
            pinned_source_vk,
            dest_x25519_secret,
            once: Once::new(dest_seed),
            sink: Sink::new(),
            dead_letters: DeadLetterSiding::new(),
        }
    }

    /// The destination's commit sink (records delivered exactly once).
    #[must_use]
    pub fn sink(&self) -> &Sink {
        &self.sink
    }

    /// The reason-coded dead-letter siding.
    #[must_use]
    pub fn dead_letters(&self) -> &DeadLetterSiding {
        &self.dead_letters
    }

    /// **Offload** a received cofre (SPEC 06, offload side).
    ///
    /// Order (the offloading pipeline): parse-before-verify the seal against the pinned source key (fail ⇒
    /// dead-letter); check `contract_fp` against the expected schema (fail ⇒ dead-letter, managed drift);
    /// AEAD-open and un-frame the `RECORD_BATCH` (fail ⇒ dead-letter); re-validate every record against the
    /// offloading contract (any fail ⇒ dead-letter); take the [`datarail_once`] admission decision —
    /// [`Disposition::Duplicate`] ⇒ drop, no commit; [`Disposition::Delivered`] ⇒ commit the records to the
    /// sink.
    ///
    /// # Errors
    /// This function does not currently return an error: every rejection is a
    /// [`Disposition::DeadLettered`] on the siding (a preserved, reason-coded event, not a failure), and a
    /// dedup drop is a [`Disposition::Duplicate`]. The `Result` matches the [`datarail_core::Terminal`] seam
    /// and reserves room for a future fallible sink commit.
    pub fn offload(&mut self, cofre: &Cofre) -> Result<Disposition, TerminalError> {
        // (1) Parse-before-verify: verify the lacre against the route-pinned source key (BLK-4/7, AC-2/3).
        if let Err(e) = datarail_cofre::verify(cofre, &self.pinned_source_vk) {
            self.dead_letters
                .push(cofre.clone(), DeadLetterReason::SealFailed(e));
            return Ok(Disposition::DeadLettered);
        }

        // (2) Route binding: the cofre must be addressed to THIS terminal's route + stream.
        if cofre.etiqueta.route_id != self.config.route_id
            || cofre.etiqueta.stream_id != self.config.stream_id
        {
            self.dead_letters
                .push(cofre.clone(), DeadLetterReason::RouteMismatch);
            return Ok(Disposition::DeadLettered);
        }

        // (3) Schema check: the cofre's claimed contract_fp must match this destination's expected contract.
        if cofre.etiqueta.contract_fp != self.contract.fingerprint {
            self.dead_letters
                .push(cofre.clone(), DeadLetterReason::ContractFingerprintMismatch);
            return Ok(Disposition::DeadLettered);
        }

        // (3) Re-derive the per-cofre data key from the authenticated eph_pk, then AEAD-open (aad = &[]); a
        // tamper / wrong-key failure dead-letters.
        let data_key = open_key(&self.dest_x25519_secret, &cofre.etiqueta.eph_pk);
        let Ok(batch) = aead_open(
            cofre.etiqueta.aead_alg,
            &data_key,
            &cofre.etiqueta.nonce,
            AAD_V1,
            &cofre.carga,
        ) else {
            self.dead_letters
                .push(cofre.clone(), DeadLetterReason::OpenFailed);
            return Ok(Disposition::DeadLettered);
        };

        // (4) Un-frame the RECORD_BATCH; a malformed batch dead-letters.
        let Ok(records) = unframe_batch(&batch) else {
            self.dead_letters
                .push(cofre.clone(), DeadLetterReason::MalformedBatch);
            return Ok(Disposition::DeadLettered);
        };

        // (5) Re-validate EVERY record against the offloading contract — any failure dead-letters (AC-9).
        if !records.iter().all(|r| self.contract.validate(r)) {
            self.dead_letters
                .push(cofre.clone(), DeadLetterReason::ContractViolation);
            return Ok(Disposition::DeadLettered);
        }

        // (6) Effectively-once admission, then commit-on-Delivered only (exactly-once at the sink).
        match self.once.admit(
            cofre.etiqueta.stream_id,
            cofre.etiqueta.seq,
            cofre.etiqueta.idempotency_key,
        ) {
            Disposition::Delivered => {
                self.sink.commit(records);
                Ok(Disposition::Delivered)
            }
            // A duplicate is dropped without committing; DeadLettered cannot come from the once gate.
            other => Ok(other),
        }
    }
}

#[cfg(test)]
mod tests;
