# OMB-PROTOCOL — frozen seam for the datarail OpenMessaging-Benchmark integration

> Goal: let the **real OMB harness** drive datarail through the SAME `BenchmarkDriver` interface it uses for
> Kafka/Pulsar/RabbitMQ, so the comparison is the industry-standard one (not a self-run hack). Two components,
> built against THIS frozen contract: a **Rust shim** (`crates/datarail-omb-shim`, a mini-broker on datarail's
> real terminals) and a **Java driver** (`bench/omb/driver-datarail`, implements OMB's `BenchmarkDriver`).

## OMB interface the Java driver must satisfy (read from driver-api, verbatim)

- `BenchmarkDriver`: `initialize(File config, StatsLogger)`, `getTopicNamePrefix()`,
  `createTopic(topic, partitions): CF<Void>`, `createProducer(topic): CF<BenchmarkProducer>`,
  `createConsumer(topic, subscription, ConsumerCallback): CF<BenchmarkConsumer>`, `close()`.
- `BenchmarkProducer`: `sendAsync(Optional<String> key, byte[] payload): CF<Void>` — the future completes when
  the shim has **acked** the message (≈ acks=1: boarded + handed to the substrate).
- `ConsumerCallback`: `messageReceived(byte[] payload, long publishTimestamp)` — called once per delivered
  message; `publishTimestamp` = epoch millis when the PRODUCER called sendAsync (full E2E, incl. client hop).

## Wire protocol — Java driver ↔ Rust shim (localhost TCP, big-endian)

The shim listens on **two TCP ports** (configurable; defaults below):
- **INGRESS** (producers connect) — default `127.0.0.1:7701`
- **EGRESS** (consumers connect) — default `127.0.0.1:7702`

### Ingress (producer → shim)
1. On connect, producer sends a header once: `[u16 topic_len][topic_utf8]`.
2. Then a stream of message frames: `[u32 payload_len][u64 publish_ts_millis][payload_bytes]`
   (`payload_len` counts ONLY `payload_bytes`, not the 8 ts bytes).
3. The shim replies on the same socket with an **ack stream**: `[u64 seq]` (monotonic from 0), one ack per
   accepted message, in receive order. The driver completes the i-th outstanding `sendAsync` future on ack `i`.
   (Pipelined: many in flight; the producer need not block per message.)

### Egress (consumer → shim)
1. On connect, consumer sends a header once: `[u16 topic_len][topic_utf8][u16 sub_len][sub_utf8]`.
2. The shim then STREAMS delivered frames to the consumer: `[u32 payload_len][u64 publish_ts_millis][payload]`.
   The driver calls `messageReceived(payload, publish_ts_millis)` for each.
3. Fan-out: every consumer on `(topic)` receives every message published to that topic (OMB subscriptions are
   independent consumer groups; for v1 each distinct `sub` is its own fan-out copy — model subscriptions as
   independent delivery, matching OMB's "each subscription gets all messages").

### createTopic
No control message needed — topics are **lazy**: the shim creates a topic's internal state on first
producer/consumer reference. The Java `createTopic(...)` returns an already-completed future.

## Shim internals (the part that makes it a REAL datarail measurement, not a passthrough)

For each topic the shim runs datarail's **real sealed datapath** — it MUST NOT shortcut crypto:
1. Ingested `{publish_ts || payload}` is the **record** (the ts travels as opaque record bytes).
2. **Board** records into a cofre via a real `SourceTerminal` (`board(&records, key)`), **batched** to amortize
   the per-cofre X25519 wrap (SPEC-mandated): flush a cofre when EITHER `batch_max_records` (default 128) OR
   `batch_max_micros` (default 1000 µs) is hit, whichever first. (Batching boosts throughput; the ≤1 ms fill
   adds a small, honest latency — documented, configurable.)
3. Move the cofre to the dest. Single-node default: an in-process handoff into a real `DestTerminal`
   (exercises encode→decode→verify→open→admit→commit — the full seal/open, signature, key-unwrap). Optionally
   a real `TcpSubstrate`/`ShmemRing` hop via config (`substrate = "loopback"|"tcp"|"shmem"`, default loopback).
4. **Offload** → records → split back into `{publish_ts, payload}` → deliver `payload` + `publish_ts` to every
   egress consumer subscribed to the topic.

Terminal API anchor: see `crates/datarail-bench/src/main.rs` `head_to_head` for the exact
`SourceTerminal::new(cfg, contract, src_seed).board(&recs, key)` /
`DestTerminal::new(cfg, contract, src_vk, [22;32], dest_secret).offload(&cofre)` usage + `TerminalConfig` /
`ContentContract::new(max_record, prefix)` construction. The contract prefix must admit arbitrary payloads
(OMB sends random bytes) — use a **zero-length prefix** + a `max_record` ≥ workload message size + 8 (ts).

## Shim config (TOML, path passed as argv[1]; all optional with the defaults above)
```toml
ingress_addr = "127.0.0.1:7701"
egress_addr  = "127.0.0.1:7702"
substrate    = "loopback"      # loopback | tcp | shmem
batch_max_records = 128
batch_max_micros  = 1000
max_record_bytes  = 1048576    # cap a frame; reject larger (parse-safety)
```

## OMB driver config (`bench/omb/datarail.yaml`, read by `initialize`)
```yaml
name: datarail
driverClass: io.openmessaging.benchmark.driver.datarail.DatarailBenchmarkDriver
ingressAddr: "127.0.0.1:7701"
egressAddr:  "127.0.0.1:7702"
```

## Honesty labels (carry into the report)
- Single-node, broker+client colocated, shim topology — **fair same-HW relative**, not lab-grade multi-node.
- datarail runs its FULL sealed path (provider-blind) here; the brokers run **plaintext** (their OMB default).
- Batching (≤128 rec / ≤1 ms) is datarail's intended mode; stated in results.
- The shim is a benchmark adapter, NOT a product component — it maps OMB's pub/sub onto datarail's
  point-to-point send/recv (a fair-but-partial fit; datarail has no native topic/partition/consumer-group).
