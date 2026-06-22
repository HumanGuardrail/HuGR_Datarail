# 05 — Effectively-Once & Checkpoint

> **MF-2 SPEC. Status: DRAFT.** Capability C / AC-4. *"Acked at boarding ⇒ offloaded exactly once"* under
> crash / retry / partition — honestly **effectively-once** (at-least-once delivery + idempotent commit).

## Durability anchors (NOT in the rail — INV-EPHEMERAL-RAIL)

1. **Source-terminal WAL** — a cofre is durable (WAL + manifest leaf) *before* it is acked to the producer.
   Survives source crash.
2. **The cofre** — content-addressed, immutable, self-verifying — re-sendable from the WAL until dest-acked.
3. **The manifest** — the record of what's been dest-acked = the checkpoint.

## Delivery loop (idempotent)

- Source sends `(cofre, seq, idempotency_key)`. The rail may deliver ≥1× (at-least-once), or the ephemeral
  rail dies mid-flight → re-spawn resends from the last un-acked `seq`.
- Destination: verify seal → check `idempotency_key` against the **dedup index** → if seen, **drop + re-ack**
  (idempotent); else **commit** (transactional / upsert-by-key into the sink) → record key → ack.
- **Exactly-once at the sink** = at-least-once delivery + dedup + transactional/idempotent commit (a clause of
  the offloading contract; only as strong as the sink allows).

## Dedup index — unbounded, crypto-anchored (fixes NATS's 2-min RAM window)

- Keyed by `idempotency_key` (32 B), **persisted** (not an in-RAM time window). 
- Compaction: once a `seq` watermark is durably committed **and** the source's checkpoint has advanced past
  it, keys below the watermark are GC'd (the source never resends below an acked watermark).

## Crash matrix (proof = DST, hypothesis H2)

| Event | Outcome |
|---|---|
| Source crash pre-ack | resend from WAL → no loss |
| Dest crash post-commit pre-ack | source resends → dedup hits → re-ack → no dup |
| Rail dies mid-flight | re-spawn → resume from checkpoint |
| Partition | retry/backoff → no loss (WAL), no dup (dedup index) |

AC-4 is proven by **DST over N seeds** with fault injection on the dumb pipe (drop/reorder/dup/kill).

## Ordering

Per `stream_id`, deliver in `seq` order; the dest holds a bounded reorder buffer + gap-fill-by-seq request
(the substrate may reorder). Beyond the window → request resend.
