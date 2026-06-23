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
offered rate, not a ceiling). The standout is the **latency tail**: datarail's **p99 = 7 ms vs Kafka 197 ms /
Pulsar 166 ms** (~24–28× tighter), and **p99.9 = 30 ms vs ~540–560 ms** (~18×). datarail does this **while
sealing every message** — the brokers were in plaintext. datarail's run was clean (0 publish errors, consumer
kept pace, backlog ≈ 0; shim log error-free).

**Why the brokers' tail is so much worse here:** Kafka/Pulsar are durable, batch-oriented logs tuned for bulk
throughput; at a steady moderate rate on a small box their flush/replication/GC introduces periodic tail
spikes. datarail's per-message sealed point-to-point path has no broker queue to spike. This is a *latency-under-
moderate-load* story, and it favors datarail; a *max-throughput* story (below) may read differently — both will
be reported honestly.

### What this is NOT (limits — stated, not hidden)
- **Not a max-throughput / saturation test.** Offered rate was capped at 20 k. The "make them sweat" run
  (`producerRate` uncapped, longer, bigger HW) is the next step — that finds each system's ceiling and its
  behavior at the limit, where the brokers' bulk-throughput design may shine.
- **Not lab-grade HW.** 2-vCPU shared runner; broker + client colocated; single partition (brokers like Kafka
  prefer many partitions — this identical-topology choice is fair-but-modest for them, and noted).
- **Single-node, 1 partition, 1 producer/consumer** — datarail's native point-to-point shape; a fair common
  denominator, not a broker's preferred fan-out topology.

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
