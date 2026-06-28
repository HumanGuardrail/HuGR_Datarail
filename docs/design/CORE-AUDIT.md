# CORE-AUDIT — brutal adversarial audit of the two pillars (seal-core + exactly-once/durability)

> Two hostile background auditors attacked the code the whole product rests on: the **sealing core** (crypto +
> cofre + terminal — the provider-blind guarantee) and the **exactly-once / no-loss core** (once + broker + topic
> + offsets). Both were disciplined and cold-verified with PoCs. This is the honest record: every finding, its
> verdict, and its disposition (**FIXED** now / **TRACKED** gap / **ACCEPTED** trade-off). It also CORRECTS two
> over-claims the audit exposed.

## Headline corrections (honesty — these were over-stated before this audit)
1. **"effectively-once" is NOT durable across crashes.** The `datarail-once` dedup index is **in-memory only**
   and is **not wired into the broker** at all (the broker is plain at-least-once). Effectively-once holds
   *within a single process run* (the `ac4_dst.rs` 1000-seed test, no crash) — NOT across a restart, where an
   at-least-once transport would re-deliver. Corrected in `LASTRO-MATRIX`. Closing this (persist the dedup index +
   wire it in) is **TRACKED** (finding D-3).
2. **"no-loss across crashes" was tested only against process-kill, not power-loss.** The chaos test used
   drop+reopen, which preserves the OS page cache — it never exercised fsync durability. The broker acked
   produces *without fsync*. **Now FIXED** (durability-before-ack, D-1); with the fsync, acked records survive a
   true power loss, and the claim is honest.

---

## Seal-core audit (crypto / cofre / terminal) — VERDICT: provider-blind HOLDS vs the wire adversary
The auditor **confirmed sound**: full signature coverage (every wire byte signed; mutate-every-byte rejected),
verify-before-decrypt + AAD binding, cross-route/replay blocked, per-cofre key freshness in normal operation,
panic-safety on hostile cofre bytes, constant-time (no secret-dependent compare), Ed25519 malleability harmless.

| # | Finding | Sev | Disposition |
|---|---|---|---|
| S-1 | **DRBG reseed keyed on PID → VM-snapshot/clone replays identical `(eph_secret, nonce)`** (catastrophic under opt-in `Gcm256`, plaintext-equality leak under GCM-SIV default). PROVEN. | HIGH | **FIXED** `675d70d`: every DRBG draw now mixes fresh OS entropy → clone-immune by construction, not by PID detection. Regression test: identical-state clones diverge every draw. |
| S-2 | **Sealed-sender `epoch` never enforced** — a revoked sender's old cert is accepted forever (revocation is theater). PROVEN. | MED | **FIXED**: `DestTerminal::with_min_sender_epoch(n)` — a cert with `epoch < n` is dead-lettered (default 0 = backward-compatible; raise after a rotation). Regression test added. |
| S-3 | No X25519 contributory/low-order-point check in key-wrap. | LOW | **FIXED** (2026-06-26, defense-in-depth): `x25519_shared` now rejects a non-contributory shared secret (`was_contributory()`, RFC 7748 §6.1); `seal_key`/`open_key` return `Option` and fail closed. Source-side a low-order `dest_x25519_pk` → `TerminalError::Seal`; dest-side a low-order `eph_pk` → `DeadLetterReason::KeyWrapInvalid` (rejected before AEAD-open). Still not wire-reachable (lacre verified first), but no longer trusts that alone. Regression test `low_order_eph_public_is_rejected_s3` (identity + order-8 points). |
| S-4 | `idempotency_key = HMAC(tenant_secret, record_key)` rides cleartext → the rail can see which cofres share a record_key within a tenant (linkability). | LOW | **ACCEPTED** trade-off: inherent to a deterministic dedup key; `record_key` itself is NOT recoverable. Documented. |
| S-5 | `sender_present` flag + a 200 B carga delta leak *whether* (not who) a cofre carries a sealed sender. | LOW | **ACCEPTED** trade-off: identity is correctly hidden (proven); presence-padding is optional future work. |

## Once / durability audit (once / broker / topic / offsets) — VERDICT (before fixes): no-loss + effectively-once did NOT hold across crashes
The auditor **confirmed sound**: resume off-by-one is correct *when the topic survives*, GC vs reject-below
interaction, offsets recovery parse-safety (CRC + torn-tail truncation, no panic), single-`Mutex` concurrency.

| # | Finding | Sev | Disposition |
|---|---|---|---|
| D-1 | **Broker acked Produce before any fsync** → power-loss loses acked records. PROVEN. | CRIT | **FIXED** `741b803`: durability-before-ack — broker `topic.sync()` (fsync) before replying `Produced`. |
| D-2 | **Fsync'd committed offset outlived un-synced topic** → resume wedges + loss. PROVEN. | CRIT | **FIXED** `741b803`: same fsync makes the topic at least as durable as the offset pointing into it. |
| D-3 | **Dedup index in-memory only** → restart = duplicate flood; "effectively-once across crashes" false. PROVEN. | CRIT | **FIXED for the transactional sink (Tier A): exactly-once into Postgres**, the dedup watermark stored ATOMICALLY in the DB (`TxnSink::commit_at`, `EXACTLY-ONCE-DESIGN.md`) — a replayed batch is a committed no-op; PROVEN live (4 rows despite 3 replays). File/webhook sinks now get a **persistent dedup index** (`FileOnce`: fsync'd log + replay + compaction, crash-tested) so a restart no longer floods duplicates — honestly at-least-once with a bounded (≤1) dup window (Tier C). |
| D-4 | **Same `seq` above the watermark with a different key → delivered twice.** PROVEN. | HIGH | **FIXED** `741b803`: seq-level dedup (`delivered_keys`, never GC'd above watermark) — a seq is committed once regardless of key. Regression test added. |
| D-5 | A permanent gap pins the watermark → `seen`/`ahead`/`delivered_keys` grow without bound (RAM DoS). | MED | **FIXED**: `MAX_REORDER_HORIZON` (1<<20) — past it the lowest gap is sealed-skipped (watermark advances, the missing seq is thereafter reject-below'd). Availability-over-completeness; never triggers in normal reorder. |
| D-6 | `FileOffsets` doesn't fsync the parent dir on create / re-fsync after compaction (rename durability). | MED | **FIXED**: `fsync_dir` helper — fsync the dir on `open` (first-create durable) and (propagated, not swallowed) after the compaction rename. |
| D-7 | `Commit` accepted an arbitrary offset (past tail → wedge). | LOW | **FIXED** `741b803`: reject a commit past the durable topic tail. |

## Txn-path audit (the new exactly-once `commit_at`) — 2 PROVEN CRITICALs, all fixed
A third brutal auditor hit the new transactional path (it claimed "exactly-once PROVEN" — durability components
always get audited). It CONFIRMED safe: no SQL injection (`hex` is hex-only; identifiers validated; bytea literal
correct), single-connection crash atomicity (COPY participates in the txn rollback), panic-safety. It PROVED two
real breaks (both fixed + regression-tested):

| # | Finding | Sev | Disposition |
|---|---|---|---|
| T-1 | **Backend `ErrorResponse` desynced the wire** — `query_simple`/`copy_in` early-returned on `'E'` without draining the mandatory trailing `ReadyForQuery`; every later query then misread → `resume_watermark` returned 0 → the WHOLE stream re-landed. PROVEN (durable wm 1, read back 0). | CRIT | **FIXED**: every query/COPY now DRAINS to `'Z'`, capturing the error, before returning. Live regression `a_backend_error_does_not_desync_the_connection`. |
| T-2 | **`SELECT … FOR UPDATE` locks no missing row** → two concurrent first-batches both land (double-land). PROVEN (2 rows for 1 batch). | CRIT | **FIXED**: a transaction-scoped `pg_advisory_xact_lock` keyed by the stream serializes concurrent `commit_at` even with no watermark row yet. |
| T-3 | Watermark `0` ambiguous (no-row vs seq 0) → first batch silently lost / re-landed. | HIGH | **FIXED**: `read_watermark` returns `Option<i64>` (None = no row, distinct from 0); a present-but-unparsable value is a hard error. |
| T-4 | A short/malformed `DataRow` silently returned 0. | MED | **FIXED**: a present-but-unparsable `DataRow` is now an error, never a silent 0. |
| T-5 | `watermark > i64::MAX` clamped silently. | LOW | **FIXED**: errors instead of clamping. |

The "exactly-once PROVEN" claim was real for the happy path but FALSE under error/concurrency — exactly what the
audit is for. It is now actually true (live C1 regression + the concurrency lock), and re-gated in CI.

## Substrate/transport audit (shmem unsafe + rail/netblob) — 1 CRITICAL fixed; unsafe waiver SOUND
A fourth auditor hit the substrate layer (the ONE `unsafe` waiver is here). It CONFIRMED the unsafe SOUND
(memory-unsafety cannot escape — all ring DATA access is through checked safe slices; the atomic cells are
fixed-offset + aligned), and the socket/QUIC/UDP/object-store framings hardened (length caps, timeouts, off-peer
drop, no path traversal, INV-OPAQUE-CARGO holds).

| # | Finding | Sev | Disposition |
|---|---|---|---|
| X-1 | **shmem `recv` trusted peer-controllable cursors + frame length** — `write - read` underflowed and `len` was not capped against the ring capacity (the `send` side caps it) → a hostile same-host peer drives `read_ring` past the mapping → OOB-slice PANIC (consumer DoS). PROVEN (OOB panic). | CRIT | **FIXED**: `recv` now `checked_sub`s the cursors (rejects `read > write`) and rejects `total > capacity` before any copy — the cursors are treated as untrusted, like a socket length prefix. 2 regression tests (hostile oversized frame + corrupt cursors → Err, not panic). |
| X-2 | netblob accept loop spawned an unbounded thread per connection (flood DoS; slowloris already covered by the 30s timeout). | MED | **FIXED**: `MAX_CONNECTIONS` cap (excess dropped), matching the kafka/serve fix. |
| X-3 | QUIC client accepts any server cert (transport MITM possible; confidentiality rests on the cofre seal). | LOW | **ACCEPTED** by-design (the DERP blind-relay model; `INV-OPAQUE-CARGO` — no plaintext crosses the transport). |

## Honest status after this pass
- **Sealing / provider-blind:** HOLDS against the wire adversary (pipe/storage/MITM) in the GCM-SIV default; the
  one real weakness (snapshot/clone key reuse) is FIXED. `Gcm256` (opt-in `vaes`) is now also clone-safe.
- **No-loss:** the broker is now **durability-before-ack** — acked records survive power loss. FIXED.
- **Effectively-once:** holds **within a process run**; **NOT yet across crashes** (dedup not persisted/wired) —
  honestly TRACKED, not claimed. The broker is at-least-once across restarts.
- **All audit findings now FIXED.** D-3 (dedup persistence / effectively-once cross-crash — the big one) was
  solved at root: Tier A exactly-once into Postgres (`commit_at`, watermark in the sink's txn) + Tier C `FileOnce`
  persistent dedup, and Tier A is now **wired into the product end-to-end** (`datarail run`/`kafka-ingest`, proven
  live). S-3 (X25519 low-order defense-in-depth) is now FIXED too (was the last TRACKED item). (S-2 epoch, D-5 gap
  horizon, D-6 offsets dir-fsync FIXED earlier.) No finding was ever a wire-adversary confidentiality/integrity
  break; the remaining S-4/S-5 entries are documented ACCEPTED trade-offs (deterministic-dedup linkability /
  sealed-sender presence), not defects.

This pass is the methodology working: the auditors proved the scary lenses *safe* (signatures, verify-before-
decrypt, panic-safety) and found the real defects (a crypto clone-reuse + the durability/dedup gaps), which were
fixed at root or honestly tracked — and two over-claims were corrected.

## Tier-A wiring audit (2026-06-27) — brutal adversarial pass on `commit_at` + the `AnySink` product wiring

Run right after Tier A was wired end-to-end (commit 5c79d5e), an isolated-worktree auditor was told to assume the
exactly-once scheme is broken and prove it. Findings (each cold-verified by the lead before acting):

| # | finding | sev | PROVEN? | disposition |
|---|---|---|---|---|
| F1 | The watermark is a cumulative **position** (landed-count) and `commit_at` lands `records[stored-base..]` **positionally**, never by record identity. Sound only for an append-ordered source; shipped on-by-default for HTTP (a GET can reorder / mid-stream-insert) and Kafka-ingest. PoC: run1 `[A,B]`, run2 `[A,C,B]` → DB `[A,B,B]` (C lost, B doubled). | **CRIT** | **YES** | **FIXED**: Tier A gated on BOTH opt-in AND an append-ordered source (`wants_tier_a`); HTTP & Kafka-ingest → at-least-once (Tier C, no loss). Over-claims corrected across README/LASTRO/EXACTLY-ONCE-DESIGN/BUILD_LOG. |
| F2 | Kafka-ingest funnels all `(topic,partition)`s into one `route_id` watermark/lock; cross-restart interleave is not a stable position (root of F1-S2). | **HIGH** | **YES** | **FIXED (scoped)**: kafka-ingest forced to at-least-once. Genuine per-(topic,partition)-offset exactly-once tracked future work (needs the broker offset threaded through + dead-letter-gap handling). |
| F3 | In-place edits/shrink within the already-landed prefix are silently masked (corollary of F1; out of the append-only contract). | LOW | SUSPECTED | **ACCEPTED** (documented append-only file contract — a log only grows; reordering a source file mid-stream is an operator contract violation, not a default path). |

**Confirmed SOUND (auditor looked, found nothing):** `commit_at` error/rollback paths (no txn left open, no
connection desync — every path drains to `'Z'`); the `pg_advisory_xact_lock` first-batch-race serialization (no
TOCTOU); the integer math (`i64::try_from`, `base = watermark - len` cannot go negative, `already` clamped — no
panic/overflow); the single-run multi-batch cursor (Duplicate/DeadLettered batches correctly skip the commit);
deterministic per-process `board` seq (so `fresh` IS deterministic for a fixed-order source — the F1 non-determinism
comes from source *ordering*, not dead-letter flapping); the `gap` (stored < base) case unreachable + clamped.

Lesson reinforced: the hardening (locks, drains, arithmetic) was solid; the *guarantee's scope* was the defect.
Attack the headline, not just the code.

## Kafka-EOS audit (2026-06-27) — brutal adversarial pass on the exactly-once ingest path

A fifth brutal auditor attacked the Kafka EOS path (`InitProducerId`, `EosCoord`, `commit_at_seq`, the per-batch
CLI routing). It audited a stale commit (worktrees branch from committed state, so it saw `cbe829b` = increment 1,
before `InitProducerId` was added — its finding [B] "InitProducerId missing" was thus STALE). The lead
cold-verified each finding against the real tree.

| # | finding | sev | PROVEN? | disposition |
|---|---|---|---|---|
| A | **ack-before-durable**: the broker acked the producer (`error_code=NONE`) the instant it enqueued the batch on the in-memory channel, BEFORE the sink landed it. A crash between ack and land loses acked records; an idempotent producer never resends an acked batch → permanent loss. (The D-1 lesson regressed in the ingest path.) | **CRIT** | **YES** | **FIXED**: ack-after-durable — `serve` waits for the integration layer's landing result before acking (`ProducedBatch.done`); a sink failure → retriable error code (KAFKA_STORAGE_ERROR), never a false NONE. |
| E | a single sink error tore down the whole ingest loop (`?`) while `serve` kept acking → amplifies A. | **HIGH** | **YES** | **FIXED**: the ingest loop reports the error back (retriable code) and KEEPS SERVING; proven by `kafka_eos_live.rs` (drop table → retriable error + daemon survives + recovers). |
| (follow-on) | the [E] continue-on-error would let a failed batch's uncommitted records mis-attribute to a later batch via the GLOBAL `committed_to_sink` cursor (the auditor confirmed the cursor was safe ONLY because errors previously killed the loop). | HIGH | derived | **FIXED**: replaced the global cursor with a **per-batch snapshot** (`committed()[before..after]`) so each batch's commit is self-contained; + a **unique in-process record_key per batch** so the sink watermark is the sole EOS authority (retries re-deliver, prior failures re-commit). |
| D | `eos_stream_id` dropped `producer_epoch` → an epoch bump (sequence resets to 0) could be no-op'd against the old epoch's higher watermark (loss). | LOW | SUSPECTED | **FIXED**: `producer_epoch` folded into the substream id; regression in `eos_stream_id_separates_distinct_substreams`. |
| C | `commit_at_seq` whole-batch precondition (stored is a clean boundary) is enforced by nothing — a malformed producer resending a bigger batch with the same `base_sequence` could double-land the prefix; i32 sequence wrap at 2^31. | LOW | SUSPECTED | **ACCEPTED**: the idempotent-producer contract guarantees identical resends + in-order whole-batch delivery (like a forged `producer_id`, a non-conformant producer is outside the trust model); 2^31 records/session is not a practical concern. Documented. |
| B | "InitProducerId not implemented" | — | STALE | Implemented (the auditor saw a pre-`InitProducerId` commit); proven by `eos_wire.rs` + `kafka_eos_live.rs`. |

**Confirmed SOUND (auditor + lead, found nothing):** `count` vs null-value records (range width is `record_count`,
correct); multi-batch/legacy → no `eos`; `high = base_sequence + count` no overflow (i64 math); the once-gate
`record_key` uniqueness; `commit_at_seq` idempotent retry + replay-after-restart + dead-letter advance; the
per-batch snapshot under interleaved substreams + partial dead-letter (traced, correct).

**Memory limitation — RESOLVED (2026-06-27, the streaming-terminal arc):** the offload terminal's sink retained
every landed record for the daemon's lifetime (pre-existing). The pipeline now DRAINS the sink every batch
(`take_committed` + a `landed_total` counter), and the dead-letter siding is BOUNDED (`MAX_DEAD_LETTERS_RETAINED`,
evict-oldest + counted) so a contract-violation flood can't OOM the destination. Regression:
`dead_letter_siding_is_bounded_under_a_flood`; live re-proof (file Tier A + Kafka EOS) unchanged.

## Kafka-CONSUME audit (2026-06-27) — brutal adversarial pass on the un-seal-on-fetch path — CLEAN

A sixth brutal auditor attacked the `kafka-broker` consume path (`consume` codec, `crc32c`/`build_record_batch`,
`serve_broker`, the `DestTerminal::open`/`open_records` refactor, the CLI sealed store) at HEAD `b7bfe1d`.
**Verdict: no exploitable bug.** Every key attack DEFEATED (lead cold-verified each):

- **Cross-route / cross-tenant read** — `open_records` rejects a `route_id`/`stream_id` mismatch (`RouteMismatch`):
  a cofre sealed for another route never opens.
- **Forged cofre injected into the store** — `open_records` runs `datarail_cofre::verify` against the pinned
  source vk first (parse-before-verify): a forged lacre fails.
- **Malformed Fetch/ListOffsets → panic/over-alloc** — every count is bounded by `remaining()`; the `Reader` is
  fully bounds-checked; varints length-capped. No panic / over-read / over-alloc (the fuzz suite agrees).
- **Bad CRC** — `crc32c(b"123456789") == 0xe3069283` (canonical Castagnoli) and the CRC covers exactly
  `attributes..records` per spec → a real consumer accepts the batch.
- **Plaintext recoverable from storage** — `encode` = etiqueta ‖ AEAD-`carga` ‖ lacre; plaintext lives only inside
  the sealed `carga`. The store holds ciphertext only — **provider-blind** (the store unit test asserts the
  plaintext is absent from the stored bytes).

**Refactor integrity:** the `offload`→`open_records` extraction preserves every check in the SAME order (verify
→ route → contract_fp → key-wrap → AEAD-open → sealed-sender → un-frame → per-record contract); nothing dropped,
reordered, or weakened; `open`/`open_records` are `&self` and touch no dedup/sink/dead-letter state (re-fetch is
idempotent). The 26 terminal tests (incl every dead-letter reason) guard it.

Findings: 3, all INFO/LOW, none a defect — a v0 `ListOffsets` path reachable only by a client that ignores
`ApiVersions` (format correct); unknown-`api_key` drops the connection (fail-safe, no desync); the Fetch `Err`
arm is effectively dead with the in-memory store (correct defensive code for a future fallible backend). No fix
required. The security-critical un-seal-on-fetch path holds up to violent probing.

## Kafka-CONSUME-DURABILITY audit (2026-06-27) — brutal adversarial pass on the increment-2 durable store

A seventh auditor (isolated worktree, branched from the committed increment-2 store `7cbe37f`) attacked
`kafka_store::SealedPartitionLog` + the `KafkaBrokerStore` durable rewrite. **3 findings (2 HIGH, 1 LOW);
path/int/concurrency/provider-blind lenses CLEAN.** Each finding was lead-cold-verified against source before
acting (one would-be path-traversal/int-overflow/plaintext-leak set was correctly self-rejected by the auditor).

- **HIGH #2 — failed-batch offset shift → FIXED AT ROOT.** A valid frame appended *before* a later record in the
  SAME batch failed (oversize record, or disk-full mid-batch) was durable on disk but the live broker never
  published its offset; on restart `open()` counted it, shifting every subsequently-acked record's logical offset
  (an acked offset would point at the wrong record). PoC: batch `["good", >MAX_RECORD]` then `["real-0"]` → live
  `real-0`@0 but restart `real-0`@1. **Fix:** `append_durable` reconciles the in-memory index to what is actually
  on disk (shared `scan_starts`) on ANY append/`fsync` failure, so the next produce's base AND a post-restart
  `open()` agree — a failed batch never silently shifts later offsets. Regression:
  `a_failed_batch_keeps_logical_offsets_stable_across_restart`.
- **HIGH #1 — silent mid-history disk-rot renumbers offsets → SCOPED HONESTLY + TRACKED.** A CRC mismatch in a
  *non-final* segment makes `datarail-replaylog` RESYNC past the corrupt segment (cold-verified at
  `replaylog/src/lib.rs:346` `resync_or_stop`); the index rebuild then drops that segment's tail and renumbers
  survivors. This is a storage-integrity failure *outside the crash-consistency model* — corruption is detected
  (wrong bytes are NEVER returned), and it does **not** affect the clean-crash durability claim (a torn tail is in
  the final segment → clean stop → contiguous index intact). Documented in the module + design non-goals; hardening
  (per-record durable logical ids / a fail-loud integrity checkpoint instead of silent renumber) is tracked, not
  claimed. *(Not a silently-shipped gap: the proven claim is precisely the clean-crash one.)*
- **LOW — doc overclaim → FIXED.** The module doc said "a crash … never loses an acked record"; softened to "a
  *clean* crash", matching what is proven.

**CLEAN lenses (cold-verified):** `partition_dir` hex-encodes the topic (charset `0-9a-f`, then `-{partition}`) →
no `/`/`..`/NUL reaches a path component, injective even for negative partitions; wire-controlled `fetch` offset
(`usize::try_from(..).unwrap_or(MAX)` → empty) and `max_bytes` (≤0 → exactly one record, the progress guarantee)
never panic/over-read; single `Mutex`, poisoning via `into_inner`, no nested locks → no deadlock; the board key
`kbroker-{topic}-…` is HMAC'd into `idempotency_key` and the carga is ciphertext → no plaintext (nor the topic)
reaches disk. Provider-blind-on-disk additionally proven by the rewritten store test + the restart wire test.

## Kafka-OFFSETS audit (2026-06-27) — brutal adversarial pass on durable consumer offsets (increment 3)

An 8th auditor (isolated worktree, from `1033745`) attacked the new `FindCoordinator`/`OffsetCommit`/`OffsetFetch`
path (`groups.rs`, the trait + serve dispatch, the CLI `FileOffsets` backing + `offset_key`). **2 findings (1 HIGH,
1 LOW); durability / key-injectivity / wire-correctness / concurrency / casts cold-verified CLEAN.**

- **HIGH — memory-amplification DoS via an untrusted array count → FIXED AT ROOT + SYSTEMICALLY.** `bounded()` capped
  an array count by remaining *bytes*, but each `Vec` element is far larger than its min wire size, and the code then
  `Vec::with_capacity(count)`'d — a single 16 MiB frame with a lying `topic_count` materialized ~struct-size× the
  frame (~168 MB measured), and a near-`i32::MAX` `with_capacity` can even ABORT on allocation failure (a remote
  crash). Cold-verify found the **same pattern pre-existed** in `consume.rs` (`parse_fetch`/`parse_list_offsets`) and
  `produce.rs` (`parse_produce`) — a prior audit had only stopped the `i32::MAX` count, not the bytes-amplification.
  **Fix:** a shared `Reader::bounded_count(count, min_entry_bytes)` divides remaining bytes by the smallest possible
  per-entry size (so the count can never exceed what physically fits), and every parser drops the untrusted
  `with_capacity` for a grow-on-demand `Vec::new()`. Applied to `groups`, `consume`, AND `produce`. The shipped
  over-alloc test was strengthened (it previously sent no filler, so it never exercised the large-remaining case).
- **LOW — `OffsetCommit` collapsed all partitions to one error code → FIXED.** A single failed partition stamped
  every partition's response with the retriable code (Kafka reports per-partition). Now each partition carries its
  own `error_code` (`offset_commit_results` + per-partition result types), so one failure never masks another's
  durable success.

**CLEAN lenses (cold-verified):** `offset_key` = `"{glen}:{group}:{tlen}:{topic}:{partition}"` is injective (the
exact decimal length precedes each variable field — uniquely decodable regardless of embedded colons/digits; 2 M
adversarial triples → 0 collisions); `FileOffsets::commit` is `write_all`→`sync_all`(fsync)→update-map and the serve
loop acks NONE only after `commit_offset` returns `Ok` → fsync-before-ack holds; wire field order + version gating
cross-checked vs the Kafka schema for all three APIs at v0/v1/v2 (FindCoordinator v1 throttle+error_message,
OffsetCommit v1 timestamp / v2 retention, OffsetFetch v2 top-level error_code) — correct; mutex poisoning via
`into_inner`; offset casts saturate, never panic.
