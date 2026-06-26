# AUDIT-WAVE-02 — wave-02 audits + dispositions

5 implementation WPs + 2 read-only audits. The audits caught a **real CRITICAL** in the wave-01 objectstore fix.
All findings lead-cold-verified; every real one fixed at the root (lead applied the fixes).

## Implementation WPs (5) — all DONE, lead-cold-verified (`clippy --workspace` clean, tests green)
- **WP1 `datarail-broker` TCP** — real `serve()` accept loop + `Client` over `TcpStream`; e2e produce 100 / poll / commit over a loopback socket (5 gates). The broker now works over a real network.
- **WP2 `datarail-replicated-topic`** — composes `Topic` + `ErasureStore`: each record appended to the log AND erasure-sharded; reconstruct after losing any `m` shards (all 15 double-loss subsets, 3 gates).
- **WP3 `datarail-netblob`** — a `BlobStore` served over TCP + a `NetBlob` client (the cold tier, made remote); put/get/list/delete + two-client round-trips over loopback (2 gates).
- **WP4 `datarail-replaylog` resync** — closed the documented limitation: a CRC failure in a non-final segment now RESYNCS to the next segment instead of ending replay (mid-segment corruption gate; final-tail truncation unchanged).
- **WP5 `datarail-offsets` compaction** — the append-only commit log now compacts (temp+fsync+rename) at ≥1024 records & >4× live groups, so on-disk size is O(groups) not O(commits), crash-safe across compaction (8 gates).

## Audits (2) + dispositions

| WP | Target | Verdict | Findings → disposition |
|---|---|---|---|
| **WP6** | `datarail-blobstore` (`FsBlob`) | **traversal/collision SOUND** | Verified: percent-encoding is injective, no escape (`..`/`/`/NUL/newline), no key↔temp collision. **Fixed:** [MED] empty-key `""` mapped to the root dir → now prefixed with a constant `B` marker (never empty, never `.`-leading); [LOW] not power-durable → added `fsync` of the temp file before rename + dir fsync after (now power-durable, matching the codebase bar). [LOW] temp-leak-on-crash + over-long-key `ENAMETOOLONG` divergence noted (both benign; cleanup sweep is future). |
| **WP7** | `datarail-substrate-objectstore` (the wave-01 cursor) | **1 CRITICAL + 2 HIGH — REAL** | The wave-01 "fix" had its own bugs. **[CRITICAL C1] `cursors` was insert-only, never GC'd** → O(every stream ever seen), the *same leak class* it claimed to kill, and the "O(live streams)" doc was dishonest. **[CRITICAL C2]** a drained stream's stale cursor silently discarded a reused stream-id's low seqs (loss). **Fixed:** GC the cursor (and the empty dir) when a stream's last object is acked → genuinely O(LIVE streams) + reuse starts fresh; the two bound-gates updated to assert bookkeeping returns to **0** after full drain (proving the reclaim). **[HIGH H1]** one non-`seq` filename poisoned the whole `recv` (a one-file DoS by a hostile storage operator) → **fixed:** skip a non-`seq` object instead of erroring. **[HIGH C3]** the high-watermark cursor assumes in-order arrival → **documented** as a precondition (true for the in-order rail). The dishonest "O(live streams)" claim is now actually true. |

## Method note
The disjoint-files + frozen-contract discipline held: 5 impl agents (disjoint crates/files) + 2 read-only audits;
2 new crates scaffolded with frozen contracts + members pre-added by the lead. The lead applied every audit fix
(judgment is not delegated) and ran the authoritative `cargo clippy --workspace --all-targets` + per-crate tests
(incl. the ~115 s fsync-heavy offsets compaction gate) before integrating. The WP7 CRITICAL is exactly why we
audit our own freshly-written "fixes" — a leak re-introduced under a new name, caught before it shipped.
