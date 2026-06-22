# FOOTER-FREEZE — MF-3 (the contract seam)

> The cross-crate **contract** is frozen. Dependent crates (`manifest`, `once`, `rail`, `terminal`, …)
> **transcribe** this interface; they do not redesign it. Drift is detectable by re-running one `shasum`.

- **Git HEAD at freeze:** `f1ca8410e1944685fe0975a35737da94d63b7de7`
- **Content hash (sha256 of the 3 contract crates' `lib.rs`, sorted, concatenated):**
  `4449a7a7f7c339a6476ae76453a6e55a54273534eb76c56992aaacdec5623d50`
- **Size:** 3 crates · 600 lines.
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
