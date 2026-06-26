//! `datarail-replication` — FROZEN API (frozen by the lead); WP3 implements the bodies + gates.
//! Erasure-codes each record into `k` data + `m` parity shards over a [`BlobStore`], tolerating `m` lost shards
//! (Kafka-grade 2-failure durability at 1.5x storage, not RF=3's 3x).
#![forbid(unsafe_code)]
use datarail_blobstore::BlobStore;

fn unimpl<T>() -> Result<T, std::io::Error> {
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "WP3: unimplemented"))
}

/// An erasure-coded replicated store over any [`BlobStore`].
pub struct ErasureStore<B: BlobStore> {
    blob: B,
}

impl<B: BlobStore> ErasureStore<B> {
    /// Build over `blob` with `k` data + `m` parity shards.
    /// # Errors
    /// Bad `k`/`m` (WP3 validates via the codec).
    pub fn new(blob: B, _k: usize, _m: usize) -> Result<Self, std::io::Error> {
        Ok(Self { blob }) // WP3: also construct + store the datarail_erasure::Rs codec and k/m
    }
    /// Borrow the backing blob store (lets WP3's tests inspect/inject; keeps the field live in the scaffold).
    #[must_use]
    pub fn blob(&self) -> &B {
        &self.blob
    }
    /// Store `data` under `id` as k+m shards. WP3: implement (encode → put each shard at `id/<i>`).
    /// # Errors
    /// Backend failure.
    pub fn put(&mut self, _id: &str, _data: &[u8]) -> Result<(), std::io::Error> {
        let _ = &mut self.blob;
        unimpl()
    }
    /// Reconstruct `data` for `id` from any `k` surviving shards. WP3: implement (get ≥k shards → reconstruct).
    /// # Errors
    /// Backend failure or too many shards lost.
    pub fn get(&self, _id: &str) -> Result<Option<Vec<u8>>, std::io::Error> {
        unimpl()
    }
}
