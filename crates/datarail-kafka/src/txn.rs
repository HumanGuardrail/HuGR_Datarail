//! The single-node Kafka **transaction coordinator** (`KAFKA-TXN-DESIGN.md`): the state behind a transactional
//! producer — `InitProducerId(transactional_id)` (epoch fencing), `AddPartitionsToTxn`, `AddOffsetsToTxn`,
//! `TxnOffsetCommit`, `EndTxn`. It tracks, per `transactional_id`, the `(producer_id, epoch)` and the partitions +
//! staged offsets enrolled in the CURRENT transaction, so `EndTxn` can tell the serve layer where to write
//! COMMIT/ABORT markers and which offsets to durably commit. Runtime state (a broker restart aborts in-flight
//! txns — ratified Q3); durable txn state is later.
//!
//! **Epoch fencing (the crux).** `InitProducerId` on an existing `transactional_id` bumps the producer epoch,
//! fencing any prior incarnation: a Produce / `AddPartitions` / `EndTxn` carrying a stale epoch is rejected with
//! `INVALID_PRODUCER_EPOCH`, so a zombie producer from a previous session can never commit into a new one's txn.

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, PoisonError};

/// NONE.
pub const NONE: i16 = 0;
/// `INVALID_PRODUCER_EPOCH` — the request's `(producer_id, epoch)` is fenced by a newer incarnation.
pub const INVALID_PRODUCER_EPOCH: i16 = 47;
/// `INVALID_TXN_STATE` — the operation is not valid in the txn's current state (e.g. `EndTxn` with no open txn).
pub const INVALID_TXN_STATE: i16 = 48;

/// Per-`transactional_id` coordinator state.
struct TxnState {
    producer_id: i64,
    epoch: i16,
    /// True between the first `AddPartitionsToTxn`/`AddOffsetsToTxn` and `EndTxn` (a txn is open).
    ongoing: bool,
    /// Partitions enrolled in the current txn (where COMMIT/ABORT markers must be written at `EndTxn`).
    partitions: BTreeSet<(String, i32)>,
    /// The consumer group whose offsets this txn commits (if any), and the staged `(topic, partition, offset)`s —
    /// applied durably only on `EndTxn(commit)`.
    group: Option<String>,
    staged_offsets: Vec<(String, i32, i64)>,
}

impl TxnState {
    fn new(producer_id: i64) -> Self {
        Self {
            producer_id,
            epoch: 0,
            ongoing: false,
            partitions: BTreeSet::new(),
            group: None,
            staged_offsets: Vec::new(),
        }
    }

    fn reset_txn(&mut self) {
        self.ongoing = false;
        self.partitions.clear();
        self.group = None;
        self.staged_offsets.clear();
    }
}

/// The result of `EndTxn`: the partitions needing a COMMIT/ABORT marker, plus (on commit) the offsets to durably
/// apply (`group`, `(topic, partition, offset)`).
pub struct EndTxnOutcome {
    /// Error code (0 = NONE).
    pub error_code: i16,
    /// Whether the txn committed (vs aborted) — the marker kind to write.
    pub committed: bool,
    /// Partitions to write the marker to.
    pub partitions: Vec<(String, i32)>,
    /// On commit: the consumer group + offsets to durably commit (empty on abort).
    pub group: Option<String>,
    /// On commit: `(topic, partition, offset)` to durably commit (empty on abort).
    pub offsets: Vec<(String, i32, i64)>,
}

/// The single-node transaction coordinator.
pub struct TxnCoordinator {
    txns: Mutex<HashMap<String, TxnState>>,
    /// Allocates `producer_id`s for transactional producers.
    next_producer_id: AtomicI64,
}

impl TxnCoordinator {
    /// A coordinator allocating `producer_id`s from `first_producer_id` upward (kept distinct from the idempotent
    /// allocator's range by the caller).
    #[must_use]
    pub fn new(first_producer_id: i64) -> Self {
        Self { txns: Mutex::new(HashMap::new()), next_producer_id: AtomicI64::new(first_producer_id) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, TxnState>> {
        self.txns.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// `InitProducerId` for a `transactional_id`: assign a `producer_id` (new id for a new txn id) and BUMP the
    /// epoch (fencing any prior incarnation), aborting any in-flight txn from the old epoch. Returns
    /// `(producer_id, epoch)`.
    pub fn init_producer_id(&self, transactional_id: &str) -> (i64, i16) {
        let mut g = self.lock();
        if let Some(st) = g.get_mut(transactional_id) {
            // Bump the epoch to fence the prior incarnation; on i16 overflow, mint a fresh producer_id at epoch 0.
            if let Some(next) = st.epoch.checked_add(1) {
                st.epoch = next;
            } else {
                st.producer_id = self.next_producer_id.fetch_add(1, Ordering::Relaxed);
                st.epoch = 0;
            }
            st.reset_txn(); // any in-flight txn of the old epoch is implicitly aborted
            (st.producer_id, st.epoch)
        } else {
            let pid = self.next_producer_id.fetch_add(1, Ordering::Relaxed);
            let st = TxnState::new(pid);
            let out = (st.producer_id, st.epoch);
            g.insert(transactional_id.to_owned(), st);
            out
        }
    }

    /// Verify `(producer_id, epoch)` against the current incarnation; `Ok(())` or an error code.
    fn fence(st: &TxnState, producer_id: i64, epoch: i16) -> i16 {
        if st.producer_id == producer_id && st.epoch == epoch {
            NONE
        } else {
            INVALID_PRODUCER_EPOCH
        }
    }

    /// `AddPartitionsToTxn`: enroll partitions in the current txn (opening it). Returns an error code.
    pub fn add_partitions(
        &self,
        transactional_id: &str,
        producer_id: i64,
        epoch: i16,
        partitions: &[(String, i32)],
    ) -> i16 {
        let mut g = self.lock();
        let Some(st) = g.get_mut(transactional_id) else {
            return INVALID_PRODUCER_EPOCH;
        };
        let code = Self::fence(st, producer_id, epoch);
        if code != NONE {
            return code;
        }
        st.ongoing = true;
        for p in partitions {
            st.partitions.insert(p.clone());
        }
        NONE
    }

    /// `AddOffsetsToTxn`: record that this txn will commit offsets for `group` (opening it). Returns an error code.
    pub fn add_offsets(&self, transactional_id: &str, producer_id: i64, epoch: i16, group: &str) -> i16 {
        let mut g = self.lock();
        let Some(st) = g.get_mut(transactional_id) else {
            return INVALID_PRODUCER_EPOCH;
        };
        let code = Self::fence(st, producer_id, epoch);
        if code != NONE {
            return code;
        }
        st.ongoing = true;
        st.group = Some(group.to_owned());
        NONE
    }

    /// `TxnOffsetCommit`: stage `(topic, partition, offset)`s to commit atomically at `EndTxn(commit)`.
    pub fn stage_offsets(
        &self,
        transactional_id: &str,
        producer_id: i64,
        epoch: i16,
        offsets: &[(String, i32, i64)],
    ) -> i16 {
        let mut g = self.lock();
        let Some(st) = g.get_mut(transactional_id) else {
            return INVALID_PRODUCER_EPOCH;
        };
        let code = Self::fence(st, producer_id, epoch);
        if code != NONE {
            return code;
        }
        st.staged_offsets.extend_from_slice(offsets);
        NONE
    }

    /// `EndTxn`: commit or abort. Returns the partitions needing a marker and (on commit) the staged offsets to
    /// durably apply, then resets the txn to ready-for-next.
    pub fn end_txn(&self, transactional_id: &str, producer_id: i64, epoch: i16, commit: bool) -> EndTxnOutcome {
        let mut g = self.lock();
        let Some(st) = g.get_mut(transactional_id) else {
            return EndTxnOutcome {
                error_code: INVALID_PRODUCER_EPOCH,
                committed: commit,
                partitions: Vec::new(),
                group: None,
                offsets: Vec::new(),
            };
        };
        let code = Self::fence(st, producer_id, epoch);
        if code != NONE {
            return EndTxnOutcome {
                error_code: code,
                committed: commit,
                partitions: Vec::new(),
                group: None,
                offsets: Vec::new(),
            };
        }
        if !st.ongoing {
            return EndTxnOutcome {
                error_code: INVALID_TXN_STATE,
                committed: commit,
                partitions: Vec::new(),
                group: None,
                offsets: Vec::new(),
            };
        }
        let partitions: Vec<(String, i32)> = st.partitions.iter().cloned().collect();
        let (group, offsets) =
            if commit { (st.group.clone(), st.staged_offsets.clone()) } else { (None, Vec::new()) };
        st.reset_txn();
        EndTxnOutcome { error_code: NONE, committed: commit, partitions, group, offsets }
    }
}

#[cfg(test)]
mod tests {
    use super::{TxnCoordinator, INVALID_PRODUCER_EPOCH, INVALID_TXN_STATE, NONE};

    #[test]
    fn init_bumps_epoch_and_fences_the_prior_incarnation() {
        let c = TxnCoordinator::new(1000);
        let (pid1, ep1) = c.init_producer_id("tx-A");
        assert_eq!(ep1, 0);
        // A second InitProducerId for the same id keeps the producer_id but bumps the epoch.
        let (pid2, ep2) = c.init_producer_id("tx-A");
        assert_eq!(pid2, pid1, "same transactional_id keeps its producer_id");
        assert_eq!(ep2, 1, "epoch bumped — the prior incarnation is fenced");
        // The OLD epoch is now fenced.
        assert_eq!(
            c.add_partitions("tx-A", pid1, ep1, &[("t".to_owned(), 0)]),
            INVALID_PRODUCER_EPOCH,
            "a zombie at the old epoch is rejected"
        );
        // The new epoch works.
        assert_eq!(c.add_partitions("tx-A", pid2, ep2, &[("t".to_owned(), 0)]), NONE);
    }

    #[test]
    fn distinct_transactional_ids_get_distinct_producer_ids() {
        let c = TxnCoordinator::new(1000);
        let (pa, _) = c.init_producer_id("tx-A");
        let (pb, _) = c.init_producer_id("tx-B");
        assert_ne!(pa, pb);
    }

    #[test]
    fn commit_returns_partitions_and_staged_offsets_then_resets() {
        let c = TxnCoordinator::new(1000);
        let (pid, ep) = c.init_producer_id("tx-A");
        assert_eq!(c.add_partitions("tx-A", pid, ep, &[("events".to_owned(), 0), ("events".to_owned(), 1)]), NONE);
        assert_eq!(c.add_offsets("tx-A", pid, ep, "grp"), NONE);
        assert_eq!(c.stage_offsets("tx-A", pid, ep, &[("src".to_owned(), 0, 42)]), NONE);
        let out = c.end_txn("tx-A", pid, ep, true);
        assert_eq!(out.error_code, NONE);
        assert!(out.committed);
        assert_eq!(out.partitions.len(), 2, "both enrolled partitions get a COMMIT marker");
        assert_eq!(out.group.as_deref(), Some("grp"));
        assert_eq!(out.offsets, vec![("src".to_owned(), 0, 42)], "staged offsets committed atomically");
        // After EndTxn the txn is reset — a second EndTxn with no open txn is INVALID_TXN_STATE.
        assert_eq!(c.end_txn("tx-A", pid, ep, true).error_code, INVALID_TXN_STATE);
    }

    #[test]
    fn abort_writes_markers_but_commits_no_offsets() {
        let c = TxnCoordinator::new(1000);
        let (pid, ep) = c.init_producer_id("tx-A");
        c.add_partitions("tx-A", pid, ep, &[("events".to_owned(), 0)]);
        c.stage_offsets("tx-A", pid, ep, &[("src".to_owned(), 0, 7)]);
        let out = c.end_txn("tx-A", pid, ep, false);
        assert_eq!(out.error_code, NONE);
        assert!(!out.committed);
        assert_eq!(out.partitions.len(), 1, "the partition still gets an ABORT marker");
        assert!(out.offsets.is_empty(), "an aborted txn commits NO offsets");
        assert!(out.group.is_none());
    }
}
