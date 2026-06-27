# KAFKA-EOS-DESIGN — exactly-once Kafka ingest into a transactional sink (the v2 moat; upgrades F2's at-least-once)

> The audit (F1/F2) correctly demoted `kafka-ingest` to at-least-once: it funnelled all partitions into one
> `route_id` stream whose cross-restart interleave is not a stable position. This is the honest fix — genuine
> exactly-once for an **idempotent Kafka producer**, keyed on the producer's own stable per-record identity.

## The key realization — use the PRODUCER's identity, not our broker's offset
Our broker assigns offsets **in-memory, non-durably**, and a producer retry gets *fresh* offsets — so the offset
is NOT a stable dedup key across broker restart or producer retry. But Kafka's **idempotent producer**
(`enable.idempotence=true`, the default for modern clients / required for Kafka's own EOS) stamps every v2
`RecordBatch` with `(producer_id, producer_epoch, base_sequence)`, and these are **identical on every retry** of
the same batch. The per-record identity is therefore `(producer_id, partition, sequence)` — stable across retry,
broker restart, and partition interleave. That is the dedup coordinate datarail keys on.

A non-idempotent producer (`producer_id == -1`, legacy `MessageSet`) has NO stable identity — its retries are
genuinely indistinguishable — so it CANNOT be exactly-once (this matches Kafka itself). Such a stream stays
**at-least-once (Tier C)**, honestly.

## The model — per `(producer_id, partition)` substream, sequence watermark
- Each v2 batch from `(producer_id, partition)` carries `base_sequence S` and `record_count N` → it occupies the
  contiguous sequence range `[S, S+N)`. Within a `(producer_id, partition)` the producer emits batches in strict
  sequence order, and TCP + our single-threaded per-connection handling preserve that order → batches arrive as
  **whole units in sequence order**. So the durable watermark for that substream is a clean batch boundary.
- **Dedup watermark** (stored transactionally in the sink, per substream) = the highest sequence processed.
  - `stream` id = `route_id ++ topic ++ partition ++ producer_id` (distinct substreams never collide).
  - `high = S + N` (the sequence after this batch).
  - **commit_at_seq(fresh, stream, high):** in ONE txn — if `stored >= high` ⇒ no-op (this batch already
    processed: an idempotent-retry or a post-crash replay); else `COPY` the `fresh` records and set the stored
    watermark to `high`. Atomic: records + watermark commit together or neither.
- **Dead-letters advance the watermark.** If a record in `[S, S+N)` violates the offload contract it is
  dead-lettered (correctly not landed), but the whole range `[S, S+N)` is *processed*, so the watermark still
  advances to `high`. A replay of `[S, S+N)` is then a no-op — no re-board, no double-land, no resurrection of a
  record that was legitimately rejected.

### Why no partial-overlap (suffix) logic is needed here (unlike the file path)
The file path (`commit_at`) lands `records[stored-base..]` to handle a *grown file re-read from the start* — a
batch whose range straddles the stored watermark. Kafka is different: an idempotent producer resends an
**identical** batch (same `S`, same `N`), and the broker delivers whole batches in order, so the stored watermark
is **always** a batch boundary — either `stored >= high` (whole batch done → no-op) or `stored == S` (the next
contiguous batch → land all `fresh`). The straddle case does not arise. We assert the boundary model and keep
`commit_at_seq` a clean whole-batch idempotent op (no per-record sequence indexing, which dead-letters would
otherwise complicate).

## Crash / retry analysis
| event | on next presentation | outcome |
|---|---|---|
| producer retry of `[S,S+N)` (broker didn't ack in time) | `stored >= S+N` ⇒ no-op | once ✓ |
| `kafka-ingest` daemon crash after landing `[S,S+N)` + watermark (atomic) | replay ⇒ `stored >= S+N` ⇒ no-op | once ✓ |
| daemon crash after landing but before watermark | impossible — same txn (atomic) | once ✓ |
| broker restart (offsets reset) but producer keeps same `producer_id`+seq | watermark in the SINK is unchanged ⇒ no-op on replay | once ✓ |
| a record in the batch dead-letters | watermark still advances to `S+N` | once ✓ (not resurrected) |
| non-idempotent producer (`producer_id == -1`) | no stable id ⇒ Tier C at-least-once | honest |

## Wiring (incremental, each step tested)
1. **`produce.rs`** — surface `(producer_id, base_sequence, record_count)` per `ProducedPartition` (the parser
   already reads them; stop discarding). Legacy `MessageSet` ⇒ `producer_id = -1`.
2. **`serve.rs`** — `ProducedBatch` carries the producer identity through the channel.
3. **`PostgresSink::commit_at_seq`** + `TxnSink` — the whole-batch idempotent op above (reuses the watermark
   table, advisory lock, drain-to-`'Z'` hardening already audited).
4. **`Pipeline::ship_batch_seq`** — board → rail → offload → `AnySink::commit_seq(fresh, stream, high)`.
5. **CLI `kafka-ingest`** — its own ship loop: per batch build `stream` + `high`; use Tier A **iff** the sink is
   transactional AND `producer_id >= 0`; otherwise Tier C (at-least-once). The in-process once-gate `record_key`
   = `kafka-{topic}-{partition}-{producer_id}-{base_seq}` (stable across retry).
6. **Tests** — unit (parser surfaces the identity; `commit_at_seq` idempotent/dead-letter); LIVE against real
   Postgres (replay a batch ⇒ no double-land; dead-letter advances watermark); CI gate with an **idempotent**
   producer (`kcat -X enable.idempotence=true` / librdkafka) sending duplicates → exactly-once rows.
7. **Adversarial audit** of the new path (correctness-critical — audited like D-3 / the txn path).

## Honest scope
- Exactly-once requires an **idempotent producer**; a non-idempotent one is at-least-once (stated, not faked).
- This delivers exactly-once **into the sink** (the datarail guarantee). It does not turn our broker into a
  durable Kafka log (we don't persist the producer log); the SINK's watermark is the source of truth, which is
  exactly what the provider-blind rail wants — no external dedup state.
