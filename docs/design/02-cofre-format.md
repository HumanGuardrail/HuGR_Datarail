# 02 — The Cofre Wire Format

> **MF-2 SPEC. Status: DRAFT (hardened by AUDIT-01).** The on-wire byte layout of a cofre — the keystone;
> everything seals, verifies, routes, and dedups off it, and MF-3 freezes it. Traces to `00-CONSTITUTION.md`,
> `01-rail-surface.md` (H1), `03-crypto-construction.md`, and the competitive teardown.

## Top-level layout

```
COFRE := MAGIC(4) ‖ VERSION(1) ‖ ETIQUETA_LEN(u32) ‖ ETIQUETA ‖ CARGA_LEN(u64) ‖ CARGA ‖ LACRE(64)
```

- `MAGIC = b"DRLC"`, `VERSION = 0x01`. Integers little-endian, fixed-width, length-prefixed — a **canonical**
  encoding (exactly one byte sequence per logical cofre), so the signature is unambiguous (`INV-SEAL-COMPLETE`).
- `LACRE` = `Ed25519("dr:lacre:v1" ‖ <MAGIC..end of CARGA>)` — domain-separated (03 BLK-5), covering every byte
  before it. No field can be silently mutated.

## Verification order (parse-before-verify guard — BLK-7)

A receiver (rail or terminal) MUST, as **step 0**, bound-check `ETIQUETA_LEN`/`CARGA_LEN` with checked
arithmetic against the frame size **before reading any field or allocating** → then verify `LACRE` → only then
parse the etiqueta / decrypt. Malformed lengths are rejected pre-allocation (no OOB / over-alloc DoS).

## ETIQUETA — authenticated header (clear to the rail; covered by the seal)

| Field | Size | Purpose | Rail use |
|---|---|---|---|
| `route_id` | 16 B | fixed A→B route (F1) | route |
| `stream_id` | 16 B | ordering domain | order |
| `seq` | u64 | per-stream sequence | order, checkpoint, **watermark-reject (05/BLK-6)** |
| `cofre_id` | 32 B | `BLAKE3(CARGA)` — **identity** (content address); == the manifest "ciphertext hash" | manifest (**not** a dedup key — MAJ-6) |
| `idempotency_key` | 32 B | `HMAC-BLAKE3(tenant_secret, record_key)` — the **sole** effectively-once dedup key | dedup |
| `contract_fp` | 32 B | `BLAKE3` of the content contract this cofre claims | (passed to dest) |
| `aead_alg` | u8 | `01`=AES-256-GCM-SIV (**default**) · `02`=ChaCha20-Poly1305 · `03`=AES-256-GCM (opt-in, key-uniqueness-gated) | — |
| `nonce` | 12 B | **random** per cofre; with GCM-SIV default, reuse leaks at worst equality (safe) | — |
| `wrapped_data_key` | len-pref | per-cofre data key, X25519-wrapped to the dest (03) | — |
| `signer_key_id` | 32 B | selector among **pre-authorized route pins**; verifier MUST validate `LACRE` only against the route-pinned key and reject on mismatch (BLK-4) | verify seal |
| `sender_present` | u8 | sealed-sender flag; sender identity lives **inside** `CARGA` | — |
| `ts` | u64 ms | informational only — never trusted for ordering (`seq`) or for proof time (04) | — |

> **Stolen patterns embodied:** content-address identity (`cofre_id`), HMAC-per-tenant-**record_key** dedup in
> the header (no convergent encryption, no equality leak beyond the token), **GCM-SIV default** (misuse-resistant),
> envelope encryption (`wrapped_data_key`), sealed-sender (`sender_present`).

## CARGA — AEAD payload (opaque to the rail)

```
CARGA := AEAD[aead_alg].encrypt(key=data_key, nonce=nonce, aad=canonical(ETIQUETA), plaintext=INNER)
INNER := [SENDER_CERT]? ‖ RECORD_BATCH
```

- **AAD = the etiqueta** (byte-identical to the on-wire etiqueta — one encoder, asserted) → binds header to
  payload beneath the outer seal.
- `SENDER_CERT` (sealed-sender): issuer-signed, bound to `cofre_id`+epoch (03 MAJ-4); validated only by the dest.
- `RECORD_BATCH`: **many records per cofre** — one Ed25519 signature amortized over the batch (the H3 finding,
  structural). Inner framing pinned in the terminal/record doc.

## LACRE — two seals, two jobs

- **AEAD tag (inner):** integrity + header-binding for the data-key holder (destination).
- **Ed25519 `LACRE` (outer, domain-separated):** public verifiability + non-repudiation, and lets the **keyless
  rail reject a tampered/forged cofre in transit without opening it** (`INV-TAMPER-REJECT`).

## Overhead budget (featherweight) — `bet`, pending a `sizeof` test

| Part | bytes |
|---|---|
| MAGIC+VERSION+lengths | 17 |
| etiqueta fixed fields | ~185 |
| `wrapped_data_key` (X25519 eph-pub + wrapped 256-bit key + tag) | ~80 |
| AEAD tag | 16 |
| LACRE | 64 |
| **fixed total** | **≈ 362 B/cofre** |

Per *record* → ~0 by batching (a 1 MiB batch ⇒ ~0.035% overhead). The fixed per-cofre cost is exactly why
`RECORD_BATCH` is mandatory. (Number labeled `bet` until a `sizeof` test backs it — Charter L2.)

## What the rail can / cannot do (H1)

- **CAN:** bound-check; verify `LACRE` over opaque bytes; route/order/dedup(on `idempotency_key`)/backpressure/dead-letter.
- **CANNOT:** decrypt `CARGA`, read `SENDER_CERT`, or read any record.

## Open SPEC items (remaining)

- `RECORD_BATCH` inner framing (Arrow IPC vs custom) → terminal/record doc.
- Max cofre size + chunking for large payloads → **owned by `07`** (ties to E4 `bao`); single owner, not duplicated here.
