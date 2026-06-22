//! AC-8 — resume across a partition. A cofre committed at the destination is offloaded **exactly once** even
//! when the link partitions before its ack lands and the source re-drives the stream on resume. Driven over
//! the [`ResumableSubstrate`]; redeliveries are deduped by the effectively-once gate inside the terminal.
//!
//! Asserts **0-loss** (every record committed) and **0-duplicate** (none committed twice), in order.

use datarail_core::{AeadAlg, Disposition, Substrate};
use datarail_crypto::{verifying_key, x25519_public};
use datarail_rail::ResumableSubstrate;
use datarail_terminal::{ContentContract, DestTerminal, SourceTerminal, TerminalConfig};

const SOURCE_SEED: [u8; 32] = [11; 32];
const DEST_SEED: [u8; 32] = [22; 32];
const DEST_X_SECRET: [u8; 32] = [5; 32];
const ROUTE: [u8; 16] = [1; 16];
const STREAM: [u8; 16] = [2; 16];
const TENANT: [u8; 32] = [6; 32];

fn config() -> TerminalConfig {
    TerminalConfig {
        route_id: ROUTE,
        stream_id: STREAM,
        aead_alg: AeadAlg::Gcmsiv256,
        dest_x25519_pk: x25519_public(&DEST_X_SECRET),
        tenant_secret: TENANT,
    }
}

fn contract() -> ContentContract {
    ContentContract::new(1024, b"evt:".to_vec())
}

#[test]
fn ac8_partition_then_resume_delivers_each_record_exactly_once() {
    const N: usize = 10;

    let source_vk = verifying_key(&SOURCE_SEED);
    let mut source = SourceTerminal::new(config(), contract(), SOURCE_SEED);
    let mut dest = DestTerminal::new(config(), contract(), source_vk, DEST_SEED, DEST_X_SECRET);
    let mut link = ResumableSubstrate::new();

    // Board N distinct cofres (distinct record-keys ⇒ distinct idempotency keys + advancing seq) and send all.
    let mut expected: Vec<Vec<u8>> = Vec::new();
    for i in 0..N {
        let payload = format!("evt:rec-{i:02}");
        let rk = format!("key-{i}");
        let cofre = source
            .board(&[payload.as_bytes()], rk.as_bytes())
            .expect("board");
        link.send(&cofre).expect("send");
        expected.push(payload.into_bytes());
    }

    // Offload + ack the first half — durable progress the resume will not re-drive.
    for _ in 0..N / 2 {
        let c = link.recv().expect("recv").expect("a cofre");
        assert_eq!(dest.offload(&c).expect("offload"), Disposition::Delivered);
        link.ack(c.etiqueta.cofre_id).expect("ack");
    }

    // Deliver one more and COMMIT it, but lose its ack (the partition hits before the ack lands).
    let mid = link.recv().expect("recv").expect("a cofre");
    assert_eq!(dest.offload(&mid).expect("offload"), Disposition::Delivered);

    // Partition: nothing flows. Then resume from the last acked position.
    link.partition();
    assert!(
        link.recv().expect("recv").is_none(),
        "no delivery while partitioned"
    );
    link.resume();

    // Drain: `mid` is redelivered (already committed ⇒ Duplicate, no re-commit); the remainder is delivered.
    while let Some(c) = link.recv().expect("recv") {
        let disp = dest.offload(&c).expect("offload");
        assert!(
            matches!(disp, Disposition::Delivered | Disposition::Duplicate),
            "every post-resume offload is Delivered or a benign Duplicate, got {disp:?}"
        );
        link.ack(c.etiqueta.cofre_id).expect("ack");
    }

    // 0-loss + 0-duplicate: every record committed exactly once, in order.
    assert_eq!(
        dest.sink().committed(),
        expected.as_slice(),
        "each record delivered exactly once, in order, across the partition"
    );
    assert!(
        dest.dead_letters().is_empty(),
        "nothing dead-lettered across the partition"
    );
}
