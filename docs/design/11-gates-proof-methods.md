# 11 — Gates & Proof Methods

> **MF-2 SPEC. Status: DRAFT.** The verification design — the Craft Charter's teeth made concrete. No WP
> merges without its AC's proof method green; every gate is encoded in CI, not prose.

## The performance gates (Charter L1 — gate the BOUND, not flatness)

| Gate | Bound | CI mechanism |
|---|---|---|
| `GATE-WARP` | per-core sealed throughput ≥ **X** (X = **PENDING** representative-VAES re-bench) | criterion bench → assert ≥ bound; regression past bound fails build |
| `GATE-LATENCY` | p50/p99 per hop ≤ bound (shmem µs · QUIC ms) | bench → assert p99 |
| `GATE-FEATHER` | **idle footprint ≈ 0** (no standing process; scale-to-zero) + active RSS ≪ a mesh sidecar | measure idle (must be ~0) + active RSS bound |

## Proof method per Acceptance Criterion (the verification map)

| AC | Proof method |
|---|---|
| AC-1 opaque cargo | **metamorphic**: rail run with payload bytes zeroed behaves byte-identically |
| AC-2 tamper-reject | **proptest**: mutate every byte position of header⊗payload → 100% rejected, dead-lettered |
| AC-3 forge-reject | **differential** vs a known-good signer; wrong-key cofre rejected |
| AC-4 effectively-once | **DST** (deterministic simulation), N seeds, fault injection (drop/reorder/dup/kill) → 0 loss, 0 dup |
| AC-5 delivery proof | **differential** vs an independent verifier of `{inclusion proof, STH, TSA, ack}` |
| AC-6 substrate parity | **one suite × 3 substrates** (shmem/QUIC/S3) → identical results |
| AC-7 featherweight | `GATE-FEATHER` measurement |
| AC-8 WAN/resume | **benchmark** vs TCP baseline on a lossy/high-RTT link + **kill-resume** test (bao) |
| AC-9 contract enforcement | **proptest** over conforming/violating cases, both terminals |
| AC-10 fairness benchmark | the rig vs **real engines**, byte-equal **before** timing, 17-case anti-cheat (external engines = **PENDING** infra) |

## Determinism note

Unlike Keepr, cross-machine byte-determinism is **not** load-bearing (the seal is verified by signature, not
re-derived). So there is no determinism golden gate; integrity is the signature, not reproducible bytes.

## The honesty taxonomy (every claim labeled)

`PROVEN` (self-contained, gated, cold-verified) · `DIRECTIONAL` (measured-but-caveated) · `PENDING`
(needs absent resources — owner-infra/representative-HW/external engines). **No number before its
artifact; losses reported beside wins.**
