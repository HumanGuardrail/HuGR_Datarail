# BUILD_LOG — single source of truth

> Re-read on every wake. Durable state lives in three places: this log, git, and the task list. Built per
> THE HUGR METHOD.

## §0 — Goal lock (the complete product)

Datarail is done when every Acceptance Criterion in [`DECOMPOSITION.md`](DECOMPOSITION.md) is green and every
`INV-*`, every `GATE-*`, and every proof method is owned, implemented, and proven. A stub is transient WIP,
never a delivered state. Performance/SOTA numbers are built-and-measured, never invented (no number before
its artifact).

## §1 — Rigor compact (inviolable)

Rigor is never loosened; no debt or gambiarra is left — unless the Owner authorizes it with a logged WAIVER.
The lead never authorizes its own waiver. A failing gate is fixed at the root, never bypassed.

## §2 — Method & pipeline

Ideia → Trio (MF-0) → Architecture + cheapest spike (MF-1) → SPEC (MF-2) → ROADMAP → WPs/contracts (MF-3) →
EXECUTE (Kage-Bunshin) → Prove (fairness gate) → Deliver. **Each stage frozen before the next.**

## §3 — Decision log

> **Append-only/historical.** Where an early entry conflicts with a later one (the cofre `idempotency_key`
> preimage; the H3 nonce wording), the **later** entry + AUDIT-01 + §6 are operative. The re-audit confirmed the
> live SPEC docs are fully consistent: **GCM-SIV default · `idempotency_key = HMAC(tenant, record_key)` · no
> derived nonce · no convergent encryption.**

- 2026-06-21 — Runtime: **Rust**.
- 2026-06-21 — Guarantee: **effectively-once** (at-least-once + idempotent/transactional sink); a clause of
  the offloading contract.
- 2026-06-21 — Architecture: **CAST** — provider-blind/zero-knowledge, **fixed A→B routes** (no
  overlay/discovery), **ephemeral rail** (serverless/scale-to-zero). See ADR-0001.
- 2026-06-21 — Doctrine: **smart sealed endpoints, dumb cheap pipes**; mechanism (ours) vs policy (user's).
- 2026-06-21 — Cofre = authenticated header (*etiqueta*) + AEAD payload (opaque) + Ed25519 *lacre*;
  idempotency key = `HMAC(per-tenant secret, content)` (NOT a raw content hash); sealed-sender (sender id
  inside the payload).
- 2026-06-21 — **Craft Charter frozen** (Owner-mandated): performance obsession + code-as-art with mechanical
  teeth (gates, `forbid(unsafe_code)`, `clippy deny`, LOC caps). See CONSTITUTION.
- 2026-06-21 — Competitive teardown (6-clone research wave) complete → patterns to steal + must-build + wedge
  captured; folded into DECOMPOSITION.
- 2026-06-21 — **H3 spike (sealed warp speed): INCONCLUSIVE / DIRECTIONAL — does NOT refute H3.** Measured on
  i7-9750H (2019 Coffee Lake, **pre-VAES**) under 2–3× host load — NOT representative of target (VAES cloud
  cores do AES-GCM at 5–15+ GiB/s). Numbers (best-of-12 MIN, AES-NI engaged): AES-256-GCM-SIV ~0.8 GiB/s,
  ChaCha20-Poly1305 ~0.9–1.0, BLAKE3 ~2.5, Ed25519 sign ~30k/s, verify ~15k/s. Two REAL signals: (1) **AEAD is
  the wall**, and GCM-SIV (two-pass) is the slowest AEAD → SPEC must weigh single-pass AES-GCM (VAES) vs
  GCM-SIV given we derive the nonce; (2) **per-cofre Ed25519 dominates small payloads** → **batch-many-records-
  per-cofre is mandatory** (validates "seal per vagão"). `GATE-WARP` target = PENDING a re-run on representative
  HW. Reusable best-of-N harness on branch `spike/h3-sealed-warp-speed`.
- 2026-06-21 — **MF-1 closed (passed-with-notes).** **H1 (ZK-completeness): CONFIRMED** — every rail function
  computes from the authenticated header + seal alone; none reassigned to the terminal (see
  `design/01-rail-surface.md`); conditioned on `INV-SEAL-COMPLETE` + the accepted metadata residual
  (size/timing/idem-token). **AEAD decision (refined AGAIN by AUDIT-01 — see design/03):** pluggable; **default = AES-256-GCM-SIV**
  (nonce-misuse-resistant — closes the fork-reseed footgun + matches DECOMPOSITION A3). Per-cofre fresh key =
  defense-in-depth; plain AES-256-GCM = opt-in perf mode **gated on key-uniqueness**; ChaCha20-Poly1305 (12-B)
  for non-AES-NI. Dedup = header `idempotency_key = HMAC(tenant_secret, record_key)` (record-key); no convergent
  encryption, no derived nonce. **batch-many-records-per-cofre LOCKED** (one Ed25519 sig amortized per lote).
  **`GATE-WARP` target = PENDING** a re-bench on representative VAES hardware (no representative box now;
  labeled, NOT blocking).

## §4 — Running log (one line per meaningful step)

- 2026-06-21 — Repo initialized (branch `main`). Mission control stood up: trio (DRAFT), CONSTITUTION,
  ADR-0001, PRE-REGISTRATION, this log, README, `.gitignore`.
- 2026-06-21 — H3 spike dispatched via clone (Kage Bunshin). Clone *looked* idle ~1h but was alive (incremental
  commits; self-pivoted criterion→best-of-N on detecting host contention). Owner killed it; work recovered from
  the terrain (Rule 5). Outcome in §3 (DIRECTIONAL, not refuted). **Process fix:** clones henceforth run
  background + hard timeout + liveness diagnosed by the *work* (commits/mtime), never by silence. **Next:**
  re-run H3 on representative HW (sets `GATE-WARP`) + H1 reasoning → then MF-2 (SPEC). MF-1 not blocking: no
  architectural dealbreaker found.
- 2026-06-21 — H1 reasoned + recorded (`design/01-rail-surface.md`); **MF-1 CLOSED**. **Autonomous mode (Owner
  directive): TechLead decides + executes, no per-step approval; escalate only STOP-THE-LINE.** Standing PENDING:
  `GATE-WARP` re-bench on a representative VAES box when one is available. **Now proceeding to MF-2 (SPEC);**
  next doc: the Cofre wire format (`design/02-cofre-format.md`).
- 2026-06-21 — **SPEC component docs 01–11 all drafted** (rail-surface/H1, cofre, crypto, manifest,
  effectively-once, terminal, rail/substrate, routing/identity, CLI/spec, durable-store, gates/proof-methods).
  **Autonomous build loop ESTABLISHED** (self-paced; goal = §0; do not stop until 100% complete/SOTA/tested/
  audited; escalate only STOP-THE-LINE). **Next iterations:** `99` coverage matrix (zero unowned) → 6-lens
  adversarial audit (clone panel) → close blockers → freeze MF-2 (content-hash) → ROADMAP → MF-3 contract
  freeze → Kage-Bunshin fan-out build → test → AC-10 benchmark → final audit → deliver. Standing PENDING:
  GATE-WARP re-bench (VAES HW), AC-10 external engines, trio ratification (owner) — labeled, never faked.
- 2026-06-21 — **6-lens audit complete (AUDIT-01): NOT freeze-ready — 8 blockers + 17 majors, all cold-verified
  REAL** (the audit earned its keep — caught a self-contradicting AC-1 test, a signer-substitution forgery, missing
  domain separation, a dedup-replay double-commit race, parse-before-verify). **Closed this pass (02/03/BUILD_LOG):**
  BLK-1 (AEAD→GCM-SIV default), BLK-4 (verify-pinned-key MUST), BLK-5 (domain separation), MAJ-4 (sender-cert
  issuer), MAJ-6 (cofre_id not a dedup key), BLK-7 (parse-before-verify in 02; 06 pending). **Remaining → task #14:**
  BLK-2 (idempotency record_key propagate), BLK-3 (AC-1 — trio), BLK-6 (dedup watermark-reject, 05), BLK-8 (proof
  binding, 04), MAJ-1/2/3/5/7/8 → then **re-audit** → freeze MF-2.
- 2026-06-21 — Mechanical-fix clone applied BLK-2/6/7(06)/8 + MAJ-1/3/5 to 01/04/05/06/08 (write-only); **lead
  cold-verified (Rule 4) — clone caught a real circularity in my BLK-8 disposition** (leaf binding `STH-root` is
  circular). Lead-resolved: leaf binds `route/stream/seq/epoch/cofre_id`, only the `dest_ack` binds `STH-root`.
  **All 8 blockers + majors now closed.** Committed. **NEXT: RE-AUDIT (fresh clone panel) → if clean, re-run 99
  coverage → freeze MF-2.**
- 2026-06-21 — **MF-2: SPEC FROZEN.** Re-audit verdict FREEZE-READY (8/8 blockers + 7 majors closed, zero new
  drift). Content-hash `5f88095fb370e4de3315653c246ceaece58a99d787ae6a9cd0a41a03e0bb6d89` over 13 design docs /
  705 lines @ HEAD `c0026da` — see `design/SPEC-FREEZE.md`. **Conditional on MF-0 owner ratification of the 3
  §6 trio reconciliations.** Tasks #11/#14/#12 done. **NEXT: ROADMAP (phased, proof-obligation per phase) → MF-3
  contract freeze (cofre/header/manifest IDL + terminal seam + compilable stubs) → Kage-Bunshin fan-out build of
  the crates (scaffold-first, disjoint WPs, cold-verify each).**

- 2026-06-22 — **P0 + P1 built (real code, not just docs).** P0: ROADMAP + cargo workspace (datarail-core
  contract crate). P1: **datarail-crypto** (BLAKE3, domain-sep Ed25519, pluggable AEAD GCM-SIV default) +
  **datarail-cofre** (canonical encode/decode, parse-before-verify, seal/verify). Every crate: `forbid(unsafe)`,
  clippy **deny(all+pedantic)** clean, tests green. **Proofs PROVEN:** AC-2 (exhaustive every-byte-flip →
  reject), AC-3 (wrong/forged key → reject), Ed25519 domain-separation, AEAD tamper/AAD reject. Commits
  `ead8b02` / `c222c30` / `d87dd98`. **NEXT:** FOOTER-FREEZE the core+crypto+cofre contract seam (content-hash)
  → fan out P2 (manifest+once: AC-5, AC-4 DST) ∥ P3 (rail+substrates: AC-1/6/8, GATE-FEATHER) to background
  clones → P4 terminals (AC-9) → P5 CLI + first vertical slice (MF-4).

- 2026-06-22 — **P2/P3 fan-out COMPLETE (3 parallel clones, all cold-verified + merged).** datarail-rail
  (`30a0022`, AC-1 metamorphic opacity + AC-6 parity), datarail-manifest (`4cbe2c4`, AC-5; **lead fixed a Charter
  `#[allow]` breach** → refactored to a `DeliveryProof` bundle), datarail-once (AC-4 DST: SplitMix64, 1000 seeds,
  drop/reorder/dup/kill → **0 loss / 0 dup**). **Full workspace: 6 crates, 34 tests green, clippy deny(all+pedantic)
  clean.** Proven: **AC-1,2,3,4,5,6** + crypto domain-sep/AEAD. Kage-Bunshin validated (disjoint crates,
  merge-on-green, cold-verify caught the breach + a stray clone `git checkout`, no damage). **Lead decision-pending
  (resolve at P4):** datarail-once returns `DeadLettered` for a below-watermark redelivery; the terminal's siding
  semantics may instead want `Duplicate` (benign re-ack). Note: `WM_CTX = b"dr:once:wm:v1"` is a crate-local
  domain label (fine; not added to the frozen `ctx`). **NEXT: P4** datarail-terminal (board/offload wiring
  core+crypto+cofre+once+manifest; content-contract enforce; dead-letter — AC-9) + a file/in-mem connector →
  **P5** spec + CLI + the **first vertical slice (MF-4)**.

- 2026-06-22 — **P4 COMPLETE: datarail-terminal merged (`c803e6d`)** — the integration. board (validate →
  frame RECORD_BATCH → `hmac_blake3` idempotency → `aead_seal` → `cofre::seal`) + offload (verify → contract_fp
  → `aead_open` → re-validate → `Once::admit` → commit) + reason-coded dead-letter siding + in-mem sink. Built by
  a clone, **cold-verified** (clippy deny(all+pedantic) clean, **no `#[allow]`** — config→`TerminalConfig`,
  errors→`DeadLetterReason` at root per C4). **AC-9 proven** (onboarding + offloading refusals → dead-letter,
  never committed) + **exactly-once** (same cofre 2× → Delivered then Duplicate, sink once). **Full workspace: 7
  crates, 47 tests green.** Proven cumulatively: **AC-1,2,3,4,5,6,9 + exactly-once**. **PENDING hardening
  (labeled, not faked):** v1 uses a shared route data-key + `AAD=&[]` + deterministic nonce → the per-cofre
  **X25519-wrapped key** (SPEC A5) is a later crypto-seam add; `aad=etiqueta` is blocked on the `cofre_id`
  ordering cycle (resolve when X25519 lands); the core `Terminal` trait impl is deferred (inherent methods used).
  **NEXT: P5** — datarail-spec (`rail.toml`) + datarail-cli + the **FIRST VERTICAL SLICE (MF-4)**:
  board→seal→rail(loopback)→verify→offload→manifest receipt, exactly-once + dead-letter, as a passing
  end-to-end integration test.

- 2026-06-22 — 🎉 **MF-4 ACHIEVED: first vertical slice GREEN (`780053e`).** `datarail-acceptance` e2e test:
  `SourceTerminal::board` → seal → `LoopbackSubstrate` (rail) send/recv → `DestTerminal::offload` = **Delivered**
  (records committed once); the **offline manifest delivery-proof verifies** (BLK-8 `cofre_id` recompute);
  **replay the same cofre → Duplicate** (exactly-once, sink unchanged); **tampered carga → DeadLettered**
  (siding, never committed). **The whole pipe moves data end-to-end: exactly-once, tamper-rejected, provable.**
  Full workspace: **8 crates, 48 tests green, clippy deny(all+pedantic) clean.** Proven: AC-1,2,3,4,5,6,9 +
  exactly-once + the end-to-end slice. **NEXT (P5 surface → P6):** datarail-spec (`rail.toml`) + datarail-cli
  (run/validate/keygen/ticket/verify) → then GATE-FEATHER/GATE-LATENCY measurement, AC-8 (WAN/resume — needs a
  network substrate beyond loopback), AC-10 fairness benchmark, final 6-lens audit over the CODE. Standing
  PENDING/STOP-THE-LINE unchanged (X25519 key-wrap, GATE-WARP on VAES HW, AC-10 engines, trio §6 ratification).

- 2026-06-22 — **P5 SURFACE COMPLETE: datarail-spec (`4214837`) + datarail-cli (`a22e3ec`).** spec: a
  **zero-dep** (Charter *leveza*) minimal-TOML-subset `rail.toml` parser → typed `RouteSpec/ContractSpec/
  KeysSpec` + builders, validate-on-parse, 11 tests. cli: the **`datarail` binary** — `validate / keygen / run
  / verify / ticket`, hand-rolled arg dispatch (no clap), `/dev/urandom` keygen (no rand), 5 tests. **Live e2e
  demo (`examples/rail.toml`): `run` boards 2 records → rail → offloads `Delivered` (committed=2); `verify`
  accepts the good cofre ✓ and rejects a tampered one (exit nonzero).** Full workspace: **10 crates, 64 tests
  green, clippy deny(all+pedantic) clean, no `#[allow]`, `forbid(unsafe)`.** The product is usable end-to-end:
  a config + a binary that moves data exactly-once, provably, tamper-rejected. **NEXT: P6** — close the X25519
  per-cofre key-wrap (the main v1 simplification), GATE-FEATHER/GATE-LATENCY micro-bench (criterion; honest HW
  labels), AC-8 (WAN/resume — needs a network substrate; likely PENDING), AC-10 fairness (PENDING engines),
  final 6-lens audit over the CODE + DoD. Standing PENDING/STOP-THE-LINE unchanged (GATE-WARP VAES HW, AC-10
  engines, trio §6 ratification).

- 2026-06-22 — **P6: X25519 per-cofre key-wrap WIRED END-TO-END (`856502f` + re-freeze `822ce2b`).** The main
  v1 simplification is **CLOSED**. `datarail-crypto` gained `seal_key`/`open_key`/`x25519_public` (ECDH-derived
  fresh data key, KDF `dr:keywrap:v1`); `Etiqueta` grew `eph_pk` (core seam, `ETIQUETA_LEN` 181→213); `board`
  seals a fresh per-cofre data key to the route dest's X25519 pubkey (random `/dev/urandom` ephemeral),
  `offload` re-derives it; added a route-binding dead-letter (`RouteMismatch`). **Forward-secure +
  provider-blind**: the data key never travels and only the dest secret opens it. Seam re-frozen (`22d1f9e`).
  Full workspace **10 crates, 65 tests green**, clippy deny(all+pedantic) clean, no `#[allow]`,
  `forbid(unsafe)`; live CLI demo Delivered + verify OK. **NEXT: P6 remainder** — criterion micro-bench
  (GATE-LATENCY/throughput, honest HW labels), AC-8 (WAN/resume — needs a network substrate; likely PENDING),
  AC-10 (PENDING engines), AUDIT-02 (6-lens over the CODE) + DoD.

- 2026-06-22 — **P6 AUDIT + DoD COMPLETE.** 6-lens adversarial code audit (parallel read-only clones, every
  finding lead-cold-verified) → `docs/design/AUDIT-02.md`: **all 6 INV-* hold**; crypto / parse / effectively-once
  / dead-letter cores confirmed sound. **5 findings FIXED** (`e74e75c`): [HIGH] random nonce (was `seq`-derived
  → fork/snapshot `(key,nonce)` reuse risk), [MED] redacted `Debug` on every secret-bearing struct, [LOW-MED]
  `unframe_batch` alloc cap, [MED] AC-4 DST now fires GC (`gc_lag=3`), [LOW] corrected `gc_lag` doc; 3
  hardening-TODOs tracked (zeroize, AAD-binding, `Gcm256` gate). **DoD ledger** → `docs/design/DOD-01.md`:
  AC-1,2,3,4,5(¬TSA),9 + 8/9 INV-* **PROVEN**; AC-6 (harness+2 substrates) & AC-8 (kill-resume) PROVEN, their
  real-substrate / WAN-bench halves post-v1; AC-7 / GATE-LATENCY / GATE-FEATHER **DIRECTIONAL**; **AC-10 +
  GATE-WARP bound PENDING** (external engines #20, VAES HW #13). 10 crates · 68 tests · clippy deny(all+pedantic)
  clean · forbid(unsafe) · no `#[allow]`. **v1 is COMPLETE + AUDITED**; open items are only the honestly-external
  blockers + the MF-0 trio ratification. (Env: rustup proxy broke mid-session → verified via the toolchain path;
  see AUDIT-02 env note.)

## §6 — STOP-THE-LINE / owner-ratification log

- 2026-06-21 — **Owner: ratify these 3 audit-driven reconciliations to the DRAFT trio (`DECOMPOSITION.md`) at MF-0.**
  They do **not** change the demand intent — they fix an over-specified mechanism + a faulty test the audit caught.
  (To be applied to the DRAFT next iteration; flagged here because the trio is owner-reserved.)
  1. **A3** → "pluggable AEAD, default AES-256-GCM-SIV (misuse-resistant), per-cofre fresh key + random nonce;
     ChaCha20/AES-GCM alternates" (was "GCM-SIV/XChaCha + derived nonce"). [BLK-1]
  2. **AC-1 metamorphic** → opacity = independence from *plaintext* (swap payload for another validly-sealed
     ciphertext of equal length, re-signed, routing fixed → identical rail behavior); was "zero payload →
     byte-identical", which contradicts `cofre_id=BLAKE3(CARGA)`. [BLK-3]
  3. **C1** → `idempotency_key` preimage = `record_key`, not `content`. [BLK-2]
