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
design-before-code → freeze rituals → fleet execution). **34 crates** · clippy
`deny(all+pedantic)` · `forbid(unsafe)` workspace-wide (one audited shared-memory waiver in the shmem
substrate). Honest evidence matrix — every load-bearing number → committed repro → rigor → verdict:
[`docs/design/LASTRO-MATRIX.md`](docs/design/LASTRO-MATRIX.md); DoD ledger:
[`docs/design/DOD-01.md`](docs/design/DOD-01.md).

What runs today: sealed cofres with per-cofre X25519 key-wrap + sealed-sender, an offline-verifiable
Merkle delivery proof, effectively-once delivery, smart terminals (content-contract + dead-letter), the
three SPEC-named substrates (**shmem · QUIC · object-store/S3**, plus TCP/UDS) behind one conformance
harness, FASP delay-based congestion control, BLAKE3-`bao` chunk-resume, a stateless DoS cookie, the
`Noise_KK` + SPAKE2 identity layer, the **v1 product flow — an HTTP API → sealed rail → Postgres**
(zero-dependency, hand-rolled Postgres driver), **Kafka wire-protocol ingest** (an unmodified Kafka producer →
sealed rail → any sink, no code change), **exactly-once delivery into Postgres across crashes** — for an
append-ordered source (`datarail run --source-file`/replay → `--sink-postgres`, `EXACTLY-ONCE-DESIGN.md`) AND for
an **idempotent Kafka producer** (kafka-ingest keys dedup on the producer's own `(producer_id, partition,
sequence)`, `KAFKA-EOS-DESIGN.md`); both land records + the dedup watermark in one atomic Postgres txn so a retry
/ replay never double-lands. HTTP (a non-stable GET) and a non-idempotent producer are honestly at-least-once.
A **bidirectional, consumer-group-capable Kafka drop-in** (`datarail kafka-broker`): unmodified producers write and
unmodified `subscribe()` consumers read back, with **durable provider-blind storage** (sealed on disk, un-sealed
only at the fetch edge, survives restart), **durable consumer offsets**, **automatic group rebalance**, and
**multi-partition** (`KAFKA-FETCH/GROUPS/REBALANCE-DESIGN.md`). And the `datarail` CLI moving real data source→sink.

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
# seal newline-delimited records from any HTTP endpoint into a Postgres table (at-least-once):
datarail run examples/rail.toml \
    --source-http https://api.example.com/events \
    --sink-postgres "host=db,user=rail,db=events,table=raw,column=data,password=secret"
# boarded → sealed cofre over the rail → COPY-landed as rows. An HTTP GET is not an append-ordered/replayable
# stream, so this path is AT-LEAST-ONCE (no silent loss; a re-run may duplicate). For EXACTLY-ONCE, use an
# append-ordered source (a file / replay), which lands records + a dedup watermark in one atomic Postgres txn:
datarail run examples/rail.toml --source-file events.ndjson \
    --sink-postgres "host=db,user=rail,db=events,table=raw,column=data,password=secret"
# Re-run it (even after appending new lines) → nothing double-lands. Verified end-to-end against real Postgres 16:
# run twice → 3 rows; append one line + re-run → 4 rows, not 7. (--at-least-once opts out.)
```

**Drop-in for Kafka producers** — point an existing producer at datarail, unchanged; it seals every record:

```sh
datarail kafka-ingest examples/rail.toml --advertised <reachable-host> \
    --sink-postgres "host=db,user=rail,db=events,table=raw,column=data"
# an UNMODIFIED Kafka producer (kcat/librdkafka/...) → datarail seals → Postgres. Verified e2e in CI.
# EXACTLY-ONCE for an IDEMPOTENT producer (enable.idempotence=true): datarail honors InitProducerId and keys
# dedup on the producer's own (producer_id, partition, sequence), stored transactionally in Postgres — a
# producer retry / ingest restart never double-lands. A non-idempotent producer is at-least-once (Kafka parity).
```

**Bidirectional drop-in** — an unmodified producer writes AND an unmodified consumer reads back, with datarail's
storage holding only **sealed** records (un-sealed only at the fetch edge — a Kafka broker whose storage is
provider-blind):

```sh
datarail kafka-broker examples/rail.toml --advertised <reachable-host> \
    --data-dir ./datarail-kafka-data --partitions 3
# ...and optionally TLS-encrypt the hop (build with `--features tls`):
#   datarail kafka-broker examples/rail.toml --advertised localhost --tls --tls-cert cert.pem --tls-key key.pem
# (a real librdkafka client with security.protocol=SSL produces+consumes over TLSv1.3 — proven in CI.)
# A full consumer-group-capable Kafka drop-in: Produce + Fetch + ListOffsets, durable consumer offsets
# (OffsetCommit/OffsetFetch/FindCoordinator), automatic group rebalance (JoinGroup/SyncGroup/Heartbeat) for
# subscribe() consumers, and multi-partition (each an independent durable log). Storage is DURABLE
# (fsync-before-ack) and provider-blind: verified e2e — produce 3 → KILL the broker → restart → fetch back the
# original plaintext, while the on-disk cofres are ciphertext; a committed offset survives a broker restart; two
# consumers auto-share one generation. PROVEN against the REAL Kafka client (kcat/librdkafka in CI,
# `kafka-broker-librdkafka.yml`): a real producer + a real simple consumer + a real subscribe() CONSUMER GROUP
# round-trip, with the on-disk store asserted ciphertext-only — not just our own wire tests. Plus a TRANSACTIONAL
# producer (buffer-until-commit: atomic multi-partition
# commit + offsets-in-txn, abort hides records, stale-epoch zombies fenced — audited, `KAFKA-TXN-DESIGN.md`; scope:
# one producer per partition/txn). Hop (1) is optionally TLS-encrypted (`--tls`, `--features tls`; server-side
# termination, proven vs real librdkafka over TLS). COMPRESSED producers work too (`--features compression`:
# gzip/lz4/zstd/snappy, decompressed broker-side + sealed — proven vs real librdkafka in CI). (Single-node; SASL +
# mTLS tracked.)
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
| Surface & proof | `datarail-spec` (`rail.toml`) · `datarail-cli` (`datarail`) · `datarail-acceptance` · `datarail-bench` |
| Connectors & compat | `datarail-connectors` (HTTP · Postgres · webhook · file) · `datarail-kafka` (Kafka wire-protocol ingest) |

## Map

| Doc | What |
|---|---|
| [`docs/PRODUCT.md`](docs/PRODUCT.md) | What it is and why it exists (the moat) |
| [`docs/WORKING_BACKWARDS.md`](docs/WORKING_BACKWARDS.md) | The launch announcement, written first |
| [`docs/DECOMPOSITION.md`](docs/DECOMPOSITION.md) | Capabilities, acceptance criteria, invariants, milestones |
| [`docs/design/00-CONSTITUTION.md`](docs/design/00-CONSTITUTION.md) | The machine shape (CAST) + the named invariants + the Craft Charter |
| [`docs/design/DOD-01.md`](docs/design/DOD-01.md) | Definition-of-Done ledger — honest PROVEN/DIRECTIONAL/PENDING per AC/gate/invariant |
| [`docs/design/KAFKA-COMPAT.md`](docs/design/KAFKA-COMPAT.md) | Kafka wire-protocol ingest — honest scope, limits, and the security boundary |
| [`docs/BUILD_LOG.md`](docs/BUILD_LOG.md) | Single source of truth — goal lock, decisions, running log |
