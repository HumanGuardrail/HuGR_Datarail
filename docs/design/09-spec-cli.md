# 09 — Spec & CLI Surface

> **MF-2 SPEC. Status: DRAFT.** Capability G. The plug-&-play surface: one declarative file + one binary.
> This is the **policy** surface (the user's), atop our **mechanism** (terminals/cofre/rail).

## The rail spec (`rail.toml`)

One declarative file describes a route end-to-end:

```toml
[route]
id        = "orders-pg-to-lake"
substrate = "auto"          # auto | shmem | quic | s3://bucket/prefix
guarantee = "effectively-once"

[source]                    # the onboarding terminal
connector = "postgres-cdc"
dsn_ref   = "env:SRC_DSN"
[source.contract]           # onboarding RULES (policy) — content contract, enforced before sealing
schema  = "schemas/orders.v3.json"
require = ["id", "ts", "amount"]

[dest]                      # the offloading terminal
connector = "s3-parquet"
uri       = "s3://lake/orders/"
[dest.contract]             # offloading RULES — enforced after unsealing, before commit
on_schema_drift = "dead-letter"   # never silent corruption
upsert_key      = "id"

[keys]
identity_ref = "kms:datarail/orders/identity"   # held by the user; never inline plaintext
```

The spec carries **no secrets** — only references (env/KMS). Keys stay with the user.

## The CLI (`datarail`, one static binary)

| Command | Does |
|---|---|
| `datarail run <spec>` | start the route — featherweight, scale-to-zero |
| `datarail validate <spec>` | check spec + contract + connectivity, no data moved |
| `datarail keygen` | emit an endpoint identity (Ed25519 + X25519) |
| `datarail ticket <route>` | emit a signed route descriptor (08) |
| `datarail verify <manifest> <proof>` | verify a delivery proof **offline** (04) — for auditors |
| `datarail replay <route> <range>` | re-ship a range from the manifest/WAL (time-travel) |

## The speedometer (`--watch`)

A live TUI: throughput, p50/p99 latency, in-flight, dead-lettered, manifest progress. Makes warp speed
**visible** (the demo wow). Optional; the daemon is headless by default.
