//! `datarail-system` — the end-to-end capstone. This crate has no API of its own; it exists to hold integration
//! tests that compose the independently-proven components into a working system and prove the whole behaves —
//! a durable, networked broker that survives restart, a record that survives shard loss, and a remote cold tier.
//! See `tests/`.
#![forbid(unsafe_code)]
