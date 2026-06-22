# BENCH-01 — directional micro-benchmarks

> **Honesty label (Craft Charter L? / THE HUGR METHOD "no number before its artifact"):** every number below
> is **DIRECTIONAL**, measured on **dev hardware**, in `--release`. It is **NOT a GATE-WARP result.**

- **Machine:** Apple Silicon (arm64), macOS — AES via the ARMv8 crypto extension, BLAKE3 via NEON.
  This is **not** the x86-64 **VAES** hardware `GATE-WARP` is specified against, so throughput here neither
  passes nor fails GATE-WARP — that gate stays **PENDING** representative hardware (task #13).
- **Harness:** `crates/datarail-bench` — a zero-dependency `std::time::Instant` loop (Charter *leveza*; no
  criterion). Warm-up then N iterations; latency = wall-ns / iters, throughput = bytes/µs (== MB/s).
- **Reproduce:** `cargo run --release -p datarail-bench`
- **Commit:** recorded at the P6 bench commit (see BUILD_LOG §4).

## Measured (one representative run)

| Benchmark | Result | Notes |
|---|---|---|
| `blake3_256` (64 KiB) | **~3380 MB/s** | content-address + manifest leaves; not the bottleneck for large carga |
| AES-256-GCM-SIV seal (64 KiB) | **~1050 MB/s** | default AEAD (nonce-misuse-resistant; two-pass SIV) |
| AES-256-GCM-SIV open (64 KiB) | **~1050 MB/s** | |
| `x25519 seal_key` (source) | **~91 µs/op** | 2 scalar mults + KDF — the per-cofre key-wrap cost (source) |
| `x25519 open_key` (dest) | **~93 µs/op** | per-cofre key-unwrap cost (dest) |
| `cofre encode` | **~400 ns/op** | wire serialization (213-byte etiqueta + carga) |
| `cofre decode` | **~106 ns/op** | parse-before-verify cursor |
| `board` end-to-end (1 record) | **~190 µs/op** | validate → X25519 wrap → AEAD → Ed25519 sign (+ `/dev/urandom`) |
| `offload` end-to-end (1 record) | **~136 µs/op** | verify → X25519 unwrap → AEAD open → admit → commit |

## Honest interpretation

- **The hot path is asymmetric-crypto-bound per cofre, not symmetric-bound.** The X25519 key-wrap (~90 µs/side)
  dominates `board`/`offload` latency; BLAKE3/AEAD are ~GB/s and negligible for small records. **This is by
  design** (per-cofre forward-secure keys) and it **amortizes over batch size**: the wrap is **once per cofre**,
  not per record — a cofre carrying a 1000-record `RECORD_BATCH` pays the ~90 µs once. High-throughput
  deployments batch; latency-sensitive single-record routes pay the ~190 µs.
- **GATE-LATENCY (per-cofre):** sub-millisecond end-to-end on dev HW — **DIRECTIONAL pass** (the gate's formal
  target is verified on representative HW; this corroborates the order of magnitude).
- **GATE-WARP (throughput):** **PENDING** — must be measured on representative VAES hardware (#13). Apple
  Silicon numbers are not comparable to the x86 VAES target.
- **GATE-FEATHER (idle ≈ 0):** **not measured here** — it is an architectural property of the *ephemeral*
  substrate (scale-to-zero; spawn-deliver-vanish), not a micro-bench. The `LoopbackSubstrate` holds no idle
  resources; a real serverless substrate's idle cost is a deployment measurement (out of v1 scope).

## Possible future optimizations (measured-first, not on a hunch)

- Batch the X25519 wrap across a window of cofres to the same destination (one ECDH per window) — would cut the
  dominant cost for many-small-cofre routes. Needs a design note + a metamorphic check that it preserves
  per-cofre opacity. **Not done in v1** (correctness + the clean per-cofre model first).
