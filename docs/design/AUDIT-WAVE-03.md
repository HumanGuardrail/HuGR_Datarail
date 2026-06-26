# AUDIT-WAVE-03 — rigor pass: 6 adversarial audits of the wave-2/3 new code + their fixes

The owner asked to **maintain SOTA rigor**. So before adding more breadth, six read-only skeptics audited the new
code from waves 2-3 + the capstone — the code written fastest and not yet attacked. They found **2 CRITICAL + 2
HIGH + 3 MED real bugs**. Every finding lead-cold-verified; every CRITICAL/HIGH/MED fixed at the root with a
regression gate; the LOWs documented. (This is exactly why we audit our own fresh code — the tiered log and the
broker resume both had real data bugs.)

| WP | Target | Verdict | Real findings → fix |
|---|---|---|---|
| **WP1** | `datarail-tieredlog` | **CRITICAL + HIGH** | **[CRITICAL] recovery ignored the cold tier** + a non-atomic rotate (evict-before-create) → a crash mid-rotate left an empty local dir → `open` reset `active_start` to 0 → **all cold history corrupted/lost**. **Fixed:** rotate now **creates the new hot segment BEFORE evicting** the old (a crash always leaves ≥1 local segment; the self-heal path re-rotates). **[HIGH] silent evict failure** (`drop(remove_file)`) → unbounded local-disk growth. **Fixed:** the unlink error is surfaced. **[LOW]** unparseable `seg/` key now errors instead of silently gapping. Offload-before-evict ordering + cross-tier stitching + parse safety verified SOUND. |
| **WP2** | `datarail-broker` (TCP + resume) | **CRITICAL** | **Poll-resume off-by-one**: the returned offset was the record's *start*, so a real client committing it would **re-deliver the last record on every restart** (the e2e test masked it by committing the produce offset). **Fixed:** Poll returns `Group::position()` (the cursor AFTER the record) = the commit/resume point; restart resumes at the NEXT record. New regression gate `restart_resumes_without_redelivering_the_last_record` + updated the two affected tests + the system capstone. Framing/parse-safety, Mutex-coarse concurrency, error-mapping, commit-durability verified SOUND. **[MED noted]** no read-timeout/conn-cap (slowloris) — tracked. |
| **WP3** | `datarail-replicated-topic` | **SOUND** | Frame split exact-inverse, padding byte-exact via the len header, reconstruct returns None (never wrong bytes) for unknown/over-degraded, `Ok ⇒ reconstructable` (two-store write order). Only a **LOW** orphan-shard leak on the `erasure.put`-fails-after-log-append path — noted (log is source-of-truth; no caller-visible inconsistency). |
| **WP4** | `datarail-netblob` | **CRITICAL** | **[CRITICAL] unbounded allocation** from a hostile 4-byte length prefix (`vec![0u8; len]`, len up to 4 GiB) → OOM/DoS on server AND client. **Fixed:** `MAX_FRAME` cap rejected before allocating. **[MED] slowloris** thread-leak. **Fixed:** read timeout on accepted streams. Cross-response isolation, round-trip integrity, error propagation, missing-vs-error distinction verified SOUND. **[MED noted]** permanent Mutex poisoning. |
| **WP5** | `datarail-offsets` (compaction) | **SOUND core** | The crash-during-compaction window (prime suspect) verified CORRECT (tmp+fsync+rename+dir-fsync; old-log-replay ≡ snapshot in every crash branch). **[MED] post-rename error path** left `self.file` on the unlinked inode → a later commit silently lost. **Fixed:** reopen the live handle immediately after the rename; the dir-fsync is best-effort after. **[MED noted]** mid-log (non-tail) corruption truncates recovery; **[LOW]** macOS `fsync` ≠ `F_FULLFSYNC`. |
| **WP6** | `datarail-rail` (FASP ACK-coalescing) | **HIGH** | **`pending_acks` escaped the receive-window bound** — a sustained/duplicate DATA flood grew it during a long pump drain (it was flushed only AFTER the drain), re-introducing the unbounded receive memory F1 forbids. **Fixed:** flush ACKs INCREMENTALLY once a datagram's worth (`ACK_FLUSH_THRESHOLD`) accumulates. Multi-frame parse safety, exactly-once under coalesced-ACK loss, Karn, F4 all verified SOUND. **[LOW noted]** drain-duration adds RTT noise. |

## Method
6 read-only audits (zero write-conflict) on disjoint crates. The lead applied every fix (judgment not delegated),
added regression gates for the CRITICALs, and ran `cargo clippy --workspace --all-targets` clean + per-crate tests
green (incl. the ~94 s offsets-compaction gate and the full FASP S1/S2/S4 suite). **Two CRITICAL data bugs (cold
history loss on crash, restart re-delivery) and a DoS (netblob OOM) were caught and killed before they shipped** —
the dividend of attacking your own freshly-written code harder than the world will.
