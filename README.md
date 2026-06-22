# Datarail

**The serverless, zero-knowledge data rail.** When you have a package to deliver from one point to
another, an ephemeral rail spawns, carries it in a **sealed vault** over the cheapest pipe available,
confirms delivery with a cryptographic receipt, and disappears — **exactly once, with proof, and the
infrastructure never sees what it carried.**

> Service-mesh reach + Kafka durability + Signal-grade end-to-end sealing — with **no broker, no
> sidecar, and zero trust in the infrastructure.** The seal is what lets the pipe be dumb, cheap, and
> untrusted (even an S3 bucket or a peer): nobody in the middle can open the vault.

## Status

**Part I — architecture (pre-spike). Nothing here is frozen yet.** This repo currently holds the *demand*
(the trio, drafted for Owner ratification) and the *machine shape* (the Constitution). No product code is
written until the architecture's cheapest falsifier (the H3 spike) passes. Built per **THE HUGR METHOD**:
architecture-first → design-before-code → freeze rituals → fleet execution.

## Map

| Doc | What |
|---|---|
| [`docs/PRODUCT.md`](docs/PRODUCT.md) | What it is and why it exists (the moat) |
| [`docs/WORKING_BACKWARDS.md`](docs/WORKING_BACKWARDS.md) | The launch announcement, written first |
| [`docs/DECOMPOSITION.md`](docs/DECOMPOSITION.md) | Capabilities, acceptance criteria, invariants, milestones |
| [`docs/design/00-CONSTITUTION.md`](docs/design/00-CONSTITUTION.md) | The machine shape (CAST) + the named invariants + the Craft Charter |
| [`docs/adr/0001-adopt-cast.md`](docs/adr/0001-adopt-cast.md) | Why CAST: fixed-route, ephemeral, zero-knowledge |
| [`docs/research/PRE-REGISTRATION.md`](docs/research/PRE-REGISTRATION.md) | The falsifiable hypotheses H1/H2/H3 and the day-zero spike |
| [`docs/BUILD_LOG.md`](docs/BUILD_LOG.md) | Single source of truth — goal lock, decisions, running log |
