# KAFKA-EOS-DESIGN — exactly-once Kafka ingest into a transactional sink (the v2 moat; upgrades F2's at-least-once)

> **STATUS: BUILT + PROVEN (2026-06-27).** All 7 build-plan steps below are done. `InitProducerId` (API 22) is
> served, the EOS coord is surfaced + threaded, `commit_at_seq` + the per-batch CLI routing land it, and it is
> proven LIVE against real Postgres 16: a duplicated idempotent `Produce` through the real `datarail kafka-ingest`
> binary lands **exactly 3 rows, not 6** (`crates/datarail-cli/tests/kafka_eos_live.rs`); the wire path
> (InitProducerId grant + identical coord on retry) is proven over a real socket
> (`crates/datarail-kafka/tests/eos_wire.rs`); `commit_at_seq` idempotency + dead-letter-gap is proven in
> `postgres_live.rs`. **Honest limit:** exactly-once is *within an idempotent producer session* (same
> `producer_id`) — Kafka's own idempotence guarantee. Cross-session EOS (a stable `transactional.id`) is a
> further increment.
>
> **Adversarial audit + fixes (2026-06-27, see `CORE-AUDIT.md` §Kafka-EOS):** a brutal audit caught a CRITICAL
> **ack-before-durable** hole (the broker acked the producer the instant it enqueued, before the sink landed the
> record → a crash lost acked records, and an idempotent producer never resends an acked batch — the same D-1
> lesson regressed). FIXED: **ack-after-durable** — the serve loop waits for the integration layer's
> durable-landing result before acking; a sink failure becomes a **retriable error code** (KAFKA_STORAGE_ERROR),
> never a false NONE ack, and the daemon **keeps serving** (one failure must not tear down ingest). The fix
> required two follow-ons, both done: (a) **per-batch snapshot** of fresh records (not a global cursor) so a
> failed batch never mis-attributes its records to a later batch now that the loop continues on error; (b) a
> **unique in-process record_key per received batch** so the offload once-gate never short-circuits a retry — the
> sink's durable watermark is the SOLE EOS authority (a retry re-delivers and is idempotently no-op'd at the sink;
> a prior commit failure re-commits). Also: **`producer_epoch` folded into the substream id** (audit D) so an
> epoch bump (sequence resets to 0) forms a fresh substream instead of being wrongly no-op'd (loss). Proven by
> the extended `kafka_eos_live.rs` (sink failure → retriable error + daemon survives + recovers).
>
> **Long-running memory (FIXED 2026-06-27 — the streaming-terminal arc):** the offload terminal's sink used to
> **retain every landed record for the daemon's lifetime** → OOM under sustained load. Now the pipeline **drains
> the terminal sink every batch** (`sink_mut().take_committed()`, cumulative count kept in a `landed_total`
> counter), so memory is bounded by one batch. The **dead-letter siding is also bounded** (`MAX_DEAD_LETTERS_
> RETAINED = 1024`; older diversions evicted + counted) so a producer flooding contract-violations can't OOM the
> destination (regression `dead_letter_siding_is_bounded_under_a_flood`). The once-gate stays bounded by its
> existing watermark+GC (seqs are contiguous in-order per source terminal). The EOS daemon is now long-running-safe.


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
