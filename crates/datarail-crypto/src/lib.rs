//! `datarail-crypto` — the primitives behind the cofre seal, per the frozen SPEC `03-crypto-construction.md`.
//!
//! BLAKE3 hashing, **domain-separated** Ed25519 signatures (BLK-5 — each role gets a distinct context prefix
//! so a signature can never be replayed across contexts), and a **pluggable AEAD** with **AES-256-GCM-SIV** as
//! the nonce-misuse-resistant default (BLK-1). No `unsafe`.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use datarail_core::AeadAlg;

/// Ed25519 domain-separation context labels (BLK-5). Each signature role gets a distinct prefix.
pub mod ctx {
    /// The cofre seal (`lacre`).
    pub const LACRE: &[u8] = b"dr:lacre:v1";
    /// A manifest Signed Tree Head.
    pub const STH: &[u8] = b"dr:sth:v1";
    /// A destination delivery ack.
    pub const ACK: &[u8] = b"dr:ack:v1";
    /// A sealed-sender certificate.
    pub const CERT: &[u8] = b"dr:cert:v1";
    /// A route descriptor / ticket.
    pub const TICKET: &[u8] = b"dr:ticket:v1";
}

/// BLAKE3-256 of `data`.
#[must_use]
pub fn blake3_256(data: &[u8]) -> [u8; 32] {
    *blake3::hash(data).as_bytes()
}

fn framed(context: &[u8], msg: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(context.len() + msg.len());
    buf.extend_from_slice(context);
    buf.extend_from_slice(msg);
    buf
}

/// Sign `msg` under a domain-separation `context` with the Ed25519 secret `seed` (32 bytes).
#[must_use]
pub fn sign_domain(context: &[u8], seed: &[u8; 32], msg: &[u8]) -> [u8; 64] {
    use ed25519_dalek::{Signer, SigningKey};
    SigningKey::from_bytes(seed)
        .sign(&framed(context, msg))
        .to_bytes()
}

/// The Ed25519 verifying (public) key for the secret `seed`.
#[must_use]
pub fn verifying_key(seed: &[u8; 32]) -> [u8; 32] {
    ed25519_dalek::SigningKey::from_bytes(seed)
        .verifying_key()
        .to_bytes()
}

/// Verify a domain-separated Ed25519 signature against verifying key `vk`.
#[must_use]
pub fn verify_domain(context: &[u8], vk: &[u8; 32], msg: &[u8], sig: &[u8; 64]) -> bool {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    let Ok(vk) = VerifyingKey::from_bytes(vk) else {
        return false;
    };
    vk.verify(&framed(context, msg), &Signature::from_bytes(sig))
        .is_ok()
}

/// An AEAD operation failed: a bad key length, or — on open — an authentication failure (tamper / wrong
/// key, nonce, or AAD).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AeadError;

impl core::fmt::Display for AeadError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("AEAD operation failed (bad key or authentication failure)")
    }
}

impl core::error::Error for AeadError {}

/// AEAD-**seal** `plaintext` with associated data `aad` under `key` (32 B) and `nonce` (12 B) using `alg`.
///
/// # Errors
/// Returns [`AeadError`] if the key is the wrong length or the cipher refuses the input.
pub fn aead_seal(
    alg: AeadAlg,
    key: &[u8; 32],
    nonce: &[u8; 12],
    aad: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, AeadError> {
    let payload = Payload {
        msg: plaintext,
        aad,
    };
    match alg {
        AeadAlg::Gcmsiv256 => aes_gcm_siv::Aes256GcmSiv::new_from_slice(key)
            .map_err(|_| AeadError)?
            .encrypt(aes_gcm_siv::Nonce::from_slice(nonce), payload)
            .map_err(|_| AeadError),
        AeadAlg::ChaCha20Poly1305 => chacha20poly1305::ChaCha20Poly1305::new_from_slice(key)
            .map_err(|_| AeadError)?
            .encrypt(chacha20poly1305::Nonce::from_slice(nonce), payload)
            .map_err(|_| AeadError),
        AeadAlg::Gcm256 => aes_gcm::Aes256Gcm::new_from_slice(key)
            .map_err(|_| AeadError)?
            .encrypt(aes_gcm::Nonce::from_slice(nonce), payload)
            .map_err(|_| AeadError),
    }
}

/// AEAD-**open** `ciphertext` with associated data `aad` under `key` (32 B) and `nonce` (12 B) using `alg`.
///
/// # Errors
/// Returns [`AeadError`] on authentication failure (tamper, or wrong key / nonce / AAD).
pub fn aead_open(
    alg: AeadAlg,
    key: &[u8; 32],
    nonce: &[u8; 12],
    aad: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>, AeadError> {
    let payload = Payload {
        msg: ciphertext,
        aad,
    };
    match alg {
        AeadAlg::Gcmsiv256 => aes_gcm_siv::Aes256GcmSiv::new_from_slice(key)
            .map_err(|_| AeadError)?
            .decrypt(aes_gcm_siv::Nonce::from_slice(nonce), payload)
            .map_err(|_| AeadError),
        AeadAlg::ChaCha20Poly1305 => chacha20poly1305::ChaCha20Poly1305::new_from_slice(key)
            .map_err(|_| AeadError)?
            .decrypt(chacha20poly1305::Nonce::from_slice(nonce), payload)
            .map_err(|_| AeadError),
        AeadAlg::Gcm256 => aes_gcm::Aes256Gcm::new_from_slice(key)
            .map_err(|_| AeadError)?
            .decrypt(aes_gcm::Nonce::from_slice(nonce), payload)
            .map_err(|_| AeadError),
    }
}

#[cfg(test)]
mod tests {
    use super::{aead_open, aead_seal, ctx, sign_domain, verify_domain, verifying_key};
    use datarail_core::AeadAlg;

    const SEED: [u8; 32] = [7u8; 32];
    const KEY: [u8; 32] = [9u8; 32];
    const NONCE: [u8; 12] = [3u8; 12];

    #[test]
    fn ed25519_domain_roundtrip_and_separation() {
        let vk = verifying_key(&SEED);
        let sig = sign_domain(ctx::LACRE, &SEED, b"hello");
        assert!(verify_domain(ctx::LACRE, &vk, b"hello", &sig));
        // BLK-5: a signature under one context must not verify under another.
        assert!(!verify_domain(ctx::ACK, &vk, b"hello", &sig));
        // Wrong message fails.
        assert!(!verify_domain(ctx::LACRE, &vk, b"hell0", &sig));
    }

    #[test]
    fn aead_roundtrip_all_algs() {
        for alg in [
            AeadAlg::Gcmsiv256,
            AeadAlg::ChaCha20Poly1305,
            AeadAlg::Gcm256,
        ] {
            let ct = aead_seal(alg, &KEY, &NONCE, b"aad", b"plaintext").unwrap();
            let pt = aead_open(alg, &KEY, &NONCE, b"aad", &ct).unwrap();
            assert_eq!(pt, b"plaintext");
        }
    }

    #[test]
    fn aead_tamper_and_aad_mismatch_fail() {
        let alg = AeadAlg::Gcmsiv256;
        let mut ct = aead_seal(alg, &KEY, &NONCE, b"aad", b"plaintext").unwrap();
        // INV-TAMPER-REJECT: flipping any ciphertext byte must fail open.
        ct[0] ^= 0x01;
        assert!(aead_open(alg, &KEY, &NONCE, b"aad", &ct).is_err());
        // AAD mismatch (e.g. a mutated etiqueta) must fail open.
        let good = aead_seal(alg, &KEY, &NONCE, b"aad", b"plaintext").unwrap();
        assert!(aead_open(alg, &KEY, &NONCE, b"different-aad", &good).is_err());
    }
}
