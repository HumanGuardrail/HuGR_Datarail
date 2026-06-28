# COMPETITIVE SCORECARD — datarail vs Kafka / Pulsar / RabbitMQ

> Honest head-to-head for the **streaming/messaging corner** (datarail also competes with MFT, service-mesh,
> and CDC tools in other corners — out of scope here). **datarail numbers = MEASURED by us** (on a VAES core,
> labelled). **Competitor numbers = their OWN PUBLISHED best-case** (vendor/OMB, configured by their experts on
> fat clusters) — used precisely so we cannot be accused of handicapping them, after an earlier self-run
> comparison was (correctly) called out as biased. Every competitor figure is cited + caveated below.

## ⚠️ Read this before the numbers — why a raw "datarail is Nx faster" claim is NOT made here

1. **Different hardware.** Every published broker number is a **fat 3-broker cluster** (Kafka/Pulsar: 3×
   i3en.2xlarge–6xlarge = 24–72 vCPU total, NVMe, 25 Gbps). datarail's numbers are **per single core**. Cluster
   totals ÷ cores ≈ tens of MB/s/core for the brokers; not directly comparable.
2. **Different jobs.** The brokers do **durable, replicated, multi-consumer log** with retention + replay.
   datarail is **point-to-point A→B sealed movement** — it does NOT replicate to a quorum, fan out to consumer
   groups, or retain history. Comparing their throughput to datarail's is apples-to-oranges.
3. **Disputed sources.** The canonical Kafka-vs-Pulsar numbers come from a **Confluent (2020) vs StreamNative
   (2020/2022) vendor fight** that reached opposite conclusions due to an async-vs-sync **durability mismatch**.
   Treat as marketing, not neutral.

**So: we do NOT claim a throughput win.** datarail's per-core sealing rate is strong (below), but the honest,
**un-riggable** verdict is on the **structural axes** — where the brokers cannot follow by construction.

## The scorecard

| Dimension | **datarail** (measured) | Kafka | Pulsar | RabbitMQ | Verdict |
|---|---|---|---|---|---|
| **Provider-blind** (operator structurally cannot read payloads) | ✅ **sealed E2E** (AEAD; the rail decodes only the cleartext header, never `carga`) | ❌ plaintext to broker; E2E (KIP-317) **never shipped** | ❌ plaintext default (client-side E2E optional, non-default) | ❌ plaintext; broker routes through a central exchange | **datarail — alone. No head-on competitor exists.** |
| **RAM to move data** (measured, same 51 MB/s) | ✅ **~22 MB load · 2-3 MB idle** · ~1.2 cores | ❌ 870 MB load · 275 MB idle · 1.49 cores | ❌ 1,757 MB load · 810 MB idle | ❌ ~256MB–1.5GB | **datarail — footprint is the disruption axis, not speed. AUTHORITATIVE figure: fsync-vs-fsync ~72× less RAM (Run 12, n=3 — see `LASTRO-MATRIX.md`/`OMB-RESULTS.md`).** The ~40× loaded / ~90× idle shown here is an EARLIER, rougher non-durable (loopback) comparison (22 MB vs 870 MB, not like-for-like rulers; corrected from an over-claimed 124× by `ADVERSARIAL-AUDIT.md`). Cite ~72× as the headline. |
| **Serverless / idle ≈ 0** | ✅ spawn → deliver → vanish; no standing state | ❌ always-on brokers + KRaft/ZK | ❌ always-on brokers + **BookKeeper** + ZK (heaviest) | ❌ always-on Erlang nodes (~256 MiB–1.5 GiB idle) | **datarail — the others bill 24/7** |
| **Exactly-once** | ✅ effectively-once (1000-seed DST: 0-loss/0-dup) | ✅ EOS (txns, costs throughput) | ✅ txns (2.8+) | ❌ **at-most / at-least-once only** | datarail ✅ · Kafka/Pulsar ✅ · Rabbit ❌ |
| **+ cryptographic delivery proof** | ✅ **Merkle receipt, independently verifiable** | ❌ | ❌ | ❌ | **datarail — alone** |
| **Survives untrusted transport** (any dumb pipe / hostile S3 / peer) | ✅ seal makes the pipe untrusted-by-design | ❌ you operate the cluster (it IS the trust) | ❌ | ❌ | **datarail — alone** |
| **Max throughput** (32-core, measured, SEALED) | **engine ~1900 MB/s @1KB / ~2500 @16KB** · full-TCP ~1534 @1KB | **1604 MB/s** (OMB, plaintext) | 296–441 (OMB) | ~38 (pub.) | **n=3 CORRECTED (`THROUGHPUT-RESULTS.md`):** sealed engine **~1.4 GB/s @1KB** (1399/1432/1400, tight) vs Kafka **1.6 GB/s plaintext** ⇒ **~0.87×, a TIE slightly BEHIND @1KB**, ahead only @≥4KB (~1.7 GB/s). The old n=1 "1900 / WINS 1.2×" was a favorable single run — **corrected: parity-class, NOT a win.** The moat is efficiency, not throughput. |
| **p99 latency** (OMB, measured) | **5 ms** @20k, sealed (32-core) | **2 ms** (32-core) | ~10 ms | ~1 ms but only ≤30 MB/s (published) | **roughly a tie on adequate HW** — both single-digit ms; datarail does it sealed. (On a starved 2-core box datarail was ~25× tighter, but that was Kafka GC/flush contention, not fundamental — honest correction in `OMB-RESULTS.md`.) |
| **Throughput under packet loss** (WAN) | 🟡 FASP delay-CC **algorithm**: 16–42× a textbook AIMD in *seeded-loss sim* (not real network; controller not yet wired to a real UDP transport — see `FASP-UDP-TRANSPORT.md`) | loss-based TCP (collapses under loss) | loss-based TCP | loss-based TCP | **datarail — the FASP physics, once realized over UDP** |
| **Durable multi-consumer fan-out / retention / replay-at-scale** | ❌ point-to-point A→B; not a log | ✅ **the brokers win** | ✅ **win** (tiered storage) | ⚠️ queues/streams | **brokers win — honest** |
| **Ecosystem / maturity** | ❌ new | ✅ **huge** (Connect, ksqlDB) | ✅ growing | ✅ mature, rich routing | **brokers win — honest** |

## The honest thesis

datarail does **not** win the "better broker" fight — Kafka and Pulsar are mature, expert-tuned, GB/s durable
logs with ecosystems datarail doesn't have, and for **durable multi-consumer fan-out + replay they beat it
outright.** That's in the scorecard, not hidden.

**datarail wins a fight the brokers structurally cannot enter:** a **provider-blind, serverless,
point-to-point sealed rail with a cryptographic delivery receipt, that rides any untrusted pipe.** The research
is explicit — *provider-blind messaging has no head-on product competitor*; all three brokers see plaintext by
default, and Kafka's own end-to-end-encryption proposal (KIP-317) has sat "Under Discussion" for years, never
shipped. The moat is the **intersection** (provider-blind × serverless × point-to-point × proof), not any
single attribute.

> One-liner: **"You don't pick datarail because it out-throughputs Kafka. You pick it when the pipe — even your
> own broker, even a cloud bucket — is not allowed to see the data. No broker can do that without becoming a
> different product."**

**Now partly MEASURED, not just published:** the OpenMessaging Benchmark (industry standard) runs in CI against datarail + Kafka + Pulsar on identical HW — see [`OMB-RESULTS.md`](OMB-RESULTS.md). On a 32-core runner at fixed 20k msg/s both datarail and Kafka show single-digit-ms p99 (datarail 5 ms sealed, Kafka 2 ms plaintext) — a tie on latency, datarail doing it sealed. (The ~25× gap seen on a 2-core box was resource contention, not fundamental — corrected honestly.) Max-throughput rate-discovery on the 32-core box is the headline still landing; on bulk throughput the brokers' batched-log design may lead.

## The credible throughput fight (offer)

To put datarail on the **industry-standard scoreboard** without us configuring (and possibly handicapping)
anyone's software: write a **datarail driver for the OpenMessaging Benchmark (OMB)** — the Linux-Foundation
harness the vendors themselves use. Then anyone runs the identical workload against datarail + Kafka + Pulsar +
RabbitMQ **on equal, well-tuned hardware**, independently. That's the only throughput comparison that can't be
accused of bias. (Caveat: OMB models broker pub/sub; datarail's point-to-point shape maps to a single
producer→consumer path — a fair-but-partial fit, documented.)

## Sources (competitor numbers — all vendor/published, cited)

- Confluent OMB (2020): Kafka 605 MB/s, Pulsar 305 MB/s, RabbitMQ 38 MB/s — `confluent.io/blog/kafka-fastest-messaging-system`
- StreamNative rebuttal (2020) + re-run (2022): Pulsar ≈/> Kafka at equal durability — `streamnative.io/blog`
- KIP-317 (Kafka E2E encryption) — **"Under Discussion", never adopted** — `cwiki.apache.org KIP-317`
- Kafka EOS — `confluent.io/blog/exactly-once-semantics...`; Pulsar txns — `streamnative.io`; RabbitMQ guarantees (no EOS) — `rabbitmq.com/docs/reliability`; RabbitMQ memory — `rabbitmq.com/docs/memory`
- (datarail numbers: `BENCH-01.md` + `DOD-01.md` — measured on a Northflank VAES core, 2026-06-23.)
