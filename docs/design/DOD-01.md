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
| AC-6 substrate parity | **PENDING** — harness PROVEN; **2 / 3 named substrates** built | The `substrate_conformance<S>` harness passes over `LoopbackSubstrate`, `ResumableSubstrate`, `SocketSubstrate` (UDS), `TcpSubstrate` (real cross-host TCP), **`ObjectStoreSubstrate`** (SPEC-10 object store), **and `QuicSubstrate`** (real QUIC over quinn — ✓ named #2; proven over a real handshake + two-endpoint transfer; blind relay — no transport-TLS confidentiality, the seal carries it). **Still owed: shmem (#24, ⛔ blocked-on-owner — the forbid(unsafe) conflict).** If shmem resolves to descope (D), AC-6 closes at 2/3 (the cross-cloud + cross-host substrates, which are the load-bearing two). |
| AC-7 featherweight (GATE-FEATHER) | **DIRECTIONAL** | in-memory substrates hold **no idle resources** (idle ≈ 0 by construction); a real serverless substrate's idle RSS is a deployment measurement → PENDING |
| AC-8 WAN/resume | **PROVEN** (real-socket kill-resume + WAN harness + bao chunk-resume) / DIRECTIONAL bench | (a) **Real-socket kill-resume PROVEN**: `real_socket_resume.rs` partitions a **real TCP** connection mid-stream, reconnects, re-drives the outbox, effectively-once gate → **0-loss/0-dup** in order. (b) `WanLink` injects latency + loss (the lossy/high-RTT harness) + a DIRECTIONAL round-trip bench (loopback ~1.2 µs vs TCP-loopback ~124 µs/cofre). (c) **E4 BLAKE3 chunk-resume PROVEN**: `manifest::bao` splits a payload into index-bound Merkle-leaf chunks; `ChunkReceiver` authenticates each against the root, tracks a bitfield, resumes by requesting only the **missing** chunks — tamper / wrong-index / wrong-payload all rejected, partial+resume reassembles byte-for-byte (6 tests). (d) **E3 FASP delay-based CC algorithm PROVEN** (`congestion::DelayController` unit tests: backs off on RTT inflation, *ignores loss* — the FASP physics). Still **DIRECTIONAL**: the real-WAN throughput-beats-loss-based-TCP *measurement* needs a real lossy link (loopback cannot exhibit it). |
| AC-9 contract enforcement | **PROVEN** (+ proptest) | `datarail-terminal` `ac9_*` (case-based: onboarding refusal, contract_fp, post-decrypt record, tampered, forged, route-mismatch — both terminals) **plus `tests::prop::*` randomized proptest** (boards-iff-all-conform · conforming-batch-round-trips · any-byte-flip-dead-lettered) — the SPEC-11 **named proof method now met** (#29). |
| AC-10 fairness benchmark | **PENDING** | needs **real competitor engines** + the 17-case anti-cheat rig (external infra, task #20). Not faked. |

## Gates

| Gate | Status | Evidence |
|---|---|---|
| `GATE-WARP` (throughput ≥ X) | **PENDING** | bound `X` unset; requires representative **x86 VAES** HW (this box reports `x86_64` — real Intel *or* Rosetta; VAES unconfirmed, task #13). BENCH-01 gives directional throughput (BLAKE3 ~3.4 GB/s, GCM-SIV ~1 GB/s) but that is **not** a GATE-WARP pass |
| `GATE-LATENCY` (p50/p99 per hop) | **DIRECTIONAL** | BENCH-01 (Rosetta-x86_64): board ~**1.45 ms** / offload ~**0.94 ms**, dominated by **emulated** X25519 (~0.5–0.7 ms/side) — **NOT sub-ms on this box**; native arm64 was ~190 µs / ~136 µs (sub-ms). Amortizes per batch (wrap once per cofre). Formal p50/p99 on representative native/VAES HW pending (#13) |
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
| INV-SUBSTRATE-POLYMORPHIC | **PROVEN** | one `substrate_conformance` harness, **6 transports** (loopback / resumable / UDS / TCP / object-store / QUIC); 2/3 *named* substrates built, shmem (#24) blocked-on-owner |
| INV-EPHEMERAL-RAIL | **DIRECTIONAL** | architectural; **E5 DoS proof-of-IP cookie** (`admission::CookieGate`) built + tested — a scale-to-zero endpoint resists spoofed floods without allocating state (the stateless-endpoint defense); a real serverless substrate's idle measurement is post-v1 (#30) |

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
