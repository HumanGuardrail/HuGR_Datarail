# 08 — Routing & Identity

> **MF-2 SPEC. Status: DRAFT.** Capability F. Fixed A→B routes; pinned endpoint identity; no discovery.

## A route is a static object

`Route = { route_id, source_endpoint, dest_endpoint, substrate, guarantee_level }` — declared in the rail
spec (09), never discovered. Fan-out 1→N = **N routes**; this never reintroduces a DHT.

## Endpoint identity

Each endpoint holds `(Ed25519 sign key, X25519 KEM key)`. Identity id = `BLAKE3(ed25519_pub ‖ x25519_pub)` —
**self-authenticating** (Syncthing pattern); or a SPIFFE SVID where SPIFFE/SPIRE already exists. Each route
**pins** the peer's static pubkeys ⇒ **Noise_KK** (both statics known a priori): mutual auth + a shared
secret, no PKI round-trips, no negotiation (WireGuard discipline — one fixed suite).

## Bootstrap (two modes)

1. **Pre-provisioned** — `datarail keygen` emits an identity; ops places peer pubkeys in the spec / a secrets
   store.
2. **PAKE short-code** — zero-PKI: an operator pastes a one-time code on both ends (SPAKE2, wormhole-style)
   → derives the pinned identities. **Audited PAKE crate only** — never roll our own (croc's CVE-2021-31603:
   home-grown PAKE → plaintext recovery). **PAKE hardening (MAJ-5):**
   - **High-entropy, single-use code** — the pairing code carries enough entropy that online guessing is
     infeasible, and it is **burned after one use** (never reusable).
   - **Hard attempt-cap with burn-on-fail** — a strict cap on failed attempts; **on exceeding it the code is
     burned** (invalidated) and pairing must restart with a fresh code, defeating online dictionary attacks.
   - **Identity binding into the transcript** — both endpoint identities (their static pubkeys) are **bound into
     the PAKE transcript**, so a successful exchange authenticates *which* endpoints paired and blocks
     relay/MITM substitution.

## Route descriptor (capability token)

A self-contained, **signed**, offline-shareable token `{route_id, endpoint ids, pubkeys, substrate hint}`
(Iroh-ticket pattern). Possessing a valid descriptor = the capability to board/receive on that route.

## Why this stays trivially provider-blind

Identity + key agreement happen **at the endpoints**; the rail only ever sees `signer_key_id` (a hash) in
the etiqueta and verifies the `LACRE` against the route's pinned key. The rail never holds a private key.
