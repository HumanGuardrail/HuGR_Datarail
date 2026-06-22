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

## §4 — Running log (one line per meaningful step)

- 2026-06-21 — Repo initialized (branch `main`). Mission control stood up: trio (DRAFT), CONSTITUTION,
  ADR-0001, PRE-REGISTRATION, this log, README, `.gitignore`. **Next: Owner ratifies the trio (MF-0); set the
  `GATE-WARP` target; then fire the H1 + H3 spike (MF-1).**
