# KAFKA-TXN-DESIGN — transactional producer EOS (the last Kafka exactly-once tier)

> **STATUS: BUILDING (2026-06-27) — Q1–Q4 RATIFIED BY THE TECHLEAD (autonomy directive: design/scope are my call).**
> The owner re-armed the autonomous loop rather than answer Q1–Q4, so per the standing "decide+execute, don't ask
> which-option" directive I ratify them with the recommendations below and build incrementally (each step green +
> the whole tier audited before any exactly-once claim is made — the delicate semantics are managed by the audit,
> not by deferring). RATIFICATIONS: **Q1 = (a)** control COMMIT/ABORT markers are stored as a distinct **un-sealed
> control record** (they carry no secret payload — pure txn-control metadata, like offsets/etiqueta already are →
> provider-blind preserved). **Q2 = (a)** `read_committed` is **edge-filtered** by us (we are the authoritative
> coordinator → return only committed records + a correct LSO + an empty aborted-list; wire-compatible with a
> `read_committed` client, and we never ship aborted plaintext). **Q3 = abort-on-restart** for v1 (txn state is
> runtime; a broker restart aborts in-flight txns and the producer retries — documented; durable txn state later).
> **Q4 = IN SCOPE, build now.** Build order: TxnCoordinator (state machine + epoch fencing, unit-tested) → codec →
> serve wiring → store markers + LSO → `read_committed` Fetch → wire test → brutal audit.
>
> **(original frozen design below.)**
> The idempotent producer EOS (per-partition exactly-once within a session) is already shipped + proven
> (`KAFKA-EOS-DESIGN.md`). This adds the TRANSACTIONAL tier: atomic multi-partition writes + consumer-offsets-in-
> the-transaction + `read_committed` consumers. It is the **hardest** Kafka feature (a transaction coordinator with
> epoch fencing, two-phase-commit control markers in the durable log, and `read_committed` isolation), so it is
> frozen as a design first — and because it raises **provider-blind design questions only the owner should rule on**
> (below), it is explicitly held for ratification rather than built unattended.

## Why this is held for the owner (the rigor-compact reason)
Transactional EOS semantics are unforgiving: a subtly-wrong abort/commit boundary silently breaks exactly-once —
the exact failure the owner most wants to avoid. And it forces a **provider-blind** decision (how control markers
live in a sealed store) that is a product call, not a mechanical one. Shipping it half-right unattended would be a
claim larger than its proof. So: design now, ratify + build with the owner in the loop.

## Protocol surface (what a transactional producer drives)
- **`InitProducerId` (22) WITH a `transactional_id`** — today we hand out a bare `producer_id` (idempotent). The
  transactional path must: look up/create the txn state for `transactional_id`, **bump the producer epoch** (fencing
  any prior incarnation), abort any in-flight txn from a previous epoch, and return `(producer_id, epoch)`.
- **`AddPartitionsToTxn` (24)** — register the `(topic, partition)`s a txn will write to (so the coordinator knows
  where to write commit/abort markers at `EndTxn`).
- **`AddOffsetsToTxn` (25)** + **`TxnOffsetCommit` (28)** — fold consumer-group offset commits INTO the txn
  (consume-process-produce atomicity) — they become visible only on commit.
- **`EndTxn` (26)** — commit or abort: write a **control batch** (COMMIT/ABORT marker) to every registered
  partition; on commit, the txn's records + offsets become visible.
- **Fetch `read_committed`** — a `read_committed` consumer must receive only committed records: the Fetch response
  carries the **Last Stable Offset (LSO)** and the **aborted-transactions list**; the consumer (or we) filter out
  aborted records. (`read_uncommitted` consumers — the current behavior — see everything.)

## Coordinator state (extends the group `coordinator.rs` pattern)
A `TxnCoordinator` keyed by `transactional_id`: `{ producer_id, epoch, state: Empty|Ongoing|PrepareCommit|
PrepareAbort|CompleteCommit|CompleteAbort, partitions: Set<(topic,partition)>, pending_offsets }`, behind a Mutex.
Epoch fencing: a Produce/EndTxn at a stale epoch is rejected (`INVALID_PRODUCER_EPOCH`). Like the group coordinator,
txn state is runtime (a crash aborts in-flight txns) UNLESS we persist it — see open question Q3.

## OPEN QUESTIONS for the owner (ratify before build)
- **Q1 — provider-blind control markers.** Kafka writes COMMIT/ABORT control batches into the partition log. In
  datarail the log holds **sealed cofres**. Options: (a) store markers as a distinct un-sealed control record type
  (markers carry no payload, only txn metadata — arguably fine to leave un-sealed); (b) seal the markers too
  (uniform, but they have no secret payload). Recommendation: **(a)** — markers are metadata, not cargo; keep them
  out of the sealed path. Owner ruling needed (it touches the provider-blind invariant's surface).
- **Q2 — `read_committed` filtering locus.** Filter aborted records (a) at our Fetch edge (we hold the txn state →
  return only committed records + a correct LSO), or (b) the Kafka-faithful way (return everything + LSO + aborted
  list, let the client filter). (a) is simpler + keeps us authoritative; (b) is wire-faithful. Recommendation:
  **(b)** for drop-in fidelity, **(a)** acceptable as a first cut. Owner ruling.
- **Q3 — durability of txn state.** Persist `TxnCoordinator` state (like `FileOffsets`) so a broker restart
  resumes in-flight txns, or accept "a broker restart aborts in-flight txns" (simpler, and a producer retries)?
  Recommendation: **accept abort-on-restart** for v1 of this tier (documented), persist later.
- **Q4 — scope.** Is the transactional tier even in scope for v1, or post-v1? The idempotent tier already gives
  per-partition exactly-once (the common case). This is the niche atomic-multi-partition / consume-transform-
  produce case. Owner call on priority.

## Honest scope / non-goals (proposed)
- Target the non-flexible versions of each API where possible; single-node coordinator.
- v1 of this tier: abort-on-broker-restart (Q3); `read_committed` filtering per the Q2 ruling.
- NOT: cross-broker txn coordination, exactly-once across a multi-node cluster.

## Steps 5–6 design decision (the delicate isolation core) — DECIDED 2026-06-28
Two models were weighed for `read_committed` isolation; the decision has real EOS-correctness scope:
- **Buffer-until-commit (rejected as the primary model).** Hold a txn's records in memory; flush to the durable log
  only on `EndTxn(commit)`, discard on abort. Then the durable log holds ONLY committed records → no filtering, no
  markers, no restart-recovery problem. **But** it breaks with **concurrent transactions on the same partition**
  (two producers each compute a provisional base offset from the log end → overlap) and it makes `read_uncommitted`
  stricter than Kafka. Acceptable only for the strictly-one-txn-per-partition-at-a-time case.
- **Full marker / LSO model (CHOSEN).** Transactional records are stored durably and interleaved as produced
  (real offsets immediately); a durable **un-sealed control marker** (Q1=a) records each `EndTxn` (COMMIT/ABORT) at
  its offset. The store tracks, per partition, the txn ranges `(producer_id, [start,end), state)`; the **LSO** = the
  first offset of any still-ongoing txn (everything below it is resolved). `read_committed` Fetch **edge-filters**
  (Q2=a): return records below the LSO that are not in an aborted range, with an empty aborted-list. On restart
  (Q3): replay the log + markers to rebuild the ranges; any txn with no terminal marker is treated as **aborted**
  (its records skipped). This is the Kafka-correct model and the only one sound under concurrent txns + restart.
- **Implementation shape:** the produce path passes `(producer_id, transactional)` (from the batch attributes bit
  4 / the `EosCoord`) to the store so it records the range as ongoing; `KafkaBroker` gains `produce_txn` +
  `end_txn_marker(producer_id, committed, partitions)`; `fetch` gains an isolation flag; the durable marker is a
  distinct control record in `SealedPartitionLog`. This is the next focused increment (carefully designed +
  unit-tested + the brutal audit before ANY exactly-once-abort claim).

## Build plan (once ratified — each step tested + audited, like every prior increment)
1. **This doc + owner ratification of Q1–Q4.**
2. `txn.rs`: the `TxnCoordinator` state machine + epoch fencing (unit-tested, no wire).
3. Codec: `InitProducerId` (transactional_id), `AddPartitionsToTxn`, `AddOffsetsToTxn`, `TxnOffsetCommit`,
   `EndTxn` + control-batch (marker) build/parse; ApiVersions advertises them.
4. `serve_broker` wiring; the marker write per Q1; the txn-aware produce (epoch fence) + EndTxn.
5. `read_committed` Fetch (LSO + aborted list / edge-filter per Q2).
6. **Wire test:** a transactional producer writes to 2 partitions + commits → a `read_committed` consumer sees
   both atomically; an aborted txn → the consumer sees neither.
7. **Brutal adversarial audit** (epoch-fencing races, abort/commit boundary correctness, the provider-blind
   marker decision, `read_committed` leak of aborted records).
