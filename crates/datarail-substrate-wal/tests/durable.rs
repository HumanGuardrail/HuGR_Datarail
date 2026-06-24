//! Durability + O(1)-RAM proofs for the write-ahead-log substrate.
//!
//! These are the falsifiable gates behind `DURABLE-LOG.md`: AC-6 conformance, power-loss recovery (every
//! fsync'd cofre survives a "crash"), torn-tail truncation, and the RAM-flat invariant (RSS does not grow with
//! stored volume).

use std::sync::atomic::{AtomicU64, Ordering};

use datarail_core::Substrate;
use datarail_rail::testsupport::cofre_seq;
use datarail_substrate_wal::{DurableLog, WalConfig};

static UNIQ: AtomicU64 = AtomicU64::new(0);

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let n = UNIQ.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!("datarail-wal-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    p
}

/// AC-6: the WAL substrate satisfies the shared conformance harness (send→recv-in-order→ack→drained).
#[test]
fn ac6_wal_passes_substrate_conformance() {
    let dir = temp_dir("conf");
    datarail_rail::substrate_conformance(|| DurableLog::open(&dir).expect("open wal"));
}

/// Power-loss durability: write N cofres + fsync (`flush`), DROP the handle (simulating a crash with no
/// in-RAM state surviving), reopen a fresh `DurableLog` on the same dir, and drain — every cofre must come back,
/// in order, byte-identical. This is the "acks=all ⇒ on stable storage" guarantee.
#[test]
fn power_loss_recovery_zero_loss() {
    const N: u64 = 2000;
    let dir = temp_dir("recover");
    {
        let mut log = DurableLog::open(&dir).expect("open");
        for seq in 0..N {
            log.send(&cofre_seq(seq)).expect("send");
        }
        log.flush().expect("fsync"); // durability point
        drop(log); // "crash": the only state that survives is what's on disk
    }
    // A brand-new process-equivalent opens the same dir and drains it.
    let mut reopened = DurableLog::open(&dir).expect("reopen");
    let mut got = Vec::new();
    while let Some(c) = reopened.recv().expect("recv") {
        let id = c.etiqueta.cofre_id;
        got.push(c.etiqueta.seq);
        reopened.ack(id).expect("ack");
    }
    assert_eq!(got.len(), usize::try_from(N).unwrap(), "every fsync'd cofre must survive the crash");
    assert_eq!(got, (0..N).collect::<Vec<_>>(), "recovered cofres must be in seq order, 0-loss/0-dup");
}

/// A torn write (power-loss mid-append) must be truncated to the last intact frame; earlier cofres survive.
#[test]
fn torn_tail_is_truncated_prior_intact() {
    const N: u64 = 50;
    let dir = temp_dir("torn");
    {
        let mut log = DurableLog::open(&dir).expect("open");
        for seq in 0..N {
            log.send(&cofre_seq(seq)).expect("send");
        }
        log.flush().expect("fsync");
    }
    // Simulate a torn write: append a partial/garbage frame to the active segment file.
    let seg = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "seg"))
        .expect("a segment file");
    {
        let mut f = std::fs::OpenOptions::new().append(true).open(&seg).unwrap();
        // a plausible-looking length prefix followed by truncated bytes (no valid CRC) = a torn tail
        std::io::Write::write_all(&mut f, &1024u32.to_be_bytes()).unwrap();
        std::io::Write::write_all(&mut f, &[0xABu8; 300]).unwrap();
        f.sync_data().unwrap();
    }
    let mut reopened = DurableLog::open(&dir).expect("reopen after torn write");
    let mut got = Vec::new();
    while let Some(c) = reopened.recv().expect("recv") {
        let id = c.etiqueta.cofre_id;
        got.push(c.etiqueta.seq);
        reopened.ack(id).expect("ack");
    }
    assert_eq!(got, (0..N).collect::<Vec<_>>(), "torn tail dropped; all intact cofres recovered exactly");
}

/// Round-trip with rotation: a small segment size forces many segments; send→recv→ack still 0-loss, and
/// fully-acked segments are GC'd (disk does not retain everything).
#[test]
fn rotation_and_gc_zero_loss() {
    const N: u64 = 1000;
    let dir = temp_dir("rotate");
    let cfg = WalConfig { flush_bytes: 4096, flush_micros: 1, segment_bytes: 16 * 1024 };
    let mut log = DurableLog::open_with(&dir, cfg).expect("open");
    for seq in 0..N {
        log.send(&cofre_seq(seq)).expect("send");
    }
    log.flush().expect("flush");
    let mut got = Vec::new();
    while let Some(c) = log.recv().expect("recv") {
        let id = c.etiqueta.cofre_id;
        got.push(c.etiqueta.seq);
        log.ack(id).expect("ack");
    }
    assert_eq!(got, (0..N).collect::<Vec<_>>(), "0-loss across many rotated segments");
    // After draining + acking everything, GC should have deleted the early segments.
    let remaining = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "seg"))
        .count();
    assert!(remaining <= 2, "fully-acked segments must be GC'd (found {remaining} seg files)");
}

/// THE masterpiece gate: RSS must NOT grow with stored volume. Store a large number of cofres and assert the
/// process's resident memory stays flat (Linux `/proc` only; skipped elsewhere).
#[test]
fn ram_stays_flat_as_stored_volume_grows() {
    const N: u64 = 100_000;
    let Some(rss0) = rss_kb() else {
        eprintln!("skip: /proc not available (non-Linux)");
        return;
    };
    let dir = temp_dir("ramflat");
    let mut log = DurableLog::open(&dir).expect("open");
    let cofre = cofre_seq(0);
    let mut peak = rss0;
    for i in 0..N {
        // reuse one cofre's bytes (distinct-content isn't the point here; flat RAM is) — send is what matters
        log.send(&cofre).expect("send");
        if i % 20_000 == 0 {
            if let Some(r) = rss_kb() {
                peak = peak.max(r);
            }
        }
    }
    log.flush().expect("flush");
    let after = rss_kb().unwrap_or(rss0);
    let stored = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum::<u64>();
    eprintln!("stored {} MB across {N} cofres; RSS {rss0}→{after} KB (peak {peak})", stored / (1 << 20));
    // Allow a generous fixed headroom (buffers + allocator slack), but NOT growth proportional to N.
    // 100k cofres at the demo payload is tens of MB on disk; if RSS grew with volume it'd be hundreds of MB.
    assert!(
        after < rss0 + 64 * 1024,
        "RSS must stay flat (grew {} KB over baseline {rss0} for {N} cofres — not O(1))",
        after.saturating_sub(rss0)
    );
}

fn rss_kb() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: u64 = s.split_whitespace().nth(1)?.parse().ok()?;
    Some(pages * 4) // 4 KiB pages → KB
}
