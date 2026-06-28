# KAFKA-REBALANCE-DESIGN — automatic consumer-group rebalance (increment 4b)

> **STATUS: DESIGN — FROZEN (2026-06-27), not yet implemented.** The last piece for a `subscribe()`-based Kafka
> consumer: the single-node group **coordinator** that ASSIGNS partitions to group members automatically via
> `JoinGroup` (11) / `SyncGroup` (14) / `Heartbeat` (12) / `LeaveGroup` (13). Builds on the already-shipped
> `FindCoordinator` (self), durable `OffsetCommit`/`OffsetFetch` (increment 3), and multi-partition (increment 4a).
> This is the **highest-concurrency-risk arc** in the project (two cross-connection parking points + timeouts), so
> it is frozen as a design first and will be implemented + adversarially audited as its own focused increment.

## Why design-first (and separately)
Every prior Kafka increment was a stateless request→response. Rebalance is different: a `JoinGroup` **blocks** the
connection thread until the *whole group* has assembled, and a follower's `SyncGroup` blocks until the *leader*
submits assignments. That means shared mutable group state across connection threads, condvar parking, and
join/session timeouts — the exact shape where deadlocks and races live. It earns its own design + its own brutal
audit, not a bolt-on.

## The coordinator state machine (per group, single node = us)
```
            JoinGroup (member joins / rejoins)
   Empty ───────────────► PreparingRebalance ──(join window closes)──► CompletingRebalance
     ▲                          ▲   │                                        │
     │ last member leaves       │   │ another member joins                   │ leader's SyncGroup arrives
     │                          │   ▼                                        ▼
     └──────────────────────── Stable ◄──────────────────────────────────────
                                  │ Heartbeat: NONE (or REBALANCE_IN_PROGRESS once a new join starts)
```
- **Empty** — no members.
- **PreparingRebalance** — ≥1 member has (re)joined; the *join window* is open. New joins are accepted; the window
  closes when every previously-known member has rejoined OR `rebalance_timeout_ms` elapses (whichever first; for a
  brand-new group, an initial short delay lets siblings join). On close: bump `generation_id`, pick the **leader**
  (oldest member), select the common assignment **protocol** (intersection of members' advertised protocols).
- **CompletingRebalance** — all parked `JoinGroup`s have been answered (leader gets the member list + their
  subscription metadata; followers get an empty list). Awaiting the leader's `SyncGroup` with assignments.
- **Stable** — assignments distributed; each member's parked `SyncGroup` answered with its assignment bytes.
  `Heartbeat` returns NONE while the generation holds.

## Per-group state (behind one `Mutex` + a `Condvar`)
```
struct Group {
    state: GroupState,
    generation_id: i32,
    protocol_type: String,            // "consumer"
    protocol: Option<String>,         // selected assignor, e.g. "range"
    leader: Option<String>,           // member_id
    members: BTreeMap<String, Member>,
    join_deadline: Option<Instant>,   // when the open join window closes
}
struct Member {
    subscription: Vec<u8>,            // JoinGroup protocol metadata (the leader needs every member's)
    assignment: Vec<u8>,             // set by the leader's SyncGroup; returned to this member
    last_heartbeat: Instant,
    session_timeout: Duration,
}
```
A single `Condvar` wakes parked `JoinGroup`/`SyncGroup` waiters on every state transition; each waiter re-checks
its predicate (its generation reached `CompletingRebalance` / its assignment is present) under the lock.

## The two parking points (the crux)
- **`JoinGroup`** — assign a `member_id` if empty (target v2–v4: assign + proceed; the v3+ `MEMBER_ID_REQUIRED`
  rejoin dance is an optional refinement), record the member's subscription, move to `PreparingRebalance`, set/extend
  `join_deadline`. Then **park on the condvar** until the group reaches `CompletingRebalance` (window closed) or the
  member's `session_timeout` expires. On wake: respond with `generation_id`, `protocol`, `leader_id`, `member_id`,
  and the **member array** (full {member_id, metadata} for the leader; empty for followers).
- **`SyncGroup`** — the **leader** carries the assignments ({member_id → assignment_bytes}); store them on each
  member, move to `Stable`, broadcast. A **follower** parks until its `assignment` is set (or timeout). Respond with
  the member's assignment bytes.

## Session expiry (liveness)
A member that stops heart-beating past its `session_timeout` is reaped (lazy: on any group access, evict members
whose `last_heartbeat` is stale; this triggers a rebalance). A lazy reaper avoids a background thread (leveza); a
bounded sweep on each coordinator call is enough for a single node.

## The seam
The coordinator is **broker-side protocol state**, independent of the sealed store — so it lives in `datarail-kafka`
(`groups` or a new `coordinator` module) as a `GroupCoordinator` owned by `serve_broker` (shared `Arc<Mutex<…>>`),
NOT behind the `KafkaBroker` trait (which is the per-(topic,partition) store seam). The assignment bytes are opaque
to us (the client's assignor produced them) — we only route them, so we never need to parse partition assignments.

## Honest scope / non-goals
- Target `JoinGroup` v2–v5, `SyncGroup` v0–v3, `Heartbeat` v0–v3, `LeaveGroup` v0–v3 (non-flexible where possible).
- The assignment itself is computed by the **client leader** (range/roundrobin/sticky) and is opaque to us — we are
  the coordinator/router, not the assignor. (Server-side assignment / KIP-848 next-gen protocol is out of scope.)
- Static membership (`group.instance.id`), incremental cooperative rebalance refinements — later.

## Build plan (each step tested; full adversarial audit of the concurrency at the end)
1. **This doc (frozen).**
2. `coordinator.rs`: the `GroupCoordinator` + `Group`/`Member` state machine, lock+condvar, lazy session expiry —
   unit-tested with synthetic concurrent joiners (no wire).
3. `groups.rs` codec for JoinGroup/SyncGroup/Heartbeat/LeaveGroup (version-gated) + ApiVersions advertises them.
4. Wire the four APIs into `serve_broker` (the coordinator is an `Arc<Mutex<…>>` shared across connection threads).
5. **Faithful wire test:** two concurrent consumer connections `subscribe()` to a 3-partition topic → both join one
   generation, the leader assigns, both get disjoint partitions, both heartbeat NONE; one leaves → rebalance.
6. (CI) a real librdkafka `subscribe()` consumer group round-trips against `datarail kafka-broker`.
7. **Brutal adversarial audit** focused on the concurrency: deadlock (lock ordering, condvar predicate), a member
   that never sends SyncGroup (timeout frees followers), a stale-generation Heartbeat/SyncGroup, a leader that
   crashes mid-rebalance, session-expiry races, and unbounded growth (members map / parked threads bounded).
