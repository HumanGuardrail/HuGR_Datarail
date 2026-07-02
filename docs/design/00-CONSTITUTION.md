# 00 — THE DESIGN CONSTITUTION

> The shape of the machine, chosen and frozen before features. **Status: DRAFT — pending Owner ratification.**
> Architecture decision recorded in [`../adr/0001-adopt-cast.md`](../adr/0001-adopt-cast.md).

## The shape: CAST — Content-Addressed Sealed Transport

A fixed-route, ephemeral, provider-blind transport. Four pillars:

1. **The sealed cofre is the universal unit; its seal is its identity.** Everything is a cofre; identity =
   the seal (a content address). Dedup, idempotency, replay, and integrity all key off it. Content
   addressing is used as *identity*, never as *routing* — routes are fixed (pillar 3).
2. **Smart endpoints, dumb pipes.** All intelligence and all secrets live in the terminal (a featherweight
   library). The rail is a polymorphic, dumb, ephemeral substrate that moves a cofre using its
   authenticated header alone.
3. **Fixed A→B routes; no discovery.** A terminal addresses a *route*, not a dynamically resolved network
   location. Fan-out 1→N = N fixed routes. (Deliberately *not* an overlay/DHT — that complexity is designed out.)
4. **Provable, ordered, effectively-once delivery as a header-only protocol.** Sequence, idempotency, and
   the manifest ride in the authenticated header; the rail enforces them without ever opening a cofre.

## The named invariants

> A wish you cannot mechanically check is not an invariant. Each `INV-*` is paired with its falsifier and
> will be referenced in code where it is enforced.

- **INV-OPAQUE-CARGO** — every rail function computes from header + seal alone; the payload is opaque to the
  rail. *Falsifier:* any rail function that needs plaintext. *(→ hypothesis H1.)*
- **INV-SEAL-COMPLETE** — the seal covers every byte whose change can matter (header ⊗ payload); no field is
  silently mutable. *Falsifier:* a field outside the seal whose change alters behavior. *(The soundness rule
  — the first invariant we hunt, the analogue of Keepr's `INV-CAD-KEY-COMPLETE`.)*
- **INV-TAMPER-REJECT** — a cofre failing verification is never offloaded → dead-letter. *Falsifier:* a
  tampered/forged cofre that lands.
- **INV-EFFECTIVELY-ONCE** — acked-at-onboarding ⇒ offloaded exactly once under crash/retry/partition with an
  idempotent sink; idempotency key in the header. *Falsifier:* a chaos sequence that loses or duplicates.
- **INV-MANIFEST-RECONCILES** — every shipment yields a signed manifest enabling offline source ⊕ destination
  reconciliation; the manifest is itself tamper-evident. *Falsifier:* an undetected missing/extra cofre.
- **INV-CONTRACT-SPLIT** — the envelope contract is enforced only at the rail (header-only); the content
  contract only at the terminals (plaintext). Neither depends on the other. *Falsifier:* the rail needing
  content, or a terminal trusting the rail for content validation.
- **INV-EPHEMERAL-RAIL** — the rail holds zero durable state and zero standing presence; it spawns per
  shipment and exits. *Falsifier:* any rail component that must persist between deliveries to function.
- **INV-SUBSTRATE-POLYMORPHIC** — the terminal API is identical across substrates (shmem/QUIC/object store).
  *Falsifier:* a terminal that behaves differently per substrate.
- **INV-DUMB-PIPE** — the rail/substrate holds zero secrets and zero intelligence; it can run on fully
  untrusted infrastructure. *Falsifier:* any rail component that must be trusted with content or keys.

## The Craft Charter (FROZEN — Owner-mandated; inviolable; no waiver without the Owner)

Performance is **designed-in, measured, and gated** — never hand-tuned on a hunch.

**Performance laws**
- **L1 — Absurd bars, gated in CI:** `GATE-WARP` (throughput), `GATE-LATENCY` (p50/p99 per hop),
  `GATE-FEATHER` (idle ≈ 0; active footprint ≪ a mesh sidecar). A regression past the bound fails the build
  at the root. Gate the **BOUND**, never flatness.
- **L2 — No number before its artifact:** every perf claim is `measured` (criterion bench in hand) or
  labeled `bet`. No bare claims.
- **L3 — Measure before you cut:** never optimize what you haven't profiled; the measurement picks the
  lever, never a hunch.
- **L4 — Performance is architecture-first:** the biggest wins live in the shape (zero-copy of ciphertext,
  no mandatory broker hop on the rail path, scale-to-zero, hardware-accelerated AEAD).
- **L5 — Hot paths are allocation-aware:** zero/pooled allocation where it counts, gated by alloc-count asserts.

**Craft laws**
- **C1 — `#![forbid(unsafe_code)]` by default;** `unsafe` only with an ADR + a safety argument + a test.
- **C2 — No GodFile, no gambiarra:** LOC caps per file/function, `clippy` at `deny` (pedantic), `rustfmt`,
  every public item documented, every `INV-*` referenced where it is enforced.
- **C3 — Clarity is craft:** the work of art is the simplest thing that is also the fastest. If a line needs
  a paragraph to explain, the line loses.
- **C4 — No `#[allow]`, no `--no-verify`, no "fix later":** a failing gate is fixed at the root.

> **The discipline:** the obsession is channeled by measurement + gates, not undisciplined micro-tuning. And
> art at fleet scale = art in the **frozen design + mechanical gates** — never hoping each agent has taste.

## Structural enforcement (detailed in the SPEC at MF-2)

- A crate dependency DAG with **no terminal↔terminal** and **no substrate↔substrate** edges — so Part III
  work-packages are conflict-disjoint and buildable in parallel.
- LOC caps and the gates above encoded into CI, not prose. An invariant in prose is a wish.
