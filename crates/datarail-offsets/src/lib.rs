//! `datarail-offsets` — FROZEN CONTRACT (frozen by the lead): durable group-offset commits. `MemOffsets` is the working
//! reference (dependents test against it); `FileOffsets` (crash-safe, fsync'd) is implemented by WP1.
#![forbid(unsafe_code)]
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

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

/// Name of the append-only commit log inside the store directory.
const LOG_NAME: &str = "offsets.log";

/// Bitwise CRC-32 (IEEE 802.3, reflected) over `bytes` — a std-only integrity
/// check so a torn write at the log tail is detected on recovery.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// A crash-safe, fsync'd, durable [`OffsetStore`] backed by an append-only
/// commit log.
///
/// Each [`commit`](OffsetStore::commit) appends a length-prefixed,
/// CRC-checked record `[group_len: u32][group][offset: u64][crc: u32]`
/// (all integers little-endian) and fsyncs the file before returning, so a
/// committed offset survives a process restart or crash. On [`open`](Self::open)
/// the log is replayed start-to-finish and the last record per group wins; a
/// torn or corrupt tail record is discarded, leaving every prior commit intact.
#[derive(Debug)]
pub struct FileOffsets {
    file: File,
    map: BTreeMap<String, u64>,
}

impl FileOffsets {
    /// Open (creating if absent) the durable offset store in directory `dir`,
    /// recovering the latest committed offset for every group.
    ///
    /// # Errors
    /// The directory cannot be created, the log cannot be opened, or an I/O
    /// error occurs while reading the log during recovery.
    pub fn open(dir: impl AsRef<Path>) -> io::Result<Self> {
        let dir = dir.as_ref();
        std::fs::create_dir_all(dir)?;
        let path: PathBuf = dir.join(LOG_NAME);
        let mut file = OpenOptions::new().read(true).append(true).create(true).open(path)?;
        let map = Self::replay(&mut file)?;
        Ok(Self { file, map })
    }

    /// Read the whole log and fold it into the latest-offset-per-group map,
    /// stopping at the first incomplete or CRC-mismatched record (a torn tail).
    fn replay(file: &mut File) -> io::Result<BTreeMap<String, u64>> {
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;
        let mut map = BTreeMap::new();
        let mut pos = 0usize;
        while let Some((group, offset, next)) = Self::parse_record(&buf, pos) {
            map.insert(group, offset);
            pos = next;
        }
        Ok(map)
    }

    /// Parse one record starting at `pos`, returning the group, offset, and the
    /// offset just past it, or `None` if the bytes are incomplete/corrupt.
    fn parse_record(buf: &[u8], pos: usize) -> Option<(String, u64, usize)> {
        let len_end = pos.checked_add(4)?;
        let len_bytes: [u8; 4] = buf.get(pos..len_end)?.try_into().ok()?;
        let group_len = u32::from_le_bytes(len_bytes) as usize;
        let group_end = len_end.checked_add(group_len)?;
        let group_bytes = buf.get(len_end..group_end)?;
        let offset_end = group_end.checked_add(8)?;
        let offset_bytes: [u8; 8] = buf.get(group_end..offset_end)?.try_into().ok()?;
        let crc_end = offset_end.checked_add(4)?;
        let crc_bytes: [u8; 4] = buf.get(offset_end..crc_end)?.try_into().ok()?;
        let stored_crc = u32::from_le_bytes(crc_bytes);
        if crc32(buf.get(pos..offset_end)?) != stored_crc {
            return None;
        }
        let group = String::from_utf8(group_bytes.to_vec()).ok()?;
        let offset = u64::from_le_bytes(offset_bytes);
        Some((group, offset, crc_end))
    }
}

impl OffsetStore for FileOffsets {
    fn commit(&mut self, group: &str, offset: u64) -> Result<(), std::io::Error> {
        let group_len = u32::try_from(group.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "group name too long"))?;
        let mut record = Vec::with_capacity(group.len() + 16);
        record.extend_from_slice(&group_len.to_le_bytes());
        record.extend_from_slice(group.as_bytes());
        record.extend_from_slice(&offset.to_le_bytes());
        let crc = crc32(&record);
        record.extend_from_slice(&crc.to_le_bytes());
        self.file.write_all(&record)?;
        self.file.sync_all()?;
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

#[cfg(test)]
mod tests {
    use super::{FileOffsets, OffsetStore};
    use std::path::PathBuf;

    /// A unique, auto-cleaned temp dir per test.
    struct TmpDir(PathBuf);
    impl TmpDir {
        fn new(tag: &str) -> Self {
            let mut p = std::env::temp_dir();
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos());
            p.push(format!("datarail-offsets-{}-{}-{nanos}", std::process::id(), tag));
            std::fs::create_dir_all(&p).expect("create temp dir");
            Self(p)
        }
    }
    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn commit_fetch_roundtrip() {
        let tmp = TmpDir::new("roundtrip");
        let mut store = FileOffsets::open(&tmp.0).expect("open");
        store.commit("g", 42).expect("commit");
        assert_eq!(store.fetch("g"), Some(42));
    }

    #[test]
    fn durable_across_reopen() {
        let tmp = TmpDir::new("durable");
        {
            let mut store = FileOffsets::open(&tmp.0).expect("open");
            store.commit("alpha", 1).expect("commit");
            store.commit("beta", 22).expect("commit");
            store.commit("gamma", 333).expect("commit");
        }
        let store = FileOffsets::open(&tmp.0).expect("reopen");
        assert_eq!(store.fetch("alpha"), Some(1));
        assert_eq!(store.fetch("beta"), Some(22));
        assert_eq!(store.fetch("gamma"), Some(333));
    }

    #[test]
    fn last_write_wins() {
        let tmp = TmpDir::new("lww");
        {
            let mut store = FileOffsets::open(&tmp.0).expect("open");
            store.commit("g", 1).expect("commit");
            store.commit("g", 2).expect("commit");
            store.commit("g", 999).expect("commit");
        }
        let store = FileOffsets::open(&tmp.0).expect("reopen");
        assert_eq!(store.fetch("g"), Some(999));
    }

    #[test]
    fn groups_isolated() {
        let tmp = TmpDir::new("iso");
        let mut store = FileOffsets::open(&tmp.0).expect("open");
        store.commit("a", 5).expect("commit");
        store.commit("b", 7).expect("commit");
        assert_eq!(store.fetch("a"), Some(5));
        assert_eq!(store.fetch("b"), Some(7));
        let mut g = store.groups();
        g.sort();
        assert_eq!(g, vec!["a".to_owned(), "b".to_owned()]);
    }

    #[test]
    fn unknown_group_is_none() {
        let tmp = TmpDir::new("unknown");
        let store = FileOffsets::open(&tmp.0).expect("open");
        assert_eq!(store.fetch("nope"), None);
    }
}
