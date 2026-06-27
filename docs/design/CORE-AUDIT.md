# CORE-AUDIT — brutal adversarial audit of the two pillars (seal-core + exactly-once/durability)

> Two hostile background auditors attacked the code the whole product rests on: the **sealing core** (crypto +
> cofre + terminal — the provider-blind guarantee) and the **exactly-once / no-loss core** (once + broker + topic
> + offsets). Both were disciplined and cold-verified with PoCs. This is the honest record: every finding, its
> verdict, and its disposition (**FIXED** now / **TRACKED** gap / **ACCEPTED** trade-off). It also CORRECTS two
> over-claims the audit exposed.

## Headline corrections (honesty — these were over-stated before this audit)
1. **"effectively-once" is NOT durable across crashes.** The `datarail-once` dedup index is **in-memory only**
   and is **not wired into the broker** at all (the broker is plain at-least-once). Effectively-once holds
   *within a single process run* (the `ac4_dst.rs` 1000-seed test, no crash) — NOT across a restart, where an
   at-least-once transport would re-deliver. Corrected in `LASTRO-MATRIX`. Closing this (persist the dedup index +
   wire it in) is **TRACKED** (finding D-3).
2. **"no-loss across crashes" was tested only against process-kill, not power-loss.** The chaos test used
   drop+reopen, which preserves the OS page cache — it never exercised fsync durability. The broker acked
   produces *without fsync*. **Now FIXED** (durability-before-ack, D-1); with the fsync, acked records survive a
   true power loss, and the claim is honest.

---

## Seal-core audit (crypto / cofre / terminal) — VERDICT: provider-blind HOLDS vs the wire adversary
The auditor **confirmed sound**: full signature coverage (every wire byte signed; mutate-every-byte rejected),
verify-before-decrypt + AAD binding, cross-route/replay blocked, per-cofre key freshness in normal operation,
panic-safety on hostile cofre bytes, constant-time (no secret-dependent compare), Ed25519 malleability harmless.

| # | Finding | Sev | Disposition |
|---|---|---|---|
| S-1 | **DRBG reseed keyed on PID → VM-snapshot/clone replays identical `(eph_secret, nonce)`** (catastrophic under opt-in `Gcm256`, plaintext-equality leak under GCM-SIV default). PROVEN. | HIGH | **FIXED** `675d70d`: every DRBG draw now mixes fresh OS entropy → clone-immune by construction, not by PID detection. Regression test: identical-state clones diverge every draw. |
| S-2 | **Sealed-sender `epoch` never enforced** — a revoked sender's old cert is accepted forever (revocation is theater). PROVEN. | MED | **FIXED**: `DestTerminal::with_min_sender_epoch(n)` — a cert with `epoch < n` is dead-lettered (default 0 = backward-compatible; raise after a rotation). Regression test added. |
| S-3 | No X25519 contributory/low-order-point check in key-wrap. | LOW | **ACCEPTED** (not reachable): `eph_pk` is always source-generated and authenticated (lacre verified) BEFORE ECDH, so no adversary can inject a low-order point. Defense-in-depth check is TRACKED. |
| S-4 | `idempotency_key = HMAC(tenant_secret, record_key)` rides cleartext → the rail can see which cofres share a record_key within a tenant (linkability). | LOW | **ACCEPTED** trade-off: inherent to a deterministic dedup key; `record_key` itself is NOT recoverable. Documented. |
| S-5 | `sender_present` flag + a 200 B carga delta leak *whether* (not who) a cofre carries a sealed sender. | LOW | **ACCEPTED** trade-off: identity is correctly hidden (proven); presence-padding is optional future work. |

## Once / durability audit (once / broker / topic / offsets) — VERDICT (before fixes): no-loss + effectively-once did NOT hold across crashes
The auditor **confirmed sound**: resume off-by-one is correct *when the topic survives*, GC vs reject-below
interaction, offsets recovery parse-safety (CRC + torn-tail truncation, no panic), single-`Mutex` concurrency.

| # | Finding | Sev | Disposition |
|---|---|---|---|
| D-1 | **Broker acked Produce before any fsync** → power-loss loses acked records. PROVEN. | CRIT | **FIXED** `741b803`: durability-before-ack — broker `topic.sync()` (fsync) before replying `Produced`. |
| D-2 | **Fsync'd committed offset outlived un-synced topic** → resume wedges + loss. PROVEN. | CRIT | **FIXED** `741b803`: same fsync makes the topic at least as durable as the offset pointing into it. |
| D-3 | **Dedup index in-memory only** → restart = duplicate flood; "effectively-once across crashes" false. PROVEN. | CRIT | **FIXED for the transactional sink (Tier A): exactly-once into Postgres**, the dedup watermark stored ATOMICALLY in the DB (`TxnSink::commit_at`, `EXACTLY-ONCE-DESIGN.md`) — a replayed batch is a committed no-op; PROVEN live (4 rows despite 3 replays). File/webhook sinks are honestly at-least-once (Tier C). |
| D-4 | **Same `seq` above the watermark with a different key → delivered twice.** PROVEN. | HIGH | **FIXED** `741b803`: seq-level dedup (`delivered_keys`, never GC'd above watermark) — a seq is committed once regardless of key. Regression test added. |
| D-5 | A permanent gap pins the watermark → `seen`/`ahead`/`delivered_keys` grow without bound (RAM DoS). | MED | **FIXED**: `MAX_REORDER_HORIZON` (1<<20) — past it the lowest gap is sealed-skipped (watermark advances, the missing seq is thereafter reject-below'd). Availability-over-completeness; never triggers in normal reorder. |
| D-6 | `FileOffsets` doesn't fsync the parent dir on create / re-fsync after compaction (rename durability). | MED | **FIXED**: `fsync_dir` helper — fsync the dir on `open` (first-create durable) and (propagated, not swallowed) after the compaction rename. |
| D-7 | `Commit` accepted an arbitrary offset (past tail → wedge). | LOW | **FIXED** `741b803`: reject a commit past the durable topic tail. |

## Txn-path audit (the new exactly-once `commit_at`) — 2 PROVEN CRITICALs, all fixed
A third brutal auditor hit the new transactional path (it claimed "exactly-once PROVEN" — durability components
always get audited). It CONFIRMED safe: no SQL injection (`hex` is hex-only; identifiers validated; bytea literal
correct), single-connection crash atomicity (COPY participates in the txn rollback), panic-safety. It PROVED two
real breaks (both fixed + regression-tested):

| # | Finding | Sev | Disposition |
|---|---|---|---|
| T-1 | **Backend `ErrorResponse` desynced the wire** — `query_simple`/`copy_in` early-returned on `'E'` without draining the mandatory trailing `ReadyForQuery`; every later query then misread → `resume_watermark` returned 0 → the WHOLE stream re-landed. PROVEN (durable wm 1, read back 0). | CRIT | **FIXED**: every query/COPY now DRAINS to `'Z'`, capturing the error, before returning. Live regression `a_backend_error_does_not_desync_the_connection`. |
| T-2 | **`SELECT … FOR UPDATE` locks no missing row** → two concurrent first-batches both land (double-land). PROVEN (2 rows for 1 batch). | CRIT | **FIXED**: a transaction-scoped `pg_advisory_xact_lock` keyed by the stream serializes concurrent `commit_at` even with no watermark row yet. |
| T-3 | Watermark `0` ambiguous (no-row vs seq 0) → first batch silently lost / re-landed. | HIGH | **FIXED**: `read_watermark` returns `Option<i64>` (None = no row, distinct from 0); a present-but-unparsable value is a hard error. |
| T-4 | A short/malformed `DataRow` silently returned 0. | MED | **FIXED**: a present-but-unparsable `DataRow` is now an error, never a silent 0. |
| T-5 | `watermark > i64::MAX` clamped silently. | LOW | **FIXED**: errors instead of clamping. |

The "exactly-once PROVEN" claim was real for the happy path but FALSE under error/concurrency — exactly what the
audit is for. It is now actually true (live C1 regression + the concurrency lock), and re-gated in CI.

## Honest status after this pass
- **Sealing / provider-blind:** HOLDS against the wire adversary (pipe/storage/MITM) in the GCM-SIV default; the
  one real weakness (snapshot/clone key reuse) is FIXED. `Gcm256` (opt-in `vaes`) is now also clone-safe.
- **No-loss:** the broker is now **durability-before-ack** — acked records survive power loss. FIXED.
- **Effectively-once:** holds **within a process run**; **NOT yet across crashes** (dedup not persisted/wired) —
  honestly TRACKED, not claimed. The broker is at-least-once across restarts.
- Remaining TRACKED: **dedup persistence (D-3)** — the big one, effectively-once cross-crash; X25519 low-order
  defense-in-depth (S-3, not reachable). (S-2 epoch, D-5 gap horizon, D-6 offsets dir-fsync are now FIXED.) None is a wire-adversary confidentiality/integrity break. D-3 is a correctness-critical durable
  component scoped for its own careful arc + audit, not a tail-of-session rush.

This pass is the methodology working: the auditors proved the scary lenses *safe* (signatures, verify-before-
decrypt, panic-safety) and found the real defects (a crypto clone-reuse + the durability/dedup gaps), which were
fixed at root or honestly tracked — and two over-claims were corrected.
