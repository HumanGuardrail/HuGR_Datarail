# FOOTER-FREEZE — MF-3 (the contract seam)

> The cross-crate **contract** is frozen. Dependent crates (`manifest`, `once`, `rail`, `terminal`, …)
> **transcribe** this interface; they do not redesign it. Drift is detectable by re-running one `shasum`.

- **Git HEAD at freeze:** `856502f4d789cd6f7ee823205afab3703c691006` (re-frozen 2026-06-22)
- **Content hash (sha256 of the 3 contract crates' `lib.rs`, sorted, concatenated):**
  `3851940485f2a1fd137c9feec4d4c07047ab0f0db7c576948ad103493cd38179`
- **Size:** 3 crates · 720 lines.
- **Re-freeze log:**
  - 2026-06-22 — **STRUCTURAL** (`datarail-core` + `datarail-cofre`): `Etiqueta` grew `sender_present: bool`
    + `ts: u64` (SPEC-02 A4 sealed-sender flag + the informational timestamp); `ETIQUETA_LEN` 213→222 +
    encode/decode. The fine-grained sender identity now rides as an issuer-signed `SENDER_CERT` **inside** the
    encrypted carga (terminal `SenderCredential` / `issue_sender_cert` / `validate_sender_cert`), invisible to
    the rail; `ts` is informational-only (stamped 0, never trusted). **Design reconciliation (tech lead):** the
    SPEC says the cert is "bound to `cofre_id`+epoch", but `cofre_id = BLAKE3(carga)` and the cert lives inside
    the carga ⇒ a literal binding is **circular**; bound to **`eph_pk`+epoch** instead (per-cofre-unique,
    lacre-authenticated, pre-seal) — same anti-lift property, non-circular. All dependents updated; 115 tests
    green. Prior stamp `22d1f9e4…f54f8` @ `856502f`.
  - 2026-06-22 — **STRUCTURAL** (`datarail-core` + `datarail-cofre`): `Etiqueta` grew an `eph_pk: [u8;32]`
    field (the per-cofre ephemeral X25519 public key); `ETIQUETA_LEN` 181→213 + encode/decode. This wires the
    X25519 key-wrap end-to-end — the shared-route-key v1 simplification is now **CLOSED** (fresh per-cofre key,
    forward-secure, provider-blind). All dependents updated; full workspace 65 tests green. Prior stamp
    `0dedb209…88f8` @ `7a0e4bf`.
  - 2026-06-22 — *additive*: `datarail-crypto::{x25519_public, seal_key, open_key}` (X25519 per-cofre
    key-wrap primitive). Prior stamp `70c8ed8b…f7fd` @ `53fcc65`. Additive only.
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
