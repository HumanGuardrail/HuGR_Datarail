# 00 — ROADMAP

> Derived from the **frozen SPEC** (`design/SPEC-FREEZE.md` · sha256 `5f88095…6d89` @ `c0026da`). Phased,
> dependency-ordered; **each phase discharges its named proof obligations before the next opens.** No phase
> closes with a red gate.

## Phases

| Phase | Builds | Crates | Proof obligation (must be green to close) |
|---|---|---|---|
| **P0 — Contracts (MF-3)** | freeze the cross-crate IDL + compilable stubs | `datarail-core` (types + traits) | seam compiles; contract-conformance skeleton; FOOTER-FREEZE content-hash |
| **P1 — Spine** | cofre encode/decode + seal/verify | `datarail-cofre`, `datarail-crypto` | **AC-2** (mutate-every-byte → reject), **AC-3** (forge reject), encode determinism golden |
| **P2 — Proof & once** | manifest + dedup/checkpoint | `datarail-manifest`, `datarail-once` | **AC-5** (inclusion-proof differential), **AC-4** (DST, N seeds, fault injection) |
| **P3 — Rail** | ephemeral lifecycle + substrates (shmem/QUIC/S3) | `datarail-rail`, `datarail-substrate-*` | **AC-1** (metamorphic opacity), **AC-6** (suite×3 substrates), **AC-8** (WAN + kill-resume), **GATE-FEATHER** |
| **P4 — Terminals** | onboarding/offloading + connectors | `datarail-terminal`, `datarail-connectors` | **AC-9** (contract proptest), **AC-2/3** end-to-end |
| **P5 — Surface (MF-4)** | rail.toml + `datarail` CLI | `datarail-spec`, `datarail-cli` | **first vertical slice**: a real route, shmem-hop + QUIC-hop, sealed + manifest + effectively-once + dead-letter |
| **P6 — Prove (MF-6)** | benchmark + final audit | `datarail-acceptance`, `bench/` | **AC-10** fairness rig · **GATE-WARP** bound · final 6-lens audit over the CODE · deliver |

## Dependency order

`P0 → P1 → { P2 ∥ P3 } → P4 → P5 → P6`. The CONSTITUTION crate DAG (no terminal↔terminal, no
substrate↔substrate edge) lets P2/P3 and the within-phase crates **fan out in parallel** (Kage-Bunshin),
zero merge conflicts.

## Gates carried throughout

`forbid(unsafe_code)` · `clippy` deny(pedantic) · `rustfmt` · LOC caps · `GATE-WARP`/`GATE-LATENCY`/`GATE-FEATHER`
in CI (Charter L1). No WP merges without its AC's proof method green (Charter C4 / `11`).

## Standing PENDING (escalate at the phase, never fake)

- **`GATE-WARP` bound** — set on representative VAES HW before MF-4/MF-6 (P1/P6).
- **AC-10 external engines** — needed in P6.
- **MF-0 trio ratification** — owner, anytime (does not block P0–P5).
