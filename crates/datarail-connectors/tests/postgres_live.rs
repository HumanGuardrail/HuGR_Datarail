//! LIVE integration test for `PostgresSink` against a REAL `PostgreSQL` server. `#[ignore]` by default — the
//! unit tests in the crate prove framing/MD5/escaping; THIS proves the wire protocol round-trips against a real
//! backend. Run it with a Postgres reachable via env (a docker container in the verification harness):
//!
//! ```text
//! DATARAIL_PG_HOST=127.0.0.1 DATARAIL_PG_PORT=5432 \
//!   cargo test -p datarail-connectors --test postgres_live -- --ignored --nocapture
//! ```
//! The harness creates the target table first and verifies the landed rows afterwards (via `psql`).

use datarail_connectors::{PgConfig, PostgresSink, Sink};

#[test]
#[ignore = "needs a live Postgres (set DATARAIL_PG_* env); run in the docker verification harness"]
fn commits_records_into_a_real_postgres() {
    let host = std::env::var("DATARAIL_PG_HOST").unwrap_or_else(|_| "127.0.0.1".to_owned());
    let port: u16 = std::env::var("DATARAIL_PG_PORT").ok().and_then(|s| s.parse().ok()).unwrap_or(5432);
    let user = std::env::var("DATARAIL_PG_USER").unwrap_or_else(|_| "postgres".to_owned());
    let dbname = std::env::var("DATARAIL_PG_DB").unwrap_or_else(|_| "postgres".to_owned());
    let password = std::env::var("DATARAIL_PG_PASSWORD").ok();

    let mut cfg = PgConfig::new(host, user, dbname, "datarail_events".to_owned(), "data".to_owned());
    cfg.port = port;
    cfg.password = password;

    let mut sink = PostgresSink::connect(cfg).expect("connect to live postgres");
    let records = vec![
        b"evt:alpha".to_vec(),
        b"evt:beta\twith-tab".to_vec(),
        b"evt:gamma\\with-backslash".to_vec(),
    ];
    sink.commit(&records).expect("commit a batch into postgres");
    // A second batch — proves the connection stays usable for repeated COPY rounds.
    sink.commit(&[b"evt:delta".to_vec()]).expect("commit a second batch");
}
