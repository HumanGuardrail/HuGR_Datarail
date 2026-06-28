# KAFKA-GROUPS-DESIGN — durable consumer offsets (increment 3)

> **STATUS: BUILT + PROVEN + AUDITED (2026-06-27).** `datarail kafka-broker` serves `FindCoordinator` (self),
> `OffsetCommit`, `OffsetFetch` (v0–2) backed by the crash-safe `datarail-offsets::FileOffsets` under
> `--data-dir/consumer-offsets`, fsync-before-ack. PROVEN: `groups.rs` unit tests (round-trips, malformed
> over-alloc) + `kafka_groups_wire.rs` (full binary: FindCoordinator → self; OffsetFetch → -1; OffsetCommit 7 →
> NONE; OffsetFetch → 7; cross-group isolation; **KILL broker → RESTART same `--data-dir` → OffsetFetch → 7**).
> **AUDITED (`CORE-AUDIT.md` §Kafka-OFFSETS):** durability, key-injectivity, wire-correctness CLEAN; a memory-
> amplification HIGH (a lying array count materializing ~struct-size× the frame) **fixed at root** AND systemically
> across the pre-existing `consume`/`produce` parsers (`Reader::bounded_count` divides remaining bytes by the min
> per-entry size; untrusted `with_capacity` dropped); a per-partition `OffsetCommit` error-code LOW fixed. Next
> (increment 4): automatic group rebalance + multi-partition.
>
> **(original DESIGN below)** The next consume-side increment: broker-side **durable committed offsets** so a
> consumer's progress survives its own restart — Kafka's `__consumer_offsets`, built the leaner way on the
> already-proven crash-safe `datarail-offsets::FileOffsets`. Adds three wire APIs: `FindCoordinator` (10),
> `OffsetCommit` (8), `OffsetFetch` (9). **Honest scope:** this is the durable-offset layer; it works with a
> consumer that uses **explicit commit + manual assignment** (or any client that drives OffsetCommit/OffsetFetch
> against a `group.id`). **Full automatic group rebalance** (`JoinGroup`/`SyncGroup`/`Heartbeat`/`LeaveGroup`) and
> **multi-partition** are the NEXT increment (increment 4) — stated, not faked.

## Why this, why now
`datarail kafka-broker` is a durable, provider-blind, bidirectional drop-in (Produce + Fetch + ListOffsets,
`KAFKA-FETCH-DESIGN.md`). Today a consumer must track its own offset (e.g. `auto.offset.reset` / explicit seek):
the broker does not remember how far a `group.id` has consumed. This increment makes the broker the durable owner
of committed offsets per `(group, topic, partition)`, so a consumer can crash and resume exactly where it left
off — the defining Kafka consumer-group feature, minus (for now) automatic partition assignment.

## The guarantee shape
- **`OffsetCommit`** durably records, per `(group, topic, partition)`, the consumer's committed offset. The commit
  is acked only **after** it is `fsync`-durable (reusing `FileOffsets`' crash-safe append + atomic-rename
  compaction). A crash right after the ack never loses a committed offset.
- **`OffsetFetch`** returns the last durably-committed offset for `(group, topic, partition)`, or `-1` (Kafka's
  "no committed offset") if none — so a restarting consumer resumes from its commit, not from the start/end.
- **`FindCoordinator`** points the consumer at **this broker** as the group coordinator (single-node: node 0 =
  the advertised host/port). No separate coordinator process — datarail is the coordinator.

These compose: produce → consumer fetches + commits → consumer restarts → `OffsetFetch` returns the commit →
consumer resumes. No record re-processed below the commit, none skipped above it (at-least-once at the consumer
boundary, exactly the Kafka contract for `enable.auto.commit=false` + commit-after-process).

## Wire surface (non-flexible encodings, like Fetch v0–4 — no KIP-482 tagged fields)
Advertise via `ApiVersions` so a client negotiates DOWN to what we implement cleanly:
- **`FindCoordinator` (10) v0–v2.** Req v0: `key`(string = group id). v1+: `+ key_type`(int8). Resp v0:
  `error_code`(i16) `node_id`(i32) `host`(string) `port`(i32). v1+: prepend `throttle_time_ms`(i32), insert
  `error_message`(nullable_string) after `error_code`. We always answer **self** (node 0, advertised host:port,
  `error_code` 0).
- **`OffsetCommit` (8) v0–v2.** Req: `group_id`(string) [v1+ `generation_id`(i32) `member_id`(string)] [v2+
  `retention_time_ms`(i64)] then `topics[]`: `name`(string), `partitions[]`: `partition`(i32) `offset`(i64) [v1
  only `timestamp`(i64)] `metadata`(nullable_string). Resp: `topics[]`: `name`, `partitions[]`: `partition`(i32)
  `error_code`(i16). (v3+ prepends `throttle_time_ms`; we cap at v2.)
- **`OffsetFetch` (9) v0–v2.** Req: `group_id`(string), `topics[]`: `name`(string), `partitions[]`:
  `partition`(i32). Resp v0/v1: `topics[]`: `name`, `partitions[]`: `partition`(i32) `offset`(i64)
  `metadata`(nullable_string) `error_code`(i16). v2+: append a top-level `error_code`(i16). (v3+ prepends
  `throttle_time_ms`; we cap at v2.)

## The seam — extend the `KafkaBroker` trait (default no-op so existing impls/stubs keep compiling)
```rust
pub trait KafkaBroker: Send + Sync {
    // … existing produce / fetch / bounds …
    /// Durably commit a consumer group's offset for a (topic, partition). Default: no-op (offsets unsupported).
    fn commit_offset(&self, _group: &str, _topic: &str, _partition: i32, _offset: i64) -> io::Result<()> { Ok(()) }
    /// The last durably-committed offset for (group, topic, partition), or None. Default: None.
    fn fetch_offset(&self, _group: &str, _topic: &str, _partition: i32) -> io::Result<Option<i64>> { Ok(None) }
}
```
`FindCoordinator` is built in `handle_broker_connection` (it already holds `host`/`port`) — pure protocol, no
store involvement. `OffsetCommit`/`OffsetFetch` route through the two trait methods.

## CLI backing — `FileOffsets` under `--data-dir`
`KafkaBrokerStore` gains a `datarail_offsets::FileOffsets` (crash-safe: append + CRC + atomic-rename compaction,
directory-fsync durable — already audited, `CORE-AUDIT.md` D-6/F1) rooted at `<data-dir>/consumer-offsets`, behind
the existing `Mutex`. The composite key is **length-prefixed** so it is injective across arbitrary group/topic
bytes: `offset_key(group, topic, partition)`. Kafka offsets are `>= 0`; stored as `u64`, surfaced back as `i64`.

## Honest scope / non-goals (stated, not faked)
- **No automatic group rebalance** (`JoinGroup`/`SyncGroup`/`Heartbeat`/`LeaveGroup`): a `subscribe()`-based
  consumer that needs the broker to ASSIGN partitions is increment 4. This increment serves explicit-commit /
  manual-assignment consumers, which is a real, common pattern. We say so.
- **Single partition** per topic still (multi-partition is increment 4; the offset store is already keyed by
  partition, so it is forward-ready).
- The Produce hop stays plaintext (seal-on-ingest); TLS on the Kafka hop is its own arc.

## Build plan (each step tested; new path audited like the rest)
1. **This doc.**
2. `groups.rs` in `datarail-kafka`: parse/build for FindCoordinator v0–2, OffsetCommit v0–2, OffsetFetch v0–2
   (version-gated, non-flexible), with unit tests (round-trip + bounded counts on garbage).
3. Extend the `KafkaBroker` trait (default no-op) + wire the three APIs into `handle_broker_connection`; advertise
   them in `ApiVersions` `SUPPORTED`.
4. CLI `KafkaBrokerStore`: `FileOffsets` backing + `offset_key` + the two trait impls.
5. **Faithful wire test:** produce + fetch, `OffsetCommit` offset N, simulate a consumer restart, `OffsetFetch` →
   N (durable across the broker restart too, via the data dir). `FindCoordinator` → self.
6. **Adversarial audit** of the new path (offset durability across crash; key injectivity; malformed
   commit/fetch → no panic/over-alloc; no cross-group leakage).
7. *(tracked, increment 4)* JoinGroup/SyncGroup/Heartbeat/LeaveGroup automatic rebalance + multi-partition.
