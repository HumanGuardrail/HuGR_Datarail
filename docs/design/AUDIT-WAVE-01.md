# AUDIT-WAVE-01 — the 4 read-only audits of integration-wave-01, and their dispositions

Part of the 10-agent wave: 6 implementation WPs + 4 read-only adversarial audits of the previously-built crates.
The lead cold-verified every finding against source before acting. Net: **no CRITICAL bugs; one HIGH (real
data-loss path) fixed; several LOW/MED hardenings applied; two limitations documented honestly.**

| WP | Target | Verdict | Real findings → disposition |
|---|---|---|---|
| **WP7** | `datarail-erasure` | **SOUND** | GF math, Cauchy-MDS, Gauss-Jordan, reconstruct-from-mixed-survivors all verified correct for *all* valid (k,m), not just the gated ones. Only a latent footgun: `gf_inv(0)=1` (unreachable). **Fix:** `debug_assert!(a != 0)` documents the contract. `EXP[255]=0` is never indexed — left a note. |
| **WP8** | `datarail-replaylog` | **No CRITICAL** | Offset math, no-straddling-frames, `scan_valid_len` panic-safety, parse safety, retention — all verified sound. **Fixed:** [LOW] reader could emit a record past the snapshot `end` → added an `offset >= end` guard. **Documented honestly (not silently shipped):** (a) a mis-aligned in-range `start_offset` is an unvalidated caller precondition (yields empty replay, not `BadOffset`); (b) a CRC failure in a *non-final* segment ends replay there (corruption detected, never wrong bytes, but later intact segments unreachable until repair) — a resyncing reader is tracked as future work. |
| **WP9** | `datarail-keyrouter` | **SOUND** | The crux (HRW weight independence) verified; monotonicity (#6) shown to hold *structurally* for any weight/ids (argmax property), not just empirically; `mix64` bijective ⇒ ties between distinct ids are impossible; distribution unbiased even for sequential small ids. Only a cosmetic, unreachable tie-break/doc inconsistency. **Fix:** aligned `route_keyhash`'s tie direction to the doc + `rank` (smaller id wins) so the `rank[0]==route` invariant holds even under hypothetical duplicate-id misuse. |
| **WP10** | `datarail-topic` | **1 HIGH** | The prime suspect — the dispatch cursor arithmetic `offset + 8 + rec.len()` — was verified to **exactly** match the replaylog frame layout (`FRAME_OVERHEAD=8`), no desync. But [HIGH]: `dispatch_next` advanced the cursor/reader **before** the fallible `route()`, so a group drained to zero members would consume-then-error → silent loss on retry. **Fix:** fail with a new `TopicError::NoMembers` **before** consuming any record; added a regression gate (`dispatch_to_an_empty_group_consumes_nothing_then_recovers`). |

## Implementation WPs (6) — all DONE, lead-cold-verified (clippy `--workspace` clean, tests green)
- **WP1 `datarail-offsets`** — `FileOffsets`: append-only CRC'd fsync'd commit log, last-write-per-group recovery (5 gates).
- **WP2 `datarail-blobstore`** — `FsBlob`: crash-atomic put (temp+rename), path-traversal-proof via percent-encoded flat filenames (6 gates, incl. a `../evil` escape test).
- **WP3 `datarail-replication`** — `ErasureStore`: k+m shards over a `BlobStore`, length-header padding, reconstructs after losing any m shards, `Ok(None)` (never wrong bytes) if <k survive (8 gates).
- **WP4 `datarail-broker`** — wire codec (`[len][body][crc]`, never-panic reader) + `Server<O: OffsetStore>` wiring produce/poll/commit (4 gates).
- **WP5 `datarail-substrate-objectstore`** — replaced the unbounded `HashSet` (the prior audit's flaw) with an O(live-streams) forward cursor + O(in-flight) ack map; `bookkeeping_len()` bound-gates + a Linux RSS flat gate (7 gates).
- **WP6 `datarail-rail` FaspLink** — ACK coalescing (pack ~70 ACKs/datagram; multi-frame-per-datagram decode); ~10 → ~14 MB/s clean-link (median, noisy laptop); all S1/S2/S4 gates intact, exactly-once preserved.

## Method note
Disjointness was guaranteed by the lead's pre-work: 4 frozen-contract scaffolds + all workspace members added
before dispatch, so agents owned disjoint files and never touched the shared manifest. Audits were read-only
(zero write-conflict). The lead applied the fixes (judgment is not delegated) and ran the authoritative
`cargo clippy --workspace --all-targets` + per-crate tests before integrating.
