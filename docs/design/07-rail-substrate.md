# 07 — Rail & Substrate

> **MF-2 SPEC. Status: DRAFT.** Capability E / AC-6,8. The ephemeral, dumb, substrate-polymorphic transport —
> the part that holds no secrets and can run on infrastructure you don't trust.

## Ephemeral lifecycle — INV-EPHEMERAL-RAIL

The rail is a **verb**: a transfer is a process that spawns on demand (the source has cofres + a route),
moves them, confirms ack, and exits. Zero standing presence; zero durable state (durability is at the
terminal/cofre/manifest, 05). It stays **warm during a burst** and **scales to zero after an idle timeout**
→ idle cost ≈ 0 (GATE-FEATHER). Granularity: ephemeral per **flow/shipment**, never cold-spawn per record.

## Substrate polymorphism — INV-SUBSTRATE-POLYMORPHIC, AC-6

The terminal speaks **one** rail API (`send(cofre) / recv() / ack`). Underneath, a `Substrate` trait,
statically bound per route:

| Substrate | When | How |
|---|---|---|
| **shmem** | same host | lock-free ring in shared memory; near-zero-copy framed cofres; µs hops |
| **QUIC** | cross host/cluster | streams, 0-RTT resume, congestion control, NAT traversal |
| **object-store / S3** | decoupled / cross-cloud | PUT sealed cofre → bucket; dest GETs; delete post-ack; brokerless, dirt-cheap, provider-blind (10) |

One acceptance suite passes over all three (AC-6). (We do **not** rely on QUIC's TLS for confidentiality — the
cofre is already sealed; QUIC is used for streams/flow/NAT only. Validates the DERP-style "blind relay".)

## Rate control — E3, AC-8 (FASP-physics, stolen from Aspera)

On the network substrate, congestion control is **delay-based** (BBR-like; act on measured queue growth),
**not loss-based** → throughput decoupled from RTT/packet-loss → large transfers over lossy / high-RTT WAN
don't collapse the way loss-based TCP does — but **inside the sealed tunnel** (FASP gives you speed *or*
zero-knowledge; we take both).

## Crash-safe resume — E4, AC-8 (BLAKE3 `bao`, stolen from Iroh)

Large cofres are chunked; a **verified-streaming `bao` bitfield** tracks received chunks. A dead rail
re-spawns and requests **only the missing chunks**; each chunk is authenticated against the BLAKE3 root
incrementally → safe resume from a partial/untrusted source. (This is also the signed-receipt root in 04.)

## DoS defense — E5 (WireGuard mac1/mac2)

The endpoint answers an unauthenticated connection with a **stateless proof-of-IP cookie** before allocating
state → spoofed-flood resistant, exactly right for a scale-to-zero endpoint.

## Open items

shmem ring format + crash cleanup · QUIC lib (`quinn`) + delay-based CC tuning · object-store layout/naming +
GC + post-ack delete · chunk size + `bao` params (ties to 02 max-cofre-size).
