# AUDIT-01 — 6-lens adversarial audit of the SPEC

> **Status: PANEL COMPLETE (3/3) — NOT freeze-ready. Lead cold-verified all 8 blockers as REAL.** The freeze
> does not proceed until every blocker is closed and a re-audit passes. (THE HUGR METHOD: this is the audit
> that catches B1/B3-class silent-correctness defects before the fleet — it did.)

Panel: Lens A (completeness/fit/consistency) 3B/4M/4P · Lens B (security/crypto) 3B/9M/4P · Lens C
(perf-SOTA/craft) 2B/4M/3P. Deduped below.

## Confirmed BLOCKERS (close before freeze) + lead disposition

- **BLK-1 — AEAD/nonce contradiction + fresh-key footgun.** `DECOMPOSITION` A3 demanded misuse-RESISTANT AEAD
  (GCM-SIV) + derived nonce; `03`/`02`/`BUILD_LOG` had switched to plain AES-GCM + per-cofre fresh key + random
  nonce; `02` even kept a stale "nonce-derivation KDF" open item (self-contradiction). And the "random nonce is
  safe" claim rests on an *ungated* fresh-key premise (fork-without-reseed → catastrophic GCM reuse). **DISPOSITION
  (head decision):** **default = AES-256-GCM-SIV** (misuse-resistant — survives reuse, matches A3, kills the
  footgun); per-cofre fresh key kept as defense-in-depth; **plain AES-256-GCM = opt-in perf mode gated on a
  key-uniqueness check**; ChaCha20-Poly1305 (12-B) for non-AES-NI; **drop XChaCha & derived-nonce entirely.** → `02,03,DECOMP A3,BUILD_LOG`.
- **BLK-2 — `idempotency_key` preimage fork:** `HMAC(tenant,content)` (DECOMP/01/BUILD_LOG) vs `HMAC(tenant,record_key)` (02/03). **DISPOSITION:** **record_key** (matches `upsert_key`/effectively-once-per-key). Propagate. Document the rail-visible per-key activity fingerprint + per-epoch salt mitigation. → `01,02,03,05,DECOMP,BUILD_LOG`.
- **BLK-3 — AC-1 metamorphic test is unprovable** ("zero payload → identical" contradicts `cofre_id=BLAKE3(CARGA)`+seal). **DISPOSITION:** redefine opacity = independence from **plaintext**, not ciphertext bytes — test = replace payload with another *validly-sealed* ciphertext of equal length (re-signed), routing fields fixed → identical rail behavior. → `DECOMP AC-1, 11`.
- **BLK-4 — Signer-substitution forgery.** **DISPOSITION:** verifier MUST verify `LACRE` **only** against the route-pinned pubkey; `signer_key_id` is a selector among *pre-authorized pins*; hard-reject on mismatch. Add to AC-3. → `02,08`.
- **BLK-5 — No domain separation across Ed25519 contexts.** **DISPOSITION:** every signature = `Ed25519(unique_ctx_label ‖ msg)`; distinct labels for lacre/STH/ack/sender-cert/ticket; fixed-width fields, no shape aliasing. → `03` (+ refs in 02,04,08).
- **BLK-6 — Dedup GC vs replay race → double-commit.** **DISPOSITION:** the dest maintains a **signed monotonic low-watermark**; **reject any arriving `seq` below it** (not merely lookup); GC floor lags the max in-flight/replay horizon, not the source checkpoint. → `05`.
- **BLK-7 — Parse-before-verify** (attacker `ETIQUETA_LEN`/`CARGA_LEN` read before seal). **DISPOSITION:** total-length sanity + checked arithmetic as **step 0**, before any field read or decrypt. → `02,06`.
- **BLK-8 — Delivery-proof not bound + no `cofre_id` recompute.** **DISPOSITION (resolved):** **ack** binds `route_id‖stream_id‖seq‖STH-root`(+epoch); **leaf** binds `route_id‖stream_id‖seq‖epoch‖cofre_id` — **NOT** STH-root (circular: leaves feed the root the STH signs; caught by the apply-clone, lead-resolved). Verifier MUST recompute `cofre_id==BLAKE3(CARGA)` and reject mismatch. → `04`.

## Key MAJORS (close before freeze)

MAJ-1 STH cadence pinned + async TSA off the delivery path + GATE-LATENCY (`04,11`) · MAJ-2 dedup index bounded
(bloom prefilter + on-disk map) + overflow→dead-letter + gap-skip; add dedup-lookup to GATE-WARP harness (`05,11`) ·
MAJ-3 TSA = anti-postdating upper-bound only (honest wording; beacon for lower bound) (`04`) · MAJ-4 SENDER_CERT
needs issuer/route-authority sig + bind to `cofre_id`/epoch + revocation (`03`) · MAJ-5 PAKE: high-entropy
single-use code + attempt-cap/burn + transcript/identity binding (`08`) · MAJ-6 strike "dedup" from `cofre_id`
(identity/manifest only); dedup keys on `idempotency_key` alone; "ciphertext hash" *is* `cofre_id` (`02,01,04`) ·
MAJ-7 GATE-WARP teeth: named gating item to set the bound on VAES HW before MF-4 + CI guard fails if still PENDING
at MF-4 (`11`) · MAJ-8 itemize the `02` byte budget (incl. `wrapped_data_key`) + label `bet`.

POLISH: substrate=`auto` policy · burst-warm→idle threshold · max-cofre-size single owner · `tenant_secret`
rotation · assert wire-etiqueta == `aad` byte-identical · expand the metadata-leak statement (signer_key_id
linkage, contract_fp, per-record-key token fingerprint).

## ⚠️ Owner-ratification items (STOP-THE-LINE — logged in BUILD_LOG §6)

BLK-1's A3 reconcile and BLK-3's AC-1 redefinition edit **`DECOMPOSITION` (owner-reserved trio)**. The trio is
still DRAFT (pre-MF-0); these are audit-driven reconciliations (the demand intent is unchanged — only an
over-specified mechanism and a faulty test definition). Applied as DRAFT reconciliations and flagged for **owner
ratification at MF-0**.

## Close order

This pass: BLK-1 (02/03/BUILD_LOG) + BLK-4/BLK-5/MAJ-4/MAJ-6 (02/03). Next: BLK-2 propagation, BLK-3/A3 (trio),
BLK-6 (05), BLK-7 (02/06), BLK-8 (04), MAJ-1/2/3/5/7/8, polish → then **re-audit** → freeze.
