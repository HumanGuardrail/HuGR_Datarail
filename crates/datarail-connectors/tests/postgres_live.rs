//! LIVE integration test for `PostgresSink` against a REAL `PostgreSQL` server. `#[ignore]` by default — the
//! unit tests in the crate prove framing/MD5/escaping; THIS proves the wire protocol round-trips against a real
//! backend. Run it with a Postgres reachable via env (a docker container in the verification harness):
//!
//! ```text
//! DATARAIL_PG_HOST=127.0.0.1 DATARAIL_PG_PORT=5432 \
//!   cargo test -p datarail-connectors --test postgres_live -- --ignored --nocapture
//! ```
//! The harness creates the target table first and verifies the landed rows afterwards (via `psql`).

use datarail_connectors::{PgConfig, PostgresSink, Sink, TxnSink};

/// Shared connection config from the env (the docker verification harness sets these).
fn live_cfg(table: &str) -> PgConfig {
    let host = std::env::var("DATARAIL_PG_HOST").unwrap_or_else(|_| "127.0.0.1".to_owned());
    let port: u16 = std::env::var("DATARAIL_PG_PORT").ok().and_then(|s| s.parse().ok()).unwrap_or(5432);
    let user = std::env::var("DATARAIL_PG_USER").unwrap_or_else(|_| "postgres".to_owned());
    let dbname = std::env::var("DATARAIL_PG_DB").unwrap_or_else(|_| "postgres".to_owned());
    let password = std::env::var("DATARAIL_PG_PASSWORD").ok();
    let mut cfg = PgConfig::new(host, user, dbname, table.to_owned(), "data".to_owned());
    cfg.port = port;
    cfg.password = password;
    cfg
}

#[test]
#[ignore = "needs a live Postgres (set DATARAIL_PG_* env); run in the docker verification harness"]
fn commits_records_into_a_real_postgres() {
    let mut sink = PostgresSink::connect(live_cfg("datarail_events")).expect("connect to live postgres");
    let records = vec![
        b"evt:alpha".to_vec(),
        b"evt:beta\twith-tab".to_vec(),
        b"evt:gamma\\with-backslash".to_vec(),
    ];
    sink.commit(&records).expect("commit a batch into postgres");
    // A second batch — proves the connection stays usable for repeated COPY rounds.
    sink.commit(&[b"evt:delta".to_vec()]).expect("commit a second batch");
}

#[test]
#[ignore = "needs a live Postgres (set DATARAIL_PG_* env); run in the docker verification harness"]
fn exactly_once_a_replayed_batch_does_not_double_land() {
    // The moat (EXACTLY-ONCE-DESIGN.md, Tier A): commit_at lands records + the watermark in one transaction, so
    // a redelivered batch (same watermark) is a committed NO-OP — exactly-once across a crash, with the dedup
    // watermark living transactionally in Postgres itself.
    let mut sink = PostgresSink::connect(live_cfg("datarail_eo")).expect("connect");
    let stream = b"route-xyz";
    let batch1 = vec![b"eo:a".to_vec(), b"eo:b".to_vec(), b"eo:c".to_vec()];

    sink.commit_at(&batch1, stream, 3).expect("commit batch1");
    // Simulate an at-least-once REDELIVERY after a 'crash': the exact same batch + watermark. Must be a no-op.
    sink.commit_at(&batch1, stream, 3).expect("replay batch1 (idempotent)");
    sink.commit_at(&batch1, stream, 3).expect("replay batch1 again");
    assert_eq!(sink.resume_watermark(stream).expect("resume"), 3, "watermark resumes at 3");

    // A genuinely new batch advances the watermark and lands.
    sink.commit_at(&[b"eo:d".to_vec()], stream, 4).expect("commit batch2");
    sink.commit_at(&[b"eo:d".to_vec()], stream, 4).expect("replay batch2"); // redelivery — no-op
    assert_eq!(sink.resume_watermark(stream).expect("resume2"), 4, "watermark resumes at 4");
    // The harness asserts exactly 4 rows landed (a,b,c,d) despite the replays.
}
