---
title: "The one thing Kafka can't give you: an offline-verifiable delivery receipt"
dek: "What a cryptographic Merkle delivery proof actually is, why it is a different guarantee than at-rest encryption, and what honest scope it has."
date: 2026-07-02
---

# The one thing Kafka can't give you: an offline-verifiable delivery receipt

Let me be precise about what I mean by "can't give you," because the honest version of this
claim is narrower than it might sound.

Kafka gives you durable delivery. With `acks=all` and replication, you know your records landed
and will survive node failures. That is a real and valuable guarantee. What Kafka does not give
you — what no general-purpose message broker gives you by design — is a delivery receipt that
a third party can verify, offline, without trusting any component of the system that produced it.

This post is about that specific gap, what it takes to close it, and what datarail actually
implements versus what it still does not.

## What a Merkle delivery receipt is

When a record is sealed into a cofre and delivered to the destination terminal, the terminal
builds a Merkle tree over the delivered records using BLAKE3-256 as the hash function. The root
of that tree is signed with the source's Ed25519 key — the signing type used throughout datarail
for authentication (domain-separated per role to prevent cross-protocol confusion). The result
is a signed tree head (STH): a compact commitment to the entire delivered set.

A Merkle proof for a specific record is the path from that record's leaf hash to the signed
root — a logarithmic-size witness that proves "this specific record is in the set the source
committed to." That proof is self-contained: it carries no runtime state, requires no network
call, and trusts nothing except the source's public key, which can be pinned offline by any
verifier. It is implemented in the `datarail-manifest` crate, with BLAKE3-bao chunk-resume for
partial-tree verification.

The Ed25519 signature is what provides non-repudiation. The source cannot produce a valid lacre
(the per-cofre signature) for a record it did not seal, and cannot produce a valid STH for a
set it did not commit to. A storage-layer attacker who intercepts and rewrites objects gets
ciphertext and cannot forge either.

## Why this is different from CSFLE or Kroxylicious

Client-Side Field-Level Encryption (CSFLE) and proxy-level encryption tools like Kroxylicious
can hide record payloads from the broker. That is a real security property: a broker that
cannot see your records cannot leak them, sell them, or be compelled to disclose them.

But they do not give you non-repudiable proof of delivery. A broker that holds only ciphertext
can still drop records, reorder them, or substitute one ciphertext for another — and the
consumer has no way to detect this without a signed commitment from the sender. The sender's
signature has to cover the ciphertext-as-delivered, not just the plaintext. If the broker is in
the middle, it can always insert, delete, or reorder records in ways that are undetectable unless
the sender has signed a commitment to the specific sequence.

The datarail cofre format addresses this at the record level: per-record X25519 key-wrap seals a
fresh data key to the destination's public key (only the destination can open it), and the
Ed25519 lacre covers `etiqueta (header) || carga (ciphertext)` — so any modification to either
the header or the payload invalidates the signature. The Merkle STH then commits the source to
the specific set and order of records delivered.

This gives you three properties together that I do not know how to get from a broker that
holds your keys or sits in the plaintext path:

1. The payload is hidden from every carrier between source and destination (including the storage
   layer, including a compromised disk, including the operator of the storage service).
2. The source has signed both the payload ciphertext and the delivery sequence. A receipt for a
   record that was not actually delivered cannot be forged.
3. The receipt can be verified by a third party with only the source's Ed25519 verifying key —
   no runtime, no trusted intermediary.

## The piece worth extracting

The delivery proof sits in `datarail-manifest`. It is the most separable piece of the system —
it depends only on the cofre format, BLAKE3, and Ed25519, and has no runtime state beyond the
tree itself. If you have a system that already does end-to-end sealing and you want to bolt on
offline-verifiable delivery proofs, `datarail-manifest` is closer to a library than to a
product. The BLAKE3-bao integration means proofs are chunk-resumable: a verifier can check a
single record's inclusion without receiving the full batch.

This is the narrow wedge that might actually matter outside datarail itself.

## The trust model distinction that must not be blurred

The README calls this out explicitly because it is easy to get wrong:

**Rail mode** (`datarail send / recv / run`): the endpoints — and only the endpoints — hold the
keys. Every carrier between source and destination (TCP, QUIC, shmem, S3) sees only opaque
ciphertext. The Merkle delivery proof is signed at the source and verified at the destination.
Nothing in the middle can produce a valid proof for a record it fabricated. This is the mode
where the zero-knowledge claim holds.

**Broker mode** (`datarail kafka-broker`): the broker process holds all keys. It seals on
produce and un-seals on fetch. The on-disk log is ciphertext-only — what the design calls
provider-blind storage, meaning a disk-theft adversary sees sealed cofres, not records. But the
running broker process has the destination's X25519 secret and can open any cofre it has
received. It is encryption at rest done properly, not zero-knowledge.

A Merkle delivery receipt signed in broker mode is signed by the broker on behalf of the route.
That is not a non-repudiable proof of what the *producer* sent — it is a proof of what the
*broker* sealed. An adversary who controls the broker process can make it sign whatever it
wants. The value in broker mode is provider-blind storage and protocol-level compatibility with
unmodified Kafka clients; the non-repudiation property belongs only to rail mode.

This distinction is important enough that the SECURITY.md threat model calls it out under
"What the design does NOT defend against": a compromised broker process can read everything.
Disk-theft protection is the goal in broker mode; process-compromise protection is not.

## What I am not claiming

I am not claiming this is a market-ready product. The broker is a single-node prototype with no
replication or failover. The delivery proof has had no third-party cryptographic review — the
SECURITY.md says this clearly: "The composition has had no third-party cryptographic review. It
is self-audited only."

I am not claiming that the Merkle delivery receipt solves a problem that most Kafka users have.
Most applications that use Kafka do not need a receipt that can be verified offline by a party
that was not present at delivery. The ordinary Kafka delivery guarantee — durable, replicated,
offset-committed — is what most people need, and Kafka gives it.

The gap I am pointing at is narrower: for applications where the producer and the storage
operator are different parties, where a regulator or auditor may need to verify delivery without
trusting either the broker or the infrastructure operator, and where the producer wants
non-repudiation to be a cryptographic property rather than an operational one, the Merkle
delivery proof is a genuinely different guarantee. The independent audit confirmed that the
provider-blind property replicates correctly: grepping the on-disk data directory after a
50,000-message produce+consume cycle produced zero plaintext hits.

That narrow wedge is what might actually matter. Whether it matters for your use case is
something I will not claim to know.
