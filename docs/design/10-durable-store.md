# 10 — Optional Durable Store

> **MF-2 SPEC. Status: DRAFT.** Capability H. A dumb commodity object store as an *optional* holding pen —
> for temporal decoupling (destination offline) or fan-out buffering. **Not infrastructure you operate.**

## What it is

Commodity object storage (S3 / GCS / R2 / MinIO) that stores **only sealed cofres**. The store operator
sees **ciphertext** — provider-blind holds (a breach of the bucket yields useless bytes). This is the
`s3` substrate of the rail (07), used when source and destination are not both online at once.

## Layout & lifecycle

```
<bucket>/<route_id>/<stream_id>/<seq>.cofre          # one sealed cofre per object
<bucket>/<route_id>/manifest/<sth_epoch>.sth         # mirrored signed tree heads (04), optional
```

- **Write:** the source terminal PUTs the sealed cofre (idempotent: `seq` is the key → re-PUT is a no-op).
- **Read:** the destination terminal GETs in `seq` order, verifies seal, offloads.
- **GC:** delete a cofre **after** the destination acks **and** the checkpoint advances past its `seq` (05).
  A retention floor guards replay (09).

## Why it's the "custo absurdo" lever

No broker cluster to run, patch, or scale — just pay-per-GB-stored + requests, at object-store prices
(orders of magnitude under a standing Kafka/MFT server). Idle cost ≈ storage of un-drained cofres only;
direct routes (shmem/QUIC, both online) use **no store at all**.

## Scope

Optional and dumb by design. It never decrypts, never validates content, never orders — those are the
terminals' and the rail-protocol's jobs. It is a passive, provider-blind shelf.
