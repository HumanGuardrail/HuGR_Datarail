# Datarail

**The serverless, zero-knowledge data rail.** When you have a package to deliver from one point to
another, an ephemeral rail spawns, carries it in a **sealed vault** over the cheapest pipe available,
confirms delivery with a cryptographic receipt, and disappears — **exactly once, with proof, and the
infrastructure never sees what it carried.**

> Service-mesh reach + Kafka durability + Signal-grade end-to-end sealing — with **no broker, no
> sidecar, and zero trust in the infrastructure.** The seal is what lets the pipe be dumb, cheap, and
> untrusted (even an S3 bucket or a peer): nobody in the middle can open the vault.

## Status

**Engine + v1 product built, proven, and audited** (per THE HUGR METHOD: architecture-first →
design-before-code → freeze rituals → fleet execution). **33 crates · 246 tests** · clippy
`deny(all+pedantic)` · `forbid(unsafe)` workspace-wide (one audited shared-memory waiver in the shmem
substrate). Honest evidence matrix — every load-bearing number → committed repro → rigor → verdict:
[`docs/design/LASTRO-MATRIX.md`](docs/design/LASTRO-MATRIX.md); DoD ledger:
[`docs/design/DOD-01.md`](docs/design/DOD-01.md).

What runs today: sealed cofres with per-cofre X25519 key-wrap + sealed-sender, an offline-verifiable
Merkle delivery proof, effectively-once delivery, smart terminals (content-contract + dead-letter), the
three SPEC-named substrates (**shmem · QUIC · object-store/S3**, plus TCP/UDS) behind one conformance
harness, FASP delay-based congestion control, BLAKE3-`bao` chunk-resume, a stateless DoS cookie, the
`Noise_KK` + SPAKE2 identity layer, the **v1 product flow — an HTTP API → sealed rail → Postgres**
(zero-dependency, hand-rolled Postgres driver), and the `datarail` CLI moving real data source→sink.

**Measured headlines — same-ruler, committed CI harnesses, NOT asserted** (see the matrix):
**~72× less RAM** than Kafka at **equal fsync durability** (n=3); **~2 ms cold-start** vs Kafka's **~5 s**
(n=30 controlled CI) → scale-to-zero; raw throughput is an **honest TIE** (datarail's *sealed* engine ≈
Kafka's *plaintext*). The moat is **efficiency + structural** (provider-blind, serverless, exactly-once
with proof), not raw speed — and every claim carries its real confidence label, because a long adversarial
audit of our own benchmarks corrected every over-claim that appeared.

## The doctrine

> **Smart sealed endpoints. Dumb cheap pipes.**

All intelligence and all secrets live in a featherweight terminal at the container's edge. The rail is the
dumbest, cheapest substrate available. Because every vault is sealed end-to-end, the pipe can be untrusted
— including ours — and a breach yields useless ciphertext.

## Try it

**The v1 product — an HTTP API → Postgres, sealed end-to-end, zero-dependency** (even the Postgres driver
is hand-rolled and provider-blind — the pipe and the database host never see plaintext):

```sh
# seal newline-delimited records from any HTTP endpoint into a Postgres table:
datarail run examples/rail.toml \
    --source-http https://api.example.com/events \
    --sink-postgres "host=db,user=rail,db=events,table=raw,column=data,password=secret"
# boarded → sealed cofre over the rail → COPY-landed as rows. Verified end-to-end against real Postgres 16.
```

Lower-level rehearsals (files, two-process TCP, identity pairing):

```sh
# (toolchain note: run cargo from the stable toolchain on PATH if the rustup proxy is unavailable)
cargo run -p datarail-cli -- validate examples/rail.toml          # check a route spec
cargo run -p datarail-cli -- keygen                               # mint an Ed25519 signing identity
printf 'evt:a\nevt:b\nevt:c\n' > /tmp/in.txt
cargo run -p datarail-cli -- run examples/rail.toml \
    --source-file /tmp/in.txt --sink-file /tmp/out.txt --watch    # board → sealed rail → offload, live
cargo run -p datarail-cli -- replay examples/rail.toml 1..3 --source-file /tmp/in.txt
cargo run -p datarail-cli -- pair                                 # F2/F3 identity layer, local rehearsal
# real two-process F3 pairing over a short code (two terminals): --listen on one, --connect on the other:
datarail pair --listen 127.0.0.1:7000 --code 0x<shared-16-byte-code>
datarail pair --connect 127.0.0.1:7000 --code 0x<shared-16-byte-code>

# genuine TWO-PROCESS transfer over a real TCP socket (run in two terminals / hosts):
datarail recv examples/rail.toml --listen 127.0.0.1:9000 --sink-file /tmp/out.txt --count 1   # destination
datarail send examples/rail.toml --connect 127.0.0.1:9000 evt:hello evt:world                 # source

# ...and optionally wrap that hop in a Noise_KK channel (mutual auth + on-wire metadata encryption, F2):
datarail keygen --noise                                                # mint each endpoint's static keypair
datarail recv examples/rail.toml --noise-secret 0x<B-sec> --peer-public 0x<A-pub> --sink-file /tmp/out.txt
datarail send examples/rail.toml --connect <addr> --noise-secret 0x<A-sec> --peer-public 0x<B-pub> evt:hi
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
