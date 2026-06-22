# 04 — Manifest & Delivery Proof

> **MF-2 SPEC. Status: DRAFT.** Capability B / AC-5. How *"it arrived, exactly once, intact"* becomes a thing
> you can **prove offline, without trusting the pipe.** Stolen pattern: transparency-log inclusion proofs
> (Sigstore/Rekor).

## The manifest = a per-route append-only Merkle log

- Each shipped cofre contributes a leaf that **commits to `route_id ‖ stream_id ‖ seq` (+ epoch) and `cofre_id`**:
  `leaf = BLAKE3(cofre_id ‖ route_id ‖ stream_id ‖ seq ‖ epoch)` (BLK-8) — binding the proof to its
  route/stream/position so it cannot be replayed onto another route or sequence. *(The leaf does NOT bind
  `STH-root` — that is circular, since leaves feed the root the STH signs; the `dest_ack` binds `STH-root`
  instead, below. Circularity flagged by the apply-clone, lead-resolved.)*
- The source terminal maintains the Merkle tree; the **Signed Tree Head (STH)** `= Ed25519(root ‖ tree_size
  ‖ ts)` by the source identity key.
- **STH cadence (MAJ-1):** the STH is cut on a **pinned cadence — every N cofres or every T ms**, fully
  **decoupled from the per-cofre ack** (the ack does not wait on an STH). **TSA anchoring is asynchronous and OFF
  the delivery path**: at shipment close (and periodically) the STH is anchored with an **external RFC-3161
  timestamp** from a TSA → time proven **without trusting the rail's clock** (B3), but never gating delivery.

## Delivery receipt = inclusion proof

- On offload, the destination returns a signed **ack** that **commits to `route_id ‖ stream_id ‖ seq ‖ STH-root`
  (+ epoch)** (BLK-8): `dest_ack = {cofre_id, route_id, stream_id, seq, STH-root, epoch, dest_sig}` — so the ack
  cannot be lifted onto a different route/stream/position.
- "Cofre X was delivered" = `{leaf, inclusion_proof→root, STH, TSA_token, dest_ack}`. Any auditor verifies
  **offline**: leaf ∈ tree (Merkle path) · STH signed by source · TSA token valid · `dest_ack` signed by dest,
  its `route_id‖stream_id‖seq`(+epoch) matches the leaf, and its bound `STH-root` matches the inclusion-proof
  root · and the verifier **recomputes `cofre_id == BLAKE3(CARGA)` and REJECTS on mismatch** (BLK-8). **Zero
  trust in the pipe.**

## Reconciliation (source ⊕ destination) — INV-MANIFEST-RECONCILES

- Source STH: "N cofres, root R". Destination keeps its own received-Merkle. Reconcile = compare roots →
  derive the exact missing/extra set. A gap is detected **with certainty** (integrity). *Filling* the gap is
  the substrate's job + resume (07); the manifest's job is to make loss **undeniable**, not impossible.

## What the rail contributes

Nothing secret — it forwards cofres + acks. It cannot forge a leaf (`cofre_id` is content-addressed +
sealed) nor a `dest_ack` (signed). Manifest integrity holds over a fully untrusted pipe.

## What the TSA proves — honestly (MAJ-3)

The RFC-3161 TSA token proves an **UPPER bound on time only**: it shows the STH existed **no later than** the
timestamp — i.e. it is **anti-postdating** (you cannot claim a shipment is newer than it is). It does **NOT** prove
a lower bound (it cannot stop antedating on its own). For a **lower bound**, fold a **fresh external
beacon/nonce** (e.g. a randomness beacon value) into the STH before signing — the STH then cannot have been
produced before that beacon was published.

## Open items

- Manifest persistence: source-terminal-local, with an optional mirror to the dumb store (10).
