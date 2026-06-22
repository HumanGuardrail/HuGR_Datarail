# ADR-0001 — Adopt CAST: fixed-route, ephemeral, provider-blind sealed transport

- **Status:** Proposed (pending Owner ratification at MF-0).
- **Date:** 2026-06-21.

## Context

Datarail must move data cross-container with absurd levels of security, leveza, performance, and low cost,
and beat incumbents that each dominate one axis while being structurally blind on ours (see
[`../PRODUCT.md`](../PRODUCT.md) and the competitive teardown). Per THE HUGR METHOD, the first engineering
decision is the *shape of the machine*, chosen and frozen before any feature.

## Decision

Adopt **CAST — Content-Addressed Sealed Transport**, with three scoping decisions that remove the project's
biggest risks up front:

1. **Provider-blind / zero-knowledge by default** — the rail never sees plaintext; cofres are sealed E2E.
   This is what lets the pipe be dumb, cheap, and untrusted (doctrine: *smart sealed endpoints, dumb cheap
   pipes*).
2. **Fixed A→B routes (no overlay/discovery)** — deletes the hardest subsystem (dynamic
   discovery/routing/DHT) and the would-be `INV-LOCATION-TRANSPARENT`. Content-addressing is retained as
   *identity*, dropped as *routing*. Fan-out = N fixed routes.
3. **Ephemeral rail (serverless / scale-to-zero)** — the rail spawns, delivers, and vanishes; zero standing
   infra; idle cost ≈ 0. Durability anchors at the terminal + cofre + manifest, never in the rail.

The four pillars and the named invariants live in [`../design/00-CONSTITUTION.md`](../design/00-CONSTITUTION.md).

## Consequences

- **+** Security, leveza, cost, and untrusted-infra portability all fall out of one fact — the seal.
- **+** The biggest *technical* risk (discovery) and the biggest *cost* risk (standing infra) are removed by
  scope, not by engineering heroics.
- **−** We give up dynamic / unknown destinations (pub/sub late-binding) — broker territory, out of scope.
- **−** To not lose where incumbents win, we **must build**: a delay-based (FASP-physics) transport for
  WAN / large transfers (E3), crash-safe resume via a `bao` bitfield (E4), and an optional dumb durable store
  for temporal decoupling / offline destinations (H1).
- **Risk register:** the architecture's life rests on **H1 (ZK-completeness)** and **H3 (sealed warp speed)**
  — pre-registered and falsified first ([`../research/PRE-REGISTRATION.md`](../research/PRE-REGISTRATION.md)).

## Alternatives rejected

- **Content-addressed overlay / DHT (IPFS-style).** Rejected: dynamic discovery is the hardest subsystem and
  unnecessary once routes are fixed. Keeps content-addressing as identity only.
- **Standing broker/relay (Kafka/NATS-style).** Rejected: a standing cluster is the cost and ops burden we
  exist to remove; durability belongs at the sealed endpoints + an optional dumb store.
- **Hardware trust (TEE/enclave) for provider-blindness.** Rejected as the *primary* mechanism: HW lock-in,
  attestation complexity, and side-channels (e.g. memory-bus attacks). Cryptographic sealing needs no special
  hardware and is portable.
