//! `datarail-once` — effectively-once (SPEC `05-effectively-once.md`): a bounded dedup index, a signed
//! monotonic low-watermark with **reject-below**, and crash/retry/partition safety.
//!
//! The destination's admission decision (transcribed from SPEC 05 / BLK-6):
//! 1. **Reject-below, not merely lookup (BLK-6):** keep a *signed monotonic low-watermark per stream* and
//!    **REJECT** any arriving `seq` *below* it outright — below-watermark `seq` is already durably accounted
//!    for, so admitting it would risk a GC-vs-replay double-commit. This rejection is a [`Disposition`], not
//!    an error (it lands on the siding: [`Disposition::DeadLettered`]).
//! 2. **Dedup:** an arriving `idempotency_key` already in the dedup index ⇒ [`Disposition::Duplicate`]
//!    (idempotent drop + re-ack).
//! 3. Otherwise **commit**: record the key, return [`Disposition::Delivered`], and advance the contiguous
//!    low-watermark across every already-delivered `seq`.
//!
//! GC of the dedup index lags the **dest watermark — NOT a source checkpoint** (BLK-6): a key is GC-eligible
//! only once the dest watermark has advanced past its `seq`, so no still-replayable `seq` can fall through a
//! GC'd slot.
//!
//! AC-4 (*"acked at boarding ⇒ offloaded exactly once"*) is proven by the **deterministic simulation test**
//! (`tests/ac4_dst.rs`): a seeded, fault-injecting in-memory pipe (drop / reorder / duplicate / kill-and-
//! respawn) over N seeds, asserting **0 acked-loss and 0 duplicate** committed at the sink.

#![forbid(unsafe_code)]

use std::collections::{HashMap, HashSet};

use datarail_core::Disposition;
use datarail_crypto::{sign_domain, verify_domain};

/// Domain-separation label for a signed low-watermark token (BLK-5 style; local to this crate — the frozen
/// `datarail_crypto::ctx` has no watermark role).
const WM_CTX: &[u8] = b"dr:once:wm:v1";

/// Per-stream effectively-once state: the dedup index, the contiguous low-watermark, and the set of
/// delivered-but-not-yet-contiguous `seq`s used to advance it.
//
// TODO(MAJ-2, SPEC 05 §"Dedup index"): for v1 the dedup index is an in-RAM `HashSet<[u8;32]>`. The SPEC
// calls for a **bloom-filter prefilter (in-RAM, fast negative) fronting an on-disk authoritative map**, with
// dead-letter + sealed gap-skip on bloom/map overflow. That bounded, persisted index is a later optimization;
// the membership *semantics* below are the contract it must preserve.
#[derive(Debug, Default)]
struct StreamState {
    /// Authoritative dedup membership, keyed by `idempotency_key` (BLK: the *sole* dedup key).
    seen: HashSet<[u8; 32]>,
    /// `seq` → `idempotency_key` for every delivered record at or above the GC floor (lets GC drop keys from
    /// `seen` as the watermark advances past their `seq`).
    delivered_keys: HashMap<u64, [u8; 32]>,
    /// Delivered `seq`s strictly **above** `low_watermark` (the reorder horizon), used to advance the
    /// contiguous floor as gaps fill.
    ahead: HashSet<u64>,
    /// Monotonic contiguous low-watermark: every `seq < low_watermark` is durably committed. An arriving
    /// `seq < low_watermark` is rejected (BLK-6).
    low_watermark: u64,
    /// The GC floor: keys for `seq < gc_floor` have been compacted out of `seen`. Lags `low_watermark` by
    /// `gc_lag` (BLK-6: bound to the **dest watermark**, never a source checkpoint).
    gc_floor: u64,
}

/// The effectively-once admission gate for one destination: a dedup index + signed monotonic low-watermark
/// **per `stream_id`**.
///
/// Construct with [`Once::new`], drive with [`Once::admit`]. The watermark is signed with the destination's
/// Ed25519 `seed`; expose it to peers via [`Once::signed_watermark`] (verify with [`verify_watermark`]).
#[derive(Debug)]
pub struct Once {
    streams: HashMap<[u8; 16], StreamState>,
    /// Ed25519 secret for signing watermark tokens.
    seed: [u8; 32],
    /// How far the GC floor lags the dest low-watermark (BLK-6). Must exceed the max in-flight / replay
    /// horizon so no still-replayable `seq` can fall through a GC'd slot.
    gc_lag: u64,
}

/// Default GC lag — how far the dedup-GC floor trails the dest low-watermark (BLK-6).
///
/// Sized to comfortably exceed a bounded reorder window + replay horizon for v1.
pub const DEFAULT_GC_LAG: u64 = 1024;

impl Once {
    /// Create an admission gate signing watermark tokens under the destination Ed25519 `seed`, with the
    /// [`DEFAULT_GC_LAG`].
    #[must_use]
    pub fn new(seed: [u8; 32]) -> Self {
        Self::with_gc_lag(seed, DEFAULT_GC_LAG)
    }

    /// As [`Once::new`], but with an explicit GC lag (BLK-6: must exceed the max in-flight / replay horizon).
    #[must_use]
    pub fn with_gc_lag(seed: [u8; 32], gc_lag: u64) -> Self {
        Self {
            streams: HashMap::new(),
            seed,
            gc_lag,
        }
    }

    /// Admit a received record for `stream` at sequence `seq` with dedup key `idempotency_key`, returning the
    /// [`Disposition`] the destination should act on. **Pure decision + state update; no I/O.**
    ///
    /// Order of checks (SPEC 05 / BLK-6):
    /// - `seq` **below** the stream's monotonic low-watermark ⇒ [`Disposition::DeadLettered`] (reject-below;
    ///   the `seq` is already durably accounted for — admitting it risks a GC-vs-replay double-commit). This
    ///   fires *before* the dedup lookup, so a redelivery of an already-contiguous `seq` is rejected even if
    ///   its key was GC'd.
    /// - `idempotency_key` already seen ⇒ [`Disposition::Duplicate`] (idempotent drop + re-ack). This catches
    ///   a redelivery of a `seq` still *at or above* the watermark (a parked, not-yet-contiguous one).
    /// - otherwise ⇒ [`Disposition::Delivered`]: the key is recorded and the contiguous low-watermark is
    ///   advanced across every already-delivered `seq`.
    pub fn admit(&mut self, stream: [u8; 16], seq: u64, idempotency_key: [u8; 32]) -> Disposition {
        let gc_lag = self.gc_lag;
        let st = self.streams.entry(stream).or_default();

        // (1) Reject-below the monotonic low-watermark (BLK-6) — *before* any dedup lookup.
        if seq < st.low_watermark {
            return Disposition::DeadLettered;
        }

        // (2) Dedup on the sole key. At/above the watermark we may still have seen this exact key (an
        // at-least-once redelivery of a not-yet-contiguous seq).
        if st.seen.contains(&idempotency_key) {
            return Disposition::Duplicate;
        }

        // (3) Commit: record the key, then advance the contiguous floor.
        st.seen.insert(idempotency_key);
        st.delivered_keys.insert(seq, idempotency_key);
        if seq == st.low_watermark {
            st.low_watermark += 1;
            // Drain any contiguous run that earlier arrived out of order.
            while st.ahead.remove(&st.low_watermark) {
                st.low_watermark += 1;
            }
        } else {
            // seq > low_watermark: a gap remains; park it on the reorder horizon.
            st.ahead.insert(seq);
        }

        // (4) GC the dedup index up to a floor that LAGS the dest watermark (BLK-6) — never a source
        // checkpoint. Keys are dropped only once the dest watermark has advanced past their seq + gc_lag.
        let new_floor = st.low_watermark.saturating_sub(gc_lag);
        while st.gc_floor < new_floor {
            if let Some(k) = st.delivered_keys.remove(&st.gc_floor) {
                st.seen.remove(&k);
            }
            st.gc_floor += 1;
        }

        Disposition::Delivered
    }

    /// The current contiguous low-watermark for `stream` (every `seq <` it is committed). `0` if unseen.
    #[must_use]
    pub fn low_watermark(&self, stream: [u8; 16]) -> u64 {
        self.streams.get(&stream).map_or(0, |s| s.low_watermark)
    }

    /// The current dedup-GC floor for `stream` — keys for `seq <` it have been compacted out (BLK-6). `0` if
    /// unseen.
    #[must_use]
    pub fn gc_floor(&self, stream: [u8; 16]) -> u64 {
        self.streams.get(&stream).map_or(0, |s| s.gc_floor)
    }

    /// A **signed** low-watermark token for `stream`: the Ed25519 signature over `stream ‖ watermark` under
    /// this destination's seed, domain-separated by `WM_CTX`. Peers verify it with [`verify_watermark`].
    ///
    /// Returns the `(watermark, signature)` pair; the watermark is the same value as [`Once::low_watermark`].
    #[must_use]
    pub fn signed_watermark(&self, stream: [u8; 16]) -> (u64, [u8; 64]) {
        let wm = self.low_watermark(stream);
        let sig = sign_domain(WM_CTX, &self.seed, &watermark_msg(stream, wm));
        (wm, sig)
    }
}

/// The signed message body for a watermark token: `stream_id ‖ watermark` (little-endian).
fn watermark_msg(stream: [u8; 16], watermark: u64) -> [u8; 24] {
    let mut m = [0u8; 24];
    m[..16].copy_from_slice(&stream);
    m[16..].copy_from_slice(&watermark.to_le_bytes());
    m
}

/// Verify a signed low-watermark token (from [`Once::signed_watermark`]) against the destination's verifying
/// key `vk`.
#[must_use]
pub fn verify_watermark(vk: &[u8; 32], stream: [u8; 16], watermark: u64, sig: &[u8; 64]) -> bool {
    verify_domain(WM_CTX, vk, &watermark_msg(stream, watermark), sig)
}

#[cfg(test)]
mod tests {
    use super::{verify_watermark, Once, DEFAULT_GC_LAG};
    use datarail_core::Disposition;
    use datarail_crypto::verifying_key;

    const SEED: [u8; 32] = [13u8; 32];
    const S: [u8; 16] = [1u8; 16];

    fn key(n: u64) -> [u8; 32] {
        let mut k = [0u8; 32];
        k[..8].copy_from_slice(&n.to_le_bytes());
        k
    }

    #[test]
    fn first_delivery_then_redelivery_is_dropped() {
        let mut o = Once::new(SEED);
        assert_eq!(o.admit(S, 0, key(0)), Disposition::Delivered);
        // Redelivery of seq 0: the watermark has already advanced past it, so reject-below (BLK-6) fires
        // *before* the dedup lookup -> DeadLettered. Either way it is a drop + re-ack, never a re-commit;
        // both dispositions satisfy AC-4 at the sink.
        assert_eq!(o.admit(S, 0, key(0)), Disposition::DeadLettered);
    }

    #[test]
    fn redelivery_at_or_above_watermark_is_a_duplicate() {
        let mut o = Once::new(SEED);
        // Park seq 3 above the watermark (gap at 0..3 keeps the watermark at 0).
        assert_eq!(o.admit(S, 3, key(3)), Disposition::Delivered);
        assert_eq!(o.low_watermark(S), 0);
        // Redelivery of the still-at/above-watermark seq is caught by the dedup index -> Duplicate.
        assert_eq!(o.admit(S, 3, key(3)), Disposition::Duplicate);
    }

    #[test]
    fn reject_below_watermark_is_not_a_lookup() {
        let mut o = Once::new(SEED);
        // Deliver 0,1,2 contiguously; watermark advances to 3.
        for n in 0..3u64 {
            assert_eq!(o.admit(S, n, key(n)), Disposition::Delivered);
        }
        assert_eq!(o.low_watermark(S), 3);
        // BLK-6: a seq below the watermark is REJECTED outright — even with a brand-new, never-seen key.
        assert_eq!(o.admit(S, 1, key(999)), Disposition::DeadLettered);
        // And a below-watermark replay of a key that was GC'd would otherwise look "new" — still rejected.
        assert_eq!(o.admit(S, 0, key(0)), Disposition::DeadLettered);
    }

    #[test]
    fn out_of_order_then_gap_fill_advances_watermark() {
        let mut o = Once::new(SEED);
        // 0 arrives, then 2 (gap at 1): watermark only reaches 1.
        assert_eq!(o.admit(S, 0, key(0)), Disposition::Delivered);
        assert_eq!(o.admit(S, 2, key(2)), Disposition::Delivered);
        assert_eq!(o.low_watermark(S), 1);
        // 1 fills the gap: watermark jumps past the parked 2 -> 3.
        assert_eq!(o.admit(S, 1, key(1)), Disposition::Delivered);
        assert_eq!(o.low_watermark(S), 3);
    }

    #[test]
    fn duplicate_of_a_parked_ahead_seq_is_caught() {
        let mut o = Once::new(SEED);
        assert_eq!(o.admit(S, 0, key(0)), Disposition::Delivered);
        assert_eq!(o.admit(S, 5, key(5)), Disposition::Delivered); // parked ahead
                                                                   // Redelivery of the parked seq's key is a duplicate, not a re-delivery.
        assert_eq!(o.admit(S, 5, key(5)), Disposition::Duplicate);
    }

    #[test]
    fn gc_floor_lags_the_dest_watermark() {
        let lag = 4u64;
        let mut o = Once::with_gc_lag(SEED, lag);
        for n in 0..10u64 {
            assert_eq!(o.admit(S, n, key(n)), Disposition::Delivered);
        }
        assert_eq!(o.low_watermark(S), 10);
        // GC floor trails the dest watermark by exactly `lag` (BLK-6), never a source checkpoint.
        assert_eq!(o.gc_floor(S), 10 - lag);
        // A key at seq below the GC floor was compacted out; replaying it below-watermark is still rejected
        // (reject-below is what makes GC safe).
        assert_eq!(o.admit(S, 0, key(0)), Disposition::DeadLettered);
    }

    #[test]
    fn streams_are_independent() {
        let mut o = Once::new(SEED);
        let s2 = [2u8; 16];
        assert_eq!(o.admit(S, 0, key(0)), Disposition::Delivered);
        // Same seq+key on a *different* stream is a fresh delivery.
        assert_eq!(o.admit(s2, 0, key(0)), Disposition::Delivered);
        assert_eq!(o.low_watermark(S), 1);
        assert_eq!(o.low_watermark(s2), 1);
    }

    #[test]
    fn signed_watermark_roundtrips_and_rejects_tamper() {
        let mut o = Once::new(SEED);
        for n in 0..5u64 {
            o.admit(S, n, key(n));
        }
        let vk = verifying_key(&SEED);
        let (wm, sig) = o.signed_watermark(S);
        assert_eq!(wm, 5);
        assert!(verify_watermark(&vk, S, wm, &sig));
        // A bumped watermark claim does not verify against the signed value.
        assert!(!verify_watermark(&vk, S, wm + 1, &sig));
        // A different stream does not verify.
        assert!(!verify_watermark(&vk, [9u8; 16], wm, &sig));
    }

    #[test]
    fn default_gc_lag_is_exposed() {
        assert_eq!(DEFAULT_GC_LAG, 1024);
    }
}
