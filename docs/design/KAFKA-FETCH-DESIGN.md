# KAFKA-FETCH-DESIGN — provider-blind Kafka CONSUME (the bidirectional drop-in)

> **STATUS: increment 1 BUILT + PROVEN (2026-06-27).** `datarail kafka-broker` serves Produce (seal+store) +
> Fetch (un-seal at the edge) + ListOffsets over the real Kafka wire. Built: a hand-rolled `CRC-32C` +
> `build_record_batch` (a real consumer accepts the batch), the `consume` codec (Fetch v0–4 / ListOffsets v0–2),
> `serve_broker` + the `KafkaBroker` trait, `DestTerminal::open` (read-only verify+decrypt, no dedup so re-fetch
> is idempotent), and the CLI sealed in-memory store. PROVEN: the store unit test (storage holds sealed
> ciphertext — provider-blind — and un-seals on fetch; idempotent re-fetch; bounds) + the FULL wire test
> (`kafka_broker_wire.rs`: the real binary, produce 3 → fetch back the plaintext, suffix fetch, ListOffsets → 3).
> **Honest scope:** the store is **in-memory** (provider-blind but not durable across restart) — durable
> `datarail-topic` backing is increment 2; single partition; no consumer groups; produce hop plaintext (seal-on-
> ingest). Next: increment 2 (durability) + adversarial audit of the un-seal-on-fetch path.


> Today `kafka-ingest` is one-way: a Kafka producer → datarail seals → an external sink. This adds the **consume**
> side: a `datarail kafka-broker` that STORES the sealed records in an offset log and serves the Kafka `Fetch`
> (API 1) + `ListOffsets` (API 2) APIs, so an **unmodified Kafka consumer** reads them back — **un-sealed only at
> the serving edge**, so the storage never holds plaintext. datarail becomes a Kafka drop-in whose broker storage
> is provider-blind: **seal-on-produce, store sealed, un-seal-on-fetch.**

## The guarantee shape (symmetric to ingest)
- **Produce hop** (producer → datarail): plaintext on the wire today (same "seal-on-ingest" caveat as kafka-ingest;
  TLS on this hop is a separate arc). datarail seals each record into a **cofre** immediately.
- **Storage** (the offset log): holds **only sealed cofres** — provider-blind. A snapshot of the log / the disk
  reveals nothing. This is the moat vs a normal Kafka broker (whose segments are plaintext).
- **Fetch hop** (datarail → consumer): datarail **un-seals at the edge** (it holds the dest key) and returns
  plaintext records in the Kafka `Fetch` wire format. The consumer is unmodified.

## Offsets — logical, contiguous, per `(topic, partition)`
Kafka consumers require **contiguous per-record logical offsets** (next fetch = `last_offset + 1`). The
`datarail-topic` log addresses by BYTE offset, which is non-contiguous — so we keep a **logical offset = record
index** per `(topic, partition)` (0,1,2,…). The Produce response already returns a logical base offset (the
`offsets` map). Fetch reads records `[fetch_offset ..]` by logical index.

## Increment 1 (this arc) — in-memory sealed store, Fetch + ListOffsets, single partition
- **Store:** per `(topic, partition)`, a `Vec<SealedCofre>` (the wire-encoded cofre bytes) — logical offset = the
  vec index (contiguous, Kafka-correct). It holds **ciphertext** → provider-blind. (Durable `datarail-topic`
  backing — mapping logical→byte offset — is **increment 2**; in-memory first proves the model + the wire.)
- **Produce:** seal each record (`SourceTerminal::board` → one cofre per record, or per batch then split) → push
  the sealed cofre bytes to the store → ack with the logical base offset. (Reuses the EOS/ack-after-durable shape;
  durability-before-ack lands with increment 2's persistent log.)
- **Fetch (API 1):** read the stored sealed cofres from `fetch_offset` up to `max_bytes`, **open** each via
  `DestTerminal` (un-seal at the edge) → plaintext record values → pack into a v2 `RecordBatch` (we already
  PARSE one for produce; now we BUILD one) → return in the Fetch response. A bad/forged cofre is skipped (it
  never came from us). Target Fetch **v0–v4** (non-flexible) for a clean encoding, like Produce v7.
- **ListOffsets (API 2):** earliest = 0, latest = the store's logical end (record count). v0–v2.
- **ApiVersions:** advertise Fetch(1) + ListOffsets(2) so a consumer negotiates.

## The serve seam — a `KafkaBroker` trait (request/response, unlike ingest's one-way channel)
Ingest streams produced batches one-way on an `mpsc` channel. Consume needs **request/response** (Fetch returns
data synchronously). So a new `serve_broker` loop is parameterized by a trait the CLI implements (it owns the
terminals + the store):

```rust
pub trait KafkaBroker {
    /// Seal + store a produced batch; return the logical base offset assigned.
    fn produce(&self, topic: &str, partition: i32, records: &[Vec<u8>]) -> io::Result<i64>;
    /// Un-seal + return the plaintext records from `offset` (bounded by `max_bytes`).
    fn fetch(&self, topic: &str, partition: i32, offset: i64, max_bytes: i32) -> io::Result<Vec<Vec<u8>>>;
    /// Logical earliest / latest (next) offset for ListOffsets.
    fn bounds(&self, topic: &str, partition: i32) -> (i64, i64);
}
```

The CLI's impl (behind a `Mutex`, shared across connection threads) seals on `produce` (board → cofre → store)
and un-seals on `fetch` (decode → `DestTerminal::open` → plaintext). The existing one-way `serve` (ingest) stays
untouched — this is an additive second mode (`datarail kafka-broker`).

## Honest scope / non-goals (stated, not faked)
- **Increment 1 store is in-memory** (provider-blind but not durable across restart) — durable `datarail-topic`
  backing is increment 2. We say so.
- **No consumer groups / offset-commit coordination** (`OffsetCommit`/`OffsetFetch`/group join) — the consumer
  tracks its own offset (auto.offset.reset / explicit seek). Group coordination is a later increment.
- **Single partition per topic**, uncompressed, Fetch v0–v4.
- The Produce hop stays plaintext (seal-on-ingest); TLS on the Kafka hop is its own arc.

## Build plan (each step tested; new path audited like the rest)
1. **This doc.** ✅
2. `KafkaBroker` trait + `serve_broker` loop + Fetch/ListOffsets parse+build in `datarail-kafka` (wire only,
   unit-tested with a stub broker).
3. ApiVersions advertises Fetch+ListOffsets.
4. CLI `kafka-broker`: the sealed in-memory store + seal-on-produce / un-seal-on-fetch impl.
5. **Faithful wire test:** produce 3 records, then Fetch from 0 → get the 3 plaintext back; assert the STORED
   bytes are sealed (not the plaintext) — the provider-blind property. ListOffsets returns (0, 3).
6. (CI) a real Kafka **consumer** (kcat -C / librdkafka) round-trips against `datarail kafka-broker`.
7. **Adversarial audit** of the new consume path (un-seal-on-fetch is security-critical — a forged/cross-route
   cofre must never open; bounds/`max_bytes`/offset must never panic or over-read).
