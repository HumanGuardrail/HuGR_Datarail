# KAFKA-TLS-DESIGN — TLS on the Kafka hop (closing hop-(1))

> **STATUS: DESIGN — ratified by the TechLead (owner delegated: "essas decisões são suas"). Build incrementally,
> each step green + a real-client test.**

## Why
`KAFKA-COMPAT.md` is explicit that **hop (1)** — the producer/consumer → `datarail kafka-ingest`/`kafka-broker`
link — is currently **PLAINTEXT** (no TLS/SASL). The moat (provider-blind) holds DOWNSTREAM (hop 2 is sealed), but
the first hop is "trusted-network only." TLS on hop (1) encrypts the wire so a passive tap on the producer↔datarail
link sees ciphertext, and lets datarail be reached over an untrusted network. This is KAFKA-COMPAT roadmap item #2
and the most-requested enterprise gate (many shops require TLS to any broker endpoint).

**Honest scope of what TLS buys (claims ≤ proof):** TLS is **transport encryption + server authentication** of
hop (1). It is NOT end-to-end sealing from the producer (that is datarail's native sealed terminals). After TLS
termination at datarail's edge, the records are plaintext in datarail's process for exactly as long as it takes to
**seal** them into a cofre (the existing moat). So: hop (1) encrypted-in-transit + datarail authenticated;
downstream still provider-blind. We will state this precisely, not imply end-to-end.

## The charter tension + the decision
`datarail-kafka` is **dependency-free by design** (its crate doc: "speaks only the wire protocol; it never sees
datarail keys or cofres"). Pulling `rustls` INTO `datarail-kafka` would break that purity. The doctrine ("smart
sealed endpoints, dumb cheap pipes") separates **transport** from **protocol**, so:

**DECISION (ratified): `datarail-kafka` stays zero-dependency. TLS is injected as a stream wrapper from the CLI.**
- `datarail-kafka` becomes **generic over the connection stream** (`S: Read + Write`) instead of hard-wired to
  `std::net::TcpStream`. The accept loop takes a small **`ConnWrap` trait** that turns a raw accepted `TcpStream`
  into a `Box<dyn ReadWrite + Send>`; `datarail-kafka` ships the identity `PlainConn` (no dep).
- `datarail-cli` provides a `TlsConn` (rustls `ServerConnection` + `rustls::StreamOwned`) behind a **`tls` cargo
  feature**, reusing the **already-vetted `rustls 0.23` + `ring`** that the `quic` feature pulls (NO new third-party
  crate enters the workspace — `rustls` is already in `Cargo.lock`). `rustls-pemfile` (tiny, ring-ecosystem) is the
  only addition, for PEM cert/key loading — logged as a WAIVER in `BUILD_LOG`.

This keeps the zero-dep wire crate pure, mirrors the `quic` feature-gating precedent (heavy deps one flag away,
default binary featherweight — G2), and reuses vetted crypto.

## Protocol/■ surface (what changes)
- **No Kafka-protocol change.** TLS is below the framing — the producer negotiates TLS, then speaks the identical
  Kafka wire bytes. ApiVersions/Metadata/Produce/Fetch/etc. are unchanged. librdkafka's `security.protocol=SSL`
  drives it.
- `serve` (ingest) and `serve_broker` both accept via the `ConnWrap`; the per-connection handlers
  (`handle_connection`, `handle_broker_connection`, `read_frame`) become generic over `S: Read + Write` — a
  mechanical change (they already only `read_exact`/`write_all`). The thread-per-connection model is unchanged;
  rustls has a synchronous `StreamOwned` that fits it exactly (no async runtime needed).

## CLI surface
`datarail kafka-ingest`/`kafka-broker` gain (behind `--features tls`):
- `--tls` — enable TLS termination on the listener.
- `--tls-cert <path>` `--tls-key <path>` — PEM server cert chain + private key.
- (later) `--tls-client-ca <path>` for **mTLS** (require + verify a client cert) — OUT of scope for v1 of this arc.

## Honest scope / non-goals (v1 of this arc)
- **Server-side TLS termination only** (datarail presents a cert; one-way auth). mTLS (client-cert) is a tracked
  follow-up (the `ConnWrap`/rustls config extends to it cleanly).
- **No SASL** (separate auth axis; tracked).
- TLS 1.2/1.3 via rustls defaults (1.3 preferred); ring backend (consistent with the quic substrate, avoids
  aws-lc-rs/nasm).
- A self-signed cert is fine for the test harness; production supplies its own chain.

## Build plan (each step green + committed; HUGR increments)
1. **This doc + ratification.** ✅
2. **Increment 1 — generic stream (no dep, no behavior change):** make `handle_connection`/`handle_broker_connection`
   /`read_frame` generic over `S: Read + Write`; introduce the `ConnWrap` trait + `PlainConn`; both serve loops route
   accepted streams through `ConnWrap`. Plaintext path identical; all existing wire tests green. PROVES the refactor
   is behavior-preserving before any TLS.
3. **Increment 2 — CLI `tls` feature + `TlsConn`:** add the `tls` feature (rustls + rustls-pemfile, WAIVER logged);
   load cert/key; build a `rustls::ServerConfig`; `TlsConn` wraps each `TcpStream` in `StreamOwned`. Wire `--tls
   --tls-cert --tls-key`.
4. **Increment 3 — real-client proof:** a CI workflow (mirrors `kafka-broker-librdkafka.yml`) — generate a
   self-signed cert, start `kafka-broker --tls`, drive **kcat with `-X security.protocol=SSL`** (real librdkafka TLS)
   → produce + consume round-trip over TLS. No PROVEN claim until this is green.
5. **Docs:** KAFKA-COMPAT (hop-1 now optionally TLS), README, LASTRO row, BUILD_LOG. Honest scope throughout.

## Open questions (TechLead-ratified, not blocking)
- **Q1 — dep locus.** rustls in the CLI (feature-gated) vs a new `datarail-kafka-tls` crate. → **CLI feature**
  (simplest; the CLI already owns the binary's heavy-feature flags). Revisit if a second consumer needs `TlsConn`.
- **Q2 — async?** No. The sync thread-per-connection model + rustls `StreamOwned` is sufficient and keeps the
  featherweight default. Async is a non-goal.
- **Q3 — cert reload / SNI / ALPN?** Out of scope v1 (single cert, no SNI). Tracked.
