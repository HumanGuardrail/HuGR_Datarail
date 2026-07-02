# PRODUCT — what Datarail is and why it exists

> **Owner-reserved.** This document defines *what* and *why*. It is never edited autonomously; a needed
> change is a STOP-THE-LINE escalation to the Owner. **Status: DRAFT — pending Owner ratification + freeze (MF-0).**

## One line

Datarail is the **serverless, sealed data rail**: it moves data from a defined source to a
defined destination — across containers, hosts, clouds, or organizations — inside a **sealed vault the
pipe and the disk never see open** (zero-knowledge in rail mode; in broker mode the broker process is the
keyholder and only the *storage* is blind), delivered **intact, with cryptographic proof — exactly-once
where the sink supports it (Postgres append-ordered / idempotent Kafka), at-least-once elsewhere, never
silently lost**.

## The thesis

Moving data looks solved and isn't, at its core. Every incumbent is strong on one axis and
**structurally blind on another**:

- **Kafka / brokers** — durable and ubiquitous, but a standing cluster that *reads your data* and costs a
  fortune at rest (cross-AZ replication alone can dwarf the hardware bill).
- **MFT (Aspera/MOVEit/GoAnywhere)** — guaranteed secure A→B is literally their job, but on legacy servers
  that *decrypt your data* in transit — which is exactly why they became the ransomware jackpot of the
  decade (MOVEit leaked ~93M people).
- **Service mesh (Istio/Linkerd)** — cross-container reach, but a heavy per-pod sidecar whose mTLS
  *terminates at the proxy* (the proxy sees your plaintext), with no delivery guarantee.
- **ETL (Fivetran/Airbyte)** — connector breadth, but managed platforms that *see your data* and bill by
  the surprise.

The white space none of them occupy: **provider-blind + proof-carrying delivery (exactly-once on the
Postgres / idempotent-Kafka paths) + serverless + featherweight, on a sealed route.** That intersection is the moat — not any single piece.

## The doctrine

> **Smart sealed endpoints. Dumb cheap pipes.**

All intelligence and all secrets live in a featherweight endpoint (a library at the container edge). The
rail is the dumbest, cheapest substrate available — shared memory same-host, QUIC same-cluster, an S3
bucket cross-cloud, or a peer. **The seal is what lets the pipe be dumb, cheap, and untrusted**: because
the cargo is sealed end-to-end, you can move it over infrastructure you don't trust — including ours — and
a breach of the pipe yields useless ciphertext. From that one fact, all five obsessions fall out at once:
security, leveza, cost, performance, and cross-container reach.

## Mechanism vs policy (the product boundary)

- **We provide the mechanism:** the terminals (onboarding/offloading checkpoints + connectors), the cofre
  (the sealed vault format + crypto), the rail (ephemeral transport), the locomotive (the warp-speed
  engine).
- **The user provides only the policy:** the rules each terminal enforces (what must be true to board; what
  must be true to offload) and the keys. They configure the turnstile; they don't build the station.

## The wedge

The sharpest beachhead is where provider-blindness is **mandatory, not nice-to-have**:
**cross-organization and regulated data movement** — where the incumbent is structurally incapable
because it requires you to trust its infrastructure with your plaintext. MFT is the ripest target: legacy,
breached, expensive, and the job is exactly ours.

## What Datarail is NOT (scope discipline)

- Not a stream processor / transform engine (no Flink/Spark ambition).
- Not a general pub/sub overlay with dynamic discovery — routes are fixed A→B.
- Not an L7 traffic manager (no Istio-style shaping/observability of payload — we are blind by design).
- Not a connector marketplace (a curated few, not a long-tail catalog).
- Not a confidential-compute platform (we never compute on plaintext; that is the user's terminal).
