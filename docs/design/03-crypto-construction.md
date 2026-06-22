# 03 — Crypto Construction

> **MF-2 SPEC. Status: DRAFT.** Pins the exact primitives behind the cofre (02). Charter: `forbid(unsafe)`,
> audited RustCrypto/dalek crates only, every choice traceable.

## Primitives

| Role | Choice | Notes |
|---|---|---|
| Hash | **BLAKE3-256** | `cofre_id`, `contract_fp`, Merkle (04) |
| AEAD (pluggable) | default **AES-256-GCM** (VAES) · **ChaCha20-Poly1305** (no-AES-NI/non-x86) · **AES-256-GCM-SIV** (hardened) | 256-bit keys |
| Signature | **Ed25519** | `lacre` + sender-cert |
| KEM / key-wrap | **X25519** ECDH → HKDF-SHA256 → AEAD-wrap | data-key wrapping |
| KDF | **HKDF-SHA256** | — |

## Key hierarchy

- **Route identity (long-lived, pinned at F2):** each endpoint holds an Ed25519 signing keypair + an X25519
  KEM keypair. The route pins peer static pubkeys (Noise_KK — both statics known).
- **Per-cofre data key (ephemeral):** **256-bit, fresh CSPRNG per cofre**, encrypts `CARGA`. Because the key
  is fresh per cofre, `(key, nonce)` uniqueness is **trivial** → plain AES-GCM is nonce-safe with a random
  96-bit nonce. The data key is **wrapped** to the destination (sender ephemeral X25519 → ECDH → HKDF →
  AEAD-wrap) and carried in `wrapped_data_key`. Rotation re-wraps data keys, never payloads (A5).

## DECISION (refines 02): no convergent encryption, no derived nonce

Dedup is the **header `idempotency_key = HMAC-BLAKE3(tenant_secret, record_key)`** — deterministic,
rail-readable, plaintext never exposed. So `CARGA` does **not** need deterministic encryption: per-cofre
fresh key + **random** nonce is simpler and leaks **no plaintext-equality** (only equality of HMAC tokens
within a tenant — the intended dedup signal). *This supersedes 02's "derived nonce" note* (02 updated).

## Sealed-sender

`SENDER_CERT` inside `CARGA`: `{sender_id, sender_ed25519_pub, expiry}`, signed by the sender's identity
key, validated **only** by the destination terminal. The rail sees only the `sender_present` flag.

## Forward secrecy / PQ (scoped, honest)

- The ephemeral-static X25519 data-key wrap gives **sender forward secrecy** per cofre. A full double-ratchet
  is **out of scope** for fixed-route batch transport (documented, accepted).
- **PQ-readiness:** KEM is X25519 today; a hybrid **X25519 + ML-KEM** wrap is a flagged post-freeze option
  (the `wrapped_data_key` field is length-prefixed precisely to allow it without a format break).
