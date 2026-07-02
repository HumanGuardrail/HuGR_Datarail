---
title: "I ran adversarial audits against my own benchmarks, and every impressive number fell"
dek: "On RAM claims that walked from 124x to 40x to 72x, a throughput win that turned out to be a tie, a FAILED gate that had to be re-scoped, and a live bug found by an independent auditor the self-audit missed entirely."
date: 2026-07-02
---

# I ran adversarial audits against my own benchmarks, and every impressive number fell

There is a temptation, after you measure something that looks good, to write it down and move on.
The number is real; you measured it honestly; what else is there to do?

The answer is: try to break it. A benchmark you did not try to break is marketing.

This post is about what happened when I dispatched adversarial auditors against the datarail
benchmark claims — specifically, the `datarail-omb-shim` comparison against Kafka, and later
a full-binary independent benchmark by a separate auditor I did not control. Every impressive
number changed. Some changed a little. Some changed a lot. One bug was found in the product
that I had not found myself.

## The first round: what "124x less RAM" actually meant

The headline claim was that datarail used 124x less RAM than Kafka at the same throughput.
The number came from a real measurement. But real measurements can be misleading.

Adversarial auditor 1 was told to assume the comparison was doing less work. It was. The
measured datarail path used a Loopback substrate, which is essentially an in-RAM struct clone:
`Ok(cofre.clone())`. The terminal's `take_committed` drops records. There is zero disk I/O. The
Kafka side was running `acks=all`, durable, with fsync. "Same throughput" was not "same work."

Adversarial auditor 2 was told to check whether the measurement instruments were comparable.
They were not. The datarail RAM was measured as a bare process via `/proc VmRSS`. The Kafka RAM
was measured via `docker stats`, which on cgroup v2 reports `memory.current − inactive_file`,
excluding the page cache. The auditor measured the real difference by pulling actual cgroup
accounting: 4.8 MiB reported by docker stats vs 84.6 MB actual. Beyond that, re-running the
datarail shim under real load gave roughly 22 MB RSS — not 7 MB, which was a light-load
optimistic sample.

With both corrections applied, the like-for-like loaded gap was approximately 40x — not 124x.
Idle was still roughly 90x, because the Rust process starts lean and the JVM floor is
real. Both numbers are large; neither is 124x.

Adversarial auditor 4 went after the throughput label. The shim batched 128 records per cofre —
amortizing the per-cofre X25519, Ed25519, and entropy work over 128 records. The benchmark
labeled this "1 KB message throughput." It was not. It was "128 KB cofre throughput." At batch
size 1, the sealing rate collapsed roughly 20x. Kafka batches too, so the comparison was
batch-vs-batch and not entirely unfair — but the label "1 KB" was wrong for both sides.

The same auditor noticed that VAES gave only about 2% improvement, which meant the AEAD was
not the bottleneck. The bottleneck was the per-cofre `/dev/urandom` opens (two per cofre,
serializing across threads) plus the asymmetric crypto and per-record allocations. The
"~1,900 MB/s engine" headline number implied a per-cofre cost that was roughly 10x faster than
what the previous benchmark had directly measured per cofre — an internal contradiction that the
auditor surfaced and documented as an inconsistency.

Adversarial auditor 5 found that the durability claim was overclaimed: the local-tmpdir benchmark
never exercised fsync. The `fs::write` + rename gave atomicity, not durability. The "competes
with Kafka on durable delivery at 1/124th the RAM" claim contradicted the project's own design
invariant that the rail has zero durable state.

## What got fixed, and how the labels changed

The honest versions of the corrected claims:

- RAM at idle: ~90x (Rust lean start vs JVM floor — still real, still large)
- RAM under load: ~40x (22 MB vs ~870 MB anon+active, same-ish ruler) — labeled NOT-like-for-like, partly because datarail at this point does not persist
- Throughput: a tie at batch-vs-batch operating points — not a win
- The throughput "win" label was retracted in the docs

The LASTRO-MATRIX (the evidence-backing audit in `docs/design/LASTRO-MATRIX.md`) uses five
verdict labels: BACKED, SINGLE-SAMPLE, NO-REPRO, STALE, and OVERCLAIMED. After the first
adversarial round, several headline claims moved from implied-BACKED to STALE or OVERCLAIMED.

## Round 2: the WAL path and GATE-WARP

The response to audit 5 (durability fake) was to build a real durable substrate — the WAL
(`datarail-substrate-wal`, fsync-on-every-batch) — and measure it against Kafka with
`flush.messages=1` (both leader-fsync, same work). Run 11 in the OMB results showed roughly 67x
less RAM at 51 MB/s. But a second adversarial round found problems with that number too.

The new Kafka baseline (`acks=all`, RF=1, stock flush) does not actually fsync. It acks on
leader-page-cache write. So "both leader-fsync, same work" was false; datarail-WAL was actually
more durable than the Kafka side it was being compared to. The framing was corrected.

The DRBG had a fork-safety bug: no fork detection meant a VM snapshot or `fork()` would clone
the CSPRNG state, making parent and child draw identical `(eph_secret, nonce)` pairs — a
catastrophic key reuse under plain GCM. The docstring had falsely claimed fork-safety. This was
caught by a Round 2 auditor who cold-read the code looking for latent crypto issues. It was fixed
with a PID-change reseed (`49ee1ce`) and a regression test (`drbg_reseeds_on_fork`).

The WAL durability had a real data-loss window: a lost cursor plus GC meant 0/300 un-acked
records recovered (total loss). There was no directory fsync anywhere. A corrupt frame silently
skipped not one but thirteen subsequent records. The O(1) RAM gate had been rigged — it only
measured the send side, hiding that the ack maps were O(in-flight) (+21 MB at 200k un-acked).
All of these were caught by Round 2 auditors, fixed with regression gates, and documented.

GATE-WARP was a performance gate that had been labeled PROVEN based on an n=1 run on a job that
had since been deleted. The LASTRO-MATRIX audit found it: no committed artifact, no reproducible
run. It was downgraded to PENDING-RIGOR, the Northflank artifact was replaced with a committed
CI workflow, and the gate was re-scoped from a per-core metric (which the per-record-bound
architecture could not meet) to an aggregate throughput metric. The per-core characteristic was
recorded honestly as a known property of the architecture.

The throughput "WINS ~1.2x" label was flagged as n=1 with no variance. It was re-run three
times (manually dispatched) and the result was a tie, not a win. The "~72x RAM at equal
fsync durability" headline was run three times with n=3 dispatches (σ 1.6x), then further
confirmed by a non-reclaimable anon measurement (datarail 9 MB vs Kafka 616 MB heap = 68.4x,
consistent with the n=3 71.6x). Tightening the ruler confirmed the number rather than shrinking
it.

## The independent audit: a bug the self-audit missed

The most instructive event was not any of the above. It was the independent benchmark on
2026-07-01 — a separate auditor, no author involvement, measuring the real `datarail kafka-broker`
binary with a real Kafka client (kafkacat 1.6.0 / librdkafka 1.8.0).

That auditor found a live bug.

A produce that violated the content contract (`required_prefix`) was answered with Kafka error
code 56 (KAFKA_STORAGE_ERROR, retriable). librdkafka treated it as retriable and re-sent the
batch every ~100 ms, forever. A contract violation is permanent — the record can never board —
and must map to INVALID_RECORD (87, non-retriable). The underlying error was also swallowed: no
broker-side log at all. The location was `datarail-kafka/src/serve.rs`, specifically the
`produce_results` function's `Err(_) => (-1, 56)` catch-all.

The self-audit had not found this. The CI tests, which ran against librdkafka 1.7.1, had not
surfaced it. The independent auditor, using librdkafka 1.8.0 and actually testing a contract
violation end-to-end, found it within the first session. The fix landed the same day: a
`TerminalError::ContractViolation` now maps to `ErrorKind::InvalidData`, which the serve loop
maps to INVALID_RECORD (87). Regression tests were added: `produce_invalid_data_maps_to_non_retriable_87`
and `produce_other_error_stays_retriable_56`. Verified against librdkafka 1.8.0: rejection in
0.28 seconds, no retry loop.

The self-audit had also not found the negative-scaling defect in the broker. The independent
auditor surfaced that too, as an empirical proof of the architecture: two producers on two
partitions, measuring aggregate throughput lower than one producer, with CPU pegged at 0.97 of
a core.

The LASTRO-MATRIX's standing rule captures the lesson: a claim is only BACKED if a committed
test or harness reproduces it from a real run, n > 1 or exhaustive. Everything else is labeled
down. BACKED, DIRECTIONAL, SINGLE-SAMPLE, NO-REPRO, STALE, OVERCLAIMED — the label costs
nothing; the missing label costs credibility.

A benchmark you did not try to break is marketing. The corrections stayed in the record.
