# ROADMAP — the systems-integration phase (TechLead wave plan)

The Kafka-waste ledger is attacked at the **component** level (8/9 PROVEN, 1 DIRECTIONAL) and the components
**compose** (INTEGRATION-01). What remains is *systems integration* — durable offsets, replicated durability,
a cold tier, a network protocol — plus hardening (audits, the object-store flaw, FASP throughput). This is the
frozen plan: disjoint work packages, frozen contracts, a 10-agent wave.

## TechLead WAVE PLAN — integration-wave-01 @ main — ✅ LANDED (10/10 WPs, lead-verified)
```
GO/NO-GO     : PARALLEL (10 agents)
DISJOINTNESS : shared file (workspace Cargo.toml members) eliminated — the LEAD pre-added all 4 new members + the
               4 new-crate scaffolds (frozen contracts) so the baseline is GREEN before dispatch. Agents own
               disjoint files ONLY; agents do NOT git-commit and do NOT edit outside their crate.
CONTRACTS    : FROZEN by the lead in the scaffolds — `datarail_blobstore::BlobStore` (+ `MemBlob` ref) and
               `datarail_offsets::OffsetStore` (+ `MemOffsets` ref) so dependents compile + test immediately;
               `datarail_broker::{Request,Response}` shapes; `datarail_replication::ErasureStore` API.
```

| WP | Owns (disjoint) | Depends on (frozen) | Deliverable |
|---|---|---|---|
| **WP1** offsets | `crates/datarail-offsets/**` | std | `FileOffsets`: crash-safe, fsync'd durable `OffsetStore` + recovery gate |
| **WP2** blobstore | `crates/datarail-blobstore/**` | std | `FsBlob`: local-filesystem `BlobStore` + conformance gates vs `MemBlob` |
| **WP3** replication | `crates/datarail-replication/**` | erasure, blobstore (trait) | `ErasureStore`: shard k+m over a `BlobStore`, reconstruct after losing any `m` shards (gate vs `MemBlob`) |
| **WP4** broker | `crates/datarail-broker/**` | topic, offsets (trait), rail | wire codec + in-process server/client: produce/poll/commit round-trip gate |
| **WP5** objectstore-fix | `crates/datarail-substrate-objectstore/src/lib.rs` | — | replace the unbounded `HashSet` with an O(1) cursor (the audit's flaw); RAM-flat gate |
| **WP6** fasp-throughput | `crates/datarail-rail/src/reliable_udp.rs` | — | batch the send path (amortize syscalls) without breaking S1/S2/S4 gates; keep exactly-once |
| **WP7** audit-erasure | READ-ONLY `datarail-erasure` | — | adversarial findings card (GF math, Cauchy MDS, edge params) |
| **WP8** audit-replaylog | READ-ONLY `datarail-replaylog` | — | adversarial findings card (recovery, offset math, flat-RAM, torn tail) |
| **WP9** audit-keyrouter | READ-ONLY `datarail-keyrouter` | — | adversarial findings card (HRW correctness, distribution, monotonicity) |
| **WP10** audit-topic | READ-ONLY `datarail-topic` | — | adversarial findings card (framing, dispatch cursor, group isolation) |

```
CONFLICT-MAP : WP1-4 = 4 new disjoint crates. WP5/WP6 = one isolated existing file each (objectstore lib /
               reliable_udp). WP7-10 = READ-ONLY (no edits) → zero write-conflict. All pairwise CONFLICT-FREE.
RETURN-SHAPE : "WP<n>: <DONE|BLOCKED> | files | `cargo test -p <crate>` result | clippy clean Y/N | 1-line note"
DOD (V1)     : forbid(unsafe), clippy deny(all+pedantic) NO #[allow], zero EXTERNAL deps, gates green, claims
               labelled, honest scope. The LEAD cold-verifies every WP (clippy + test) before integration.
MERGE ORDER  : seams (WP1,WP2) → dependents (WP3,WP4) conceptually; but all build against frozen contracts so
               order is immaterial; the lead runs one authoritative `cargo test --workspace` at integration.
```

## After this wave (future waves, not in scope here)
- Wire `datarail-replication` durability into `datarail-topic` (replicated topic).
- Point `datarail-replaylog`/`blobstore` cold reads at real S3; measure RAM-flat replay from S3.
- A real-WAN FASP field number (multi-hop, real infra).
- Optional Kafka wire-protocol compatibility (inherit the ecosystem).
