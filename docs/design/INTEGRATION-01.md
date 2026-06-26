# INTEGRATION-01 — the proven components compose into Kafka's core behaviour

> After attacking the whole Kafka-waste ledger at the *component* level (each codec/mechanism built + gated +
> measured), the open question was: **do the pieces compose into the actual thing Kafka does?** This is the first
> integration slice that answers yes — `datarail-topic` wires two independently-proven crates into the core
> Kafka behaviour, built the leaner way.

## What composes
| Component (proven separately) | Role in the topic |
|---|---|
| `datarail-replaylog` (retained durable log, replay-from-offset, **flat RAM**) | the topic's durable, re-readable history |
| `datarail-keyrouter` (rendezvous routing, **no partitions, monotonic**) | per-group, per-key consumer assignment |

`datarail-topic` adds the thin glue: `produce(key, payload)`, consumer `Group`s each with their **own** offset, and
`dispatch_next` (route each record's key to its owning member, advancing the group's cursor).

## What the 4 integration gates prove (all green, `forbid(unsafe)`, clippy `deny(all+pedantic)`)
1. **Durable multi-consumer log** — two `Group`s replay the *same* 3000-record history *independently* (own
   offsets); both see everything because history is retained. (Kafka's defining feature.)
2. **In-group distribution + per-key order** — 4000 records over 4 members: every key lands on exactly one member,
   the union covers all records, per-key payloads stay in produce order, and all 4 members get work (real
   parallelism, no idle partitions, no partition-count config).
3. **No stop-the-world rebalance** — a 4th member joins mid-stream: the 1000 already-dispatched records are
   untouched (the new member never appears retroactively), only later records can use it, nothing is lost.
4. **Replay / rewind** — a group `seek`s back to the 600th offset and re-dispatches the retained tail (Kafka's
   reprocess-from-offset).

## Honest scope (integration *milestone*, not a finished broker)
- **In-memory group offsets** — durable offset commit (Kafka's `__consumer_offsets`) is future.
- **Single-node, no wire protocol** — this is the in-process *mechanism*; a network/broker protocol (and optional
  Kafka wire-compatibility) is future.
- **Cold tier = local filesystem** — pointing the log's segment reads at S3/object-store is the same reader with
  the GET verb swapped, future.
- **No cross-node replication / shard placement** — the erasure codec (`datarail-erasure`) and replication layer
  are not yet wired into the topic; durability here is single-node fsync.

## Why it matters
It closes the gap between "we proved each Kafka-waste fix in isolation" and "the fixes actually compose into the
behaviour Kafka exists to provide." A durable multi-consumer log with consumer groups, dynamic parallelism,
rewindable replay, and **no stop-the-world rebalance** — assembled from a flat-RAM retained log and monotonic
rendezvous routing — runs and is gated. The remaining work is *systems* integration (durable offsets, network
protocol, cross-node replication), not new mechanisms.
