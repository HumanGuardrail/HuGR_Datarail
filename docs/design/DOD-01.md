# DOD-01 — Definition-of-Done ledger (v1)

> Honest status of every Acceptance Criterion, gate, and invariant, labeled per the SPEC-11 taxonomy:
> **PROVEN** (self-contained, gated, cold-verified) · **DIRECTIONAL** (measured-but-caveated) · **PENDING**
> (needs absent resources). 11 crates · 72 tests · clippy `deny(all+pedantic)` clean · `forbid(unsafe)` ·
> no `#[allow]`. Verified via the toolchain directly (see AUDIT-02 env note).
>
> ## ⚠️ CORRECTION (2026-06-22) — P3 RE-OPENED; v1 is **NOT** complete
>
> A prior revision of this ledger claimed "the v1 product is complete" and marked **AC-6 PROVEN (×3
> substrates)**. That was **false against the frozen SPEC**. SPEC `07-rail-substrate.md` names exactly three
> real substrates — **shmem · QUIC · object-store/S3** — and **none were built**; the "3 substrates" counted
> two in-memory stubs + a Unix-socket toy. The roadmap's P3 also mandates **AC-8 WAN bench vs TCP**, **E3**
> FASP delay-based CC, **E4** BLAKE3-`bao` chunk resume, **E5** DoS proof-of-IP cookie, and a **real**
> GATE-FEATHER idle measurement — all unbuilt. P3 is **re-opened** (tasks #22–#30). Honest position today:
> **P0–P2 + P4–P5(partial) PROVEN; P3 substrate layer in progress** (`TcpSubstrate` = first real cross-host
> transport + AC-8 baseline, landed). The statuses below are corrected to reflect this.

## Acceptance Criteria

| AC | Status | Proof artifact |
|---|---|---|
| AC-1 opaque cargo (metamorphic) | **PROVEN** | `datarail-rail` metamorphic trace test (payload-swap under fixed header ⇒ byte-identical substrate trace) + MF-4 slice |
| AC-2 tamper-reject | **PROVEN** | `datarail-cofre::ac2_mutate_every_byte_is_rejected` (exhaustive byte-flip — stronger than sampled proptest) |
| AC-3 forge-reject (differential) | **PROVEN** | `datarail-cofre::ac3_wrong_and_forged_key_rejected` + `datarail-terminal::ac9_forged_cofre_wrong_signer` |
| AC-4 effectively-once (DST) | **PROVEN** | `datarail-once/tests/ac4_dst.rs` — 1000 seeds, drop/reorder/dup/kill-respawn, **0 loss / 0 dup**, now with `gc_lag=3` so the GC path fires (AUDIT-02 F7) |
| AC-5 delivery proof (differential) | **PROVEN** (TSA out of scope) | `datarail-manifest` 13 tests (independent root re-derivation + tamper rejection) + MF-4 `verify_delivery`. Bundle = `{inclusion, STH, ack}`; **TSA/RFC-3161 anchoring is async, off the delivery path** (SPEC 04 / MAJ-1) → not in v1 |
| AC-6 substrate parity | **PENDING** — harness PROVEN; **0 / 3 named substrates** built | The `substrate_conformance<S>` harness is real and passed by `LoopbackSubstrate`, `ResumableSubstrate`, `SocketSubstrate` (UDS), **and `TcpSubstrate`** (real cross-host TCP; `tcp_cross_endpoint_transfers_cofre_byte_for_byte`). **But AC-6 requires the three SPEC-named substrates — shmem / QUIC / object-store-S3 — and those are unbuilt** (#24/#25/#23). The polymorphism *mechanism* is proven over RAM + a kernel pipe + a TCP socket; the *named substrate set* is not yet met. |
| AC-7 featherweight (GATE-FEATHER) | **DIRECTIONAL** | in-memory substrates hold **no idle resources** (idle ≈ 0 by construction); a real serverless substrate's idle RSS is a deployment measurement → PENDING |
| AC-8 WAN/resume | **PARTIAL** — in-memory kill-resume PROVEN; real-socket resume + WAN bench + `bao` PENDING | `ac8_resume.rs` proves partition + resume-from-last-acked, **0 loss / 0 dup**, in order — but over an *in-memory* substrate. The SPEC-11 proof needs (a) a throughput bench on a lossy/high-RTT link **vs the TCP baseline** (`TcpSubstrate` is now that baseline; WAN shim + bench = #26) and (b) **E4 `bao`** chunk-level resume (#27). Whole-cofre cursor resume ≠ bao chunk resume. |
| AC-9 contract enforcement | **PROVEN** | `datarail-terminal` `ac9_*` — onboarding refusal (never boards) + offloading refusals (contract_fp, post-decrypt record, tampered, forged, route-mismatch), both terminals; case-based (proptest randomization a strengthening TODO) |
| AC-10 fairness benchmark | **PENDING** | needs **real competitor engines** + the 17-case anti-cheat rig (external infra, task #20). Not faked. |

## Gates

| Gate | Status | Evidence |
|---|---|---|
| `GATE-WARP` (throughput ≥ X) | **PENDING** | bound `X` unset; requires representative **x86 VAES** HW (this box is Apple Silicon arm64, task #13). BENCH-01 gives directional throughput (BLAKE3 ~3.4 GB/s, GCM-SIV ~1 GB/s) but that is **not** a GATE-WARP pass |
| `GATE-LATENCY` (p50/p99 per hop) | **DIRECTIONAL** | BENCH-01: board ~190 µs / offload ~136 µs end-to-end (sub-ms per cofre) on dev HW; X25519 wrap dominates (~90 µs/side), amortizes per batch. Formal p50/p99 on representative HW pending |
| `GATE-FEATHER` (idle ≈ 0) | **DIRECTIONAL** | architectural: the in-process substrates hold no standing resources; the scale-to-zero claim holds by construction; a real-substrate idle measurement is PENDING |

## Invariants (verified in code — AUDIT-02)

| INV | Status | Where enforced |
|---|---|---|
| INV-OPAQUE-CARGO | **PROVEN** | substrates never read `carga` (audit + AC-1) |
| INV-SEAL-COMPLETE | **PROVEN** | `signed_region` covers all 10 etiqueta fields incl. `eph_pk` + carga (audit + AC-2) |
| INV-TAMPER-REJECT | **PROVEN** | `cofre::verify` (AC-2/3) |
| INV-EFFECTIVELY-ONCE | **PROVEN** | `Once` reject-below + dedup + GC (AC-4, AC-8) |
| INV-MANIFEST-RECONCILES | **PROVEN** | `verify_delivery` (AC-5) |
| INV-CONTRACT-SPLIT | **PROVEN** | content contract at the terminal only (AC-9, audit) |
| INV-DUMB-PIPE | **PROVEN** | substrates hold no keys / do no crypto (audit) |
| INV-SUBSTRATE-POLYMORPHIC | **PROVEN** (mechanism) | one `substrate_conformance` harness, 4 transports (loopback/resumable/UDS/TCP); the 3 *named* substrates (shmem/QUIC/S3) still owed for AC-6 |
| INV-EPHEMERAL-RAIL | **DIRECTIONAL** | architectural; in-memory substrates; a real ephemeral (serverless) substrate is post-v1 |

## Standing STOP-THE-LINE for the owner

1. **GATE-WARP bound `X` + representative VAES hardware** (#13) — set X and re-bench on the target HW; the CI guard must fail until X is set (MAJ-7).
2. **AC-10 fairness** — provide the real competitor engines + the anti-cheat rig (#20).
3. **MF-0 trio ratification** (DECOMPOSITION.md §6): A3 AEAD→GCM-SIV default, AC-1 metamorphic redefinition, C1 idempotency = record_key.
4. **Post-v1 (not blocking v1):** real network substrates (QUIC/shmem/S3) to upgrade AC-6 to "× 3 real substrates" and complete the AC-8 WAN-vs-TCP benchmark. *(The AUDIT-02 hardening TODOs F4 zeroize / F5 AEAD-AAD-binding are now FIXED in `cb79d75`; F2 resolved-by-design.)*

## Summary

**v1 is NOT complete — corrected 2026-06-22.** The *spine* is real and PROVEN: sealed cofres with per-cofre
X25519-wrapped keys, smart terminals with content-contracts + dead-letter, an offline-verifiable manifest, a
`rail.toml` + `datarail` CLI, effectively-once, and a vertical slice over in-memory + real cross-process/host
(UDS/TCP) transports. **What v1 still owes (frozen P3 scope, re-opened):** the three SPEC-named substrates
(shmem #24, QUIC #25, object-store/S3 #23); the AC-8 WAN bench vs the TCP baseline (#26); E4 `bao` chunk
resume (#27); E3 FASP CC + E5 DoS cookie (#28); AC-9 proptest (#29); the real-hop P5 slice + real
GATE-FEATHER measurement (#30). Genuinely external (STOP-THE-LINE): GATE-WARP bound + VAES HW (#13), AC-10
engines (#20), MF-0 trio ratification.
