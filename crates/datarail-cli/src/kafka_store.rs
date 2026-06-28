//! Durable, provider-blind, contiguous-offset partition log for `datarail kafka-broker`
//! (`KAFKA-FETCH-DESIGN.md` increment 2 — durability across restart).
//!
//! It wraps the proven flat-RAM [`datarail_replaylog::ReplayLog`] (fsync-durable, segment-rotating, torn-tail-safe
//! — see `LASTRO-MATRIX.md`) and maps **Kafka's contiguous logical offset** (record index 0,1,2,…; next fetch =
//! `last + 1`) onto the log's *byte* offsets. The log on disk holds **only sealed cofre bytes**, so a snapshot of
//! the data directory reveals nothing — the provider-blind property the in-memory increment-1 store already had,
//! now surviving a restart.
//!
//! Two invariants make it Kafka-correct AND crash-safe:
//! - **Contiguous logical offset.** A side index `starts[i]` = the byte offset where record `i` begins. Rebuilt by
//!   scanning the durable log on [`open`](SealedPartitionLog::open), so every previously-acked record is
//!   addressable again after a restart.
//! - **Durability-before-visibility (= durability-before-ack).** [`append_durable`](SealedPartitionLog::append_durable)
//!   appends the batch, `fsync`s, and only THEN publishes the records' logical offsets. A record is fetchable only
//!   once it is on stable storage, so a crash immediately after the produce ack never loses an acked record —
//!   closing the in-memory increment-1 caveat (`KAFKA-FETCH-DESIGN.md` line 44: "durability-before-ack lands with
//!   increment 2's persistent log").

use std::io;
use std::path::Path;

use datarail_replaylog::ReplayLog;

/// Per-partition segment size. The flat-RAM cost scales as `total / segment_bytes` (the per-replay segment-start
/// list), independent of how much history is retained — see `datarail-replaylog`.
const SEGMENT_BYTES: u64 = datarail_replaylog::DEFAULT_SEGMENT_BYTES;

/// One `(topic, partition)`'s durable, sealed, contiguous-offset record log.
pub(crate) struct SealedPartitionLog {
    log: ReplayLog,
    /// `starts[logical_offset]` = the byte offset where that record begins in `log` (a valid `replay_from` seek
    /// point). `starts.len()` = the Kafka *latest* offset (record count). Costs 8 bytes per record — an index,
    /// not the data; the sealed payloads live on disk and stream through a bounded buffer on read.
    starts: Vec<u64>,
}

impl SealedPartitionLog {
    /// Open or recover the partition log rooted at `dir`. Rebuilds the contiguous logical-offset index by scanning
    /// the durable log from the start, so all previously-acked records are addressable again after a restart. A
    /// torn tail (a crash mid-append) is dropped by the replay layer, so only fully-written records are recovered.
    ///
    /// # Errors
    /// [`io::Error`] if the backing log cannot be opened or scanned.
    pub(crate) fn open(dir: impl AsRef<Path>) -> io::Result<Self> {
        let log = ReplayLog::open(dir, SEGMENT_BYTES).map_err(io::Error::other)?;
        let mut starts = Vec::new();
        let mut replay = log.replay_from(0).map_err(io::Error::other)?;
        while let Some((offset, _record)) = replay.read_next().map_err(io::Error::other)? {
            starts.push(offset);
        }
        Ok(Self { log, starts })
    }

    /// The number of durably-stored records = the Kafka *latest* (next) logical offset.
    pub(crate) fn len(&self) -> usize {
        self.starts.len()
    }

    /// Append a batch of already-sealed cofre bytes, `fsync` once, and only THEN publish their logical offsets.
    /// The single `fsync` is the durability barrier for the whole batch; publishing the offsets afterwards means a
    /// record is never visible (fetchable / counted) before it is durable. If the `fsync` fails the offsets are
    /// NOT published (the in-memory index is untouched), so the caller can surface a retriable error without ever
    /// having exposed a non-durable record. Returns the logical offset assigned to the FIRST record in the batch.
    ///
    /// # Errors
    /// [`io::Error`] on an append (e.g. record too large) or `fsync` failure.
    pub(crate) fn append_durable(&mut self, sealed: &[Vec<u8>]) -> io::Result<i64> {
        let base = i64::try_from(self.starts.len()).unwrap_or(i64::MAX);
        let mut new_starts = Vec::with_capacity(sealed.len());
        for bytes in sealed {
            new_starts.push(self.log.append(bytes).map_err(io::Error::other)?);
        }
        // Durability barrier BEFORE the records become visible — their offsets are published only after this.
        self.log.sync().map_err(io::Error::other)?;
        self.starts.extend(new_starts);
        Ok(base)
    }

    /// Read the **sealed** cofre bytes from logical `offset` onward, capping the batch at `max_bytes` of *ciphertext*
    /// (but always returning at least one record if any exist — Kafka semantics: a fetch must make progress). The
    /// caller un-seals at the edge. Streams through the flat-RAM replay cursor, so RAM stays flat regardless of how
    /// much history is retained. An `offset` at/past the end returns empty (the consumer waits / retries).
    ///
    /// The cap is on ciphertext, which is `>=` the plaintext size the caller will return, so the effective
    /// plaintext bytes never exceed `max_bytes` by more than one record — an honest, safe over-approximation.
    ///
    /// # Errors
    /// [`io::Error`] on a read/framing error from the backing log.
    pub(crate) fn read_sealed_from(&self, offset: usize, max_bytes: i64) -> io::Result<Vec<Vec<u8>>> {
        let Some(&start) = self.starts.get(offset) else {
            return Ok(Vec::new());
        };
        let mut replay = self.log.replay_from(start).map_err(io::Error::other)?;
        let mut out = Vec::new();
        let mut bytes = 0i64;
        while let Some((_offset, record)) = replay.read_next().map_err(io::Error::other)? {
            bytes = bytes.saturating_add(i64::try_from(record.len()).unwrap_or(i64::MAX));
            out.push(record);
            if bytes >= max_bytes {
                break;
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::SealedPartitionLog;

    fn tmpdir(tag: &str) -> std::path::PathBuf {
        let mut d = std::env::temp_dir();
        d.push(format!("datarail-kafka-store-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn append_assigns_contiguous_logical_offsets_and_reads_back_in_order() {
        let dir = tmpdir("contig");
        let mut log = SealedPartitionLog::open(&dir).unwrap();
        assert_eq!(log.len(), 0);
        // Two batches: offsets must be contiguous 0,1 then 2.
        assert_eq!(log.append_durable(&[b"a".to_vec(), b"b".to_vec()]).unwrap(), 0);
        assert_eq!(log.append_durable(&[b"c".to_vec()]).unwrap(), 2);
        assert_eq!(log.len(), 3);
        assert_eq!(log.read_sealed_from(0, 1 << 20).unwrap(), vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()]);
        assert_eq!(log.read_sealed_from(1, 1 << 20).unwrap(), vec![b"b".to_vec(), b"c".to_vec()]);
        // At/past the end → empty, never a panic.
        assert!(log.read_sealed_from(3, 1 << 20).unwrap().is_empty());
        assert!(log.read_sealed_from(99, 1 << 20).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn records_survive_a_restart() {
        let dir = tmpdir("restart");
        {
            let mut log = SealedPartitionLog::open(&dir).unwrap();
            log.append_durable(&[b"sealed-0".to_vec(), b"sealed-1".to_vec()]).unwrap();
            log.append_durable(&[b"sealed-2".to_vec()]).unwrap();
            // drop → simulate process exit
        }
        // Reopen: the contiguous index is rebuilt from the durable log, and the next append continues at 3.
        let mut reopened = SealedPartitionLog::open(&dir).unwrap();
        assert_eq!(reopened.len(), 3, "all acked records recovered after restart");
        assert_eq!(
            reopened.read_sealed_from(0, 1 << 20).unwrap(),
            vec![b"sealed-0".to_vec(), b"sealed-1".to_vec(), b"sealed-2".to_vec()]
        );
        assert_eq!(reopened.append_durable(&[b"sealed-3".to_vec()]).unwrap(), 3, "logical offset continues past recovery");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn max_bytes_bounds_the_batch_but_always_returns_at_least_one() {
        let dir = tmpdir("maxbytes");
        let mut log = SealedPartitionLog::open(&dir).unwrap();
        log.append_durable(&[vec![1u8; 100], vec![2u8; 100], vec![3u8; 100]]).unwrap();
        // A tiny cap still returns exactly one record (progress guarantee).
        assert_eq!(log.read_sealed_from(0, 1).unwrap().len(), 1);
        // A cap that fits ~two records stops early (does not over-read the whole log).
        assert_eq!(log.read_sealed_from(0, 150).unwrap().len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
