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
| effectively-once 0-loss/0-dup **within a process run** | DOD-01 AC-4 | `datarail-once/tests/ac4_dst.rs` | 1000 seeds | **BACKED (in-process)** — NOT across crashes: the dedup index is in-memory + not wired into the broker (`CORE-AUDIT.md` D-3, tracked); the broker is at-least-once across restarts |
| erasure RS(4,2) every double-loss @ 1.5× | ledger #3 | `datarail-erasure` gates (exhaustive + GF axioms) | exhaustive | **BACKED** |
| keyrouter even-spread / 0-reshuffle / churn ~1/N | ledger #5,#6 | `datarail-keyrouter` gates | 60–80k keys | **BACKED** |
| replay flat-RAM (40 MiB, bounded buffer) | ledger #2 | `datarail-replaylog` gate | asserted bound | **BACKED** |
| WAL durability-decoupled-from-RAM (60k cycle, power-loss, torn-tail) | ledger #12 | `datarail-substrate-wal/tests/durable.rs` | asserted | **BACKED** |
| tiered offload + cross-tier replay (local+remote) | ledger #2 | `datarail-tieredlog` + `datarail-system` gates | asserted | **BACKED** |
| FaspLink reliability/window/Karn + S4 audit fixes | AUDIT-FASP | `datarail-rail` S1/S2/S4 gates | asserted | **BACKED** |
| GCM-256 wire-compat KAT | BENCH-01 | `datarail-crypto` KAT | KAT | **BACKED** |
| all parsers no-panic-on-garbage + roundtrip | — | `datarail-fuzz` (7 gates) | ~millions, deterministic | **BACKED** |
| restart-resume / shard-loss / remote-tier (system) | INTEGRATION/capstone | `datarail-system/tests/e2e.rs` | asserted | **BACKED** |
| **no-loss of acked records across crash** | system + broker | `datarail-system/tests/chaos.rs` (process-kill resume) + broker durability-before-ack (`741b803`) | seeded 2000; fsync-before-ack | **BACKED** — the chaos test models process-kill (page-cache survives); power-loss safety comes from the broker fsync-before-ack fix (`CORE-AUDIT.md` D-1) |
| **v1 product flow: HTTP API → sealed rail → Postgres** | connectors | `datarail run --source-http --sink-postgres` + `tests/postgres_live.rs` | live docker PG 16 (trust+md5), e2e 3 sealed rows | **BACKED** |
| **Kafka ingest: unmodified producer → sealed → Postgres** | datarail-kafka | `kafka-ingest.yml` (kcat + PG service) | real librdkafka, e2e 3 sealed rows in CI | **BACKED** |
| **exactly-once into Postgres across crashes (Tier A)** | TxnSink | `PostgresSink::commit_at` + `tests/postgres_live.rs` | live PG: batch + 3 replays → 4 rows, not 7 | **BACKED** — watermark stored transactionally in the DB; replay is a committed no-op |
| **persistent dedup survives restart (Tier C)** | FileOnce | `datarail-once/src/persist.rs` | crash-tested: dedup + above-watermark keys + torn-tail | **BACKED** — restart no longer floods dups; bounded ≤1 (file/webhook) |
| **~72× less RAM @ equal fsync durability, ~92× idle** | OMB Run 12 | `bench/omb/run-omb.sh` + `omb-benchmark.yml` (`kafka_fsync`) | **n=3, σ1.6** | **BACKED** (the headline) |
| FASP holds flat / TCP collapses past 15% loss (real netem) | WAN-RESULTS | `bench/wan/netem-fasp-vs-tcp.sh` + `wan-bench.yml` | n=2, labelled DIRECTIONAL, no headline × | **BACKED** (honest n=2) |
| FASP "16.3/29.6/42.1×" *specific* multipliers | DOD-01, scorecard | `fasp_vs_lossbased.rs` asserts only **>1.5× + widening** | one seed | **SINGLE-SAMPLE** → flagged |
| throughput: sealed engine **~1.4 GB/s @1KB (TIE, ~0.87× Kafka)** | THROUGHPUT-RESULTS | `loadgen.yml` ×3 | **n=3, tight** | **BACKED** — n=3 corrected the n=1 "1900/WINS" down to a TIE |
| **cold-start: datarail p50 2 ms vs Kafka p50 5.1 s** | COLD-START-RESULTS | `bench/cold-start/cold-start.sh` + `cold-start.yml` | **n=30/5 controlled CI, datarail spread=0** | **BACKED** (gap closed 2026-06-26) |
| GATE-WARP (re-scoped to intent, owner-ratified) | DECISION-GATE-WARP | `loadgen.yml` (VAES, committed) | aggregate ~2.3 GB/s ÷ 51 MB/s ≈ 45× (≥10× bar) | **BACKED/PASS** — per-core ~134 MB/s recorded as known characteristic (per-record-bound); original per-core metric retired (disproven premise) |
| Run 11 "~67×, 13 MB vs 877 MB" | OMB Run 11 | OMB harness | n=1 (self-stated) | **STALE** → superseded by Run 12 (n=3) |
| "124× / 7 MB" loaded RAM | OMB Run 10 | OMB harness | single light-load sample | **STALE** → self-corrected to ~40× then n=3 ~72× |
| "3.4× faster" / 2-core "25×" | DOD-01 AC-10 | none (self-configured Kafka) | retracted | **STALE** → retracted in-doc |

| brutal core audit (seal + once/durability) — findings + dispositions | CORE-AUDIT | `CORE-AUDIT.md` | 2 auditors, PoCs | **BACKED** — provider-blind HOLDS (wire adversary); DRBG clone-reuse FIXED; durability-before-ack FIXED; cross-crash dedup TRACKED |

## Flagged this pass (2026-06-26) — the cold-start-class soft spots, now corrected
1. **HTML cold-start** showed `8 ms / MEASURED` contradicting its PENDING-RIGOR source → first re-badged DIRECTIONAL, then **CLOSED**: re-measured on controlled CI (`cold-start.yml`, n=30/5) → datarail p50 **2 ms** (zero spread) vs Kafka **5.1 s**; HTML now shows the BACKED number.
2. **GATE-WARP** labelled PROVEN on an n=1 **deleted** Northflank job → **downgraded to PENDING-RIGOR** (harness exists; needs a committed CI VAES artifact to bank).
3. **FASP 16–42× exact multipliers** → flagged single-seed (the gate proves the *shape*, not the range).
4. **Throughput "WINS ~1.2×"** → flagged n=1/no-variance → DIRECTIONAL (parity-class is the robust claim).
5. **EFFICIENCY-TCO** led with the n=1 Run 11 ~67× → **re-pointed to the n=3 Run 12 ~72×**.

## Standing rule
Before any number is written as PROVEN/MEASURED in a doc or the HTML, it must trace to a committed `cargo test`
gate or a committed `bench/` + workflow, run at n>1 (or exhaustively). The one durable benchmark headline that
meets this fully now are **~72× RAM (Run 12, n=3)** and **cold-start (2 ms vs 5.1 s, n=30/5 controlled CI)**;
FASP-WAN (n=2) and the directional throughput / GATE-WARP figures are honestly labelled below PROVEN until their
n≥3 / real-WAN / VAES-artifact work lands.
