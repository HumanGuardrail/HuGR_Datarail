//! AC-9 (content-contract refusals, both sides) + exactly-once integration proofs for the terminals.

use super::{
    ContentContract, DeadLetterReason, DestTerminal, SourceTerminal, TerminalConfig, TerminalError,
};
use datarail_core::{AeadAlg, Disposition};
use datarail_cofre::CofreError;
use datarail_crypto::{verifying_key, x25519_public};

const SOURCE_SEED: [u8; 32] = [11u8; 32];
const DEST_SEED: [u8; 32] = [22u8; 32];
const EVIL_SEED: [u8; 32] = [99u8; 32];
const ROUTE: [u8; 16] = [1u8; 16];
const STREAM: [u8; 16] = [2u8; 16];
const DEST_X_SECRET: [u8; 32] = [33u8; 32];
const TENANT: [u8; 32] = [5u8; 32];

const PREFIX: &[u8] = b"OK:";
const MAX_LEN: usize = 64;

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
    ContentContract::new(MAX_LEN, PREFIX.to_vec())
}

fn source() -> SourceTerminal {
    SourceTerminal::new(config(), contract(), SOURCE_SEED)
}

fn dest() -> DestTerminal {
    DestTerminal::new(config(), contract(), verifying_key(&SOURCE_SEED), DEST_SEED, DEST_X_SECRET)
}

// ---- ContentContract unit behaviour ----------------------------------------------------------------------

#[test]
fn contract_validate_rules() {
    let c = contract();
    assert!(c.validate(b"OK:hello"));
    assert!(!c.validate(b""), "empty is rejected");
    assert!(!c.validate(b"NO:hello"), "wrong prefix is rejected");
    let too_long = [b'x'; MAX_LEN + 1];
    assert!(!c.validate(&too_long), "over max_record_len is rejected");
    assert!(c.validate(PREFIX), "exactly the prefix, at min len, is ok");
}

#[test]
fn same_rules_same_fingerprint() {
    // Two terminals configured with the same rules must agree on the schema id; different rules differ.
    assert_eq!(contract().fingerprint, contract().fingerprint);
    assert_ne!(
        contract().fingerprint,
        ContentContract::new(MAX_LEN, b"OTHER:".to_vec()).fingerprint
    );
    assert_ne!(
        contract().fingerprint,
        ContentContract::new(MAX_LEN + 1, PREFIX.to_vec()).fingerprint
    );
}

// ---- Happy path: a conforming batch boards and offloads, records land in the sink -------------------------

#[test]
fn round_trip_delivers_records() {
    let mut src = source();
    let mut dst = dest();
    let recs: [&[u8]; 2] = [b"OK:alpha", b"OK:beta"];
    let cofre = src.board(&recs, b"record-key-1").expect("board");
    assert_eq!(dst.offload(&cofre).expect("offload"), Disposition::Delivered);
    assert_eq!(dst.sink().committed(), &[b"OK:alpha".to_vec(), b"OK:beta".to_vec()]);
    assert!(dst.dead_letters().is_empty());
    assert_eq!(src.next_seq(), 1);
}

// ---- AC-9 (a): onboarding refuses a contract-violating record — NEVER boards -----------------------------

#[test]
fn ac9_board_refuses_contract_violating_record() {
    let mut src = source();
    // One good, one bad (wrong prefix) record: the whole batch must be refused.
    let recs: [&[u8]; 2] = [b"OK:good", b"BAD:nope"];
    assert_eq!(src.board(&recs, b"rk"), Err(TerminalError::ContractViolation));
    // It never boarded: the sequence did not advance.
    assert_eq!(src.next_seq(), 0);

    // An empty record is likewise refused.
    let recs2: [&[u8]; 1] = [b""];
    assert_eq!(src.board(&recs2, b"rk"), Err(TerminalError::ContractViolation));
    assert_eq!(src.next_seq(), 0);
}

// ---- AC-9 (b): offload of contract-violating records ⇒ dead-lettered, NOT committed ----------------------

#[test]
fn ac9_offload_dead_letters_contract_violation_not_committed() {
    // The *source* enforces a permissive contract (boards anything non-empty); the *dest* enforces the
    // strict PREFIX contract — so a record that boards can still violate the offloading contract (AC-9).
    let permissive = ContentContract::new(MAX_LEN, Vec::new());
    let mut src = SourceTerminal::new(config(), permissive, SOURCE_SEED);
    let mut dst = dest(); // strict PREFIX contract

    let recs: [&[u8]; 1] = [b"NO-PREFIX-here"]; // boards under permissive, violates strict dest contract
    let cofre = src.board(&recs, b"rk").expect("boards under permissive contract");

    // The dest's contract_fp differs from the cofre's, so the *fingerprint* check fires first (still AC-9:
    // a managed schema event, dead-lettered, never committed).
    let disp = dst.offload(&cofre).expect("offload");
    assert_eq!(disp, Disposition::DeadLettered);
    assert!(dst.sink().is_empty(), "nothing committed");
    assert_eq!(dst.dead_letters().len(), 1);
    assert_eq!(
        dst.dead_letters().entries()[0].reason,
        DeadLetterReason::ContractFingerprintMismatch
    );
}

#[test]
fn ac9_offload_dead_letters_post_decrypt_record_violation() {
    // Isolate step (5) — the *record* re-validation — from the (4) fingerprint check. Honestly-configured
    // terminals with the same rules share a fingerprint, so a record that boards also passes at the dest. To
    // exercise the post-decrypt record check directly, the dest pins a STRICTER rule (PREFIX b"OK:STRICT:")
    // but advertises the *source's* fingerprint, so the fingerprint gate passes and the record-validation
    // gate is the one that fires.
    let mut src = source(); // boards b"OK:..." records
    let mut dst_strict = ContentContract::new(MAX_LEN, b"OK:STRICT:".to_vec());
    dst_strict.fingerprint = contract().fingerprint; // align fp so step (4) passes, step (5) is reached
    let mut dst =
        DestTerminal::new(config(), dst_strict, verifying_key(&SOURCE_SEED), DEST_SEED, DEST_X_SECRET);

    let recs: [&[u8]; 1] = [b"OK:loose"]; // valid at source, fails the dest's stricter prefix
    let cofre = src.board(&recs, b"rk").expect("board");
    let disp = dst.offload(&cofre).expect("offload");
    assert_eq!(disp, Disposition::DeadLettered);
    assert!(dst.sink().is_empty(), "contract-violating records are NOT committed");
    assert_eq!(
        dst.dead_letters().entries()[0].reason,
        DeadLetterReason::ContractViolation
    );
}

// ---- AC-9 (c): wrong contract_fp ⇒ dead-lettered ---------------------------------------------------------

#[test]
fn ac9_wrong_contract_fingerprint_dead_lettered() {
    // Source stamps one schema; dest pins a *different* schema version => managed drift, dead-lettered.
    let src_contract = ContentContract::new(MAX_LEN, PREFIX.to_vec());
    let dst_contract = ContentContract::new(MAX_LEN, b"V2:".to_vec()); // different fingerprint
    let mut src = SourceTerminal::new(config(), src_contract, SOURCE_SEED);
    let mut dst =
        DestTerminal::new(config(), dst_contract, verifying_key(&SOURCE_SEED), DEST_SEED, DEST_X_SECRET);

    let recs: [&[u8]; 1] = [b"OK:payload"];
    let cofre = src.board(&recs, b"rk").expect("board");
    assert_eq!(dst.offload(&cofre).expect("offload"), Disposition::DeadLettered);
    assert!(dst.sink().is_empty());
    assert_eq!(
        dst.dead_letters().entries()[0].reason,
        DeadLetterReason::ContractFingerprintMismatch
    );
}

// ---- AC-9 (d): tampered / forged cofre ⇒ dead-lettered --------------------------------------------------

#[test]
fn ac9_tampered_carga_dead_lettered() {
    let mut src = source();
    let mut dst = dest();
    let recs: [&[u8]; 1] = [b"OK:payload"];
    let mut cofre = src.board(&recs, b"rk").expect("board");
    // Flip a ciphertext byte: the lacre covers etiqueta ⊗ carga, so verify() fails (cofre_id mismatch).
    cofre.carga[0] ^= 0x01;
    let disp = dst.offload(&cofre).expect("offload");
    assert_eq!(disp, Disposition::DeadLettered);
    assert!(dst.sink().is_empty());
    assert!(matches!(
        dst.dead_letters().entries()[0].reason,
        DeadLetterReason::SealFailed(_)
    ));
}

#[test]
fn ac9_forged_cofre_wrong_signer_dead_lettered() {
    // A cofre sealed by an impostor key, verified against the pinned source key, is rejected (BLK-4, AC-3).
    let mut evil = SourceTerminal::new(config(), contract(), EVIL_SEED);
    let mut dst = dest(); // pins verifying_key(SOURCE_SEED)
    let recs: [&[u8]; 1] = [b"OK:payload"];
    let cofre = evil.board(&recs, b"rk").expect("board");
    let disp = dst.offload(&cofre).expect("offload");
    assert_eq!(disp, Disposition::DeadLettered);
    assert!(dst.sink().is_empty());
    assert_eq!(
        dst.dead_letters().entries()[0].reason,
        DeadLetterReason::SealFailed(CofreError::SignerMismatch)
    );
}

#[test]
fn ac9_tampered_etiqueta_seq_dead_lettered() {
    // Mutating an authenticated header field breaks the lacre (INV-SEAL-COMPLETE) => dead-letter.
    let mut src = source();
    let mut dst = dest();
    let recs: [&[u8]; 1] = [b"OK:payload"];
    let mut cofre = src.board(&recs, b"rk").expect("board");
    cofre.etiqueta.seq ^= 0x01;
    let disp = dst.offload(&cofre).expect("offload");
    assert_eq!(disp, Disposition::DeadLettered);
    assert!(dst.sink().is_empty());
    assert!(matches!(
        dst.dead_letters().entries()[0].reason,
        DeadLetterReason::SealFailed(_)
    ));
}

// ---- Exactly-once mini-proof: the SAME cofre offloaded twice ---------------------------------------------

#[test]
fn exactly_once_same_cofre_twice() {
    let mut src = source();
    let mut dst = dest();
    let recs: [&[u8]; 2] = [b"OK:one", b"OK:two"];
    let cofre = src.board(&recs, b"record-key-A").expect("board");

    // First offload: Delivered, records committed once.
    assert_eq!(dst.offload(&cofre).expect("offload 1"), Disposition::Delivered);
    assert_eq!(dst.sink().len(), 2);
    assert_eq!(dst.sink().committed(), &[b"OK:one".to_vec(), b"OK:two".to_vec()]);

    // Second offload of the identical cofre: Duplicate, sink UNCHANGED (no re-commit).
    assert_eq!(dst.offload(&cofre).expect("offload 2"), Disposition::Duplicate);
    assert_eq!(dst.sink().len(), 2, "duplicate must not re-commit");
    assert_eq!(dst.sink().committed(), &[b"OK:one".to_vec(), b"OK:two".to_vec()]);
    assert!(dst.dead_letters().is_empty());
}

#[test]
fn distinct_record_keys_are_both_delivered() {
    // Two genuinely distinct cofres (different record-key => different idempotency_key, advancing seq) both
    // commit — exactly-once is per-record, not "deliver only one ever".
    let mut src = source();
    let mut dst = dest();
    let c0 = src.board(&[b"OK:zero"], b"rk-0").expect("board 0");
    let c1 = src.board(&[b"OK:one"], b"rk-1").expect("board 1");
    assert_eq!(dst.offload(&c0).expect("off 0"), Disposition::Delivered);
    assert_eq!(dst.offload(&c1).expect("off 1"), Disposition::Delivered);
    assert_eq!(dst.sink().len(), 2);
}
