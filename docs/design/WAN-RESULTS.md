# WAN-RESULTS — FASP vs kernel TCP under REAL tc-netem loss (S3)

> **What changed.** The FASP delay-based controller is no longer only proven against a textbook AIMD in an
> in-process simulation — it now moves real bytes over a real `UdpSocket` (`reliable_udp::FaspLink`), measured
> against real kernel TCP on the same loopback, under **real `tc netem` kernel packet loss + delay**. See the
> build in `FASP-UDP-TRANSPORT.md` (S1–S3). **Confidence: MEASURED-ON-REAL-SOCKET, n=1→3, S4 audit PENDING
> before any PROVEN label.**

## The sweep (ubuntu-latest, 25 ms one-way delay ≈ 50 ms RTT, 5 s/transfer; n=2 of 3 — the 3rd run hung, see below)

| loss | FASP MB/s (run A / run B) | kernel TCP MB/s (A / B) | ratio (range) |
|---|---|---|---|
| 0%  | 18.9 / 16.9 | 4.00 / 4.01 | ~4× |
| 5%  | 18.3 / 18.3 | 6.93 / 4.38 | 2.6–4× |
| 15% | 16.4 / 17.4 | 0.14 / 1.25 | 14–119× |
| 30% | 12.6 / 13.3 | 0.02 / 0.33 | 40–776× |

**Two things the second run revealed (honesty):**
- **FASP goodput is rock-solid run-to-run** — 18.9/16.9, 18.3/18.3, 16.4/17.4, 12.6/13.3. The flatness under loss
  is *robust*, not a lucky draw.
- **The TCP collapse magnitude is wildly variable** — at 15% loss TCP gave 0.14 one run and 1.25 the next (≈10×);
  at 30%, 0.02 vs 0.33 (≈16×). Random loss + loss-based collapse is chaotic. **So the *ratio* is NOT a stable
  number** (the 15% ratio swings 14×–119×, the 30% swings 40×–776×). The robust, defensible claim is the SHAPE:
  *FASP holds ~13–19 MB/s flat; single-stream TCP collapses to under ~1.3 MB/s past 15% loss (often under 0.3) —
  a one-to-two-order-of-magnitude collapse, exact value run-dependent.* We do NOT headline a single multiplier.
- **The 3rd run hung (>10 min) and was cancelled** — a real bench limitation: `bench_tcp` drains its post-shutdown
  send buffer under heavy loss with no timeout, so an unlucky netem draw can stall the TCP retransmit drain for
  minutes. The finding stands on n=2; firming to n≥3 needs a per-transfer timeout in the harness (future).

## The honest reading (the SHAPE, not the gaudy ratio)
- **FASP goodput is nearly FLAT as loss climbs** — ~13–19 MB/s across 0→30% loss, tight across both runs. The
  delay-based window ignores loss (recovers by retransmission, Karn-gated RTT), so packet loss barely dents it.
  **This is the moat, and it now holds on a real socket with real kernel loss, not just in a simulation.**
- **Kernel single-stream TCP COLLAPSES under loss** — to ~0.1–1.3 MB/s at 15% and ~0.02–0.33 MB/s at 30% (the
  magnitude varies run-to-run, but the collapse itself is consistent). This is the textbook loss-based-AIMD
  collapse, and it's exactly what Kafka's MirrorMaker (single-stream TCP) suffers on a lossy WAN.

## What is NOT claimed / the caveats (no spin)
- **Absolute FASP throughput is modest (~19 MB/s)** — our mover is a single-threaded, user-space, one-syscall-
  per-1200-byte-datagram loop. It is *resilient*, not *fast*. A clean-link, in-process loopback (no netem) had
  kernel TCP at **6+ GB/s** vs FASP's ~50 MB/s — on a fast clean link TCP wins by orders of magnitude. The FASP
  win is **specifically loss/WAN resilience**, nothing else.
- **The 0%/5% TCP numbers are anomalously low** (4–7 MB/s for clean 50 ms-RTT TCP is far below what a warmed,
  window-scaled stream does). It smells like single-stream cold-start + netem-delay interaction inside a 5 s
  window. **So we do NOT lead with the 0% "FASP wins 4.7×" — that's likely a short-window artifact.** The robust,
  well-understood finding is the **loss-driven TCP collapse at 15–30%** and FASP's flatness.
- **n=1 (this table); the tiny TCP values are noisy.** An n=3 confirmation is in flight; the median + range will
  replace this once collected.
- **Loopback `tc netem`, not a real geographic WAN.** netem injects genuine kernel loss/delay (a real qdisc, not
  a simulation), but it is not a transcontinental path. A real multi-hop field run is still future work.
- **S4 PENDING:** `FaspLink` has not yet had its adversarial audit. Reliable transport is bug-prone (the WAL had
  5 data-loss windows; the S2 gate already caught a real Karn-algorithm bug here). **No PROVEN label until the
  audit passes.**

## Bottom line
The FASP thesis — *delay-based congestion control holds goodput where loss-based TCP collapses* — is now
demonstrated on a **real socket under real kernel packet loss**, not only in simulation. The honest, defensible
statement is about the **shape** (FASP flat, TCP collapses past ~15% loss), with the exact multiplier left as a
range pending n=3, and the whole claim gated behind the S4 audit. The absolute throughput of our mover is modest
and we say so; the moat is resilience, and it is real.
