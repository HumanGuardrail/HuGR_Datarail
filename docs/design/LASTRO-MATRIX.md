# LASTRO-MATRIX — evidence backing for every load-bearing claim

> The owner asked: *"are these benchmarks documented with reproducible backing (lastro)?"* — and caught one
> (cold-start) that was a single noisy sample with no committed repro. This matrix is the systematic answer: a
> read-only evidence audit (2026-06-26) of every load-bearing number, cross-checked against committed harnesses
> (`bench/`, `.github/workflows/`) and `cargo test` gates. **Rule of the house: a claim is only `PROVEN` if a
> committed test/harness reproduces it from a real run (n>1 or exhaustive). Everything else is labelled down.**

## Verdict legend
- **BACKED** — committed repro (a `cargo test` gate or a CI harness) + a real/exhaustive run + an honest label.
- **SINGLE-SAMPLE** — real harness exists, but the quoted number is n=1 / one-seed / no variance → DIRECTIONAL.
- **NO-REPRO** — no committed harness/test reproduces it.
- **STALE** — superseded/self-corrected by a newer number.
- **OVERCLAIMED** — label was stronger than the evidence (now corrected).

## The matrix

| Claim | Source | Repro artifact (committed) | Rigor | Verdict |
|---|---|---|---|---|
| effectively-once 0-loss/0-dup | DOD-01 AC-4 | `datarail-once/tests/ac4_dst.rs` | 1000 seeds | **BACKED** |
| erasure RS(4,2) every double-loss @ 1.5× | ledger #3 | `datarail-erasure` gates (exhaustive + GF axioms) | exhaustive | **BACKED** |
| keyrouter even-spread / 0-reshuffle / churn ~1/N | ledger #5,#6 | `datarail-keyrouter` gates | 60–80k keys | **BACKED** |
| replay flat-RAM (40 MiB, bounded buffer) | ledger #2 | `datarail-replaylog` gate | asserted bound | **BACKED** |
| WAL durability-decoupled-from-RAM (60k cycle, power-loss, torn-tail) | ledger #12 | `datarail-substrate-wal/tests/durable.rs` | asserted | **BACKED** |
| tiered offload + cross-tier replay (local+remote) | ledger #2 | `datarail-tieredlog` + `datarail-system` gates | asserted | **BACKED** |
| FaspLink reliability/window/Karn + S4 audit fixes | AUDIT-FASP | `datarail-rail` S1/S2/S4 gates | asserted | **BACKED** |
| GCM-256 wire-compat KAT | BENCH-01 | `datarail-crypto` KAT | KAT | **BACKED** |
| all parsers no-panic-on-garbage + roundtrip | — | `datarail-fuzz` (7 gates) | ~millions, deterministic | **BACKED** |
| restart-resume / shard-loss / remote-tier (system) | INTEGRATION/capstone | `datarail-system/tests/e2e.rs` | asserted | **BACKED** |
| **~72× less RAM @ equal fsync durability, ~92× idle** | OMB Run 12 | `bench/omb/run-omb.sh` + `omb-benchmark.yml` (`kafka_fsync`) | **n=3, σ1.6** | **BACKED** (the headline) |
| FASP holds flat / TCP collapses past 15% loss (real netem) | WAN-RESULTS | `bench/wan/netem-fasp-vs-tcp.sh` + `wan-bench.yml` | n=2, labelled DIRECTIONAL, no headline × | **BACKED** (honest n=2) |
| FASP "16.3/29.6/42.1×" *specific* multipliers | DOD-01, scorecard | `fasp_vs_lossbased.rs` asserts only **>1.5× + widening** | one seed | **SINGLE-SAMPLE** → flagged |
| throughput "engine WINS ~1.2× / 1.9 GB/s" | OMB Run 7–8 | `engine_bench.rs`/`loadgen.rs` + `loadgen.yml` | n=1, swings 2–3× | **SINGLE-SAMPLE** → flagged DIRECTIONAL |
| **cold-start 8 ms / 5.5 s / ~690×** | COLD-START-RESULTS | `bench/cold-start/cold-start.sh` | n=1 sample; re-run 141 ms/22 s | **SINGLE-SAMPLE** → `PENDING-RIGOR` (regime holds) |
| GATE-WARP "LITERAL PASS 1.43 GB/s" | BENCH-01, DOD-01 | `loadgen.yml --vaes` (harness only; cited job **deleted**) | n=1, no committed artifact | **OVERCLAIMED** → downgraded to PENDING-RIGOR |
| Run 11 "~67×, 13 MB vs 877 MB" | OMB Run 11 | OMB harness | n=1 (self-stated) | **STALE** → superseded by Run 12 (n=3) |
| "124× / 7 MB" loaded RAM | OMB Run 10 | OMB harness | single light-load sample | **STALE** → self-corrected to ~40× then n=3 ~72× |
| "3.4× faster" / 2-core "25×" | DOD-01 AC-10 | none (self-configured Kafka) | retracted | **STALE** → retracted in-doc |

## Flagged this pass (2026-06-26) — the cold-start-class soft spots, now corrected
1. **HTML cold-start** showed `8 ms` with a green `MEASURED` badge while its source doc was already PENDING-RIGOR → **re-badged DIRECTIONAL/PENDING in the HTML** (3 spots).
2. **GATE-WARP** labelled PROVEN on an n=1 **deleted** Northflank job → **downgraded to PENDING-RIGOR** (harness exists; needs a committed CI VAES artifact to bank).
3. **FASP 16–42× exact multipliers** → flagged single-seed (the gate proves the *shape*, not the range).
4. **Throughput "WINS ~1.2×"** → flagged n=1/no-variance → DIRECTIONAL (parity-class is the robust claim).
5. **EFFICIENCY-TCO** led with the n=1 Run 11 ~67× → **re-pointed to the n=3 Run 12 ~72×**.

## Standing rule
Before any number is written as PROVEN/MEASURED in a doc or the HTML, it must trace to a committed `cargo test`
gate or a committed `bench/` + workflow, run at n>1 (or exhaustively). The one durable benchmark headline that
meets this fully is **~72× RAM (Run 12, n=3)**; FASP-WAN and the directional throughput/cold-start figures are
honestly labelled below PROVEN until their n≥3 / real-WAN / VAES-artifact work lands.
