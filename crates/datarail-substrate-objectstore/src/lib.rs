//! `datarail-substrate-objectstore` — the SPEC `10` / `07` **object-store (S3) substrate**: a dumb,
//! provider-blind durable holding pen for temporal decoupling (the destination is offline) or fan-out
//! buffering. The source PUTs sealed cofres keyed by `<route_id>/<stream_id>/<seq>`; the destination GETs them
//! in `seq` order and deletes each post-ack (GC). The store sees **only ciphertext** (`INV-OPAQUE-CARGO`) and
//! holds **no keys** (`INV-DUMB-PIPE`) — a breach of the bucket yields useless bytes.
//!
//! This crate ships a **filesystem-backed** implementation (a `bucket` is a directory, an object is a file):
//! it fully exercises the store semantics — keyed idempotent PUT, ordered GET, delete-post-ack — and passes
//! the same [`substrate_conformance`](datarail_rail::substrate_conformance) harness as every other substrate
//! (`INV-SUBSTRATE-POLYMORPHIC`). A real S3/GCS/R2 backend is the *same* trait with the PUT/GET/DELETE verbs
//! pointed at a bucket API behind a credential-gated adapter — not a redesign (the cofre is already sealed, so
//! the object client adds no crypto). Keeping this substrate in its own crate keeps the std-only rail core
//! dependency-free.

#![forbid(unsafe_code)]

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use datarail_core::{Cofre, Substrate};

/// A provider-blind object-store [`Substrate`], filesystem-backed (the reference for the `s3` substrate).
///
/// Layout per SPEC `10`: `<bucket>/<route_id>/<stream_id>/<seq>.cofre`, one sealed cofre per object.
/// [`send`](Substrate::send) PUTs (atomically: write-temp-then-rename; re-PUT of the same `seq` is a no-op);
/// [`recv`](Substrate::recv) GETs the lowest `(route, stream, seq)` object not yet delivered (seq order within
/// a stream); [`ack`](Substrate::ack) deletes the object (GC). The store never decrypts, validates, or orders
/// content — those are the terminal's and the rail-protocol's jobs.
///
/// v1 note: `recv` scans the bucket each call (`O(n)` in stored objects) — correct and simple for bounded
/// holding pens; a production driver would keep a per-stream `seq` cursor.
#[derive(Debug)]
pub struct ObjectStoreSubstrate {
    bucket: PathBuf,
    /// Object keys already handed out by `recv` (so a re-scan does not redeliver them).
    delivered: HashSet<PathBuf>,
    /// `cofre_id` → object key, so `ack` can delete exactly the right object.
    by_id: HashMap<[u8; 32], PathBuf>,
    /// `cofre_id`s acked so far, in ack order.
    acked: Vec<[u8; 32]>,
}

impl ObjectStoreSubstrate {
    /// Open a bucket rooted at `bucket`, creating the directory if it does not exist.
    ///
    /// # Errors
    /// [`std::io::Error`] if the bucket directory cannot be created.
    pub fn open(bucket: impl Into<PathBuf>) -> std::io::Result<Self> {
        let bucket = bucket.into();
        std::fs::create_dir_all(&bucket)?;
        Ok(Self {
            bucket,
            delivered: HashSet::new(),
            by_id: HashMap::new(),
            acked: Vec::new(),
        })
    }

    /// The `cofre_id`s acked so far, in ack order.
    #[must_use]
    pub fn acked(&self) -> &[[u8; 32]] {
        &self.acked
    }

    /// The object key for a cofre: `<bucket>/<route_id>/<stream_id>/<seq>.cofre` (SPEC 10). `seq` is
    /// zero-padded so lexicographic key order equals numeric seq order.
    fn object_path(&self, cofre: &Cofre) -> PathBuf {
        let e = &cofre.etiqueta;
        self.bucket
            .join(hex16(&e.route_id))
            .join(hex16(&e.stream_id))
            .join(format!("{:020}.cofre", e.seq))
    }

    /// Every stored object key, sorted — `(route, stream, seq)` order (seq order within a stream).
    fn sorted_objects(&self) -> std::io::Result<Vec<PathBuf>> {
        let mut out = Vec::new();
        collect_cofres(&self.bucket, &mut out)?;
        out.sort();
        Ok(out)
    }
}

impl Substrate for ObjectStoreSubstrate {
    type Error = std::io::Error;

    /// PUT the sealed cofre at its `seq`-keyed object path, atomically (write a temp sibling, then rename).
    /// Idempotent: re-PUT of the same `seq` overwrites identical bytes — a no-op in effect.
    ///
    /// # Errors
    /// [`std::io::Error`] on a directory-create, write, or rename failure.
    fn send(&mut self, cofre: &Cofre) -> Result<(), Self::Error> {
        let path = self.object_path(cofre);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = datarail_cofre::encode(cofre);
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, &bytes)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// GET the lowest-keyed object not yet delivered (seq order within a stream); `None` when the bucket holds
    /// nothing new. Decodes the wire bytes to return the cofre but never inspects `carga` (`INV-OPAQUE-CARGO`).
    ///
    /// # Errors
    /// [`std::io::Error`] on a read failure, or `InvalidData` if a stored object fails to decode.
    fn recv(&mut self) -> Result<Option<Cofre>, Self::Error> {
        for path in self.sorted_objects()? {
            if !self.delivered.contains(&path) {
                let bytes = std::fs::read(&path)?;
                let cofre = datarail_cofre::decode(&bytes).map_err(|e| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
                })?;
                self.delivered.insert(path.clone());
                self.by_id.insert(cofre.etiqueta.cofre_id, path);
                return Ok(Some(cofre));
            }
        }
        Ok(None)
    }

    /// Record an acknowledgement and GC the object (delete-post-ack, SPEC 10). A missing object is fine — ack
    /// is idempotent and the object may already have been collected.
    ///
    /// # Errors
    /// [`std::io::Error`] on a delete failure other than "not found".
    fn ack(&mut self, cofre_id: [u8; 32]) -> Result<(), Self::Error> {
        self.acked.push(cofre_id);
        if let Some(path) = self.by_id.remove(&cofre_id) {
            if let Err(e) = std::fs::remove_file(&path) {
                // A missing object is fine (already collected / idempotent ack); anything else propagates.
                if e.kind() != std::io::ErrorKind::NotFound {
                    return Err(e);
                }
            }
        }
        Ok(())
    }
}

/// Lowercase-hex a 16-byte id for use as a filesystem-safe path segment.
fn hex16(bytes: &[u8; 16]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(32);
    for &b in bytes {
        s.push(char::from(HEX[(b >> 4) as usize]));
        s.push(char::from(HEX[(b & 0x0f) as usize]));
    }
    s
}

/// Recursively collect every `*.cofre` object path under `dir` (a non-directory or missing path yields none).
fn collect_cofres(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_cofres(&path, out)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("cofre") {
            out.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::ObjectStoreSubstrate;
    use datarail_core::Substrate;
    use datarail_rail::{substrate_conformance, testsupport};
    use std::sync::atomic::{AtomicU64, Ordering};

    static UNIQ: AtomicU64 = AtomicU64::new(0);

    /// A unique, per-test temp bucket path (no tempfile dep — `forbid(unsafe)` + leveza).
    fn temp_bucket() -> std::path::PathBuf {
        let n = UNIQ.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("datarail-objstore-{}-{n}", std::process::id()))
    }

    #[test]
    fn ac6_objectstore_passes_substrate_conformance() {
        // The SAME AC-6 flow over a real on-disk, provider-blind object store: seq-keyed PUT → ordered GET → ack.
        let dir = temp_bucket();
        substrate_conformance(|| ObjectStoreSubstrate::open(&dir).expect("open bucket"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn decoupled_source_puts_then_a_separate_dest_drains_in_seq_order() {
        // The store's reason to exist: SOURCE and DEST are never online together — they rendezvous only via
        // the bucket. Source PUTs a batch and goes away; a SEPARATE dest instance later drains it in seq order
        // and GCs each object on ack.
        let dir = temp_bucket();
        {
            let mut src = ObjectStoreSubstrate::open(&dir).expect("open source");
            for seq in 0..4 {
                src.send(&testsupport::cofre_seq(seq)).expect("PUT");
            }
        } // source dropped — only the bucket persists

        let mut dst = ObjectStoreSubstrate::open(&dir).expect("reopen as dest");
        for seq in 0..4 {
            let got = dst.recv().expect("GET").expect("a cofre is present");
            assert_eq!(
                got,
                testsupport::cofre_seq(seq),
                "objects delivered in seq order, byte-for-byte"
            );
            dst.ack(got.etiqueta.cofre_id).expect("ack + GC");
        }
        assert!(dst.recv().expect("drain").is_none(), "bucket fully drained after GC");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reput_same_seq_is_idempotent() {
        // SPEC 10: the seq is the key → a re-PUT is a no-op (at-least-once sources don't duplicate objects).
        let dir = temp_bucket();
        let mut store = ObjectStoreSubstrate::open(&dir).expect("open");
        let cofre = testsupport::cofre_seq(7);
        store.send(&cofre).expect("PUT 1");
        store.send(&cofre).expect("PUT 2 (idempotent)");

        let got = store.recv().expect("GET").expect("one cofre");
        assert_eq!(got, cofre);
        assert!(store.recv().expect("drain").is_none(), "re-PUT produced exactly one object, not two");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
