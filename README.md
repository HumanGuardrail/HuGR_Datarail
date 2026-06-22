# Datarail

**The serverless, zero-knowledge data rail.** When you have a package to deliver from one point to
another, an ephemeral rail spawns, carries it in a **sealed vault** over the cheapest pipe available,
confirms delivery with a cryptographic receipt, and disappears — **exactly once, with proof, and the
infrastructure never sees what it carried.**

> Service-mesh reach + Kafka durability + Signal-grade end-to-end sealing — with **no broker, no
> sidecar, and zero trust in the infrastructure.** The seal is what lets the pipe be dumb, cheap, and
> untrusted (even an S3 bucket or a peer): nobody in the middle can open the vault.

## Status

**Engine + product surface built, proven, and audited** (per THE HUGR METHOD: architecture-first →
design-before-code → freeze rituals → fleet execution). 16 crates · clippy `deny(all+pedantic)` ·
`forbid(unsafe)` workspace-wide (one audited shared-memory waiver in the shmem substrate). Honest ledger
of every Acceptance Criterion / gate / invariant: [`docs/design/DOD-01.md`](docs/design/DOD-01.md).

What runs today: sealed cofres with per-cofre X25519 key-wrap + sealed-sender, an offline-verifiable
Merkle delivery proof, effectively-once delivery, smart terminals (content-contract + dead-letter), the
three SPEC-named substrates (**shmem · QUIC · object-store/S3**, plus TCP/UDS) behind one conformance
harness, FASP delay-based congestion control, BLAKE3-`bao` chunk-resume, a stateless DoS cookie, the
`Noise_KK` + SPAKE2 identity layer, and the `datarail` CLI moving real data source→sink.

**Pending only on external physical resources (not code):** the `GATE-WARP` throughput *measurement*
needs representative x86-VAES hardware; the AC-10 fairness *bake-off* needs the real competitor engines
(Kafka/MFT/Fivetran). The rig, the bound, and everything provable on this box are done.

## The doctrine

> **Smart sealed endpoints. Dumb cheap pipes.**

All intelligence and all secrets live in a featherweight terminal at the container's edge. The rail is the
dumbest, cheapest substrate available. Because every vault is sealed end-to-end, the pipe can be untrusted
— including ours — and a breach yields useless ciphertext.

## Try it

```sh
# (toolchain note: run cargo from the stable toolchain on PATH if the rustup proxy is unavailable)
cargo run -p datarail-cli -- validate examples/rail.toml          # check a route spec
cargo run -p datarail-cli -- keygen                               # mint an Ed25519 signing identity
printf 'evt:a\nevt:b\nevt:c\n' > /tmp/in.txt
cargo run -p datarail-cli -- run examples/rail.toml \
    --source-file /tmp/in.txt --sink-file /tmp/out.txt --watch    # board → sealed rail → offload, live
cargo run -p datarail-cli -- replay examples/rail.toml 1..3 --source-file /tmp/in.txt
cargo run -p datarail-cli -- pair                                 # F2/F3 identity layer, local rehearsal
```

A route is one declarative `rail.toml` (source + onboarding rules + destination + offloading rules +
`substrate` + keys-ref). See [`examples/rail.toml`](examples/rail.toml). The `substrate` field selects the
real transport: `auto`/`loopback` · `tcp` · `shmem` · `s3` · `quic` (the last behind `--features quic`).

## Crates

| Layer | Crates |
|---|---|
| Core seam | `datarail-core` (types/traits) · `datarail-crypto` · `datarail-cofre` (wire + seal/verify) |
| Proof & once | `datarail-manifest` (Merkle proof + `bao` chunk-resume) · `datarail-once` (effectively-once) |
| Rail | `datarail-rail` (substrate trait + loopback/resumable/UDS/TCP + WAN harness + FASP CC + DoS cookie) · `datarail-substrate-{shmem,quic,objectstore}` |
| Terminals & identity | `datarail-terminal` (contract/seal/dead-letter/sealed-sender) · `datarail-identity` (Noise_KK + SPAKE2) · `datarail-connectors` |
| Surface & proof | `datarail-spec` (`rail.toml`) · `datarail-cli` (`datarail`) · `datarail-acceptance` · `datarail-bench` (incl. the AC-10 fairness rig) |

## Map

| Doc | What |
|---|---|
| [`docs/PRODUCT.md`](docs/PRODUCT.md) | What it is and why it exists (the moat) |
| [`docs/WORKING_BACKWARDS.md`](docs/WORKING_BACKWARDS.md) | The launch announcement, written first |
| [`docs/DECOMPOSITION.md`](docs/DECOMPOSITION.md) | Capabilities, acceptance criteria, invariants, milestones |
| [`docs/design/00-CONSTITUTION.md`](docs/design/00-CONSTITUTION.md) | The machine shape (CAST) + the named invariants + the Craft Charter |
| [`docs/design/DOD-01.md`](docs/design/DOD-01.md) | Definition-of-Done ledger — honest PROVEN/DIRECTIONAL/PENDING per AC/gate/invariant |
| [`docs/BUILD_LOG.md`](docs/BUILD_LOG.md) | Single source of truth — goal lock, decisions, running log |
