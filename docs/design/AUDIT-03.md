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
   honest figures for this machine, flagged DIRECTIONAL. (The machine was later verified as a native Intel
   i7-9750H Coffee Lake — earlier "arm64" and "Rosetta" labels were both wrong; corrected 2026-06-22.) No
   fabricated greens.

## Verdict

The P3 re-open code is **sound**; the three findings (one MED memory-DoS, two LOW/LOW-MED hardening) are
**FIXED at the root** with a shared cap constant + a redacted secret. Every buildable P3/P5 rung is now
**PROVEN + AUDITED**. Remaining open items are external **physical resources**: representative VAES HW for
GATE-WARP (#13) and real competitor engines for AC-10 (#20); the MF-0 trio is a tech-lead call to apply.

## Addendum (2026-06-22) — shmem `ShmemRing` `unsafe` audit (#24)

The owner delegated the frozen `forbid(unsafe)` vs SPEC "lock-free shmem" conflict to the tech lead, who took
**option (A)**: one contained, audited `unsafe` in the quarantined `datarail-substrate-shmem` crate (`deny`,
not `forbid`, + a single `#[allow(unsafe_code, clippy::cast_ptr_alignment)]` — logged WAIVER). Two unsafe
operations, both audited **sound**:

- **`cell()` — `&*(map.as_ptr().add(off).cast::<AtomicUsize>())`** forms the shared-memory atomic cursors.
  The mmap base is page-aligned (4 KiB) ⇒ offsets 0/8 are 8-aligned (AtomicUsize alignment); they sit inside
  the reserved 64-byte header (in-bounds); the cells are accessed **only** via atomic ops (never aliased as
  plain memory) and the data region (`≥ HEADER_LEN`) never overlaps them. Producer Release-stores `write`
  after writing bytes; consumer Acquire-loads `write` before reading — textbook SPSC happens-before, correct
  on any arch; `MAP_SHARED` gives cross-process visibility. `cast_ptr_alignment` is a clippy false-positive the
  page-aligned base resolves.
- **`map_file()` — `MmapMut::map_mut(file)`**: memmap2 marks file mapping `unsafe` (the file could change under
  a `&[u8]`). Here it is our own ring file, sized once; all data access is synchronized by the atomic cursors,
  so concurrent access via the shared mapping is the intended SPSC protocol, not UB.

The ring **data** bytes are copied via **safe** slices (no unsafe). Tests: conformance, cross-mapping transfer
(two independent mappings of one file), ring-full back-pressure, wraparound. The rest of the workspace stays
`forbid(unsafe)`. **Verdict: the single waiver is minimal, contained, and sound; AC-6 is now 3/3 named.**
