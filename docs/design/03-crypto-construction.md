# 03 — Crypto Construction

> **MF-2 SPEC. Status: DRAFT (hardened by AUDIT-01).** Pins the exact primitives behind the cofre (02).
> Charter: `forbid(unsafe)`, audited RustCrypto/dalek crates only, every choice traceable.

## Primitives

| Role | Choice | Notes |
|---|---|---|
| Hash | **BLAKE3-256** | `cofre_id`, `contract_fp`, Merkle (04) |
| AEAD (pluggable, `aead_alg`) | **default AES-256-GCM-SIV** (nonce-misuse-RESISTANT) · ChaCha20-Poly1305 (12-B nonce; non-AES-NI/non-x86) · AES-256-GCM (opt-in perf, **gated on key-uniqueness**) | 256-bit keys |
| Signature | **Ed25519** (domain-separated — below) | lacre · STH · ack · sender-cert · ticket |
| KEM / key-wrap | **X25519** ECDH → HKDF-SHA256 → AEAD-wrap | data-key wrapping |
| KDF | **HKDF-SHA256** | — |

## Key hierarchy

- **Route identity (long-lived, pinned at F2):** each endpoint holds an Ed25519 signing keypair + an X25519
  KEM keypair; routes pin peer statics (Noise_KK).
- **Per-cofre data key (ephemeral):** 256-bit, fresh CSPRNG per cofre; wrapped to the destination (sender
  ephemeral X25519 → ECDH → HKDF → AEAD-wrap) in `wrapped_data_key`. Rotation re-wraps data keys, not payloads.

## DECISION — AEAD default = GCM-SIV (refines the earlier "plain AES-GCM"; AUDIT-01 BLK-1)

The default is **AES-256-GCM-SIV**: nonce-misuse-**resistant**, so a repeated `(key, nonce)` leaks at worst
*equality*, never the catastrophic GCM failure. This (a) matches `DECOMPOSITION` A3's demand, (b) removes the
**fork-without-reseed footgun** of "fresh key ⇒ random nonce is safe" (that premise was ungated). The fresh
per-cofre key is kept as **defense-in-depth**. **Plain AES-256-GCM** stays available as an opt-in
high-throughput mode **only behind a key-uniqueness gate** (per-process CSPRNG reseed-on-fork asserted).
**No derived nonce, no convergent encryption.** Dedup is the header `idempotency_key = HMAC-BLAKE3(tenant_secret,
record_key)` (record-key, matches `upsert_key`) — so the rail dedups on an opaque token, plaintext never exposed.

## Audit-driven hardening (AUDIT-01)

- **Signer verification (BLK-4) — soundness-critical.** A verifier MUST validate `LACRE` **only** against the
  **route-pinned** Ed25519 pubkey. `signer_key_id` (in the etiqueta) is a *selector among pre-authorized pins*,
  never a key source; **hard-reject if `signer_key_id ≠ hash(pinned_key)`**. (Closes signer-substitution forgery;
  added to AC-3.)
- **Domain separation (BLK-5).** Every signature is `Ed25519(CTX ‖ msg)` with a unique, fixed-width context
  label per role: `dr:lacre:v1` · `dr:sth:v1` · `dr:ack:v1` · `dr:cert:v1` · `dr:ticket:v1`. No message shape may
  alias another. (Closes cross-protocol signature confusion.)
- **Sender-cert (MAJ-4).** `SENDER_CERT` must be signed by a **route-authority/issuer** binding `sender_id ↔ key`
  (not merely self-signed), **bound to `cofre_id` + an epoch**, with a short expiry and a revocation list checked
  at the destination.
- **Metadata residual (honest).** The cleartext etiqueta leaks `signer_key_id` (links a sender's cofres),
  `contract_fp`, and a **deterministic-per-record-key `idempotency_key`** (the rail can fingerprint per-key
  activity over time). Stated plainly; mitigation: **per-epoch salt** on the dedup token + rotate/omit
  `signer_key_id` exposure. Not sold as solved.

## Forward secrecy / PQ (scoped)

Ephemeral-static X25519 wrap gives per-cofre sender FS; full double-ratchet is out of scope (fixed-route batch).
PQ: hybrid **X25519 + ML-KEM** wrap is a post-freeze option — `wrapped_data_key` is length-prefixed to allow it.
