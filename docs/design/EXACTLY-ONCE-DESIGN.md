# EXACTLY-ONCE-DESIGN — exactly-once delivery to the sink, durable across crashes (closes audit D-3)

> The hard problem the audit exposed: "effectively-once" was in-memory only — a destination crash forgot the
> dedup state and re-landed redeliveries. Kafka and friends punt this to "use a transactional consumer." We solve
> it for real, and turn it into a moat: **exactly-once INTO the sink, with the dedup watermark stored
> transactionally in the sink itself — no external dedup state to keep in sync, provider-blind end to end.**

## The precise problem
A record R may be redelivered (at-least-once transport / replay). R's effect on the sink must happen **exactly
once**, even across a crash at any instant. Per-record state machine:

```
[arrived] → [admitted: dedup says "new"] → [landed in sink] → [dedup-committed: watermark persisted]
```

Crash analysis (where it breaks):

| crash point | on restart (R redelivered) | outcome |
|---|---|---|
| before admit | admit fresh → land → commit | once ✓ |
| after admit, before land (dedup in RAM) | dedup forgot → admit → land → commit | once ✓ |
| **after land, before dedup-commit** | **dedup forgot → land AGAIN** | **DUPLICATE ✗** |
| after dedup-commit | dedup says Duplicate → skip | once ✓ |

**The entire problem is the one window: "landed but dedup not yet durable."** Eliminate it by making **land +
dedup-commit a single atomic step.** Everything else follows.

## The solution, tiered by what the sink can do (give each sink the strongest guarantee it supports)

### Tier A — transactional sink (Postgres, the v1 product): TRUE exactly-once
Store the dedup **watermark inside the sink's own transaction**, alongside the records:

```sql
BEGIN;
  SELECT seq FROM datarail_watermark WHERE stream = $s FOR UPDATE;   -- current durable watermark
  -- if stored_seq >= batch_high_seq:  COMMIT (no-op) — this batch already landed (idempotent replay)
  -- else:
  COPY <table> (...) FROM STDIN;                                     -- land the new records
  INSERT INTO datarail_watermark(stream, seq) VALUES ($s, $high)
    ON CONFLICT (stream) DO UPDATE SET seq = EXCLUDED.seq;           -- advance the watermark
COMMIT;                                                              -- records + watermark atomic
```

- **Atomic:** a crash leaves both the records and the watermark committed, or neither. The "landed but dedup not
  durable" window is gone — they are the same commit.
- **Idempotent on replay:** re-presenting the same batch finds `stored_seq >= batch_high_seq` → no-op. No dup.
- **Resume needs NO external state:** on open, read the watermark FROM the sink; the source resumes there. The
  sink IS the dedup store. Nothing to keep in sync, nothing to lose separately.
- This is the **moat**: exactly-once into your database with zero operational dedup state, and (because datarail
  seals upstream) the rail/pipe still never see plaintext — provider-blind AND exactly-once.

### Tier B — idempotent sink (sink dedups by key): effectively-once
datarail already carries a stable per-record `idempotency_key`. A sink that can `INSERT … ON CONFLICT
(idempotency_key) DO NOTHING` gets effectively-once for free (at-least-once delivery × idempotent apply).

### Tier C — plain sink (file, webhook): honest at-least-once, bounded
No atomicity available. With a durable dedup index (persisted watermark, fsync'd), a restart no longer floods
duplicates — at most the single in-flight record at the crash instant can repeat (land-then-commit ordering) or
be lost (commit-then-land ordering). We pick **land-then-commit (at-least-once, bounded dup of 1)** and say so.
True exactly-once to a plain sink is impossible without sink cooperation; we state that plainly rather than fake it.

## The interface
A capability trait, additive to the existing `Sink` (Tier C stays the default):

```rust
/// A sink that can land a batch AND record a monotonic watermark ATOMICALLY (Tier A). `commit_at` must be a
/// single atomic, idempotent operation: if `watermark <= the sink's stored watermark`, it is a no-op (the batch
/// already landed); otherwise it lands `records` and advances the stored watermark to `watermark`, all-or-nothing.
pub trait TxnSink {
    fn commit_at(&mut self, records: &[Vec<u8>], stream: &[u8], watermark: u64) -> io::Result<()>;
    fn resume_watermark(&mut self, stream: &[u8]) -> io::Result<u64>;
}
```

`PostgresSink` implements `TxnSink` (the SQL above). The offload path, when its sink is a `TxnSink`, feeds the
cofre's monotonic seq as the watermark — delivering Tier A exactly-once. A non-`TxnSink` falls back to Tier C.

## Guarantees, stated honestly (this is what we will claim — nothing more)
- **Postgres (Tier A): exactly-once across crashes, PROVEN against a real Postgres with crash simulation.**
- Idempotent sink (Tier B): effectively-once (sink-side dedup on the provided key).
- File/webhook (Tier C): at-least-once with a bounded (≤1) dup window across a crash; NOT exactly-once
  (impossible without sink cooperation) — said plainly.

## Build plan (careful increments, each tested; the durable component gets its own adversarial audit)
1. **This doc** — the guarantee model + protocol (design-before-code). ✅
2. `TxnSink` trait + `PostgresSink::commit_at`/`resume_watermark` (the watermark table + the atomic txn).
3. **Live crash test** against docker Postgres: land batch, simulate crash mid-protocol, re-present → assert
   exactly-once (the row count is correct, the watermark consistent).
4. Wire Tier A into the offload/CLI path (feed the seq as the watermark); fall back to Tier C otherwise.
5. **Adversarial audit** of the new transactional path (it is correctness-critical — audit it like the core).

## Integration plan — wiring Tier A into the product (`datarail run` / `kafka-ingest`)
Tier A is implemented + proven + CI-gated at the `PostgresSink` level (`commit_at`). The remaining step is making
the product flows USE it so exactly-once is end-to-end, not just available:

- **Sink selection** returns a capability-typed sink, not a bare `Box<dyn Sink>`: either `Plain(Box<dyn Sink>)`
  (file/webhook/memory → `commit`) or `Txn(PostgresSink)` (→ `commit_at`). The CLI already knows when the sink is
  Postgres, so no downcast/`Any` is needed.
- **The watermark per source** (monotonic, deterministic on replay):
  - `datarail run` over a file/replay source: a cumulative record count (`watermark += batch.len()` per batch) —
    a replayed run re-presents the same batches in order, so `commit_at` no-ops the already-landed prefix.
  - `kafka-ingest`: the Kafka **offset** is the natural watermark (already monotonic per topic/partition); feed
    `base_offset + batch.len()` so a producer re-send is idempotent at the sink.
- **The stream id** = the route's `route_id` (from `rail.toml`) so distinct rails don't collide in the watermark
  table.
- **Flag/auto:** Postgres sink → Tier A automatically (it's strictly better); `--at-least-once` opts out if a
  caller wants the plain COPY path.

This keeps `run_pipe`'s shape, adds one `Txn` branch, and makes "exactly-once into Postgres" the default for the
v1 product flow — verified by extending the e2e CI gate to replay a `datarail run` and assert no double-land.
