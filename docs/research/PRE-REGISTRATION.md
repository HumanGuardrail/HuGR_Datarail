# PRE-REGISTRATION — the falsifiable hypotheses

> Pre-registered *before* the spike, so a result cannot be rationalized after the fact. The method's
> instinct: **falsify the cheapest, most foundational claim first** — for the price of one experiment, before
> designing the whole product on top of it.

## H1 — ZK-completeness (most foundational)

**Claim.** The rail's entire job (route, order, dedup, checkpoint, backpressure, dead-letter) is computable
from the authenticated header + seal alone — never the plaintext payload.

**Falsifier.** Any rail function whose required inputs are not a subset of {header, seal}.

**Spike.** Enumerate every rail function and prove its input set ⊆ {header, seal}. Mostly head reasoning
(cheap). If a function needs plaintext, it either leaks content to the rail (breaks zero-knowledge) or must
move to the terminal (changes the shape) — learn it now, not after the SPEC.

**Bar.** Every rail function provably header-only, or the function is reassigned to the terminal and recorded.

## H3 — Sealed warp speed (cheapest empirical — fires first)

**Claim.** Sealing (AEAD-encrypt + BLAKE3/Merkle + Ed25519 sign) and verifying a cofre sustains the
throughput bar on commodity hardware — i.e., E2E sealing does **not** cost us the "warp speed" promise.

**Falsifier.** Seal+verify throughput below `GATE-WARP` on a commodity core; or the terminal cannot be both
featherweight and fast across substrates.

**Spike — `spike/h3-sealed-warp-speed`.** A criterion microbench of the seal/verify pipeline across
representative batch sizes, plus a substrate-polymorphism + featherweight probe: the *same* terminal API
moving cofres over **shared-memory same-host vs QUIC cross-host**, measuring latency, throughput, and
idle/active footprint. If the terminal can't be both featherweight and fast across substrates, the
leveza / performance / cost thesis falls on day zero.

**Bar.** `GATE-WARP` (throughput target — *TBD with Owner*) + `GATE-FEATHER` (idle ≈ 0; active ≪ a sidecar).

> **Note vs Keepr.** Keepr's cheapest falsifier was byte-determinism, because its content-address is identity
> *across machines*. Datarail's seal is created at the source and *verified by signature* at the destination,
> not re-derived — so cross-machine encode-determinism is **not** load-bearing here. Our
> cheapest × most-foundational pair is **H1 + H3**.

## H2 — Effectively-once under chaos (after the skeleton)

**Claim.** Crash / retry / partition ⇒ 0 loss, 0 duplicate, given an idempotent/transactional sink, with
durability anchored at the terminal + cofre + manifest (not the rail).

**Falsifier.** Any chaos sequence that loses or duplicates an acked record.

**Spike.** A deterministic-simulation (DST) gauntlet over a minimal skeleton, N seeds, fault injection on the
dumb pipe (drop / reorder / duplicate / kill-and-respawn).

**Bar.** 0 acked-loss and 0 duplicate across all seeds.
