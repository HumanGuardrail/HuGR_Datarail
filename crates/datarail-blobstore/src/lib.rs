//! `datarail-blobstore` — FROZEN CONTRACT (frozen by the lead): the cold-tier blob seam. `MemBlob` is the working
//! reference (dependents test against it); `FsBlob` is implemented by WP2.
#![forbid(unsafe_code)]
use std::collections::BTreeMap;

/// A content-addressed-ish blob store: opaque keys → bytes. The seam that lets the cold tier be local FS today
/// and S3/GCS tomorrow (same trait, GET/PUT/LIST/DELETE verbs).
pub trait BlobStore {
    /// Store/overwrite `key` with `bytes`.
    /// # Errors
    /// Backend write failure.
    fn put(&mut self, key: &str, bytes: &[u8]) -> Result<(), std::io::Error>;
    /// Fetch `key`, or `None` if absent.
    /// # Errors
    /// Backend read failure.
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, std::io::Error>;
    /// All keys with the given prefix, sorted.
    /// # Errors
    /// Backend list failure.
    fn list(&self, prefix: &str) -> Result<Vec<String>, std::io::Error>;
    /// Delete `key`; returns whether it existed.
    /// # Errors
    /// Backend delete failure.
    fn delete(&mut self, key: &str) -> Result<bool, std::io::Error>;
}

/// The working in-memory reference implementation — frozen by the lead so dependents (WP3) can test immediately.
#[derive(Debug, Default)]
pub struct MemBlob {
    map: BTreeMap<String, Vec<u8>>,
}
impl MemBlob {
    /// A new empty store.
    #[must_use]
    pub fn new() -> Self {
        Self { map: BTreeMap::new() }
    }
}
impl BlobStore for MemBlob {
    fn put(&mut self, key: &str, bytes: &[u8]) -> Result<(), std::io::Error> {
        self.map.insert(key.to_owned(), bytes.to_vec());
        Ok(())
    }
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, std::io::Error> {
        Ok(self.map.get(key).cloned())
    }
    fn list(&self, prefix: &str) -> Result<Vec<String>, std::io::Error> {
        Ok(self.map.keys().filter(|k| k.starts_with(prefix)).cloned().collect())
    }
    fn delete(&mut self, key: &str) -> Result<bool, std::io::Error> {
        Ok(self.map.remove(key).is_some())
    }
}
