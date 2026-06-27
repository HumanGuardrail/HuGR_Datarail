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

## ⚠️ Scope correction (audit F1/F2, 2026-06-27) — the watermark is POSITIONAL; Tier A needs an append-ordered source
A brutal self-audit of the wiring caught a real over-claim. The watermark is a cumulative **position** (count of
records landed), and `commit_at` lands `records[stored-base..]` **positionally** — it never compares record
identity. That is sound only when the source re-presents records in a **stable, append-ordered** sequence on
replay (record N is always the same record). It is NOT sound for:
- **an arbitrary HTTP GET** — the endpoint may return rows reordered or with mid-stream insertions between runs;
- **Kafka ingest as wired** — all `(topic,partition)`s funnel into one `route_id` stream whose cross-restart
  interleave is not a stable position (F2).

For those, a positional watermark could **skip a genuinely-new record (loss) or re-land one (dup)** — PROVEN by a
PoC (run1 `[A,B]`, run2 `[A,C,B]` → DB `[A,B,B]`, C lost). **Resolution:** Tier A is gated on BOTH opt-in (default
on) AND an **append-ordered source** (`wants_tier_a` in the CLI). File / replay / inline → Tier A exactly-once.
**HTTP and Kafka-ingest → at-least-once (Tier C)** — no false guarantee, and Tier C never loses a record. Genuine
per-partition-offset Kafka exactly-once (and append-only-feed HTTP exactly-once) are tracked future increments.
The append-only file contract is the natural one (a log only grows); reordering a source file mid-stream between
runs is an operator contract violation, not a default product path.

## ✅ Integration DONE — Tier A wired into the product, PROVEN end-to-end for append-ordered sources (2026-06-26, scoped 06-27)
Tier A is the **default** for the Postgres sink **when the source is append-ordered** (`datarail run --source-file`
/ replay / inline). `datarail run --source-http` and `kafka-ingest` are at-least-once. `--at-least-once` opts out.

**Wiring (as built):**
- Sink selection returns a capability-typed `AnySink` — `Plain(Box<dyn Sink>)` (file/webhook/memory → `commit`,
  Tier C) or `Txn(Box<PostgresSink>)` (→ `commit_at`, Tier A). The CLI knows when the sink is Postgres, so no
  `Any`/downcast. Shared by `run` and `kafka-ingest` via `select_sink`.
- **Watermark = cumulative count of records LANDED for the stream** (the `Pipeline`'s commit cursor), not boarded —
  so it tracks the durable sink position regardless of dead-letters/dedup. Stream id = the route's `route_id`.
- The run report prints the honest guarantee line (`guarantee = exactly-once into Postgres (Tier A …)` / `at-least-once (Tier C)`).

**Correctness refinement found + fixed while wiring (record-granularity, not batch-granularity):** a `LineFileSource`
reads the whole file as ONE batch, so a source that GREW between runs re-presents a *larger* batch under a higher
cumulative watermark. Batch-granularity idempotency (`if watermark <= stored: no-op else land ALL`) would re-land the
overlap. `commit_at` now lands only the suffix past the stored watermark — `base = watermark - records.len()`, land
`records[(stored - base)..]`. Proven by the `a_grown_replay_batch_lands_only_the_new_suffix_not_the_overlap` live test.

**MEASURED proof (against a real Postgres 16, independently verified at the DB level):**
- `datarail run --source-file … --sink-postgres` run **twice** (identical replay, fresh process → empty in-memory
  dedup) → **4 rows, not 8**; append one line + re-run → **4, not 7**. The runs produced *different cofre_ids* yet
  the replays landed zero/only-new rows — the Postgres-resident watermark (not in-memory state) enforces
  exactly-once across invocations. `--source-http` reports `at-least-once (Tier C)` (proven: guarantee line + 3 rows).
- `--at-least-once` opt-out → **8 rows** (doubles, as labelled). Both paths behave exactly as their guarantee claims.
- Live tests: batch + 3 replays → 4 rows / watermark 4; grown-source partial-overlap → 4 (overlap not re-landed);
  backend-error-does-not-desync → durable watermark survives. CI gate (`connectors-live.yml`) now asserts the
  end-to-end replay (run twice → 3 rows, not 6) + the grown-batch partial-overlap.

### Original integration plan (for the record)
The remaining step was making the product flows USE Tier A so exactly-once is end-to-end, not just available:

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
