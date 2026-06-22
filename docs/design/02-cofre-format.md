# 02 — The Cofre Wire Format

> **MF-2 SPEC. Status: DRAFT.** The on-wire byte layout of a cofre — the keystone artifact; everything seals,
> verifies, routes, and dedups off it, and MF-3 freezes it. Traces to `00-CONSTITUTION.md` (invariants),
> `01-rail-surface.md` (H1), and the competitive teardown (stolen patterns).

## Top-level layout

```
COFRE := MAGIC(4) ‖ VERSION(1) ‖ ETIQUETA_LEN(u32) ‖ ETIQUETA ‖ CARGA_LEN(u64) ‖ CARGA ‖ LACRE(64)
```

- `MAGIC = b"DRLC"` (DataRaiL Cofre). `VERSION = 0x01`.
- Integers little-endian, fixed-width, length-prefixed — a **canonical** encoding (exactly one byte sequence
  per logical cofre), required so the signature is unambiguous (`INV-SEAL-COMPLETE`).
- `LACRE` = Ed25519 signature (64 B) over **every byte from `MAGIC` through the end of `CARGA`**. The seal
  covers the whole cofre, by construction — no field can be silently mutated (`INV-SEAL-COMPLETE`).

## ETIQUETA — the authenticated header (clear to the rail; covered by the seal)

Fixed order, length-prefixed. *Authenticated, not secret* — the rail reads every field but cannot forge one.

| Field | Size | Purpose | Read by rail |
|---|---|---|---|
| `route_id` | 16 B | fixed A→B route (F1) | route |
| `stream_id` | 16 B | ordering domain | order |
| `seq` | u64 | per-stream sequence | order, checkpoint |
| `cofre_id` | 32 B | `BLAKE3(CARGA)` — content address of the sealed payload (identity) | dedup, manifest |
| `idempotency_key` | 32 B | `HMAC(tenant_secret, record_key)` — effectively-once key | dedup |
| `contract_fp` | 32 B | `BLAKE3` of the content-contract/schema this cofre claims | (passes to dest) |
| `aead_alg` | u8 | `01`=AES-256-GCM (default) · `02`=ChaCha20-Poly1305 · `03`=AES-256-GCM-SIV | — |
| `nonce` | 12/24 B | **random per cofre** — reuse-safe because the data key is fresh per cofre (see 03) | — |
| `wrapped_data_key` | len-pref | per-cofre data key, wrapped under the route key (envelope enc.) | — |
| `signer_key_id` | 32 B | hash of the Ed25519 pubkey that produced `LACRE` (pinned per route, F2) | verify seal |
| `sender_present` | u8 | sealed-sender flag; sender identity lives **inside** `CARGA` | — |
| `ts` | u64 ms | informational only — **never** trusted for ordering (`seq` is) | — |

> **Stolen patterns embodied:** content-address identity (`cofre_id`), HMAC-per-tenant dedup key in the header
> (so dedup needs **no** convergent encryption), per-cofre fresh data key (GCM-safe with a random nonce — no
> equality leak; see 03), envelope encryption (`wrapped_data_key`, rotation without re-encrypt), sealed-sender
> (`sender_present`).

## CARGA — the AEAD payload (opaque to the rail)

```
CARGA := AEAD[aead_alg].encrypt(key = data_key, nonce = nonce, aad = canonical(ETIQUETA), plaintext = INNER)
INNER := [SENDER_CERT]? ‖ RECORD_BATCH
```

- **AAD = the etiqueta** → binds header to payload cryptographically (defense-in-depth beneath the outer
  Ed25519 seal): mutating any header field also breaks AEAD decryption at the destination.
- `SENDER_CERT` (sealed-sender, optional): short-lived `{sender_id, sender_pubkey, expiry}` signed by the
  sender — validated **only** by the destination terminal; the rail never sees who sent it.
- `RECORD_BATCH`: **many records packed into one cofre.** This is the H3 finding made structural — a single
  Ed25519 signature amortized over the whole batch (per-cofre signing caps small-record throughput; batching
  dissolves it). Inner framing (Arrow IPC vs length-prefixed) is pinned in the record/terminal doc.

## LACRE — why an outer signature *and* an inner AEAD tag

Two seals, two jobs — both load-bearing:

- **AEAD tag (inner):** integrity + header-binding for whoever holds the `data_key` (the destination).
- **Ed25519 `LACRE` (outer):** **public verifiability + non-repudiation**, and — critically — it lets the
  **rail, which holds no key, verify integrity/authenticity over opaque bytes.** The dumb pipe can reject a
  tampered or forged cofre in transit *without ever opening it*. This is what makes `INV-TAMPER-REJECT`
  enforceable at an untrusted pipe, and it is the cryptographic basis of the delivery proof (manifest, doc 03).

## What the rail can and cannot do (restating H1)

- **CAN:** read `ETIQUETA`; verify `LACRE` over opaque bytes; route / order / dedup / backpressure / dead-letter.
- **CANNOT:** decrypt `CARGA` (the `data_key` is wrapped under a key the rail never holds), read `SENDER_CERT`,
  or read any record.

## Overhead budget (featherweight)

Fixed overhead ≈ `MAGIC`+`VERSION`+lengths (~17 B) + `ETIQUETA` (~220 B) + AEAD tag (16 B) + `LACRE` (64 B)
≈ **~320 B/cofre**. Per *record* this → ~0 by batching (a 1 MiB batch ⇒ ~0.03% overhead). The tiny per-cofre
fixed cost is precisely why `RECORD_BATCH` (batch-per-cofre) is mandatory, not optional.

## Open SPEC items (resolve within MF-2)

- Exact nonce-derivation KDF (HKDF vs HMAC-truncate) → `03-crypto-construction.md`.
- `wrapped_data_key` scheme (X25519 + HKDF + AEAD-wrap, or KMS envelope) → `03-crypto-construction.md`.
- `RECORD_BATCH` inner framing (Arrow IPC vs custom) → terminal/record doc.
- Max cofre size + chunking for large payloads (ties to E4 `bao` resume) → rail/substrate doc.
