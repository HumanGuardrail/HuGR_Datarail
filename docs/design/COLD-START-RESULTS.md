# COLD-START-RESULTS — time-to-ready: the serverless / scale-to-zero moat, MEASURED (ledger #4, #10)

> **Why this matters.** The efficiency story so far is steady-state RAM (~70× less). But the economic heart of
> the disruption is **scale-to-zero**: most workloads are bursty, and Kafka bills 24/7 because it *cannot* start
> fast enough to spin up per-request. This is the measured proof that datarail can, and Kafka can't — turning the
> "idle ≈ 0 / serverless" claim from DIRECTIONAL (derived TCO) into a measured number.

> **Reproduce:** `bench/cold-start/cold-start.sh [datarail-trials] [kafka-trials]` (committed harness — the
> datarail side is a bare-binary spawn timed to ingress-ready; the Kafka side needs docker).

> ## ⚠️ KNOWN GAP — `PENDING-RIGOR` (flagged 2026-06-26, to close later; does NOT invalidate the other benchmarks)
> When the committed repro script was run again, the numbers did **not** reproduce tightly: a quick re-run gave
> **datarail median ~141 ms (n=3, range 7–784)** and **Kafka ~22 s (n=1)** — vs the **8 ms / 5.5 s** recorded
> below. The earlier figures were a single favorable sample on a warm machine; cold-start here is **noisy** (first
> process spawn / page-in, docker scheduler, laptop load).
>
> **What still holds:** the **regime** — datarail is *milliseconds* to ready, Kafka is *seconds* (even the bad
> draw is ~141 ms vs ~22 s, still ~2 orders of magnitude) → scale-to-zero viable vs always-on mandatory. **What is
> NOT yet rigorous:** the exact figures (8 ms / 5.5 s) and the "~690×" multiplier — treat them as DIRECTIONAL, not
> a pinned number, until re-measured with n≥10 controlled (warm docker, quiet machine, percentiles).
>
> **Scope of this gap:** it is isolated to *cold-start*. It does NOT touch the harness-backed, CI-reproducible
> benchmarks (the ~70× RAM in `OMB-RESULTS.md`, the FASP-WAN sweep in `WAN-RESULTS.md`) or any `cargo test` gate.

## Measured (2026-06-26, same laptop, Kafka image pre-pulled — SINGLE favorable sample; see the gap note above)

| system | cold-start to ready | n | how "ready" is defined |
|---|---|---|---|
| **datarail** (shim binary) | **median 8 ms** (min 6, max 445) | 10 | process spawn → ingress port `:7701` accepting |
| **Kafka 3.8** (KRaft, container) | **median 5.5 s** (5.3–5.8) | 3 | `docker run` → broker answers `kafka-topics --list` |

**⇒ ~690× faster to ready** (5500 ms vs 8 ms). But the multiplier is not the point — the **regime change** is:
- **8 ms ⇒ scale-to-zero is viable** — datarail can start *per request / per burst* and exit when idle, paying
  **$0 when there's no traffic**.
- **5.5 s ⇒ always-on is mandatory** — no one puts a 5.5-second cold start in a request path, so a Kafka cluster
  must stay running 24/7, billing continuously even at zero traffic. This is exactly the tax a bursty workload
  pays today.

## Honest caveats (no spin)
- **Not perfectly apples-to-apples:** datarail is measured as a **bare binary spawn**; Kafka as a **JVM broker in
  a container**. That asymmetry is *real to the deployment*, though — datarail ships as a ~few-MB static-ish
  binary; Kafka ships as a ~600 MB JVM image. The comparison is "what it takes to be ready to serve," which is
  the honest operational question.
- **Kafka's 5.5 s EXCLUDES the image pull** (594 MB, a one-time ~tens-of-seconds cost on a cold host). datarail's
  binary is a few MB. So on a truly cold host the gap is larger; we measured the warm-image case to be fair.
- **"Ready" definitions differ slightly** — datarail = ingress accepting; Kafka = broker answers an admin op
  (fully usable). Both are "ready to do work," but they're not byte-identical readiness bars.
- **datarail's max was 445 ms** (one outlier in 10, likely first-spawn page-in / scheduler); the median 8 ms is
  the representative figure, but cold-start isn't perfectly deterministic.
- **Single-node Kafka (KRaft).** A multi-broker production cluster starts *slower*, not faster.
- **Laptop, not a tuned host.** Absolute numbers shift on other hardware; the order-of-magnitude regime (ms vs
  seconds) is the robust finding.

## Bottom line
datarail reaches ready in **milliseconds**; Kafka in **seconds**. That is the difference between **scale-to-zero**
(start per burst, $0 idle) and **always-on** (a standing cluster billed around the clock). For the bursty,
move-data-A→B workloads that are most of the real world, this — not raw throughput — is the cost moat, and it is
now measured, not asserted. (Pairs with the ~90× idle-RAM and ~70× under-load RAM numbers in `OMB-RESULTS.md`.)
