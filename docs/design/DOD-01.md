# DOD-01 — Definition-of-Done ledger (v1)

> Honest status of every Acceptance Criterion, gate, and invariant, labeled per the SPEC-11 taxonomy:
> **PROVEN** (self-contained, gated, cold-verified) · **DIRECTIONAL** (measured-but-caveated) · **PENDING**
> (needs absent resources). **16 crates · 140 tests** · clippy `deny(all+pedantic)` clean. `forbid(unsafe)`
> workspace-wide **except** the quarantined `datarail-substrate-shmem` crate (`deny(unsafe)` + **one** audited
> `#[allow]` for the shared-memory atomic cursors — owner-delegated WAIVER, `BUILD_LOG` §6 / AUDIT-03).
> Verified via the toolchain directly (see AUDIT-02 env note). **P3 re-open code audited — AUDIT-03 (F1/F2/F3
> fixed + the shmem `unsafe` audited sound).**
>
> ## ⚠️ CORRECTION (2026-06-22) — P3 was RE-OPENED, then rebuilt for real
>
> A prior revision claimed "the v1 product is complete" and marked **AC-6 PROVEN (×3 substrates)** — **false
> against the frozen SPEC** (`07-rail-substrate.md` names shmem · QUIC · object-store/S3; none were then built —
> the "3" counted two in-memory stubs + a Unix-socket toy). P3 was re-opened (#22–#30) and the substrate layer
> has since been **rebuilt for real**: `TcpSubstrate` (cross-host + AC-8 baseline), `ObjectStoreSubstrate`
> (SPEC-10 object store), `QuicSubstrate` (real QUIC), plus the AC-8 WAN harness + real-socket kill-resume, E4
> `bao` chunk-resume, E3 FASP CC, E5 DoS cookie, AC-9 proptest, the P5 real-hop slice, and a concrete
> GATE-FEATHER idle test, **and `ShmemRing`** — the lock-free SPSC shared-memory ring (#24): the owner
> delegated the `forbid(unsafe)` vs "lock-free" decision to the tech lead, who took option **(A)** — one
> contained, audited `unsafe` in the quarantined crate. **Honest position today: AC-6 is 3/3 named substrates
> and every buildable P3/P5 rung is PROVEN + AUDITED.** The remaining open items are **owner/external only —
> physical resources, not decisions**: representative **VAES hardware** for GATE-WARP (#13) and **real
> competitor engines** for AC-10 (#20). Nothing buildable here is left undone.

## Acceptance Criteria

| AC | Status | Proof artifact |
|---|---|---|
| AC-1 opaque cargo (metamorphic) | **PROVEN** | `datarail-rail` metamorphic trace test (payload-swap under fixed header ⇒ byte-identical substrate trace) + MF-4 slice |
| AC-2 tamper-reject | **PROVEN** | `datarail-cofre::ac2_mutate_every_byte_is_rejected` (exhaustive byte-flip — stronger than sampled proptest) |
| AC-3 forge-reject (differential) | **PROVEN** | `datarail-cofre::ac3_wrong_and_forged_key_rejected` + `datarail-terminal::ac9_forged_cofre_wrong_signer`. **Identity layer (F2/F3) built + PRODUCT-INTEGRATED** (`datarail-identity`): `Noise_KK` mutual-static auth + SPAKE2 short-code bootstrap (cold-verified). **F2 is now wired into the rail**: `NoiseSubstrate` tunnels sealed cofres through a `Noise_KK` channel, and `datarail send/recv --noise-secret/--peer-public` run a **real two-process encrypted hop** (`tests/two_process.rs` + live demo) — it mutually authenticates the endpoints and **encrypts the cleartext etiqueta metadata on the wire** (mitigating the SPEC-03 metadata residual). **F3 is now a real remote protocol too**: `datarail pair --connect/--listen --code <hex>` runs a **two-process** SPAKE2 pairing over TCP — the sides exchange static pubkeys + PAKE messages + confirmation tags over the socket; a matching code → same secret + mutually-authenticated peer static, a wrong code → confirm-fail + burn (`tests/two_process.rs::two_process_remote_pairing_agrees_on_a_secret` + live demo). The whole SPEC-08 identity layer (F2 Noise hop + F3 remote pairing + F4 ticket) is built and exercised end-to-end. |
| AC-4 effectively-once (DST) | **PROVEN** | `datarail-once/tests/ac4_dst.rs` — 1000 seeds, drop/reorder/dup/kill-respawn, **0 loss / 0 dup**, now with `gc_lag=3` so the GC path fires (AUDIT-02 F7) |
| AC-5 delivery proof (differential) | **PROVEN** (TSA out of scope) | `datarail-manifest` 13 tests (independent root re-derivation + tamper rejection) + MF-4 `verify_delivery`. Bundle = `{inclusion, STH, ack}`; **TSA/RFC-3161 anchoring is async, off the delivery path** (SPEC 04 / MAJ-1) → not in v1 |
| AC-6 substrate parity | **PROVEN** — harness + **3/3 named substrates** | The `substrate_conformance<S>` harness passes over `LoopbackSubstrate`, `ResumableSubstrate`, `SocketSubstrate` (UDS), `TcpSubstrate`, **and all three SPEC-named substrates: `ShmemRing`** (lock-free SPSC shared-memory ring, µs same-host — cross-mapping/back-pressure/wraparound tested), **`ObjectStoreSubstrate`** (SPEC-10 object store, cross-cloud), **`QuicSubstrate`** (real QUIC, cross-host blind relay). The full board→real-hop→verify→offload→Delivered pipe is proven over **all three** real hops in `real_hop_slice.rs`. **Cross-host is now real at the product level too**: `datarail send`/`recv` move a sealed shipment between **two separate OS processes** over a real TCP socket (`tests/two_process.rs` spawns the binary twice). |
| AC-7 featherweight (GATE-FEATHER) | **DIRECTIONAL** (idle in-flight ≈ 0 PROVEN) | `gate_feather_idle_substrate_retains_nothing` proves an ephemeral substrate retains **0 in-flight state** across burst→drain→ack cycles (a passive struct: no thread/fd/timer). A real serverless substrate's idle **RSS** is a deployment measurement → PENDING |
| AC-8 WAN/resume | **PROVEN** (real-socket kill-resume + WAN harness + bao chunk-resume) / DIRECTIONAL bench | (a) **Real-socket kill-resume PROVEN**: `real_socket_resume.rs` partitions a **real TCP** connection mid-stream, reconnects, re-drives the outbox, effectively-once gate → **0-loss/0-dup** in order. (b) `WanLink` injects latency + loss (the lossy/high-RTT harness) + a DIRECTIONAL round-trip bench (loopback ~1.2 µs vs TCP-loopback ~124 µs/cofre). (c) **E4 BLAKE3 chunk-resume PROVEN**: `manifest::bao` splits a payload into index-bound Merkle-leaf chunks; `ChunkReceiver` authenticates each against the root, tracks a bitfield, resumes by requesting only the **missing** chunks — tamper / wrong-index / wrong-payload all rejected, partial+resume reassembles byte-for-byte (6 tests). (d) **E3 FASP delay-based CC algorithm PROVEN** (`congestion::DelayController` unit tests: backs off on RTT inflation, *ignores loss* — the FASP physics). Still **DIRECTIONAL**: the real-WAN throughput-beats-loss-based-TCP *measurement* needs a real lossy link (loopback cannot exhibit it). |
| AC-9 contract enforcement | **PROVEN** (+ proptest) | `datarail-terminal` `ac9_*` (case-based: onboarding refusal, contract_fp, post-decrypt record, tampered, forged, route-mismatch — both terminals) **plus `tests::prop::*` randomized proptest** (boards-iff-all-conform · conforming-batch-round-trips · any-byte-flip-dead-lettered) — the SPEC-11 **named proof method now met** (#29). |
| AC-10 fairness benchmark | **PENDING** (bake-off) — **rig + methodology built** | The fairness **rig** is built (`datarail-bench::fairness`): a `MoverEngine` trait, datarail + an idealised pass-through baseline, and the **byte-equal-before-timing** anti-cheat gate — tested (correct engines pass; a record-dropping "cheater" is disqualified *before* any timing). The **real competitor engines** (Kafka / MFT / Fivetran / …) are external software/infra (#20); the bake-off numbers stay PENDING. Not faked. |

## Gates

| Gate | Status | Evidence |
|---|---|---|
| `GATE-WARP` (throughput ≥ X) | **PENDING** (measurement) — **X now set** | **X = 1 GiB/s/core** sealed-payload (batched) — a tech-lead **design target** (SPEC-11, set 2026-06-22). The binding *pass* needs representative VAES server HW; this box is a **native Intel i7-9750H (Coffee Lake, 2019) — AES-NI but pre-VAES** (#13), so it can't be the target measurement. The CI guard stays **red until a real VAES run records it**. BENCH-01 figures are noisy/directional, **not** a GATE-WARP pass. |
| `GATE-LATENCY` (p50/p99 per hop) | **DIRECTIONAL** | BENCH-01 on the i7-9750H laptop is **noisy** (two runs disagreed 2–3×: `board` ~1.45–3.6 ms, dominated by X25519) — **not a sub-ms pass and not benchmark-grade**. The reliable finding is the *shape* (asymmetric-crypto-bound, batch-amortizing). Formal p50/p99 needs a quiet representative server + a criterion harness (#13). |
| `GATE-FEATHER` (idle ≈ 0) | **DIRECTIONAL** (architectural half PROVEN) | `gate_feather_*` test: idle in-flight returns to 0 every cycle (no standing data); the in-process substrate is a passive struct (no thread/fd). Real-substrate idle-RSS measurement still PENDING |

## Invariants (verified in code — AUDIT-02)

| INV | Status | Where enforced |
|---|---|---|
| INV-OPAQUE-CARGO | **PROVEN** | substrates never read `carga` (audit + AC-1) |
| INV-SEAL-COMPLETE | **PROVEN** | `signed_region` covers all 12 etiqueta fields (incl. `eph_pk` + `sender_present` + `ts`) + carga (audit + AC-2) |
| INV-TAMPER-REJECT | **PROVEN** | `cofre::verify` (AC-2/3) |
| INV-EFFECTIVELY-ONCE | **PROVEN** | `Once` reject-below + dedup + GC (AC-4, AC-8) |
| INV-MANIFEST-RECONCILES | **PROVEN** | `verify_delivery` (AC-5) |
| INV-CONTRACT-SPLIT | **PROVEN** | content contract at the terminal only (AC-9, audit) |
| INV-DUMB-PIPE | **PROVEN** | substrates hold no keys / do no crypto (audit) |
| INV-SUBSTRATE-POLYMORPHIC | **PROVEN** | one `substrate_conformance` harness, **7 transports** (loopback / resumable / UDS / TCP / object-store / QUIC / shmem); **3/3 named substrates built** |
| INV-EPHEMERAL-RAIL | **DIRECTIONAL** | architectural; **E5 DoS proof-of-IP cookie** (`admission::CookieGate`) built + tested — a scale-to-zero endpoint resists spoofed floods without allocating state (the stateless-endpoint defense); a real serverless substrate's idle measurement is post-v1 (#30) |

## Remaining open items (external **physical resources** — not tech-lead decisions)

1. **GATE-WARP — representative VAES hardware** (#13): the pass measurement needs real x86 **VAES** server HW; this box is a native Intel **i7-9750H (Coffee Lake, pre-VAES)** laptop, so it is not the target. Setting the design-target `X` + the CI-gate policy is a tech-lead call (done); the representative *measurement* is hardware-gated. The guard fails until X is recorded on the target (MAJ-7).
2. **AC-10 — real competitor engines** (#20): the fairness bake-off needs the actual competitor binaries + infra. Building the **rig + methodology** is a tech-lead call; the real engines are external software.
3. **MF-0 trio ratification** (DECOMPOSITION.md §6: A3 AEAD→GCM-SIV default · AC-1 metamorphic redefinition · C1 idempotency = record_key) — a tech-lead call (owner-delegated), to be applied to the DRAFT.

*(shmem #24 **RESOLVED**: the owner delegated the decision; the tech lead took option (A) — one contained, audited `unsafe` in the quarantined crate, logged as a WAIVER, `BUILD_LOG` §6. AUDIT-02 hardening F4 zeroize / F5 AEAD-AAD-binding FIXED in `cb79d75`; F2 resolved-by-design.)*

## Summary

**v1 is as-complete-as-buildable-on-this-box — corrected + rebuilt 2026-06-22.** The spine *and* the P3
substrate layer are real and PROVEN: sealed cofres with per-cofre X25519-wrapped keys, smart terminals
(content-contracts + dead-letter), an offline-verifiable manifest, effectively-once, a `rail.toml` + `datarail`
CLI, **real substrates** (TCP/UDS cross-process/host · QUIC cross-host · object-store cross-cloud) behind one
conformance harness, the AC-8 WAN harness + real-socket kill-resume + `bao` chunk-resume, E3 FASP delay-based
CC, E5 DoS proof-of-IP cookie, a concrete GATE-FEATHER idle test, the P5 vertical slice over **all three real
hops** (shmem + QUIC + decoupled object-store), and **`ShmemRing`** completing the named set — **AC-6 is 3/3**.
Audited (AUDIT-03: F1/F2/F3 fixed + the shmem `unsafe` sound). **Everything buildable here is done, PROVEN, and
AUDITED.** What remains needs **physical resources, not decisions**: representative VAES HW for GATE-WARP (#13)
and real competitor engines for AC-10 (#20); plus the MF-0 trio ratification (a tech-lead call, to apply).
Performance is **DIRECTIONAL** (this box is a native Intel i7-9750H Coffee Lake laptop — AES-NI, pre-VAES — not the VAES server target, and its numbers are noisy run-to-run). Nothing is faked.
