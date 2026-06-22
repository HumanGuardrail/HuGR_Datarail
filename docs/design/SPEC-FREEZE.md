# SPEC-FREEZE — MF-2

> The SPEC is **frozen**. From here, "what datarail is, at the design level" is a **byte-pinned object** — any
> drift is detectable by re-running one `shasum`. Nothing in the frozen set changes without a new freeze + a
> logged reason (rigor compact).

- **Git HEAD at freeze:** `c0026da89cdcb5a0448f57fe207241cc1a9f9dfb`
- **Content hash (sha256 of the concatenated frozen set, sorted):**
  `5f88095fb370e4de3315653c246ceaece58a99d787ae6a9cd0a41a03e0bb6d89`
- **Size:** 13 docs · 705 lines.
- **Reproduce:**
  ```
  ls docs/design/*.md | grep -vE 'AUDIT-01|SPEC-FREEZE' | sort | tr '\n' '\0' | xargs -0 cat | shasum -a 256
  ```

## What is frozen

`docs/design/00-CONSTITUTION` … `11-gates-proof-methods` + `99-COVERAGE-MATRIX` — the machine shape (CAST), the
named invariants + Craft Charter, the cofre/crypto/manifest/effectively-once/terminal/rail/routing/CLI/store
designs, the gates & proof methods, and the zero-unowned coverage matrix. `AUDIT-01.md` and this file are
excluded from the hash.

## How it earned the freeze

- **Coverage matrix:** zero unowned rows (re-validated post-audit).
- **6-lens adversarial audit (AUDIT-01):** 8 blockers + 17 majors found, all cold-verified real, all closed —
  including a self-contradicting AC-1 test, a signer-substitution forgery, missing domain separation, a
  dedup-replay double-commit race, and parse-before-verify.
- **Re-audit (freeze gate):** FREEZE-READY, zero new drift, evidence cited per doc.

## ⚠️ Conditional on MF-0 (owner ratification — STOP-THE-LINE)

This freeze is engineering-clean but **conditional on the owner ratifying the 3 DRAFT-trio reconciliations in
`../BUILD_LOG.md` §6** (A3 AEAD→GCM-SIV; AC-1 metamorphic redefinition; C1 idempotency=`record_key`). These do
not change the demand intent. If the owner overrides any, the affected SPEC docs + this hash are revised and
re-frozen with a new stamp.

## Labeled PENDING (honesty taxonomy — not freeze blockers)

- **`GATE-WARP` bound** — needs a representative VAES-HW re-bench (must be set before MF-4).
- **AC-10 fairness benchmark** — needs the external comparison engines installed.

Next: ROADMAP (derived from this frozen SPEC) → MF-3 contract freeze (cofre/header/manifest IDL + terminal
seam) → Kage-Bunshin fan-out build.
