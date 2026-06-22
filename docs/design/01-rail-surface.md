# 01 — The Rail Surface & ZK-Completeness (H1 result)

> Part I result (MF-1). The *rail* is the dumb, ephemeral, provider-blind transport — **not** the
> terminals. This doc (a) enumerates every function the rail performs and (b) proves each computes from the
> authenticated header + seal alone (hypothesis **H1**), never the plaintext payload.
> **Status: H1 CONFIRMED**, conditioned on `INV-SEAL-COMPLETE` + the standard metadata residual (below).

## The cofre, as the rail sees it

A cofre on the wire is three parts; the rail touches only the header and the seal:

- **etiqueta (header)** — *authenticated, NOT secret*. The rail reads it; the seal covers it, so the rail
  can read but cannot forge. Carries: route id, stream id, sequence no., idempotency key
  (`HMAC(tenant, record_key)`), cofre id, byte length, contract fingerprint, wrapped data key, timestamp.
- **carga (payload)** — AEAD ciphertext. **Opaque** to the rail. The rail may move or hash its bytes but
  never derives plaintext.
- **lacre (seal)** — Ed25519 signature over `etiqueta ⊗ ciphertext`. The rail can verify it over opaque bytes.

## Every rail function, and its inputs

| Rail function | Needs | Plaintext? |
|---|---|---|
| Route A→B (fixed) | route id (etiqueta) + route→substrate binding (config) | **no** |
| Order per stream | stream id + sequence no. (etiqueta) | **no** |
| Dedup (effectively-once at the rail) | idempotency key (etiqueta) | **no** — dedups on the opaque HMAC token |
| Checkpoint / resume | cofre id + sequence + ack state (etiqueta + manifest) | **no** |
| Backpressure | cofre byte length + ack timing (etiqueta / transport) | **no** |
| Integrity gate (transit corruption) | seal over `etiqueta ⊗ ciphertext` (opaque bytes) | **no** — verifies bytes, not meaning |
| Dead-letter (seal fails) | seal verdict | **no** |
| Manifest / reconciliation | cofre ids + ciphertext hash + counts | **no** |
| Substrate select / fallback | route config + transport signals | **no** |
| DoS defense (proof-of-IP cookie) | connection metadata | **no** |

**Result: no rail function requires the plaintext. H1 holds.** No function had to be reassigned to the
terminal — content validation was already the terminal's job (`INV-CONTRACT-SPLIT`).

## The two conditions (stated honestly)

1. **`INV-SEAL-COMPLETE` is load-bearing.** The rail reads etiqueta fields in the clear (route, seq,
   idempotency key); it must be impossible to mutate them undetected, else an attacker reroutes, reorders,
   or duplicates. Therefore the seal **must cover the entire `etiqueta ⊗ ciphertext`, byte-for-byte**. The
   header is *authenticated, not secret* — which is exactly what makes it rail-readable yet unforgeable.
2. **Metadata residual (accepted, not solved).** The rail necessarily learns cofre **sizes, timing, route,
   and which idempotency tokens repeat**. This is the standard end-to-end residual (Signal and every E2E
   system carry the same). It does **not** break zero-knowledge — the plaintext is never exposed — but it is
   a metadata-confidentiality caveat to state plainly. Padding / batching are SPEC mitigations, never sold
   as promises.

## Consequence for the SPEC

The rail's interface is fixed by this surface: it is a pure function of
`(etiqueta, ciphertext-bytes, seal, route-config) → { deliver | dead-letter | backpressure }`, with **zero
plaintext access**. This is the seam the terminal SDK plugs into, and the contract that MF-3 will freeze.
