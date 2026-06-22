# 99 — Coverage Matrix

> **MF-2 freeze gate.** Every capability, Acceptance Criterion, invariant, gate, must-build, and stolen
> pattern must map to an owning design doc. **Verdict line has teeth: a row with no owner ⇒ not frozen.**
> Status: DRAFT — re-run before the content-hash freeze.

## Capabilities → owner

| Leaf | Owner |
|---|---|
| A1–A5 (cofre) | `02` (+ `03`) |
| B1–B3 (manifest) | `04` |
| C1–C3 (effectively-once) | `05` |
| D1–D4 (terminals) | `06` |
| E1–E5 (rail) | `07` |
| F1–F4 (routing/identity) | `08` |
| G1–G3 (spec/CLI) | `09` |
| H1 (durable store) | `10` |

## Acceptance Criteria → owner + proof method

| AC | Owner | Proof (per `11`) |
|---|---|---|
| AC-1 opaque cargo | 01,02 | metamorphic |
| AC-2 tamper-reject | 02,06 | proptest (mutate every byte) |
| AC-3 forge-reject | 03,06,08 | differential |
| AC-4 effectively-once | 05 | DST |
| AC-5 delivery proof | 04 | differential |
| AC-6 substrate parity | 07 | one suite × 3 substrates |
| AC-7 featherweight | 07,09 | GATE-FEATHER |
| AC-8 WAN/resume | 07 | bench + kill-resume |
| AC-9 contract enforcement | 06 | proptest |
| AC-10 fairness benchmark | 11 | rig (external engines = **PENDING**) |

## Invariants → owner

| INV | Owner |
|---|---|
| INV-OPAQUE-CARGO | 01,02 |
| INV-SEAL-COMPLETE | 02,03 |
| INV-TAMPER-REJECT | 02,06 |
| INV-EFFECTIVELY-ONCE | 05 |
| INV-MANIFEST-RECONCILES | 04 |
| INV-CONTRACT-SPLIT | 01,06 |
| INV-EPHEMERAL-RAIL | 05,07 |
| INV-SUBSTRATE-POLYMORPHIC | 07 |
| INV-DUMB-PIPE | 02,07 |

## Gates → owner

`GATE-WARP` · `GATE-LATENCY` · `GATE-FEATHER` → defined in `11`; measured in `07`/`02`. (`GATE-WARP` bound = **PENDING** representative-HW.)

## Stolen patterns → owner (the "did we incorporate every moat?" check, mechanized)

| Pattern (source) | Owner |
|---|---|
| zero-copy of ciphertext (Kafka inversion) | 07 + Charter L4 |
| idempotency seq/clock in envelope (Kafka/Gazette) | 02,05 |
| FASP delay-based rate control (Aspera) ⚙️ must-build | 07 |
| BLAKE3 `bao` verified-streaming + resume (Iroh) ⚙️ must-build | 07,04 |
| sealed-sender (Signal) | 03,02 |
| transparency-log manifest + inclusion proof (Sigstore) | 04 |
| AEAD GCM-SIV / nonce-misuse hardening | 03 |
| envelope encryption + wrapped data key | 03,02 |
| micro-proxy / featherweight (Linkerd) | 07,09,11 |
| DERP blind relay | 07 (INV-DUMB-PIPE) |
| CDC-log-based at source (Debezium) | 06 |
| SPIFFE identity + Noise_KK (mesh/WireGuard) | 08 |
| PAKE short-code bootstrap (wormhole) | 08 |
| dumb durable store / S3 substrate ⚙️ must-build | 10,07 |

## Verdict

**ZERO unowned rows** across capabilities (8 groups), ACs (10), invariants (9), gates (3), and stolen
patterns/must-builds (14). The two crypto **design corrections** (HMAC-per-tenant idempotency key; sealed
sender) are owned by `03`/`02`. Items legitimately not yet closed are **labeled PENDING** (not unowned):
`GATE-WARP` bound (representative HW), AC-10 external engines, owner ratification of the trio.

Re-run this matrix immediately before the SPEC freeze; the freeze does not proceed with any unowned row.
