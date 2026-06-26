//! `datarail-offsets` — FROZEN CONTRACT (frozen by the lead): durable group-offset commits. `MemOffsets` is the working
//! reference (dependents test against it); `FileOffsets` (crash-safe, fsync'd) is implemented by WP1.
#![forbid(unsafe_code)]
use std::collections::BTreeMap;

/// Where consumer groups durably record how far they've consumed.
pub trait OffsetStore {
    /// Durably commit `group`'s offset (survives restart).
    /// # Errors
    /// Backend write/fsync failure.
    fn commit(&mut self, group: &str, offset: u64) -> Result<(), std::io::Error>;
    /// The last committed offset for `group`, or `None`.
    fn fetch(&self, group: &str) -> Option<u64>;
    /// All known group names.
    fn groups(&self) -> Vec<String>;
}

/// The working in-memory reference — frozen by the lead so dependents (WP4) can test immediately.
#[derive(Debug, Default)]
pub struct MemOffsets {
    map: BTreeMap<String, u64>,
}
impl MemOffsets {
    /// A new empty store.
    #[must_use]
    pub fn new() -> Self {
        Self { map: BTreeMap::new() }
    }
}
impl OffsetStore for MemOffsets {
    fn commit(&mut self, group: &str, offset: u64) -> Result<(), std::io::Error> {
        self.map.insert(group.to_owned(), offset);
        Ok(())
    }
    fn fetch(&self, group: &str) -> Option<u64> {
        self.map.get(group).copied()
    }
    fn groups(&self) -> Vec<String> {
        self.map.keys().cloned().collect()
    }
}
