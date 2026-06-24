# EFFICIENCY-TCO — what datarail's footprint means in dollars

> The disruption thesis, grounded in **measured** numbers (see `OMB-RESULTS.md` Run 10): datarail moves the same
> data at a footprint 100–250× smaller. Here we translate that into instance sizing + TCO — with explicit
> assumptions, labeled DERIVED (illustrative), not measured. Cloud prices are rough, ~2026, US regions; verify
> against current rates. The point is the *ratio*, which is footprint-driven and robust.

## The measured facts (Run 10 — 32-core, OMB harness, all at an identical 51 MB/s)

| | datarail | Kafka | Pulsar |
|---|---|---|---|
| RSS idle | 3 MB | 275 MB | 810 MB |
| RSS under load | 7 MB | 870 MB | 1,757 MB |
| CPU under load | 1.24 cores | 1.49 cores | 0.83 cores |

**RAM: 124× (Kafka) / 251× (Pulsar) less. CPU: ~parity. Idle: 92–270× less.**

## Why footprint → dollars: instance sizing is RAM-bound for brokers

A cloud instance is sized by its **binding constraint**. For a broker that constraint is **memory**: Kafka/Pulsar
need GBs (JVM heap + off-heap + page cache + — for Pulsar — BookKeeper + ZK). For datarail the constraint is
**CPU** (a few cores), because its memory is negligible. So:

- **datarail** at 51 MB/s fits a **1-vCPU / 256 MB** task — or a serverless function (AWS Lambda's floor is
  128 MB; datarail's 7 MB lives there with room to spare). It is sized by its ~1.3 cores, not memory.
- **Kafka** at the same 51 MB/s needs **~1 GB+ per broker just to run**, and for any durability/HA a **3-broker
  cluster** that is **always on** (it holds the durable log — it cannot scale to zero). Realistically a managed
  Kafka (MSK / Confluent) has a **multi-hundred-$/month floor** regardless of how little data you move.

## TCO scenario (DERIVED, illustrative) — "move 51 MB/s of sealed data, 8 h/day"

A typical business-hours / batch movement workload (data flows during the day, idle at night).

| | datarail (serverless mover) | Kafka (always-on cluster) |
|---|---|---|
| Footprint | 1.3 vCPU + 8 MB, scale-to-zero | ~3 brokers × (2 vCPU + 4 GB), 24/7 |
| Hours billed/month | ~8 h × 30 = 240 h | 24 h × 30 = 720 h × 3 brokers |
| Idle cost | **$0** (vanishes when not moving) | full cluster, even at night |
| Rough $/month | **~$10–15** (Fargate ~1.3 vCPU × 240 h, or Lambda for bursts) | **~$90 self-managed minimum → $200–500+ managed** |
| **Ratio** | — | **≈ 7–40× cheaper for datarail** |

The two multipliers, both grounded in measured facts:
1. **Footprint** (measured): datarail fits ~100× smaller instances → lower $/hour.
2. **Scale-to-zero** (architectural): datarail's idle is **0**; a Kafka cluster bills 24/7 because it *stores*
   the data. For bursty/periodic movement — most real movement workloads — this alone is the dominant cost gap.

For a workload that moves data **1 h/day**, the idle multiplier widens the gap toward **50–100×**; for a steady
24/7 firehose it narrows toward the footprint ratio (still large). The burstier the workload, the more datarail
wins.

## The deeper point: storage is abundant, RAM is scarce — datarail decouples durability from RAM

An earlier draft of this doc drew a caveat — *"this is efficiency for moving, not storing; if you need
durability, Kafka is the right tool."* **That caveat was wrong, and it conceded too much to Kafka.** The
correction is the sharpest part of the thesis:

**Durability means writing bytes to storage. Storage (disk / object store) is cheap and abundant. RAM is
expensive and scarce.** Kafka achieves durability+performance by keeping its working set in the **OS page
cache** — i.e. it spends the **scarce** resource (RAM) to make the durable log fast. Its RAM bill **grows with
retention and throughput** (bigger working set → more brokers → more RAM).

datarail does the opposite by design (SPEC-10 durable store): the source seals a cofre and **writes it to
commodity object storage / disk** (S3 / GCS / R2 / MinIO — cheap, abundant, provider-blind: the store sees only
ciphertext), keeping **only a tiny index in RAM**. So **durability scales on the cheap/abundant axis (disk),
and RAM stays FLAT regardless of how much you store.**

**Measured proof** (`datarail-store-bench`, Linux real `/proc`): sealing + durably storing **30,000 sealed
cofres = ~1.9 GB of provider-blind data on disk** held the process at **RSS ≈ 2 MB the whole time** — flat while
disk grew to gigabytes. Store 10× more → disk grows 10×, **RAM does not move.** Kafka cannot do this: more
durable data in its hot set means more page-cache RAM.

> **Cold-verify note (we checked our own number — it looked too good):** the ~2 MB flat figure is the
> **write/store** path (the source seals → writes to disk → forgets; it accumulates nothing in RAM). The
> *drain/read* side of the current **demo** object-store substrate keeps per-message bookkeeping that grows with
> messages processed — a noted demo simplification (a production seq-cursor is O(1)), not a property of the
> architecture. The fair **full send+receive** footprint is the **7 MB** from Run 10 (loopback substrate), not
> 2 MB. We report 7 MB as the pipeline number and ~2 MB as the proven *durability-decouples-from-RAM* number —
> no rounding down. (An earlier draft said "1 MB"; that was an imprecise macOS `ps` reading — the real Linux
> figure is ~2 MB.)

> **So datarail is NOT "a mover that can't replace Kafka's durability." It is durability done on the abundant
> resource (cheap object storage) instead of the scarce one (RAM) — which is why it competes with Kafka on
> durable delivery AT ~100× less RAM, and the advantage WIDENS as data volume grows.** Kafka couples
> durability to RAM; datarail decouples them. That is the architecture-level moat.

(Honest residual: object-store durability adds latency vs Kafka's RAM-hot reads — fine for store-and-forward /
temporal-decoupling / async movement, which is the use case; not for sub-ms hot replay of a huge working set.
And when both endpoints are online, datarail uses **no store at all** — direct sealed shmem/QUIC/TCP.)

## Honest boundaries (what stays true)

- **CPU is ~parity** — the win is RAM + idle/scale-to-zero + durability-on-cheap-storage, not compute. Don't
  claim a CPU advantage.
- **Prices are illustrative.** The measured *resource ratios* are solid; the *dollar figures* depend on
  provider, region, and sizing. The ratio is robust because it is footprint-driven.

## The one-line thesis (measured, honest)

> **datarail moves AND durably stores the same data as Kafka using ~1/124th the RAM under load and ~1/92nd at
> rest, at comparable CPU, scaling to $0 when idle — because it spends the abundant resources (disk, object
> storage) where Kafka spends the scarce one (RAM). Same throughput class, full durability; a footprint and cost
> from another category, and the gap WIDENS with data volume.** That is the disruption: not faster — it moves
> the cost off the scarce axis (RAM) onto the abundant one (storage), which Kafka architecturally cannot do.
