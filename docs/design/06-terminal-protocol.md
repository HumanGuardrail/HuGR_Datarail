# 06 — Terminal Protocol (onboarding / offloading)

> **MF-2 SPEC. Status: DRAFT.** Capability D / AC-2,3,9. Terminals are **our mechanism running in the user's
> trust zone** (they hold the keys, see plaintext). The rules are the **user's policy** (D4). This is where
> `INV-CONTRACT-SPLIT` lives: content contract here, envelope contract at the rail.

## Onboarding (source terminal)

1. Pull/receive records from the source connector (plaintext, user's zone). CDC-log-based capture (Debezium
   pattern) lives **here**, at the connector — never in the rail.
2. **Enforce the content contract** (user's declarative rules): schema, validation, required fields,
   transforms. A failing record → onboarding dead-letter, **never boards** (AC-9, boarding side).
3. Batch conforming records → `RECORD_BATCH`; compute `idempotency_key` per record-key; build the cofre (02):
   fresh data key, AEAD-encrypt, etiqueta (incl. `contract_fp`), Ed25519 sign; WAL it (05); hand to the rail.

## Offloading (destination terminal)

1. Receive cofre. **Verify `LACRE`** (Ed25519, pinned source key). Fail → dead-letter (`INV-TAMPER-REJECT`,
   AC-2/3).
2. Unwrap data key (X25519); **AEAD-open** (verifies tag + `aad = etiqueta`).
3. **Dedup** on `idempotency_key` (05).
4. **Enforce the offloading content contract** on the now-plaintext records; check `contract_fp` against the
   expected schema version → **schema drift = a managed event, not silent corruption**. Fail → dead-letter
   (AC-9, offload side).
5. **Commit** transactionally / idempotently to the sink connector (upsert-by-key) → ack → manifest (04).

## Dead-letter (the siding) — D3

Two sidings, both reason-coded and preserved, never silently dropped, never delivered:
- **onboarding** (content-contract fail at source),
- **offloading** (seal fail, or content-contract/schema fail at dest).

## Contract = declarative policy — D4

The content contract is config/data (the rail spec, 09), loaded by our terminal binary — **no recompile per
rule**. Terminal = mechanism (ours); contract = policy (the user's). Connectors (api/kafka/postgres/…) are our
catalog, configured by the user.
