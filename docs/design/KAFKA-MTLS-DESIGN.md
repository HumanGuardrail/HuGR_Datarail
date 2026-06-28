# KAFKA-MTLS-DESIGN — mutual TLS (client-certificate auth) on the Kafka hop

> **STATUS: DESIGN — ratified by the TechLead (owner-delegated "siga autônomo"). Build incrementally, each step
> green + proven against real librdkafka.**

## Why
TLS (shipped, `KAFKA-TLS-DESIGN.md`) authenticates the *server* and encrypts hop (1). **mTLS** adds the other
direction: the broker **requires + verifies the CLIENT's certificate**, so only a client holding a cert signed by a
configured CA may connect. It is the cert-based alternative to SASL for client authentication — common where an
mTLS mesh / PKI already issues client certs. It completes the mutual-auth story and **reuses the entire TLS
machinery** (the rustls `ConnWrap` + the `tls` feature) — the only change is the rustls `ServerConfig` verifier.

## Scope (v1)
- Behind the existing **`tls` feature**. `kafka-broker --tls --tls-cert --tls-key --tls-client-ca <pem>`: when
  `--tls-client-ca` is given, the broker **requires** a client cert chaining to that CA; a client with no cert (or a
  cert from another CA) **fails the TLS handshake** (the connection never reaches the Kafka protocol).
- **Authentication, not authorization.** The broker proves the client holds a CA-signed cert; it does NOT map the
  cert to a per-identity ACL (that is a separate axis, tracked). Combine with nothing else needed — mTLS alone
  gates the connection.
- `--tls-client-ca` **requires `--tls`** (mTLS is meaningless without server TLS).

## Mechanics (the only change vs one-way TLS)
`cli/src/tls.rs` `TlsConn::from_pem` gains an optional `client_ca` path. The rustls `ServerConfig` builder picks the
verifier:
- `None` → `.with_no_client_auth()` (today's one-way TLS, unchanged).
- `Some(ca)` → load the CA PEM into a `rustls::RootCertStore`, build
  `rustls::server::WebPkiClientVerifier::builder(Arc::new(roots)).build()` → `Arc<dyn ClientCertVerifier>`, and use
  `.with_client_cert_verifier(verifier)`. rustls then **requires** a client cert during the handshake and verifies
  its chain to the roots; an unverifiable/absent client cert aborts the handshake (our `wrap` returns the rustls
  error → the connection is dropped). No new dependency — `rustls` (already pulled by `tls`) ships the WebPKI client
  verifier.

## CLI surface
`datarail kafka-broker … --tls --tls-cert server.pem --tls-key server.key --tls-client-ca ca.pem` → mutual TLS.
Without `--tls-client-ca`, behavior is exactly today's one-way TLS.

## Honest scope / non-goals (v1)
- Client **authentication** by cert chain only; no SAN/CN → identity → ACL mapping (tracked).
- Single client-CA roots file (a bundle of CA certs is fine — `RootCertStore` accepts multiple).
- `kafka-broker` mode (the `tls` feature already targets it).

## Build plan (each step green + a real-librdkafka-mTLS CI proof)
1. **This doc + ratification.** ✅
2. **Incr 1 — verifier + flag:** `TlsConn::from_pem(cert, key, client_ca: Option<&str>)` builds the
   `WebPkiClientVerifier` when `client_ca` is `Some`; CLI `--tls-client-ca` (requires `--tls`); thread through
   `build_conn`. Verify against rustls 0.23's exact API (adapt to what compiles). Default (no `tls` feature) and
   one-way TLS unchanged.
3. **Incr 2 — real-client CI** (`kafka-broker-mtls.yml`): openssl generates a CA + a server cert + a client cert
   (both signed by the CA); start `kafka-broker --tls --tls-cert --tls-key --tls-client-ca ca.pem`; kcat with
   `-X security.protocol=SSL -X ssl.ca.location=ca.pem -X ssl.certificate.location=client.pem -X
   ssl.key.location=client.key` produce+consume round-trip; a kcat run **without** a client cert **MUST fail**.
   Force `127.0.0.1`. NO PROVEN claim until green.
4. **Docs:** KAFKA-COMPAT (mTLS now supported), README, LASTRO, BUILD_LOG.

## Open questions (TechLead-ratified)
- **Q1 — optional vs required client cert?** Required (when `--tls-client-ca` is set) — that is the point of mTLS.
  An "optional client cert" mode is not useful for gating; skip it.
- **Q2 — cert → identity ACL?** Out of scope v1 (authentication only). Tracked.
