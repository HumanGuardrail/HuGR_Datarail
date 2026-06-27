# KAFKA-COMPAT — the Kafka wire-protocol ingest bridge (scope, limits, security posture)

> **The pitch:** point an existing, **unmodified** Kafka producer at datarail; every record it sends is sealed
> into a provider-blind cofre and landed via any datarail `Sink`. *Your Kafka producers, now provider-blind +
> serverless, no code change.* This doc is the HONEST scope: exactly what is implemented, what is not, and the
> precise security boundary — so no one over-claims.

## What is implemented (`datarail-kafka`, zero-dependency)
datarail presents itself as a **single-broker, single-partition-per-topic** Kafka-compatible **ingest** endpoint.

| API | key | versions | notes |
|---|---|---|---|
| `ApiVersions` | 18 | v0–v3 | responds in the client's version (flexible body for v3; response header always v0 per KIP-482) |
| `Metadata` | 3 | v0–v1 | advertises this process as broker `node 0` (`--advertised HOST:port`), the leader of every requested topic's single partition |
| `Produce` | 0 | v0–v7 | parses the request envelope + the records blob (incl. the idempotent producer's `producer_id`/`base_sequence`) |
| `InitProducerId` | 22 | v0–v1 | grants a `producer_id` so a client can `enable.idempotence=true` → exactly-once ingest (`KAFKA-EOS-DESIGN.md`) |

**Record formats:** both the modern **v2 `RecordBatch`** AND the legacy **v0/v1 `MessageSet`** are parsed
(librdkafka emits legacy magic-0 on a fallback path — this was caught by live-testing against real `kcat`, not
assumed). **Uncompressed only.**

**Flow:** `datarail kafka-ingest <rail.toml> --advertised HOST --sink-…` runs the endpoint; each produced batch's
record values stream into the normal rail (`board` → **seal** → substrate → `offload` → `Sink`). Produced offsets
are tracked per `(topic, partition)` and returned in the Produce response.

**Delivery guarantee:** **exactly-once** into a transactional sink (Postgres) for an **idempotent producer**
(`enable.idempotence=true`) — datarail honors `InitProducerId` and keys dedup on the producer's stable
`(producer_id, partition, sequence)`, stored transactionally in the sink, so a producer retry / ingest restart
never double-lands (`KAFKA-EOS-DESIGN.md`). A **non-idempotent** producer is **at-least-once** (Kafka's own
default). Honest scope: exactly-once is *within an idempotent producer session*; cross-session (transactional)
EOS is a further increment.

**Verified:** `kafka-ingest.yml` (CI) runs a real `kcat`/librdkafka producer → datarail → sealed rail → a Postgres
service container, asserting the sealed rows land; plus the EOS gate — a duplicated idempotent `Produce` through
the real binary lands exactly once (3 rows, not 6). Continuously gated, not a one-off (`LASTRO-MATRIX`).

## What is NOT implemented (honest limits — do not claim these)
- **No consumer side (`Fetch`, `ListOffsets`, group coordination).** This is **produce-ingest only** — a one-way
  bridge INTO datarail. Reading back via the Kafka protocol is the next arc.
- **No compression** (gzip/snappy/lz4/zstd). A compressed batch is rejected with a clear error. Producers must
  send uncompressed (`compression.type=none`) for now.
- **No TRANSACTIONAL producer** (the `AddPartitionsToTxn` / `EndTxn` / transaction-coordinator APIs, a stable
  `transactional.id`). The **idempotent** producer (`InitProducerId` + `enable.idempotence=true`) IS supported and
  gives exactly-once within a producer session; cross-session/transactional EOS is the next increment.
- **No SASL / TLS on the Kafka hop** (see security posture below).
- **Single partition per topic, single broker.** No real partitioning/replication on the Kafka-facing side — the
  durability/replication is datarail's own (the rail + substrate), not Kafka-style partition replicas.

## Security posture — READ THIS, it is the boundary that must not be over-claimed
datarail's core guarantee is **provider-blind**: the rail, the cheap/untrusted pipe, and the storage/sink-side
infrastructure never see plaintext. With Kafka ingest, the trust boundary is:

```
[Kafka producer] --(1) plaintext Kafka wire-->  [datarail kafka-ingest]  --(2) SEALED cofre-->  [rail → storage → sink]
```

- **Hop (2) — the rail and everything downstream — is sealed and provider-blind.** This is the moat and it holds:
  the cheap pipe, any intermediate, and the storage provider see only ciphertext.
- **Hop (1) — the producer → datarail link — is currently PLAINTEXT** (no TLS/SASL on the Kafka endpoint yet).
  So the Kafka ingest is **NOT end-to-end sealed from the producer**; it is "seal-on-ingest." Use it when the
  producer→datarail hop is on a trusted network (same datacenter / mTLS mesh / loopback sidecar), and rely on
  datarail for provider-blindness DOWNSTREAM (the part that is usually the untrusted, multi-tenant, cost-bearing
  infrastructure). For a fully end-to-end-sealed source, use datarail's native sealed terminals, not Kafka ingest.
- **We state this plainly rather than implying "end-to-end sealed from your Kafka producer," which would be false
  until hop (1) gets TLS.**

## Roadmap (next arcs, in rough value order)
1. **`Fetch` consumer side** → datarail becomes a full Kafka drop-in (produce AND consume), un-sealing on read for
   authorized consumers.
2. **TLS on the Kafka hop** → closes hop (1), making Kafka ingest end-to-end sealed.
3. **Compression** (at least the common codecs) for throughput parity.
4. ✅ **Idempotent producer (`InitProducerId`) → exactly-once from idempotent producers — DONE** (2026-06-27,
   `KAFKA-EOS-DESIGN.md`). Next within this line: **transactional** producer (cross-session EOS via a stable
   `transactional.id`).

## Why this is still a big deal even produce-only + seal-on-ingest
The expensive, untrusted, always-on part of a Kafka deployment is the **broker cluster + its storage**. datarail
replaces exactly that with a serverless, provider-blind, ~70×-leaner rail — while the producer keeps its existing
Kafka client. The first hop being trusted-network is the same assumption most in-datacenter Kafka deployments
already make (PLAINTEXT or mTLS between app and broker); datarail adds provider-blindness for everything after.
