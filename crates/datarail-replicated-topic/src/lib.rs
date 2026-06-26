//! `datarail-replicated-topic` — FROZEN API (frozen by the lead); WP2 implements the bodies + gates.
//! Composes a [`datarail_topic::Topic`] (durable retained log) with a `datarail_replication::ErasureStore`
//! (erasure-coded shards over a [`BlobStore`]): every produced record is appended to the log AND sharded into
//! k+m blobs keyed by its offset, so a record survives the loss of any `m` shards — node-loss durability at 1.5x.
#![forbid(unsafe_code)]
use datarail_blobstore::BlobStore;

/// A decoded record: `(key, payload)`.
pub type Record = (Vec<u8>, Vec<u8>);

fn unimpl<T>() -> Result<T, std::io::Error> {
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "WP2: unimplemented"))
}

/// A topic with erasure-coded node-loss durability over a [`BlobStore`].
pub struct ReplicatedTopic<B: BlobStore> {
    blob: B,
}

impl<B: BlobStore> ReplicatedTopic<B> {
    /// Open at `dir` with the shard store `blob` and `k`+`m` erasure parameters.
    /// # Errors
    /// Filesystem/codec error.
    pub fn open(_dir: &std::path::Path, blob: B, _k: usize, _m: usize) -> Result<Self, std::io::Error> {
        Ok(Self { blob }) // WP2: open the Topic + build the ErasureStore over `blob`
    }
    /// Borrow the shard store (keeps the field live in the scaffold; WP2's tests may inspect it).
    #[must_use]
    pub fn blob(&self) -> &B {
        &self.blob
    }
    /// Produce a record: append to the log AND erasure-shard it. Returns the log offset. WP2: implement.
    /// # Errors
    /// Filesystem/codec error.
    pub fn produce(&mut self, _key: &[u8], _payload: &[u8]) -> Result<u64, std::io::Error> {
        let _ = &mut self.blob;
        unimpl()
    }
    /// Reconstruct a record by its offset from its erasure shards (works even after losing any `m`). WP2: implement.
    /// # Errors
    /// Filesystem/codec error.
    pub fn reconstruct(&self, _offset: u64) -> Result<Option<Record>, std::io::Error> {
        unimpl()
    }
}
