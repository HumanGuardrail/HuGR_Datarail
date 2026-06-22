# AUDIT-02 — adversarial audit over the CODE (post-implementation)

> AUDIT-01 audited the *spec*. AUDIT-02 audits the *implementation*: 6 adversarial lenses swept the 10 crates
> in parallel (read-only clones), each returning findings; **every finding was cold-verified by the lead**
> (re-reading the cited code + re-running the gates) before action. Fixes landed in `e74e75c`.

- **Date:** 2026-06-22 · **Baseline before fixes:** `4bc330a` · **Fixes:** `e74e75c`
- **Verification:** `cargo clippy --workspace --all-targets` clean (deny all+pedantic) + `cargo test --workspace`
  **68 passed / 0 failed**, run via the toolchain directly (`~/.rustup/toolchains/stable-x86_64-apple-darwin/bin`)
  after the rustup proxy symlink broke mid-session (see "Environment note").
- **Method honesty:** a transient "session limit" cut the **completeness/claims** lens short; its key items were
  self-verified by the lead (recorded below) — that lens is marked PARTIAL, not PROVEN-by-clone.

## Per-lens verdicts

| Lens | Verdict | Findings |
|---|---|---|
| Invariants (INV-*) | **ALL 6 HOLD** | 0 |
| Crypto / key-handling | core constructions correct | 1 HIGH, 2 MED, 2 LOW |
| Parse / memory / int safety | 1 finding, rest CLEAN | 1 LOW-MED |
| Effectively-once | logic correct; test-coverage gap | 1 MED, 2 LOW |
| Dead-letter / error | **CLEAN** | 0 (2 INFO) |
| Completeness / claims | PARTIAL (clone cut off) | self-verified, no defect found |

## Findings + dispositions

| # | Sev | Location | Finding | Disposition |
|---|---|---|---|---|
| F1 | **HIGH** | terminal `fresh_nonce` (was `next_nonce`) | `seq`-derived nonce + `seq` resets on `new()` ⇒ a fork/VM-snapshot entropy replay could re-pair `(data_key, nonce)`; catastrophic under non-SIV algs | **FIXED** `e74e75c` — nonce is now a fresh CSPRNG draw, decoupled from `seq` (defense-in-depth atop fresh per-cofre keys, which already give one-message-per-key) |
| F2 | MED | core/terminal `Gcm256` "key-uniqueness gate" | documented gate not enforced in code | **RESOLVED-BY-DESIGN** (`cb79d75`) — the per-cofre fresh data key (one message per key ⇒ no within-key nonce reuse) + the F1 random nonce *are* the key-uniqueness gate; no runtime gate needed. (Core enum doc left as-is to avoid a seam re-freeze for a comment.) |
| F3 | MED | terminal `TerminalConfig`/`SourceTerminal`/`DestTerminal`, once `Once` | secret-bearing structs `#[derive(Debug)]` over raw key bytes ⇒ leak-via-log | **FIXED** `e74e75c` — hand-written `Debug` redacts `tenant_secret`/`source_seed`/`dest_x25519_secret`/watermark `seed` (`<redacted>`) |
| F4 | LOW | terminal `board` | ephemeral X25519 secret not explicitly zeroized | **FIXED** (`cb79d75`) — `eph_secret` + the per-cofre `data_key` are `zeroize()`d immediately after use (`zeroize` dep, already in-tree via dalek) |
| F5 | LOW | terminal AEAD AAD | empty AEAD AAD was safe only via the lacre; binding lived outside the AEAD | **FIXED** (`cb79d75`) — `aead_aad()` binds the AEAD to every seal-time-final header field (route‖stream‖seq‖idempotency_key‖contract_fp‖aead_alg‖nonce‖eph_pk); board + offload recompute identically, sidestepping the `cofre_id` cycle |
| F6 | LOW-MED | terminal `unframe_batch` | `Vec::with_capacity(count)` from an untrusted `u32` ⇒ ~100 GB alloc DoS (post-auth: only a compromised/buggy same-trust-zone source can reach it) | **FIXED** `e74e75c` — `with_capacity(count.min(bytes.len()/4))` (a record needs ≥4 header bytes) |
| F7 | MED | once AC-4 DST | GC-vs-replay path never exercised (`gc_lag=1024` ≫ run ⇒ GC floor ≡ 0); logic statically sound but untested | **FIXED** `e74e75c` — DST now builds the gate with `gc_lag=3` (< `PER_STREAM=8`) so GC fires mid-run under fault injection; still 0-loss/0-dup over 1000 seeds |
| F8 | LOW | once `gc_lag` doc | doc overstated `gc_lag` as a correctness obligation ("must exceed the replay horizon") | **FIXED** `e74e75c` — corrected: GC is safe for **any** `gc_lag` (incl. 0) because reject-below dominates; it is a dedup-retention/re-ack-cost knob |
| F9 | INFO | terminal/once | `gc_lag` is unreachable from production (`DestTerminal::new` always uses `Once::new`) — `with_gc_lag` is "dead config" outside tests | **NOTED** — thread `gc_lag` through a future config if a stream can exceed `DEFAULT_GC_LAG`; not a v1 defect |

## Confirmed holding (verified, no change needed)

- **All 6 invariants hold in code.** INV-OPAQUE-CARGO: neither substrate reads `carga` (only `cofre.clone()` +
  `etiqueta.cofre_id`). INV-SEAL-COMPLETE: `encode_etiqueta` covers all 10 fields incl. `eph_pk`; every field
  read at offload is inside the signed region; AC-2 flips every byte and none verify. INV-DUMB-PIPE: substrates
  hold no keys, do no crypto (`datarail-crypto` is a rail dev-dep only). INV-CONTRACT-SPLIT, INV-EFFECTIVELY-ONCE,
  INV-SUBSTRATE-POLYMORPHIC all hold (both substrates pass the one `substrate_conformance` harness).
- **Crypto cores correct:** X25519 KDF binds both endpoints + domain-separated; Ed25519 contexts all distinct;
  signer pinning (BLK-4) + cofre_id recompute (BLK-8); keyed idempotency MAC; genuine RFC-8452 GCM-SIV default;
  parse-before-verify (BLK-7) exhaustive.
- **Effectively-once logic sound under adversarial reasoning:** reject-below dominates GC (a GC'd `seq` is
  always `< low_watermark` ⇒ replay → `Duplicate`); GC keyed off the *contiguous* watermark, so reorder distance
  is irrelevant; no double-commit, no loss, no off-by-one; per-stream isolation holds.
- **Dead-letter completeness:** all six offload failure classes reason-coded onto the siding + `DeadLettered`,
  cofre preserved, no silent drop / panic / `?`-into-loss; `board` never partially boards; error taxonomies
  exhaustive (no `_ =>` swallowing); CLI exits nonzero on error; a dead-letter is a SUCCESS-exit managed outcome.
- **Parse/safety:** cofre `decode` + spec parser + CLI hex are panic-free / bounded on hostile input;
  `forbid(unsafe)` enforced workspace-wide (zero `unsafe` in any `src/`).
- **Completeness (self-verified, lens cut off):** `ETIQUETA_LEN = 213` arithmetic correct
  (16+16+8+32+32+32+1+12+32+32); no `todo!()`/`unimplemented!()` in non-test code; CLI implements
  validate/keygen/run/verify/ticket; spec key names match examples/tests (`dest_x25519_secret`); no AC claimed
  proven without a test.

## Environment note (for future sessions)

The `rustup` proxy symlinks in `~/.cargo/bin` (`cargo`/`cargo-clippy`/… → `rustup`) broke mid-session (the
`rustup` shim is missing), so `cargo` is "command not found" on the login `PATH`. **Workaround:** invoke the
toolchain directly — `export PATH="$HOME/.rustup/toolchains/stable-x86_64-apple-darwin/bin:$PATH"` (stable =
1.96.0). All AUDIT-02 verification used this path. (Caught a false-green: `grep -c "warning:"` on a
"command not found" message returns 0 — always confirm `cargo --version` first.)
