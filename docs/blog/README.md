# Datarail — technical blog posts

Three posts drawn from actual events in this repository.

| Post | Summary |
|---|---|
| [01 — My Kafka broker scaled negatively: a mutex story](01-negative-scaling.md) | An independent benchmark found that adding a second producer reduced aggregate throughput from 6.0 to 4.4 MB/s; the diagnosis was per-record sealing inside a global Mutex, the fix was a reserved-seq parallel `board_at`, and the measurement required a same-minute interleaved A/B to avoid a ~4.5x host-drift artifact. |
| [02 — I ran adversarial audits against my own benchmarks, and every impressive number fell](02-auditing-my-own-benchmarks.md) | A systematic adversarial audit dropped the RAM headline from 124x to ~40x loaded once the comparison was made fair, then re-measured ~72x against an equal-durability baseline, retracted a throughput "win" to an honest tie, recorded a performance gate as FAILED before re-scoping it, and an independent auditor found a live contract-violation bug (librdkafka infinite-retry loop) that the self-audit had missed entirely. |
| [03 — The one thing Kafka can't give you: an offline-verifiable delivery receipt](03-merkle-delivery-receipt.md) | A Merkle-over-BLAKE3 delivery proof signed with an Ed25519 lacre gives a third party offline-verifiable, non-repudiable evidence that a specific record was delivered — a different guarantee than CSFLE or proxy encryption — but only in rail mode where the endpoints hold the keys, not in broker mode where the broker is the trusted keyholder. |
