# BENCH-01 — directional micro-benchmarks

> **Honesty label (THE HUGR METHOD "no number before its artifact"):** every figure below is **DIRECTIONAL**
> — order-of-magnitude only, **not benchmark-grade and NOT a GATE-WARP result.**

## Machine (verified 2026-06-22 via `sysctl`)

- **Native Intel Core i7-9750H** (Coffee Lake, 2019), `x86_64-apple-darwin`, macOS. **Not** Rosetta
  (`sysctl.proc_translated` absent ⇒ native), **not** Apple Silicon. Crypto features: **AES-NI + AVX2**, but
  **pre-VAES** (no AVX-512 / VAES). This is a laptop, run under variable load + thermal throttling.
- **Correction log (honesty):** an early revision wrongly labeled this "Apple Silicon arm64 / NEON"; a later
  one wrongly "Rosetta-x86_64 (emulated)". **Both were wrong** — I inferred from slow numbers instead of
  checking the CPU. It is a native Intel i7-9750H (the same chip as the H3 spike, BUILD_LOG §3). Fixed
  everywhere 2026-06-22.
- **Why GATE-WARP is still pending here (for the *right* reason):** `GATE-WARP` is specified against
  representative **VAES** server hardware; this 2019 mobile chip genuinely has no VAES, so its throughput
  neither passes nor fails the gate — it is simply not the target HW (task #13). A modern server core (e.g. a
  cloud VM) would give the representative number.

## Harness & the variance problem

`crates/datarail-bench` — a zero-dependency `std::time::Instant` loop (Charter *leveza*; no criterion).
**Two `--release` runs on this same laptop disagreed by 2–3×** (e.g. `board` 1.45 ms vs 3.62 ms; GCM-SIV seal
328 vs 687 MB/s; x25519 685 vs 1036 µs/op). That swing is the honest headline: **on a loaded laptop with a
rough timer, the absolute numbers are not trustworthy.** A real measurement needs a quiet, representative
machine **and** a proper statistical harness (criterion). Reproduce: `cargo run --release -p datarail-bench`.

## Measured (two runs, to show the spread — DIRECTIONAL only)

| Benchmark | run A | run B | what it is |
|---|---|---|---|
| `blake3_256` (64 KiB) | 791 MB/s | 770 MB/s | content-address + manifest leaves |
| AES-256-GCM-SIV seal (64 KiB) | 328 | 687 MB/s | default AEAD (two-pass SIV) |
| AES-256-GCM-SIV open (64 KiB) | 256 | 794 MB/s | |
| `x25519 seal_key` (source) | 685 | 1036 µs/op | per-cofre key-wrap |
| `x25519 open_key` (dest) | 535 | 1356 µs/op | per-cofre key-unwrap |
| `cofre encode` | 2.24 | 4.99 µs/op | wire serialization |
| `cofre decode` | 0.50 | 1.78 µs/op | parse-before-verify cursor |
| `board` end-to-end (1 record) | 1.45 | 3.62 ms/op | validate → wrap → AEAD → sign (+ entropy) |
| `offload` end-to-end (1 record) | 0.94 | 2.56 ms/op | verify → unwrap → open → admit → commit |
| `LoopbackSubstrate` round-trip | 1.2 | 0.31 µs/cofre | in-process queue |
| TCP loopback round-trip | 124 | 354 µs/cofre | real kernel hop on `127.0.0.1` |

## Honest interpretation (shape, not absolutes)

- **The per-cofre hot path is asymmetric-crypto-bound, not symmetric-bound.** The X25519 key-wrap dominates
  `board`/`offload`; BLAKE3/AEAD are negligible for small records. **By design** (per-cofre forward-secure keys),
  and it **amortizes over batch size** — the wrap is once per cofre, not per record (a 1000-record batch pays it
  once). This *shape* is the reliable finding; the absolute µs are not.
- **GATE-LATENCY:** not a pass here (board is ~1–4 ms on this throttling laptop, dominated by X25519). Formal
  p50/p99 needs representative HW (#13).
- **GATE-WARP:** PENDING — needs a representative VAES server core (#13). Nothing here passes or fails it.
- **GATE-FEATHER:** architectural (idle ≈ 0 by construction); see the `gate_feather_*` test. A real serverless
  idle-RSS is a deployment measurement.

## Future optimization (measured-first, not on a hunch)

- Batch the X25519 wrap across a window of cofres to one destination (one ECDH per window) — cuts the dominant
  cost for many-small-cofre routes. Needs a design note + a metamorphic opacity check. Not done in v1.
