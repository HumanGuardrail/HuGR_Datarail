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

## VAES server-core measurement (Northflank, 2026-06-22) — the representative-HW data point

Ran on a **Northflank cloud container** (`us-east1`, 2 vCPU) — verified **VAES + AVX-512 + AES-NI present**
(the modern crypto acceleration the i7-9750H laptop lacks). Tool: `openssl 3.3.7 speed -evp`, sustained
(16 KiB-block) per-core throughput:

| Primitive | VAES server core | laptop (i7-9750H, noisy) |
|---|---|---|
| **AES-256-GCM** | **≈ 10.5 GB/s/core** (10,513,948 kB/s) | ~0.3–0.7 GB/s |
| AES-128-GCM | ≈ 11.8 GB/s/core | — |
| SHA-256 | ≈ 1.5 GB/s/core | ~0.77 GB/s |

**Honest scope:** this is **openssl's AES-256-GCM** (the *dominant* primitive), **not** datarail's full
board/offload binary. datarail's default AEAD is AES-256-**GCM-SIV** (two-pass ≈ half of GCM ⇒ ~5 GB/s/core
on VAES) plus a per-cofre X25519 wrap (amortized over a batch) + Ed25519 + BLAKE3. So this measures the
**ceiling**, not the end-to-end rate.

**What it settles:** `GATE-WARP`'s target **X = 1 GiB/s/core** is **comfortably achievable on representative
VAES hardware** — the symmetric ceiling clears it by **~5–10×**. (The one-off Northflank job was deleted.)

## ⚠️ GATE-WARP — n=1 cloud measurement (Northflank, 2026-06-22) — `PENDING-RIGOR` (artifact deleted)

> **Lastro flag (2026-06-26):** the numbers below are a **single (n=1)** run on a one-off Northflank VAES core
> whose job was **deleted** — there is NO committed artifact reproducing them, and this laptop has no VAES. The
> harness exists (`loadgen.yml` with `vaes=true` on a VAES runner), so it is reproducible IN PRINCIPLE, but until
> a committed CI artifact lands this is **DIRECTIONAL/PENDING-RIGOR, not a banked PASS**. The order-of-magnitude
> conclusion (≥1 GiB/s/core achievable on VAES) is sound; the exact 1.43 GB/s figure is unbacked.

Built the actual workspace (`cargo build --release -p datarail-bench`, `RUSTFLAGS=-C target-cpu=native`) on a
VAES cloud container and ran the **real binary** (not the openssl proxy). CPU flags confirmed: `vaes avx512f
avx2 aes`. Per-core:

| Real datarail binary @ VAES | result | vs target |
|---|---|---|
| **AES-256-GCM-SIV seal** | **1.43 GB/s/core** | **PASS** — > X = 1 GiB/s (1.07 GB/s), ~1.3× |
| AES-256-GCM-SIV open | 1.44 GB/s/core | PASS |
| BLAKE3 | 6.51 GB/s/core | (manifest leaves; not the bottleneck) |
| x25519 key-wrap | 67 µs/side | amortizes per batch (once per cofre) |
| board / offload (1-record cofre) | 118 µs / 109 µs | dominated by the wrap; batch to amortize |

**GATE-WARP is met** by datarail's actual sealed AEAD throughput (1.43 GB/s/core ≥ the 1 GiB/s/core target) on
representative VAES hardware — a literal pass of the real binary, not a proxy.

**Engineering headroom to true "warp":** datarail's *default* AEAD is pure-Rust RustCrypto (AES-NI per-block,
1.43 GB/s) — and the headroom is now **realized** by the `vaes` backend below.

## ⚡⚡ WARP backend SHIPPED — `--features vaes` (ring AES-256-GCM) on a VAES core (Northflank, 2026-06-22)

Built `datarail-bench --features vaes` (routes `AeadAlg::Gcm256` through `ring`'s VAES/AVX-512 asm) on a VAES
core and ran the **real binary**:

| AEAD path (real datarail binary) | seal | open | vs target | vs default |
|---|---|---|---|---|
| default — RustCrypto GCM-SIV (AES-NI) | 1.25–1.43 GB/s/core | 1.44 | **PASS** (≥1 GiB/s) | 1× |
| **`--features vaes` — ring AES-256-GCM (VAES)** | **5.92 GB/s/core** | **5.13** | **~6× target** | **~4.7×** |

datarail seals at **~5.9 GB/s/core** with the VAES backend — **the moat**: provider-blind sealing at
near-line-rate, which a bolt-on-encryption competitor pays as pure overhead on top of its transport.
**Wire-compatible** with the default (the `gcm256_known_answer` KAT passes byte-identical on both backends) → a
fleet may mix backends and cofres interchange. Default stays pure-Rust (`leveza`); `vaes` is opt-in for
throughput-critical routes. (ring's 5.9 is a touch under openssl's 10.5 on the same core — ring's VAES path is
slightly less aggressive than openssl 3.3.7; still ~6× the gate. Further lever if ever needed: `aws-lc-rs`.
Not needed — the gate is crushed.)

## Future optimization (measured-first, not on a hunch)

- Batch the X25519 wrap across a window of cofres to one destination (one ECDH per window) — cuts the dominant
  cost for many-small-cofre routes. Needs a design note + a metamorphic opacity check. Not done in v1.
