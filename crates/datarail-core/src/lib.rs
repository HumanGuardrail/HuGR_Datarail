//! `datarail-core` — the trilho's heart: the **Cofre** envelope and the rail/terminal **contracts**.
//!
//! Transcribes the frozen SPEC (`docs/design/SPEC-FREEZE.md`, sha256 `5f88095…6d89`). This is P0 (MF-3):
//! the cross-crate IDL + traits the dependent crates build against. Stubs are `todo!()` — they compile and
//! pin the interface; behavior lands in later phases under each AC's proof method.
//!
//! Invariants embodied here: `INV-OPAQUE-CARGO` (the rail sees only the header + seal),
//! `INV-SEAL-COMPLETE` (the lacre covers the whole cofre), `INV-DUMB-PIPE` (the substrate holds no keys).

/// AEAD algorithm selector. **Default = `Gcmsiv256`** (nonce-misuse-resistant — AUDIT-01 BLK-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AeadAlg {
    /// AES-256-GCM-SIV — the default; survives nonce reuse (leaks at most equality).
    #[default]
    Gcmsiv256,
    /// ChaCha20-Poly1305 (12-byte nonce) — non-AES-NI / non-x86.
    ChaCha20Poly1305,
    /// AES-256-GCM — opt-in perf mode, only behind a key-uniqueness gate.
    Gcm256,
}

/// The authenticated header (*etiqueta*): clear to the rail, but wholly covered by the seal.
///
/// `INV-SEAL-COMPLETE` — every field here is signed by the `lacre`, so the rail can read but never forge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Etiqueta {
    /// Fixed A→B route id.
    pub route_id: [u8; 16],
    /// Ordering domain.
    pub stream_id: [u8; 16],
    /// Per-stream sequence number (ordering, checkpoint, watermark-reject).
    pub seq: u64,
    /// `BLAKE3(carga)` — content-address **identity** (NOT a dedup key).
    pub cofre_id: [u8; 32],
    /// `HMAC(tenant_secret, record_key)` — the **sole** effectively-once dedup key.
    pub idempotency_key: [u8; 32],
    /// `BLAKE3` of the content contract this cofre claims.
    pub contract_fp: [u8; 32],
    /// AEAD selector for the `carga`.
    pub aead_alg: AeadAlg,
    /// Random per-cofre nonce (safe under the GCM-SIV default).
    pub nonce: [u8; 12],
    /// Selector among the route's pre-authorized signing-key pins (verified against the pinned key only).
    pub signer_key_id: [u8; 32],
    /// Ephemeral X25519 public key for the per-cofre key-wrap: the destination derives the carga's data key via
    /// `open_key(dest_x25519_secret, eph_pk)`. Authenticated by the `lacre` like every other header field;
    /// `INV-OPAQUE-CARGO` still holds (the rail can read it but cannot derive the key without the dest secret).
    pub eph_pk: [u8; 32],
}

/// A sealed cofre on the wire: `etiqueta` (authenticated) + `carga` (opaque ciphertext) + `lacre` (Ed25519).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cofre {
    /// The authenticated, rail-readable header.
    pub etiqueta: Etiqueta,
    /// AEAD ciphertext — **opaque to the rail** (`INV-OPAQUE-CARGO`).
    pub carga: Vec<u8>,
    /// Ed25519 signature over `etiqueta ⊗ carga`, domain-separated (`dr:lacre:v1`).
    pub lacre: [u8; 64],
}

/// What the offloading side decided for a received cofre.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// Delivered (and committed) exactly once.
    Delivered,
    /// Already seen — idempotent drop + re-ack.
    Duplicate,
    /// Failed verification or content contract → siding, never delivered.
    DeadLettered,
}

/// The **rail**: moves cofres over a substrate using ONLY the `etiqueta` + `lacre` — never the `carga`.
///
/// `INV-OPAQUE-CARGO` + `INV-DUMB-PIPE`: an implementor holds no keys and never reads the payload.
pub trait Substrate {
    /// Transport error.
    type Error;
    /// Hand a sealed cofre to the substrate for delivery.
    ///
    /// # Errors
    /// Returns `Self::Error` if the underlying substrate fails to enqueue/transmit the cofre.
    fn send(&mut self, cofre: &Cofre) -> Result<(), Self::Error>;
    /// Receive the next sealed cofre, if any.
    ///
    /// # Errors
    /// Returns `Self::Error` if the substrate fails to read (transport/decode of the framed bytes).
    fn recv(&mut self) -> Result<Option<Cofre>, Self::Error>;
    /// Acknowledge delivery of a cofre by its `cofre_id`.
    ///
    /// # Errors
    /// Returns `Self::Error` if the acknowledgement cannot be recorded/transmitted.
    fn ack(&mut self, cofre_id: [u8; 32]) -> Result<(), Self::Error>;
}

/// A **terminal**: seals/unseals at the container edge (sees plaintext, holds keys). The mechanism is ours;
/// the rules it enforces are the user's policy (`INV-CONTRACT-SPLIT`).
pub trait Terminal {
    /// Terminal error (contract violation, crypto, I/O).
    type Error;
    /// Onboarding: enforce the content contract, then seal a record batch into a cofre.
    ///
    /// # Errors
    /// Returns `Self::Error` if a record violates the onboarding content contract, or if sealing fails.
    fn board(&mut self, records: &[u8]) -> Result<Cofre, Self::Error>;
    /// Offloading: parse-before-verify → verify lacre → open → enforce contract → idempotent commit.
    ///
    /// # Errors
    /// Returns `Self::Error` if the seal/contract check or the sink commit fails. (A duplicate or a
    /// dead-lettered cofre is a `Disposition`, not an error.)
    fn offload(&mut self, cofre: &Cofre) -> Result<Disposition, Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aead_default_is_gcm_siv() {
        // AUDIT-01 BLK-1: the default AEAD is the nonce-misuse-resistant one.
        assert_eq!(AeadAlg::default(), AeadAlg::Gcmsiv256);
    }
}
