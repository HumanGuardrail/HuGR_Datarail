# OMB-RESULTS — datarail vs Kafka/Pulsar via the OpenMessaging Benchmark

> **The industry-standard harness, run by CI, reproducible by anyone.** This is the OpenMessaging Benchmark
> (Linux Foundation — the same harness Confluent/StreamNative use), driving datarail through a `BenchmarkDriver`
> over the `datarail-omb-shim`, against Kafka and Pulsar through their official drivers — **same machine, same
> workload, same harness**. Re-run it: GitHub Actions → "OMB Benchmark" workflow (`.github/workflows/omb-benchmark.yml`).

## Run 1 — smoke / fixed-load latency (2026-06-23)

- **Runner:** `ubuntu-latest` (2 vCPU, 7 GB) — a small free runner. **DIRECTIONAL, not a ceiling.**
- **Workload:** `ci-smoke.yaml` — 1 topic / 1 partition / 1 producer / 1 consumer, 1 KB messages, **offered rate
  capped at 20 000 msg/s**, 1 min. Identical topology for every system.
- **What it measures:** end-to-end **latency at a fixed, modest, sustainable load** — NOT maximum throughput
  (everyone is rate-limited to 20 k msg/s, so all three sustain ~20 k; the differentiator is the latency tail).
- **datarail** runs its **FULL sealed, provider-blind path** (board → AEAD-seal → sign → loopback → verify →
  open → offload); **Kafka and Pulsar run their plaintext OMB defaults** (acks=1/quorum 1, no encryption).

| system | achieved msg/s | MB/s | E2E p50 | **E2E p99** | **E2E p99.9** |
|---|---|---|---|---|---|
| Kafka 3.8 (KRaft) | 20 014 | 20.5 | 2 ms | **197 ms** | 564 ms |
| Pulsar 3.3 (standalone) | 20 034 | 20.5 | 9 ms | **166 ms** | 542 ms |
| **datarail** (sealed) | 20 040 | 20.5 | 3 ms | **7 ms** | **30 ms** |

**Honest reading:** at a fixed 20 k msg/s, 1 KB, single-node, all three keep up on throughput (that's the
offered rate, not a ceiling). On this **2-core runner** datarail's tail was far tighter — **p99 7 ms vs Kafka
197 ms / Pulsar 166 ms**.

> ⚠️ **CORRECTION (Run 3, Turbo 32-core):** that huge gap was **largely resource contention on the starved
> 2-core box, NOT a fundamental Kafka problem.** Re-run at the same fixed 20 k on a 32-core/128 GB runner:
> **Kafka p99 = 2 ms, datarail p99 = 5 ms** — both excellent, Kafka slightly *lower*. So do **not** cite
> "datarail's p99 is ~25× Kafka's" as a general claim; it holds only on a CPU-starved box (where Kafka's
> GC/flush spikes balloon). The honest fixed-load statement is: **on adequate HW datarail's latency is
> single-digit-ms and competitive with Kafka — while sealing every message (Kafka was plaintext).** The
> durable structural wins (provider-blind, serverless, exactly-once+proof) stand regardless; raw latency at
> moderate load is roughly a tie on real HW.

**Why the brokers' tail is so much worse here:** Kafka/Pulsar are durable, batch-oriented logs tuned for bulk
throughput; at a steady moderate rate on a small box their flush/replication/GC introduces periodic tail
spikes. datarail's per-message sealed point-to-point path has no broker queue to spike. This is a *latency-under-
moderate-load* story, and it favors datarail; a *max-throughput* story (below) may read differently — both will
be reported honestly.

### What this is NOT (limits — stated, not hidden)
- **Not a max-throughput / saturation test.** Offered rate was capped at 20 k. The "make them sweat" run
  (`producerRate` uncapped, longer, bigger HW) is the next step — that finds each system's ceiling and its
  behavior at the limit, where the brokers' bulk-throughput design may shine.

## Run 2 — saturation attempt + a real backpressure finding (2026-06-23)

Pushing an **unreachable offered rate** (`producerRate: 1_000_000`, 4 topics) to find ceilings surfaced two
honest things:

1. **datarail backpressures gracefully — verified.** First the OMB shim's ingress was unbounded and acked
   before sealing, so under a firehose the internal queue grew (backlog 1.1 M → 6.7 M msgs) until OOM — a real
   bug that *violated* datarail's own 0-loss/backpressure claim. Fixed at the root: a **bounded ingress queue**
   (the producer is throttled when the seal/open worker is the bottleneck) **+ draining the delivery sink each
   batch** (the demo sink otherwise retains every record). **Local firehose test (533 k msgs, deliberately slow
   consumer): shim RSS stays flat at ~64 MB, 0 loss** — datarail throttles instead of growing. On the runner,
   the fair 4-topic run then held **backlog ≈ 5–8 k msgs, pub ≈ cons ≈ ~60 k msg/s (~60 MB/s) sealed, 0 errors**
   — graceful degradation, exactly the claim.
2. **The unreachable-rate method OOMs the OMB *client*, not datarail.** With offered ≫ capacity and datarail
   backpressuring to ~60 k msg/s, the OMB Java producer buffers the ~940 k msg/s it can't send into its own
   heap → the **OMB client** OOMs (verified: datarail's shim memory is flat under the same firehose). So a fair
   **max-throughput** number needs OMB's **rate-discovery** (`tool/` ramps to each system's max sustainable
   rate), not a fixed unreachable rate — and ideally a larger runner. That run is pending (it's the headline
   throughput comparison; this CI proved the harness + datarail's backpressure, not the ceiling).

**Honest status:** datarail's *latency-at-sustainable-load* win is measured (Run 1). datarail's *graceful
backpressure / 0-loss under overload* is measured (Run 2, local + CI). The *max-throughput ceiling vs the
brokers* is not yet a fair number — it needs rate-discovery on adequate HW, and on bulk throughput the brokers'
batched-log design may well lead (stated up front, not hidden).
- **Not lab-grade HW.** 2-vCPU shared runner; broker + client colocated; single partition (brokers like Kafka
  prefer many partitions — this identical-topology choice is fair-but-modest for them, and noted).
- **Single-node, 1 partition, 1 producer/consumer** — datarail's native point-to-point shape; a fair common
  denominator, not a broker's preferred fan-out topology.

## Run 3 — THE headline: max-throughput (rate-discovery, 32-core Turbo, 2026-06-23)

- **Runner:** GitHub-hosted **`Turbo` — 32 vCPU / 128 GB**. **Workload:** `discovery-1kb.yaml` —
  `producerRate: 0` (OMB `findMaximumSustainableRate` ramps to each system's ceiling), 4 topics, 1 KB, 1 min
  warmup + 4 min. datarail **sealed**; Kafka/Pulsar **plaintext** OMB defaults.

| system | **max sustainable msg/s** | **MB/s** | p50 | p99 | p99.9 |
|---|---|---|---|---|---|
| **Kafka 3.8** | **1,498,609** | **1,534.6** | 3 ms | 177 ms | 590 ms |
| Pulsar 3.3 | 430,440 | 440.8 | 8 ms | 49 ms | 100 ms |
| **datarail** (sealed) | **226,776** | **232.2** | 58 ms | 72 ms | 86 ms |

**Honest verdict — Kafka wins raw throughput, decisively.** Kafka sustained **1.5 GB/s (6.6× datarail)**, Pulsar
**440 MB/s (1.9× datarail)**, datarail **232 MB/s** — the slowest of the three on bulk throughput. This is the
brokers' home turf (batched, append-only logs built for exactly this) and the result is unambiguous: **datarail
is NOT a throughput-beats-Kafka story.** Anyone claiming otherwise is selling something.

**Honest context (not excuses):**
- datarail moves **232 MB/s while AEAD-sealing + signing every message and staying provider-blind**; Kafka/Pulsar
  moved plaintext the broker can read. Adding real end-to-end encryption to a broker costs throughput it didn't
  pay here.
- The OMB **shim runs one worker thread per topic**, so with 4 topics datarail used ~4 of the 32 cores
  (~58 MB/s/core sealed); Kafka/Pulsar use many threads across the whole box. datarail's per-core sealed rate is
  respectable, but the shim does not yet parallelize one topic across cores — a benchmark-adapter limit, honestly
  noted, not benchmarked away (the measured number is 232 MB/s, full stop).
- **Latency shape:** at each system's *own* max, datarail's distribution is the **tightest** (p50→p99.9 =
  58→86 ms, a 1.5× spread), while Kafka runs hot at its ceiling (3→590 ms, ~200× spread) and Pulsar sits between.
  datarail trades peak throughput for predictability.

**So who's "the brabo"?** On throughput: **Kafka.** datarail's case was never raw GB/s — it's the structural
moat (provider-blind × serverless × exactly-once + proof) at *respectable* sealed throughput (232 MB/s would
saturate a 1.8 Gbps link) with the most predictable latency. Different tool, different job — measured honestly.

## Run 4 — ENGINEERING closes the gap (2026-06-23)

The Run 3 throughput gap was **not fundamental — it was unbuffered, per-message socket I/O on both sides.**
Fixing it (pure engineering, same workload/HW: 16-topic rate-discovery, 32-core Turbo, sealed):

| datarail config | throughput | note |
|---|---|---|
| 4 topics, unbuffered | 232 MB/s | Run 3 baseline |
| 16 topics, unbuffered | 269 MB/s | +cores barely moved it ⇒ **not core-bound** |
| 16 topics, **shim** I/O buffered | 278 MB/s | shim `BufReader`/`BufWriter` |
| 16 topics, **both** sides buffered | **624 MB/s** | + Java producer batch-flush (was flushing per message) |

**+2.7× from I/O buffering alone.** datarail now sustains **624 MB/s sealed at p99 7 ms** (tight: p50 4 /
p99 7 / p99.9 8 ms). **The gap to Kafka (1,604 MB/s) fell from 6.6× to 2.6×** — and datarail is sealing every
message while Kafka runs plaintext.

**Root cause (both were the same classic bug):** the shim did ~4 syscalls/msg (unbuffered frame reads + acks +
delivery writes); the OMB Java producer did `flush()` per `sendAsync`. At ~270k msg/s that is ~1 M syscalls/s —
the ceiling. Coalescing I/O (BufReader/BufWriter on the shim, a ~0.8 ms background flusher on the producer)
removed it. Kafka was never CPU-faster here; it just batched I/O and the adapter didn't.

**Remaining gap (honest):** datarail is now ~38 k msg/s per topic-worker × 16 ≈ 609 k — the per-worker rate is
capped by per-message processing overhead (Vec allocations + the fan-out payload clone + channel ops) and by
thread oversubscription (16 shim workers + the OMB client's threads on 32 cores). Closing the last 2.6× needs
hot-path allocation reduction + parallelism tuning — real work, not yet done. **Crypto is NOT the limit**
(GCM-SIV seals 1 KB in ~0.7 µs, <6 % of the per-message cost — so the VAES backend wouldn't move this number).

**Honest standing:** Kafka still leads raw throughput (1.6 GB/s vs 624 MB/s, 2.6×), but the "6.6× slower"
verdict was an artifact of an unoptimized adapter, not datarail's design. With straightforward engineering
datarail does **624 MB/s sealed + provider-blind at 7 ms p99**.

### How far further engineering got (and where it hit a wall — honest)
After the I/O-buffering win, two more optimizations were tried and **measured on the 32-core box**:
- **Hot-path allocation cut** (Arc record + zero-split egress: no per-message payload copy/clone): +28 % on a
  single local worker, but **0 % on the 32-core aggregate** (still 624 MB/s). ⇒ at 16 topics datarail is **not
  per-worker-CPU-bound**.
- **Consolidating 16 per-producer flusher threads into one**: **0 % aggregate change** (622 MB/s).
- Diagnosis: per-stream rate is **65 k msg/s at 8 topics but 38 k at 16** — i.e. the ceiling is **thread
  oversubscription in the thread-per-connection model** (≈112 threads on 32 cores), not CPU and not crypto. The
  aggregate plateaus at **~620 MB/s** regardless of CPU or thread-count tweaks.

**Verdict on "can engineering beat Kafka here":** I/O buffering closed the gap from 6.6× to **2.6×** — real and
measured. Breaking past ~620 MB/s would need a **different concurrency model** (async I/O / epoll-style event
loop instead of thread-per-connection) — a genuine shim rewrite with **uncertain** payoff against Kafka's
decade-tuned 1.6 GB/s plaintext log. **Honest bottom line: datarail is now 2.6× off the throughput king while
sealing every message; closing the rest is a real async-rewrite project, not a quick win — and raw throughput
was never datarail's reason to exist.** The structural moat (provider-blind × serverless × exactly-once+proof)
stands independent of this race.

## RabbitMQ — honest non-result
RabbitMQ's container **could not start in this CI sandbox**: the Erlang node fails with
`.erlang.cookie: eacces` on the GitHub runner's overlay filesystem (reproduced across `-p`, `--network host`,
`RABBITMQ_ERLANG_COOKIE`, and a tmpfs data dir — a known container/overlay-perms quirk, **orthogonal to the
benchmark**). Rather than ship a rigged or absent number, we cite its **published** figure: **~38 MB/s** (Confluent
OMB 2020, 1 KB, 3× i3en.2xlarge) — the slowest of the three, and RabbitMQ has **no exactly-once** (see
`COMPETITIVE-SCORECARD.md`). The driver + config are committed (`bench/omb/`), so the run reproduces in any
environment where the broker starts normally.

## Reproduce
GitHub → Actions → **OMB Benchmark** → Run workflow. Inputs: `runner` (use a larger-runner label for the real
ceiling run), `systems` (`datarail,kafka,pulsar[,rabbitmq]`), `workload`. Raw result JSON + per-system logs are
uploaded as an artifact; the step summary renders the table. Driver: `bench/omb/driver-datarail`; shim:
`crates/datarail-omb-shim`; protocol: `docs/design/OMB-PROTOCOL.md`.
