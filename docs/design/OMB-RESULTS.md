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
