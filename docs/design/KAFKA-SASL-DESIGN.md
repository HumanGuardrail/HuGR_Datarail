# KAFKA-SASL-DESIGN — SASL/PLAIN authentication on the Kafka hop

> **STATUS: DESIGN — ratified by the TechLead (owner-delegated "siga autônomo"). Build incrementally, each step
> green + proven against real librdkafka.**

## Why
`KAFKA-COMPAT.md` lists SASL as the remaining auth axis after TLS (now shipped). **SASL + TLS is the standard
enterprise broker-auth combo** — many shops require both before any client may produce/consume. TLS gives transport
encryption + server auth; SASL adds **client authentication** (the broker verifies *who* the producer/consumer is).
Closing it makes datarail usable where an authenticated broker endpoint is mandatory.

## Scope (v1) — SASL/PLAIN, the modern wrapped handshake
- **Mechanism: `PLAIN`** (RFC 4616) — a username + password. Other mechanisms (SCRAM-SHA-256/512, GSSAPI/OAUTH) are
  tracked follow-ups; PLAIN is the common case and the simplest correct one.
- **KIP-152 wrapped form** (what modern librdkafka uses): `SaslHandshake` (API 17) **v1** negotiates the mechanism,
  then the SASL token travels inside **`SaslAuthenticate`** (API 36) requests (not as raw bytes). We target v1.
- **Single configured credential** for v1 (`--sasl-user` / `--sasl-pass`), checked with a **constant-time** compare.
  A credential store / multiple users is a follow-up.
- **`kafka-broker` mode.** Pairs with `--tls` for `SASL_SSL`; works alone as `SASL_PLAINTEXT` (PLAIN sends the
  password in the clear, so production should combine it with `--tls` — documented honestly).

## The wire flow (per connection)
```
[client] ApiVersions ───────────────►  (always allowed)
[client] SaslHandshake(v1, "PLAIN") ─►  [broker] error_code=0 + mechanisms=["PLAIN"]   (or 33 if unsupported)
[client] SaslAuthenticate(authbytes) ►  [broker] verify → error_code=0 (+ empty bytes + session_lifetime)
                                                          or 58 SASL_AUTHENTICATION_FAILED
[client] Produce / Fetch / … ───────►  (only AFTER a successful SaslAuthenticate)
```
- **PLAIN authbytes** (RFC 4616): `authzid \0 authcid \0 passwd` (authzid usually empty) → split on NUL, take
  authcid (username) + passwd, constant-time compare to the configured credential.
- **Pre-auth gating:** when SASL is enabled, a connection is **unauthenticated** until a successful
  `SaslAuthenticate`. Before that, only `ApiVersions`, `SaslHandshake`, `SaslAuthenticate` are served; any other API
  → `35 ILLEGAL_SASL_STATE` (or close). After success → normal serving.

## Error codes
`33 UNSUPPORTED_SASL_MECHANISM`, `58 SASL_AUTHENTICATION_FAILED`, `35 ILLEGAL_SASL_STATE`, `0 NONE`.

## Where it lives (no new dep)
SASL is connection-level auth, not transport — it sits in the request loop, above the framing (and above the TLS
`ConnWrap`). So it lives in `datarail-kafka` `serve_broker` / `handle_broker_connection`: a per-connection `SaslState`
(`Unauthenticated` → `Authenticated`) + the two new API handlers. **No new dependency** — PLAIN is NUL-split +
a hand-rolled constant-time byte compare (XOR-accumulate; avoids a `subtle` dep + keeps `forbid(unsafe)`). The
credential is passed into `serve_broker` (e.g. `Option<SaslCreds { user, pass }>`); `None` → SASL off (today's
behavior, unchanged). The codec (parse/build for APIs 17 + 36) is pure wire, zero-dep like the rest of the crate.

## CLI surface
`datarail kafka-broker … --sasl-user <u> --sasl-pass <p>` enables SASL/PLAIN. Combine with `--tls --tls-cert
--tls-key` for `SASL_SSL`. Without `--tls`, it is `SASL_PLAINTEXT` (a warning is printed: the password is on the
wire in the clear).

## Honest scope / non-goals (v1)
- PLAIN only (SCRAM/GSSAPI/OAUTH tracked). Single credential (a store is tracked). `kafka-broker` mode.
- SASL/PLAIN sends the password base64-of-nothing — i.e. in the clear — so it is only secure **over TLS**; we state
  that plainly and let the operator combine `--sasl` with `--tls`.
- Not authorization (ACLs) — this is authentication only.

## Build plan (each step green + a real-librdkafka-SASL CI proof)
1. **This doc + ratification.** ✅
2. **Incr 1 — codec (`sasl.rs`):** parse/build `SaslHandshake` (17) v0–v1 + `SaslAuthenticate` (36) v0–v1; PLAIN
   authbytes split; constant-time compare; advertise both in `ApiVersions` SUPPORTED. Unit tests (parse a real
   handshake + a PLAIN token; good vs bad credential; constant-time compare).
3. **Incr 2 — serve wiring:** a per-connection `SaslState`; `serve_broker` takes `Option<SaslCreds>`; the request
   loop gates non-auth APIs pre-auth (`ILLEGAL_SASL_STATE`) and authenticates on `SaslAuthenticate`. A full-binary
   wire test (handshake → auth → produce/fetch works; wrong password → 58; produce-before-auth → 35).
4. **Incr 3 — CLI flags** `--sasl-user/--sasl-pass` (+ the no-TLS warning); `None` keeps today's behavior.
5. **Incr 4 — real-client CI** (`kafka-broker-sasl.yml`): kcat with `-X security.protocol=SASL_PLAINTEXT -X
   sasl.mechanism=PLAIN -X sasl.username -X sasl.password` produce+consume round-trip; a wrong-password run is
   rejected. NO PROVEN claim until green. (Also a `SASL_SSL` leg combining with `--tls`.)
6. **Docs:** KAFKA-COMPAT (SASL now supported), README, LASTRO, BUILD_LOG.

## Open questions (TechLead-ratified)
- **Q1 — mechanism set:** PLAIN only for v1 (SCRAM is the obvious next, needs an HMAC-SHA impl — a dep decision then).
- **Q2 — credential source:** CLI flags for v1; a file/env store later.
- **Q3 — apply to `kafka-ingest` too?** v1 targets `kafka-broker` (the full drop-in clients authenticate to); the
  ingest path can adopt the same `SaslState` after.
