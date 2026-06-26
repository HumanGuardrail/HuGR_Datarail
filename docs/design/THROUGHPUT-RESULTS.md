# THROUGHPUT-RESULTS — sealed engine ceiling, n=3 (corrects the n=1 overclaim)

> Closing a PENDING-RIGOR item: the throughput "engine ~1900 MB/s @1KB, WINS ~1.2× vs Kafka" was an n=1 number.
> Re-run on the Turbo runner **3×** (`loadgen.yml`, `datarail-loadgen`, 20 s, topics 16/24/32) — and the n=3
> result is TIGHT, but **lower than the single-run claim**.

## datarail sealed-engine throughput (n=3, Turbo, GCM-SIV, no sockets)

| msg size | best topics | run A / B / C (MB/s) | median | vs the old n=1 claim |
|---|---|---|---|---|
| 1 KB  | 32 | 1399 / 1432 / 1400 | **~1400** | claim was **1900** → corrected down (n=1 was favorable) |
| 4 KB  | 32 | 1682 / 1603 / 1665 | **~1665** | — |
| 16 KB | 24 | 1711 / 1717 / 1687 | **~1705** | — |

**Honest verdict — it's a TIE, not a win.** datarail's sealed engine is **~1.4 GB/s @1KB** (n=3, σ tiny) vs
Kafka's **~1.6 GB/s @1KB plaintext** (OMB) ⇒ **~0.87× — parity-class, slightly BEHIND at 1 KB**, and ahead only
at ≥4 KB (~1.7 GB/s). The "sealed engine WINS ~1.2×" claim came from a single favorable run; **n=3 corrects it to
the honest framing that was the robust one all along: datarail moves data in Kafka's class, sealed — a tie, not
faster.** (This matches the project's standing position: the moat is efficiency/leanness, not raw throughput.)

## VAES path (n=1, vaes=true on Turbo — Turbo HAS VAES)
The `vaes` build ran (no illegal-instruction) and was ~1.1× the GCM-SIV pipeline (1554 vs 1400 MB/s @1KB/32t),
confirming **the Turbo runner has VAES** — so GATE-WARP's per-core AEAD number can be measured here without
Northflank. **But `loadgen` is the full pipeline (~48 MB/s/core aggregate), NOT the per-core AEAD ceiling
GATE-WARP specs (1 GiB/s/core)** — so this does NOT bank GATE-WARP. The right bench is `engine_bench --vaes`
(single-thread AEAD); GATE-WARP stays PENDING-RIGOR until that committed run lands.

## Reproduce
`gh workflow run loadgen.yml -f runner=Turbo -f vaes=false` (×3 for variance). Per-core AEAD / GATE-WARP:
`engine_bench --vaes` on a VAES runner (next).
