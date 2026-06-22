# BENCH-01 — directional micro-benchmarks

> **Honesty label (Craft Charter L? / THE HUGR METHOD "no number before its artifact"):** every number below
> is **DIRECTIONAL**, measured on **dev hardware**, in `--release`. It is **NOT a GATE-WARP result.**

- **Machine:** target `x86_64-apple-darwin`, macOS (`uname -m` = `x86_64` — a real Intel Mac *or* an x86_64
  toolchain under Rosetta 2; the two are indistinguishable from `uname`, and Rosetta notably lacks AVX/VAES).
  AES via AES-NI, BLAKE3 via SSE/AVX2 — or their Rosetta-translated equivalents. **(Correction 2026-06-22: an
  earlier revision wrongly attributed these to "Apple Silicon arm64 / ARMv8 crypto extension / NEON"; the
  toolchain is x86_64. The figures below are unchanged DIRECTIONAL numbers and warrant a re-confirmation run on
  the verified target — task #13.)** This is **not** the representative **VAES** hardware `GATE-WARP` is
  specified against (Rosetta has no VAES; a real Intel Mac's VAES is unconfirmed), so throughput here neither
  passes nor fails GATE-WARP — that gate stays **PENDING** representative hardware (task #13).
- **Harness:** `crates/datarail-bench` — a zero-dependency `std::time::Instant` loop (Charter *leveza*; no
  criterion). Warm-up then N iterations; latency = wall-ns / iters, throughput = bytes/µs (== MB/s).
- **Reproduce:** `cargo run --release -p datarail-bench`
- **Commit:** recorded at the P6 bench commit (see BUILD_LOG §4).

## Measured — fresh `--release` run, 2026-06-22 (target `x86_64-apple-darwin`)

> ⚠️ **These numbers are ~4–7× slower than this doc's previous figures.** That divergence is strong evidence
> the box is **Rosetta-2 x86_64** (emulated; no AVX/VAES) and the earlier figures were **native arm64** — a
> *different environment*, not a regression. Neither is the GATE-WARP target; both are **DIRECTIONAL**.
> Recorded under the verified toolchain (cargo 1.96.0 x86_64) per the env note. "No number before its artifact"
> (Charter): the current verifiable numbers are the x86_64 column; the arm64 column is retained for contrast.

| Benchmark | x86_64 / Rosetta (now) | prior (native arm64) | Notes |
|---|---|---|---|
| `blake3_256` (64 KiB) | **791 MB/s** | ~3380 | content-address + manifest leaves |
| AES-256-GCM-SIV seal (64 KiB) | **328 MB/s** | ~1050 | default AEAD; no VAES under Rosetta |
| AES-256-GCM-SIV open (64 KiB) | **256 MB/s** | ~1050 | |
| `x25519 seal_key` (source) | **685 µs/op** | ~91 | per-cofre key-wrap (emulated scalar mult is brutal) |
| `x25519 open_key` (dest) | **535 µs/op** | ~93 | per-cofre key-unwrap |
| `cofre encode` | **2.24 µs/op** | ~0.40 | wire serialization (213-byte etiqueta + carga) |
| `cofre decode` | **0.50 µs/op** | ~0.11 | parse-before-verify cursor |
| `board` end-to-end (1 record) | **1.45 ms/op** | ~190 µs | validate → X25519 wrap → AEAD → Ed25519 sign (+ entropy) |
| `offload` end-to-end (1 record) | **0.94 ms/op** | ~136 µs | verify → X25519 unwrap → AEAD open → admit → commit |
| `LoopbackSubstrate` round-trip | **1.2 µs/cofre** (~829 k/s) | — | in-process queue (AC-8 #26 round-trip bench) |
| TCP loopback round-trip | **124 µs/cofre** (~8.1 k/s) | — | real kernel hop on `127.0.0.1`; incl. busy-spin recv |

## Honest interpretation

- **The hot path is asymmetric-crypto-bound per cofre, not symmetric-bound.** The X25519 key-wrap (~0.5–0.7 ms/side
  *under Rosetta emulation*; ~90 µs/side native arm64) dominates `board`/`offload`; BLAKE3/AEAD are negligible
  for small records. **By design** (per-cofre forward-secure keys) and it **amortizes over batch size**: the wrap
  is **once per cofre**, not per record — a cofre carrying a 1000-record `RECORD_BATCH` pays it once.
  High-throughput deployments batch; latency-sensitive single-record routes pay the full per-cofre cost.
- **GATE-LATENCY (per-cofre):** board ~**1.45 ms** / offload ~**0.94 ms** on this **Rosetta-x86_64** box — i.e.
  **NOT sub-millisecond here**, dominated by emulated X25519. Native arm64 previously measured ~190 µs / ~136 µs
  (sub-ms). So GATE-LATENCY is **DIRECTIONAL, NOT a pass on this box**; its formal target needs representative
  native / VAES HW (#13). The *shape* (asymmetric-bound, batch-amortizing) holds on both.
- **GATE-WARP (throughput):** **PENDING** — must be measured on representative VAES hardware (#13). This box
  reports x86_64 but VAES is unconfirmed (Rosetta has none; a real Intel Mac may or may not), so these numbers
  are not a GATE-WARP comparison either way.
- **GATE-FEATHER (idle ≈ 0):** **not measured here** — it is an architectural property of the *ephemeral*
  substrate (scale-to-zero; spawn-deliver-vanish), not a micro-bench. The `LoopbackSubstrate` holds no idle
  resources; a real serverless substrate's idle cost is a deployment measurement (out of v1 scope).

## Possible future optimizations (measured-first, not on a hunch)

- Batch the X25519 wrap across a window of cofres to the same destination (one ECDH per window) — would cut the
  dominant cost for many-small-cofre routes. Needs a design note + a metamorphic check that it preserves
  per-cofre opacity. **Not done in v1** (correctness + the clean per-cofre model first).
