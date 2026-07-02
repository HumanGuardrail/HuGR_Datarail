# Independent benchmark — `datarail kafka-broker` (the product, not the shim)

**Date:** 2026-07-01 · **Run by:** independent audit (Claude/Cowork), no author involvement in the setup
**Object measured:** the real `datarail kafka-broker` binary (release build, rustc 1.91.1), driven by a real
Kafka client (kafkacat 1.6.0 / librdkafka 1.8.0) — unlike the repo's own benchmark docs, which measure the
`datarail-omb-shim`.

## Environment

Sandboxed Linux VM (Ubuntu 22.04), **4 vCPU i7-9750H @ 2.60GHz** (AES-NI present, **no VAES/AVX-512** — the
same CPU class as the laptop runs recorded in `BENCH-01.md`), 3.8 GB RAM, NVMe. Shared CPU (measured noise:
~4.7–6.0 MB/s spread across identical batches). Default broker config: fsync-before-ack,
`examples/rail.toml`, records `evt:` + 1019 bytes ≈ 1 KiB.

## Results

| Metric | Measured | Repo claim | Verdict |
|---|---|---|---|
| Cold start (empty dir, n=30) | **median 3.0 ms** (p90 3.7; max 12.2) | ~2 ms | ✅ replicates |
| Binary size | 2.0 MB | — | ✅ tiny |
| RSS idle | 3.3 MB | ~27 MB (shim, under load) | ✅ even smaller |
| RSS peak (50k msgs, produce+consume) | **9.0 MB** | — | ✅ real footprint |
| Produce 1 KiB, 1 partition, 1 producer | **~6.0 MB/s (≈6,000 msg/s)** | 51 MB/s (shim, efficiency) / 658 MB/s (OMB 32-core) | ⚠️ product ≈ **8–10× below** the shim |
| Consume 1 KiB | ~6.1 MB/s | — | same |
| **2 producers, 2 partitions** | **4.4 MB/s aggregate** (less than 1 producer!) | — | ❌ **negative scaling** |
| Broker CPU during the parallel test | 0.97 core | — | ❌ global mutex serializes everything |
| Integrity (50k produced → consumed) | 50,000/50,000, zero loss | — | ✅ |
| Disk: ciphertext only (plaintext grep) | 0 hits; 1.3× overhead | provider-blind | ✅ replicates |
| Restart with 20k records on disk | listener up in 3 ms | — | ⚠️ listener opens **before** the recovery scan; "ready to serve" was never what the docs measured |

## Bug found during setup

> **UPDATE (same day): fixed.** A contract violation now maps to `INVALID_RECORD` (87, non-retriable) via
> `TerminalError::ContractViolation` → `ErrorKind::InvalidData`, with a broker-side log line and regression
> tests (`produce_invalid_data_maps_to_non_retriable_87`, `produce_other_error_stays_retriable_56`).
> Verified against librdkafka 1.8.0: rejection in 0.28 s, no retry loop. The text below describes the
> behavior **as of the audit date**.

A produce that violates the content contract (`required_prefix`) was answered with **error 56
(KAFKA_STORAGE_ERROR, retriable)** → librdkafka entered an **infinite retry** of the same batch (observed:
re-send every ~100 ms, forever). A contract violation is permanent and should be **INVALID_RECORD (87)** or
another non-retriable error. Location: `datarail-kafka/src/serve.rs` (`produce_results`, the
`Err(_) => (-1, 56)` mapping); the underlying error was swallowed — no broker-side log at all.

## The honest reading

1. **What holds up:** millisecond cold start, single-digit-MB RAM footprint under load, provider-blind
   storage, zero loss with fsync-before-ack, and real compatibility with librdkafka 1.8 (the repo's CI used
   1.7.1).
2. **What does not:** the throughput numbers in the docs **do not describe the product**. The real broker
   moves ~6 MB/s sealing 1 KiB records on this CPU class — the bottleneck is the per-record seal (X25519 +
   Ed25519 + AEAD per message) executed **inside a global Mutex**, not the disk and not the client.
3. **The empirical proof of the architectural defect:** adding a second producer on another partition
   **reduces** aggregate throughput (6.0 → 4.4 MB/s) and the broker never uses more than ~1 core.
   Parallelizing the seal outside the lock (the seal is embarrassingly parallel — every record is
   independent) is probably the single highest-return optimization in the entire project: on 4 cores, ~4× is
   plausible; with VAES, more.
4. **Rule of three (grain of salt):** if the seal dominates, a modern VAES server with a parallel seal could
   take the real broker to tens of MB/s per core — but that is a projection, not a measurement. Today, the
   honest product number is **~6 MB/s per partition on this hardware class**.

## Addendum 2026-07-02 — the parallel seal, A/B'd

The "highest-return optimization" above was implemented the next day: the broker reserves the batch's seq
range under its lock (`SourceTerminal::reserve_seqs`), then seals across cores through an immutable
`board_at(&self, …, seq)` — same rkeys, same order, fresh per-cofre keys from the per-thread CSPRNG. The
per-draw `/dev/urandom` **open** was also replaced with a per-thread persistent fd (the AUDIT-05 property is
untouched: freshness comes from the kernel pool at read time, so a restored snapshot/clone still diverges).

**Method note (important):** between the two sessions this sandbox degraded ~4.5× — the *same serial code
path* that measured 6.0 MB/s on 2026-07-01 measured 1.27–1.40 MB/s on 2026-07-02 (steal=0; shared host).
Cross-session absolute numbers are therefore not comparable; the honest measurement is a **same-minute
interleaved A/B** of two binaries from the same commit (parallel threshold 16 vs `usize::MAX`):

| Run (interleaved, same box, same minutes) | Throughput |
|---|---|
| serial #1 | 1.27 MB/s |
| **parallel** | **3.31 MB/s** |
| serial #2 | 1.40 MB/s |

**≈2.5× from the parallel seal** (3 workers = N−1 on 4 vCPU; adjacent parallel runs ranged 2.6–4.1).
Two producers on two partitions: 2.8 MB/s aggregate — the *catastrophic* negative scaling is gone, but the
global lock still serializes batches, so real multi-producer scaling awaits per-partition locking.
Integrity re-verified: 10,000 + 10,000 records, both partitions, zero loss; a 100-record order-roundtrip
regression test locks the parallel path's semantics. RSS peak ~9.5 MB (unchanged).

```sh
cargo build --release -p datarail-cli
./target/release/datarail kafka-broker examples/rail.toml --listen 0.0.0.0:9092 \
    --advertised 127.0.0.1 --data-dir /tmp/kb --partitions 1
# payload: lines of "evt:" + 1019×'x'; produce: kafkacat -P -t bench -l msgs.txt -X linger.ms=50 \
#   -X batch.num.messages=10000; consume: kafkacat -C -o beginning -c N -e
# cold start: time from exec to a successful connect() on the port, n=30, fresh dir per run
```
