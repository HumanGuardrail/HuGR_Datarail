# AUDIT-03 — 6-lens adversarial audit of the P3 re-open code

> **Scope:** the code built during the 2026-06-22 P3 re-open (after the owner caught a false "v1 complete"):
> `StreamSubstrate`/`TcpSubstrate` (rail), `ObjectStoreSubstrate` (objectstore crate), `QuicSubstrate` (quic
> crate), `WanLink` + `congestion::DelayController` + `admission::CookieGate` (rail), `manifest::bao`, and the
> `real_socket_resume` / `real_hop_slice` / `gate_feather` acceptance tests.
> **Method:** focused self-audit; **every finding cold-verified in the cited code** before acting (Rule 5 — no
> trusting a self-report). Fixes at the root (no `#[allow]`, no gambiarra), re-verified green after each.
> **Result: 3 findings, all FIXED.** 13 crates · 99 tests · clippy `deny(all+pedantic)` · `forbid(unsafe)` ·
> no `#[allow]`. Verified via the toolchain directly (rustup proxy broken — see the env note).

## Findings

| # | Sev | Lens | Where | Disposition |
|---|---|---|---|---|
| **F1** | **MED** | parse/memory | `StreamSubstrate::recv` (rail) + `QuicSubstrate` `fill_buf`/`decode_frame` (quic) | **FIXED** — unbounded frame length: a peer could declare a ~4 GiB frame (or dribble endlessly) and force unbounded buffering → memory-DoS. Added `datarail_core::MAX_COFRE_WIRE_LEN` (64 MiB transport cap); both substrates now (a) reject a declared `len > MAX` up front and (b) bound the read buffer to `MAX + 4`. `send` enforces the same cap symmetrically. Test: `oversized_frame_length_is_rejected` (a raw peer declaring `u32::MAX` is rejected `InvalidData`, not buffered). |
| **F2** | **LOW-MED** | crypto/key-handling | `admission::CookieGate` (rail) | **FIXED** — `#[derive(Debug)]` would print the endpoint **secret** (the same leak AUDIT-02 closed for the terminal secrets). Replaced with a manual `Debug` that redacts the secret (`secret: "<redacted>"`). |
| **F3** | **LOW** | parse/memory | `ObjectStoreSubstrate::recv` (objectstore) | **FIXED** — `fs::read` of a whole object with no size cap: a malicious storage operator (the store is untrusted infra) could inject a huge file → unbounded read. Now checks `fs::metadata().len() > MAX_COFRE_WIRE_LEN` and refuses (`InvalidData`) before reading. (Verified by inspection — a 64 MiB test file is wasteful; the cap const is exercised by F1's test.) |

## Per-lens verdict

1. **Crypto / key-handling** — `CookieGate` HMAC is `hmac_blake3(secret, addr ‖ epoch_le)` with a
   **constant-time** compare (`ct_eq` folds over all 32 bytes, no early return). `bao::chunk_leaf` is
   domain-separated (`dr:bao:chunk:v1`) and binds index + total + bytes; position/content binding verified by
   the bao tests (wrong-index / cross-payload rejected). **QUIC "accept any server cert" is correct by design,
   not a bug** (SPEC 07 blind relay): confidentiality + integrity ride on the cofre's AEAD seal + Ed25519
   lacre, never on transport TLS — a MITM sees only ciphertext and any tamper is caught at the dest's `verify`;
   replay is absorbed by effectively-once. The embedded dev cert/key are non-secret (handshake only). **F2
   fixed** (secret no longer in `Debug`). No cofre keys live in any substrate.
2. **Parse / memory / int-safety** — **F1 + F3 fixed** (frame + object caps). Object-store paths are
   `hex16(route)/hex16(stream)/{seq:020}.cofre`: hex + digits only ⇒ **no path traversal** even from a
   maliciously-crafted (pre-verification) etiqueta. All `as` casts are widening (`usize→u64`, `u32→usize`) —
   no truncation. Slice indexing in `recv`/`decode_frame` is guarded by the length checks ⇒ no panic.
3. **Effectively-once** — the real-socket resume re-drives + dedups (`real_socket_resume.rs`: 0-loss/0-dup over
   a real TCP partition); `bao` reassembly is deterministic and idempotent. Sound.
4. **Dead-letter / error** — object-store `ack` tolerates `NotFound` (idempotent GC); QUIC/stream errors map to
   `io::Error`; recv timeouts surface as `None` (not error). Sound.
5. **Invariants** — `INV-OPAQUE-CARGO`: substrates wire-decode the cofre to return it but never interpret
   `carga`. `INV-DUMB-PIPE`: no substrate holds a cofre key or does cofre crypto (the QUIC transport cert and
   the cookie endpoint-secret are *not* cofre keys — transport/admission only). `INV-SUBSTRATE-POLYMORPHIC`:
   one `substrate_conformance` harness passes over all six transports. Hold.
6. **Completeness / claims** — DOD-01 was rewritten to the true state (no overclaim); BENCH-01 numbers are the
   honest Rosetta-x86_64 figures with the arch divergence flagged. No fabricated greens.

## Verdict

The P3 re-open code is **sound**; the three findings (one MED memory-DoS, two LOW/LOW-MED hardening) are
**FIXED at the root** with a shared cap constant + a redacted secret. Every buildable P3/P5 rung is now
**PROVEN + AUDITED**. Remaining open items are **owner/external only**: shmem (#24, frozen-conflict
STOP-THE-LINE), GATE-WARP bound + representative VAES HW (#13), AC-10 competitor engines (#20), MF-0 trio
ratification.
