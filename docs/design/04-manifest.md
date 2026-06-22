# 04 — Manifest & Delivery Proof

> **MF-2 SPEC. Status: DRAFT.** Capability B / AC-5. How *"it arrived, exactly once, intact"* becomes a thing
> you can **prove offline, without trusting the pipe.** Stolen pattern: transparency-log inclusion proofs
> (Sigstore/Rekor).

## The manifest = a per-route append-only Merkle log

- Each shipped cofre contributes a leaf `= BLAKE3(cofre_id ‖ seq ‖ idempotency_key)`.
- The source terminal maintains the Merkle tree; the **Signed Tree Head (STH)** `= Ed25519(root ‖ tree_size
  ‖ ts)` by the source identity key.
- At shipment close (and periodically), the STH is anchored with an **external RFC-3161 timestamp** from a TSA
  → time proven **without trusting the rail's clock** (B3).

## Delivery receipt = inclusion proof

- On offload, the destination returns a signed **ack** `{cofre_id, dest_sig}`.
- "Cofre X was delivered" = `{leaf, inclusion_proof→root, STH, TSA_token, dest_ack}`. Any auditor verifies
  **offline**: leaf ∈ tree (Merkle path) · STH signed by source · TSA token valid · dest_ack signed by dest.
  **Zero trust in the pipe.**

## Reconciliation (source ⊕ destination) — INV-MANIFEST-RECONCILES

- Source STH: "N cofres, root R". Destination keeps its own received-Merkle. Reconcile = compare roots →
  derive the exact missing/extra set. A gap is detected **with certainty** (integrity). *Filling* the gap is
  the substrate's job + resume (07); the manifest's job is to make loss **undeniable**, not impossible.

## What the rail contributes

Nothing secret — it forwards cofres + acks. It cannot forge a leaf (`cofre_id` is content-addressed +
sealed) nor a `dest_ack` (signed). Manifest integrity holds over a fully untrusted pipe.

## Open items

- Manifest persistence: source-terminal-local, with an optional mirror to the dumb store (10).
- STH cadence: per-cofre STH is expensive → **batch the STH** (amortize, same instinct as batch-sign).
