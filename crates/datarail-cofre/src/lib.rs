//! `datarail-cofre` — canonical wire encoding of the [`Cofre`] (SPEC `02-cofre-format.md`) + seal/verify.
//!
//! Layout: `MAGIC ‖ VERSION ‖ ETIQUETA_LEN ‖ ETIQUETA ‖ CARGA_LEN ‖ CARGA ‖ LACRE`. The `lacre` is a
//! domain-separated Ed25519 signature over the whole region before it (`INV-SEAL-COMPLETE`). Decoding is
//! **parse-before-verify** (BLK-7: every length is bound-checked before any field is read); verification
//! recomputes `cofre_id == BLAKE3(carga)` (BLK-8) and checks the signature only against the **route-pinned**
//! key (BLK-4). No `unsafe`.

use datarail_core::{AeadAlg, Cofre, Etiqueta};
use datarail_crypto::{blake3_256, ctx, sign_domain, verify_domain, verifying_key};

const MAGIC: [u8; 4] = *b"DRLC";
const VERSION: u8 = 1;
/// Canonical encoded size of an `Etiqueta` (v1 — all fixed-width fields).
const ETIQUETA_LEN: usize = 16 + 16 + 8 + 32 + 32 + 32 + 1 + 12 + 32 + 32 + 1 + 8;
const LACRE_LEN: usize = 64;

const fn aead_to_u8(a: AeadAlg) -> u8 {
    match a {
        AeadAlg::Gcmsiv256 => 1,
        AeadAlg::ChaCha20Poly1305 => 2,
        AeadAlg::Gcm256 => 3,
    }
}

const fn aead_from_u8(b: u8) -> Option<AeadAlg> {
    match b {
        1 => Some(AeadAlg::Gcmsiv256),
        2 => Some(AeadAlg::ChaCha20Poly1305),
        3 => Some(AeadAlg::Gcm256),
        _ => None,
    }
}

/// An error decoding or verifying a cofre.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CofreError {
    /// The byte stream ended before a field could be read (parse-before-verify guard).
    Truncated,
    /// Wrong magic bytes.
    BadMagic,
    /// Unsupported format version.
    BadVersion,
    /// A declared length is inconsistent with the frame (incl. trailing bytes).
    BadLength,
    /// Unknown AEAD selector byte.
    BadAead,
    /// `cofre_id` does not equal `BLAKE3(carga)` (BLK-8).
    CofreIdMismatch,
    /// `signer_key_id` does not match the pinned route key (BLK-4).
    SignerMismatch,
    /// The Ed25519 seal failed verification.
    BadSignature,
}

impl core::fmt::Display for CofreError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = match self {
            Self::Truncated => "cofre truncated",
            Self::BadMagic => "bad magic",
            Self::BadVersion => "unsupported version",
            Self::BadLength => "inconsistent length",
            Self::BadAead => "unknown AEAD selector",
            Self::CofreIdMismatch => "cofre_id != BLAKE3(carga)",
            Self::SignerMismatch => "signer_key_id != pinned key",
            Self::BadSignature => "seal verification failed",
        };
        f.write_str(s)
    }
}

impl core::error::Error for CofreError {}

/// A bounds-checked forward cursor — the parse-before-verify primitive (BLK-7).
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], CofreError> {
        let end = self.pos.checked_add(n).ok_or(CofreError::BadLength)?;
        let slice = self.bytes.get(self.pos..end).ok_or(CofreError::Truncated)?;
        self.pos = end;
        Ok(slice)
    }

    fn take_u32(&mut self) -> Result<u32, CofreError> {
        let a: [u8; 4] = self
            .take(4)?
            .try_into()
            .map_err(|_| CofreError::Truncated)?;
        Ok(u32::from_le_bytes(a))
    }

    fn take_u64(&mut self) -> Result<u64, CofreError> {
        let a: [u8; 8] = self
            .take(8)?
            .try_into()
            .map_err(|_| CofreError::Truncated)?;
        Ok(u64::from_le_bytes(a))
    }

    fn take_arr<const N: usize>(&mut self) -> Result<[u8; N], CofreError> {
        self.take(N)?.try_into().map_err(|_| CofreError::Truncated)
    }
}

fn encode_etiqueta(e: &Etiqueta) -> Vec<u8> {
    let mut v = Vec::with_capacity(ETIQUETA_LEN);
    v.extend_from_slice(&e.route_id);
    v.extend_from_slice(&e.stream_id);
    v.extend_from_slice(&e.seq.to_le_bytes());
    v.extend_from_slice(&e.cofre_id);
    v.extend_from_slice(&e.idempotency_key);
    v.extend_from_slice(&e.contract_fp);
    v.push(aead_to_u8(e.aead_alg));
    v.extend_from_slice(&e.nonce);
    v.extend_from_slice(&e.signer_key_id);
    v.extend_from_slice(&e.eph_pk);
    v.push(u8::from(e.sender_present));
    v.extend_from_slice(&e.ts.to_le_bytes());
    v
}

fn parse_etiqueta(bytes: &[u8]) -> Result<Etiqueta, CofreError> {
    let mut r = Reader { bytes, pos: 0 };
    let route_id = r.take_arr()?;
    let stream_id = r.take_arr()?;
    let seq = r.take_u64()?;
    let cofre_id = r.take_arr()?;
    let idempotency_key = r.take_arr()?;
    let contract_fp = r.take_arr()?;
    let aead_alg = aead_from_u8(r.take(1)?[0]).ok_or(CofreError::BadAead)?;
    let nonce = r.take_arr()?;
    let signer_key_id = r.take_arr()?;
    let eph_pk = r.take_arr()?;
    let sender_present = r.take(1)?[0] != 0;
    let ts = r.take_u64()?;
    Ok(Etiqueta {
        route_id,
        stream_id,
        seq,
        cofre_id,
        idempotency_key,
        contract_fp,
        aead_alg,
        nonce,
        signer_key_id,
        eph_pk,
        sender_present,
        ts,
    })
}

fn signed_region(e: &Etiqueta, carga: &[u8]) -> Vec<u8> {
    let et = encode_etiqueta(e);
    // Both casts are unreachable-by-construction (the etiqueta is a small fixed-width header; `usize` ≤ `u64` on
    // supported targets), but the charter forbids `expect()` in non-test code → saturate instead of panicking.
    let et_len = u32::try_from(et.len()).unwrap_or(u32::MAX);
    let carga_len = u64::try_from(carga.len()).unwrap_or(u64::MAX);
    let mut v = Vec::with_capacity(4 + 1 + 4 + et.len() + 8 + carga.len());
    v.extend_from_slice(&MAGIC);
    v.push(VERSION);
    v.extend_from_slice(&et_len.to_le_bytes());
    v.extend_from_slice(&et);
    v.extend_from_slice(&carga_len.to_le_bytes());
    v.extend_from_slice(carga);
    v
}

/// Encode a cofre to its canonical wire bytes.
#[must_use]
pub fn encode(cofre: &Cofre) -> Vec<u8> {
    let mut v = signed_region(&cofre.etiqueta, &cofre.carga);
    v.extend_from_slice(&cofre.lacre);
    v
}

/// Decode a cofre from wire bytes — **parse-before-verify** (BLK-7): all lengths bound-checked first.
///
/// # Errors
/// Returns [`CofreError`] on bad magic/version, a truncated or inconsistent frame, or an unknown AEAD selector.
pub fn decode(bytes: &[u8]) -> Result<Cofre, CofreError> {
    let mut r = Reader { bytes, pos: 0 };
    if r.take(4)? != MAGIC {
        return Err(CofreError::BadMagic);
    }
    if r.take(1)?[0] != VERSION {
        return Err(CofreError::BadVersion);
    }
    let etq_len = usize::try_from(r.take_u32()?).map_err(|_| CofreError::BadLength)?;
    if etq_len != ETIQUETA_LEN {
        return Err(CofreError::BadLength);
    }
    let etiqueta = parse_etiqueta(r.take(etq_len)?)?;
    let carga_len = usize::try_from(r.take_u64()?).map_err(|_| CofreError::BadLength)?;
    let carga = r.take(carga_len)?.to_vec();
    let lacre = r.take_arr::<LACRE_LEN>()?;
    if r.pos != bytes.len() {
        return Err(CofreError::BadLength);
    }
    Ok(Cofre {
        etiqueta,
        carga,
        lacre,
    })
}

/// Seal a record's `carga` into a cofre: set `cofre_id = BLAKE3(carga)` and `signer_key_id`, then sign the
/// whole region with the domain-separated `lacre`. The caller supplies a fully-populated `etiqueta` (its
/// `cofre_id`/`signer_key_id` are overwritten here).
#[must_use]
pub fn seal(mut etiqueta: Etiqueta, carga: Vec<u8>, signing_seed: &[u8; 32]) -> Cofre {
    etiqueta.cofre_id = blake3_256(&carga);
    etiqueta.signer_key_id = blake3_256(&verifying_key(signing_seed));
    let region = signed_region(&etiqueta, &carga);
    let lacre = sign_domain(ctx::LACRE, signing_seed, &region);
    Cofre {
        etiqueta,
        carga,
        lacre,
    }
}

/// Verify a cofre against the **route-pinned** verifying key (BLK-4).
///
/// # Errors
/// Returns [`CofreError::CofreIdMismatch`] if `cofre_id != BLAKE3(carga)` (BLK-8),
/// [`CofreError::SignerMismatch`] if the `signer_key_id` does not match the pinned key (BLK-4), or
/// [`CofreError::BadSignature`] if the `lacre` does not verify.
pub fn verify(cofre: &Cofre, pinned_vk: &[u8; 32]) -> Result<(), CofreError> {
    if cofre.etiqueta.cofre_id != blake3_256(&cofre.carga) {
        return Err(CofreError::CofreIdMismatch);
    }
    if cofre.etiqueta.signer_key_id != blake3_256(pinned_vk) {
        return Err(CofreError::SignerMismatch);
    }
    let region = signed_region(&cofre.etiqueta, &cofre.carga);
    if verify_domain(ctx::LACRE, pinned_vk, &region, &cofre.lacre) {
        Ok(())
    } else {
        Err(CofreError::BadSignature)
    }
}

#[cfg(test)]
mod tests {
    use super::{decode, encode, seal, verify, CofreError};
    use datarail_core::{AeadAlg, Etiqueta};
    use datarail_crypto::verifying_key;

    const SEED_A: [u8; 32] = [11u8; 32];
    const SEED_B: [u8; 32] = [22u8; 32];

    fn fixture() -> Etiqueta {
        Etiqueta {
            route_id: [1; 16],
            stream_id: [2; 16],
            seq: 42,
            cofre_id: [0; 32],
            idempotency_key: [4; 32],
            contract_fp: [5; 32],
            aead_alg: AeadAlg::Gcmsiv256,
            nonce: [6; 12],
            signer_key_id: [0; 32],
            eph_pk: [8; 32],
            sender_present: false,
            ts: 0,
        }
    }

    #[test]
    fn roundtrip_and_verify() {
        let cofre = seal(fixture(), b"opaque-ciphertext".to_vec(), &SEED_A);
        let bytes = encode(&cofre);
        let decoded = decode(&bytes).expect("decode");
        assert_eq!(decoded, cofre);
        verify(&decoded, &verifying_key(&SEED_A)).expect("verify");
    }

    #[test]
    fn ac2_mutate_every_byte_is_rejected() {
        // AC-2: a sealed cofre, with ANY single byte flipped, must never decode-and-verify.
        let cofre = seal(fixture(), b"opaque-ciphertext".to_vec(), &SEED_A);
        let vk = verifying_key(&SEED_A);
        let bytes = encode(&cofre);
        for i in 0..bytes.len() {
            let mut m = bytes.clone();
            m[i] ^= 0x01;
            let accepted = decode(&m).is_ok_and(|c| verify(&c, &vk).is_ok());
            assert!(!accepted, "byte {i} flip was accepted");
        }
    }

    #[test]
    fn ac3_wrong_and_forged_key_rejected() {
        // AC-3: verifying against any key other than the sealer's is rejected (BLK-4).
        let cofre = seal(fixture(), b"opaque-ciphertext".to_vec(), &SEED_A);
        assert_eq!(
            verify(&cofre, &verifying_key(&SEED_B)),
            Err(CofreError::SignerMismatch)
        );
        // A cofre forged by B, verified against the pinned key A, is rejected.
        let forged = seal(fixture(), b"opaque-ciphertext".to_vec(), &SEED_B);
        assert_eq!(
            verify(&forged, &verifying_key(&SEED_A)),
            Err(CofreError::SignerMismatch)
        );
    }
}
