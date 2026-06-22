# DECOMPOSITION — the demand

> **Owner-reserved. Status: DRAFT — pending ratification + freeze (MF-0).** Everything downstream (SPEC →
> ROADMAP → WPs → code) is derived from this and proven against it. Completeness is anchored here, not on a
> doc count: the product is done when every Acceptance Criterion is green and every invariant, gate, and
> proof method is owned, implemented, and proven.

## Capabilities (the leaves)

Grouped by responsibility; these map toward the eventual crate boundaries.

### A — The Cofre (sealed vault)
- **A1** Envelope structure: authenticated header (*etiqueta*) + AEAD-encrypted payload + signature (*lacre*).
- **A2** Seal completeness: signature covers header ⊗ payload, byte-for-byte (`INV-SEAL-COMPLETE`).
- **A3** Pluggable AEAD; **default AES-256-GCM-SIV** (nonce-misuse-resistant); per-cofre fresh data key + **random** nonce; ChaCha20-Poly1305 / AES-256-GCM as alternates. *(audit-reconciled: no derived nonce — AUDIT-01 §6, owner-ratify at MF-0)*
- **A4** Sealed-sender: sender identity lives encrypted *inside* the payload; the header carries only routing-minimum.
- **A5** Envelope encryption: payload under a per-shipment data key; wrapped data key rides in the header (rotation without re-encrypt).

### B — The Manifest (provable delivery)
- **B1** Per-shipment signed manifest; reconciliation source ⊕ destination to record level.
- **B2** Delivery receipt as a Merkle inclusion proof (transparency-log pattern), verifiable offline without trusting the pipe.
- **B3** External RFC-3161 timestamp (never trust the pipe's clock).

### C — Effectively-once
- **C1** Idempotency key = `HMAC(per-tenant secret, record_key)` in the authenticated header — never a raw content hash. *(audit-reconciled: record_key not content — AUDIT-01 §6)*
- **C2** Destination dedup against the seal; unbounded window (crypto-anchored), not an in-RAM time window.
- **C3** Crash/retry/partition safety: durability anchored at source terminal + cofre + manifest, not in the rail.

### D — The Terminals (mechanism; rules are policy)
- **D1** Onboarding terminal: enforce the content contract (schema/validation) on plaintext, then seal.
- **D2** Offloading terminal: verify seal → open → enforce content contract → idempotent/transactional commit.
- **D3** Tampered / contract-failing cofre → dead-letter (siding), never delivered.
- **D4** Content contract is declarative policy (config/data), enforced by our terminal binary (no recompile per rule).

### E — The Rail (ephemeral, substrate-polymorphic, dumb)
- **E1** Ephemeral lifecycle: spawn → deliver → vanish; zero standing presence; zero durable state (`INV-EPHEMERAL-RAIL`).
- **E2** Substrate polymorphism, statically bound per route: shared-memory (same host), QUIC (cross host), object store/S3 (decoupled).
- **E3** Rate-control by *delay* not loss (FASP-physics) on the network substrate — WAN / large-transfer performance.
- **E4** Crash-safe resume via verified-streaming bitfield (BLAKE3 `bao`): re-spawn and resume only the missing chunks.
- **E5** Stateless DoS defense for the endpoint (proof-of-IP cookie ladder).

### F — Routing & identity (fixed A→B)
- **F1** Routes are fixed/pre-defined A→B; fan-out 1→N = N fixed routes. No discovery, no overlay, no DHT.
- **F2** Endpoint identity (SPIFFE-style / hash-of-pubkey) pins A↔B and derives the seal key (Noise_KK; both statics known).
- **F3** Zero-PKI bootstrap option via an audited PAKE short-code.
- **F4** Self-contained, signed, offline-shareable route descriptor (capability token).

### G — Spec & CLI (the plug & play surface)
- **G1** One declarative rail spec (source + onboarding rules + destination + offloading rules + guarantee level + keys ref).
- **G2** `datarail run` single static binary; featherweight; scale-to-zero.
- **G3** Live throughput/latency view (the speedometer).

### H — Optional durable holding pen
- **H1** Dumb commodity store (S3-class) for temporal decoupling / offline destination — stores sealed cofres only (provider-blind).

## Acceptance Criteria (the completeness oracle)

| AC | Statement | Owns | Proof method |
|---|---|---|---|
| **AC-1** | A sealed cofre is byte-opaque to the rail; route/dedup/checkpoint use header+seal only | A1,A4,E | metamorphic: swap CARGA for another validly-sealed ciphertext of equal length (re-signed), routing fixed → identical rail behavior *(AUDIT-01 §6)* |
| **AC-2** | Any single-byte mutation of header or payload is detected → dead-lettered, never delivered | A2,D3 | property test: mutate every byte position → 100% reject |
| **AC-3** | A forged cofre (wrong source key) is rejected | A2,F2 | differential vs a known-good signer |
| **AC-4** | Under crash/retry/partition with an idempotent sink, every acked record is delivered exactly once (0 loss, 0 dup) | C1,C2,C3 | DST (deterministic simulation), N seeds |
| **AC-5** | Every shipment yields a signed manifest; an offline verifier confirms delivery via inclusion proof | B1,B2,B3 | differential vs an independent verifier |
| **AC-6** | A route runs identically over shared-memory, QUIC, and object-store substrates (same terminal API) | E2,F1 | one suite across 3 substrates |
| **AC-7** | Endpoint idle footprint ≈ 0 (no standing process); active footprint ≪ a service-mesh sidecar | E1,G2 | `GATE-FEATHER` measurement |
| **AC-8** | Large transfer over a lossy / high-RTT link sustains throughput decoupled from loss; resumes after a kill | E3,E4 | benchmark vs TCP baseline + kill-resume test |
| **AC-9** | A content-contract violation is refused at onboarding (before sealing) and at offloading (before commit) | D1,D2 | property test over contract cases |
| **AC-10** | The fairness benchmark reproduces vs the strongest incumbent per corner (throughput, latency, footprint, cost-at-rest) | all | benchmark rig, byte-equal before timing, anti-cheat |

*(The AC list is the completeness oracle; finalized at trio-freeze — "no AC unowned.")*

## Invariants & gates

`INV-*` and the Craft-Charter gates `GATE-*` are defined and owned in
[`design/00-CONSTITUTION.md`](design/00-CONSTITUTION.md). Each must map to an owning design doc in the
coverage matrix before SPEC freeze ("zero unowned rows").

## Milestones (freeze-points)

- **MF-0 — Trio freeze.** PRODUCT + WORKING_BACKWARDS + DECOMPOSITION ratified by Owner, content-hashed.
- **MF-1 — Spike gate (Part I).** H1 (ZK-completeness) reasoned + H3 (sealed warp speed) measured. Refuted → rethink the shape *now*.
- **MF-2 — SPEC freeze.** Full design set + coverage matrix (zero unowned rows) + 6-lens audit closed; content-hashed.
- **MF-3 — Contract freeze.** Cofre / header / manifest IDL + terminal seam frozen; compilable stubs shipped; Part III may transcribe.
- **MF-4 — First vertical slice (PROVEN).** One full route end-to-end — same-host shmem hop + cross-host QUIC hop, sealed + manifest + effectively-once + dead-letter — proving the rail.
- **MF-5 — The wedge slice.** Cross-org / cross-cloud sealed route, provider-blind story demoable.
- **MF-6 — Fairness benchmark.** AC-10.

## Pre-registered hypotheses

H1 / H2 / H3 are defined in [`research/PRE-REGISTRATION.md`](research/PRE-REGISTRATION.md). H3 (sealed warp
speed) is the cheapest empirical falsifier and fires first, with H1 (ZK-completeness) reasoned alongside.

## Stolen-pattern provenance

The capabilities above incorporate, deliberately, the strongest ideas of the incumbents (zero-copy of
ciphertext, FASP delay-based rate control, BLAKE3 `bao` verified streaming as a signed receipt,
sealed-sender, transparency-log manifests, AES-GCM-SIV, Noise_KK, SPIFFE identity, CDC-at-source). Each is
re-implemented to also be provider-blind, exactly-once-proven, and serverless — the things the originals
are not.
