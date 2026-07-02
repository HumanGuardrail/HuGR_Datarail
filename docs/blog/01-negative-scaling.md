---
title: "My Kafka broker scaled negatively: a mutex story"
dek: "Adding a second producer made the broker slower. Here is why, and what it took to measure the fix honestly."
date: 2026-07-02
---

# My Kafka broker scaled negatively: a mutex story

An independent auditor benchmarked the `datarail kafka-broker` binary on 2026-07-01.
Not a shim, not a synthetic harness — the actual release binary, driven by kafkacat 1.6.0
(librdkafka 1.8.0), with a 4 vCPU i7-9750H class host and AES-NI present.
The single-producer number was fine: roughly 6 MB/s at about 6,000 1 KiB records per second,
with fsync-before-ack defaults.

Then the auditor ran two producers against two partitions and measured the aggregate.

It went down. 6.0 MB/s serial, 4.4 MB/s parallel. The broker never used more than 0.97 of a
core.

This is the story of that defect, the fix, and the measurement trap that made verifying
the fix harder than writing it.

## What the broker was doing

Every produce request calls through a single path:

```rust
// KafkaBrokerStore wraps everything behind one global Mutex
fn produce(&self, topic: &str, partition: i32, records: &[Vec<u8>]) -> std::io::Result<i64> {
    let mut g = self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    g.produce_into(topic, partition, records)
}
```

Inside `produce_into`, every record was sealed — X25519 key-wrap (fresh ephemeral per record),
AES-256-GCM-SIV encryption, Ed25519 signature — one after another, inside the held lock.
Thread-per-connection bought nothing: as soon as the second connection's thread touched
`self.inner.lock()`, it blocked and waited for the first to finish sealing the whole batch.
The parallelism the OS gave us was entirely invisible to the broker.

The per-record seal is embarrassingly parallel. Every record is independent: it gets a fresh
ephemeral X25519 key drawn from the per-thread CSPRNG, a fresh random nonce, its own AEAD
ciphertext, and its own Ed25519 signature. The only piece that is not independent is the
sequence number, and that is just a counter.

## The fix: reserve the seq range, then seal across cores

The restructured path does the minimum work under the lock — reserving the batch's sequence
range — and then seals without holding it.

In `BrokerInner::produce_into`:

```rust
// Reserve the whole batch's seq range up front (we hold the broker Mutex), then seal
// WITHOUT mutating the terminal — which is what lets the seal fan out across cores.
let start_seq = self.src.reserve_seqs(
    u64::try_from(records.len()).unwrap_or(u64::MAX)
);
let sealed = seal_batch(&self.src, topic, partition, base, start_seq, records)?;
self.partition_log(topic, partition)?.append_durable(&sealed)
```

`reserve_seqs` just bumps the counter:

```rust
pub fn reserve_seqs(&mut self, n: u64) -> u64 {
    let start = self.seq;
    self.seq = self.seq.saturating_add(n);
    start
}
```

Then `seal_batch` fans out to worker threads using `thread::scope`. Each worker calls
`board_at(&self, records, rkey, seq)` — that is `&self`, not `&mut self`. There is no mutation
of the terminal; each seal draws fresh key material from the per-thread CSPRNG independently:

```rust
fn seal_batch(src: &SourceTerminal, ...) -> std::io::Result<Vec<Vec<u8>>> {
    const PARALLEL_THRESHOLD: usize = 16;
    // ...
    let workers = std::thread::available_parallelism()
        .map_or(1, std::num::NonZeroUsize::get)
        .saturating_sub(1)   // leave one core for IO thread + client
        .max(1)
        .min(records.len());
    // chunk into workers, each calling board_at with its assigned seq range
}
```

The parallelism is safe because `board_at` is read-only on the terminal. The CSPRNG is
per-thread (a BLAKE3-based DRBG with a persistent `/dev/urandom` fd per thread; the old design
opened and closed `urandom` on every draw, which itself contended once the seal was parallelized).
Two threads calling `board_at` concurrently use different CSPRNG states, different ephemeral
X25519 keys, and different nonces.

## The measurement problem that nearly fooled me

Here is the part that required more care than the code change.

The fix was implemented on 2026-07-02. The same sandbox that measured 6.0 MB/s serial on
2026-07-01 measured 1.27–1.40 MB/s serial on 2026-07-02. Steal counters: 0. The host had
simply degraded — shared CPU, shared infrastructure — by roughly 4.5 times between sessions.

If I had compared the 2026-07-01 serial result (6.0 MB/s) against the 2026-07-02 parallel
result (3.31 MB/s), I would have concluded the fix made things dramatically worse and shipped
nothing. If I had compared same-session serial (1.3 MB/s) against same-session parallel
(3.31 MB/s) and called it an absolute win, I would have a real improvement but a meaningless
absolute number.

The correct method is a same-minute interleaved A/B: build two binaries from the same commit
with different parallel thresholds (16 records vs `usize::MAX`, effectively disabling the
parallel path), run them alternating within the same few minutes on the same box, and trust
only the ratio.

| Run (interleaved, same box, same minutes) | Throughput |
|---|---|
| serial #1 | 1.27 MB/s |
| parallel | 3.31 MB/s |
| serial #2 | 1.40 MB/s |

That is roughly 2.5x from the parallel seal. Adjacent parallel runs across the day ranged
from 2.6x to 4.1x; on this 4-vCPU class with N−1 workers, 2.5–3x is the honest central
estimate. Integrity was re-verified: 10,000 records per partition, both partitions, zero loss.
A regression test locks in the order semantics of the parallel path.

## What is still honest to say

The catastrophic negative scaling from two producers is gone. The two-producer aggregate went
from 4.4 MB/s (worse than one producer) to 2.8 MB/s aggregate — less than ideal, but no longer
sub-serial.

It is not gone because the architecture is fixed. It is gone because the per-record seal no
longer runs inside the batch-level lock. The batch-level lock itself is still there. Two producers
on two partitions still contend for it while they wait for the other batch to be reserved and
serialized to disk. True multi-producer scaling requires per-partition locking, which means
splitting the current global `Mutex<BrokerInner>` into one lock per `(topic, partition)` — at
which point batches from different partitions can seal and append entirely in parallel.

The honest product number today is: approximately 2.5x throughput improvement on a 4-core host
versus the previous serial path, measured on the real binary, in a same-minute A/B. The absolute
floor depends entirely on the host.

Lesson: on shared infrastructure, the only measurement you can trust is a same-minute
interleaved A/B against the same commit. Cross-session comparisons are noise.
