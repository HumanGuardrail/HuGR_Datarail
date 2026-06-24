# ADVERSARIAL-AUDIT — 5 brutal skeptics vs the datarail benchmark claims (2026-06-24)

> The owner's instinct ("the benchmark looks too good to be true") was correct. We dispatched 5 independent
> adversarial auditors, each told to assume fraud and DESTROY a specific claim, with license to cold-read the
> code + re-run the benchmarks. The lead cold-verified every load-bearing finding against source. **Result: the
> measured numbers are REAL (not fabricated), but several headline CLAIMS were more aggressive than the truth.**
> This document records the findings and the corrections. Honesty over hype.

## Verdicts

| # | Attack | Verdict | Load-bearing finding (lead-verified) |
|---|---|---|---|
| 1 | "datarail does less work" | **PARTLY MISLEADING** | The measured path (`Loopback` substrate) is `Ok(cofre.clone())` — an in-RAM struct clone, no disk/socket/persist; `take_committed` drops records. The terminal has **zero** disk I/O. Kafka ran `acks=all` (durable fsync). So "124× less RAM at same throughput" compared *in-memory sealed movement* vs *durable replicated log*. **Idle ~90× (3 MB vs 275 MB) is a real lean-Rust-vs-JVM win; the under-load delta is mostly "Kafka stores, datarail forgets."** |
| 2 | "the measurement instrument is broken" | **BIASED / NOT LIKE-FOR-LIKE** | datarail measured as a **bare process** `/proc VmRSS`; Kafka as a **container** `docker stats` which on cgroup v2 = `memory.current − inactive_file` → **excludes Kafka's page cache** (empirically: 4.8 MiB reported vs 84.6 MB real). And the cited **7 MB is not reproducible** — re-running the real shim under load on Linux gives **~22 MB RSS/Pss** (lead re-measured: idle 2 MB, loaded ~22 MB). Recomputed gap: **~40×, not 124×.** |
| 3 | "datarail is faking/skipping the crypto" | **CLEAN — work is real** | Fresh ephemeral X25519 + fresh nonce + Ed25519 sign per cofre; no no-op cipher variant; no pass-by-ref cheat (loopback skips byte-serialization but the carga is real ciphertext, really decrypted); bounded channels back-pressure (0 `try_send`, no silent drops); roundtrip test proves 0-loss/0-dup exactly-once. **The sealing and delivery are genuinely performed.** |
| 4 | "the throughput numbers are inflated" | **INFLATED LABEL + INTERNAL INCONSISTENCY** | The shim batches **128 records/cofre** (`batch_max_records=128`), amortizing the per-cofre X25519+Ed25519+entropy over 128 records. **"1 KB-message throughput" is really "128 KB-cofre throughput"** — batch=1 (true per-1KB-message sealing) collapses **~20×**. **VAES only +2% because AEAD was never the bottleneck** — the cost is 2× `/dev/urandom` opens per cofre (which serialize across threads), asymmetric crypto, and per-record `Vec` allocations. The ~1,900 MB/s engine implies ~2,207 µs/cofre vs BENCH-01's measured 227 µs/cofre — a **~10× internal contradiction** (the engine number is likely contention/memory-bound on 32 saturated vCPUs, not a sealing rate). Kafka also batches (1 MB / linger 1 ms), so the *comparison* is batch-vs-batch (fair-ish); the *label* "1 KB" is misleading for both. |
| 5 | "the durability is fake" | **DATA REAL, RSS-FLAT REAL, but "durable" OVERCLAIMED** | Each stored cofre is genuinely distinct, non-sparse ciphertext on disk (fresh key+nonce → verified distinct); RSS stays flat at ~2 MB on the **write** path (architecturally guaranteed — the source accumulates nothing). BUT: **zero fsync anywhere in the workspace** (`fs::write`+rename = atomicity, not durability → lost on power-loss) and **no replication** (single local copy → lost on disk/node loss). This is **NOT Kafka-grade durability** (acks=all + fsync + RF≥3). SPEC-10 delegates real durability to S3's replication, which the local-tmpdir bench never exercises. The "competes with Kafka on durable delivery at 1/124th the RAM" claim contradicts the project's own `INV-EPHEMERAL-RAIL` ("zero durable state"). |

## What was REAL (survives the audit)
- **The crypto + delivery are genuine** (Agent 3): every message sealed end-to-end, 0-loss exactly-once.
- **datarail is genuinely lean** — idle **2–3 MB** vs Kafka's **275 MB** JVM floor ≈ **~90×**, a real lean-Rust-vs-JVM advantage that holds at any rate and underpins the serverless/TCO story.
- **Loaded footprint ~22 MB** vs Kafka's ~870 MB (anon+active, page cache excluded) ≈ **~40×** resident — large and real, even if not perfectly like-for-like.
- **The write-side RAM-flatness as stored volume grows is real** (~2 MB while disk → GBs) — a genuine "stored-volume decoupled from RAM" property for store-and-forward buffering.
- **Throughput is in Kafka's class** (~1.4–1.9 GB/s batched, both batch) — a genuine tie, not a loss.

## What was OVERCLAIMED (corrected in the docs/HTML)
1. **"124× less RAM"** → honest **~40× loaded (22 MB vs 870 MB), ~90× idle** — and labeled NOT-like-for-like (bare `/proc` vs container `docker stats` minus page cache) + partly does-less (loopback doesn't persist).
2. **"7 MB under load"** → **~22 MB** measured (the 7 MB was a light-rate/optimistic sample).
3. **"moves AND durably stores at 1/124th the RAM"** → split: leanness PROVEN; "durable" downgraded — the store is un-fsync'd, un-replicated (NOT Kafka-grade); real durability delegated to S3, not yet measured.
4. **"1 KB sealed throughput ~1.9 GB/s"** → labeled **128 KB-cofre batched**; per-1KB-message sealing is ~20× lower; VAES irrelevant at 1 KB; engine number is contention-bound (10× off BENCH-01's per-cofre cost).

## The honest path forward (the fair "durable AND leaner" proof)
To make the durability/efficiency claim defensible — **same work, same ruler** — requires engineering, not assertion:
1. A durable substrate that **actually fsyncs** (match Kafka's `acks=all` power-loss durability) with an **O(1) bounded drain** (the current demo drain bookkeeping grows).
2. Measure **both** datarail-durable and Kafka-durable with the **same ruler** (both containerized, cgroup `memory.current` *including* page cache; report peak + avg after a real JVM warm-up).
3. Then the "datarail does the same durable work at a fraction of the RAM" number is honest — and likely still a large win (a Rust appender + fsync + bounded drain has no GB-scale page-cache working set), but it must be **measured**, not claimed.

> Bottom line: the engineering is real and the lean-process win is real (~40–90×). The benchmark's sin was
> **framing** — comparing non-equivalent work with non-equivalent rulers and labeling batched/optimistic numbers
> as more than they were. Every such claim is now corrected. The adversarial audit did exactly its job.
