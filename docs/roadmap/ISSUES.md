# Roadmap — open work, as issues

Honest backlog. Each item is written so it can be pasted straight into a GitHub issue (title + labels +
context + acceptance). Ordered roughly by leverage. "Done so far" at the bottom records what already shipped,
so the trajectory is legible to anyone reading cold.

---

## High leverage

### #1 — Per-partition locking (kill the last throughput serializer)
**labels:** `perf`, `broker`, `good-second-issue`
The per-batch seal is now parallel (see `docs/BENCH-INDEPENDENT-2026-07-01.md` addendum, ~2.5× measured),
but a single `Mutex<BrokerInner>` still serializes *whole batches* across all partitions and connections. Two
producers on two partitions aggregate to ~2.8 MB/s — the catastrophic negative scaling is gone, but there is
no true multi-partition parallelism yet.
**Do:** split `BrokerInner` so each `(topic, partition)` log has its own lock (or shard the map behind
per-partition `Mutex`/`RwLock`); keep the seq-reservation invariant per partition; the offsets store and txn
buffers need their own synchronization.
**Acceptance:** two producers on two partitions exceed a single producer's throughput (positive scaling), with
an interleaved-A/B measurement committed to the bench doc; all existing wire/durability tests still green.

### #2 — Power-loss durability test (beyond `kill -9`)
**labels:** `durability`, `test`, `hard`
`tests/kill9_crash.rs` proves records survive a SIGKILL, but the OS page cache survives a killed process, so
fsync-ordering bugs that only a real power cut exposes are still untested. The dir-fsync fix
(`055b568`) is currently argued, not demonstrated under fault injection.
**Do:** a harness that runs the broker against a filesystem whose fsync/rename can be reordered/dropped (e.g.
a FUSE shim, `dm-flakey`, or CharybdeFS-style injector), then asserts no acked record is lost after an
injected crash-consistent cut.
**Acceptance:** a reproducible CI job (or documented local harness) that would FAIL without the dir-fsync and
PASS with it.

### #3 — Kafka protocol conformance against real client libraries
**labels:** `compat`, `test`
CI exercises kcat/librdkafka only (see `docs/design/KAFKA-COMPAT.md` → tested-client matrix). Untested: the
Apache Kafka **Java** client, **franz-go**, **kafka-python**, **Sarama**, **librdkafka 2.x**.
**Do:** add a CI matrix that produces+consumes (and, where supported, consumer-group + transactions) against
each; record pass/fail per API in the matrix.
**Acceptance:** the KAFKA-COMPAT matrix rows move from "not yet tested" to a real status, green or honestly
red.

---

## Correctness / protocol edges

### #4 — Transactional `EndTxn` is not crash-atomic across partitions
**labels:** `correctness`, `kafka-txn`, `hard`
The txn coordinator state is in-memory; a crash mid-`commit_txn` can leave a partial multi-partition commit
(documented in `KAFKA-TXN-DESIGN.md` and README Known limitations). Contract violations are now rejected at
buffer time (`529b040`), so the loop hazard is closed, but atomicity across a crash is not.
**Do:** a durable transaction log (write intent → flush partitions → commit marker), replayed on restart.
**Acceptance:** a kill-during-commit test shows all-or-nothing visibility of a multi-partition txn.

### #5 — `BatchTooLarge` should map to `MESSAGE_TOO_LARGE` (10), not retriable 56
**labels:** `kafka`, `good-first-issue`
An oversize batch (>u32 framing) currently returns the retriable `KAFKA_STORAGE_ERROR` (56); it is permanent
for that batch, so a client retries forever — the same class of bug as the contract-violation infinite loop
already fixed in `529b040`. See the comment in `KafkaBrokerStore::produce_into`.
**Do:** map the framing-too-large error to non-retriable `MESSAGE_TOO_LARGE` (10); regression test.
**Acceptance:** an oversize produce returns 10 and is not retried; a normal produce is unaffected.

### #6 — Broker restarts are at-least-once for non-idempotent producers
**labels:** `correctness`, `broker`
The cross-restart dedup index exists as library code but is not wired into the broker; a non-idempotent
producer that retries across a restart can double-land (README Known limitations).
**Do:** persist and consult the dedup watermark in the broker path, as the Postgres sink already does.
**Acceptance:** a produce→restart→retry sequence lands each record once.

---

## Security / ops

### #7 — Real key management (stop inlining secrets in `rail.toml`)
**labels:** `security`, `ops`
`examples/rail.toml` carries raw demo seeds; production has no key-ref/KMS path (see `SECURITY.md`).
**Do:** support key references (env / file / a KMS handle) in the spec; never require inline secrets.
**Acceptance:** a broker/rail boots from key-refs with no raw key material in the config file.

### #8 — QUIC substrate: real cert verification (retire the dev-only embedded cert)
**labels:** `security`, `substrate`
The QUIC substrate ships an embedded dev cert and the client accepts any server cert — an active MITM sees
envelope metadata (never payloads). Documented as dev-only in the crate.
**Do:** pluggable cert verification (CA chain + hostname), embedded cert gated behind an explicit dev flag.
**Acceptance:** a MITM with a wrong cert is rejected; the dev path requires opting in.

### #9 — External crypto review of the composition
**labels:** `security`, `help-wanted`
The primitives are vetted crates, but the *construction* (per-cofre X25519 wrap → AEAD, Ed25519 lacre, the
DRBG's reseed-on-fork/CRIU story) has had no third-party review — only self-audit (`ADVERSARIAL-AUDIT.md`).
**Do:** solicit a review; publish findings and fixes.

---

## Longer-term / product

### #10 — Extract the Merkle delivery receipt as a standalone crate
**labels:** `product`, `crate`
`datarail-manifest` (offline-verifiable delivery proof) is the piece most likely to matter to others
independent of the broker (see `docs/blog/03-merkle-delivery-receipt.md`).
**Do:** carve it into a dependency-light crate with its own docs + verifier CLI; publish.

### #11 — Wire the designed-ahead replication tier into the binary, or cut it
**labels:** `architecture`, `decision`
`datarail-broker`, `datarail-replicated-topic`, `datarail-replication`, `datarail-erasure` are tested library
code but unreachable from the CLI (README crate table calls this out). Single-node is currently a hard truth.
**Do:** decide — either wire a real multi-node path (leader/replication/failover) or explicitly descope it and
say "edge/embedded, single-node by design." Do not leave it in limbo.

---

## Done so far (trajectory, newest first)

- Parallel per-batch seal (`88fe891`); dir-fsync on rotation + loud-fail on fetch corruption (`055b568`);
  `acks=0` respected + produce CRC-32C validated (`a924e0a`); `kill -9` crash harness (`fa29cb8`);
  SCRAM-SHA-256 in the Postgres driver (`f8691cc`).
- Contract violation → non-retriable `INVALID_RECORD` on plain + txn paths (`529b040`, found by the
  independent benchmark's librdkafka run).
- Honest README + independent broker benchmark (`c8c85ed`); repo-wide claim reconciliation (`073966b`);
  dual license + crate metadata (`c8ec420`); `SECURITY.md` + 60-second demo (`a61b3f6`); architecture diagram
  + client matrix (`ba98df5`).
