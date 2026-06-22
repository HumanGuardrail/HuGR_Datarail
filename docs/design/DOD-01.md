# DOD-01 — Definition-of-Done ledger (v1)

> Honest status of every Acceptance Criterion, gate, and invariant, labeled per the SPEC-11 taxonomy:
> **PROVEN** (self-contained, gated, cold-verified) · **DIRECTIONAL** (measured-but-caveated) · **PENDING**
> (needs absent resources). 13 crates · 99 tests · clippy `deny(all+pedantic)` clean · `forbid(unsafe)` ·
> no `#[allow]`. Verified via the toolchain directly (see AUDIT-02 env note). **The P3 re-open code is audited
> — AUDIT-03 (3 findings: F1 frame-cap MED, F2 cookie-Debug-redact, F3 object-size-cap; all FIXED).**
>
> ## ⚠️ CORRECTION (2026-06-22) — P3 was RE-OPENED, then rebuilt for real
>
> A prior revision claimed "the v1 product is complete" and marked **AC-6 PROVEN (×3 substrates)** — **false
> against the frozen SPEC** (`07-rail-substrate.md` names shmem · QUIC · object-store/S3; none were then built —
> the "3" counted two in-memory stubs + a Unix-socket toy). P3 was re-opened (#22–#30) and the substrate layer
> has since been **rebuilt for real**: `TcpSubstrate` (cross-host + AC-8 baseline), `ObjectStoreSubstrate`
> (SPEC-10 object store), `QuicSubstrate` (real QUIC), plus the AC-8 WAN harness + real-socket kill-resume, E4
> `bao` chunk-resume, E3 FASP CC, E5 DoS cookie, AC-9 proptest, the P5 real-hop slice, and a concrete
> GATE-FEATHER idle test. **Honest position today: every buildable P3/P5 rung is PROVEN; the only un-built named
> substrate is shmem (#24), ⛔ blocked-on-owner by a frozen `forbid(unsafe)` vs "lock-free" conflict
> (`BUILD_LOG` §6).** v1 is **as-complete-as-buildable-here** — remaining open items are owner/external only
> (shmem #24, GATE-WARP HW #13, AC-10 #20, MF-0 trio); *not* "done", but nothing buildable is left undone.

## Acceptance Criteria

| AC | Status | Proof artifact |
|---|---|---|
| AC-1 opaque cargo (metamorphic) | **PROVEN** | `datarail-rail` metamorphic trace test (payload-swap under fixed header ⇒ byte-identical substrate trace) + MF-4 slice |
| AC-2 tamper-reject | **PROVEN** | `datarail-cofre::ac2_mutate_every_byte_is_rejected` (exhaustive byte-flip — stronger than sampled proptest) |
| AC-3 forge-reject (differential) | **PROVEN** | `datarail-cofre::ac3_wrong_and_forged_key_rejected` + `datarail-terminal::ac9_forged_cofre_wrong_signer` |
| AC-4 effectively-once (DST) | **PROVEN** | `datarail-once/tests/ac4_dst.rs` — 1000 seeds, drop/reorder/dup/kill-respawn, **0 loss / 0 dup**, now with `gc_lag=3` so the GC path fires (AUDIT-02 F7) |
| AC-5 delivery proof (differential) | **PROVEN** (TSA out of scope) | `datarail-manifest` 13 tests (independent root re-derivation + tamper rejection) + MF-4 `verify_delivery`. Bundle = `{inclusion, STH, ack}`; **TSA/RFC-3161 anchoring is async, off the delivery path** (SPEC 04 / MAJ-1) → not in v1 |
| AC-6 substrate parity | **PENDING** — harness PROVEN; **2 / 3 named substrates** built | The `substrate_conformance<S>` harness passes over `LoopbackSubstrate`, `ResumableSubstrate`, `SocketSubstrate` (UDS), `TcpSubstrate` (real cross-host TCP), **`ObjectStoreSubstrate`** (SPEC-10 object store), **and `QuicSubstrate`** (real QUIC over quinn — ✓ named #2; proven over a real handshake + two-endpoint transfer; blind relay — no transport-TLS confidentiality, the seal carries it). **Still owed: shmem (#24, ⛔ blocked-on-owner — the forbid(unsafe) conflict).** If shmem resolves to descope (D), AC-6 closes at 2/3 (the cross-cloud + cross-host substrates, which are the load-bearing two). The full board→real-hop→verify→offload→Delivered pipe is proven over **both** real hops in `real_hop_slice.rs`. |
| AC-7 featherweight (GATE-FEATHER) | **DIRECTIONAL** (idle in-flight ≈ 0 PROVEN) | `gate_feather_idle_substrate_retains_nothing` proves an ephemeral substrate retains **0 in-flight state** across burst→drain→ack cycles (a passive struct: no thread/fd/timer). A real serverless substrate's idle **RSS** is a deployment measurement → PENDING |
| AC-8 WAN/resume | **PROVEN** (real-socket kill-resume + WAN harness + bao chunk-resume) / DIRECTIONAL bench | (a) **Real-socket kill-resume PROVEN**: `real_socket_resume.rs` partitions a **real TCP** connection mid-stream, reconnects, re-drives the outbox, effectively-once gate → **0-loss/0-dup** in order. (b) `WanLink` injects latency + loss (the lossy/high-RTT harness) + a DIRECTIONAL round-trip bench (loopback ~1.2 µs vs TCP-loopback ~124 µs/cofre). (c) **E4 BLAKE3 chunk-resume PROVEN**: `manifest::bao` splits a payload into index-bound Merkle-leaf chunks; `ChunkReceiver` authenticates each against the root, tracks a bitfield, resumes by requesting only the **missing** chunks — tamper / wrong-index / wrong-payload all rejected, partial+resume reassembles byte-for-byte (6 tests). (d) **E3 FASP delay-based CC algorithm PROVEN** (`congestion::DelayController` unit tests: backs off on RTT inflation, *ignores loss* — the FASP physics). Still **DIRECTIONAL**: the real-WAN throughput-beats-loss-based-TCP *measurement* needs a real lossy link (loopback cannot exhibit it). |
| AC-9 contract enforcement | **PROVEN** (+ proptest) | `datarail-terminal` `ac9_*` (case-based: onboarding refusal, contract_fp, post-decrypt record, tampered, forged, route-mismatch — both terminals) **plus `tests::prop::*` randomized proptest** (boards-iff-all-conform · conforming-batch-round-trips · any-byte-flip-dead-lettered) — the SPEC-11 **named proof method now met** (#29). |
| AC-10 fairness benchmark | **PENDING** | needs **real competitor engines** + the 17-case anti-cheat rig (external infra, task #20). Not faked. |

## Gates

| Gate | Status | Evidence |
|---|---|---|
| `GATE-WARP` (throughput ≥ X) | **PENDING** | bound `X` unset; requires representative **x86 VAES** HW (this box reports `x86_64` — real Intel *or* Rosetta; VAES unconfirmed, task #13). BENCH-01 gives directional throughput (BLAKE3 ~3.4 GB/s, GCM-SIV ~1 GB/s) but that is **not** a GATE-WARP pass |
| `GATE-LATENCY` (p50/p99 per hop) | **DIRECTIONAL** | BENCH-01 (Rosetta-x86_64): board ~**1.45 ms** / offload ~**0.94 ms**, dominated by **emulated** X25519 (~0.5–0.7 ms/side) — **NOT sub-ms on this box**; native arm64 was ~190 µs / ~136 µs (sub-ms). Amortizes per batch (wrap once per cofre). Formal p50/p99 on representative native/VAES HW pending (#13) |
| `GATE-FEATHER` (idle ≈ 0) | **DIRECTIONAL** (architectural half PROVEN) | `gate_feather_*` test: idle in-flight returns to 0 every cycle (no standing data); the in-process substrate is a passive struct (no thread/fd). Real-substrate idle-RSS measurement still PENDING |

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
4. **shmem substrate (#24)** — the only un-built named substrate; blocked on the frozen `forbid(unsafe)` vs "lock-free shmem" conflict (`BUILD_LOG` §6): owner picks (A) contained-unsafe waiver / (B) flock / (C) safe-API dep / (D) descope → AC-6 stays 2/3. QUIC + object-store are built; the AC-8 WAN-vs-TCP throughput differentiator needs a real WAN link (loopback can't show it). *(AUDIT-02 hardening TODOs F4 zeroize / F5 AEAD-AAD-binding FIXED in `cb79d75`; F2 resolved-by-design.)*

## Summary

**v1 is as-complete-as-buildable-on-this-box — corrected + rebuilt 2026-06-22.** The spine *and* the P3
substrate layer are real and PROVEN: sealed cofres with per-cofre X25519-wrapped keys, smart terminals
(content-contracts + dead-letter), an offline-verifiable manifest, effectively-once, a `rail.toml` + `datarail`
CLI, **real substrates** (TCP/UDS cross-process/host · QUIC cross-host · object-store cross-cloud) behind one
conformance harness, the AC-8 WAN harness + real-socket kill-resume + `bao` chunk-resume, E3 FASP delay-based
CC, E5 DoS proof-of-IP cookie, a concrete GATE-FEATHER idle test, and the P5 vertical slice over **real hops**
(QUIC + decoupled object-store). **The only un-built named substrate is shmem (#24, ⛔ owner-blocked.)**
Genuinely external / owner — not buildable here: shmem decision (#24), GATE-WARP bound + representative VAES HW
(#13), AC-10 competitor engines (#20), MF-0 trio ratification. Performance is **DIRECTIONAL** (this box is
Rosetta-x86_64, not the VAES target). Everything provable here is **PROVEN**; nothing is faked.
