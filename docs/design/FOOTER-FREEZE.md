# FOOTER-FREEZE — MF-3 (the contract seam)

> The cross-crate **contract** is frozen. Dependent crates (`manifest`, `once`, `rail`, `terminal`, …)
> **transcribe** this interface; they do not redesign it. Drift is detectable by re-running one `shasum`.

- **Git HEAD at freeze:** `53fcc6564a7d24fc94fceaf205989e76a86d0f55` (re-frozen 2026-06-22)
- **Content hash (sha256 of the 3 contract crates' `lib.rs`, sorted, concatenated):**
  `70c8ed8b519c8a2be723879922794aec1d33b9aa06931a108dd3060adb25f7fd`
- **Size:** 3 crates · 620 lines.
- **Re-freeze log:** 2026-06-22 — *additive*: `datarail-crypto::hmac_blake3` (keyed BLAKE3 MAC) added for the
  idempotency key `HMAC(tenant_secret, record_key)`, needed by the terminal (P4). Prior stamp `4449a7a7…3d50`
  @ `f1ca841`. Additive only — no existing API changed; all dependents still transcribe-compatible.
- **Reproduce:**
  ```
  ls crates/datarail-core/src/lib.rs crates/datarail-crypto/src/lib.rs crates/datarail-cofre/src/lib.rs \
    | sort | tr '\n' '\0' | xargs -0 cat | shasum -a 256
  ```

## What dependents may rely on (transcribe, never change)

- **`datarail-core`** — `Cofre`, `Etiqueta`, `AeadAlg`, `Disposition`, and the `Substrate` / `Terminal` traits.
- **`datarail-crypto`** — `blake3_256`, `sign_domain` / `verify_domain` + `ctx::*` labels, `aead_seal` / `aead_open`.
- **`datarail-cofre`** — `encode` / `decode` (parse-before-verify), `seal` / `verify`.

Any change to this seam post-freeze is **STOP-THE-LINE** (re-freeze with a new stamp + a logged reason).
Still conditional on MF-0 owner ratification of the §6 trio items (does not block transcription).
