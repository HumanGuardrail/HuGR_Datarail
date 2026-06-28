# KAFKA-COMPRESSION-DESIGN — accept compressed producer batches

> **STATUS: DESIGN — ratified by the TechLead (owner-delegated). Build incrementally, one codec per step, each
> green + proven against real librdkafka.**

## Why
Real Kafka producers very commonly enable compression (`compression.type=gzip|snappy|lz4|zstd`) — it is on by
default in many client stacks. Today datarail **rejects** any compressed batch (`produce.rs::parse_v2_records`:
`attributes & 0x07 != 0 → "compressed batches not supported"`). That is the single biggest *drop-in* wall: a
producer with default compression simply fails. Closing it makes datarail accept the producers people actually run.

## Where decompression lives (the key question — datarail-kafka is zero-dep)
A compressed v2 `RecordBatch` has an **uncompressed header** (baseOffset … recordCount) followed by a
**compressed records blob** (the codec is `attributes & 0x07`). Decompression is intrinsic to *parsing* the batch —
it cannot be cleanly pushed out of the crate the way TLS was (TLS sat below the framing; compression sits inside it).

**DECISION (ratified): a feature-gated `compression` ON `datarail-kafka`.** Off by default, the crate stays
dependency-free and rejects compressed batches exactly as today. With `--features compression`, the crate pulls
**pure-Rust, decompress-only** codec crates and decompresses the records blob in place. This mirrors the `quic`/`tls`
feature-gating precedent (heavy/extra deps one flag away; default stays featherweight). `forbid(unsafe)` is
per-our-crate, so the codecs' internal `unsafe` is theirs, not ours. **Dep WAIVER logged in BUILD_LOG.**

Rejected alternative — a `Decompressor` trait injected from the CLI (like `ConnWrap`): awkward, because the parse
is in-crate and would have to hand the raw blob + codec id out and back; it buys nothing over a clean feature gate.

## The Kafka per-codec framing (the gotchas — claims ≤ proof)
The records blob is NOT always the "raw" codec stream. Per codec:
- **gzip (codec 1)** — standard gzip stream. Clean. → `flate2` (default `miniz_oxide` backend = pure Rust).
- **lz4 (codec 3)** — the **LZ4 frame format** (not raw block). → `lz4_flex` frame API. (Historical note: very old
  Kafka had an lz4 frame-checksum bug; modern librdkafka is correct — we target correct frames.)
- **zstd (codec 4)** — standard zstd frame. → `ruzstd` (pure-Rust zstd **decoder**; the `zstd` crate is C-bound —
  avoid, like we avoid aws-lc-rs).
- **snappy (codec 2) — THE GOTCHA**: Kafka does NOT use the standard snappy stream/frame format; it uses the
  **xerial / snappy-java block framing** — an 8-byte magic header `\x82SNAPPY\x00` + version/compat int32s, then a
  sequence of `[int32 block_len][raw-snappy block]` chunks. So snappy = custom xerial de-framing + `snap` raw-block
  decode. Scoped LAST because of this.

## Scope / non-goals
- **DECOMPRESS on produce** only: the producer's compressed batch → decompress → seal each record individually (the
  records become normal sealed cofres; nothing about storage changes). **Fetch keeps returning UNCOMPRESSED** record
  batches — every consumer accepts uncompressed, so there is no need to re-compress on the read path (documented
  honestly; re-compression for fetch-bandwidth is a possible later optimization, not correctness).
- Bound the decompressed size (a zip-bomb guard): cap the decompressed records blob (e.g. ≤ `MAX_FRAME`) so a small
  compressed batch can't expand to exhaust memory — a real parse-safety concern for an untrusted producer.
- No change to the legacy `MessageSet` path beyond rejecting its compression (rare; tracked).

## Mechanics (what changes in `produce.rs`)
- `parse_record_batch` already reads `batchLength`; pass it (or the batch-end offset) into `parse_v2_records` so the
  compressed blob can be bounded (records blob = batch end − current position).
- `Reader` gains a `take(n)` / remaining-slice accessor (zero-copy borrow) to lift the compressed bytes out.
- `parse_v2_records`: when `codec = attributes & 0x07` is non-zero AND the `compression` feature is on, read the
  records blob, `decompress(codec, blob, cap)` → a `Vec<u8>`, then run the existing record loop over a `Reader` on
  the decompressed bytes. Feature OFF → the current clear error (unchanged default behavior).
- A new `compress` module (feature-gated) with `fn decompress(codec: u8, input: &[u8], max: usize) -> io::Result<Vec<u8>>`
  dispatching per codec, each behind its own `#[cfg]` so the build only pulls the codecs compiled in.

## Build plan (each step green + a real-librdkafka-compression CI proof)
1. **This doc + ratification.** ✅
2. **Incr 1 — gzip** (the cleanest): `Reader::take` + `batchLength` passthrough + the `compress` module with gzip;
   unit test (compress a known blob with flate2, parse it back). Default build unchanged.
3. **Incr 2 — lz4 + zstd** (clean frames): add `lz4_flex` + `ruzstd` codecs + unit tests.
4. **Incr 3 — snappy (xerial)**: the xerial de-framing + `snap` raw blocks + unit test with a real xerial blob.
5. **Incr 4 — real-client CI**: `kafka-broker-compression.yml` (mirrors `kafka-broker-librdkafka.yml`) — for each of
   gzip/lz4/zstd/snappy, kcat `-X compression.codec=<c>` PRODUCE → consume back + on-disk-ciphertext assertion.
   **No PROVEN claim per codec until its CI leg is green.**
6. **Docs**: KAFKA-COMPAT (compression now supported + which codecs), README, LASTRO, BUILD_LOG.

## Open questions (TechLead-ratified)
- **Q1 — which codecs ship in the default `compression` feature?** All four (gzip/lz4/zstd/snappy) once proven, since
  a producer can pick any. Each is also a sub-feature so a size-sensitive build can pick a subset. → ship all four
  under `compression`, with `compression-gzip` / `-lz4` / `-zstd` / `-snappy` sub-features.
- **Q2 — re-compress on Fetch?** No (v1). Consumers accept uncompressed; it's a bandwidth optimization only. Tracked.
- **Q3 — zip-bomb cap?** Yes — cap decompressed size at `MAX_FRAME` and error past it (parse-safety, audit-aligned).
