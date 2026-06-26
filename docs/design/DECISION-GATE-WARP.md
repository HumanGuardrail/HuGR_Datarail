# DECISION — GATE-WARP disposition (tech-lead, 2026-06-26, owner-authorized)

## The situation (measured, not asserted)
`GATE-WARP` (11-gates-proof-methods.md) requires **per-core sealed throughput ≥ 1 GiB/s/core (1024 MB/s)** of the
full sealed-payload datapath. Measured properly — `engine_bench` at `threads=1`, full datapath (AEAD-seal +
Ed25519 sign + X25519 per-cofre wrap + verify/open/admit/commit), on a **VAES** runner via committed CI
(`loadgen.yml`):

- **Per-core: ~101 MB/s @1KB, ~134 MB/s @16KB** (VAES) — **~7.6–10× BELOW the target. The gate FAILS.**
- **Batch-amortization curve (128→8192): FLAT at 125–134 MB/s** → the bottleneck is **NOT** the per-cofre
  asymmetric crypto (it amortizes; the curve would rise). It is **per-RECORD work that doesn't amortize**: the
  effectively-once **dedup-index admit + commit + per-record framing** on the offload side.

## Why the metric was wrong (the assumption it was built on is disproven)
The 1 GiB/s/core target's stated rationale was *"AES-256-GCM-SIV on VAES ≈ 2–4 GB/s/core and the per-cofre wrap
amortizes over a batch, so ~1 GiB/s/core end-to-end is conservative-but-real."* Measurement **disproved the
premise**: the AEAD/crypto is **not** the limiter at realistic batches (it amortizes to negligible per-record);
the **per-record dedup+commit** is. So the "conservative" 1 GiB/s/core was set against the wrong bottleneck.

## Is the gate's INTENT met? — YES
GATE-WARP exists to ensure **datarail's sealing/crypto is not a throughput bottleneck for the product.** Checked:
- **Aggregate** sealed ceiling: **~2.3 GB/s on 32 cores** (engine_bench, CI) — healthy.
- **Real workloads**: the OMB benchmarks deliver at their target rate (51 MB/s) with the engine at a tiny fraction
  of its ceiling. datarail is **not throughput-limited at any tested workload**.
- The product's moat is **efficiency** (~72× RAM) and **resilience**, not raw per-core crypto throughput — which
  was never a product claim (the throughput position is an honest TIE).

## DECISION (and what it is NOT)
**I do NOT move the goalpost to "pass."** GATE-WARP stays **RED** against its original metric — that is the honest
record, and it is in BENCH-01 / DOD-01 / LASTRO-MATRIX. The rigor compact forbids loosening a gate to pass; this
decision does not do that. Instead, logged tech-lead findings + direction:

1. **The original per-core metric (1 GiB/s/core) is over-specified** — built on a disproven AEAD-dominated
   assumption. It does not reflect what bounds the real datapath (per-record dedup/commit).
2. **The gate's intent is MET** at the aggregate/product level (not throughput-limited). So the RED has **nil
   product impact** — it is a metric/spec mismatch, not a product defect.
3. **Two honest paths (owner's call, not auto-applied):**
   - **(a) Re-scope** GATE-WARP to its intent — e.g. *"aggregate sealed throughput ≥ K× the max supported
     workload rate"* (currently ~45× over the 51 MB/s OMB rate) — and record the measured per-core ~134 MB/s as a
     known characteristic. This would PASS honestly because it measures the intent, not a disproven proxy.
   - **(b) Keep RED** and pursue the **per-record optimization** (the dedup-index admit + commit hot path) — real
     perf engineering on `datarail-once`/`datarail-terminal`, correctness-critical, **optional** given (2).
4. **Recommendation:** (a) — re-scope to the measured intent, because the per-core proxy is disproven and the
   per-record optimization is low-ROI (the product is not throughput-limited). But this is a **spec change** to a
   tech-lead gate; flagged for owner ratification rather than silently applied. Until ratified, **GATE-WARP stays
   RED** and the per-record optimization is **tracked optional future work** with the measured target.

## Status
RED (honest, against the original metric) · intent MET · re-scope recommended, **pending owner ratification** ·
per-record optimization = optional future work. No goalpost moved; the truth is measured and on the record.
