//! `datarail-fuzz` — deterministic property/fuzz tests over every parser that ingests untrusted bytes. No API of
//! its own; the work is in `tests/`. A zero-dependency `SplitMix64` PRNG keeps every run reproducible.
#![forbid(unsafe_code)]
