# FOOTER-FREEZE — MF-3 (the contract seam)

> The cross-crate **contract** is frozen. Dependent crates (`manifest`, `once`, `rail`, `terminal`, …)
> **transcribe** this interface; they do not redesign it. Drift is detectable by re-running one `shasum`.

- **Git HEAD at freeze:** `7a0e4bfb825a88adc8d7482f13d32e6ba32b5b75` (re-frozen 2026-06-22)
- **Content hash (sha256 of the 3 contract crates' `lib.rs`, sorted, concatenated):**
  `0dedb209717c4e3b12609a4266f45b85bc1e2902ce05463f3d4e54a52b3688f8`
- **Size:** 3 crates · 689 lines.
- **Re-freeze log:**
  - 2026-06-22 — *additive*: `datarail-crypto::{x25519_public, seal_key, open_key}` (X25519 per-cofre
    key-wrap) added to close the v1 shared-route-key simplification. Prior stamp `70c8ed8b…f7fd` @ `53fcc65`.
    Additive only — no existing API changed. NOTE: the *terminal wiring* of this (an `eph_pk` field on
    `Etiqueta`) is a pending **core-seam** change, to be re-frozen when landed.
  - 2026-06-22 — *additive*: `datarail-crypto::hmac_blake3` (keyed BLAKE3 MAC) for the idempotency key
    `HMAC(tenant_secret, record_key)`, needed by the terminal (P4). Prior stamp `4449a7a7…3d50` @ `f1ca841`.
    Additive only — no existing API changed; all dependents still transcribe-compatible.
- **Reproduce:**
  ```
  ls crates/datarail-core/src/lib.rs crates/datarail-crypto/src/lib.rs crates/datarail-cofre/src/lib.rs \
    | sort | tr '\n' '\0' | xargs -0 cat | shasum -a 256
  ```

## What dependents may rely on (transcribe, never change)

- **`datarail-core`** — `Cofre`, `Etiqueta`, `AeadAlg`, `Disposition`, and the `Substrate` / `Terminal` traits.
- **`datarail-crypto`** — `blake3_256`, `hmac_blake3`, `sign_domain` / `verify_domain` + `ctx::*` labels,
  `aead_seal` / `aead_open`, and the X25519 key-wrap `x25519_public` / `seal_key` / `open_key`.
- **`datarail-cofre`** — `encode` / `decode` (parse-before-verify), `seal` / `verify`.

Any change to this seam post-freeze is **STOP-THE-LINE** (re-freeze with a new stamp + a logged reason).
Still conditional on MF-0 owner ratification of the §6 trio items (does not block transcription).
