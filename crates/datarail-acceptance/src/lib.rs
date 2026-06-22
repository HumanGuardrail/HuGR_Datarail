//! `datarail-acceptance` — cross-crate, end-to-end acceptance tests for datarail.
//!
//! This crate has no runtime code; it exists to host the integration tests that exercise the whole rail
//! across crate boundaries. See `tests/mf4_slice.rs` for the **first vertical slice (MF-4)**:
//! board → seal → rail → verify → offload, with exactly-once and dead-letter proven end-to-end.
