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
  (size/timing/idem-token). **AEAD decision:** AEAD is pluggable; **default = single-pass AES-256-GCM with the
  nonce DERIVED from the idempotency key** — distinct plaintext → distinct nonce (no catastrophic GCM reuse);
  identical plaintext → identical ciphertext (the intended per-tenant dedup); fast on VAES. **GCM-SIV retained
  as a hardened option.** **batch-many-records-per-cofre LOCKED** (one Ed25519 sig amortized per lote).
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
