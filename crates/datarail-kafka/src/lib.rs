//! `datarail-kafka` — a minimal, zero-dependency implementation of the Apache Kafka wire protocol (the broker
//! side), enough that an UNMODIFIED Kafka producer can send records to datarail — which seals each into a
//! provider-blind cofre and lands it via any datarail `Sink`. The adoption bridge: *your Kafka producers, now
//! provider-blind and serverless, with no code change.*
//!
//! Layers (built incrementally): [`codec`] (wire primitives + request/response framing) → handlers
//! (`ApiVersions`, `Metadata`, `Produce` + the `RecordBatch` v2 parser) → a TCP serve loop feeding a sink.
#![forbid(unsafe_code)]

pub mod codec;
