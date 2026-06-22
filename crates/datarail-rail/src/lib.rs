//! `datarail-rail` — the ephemeral, substrate-polymorphic transport (SPEC `07-rail-substrate.md`): it moves
//! cofres using only the authenticated header + seal (`INV-OPAQUE-CARGO`), never the payload.
//!
//! This crate provides the first concrete [`Substrate`] — an in-process [`LoopbackSubstrate`] — plus the
//! reusable [`substrate_conformance`] harness (AC-6) that every later substrate (shmem / QUIC / object-store)
//! must also pass. The loopback is the reference oracle: a `VecDeque<Cofre>` queue whose `send` enqueues,
//! `recv` dequeues in FIFO order, and `ack` records the acknowledged `cofre_id`s — routing and acking from
//! the [`Etiqueta`](datarail_core::Etiqueta) alone, **never** reading [`Cofre::carga`](datarail_core::Cofre).

#![forbid(unsafe_code)]

use std::collections::VecDeque;

use datarail_core::{Cofre, Substrate};

/// An in-process, single-queue [`Substrate`] — the reference loopback used as the conformance oracle.
///
/// `send` enqueues a clone of the cofre at the back of a `VecDeque`; `recv` pops the front (FIFO, preserving
/// per-stream order); `ack` appends the acked `cofre_id` to an audit log. It is a pure header-driven pipe:
/// it inspects only the routing fields it is given (`cofre_id` for acking) and **never** reads `carga`
/// (`INV-OPAQUE-CARGO`) — the metamorphic proof in this crate's tests pins that property.
///
/// It holds no keys and performs no crypto (`INV-DUMB-PIPE`); verification/sealing live in the terminal.
#[derive(Debug, Default, Clone)]
pub struct LoopbackSubstrate {
    queue: VecDeque<Cofre>,
    acked: Vec<[u8; 32]>,
}

impl LoopbackSubstrate {
    /// Create an empty loopback substrate.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of cofres currently queued (sent but not yet received).
    #[must_use]
    pub fn queued_len(&self) -> usize {
        self.queue.len()
    }

    /// The `cofre_id`s acked so far, in ack order. Used by tests to observe ack behavior.
    #[must_use]
    pub fn acked(&self) -> &[[u8; 32]] {
        &self.acked
    }
}

/// The loopback substrate cannot fail: it is a pure in-memory queue with no transport to break.
///
/// It is an inhabited-but-unconstructable error type (no public constructor) so the [`Substrate`] contract is
/// honoured without ever yielding an `Err`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopbackError {}

impl core::fmt::Display for LoopbackError {
    fn fmt(&self, _f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {}
    }
}

impl core::error::Error for LoopbackError {}

impl Substrate for LoopbackSubstrate {
    type Error = LoopbackError;

    /// Enqueue a clone of the sealed cofre at the back of the queue.
    ///
    /// Routes by header only: the bytes of `cofre.carga` are copied verbatim and never inspected
    /// (`INV-OPAQUE-CARGO`).
    ///
    /// # Errors
    /// Never returns an error — [`LoopbackError`] is uninhabited; the signature satisfies the [`Substrate`]
    /// contract.
    fn send(&mut self, cofre: &Cofre) -> Result<(), Self::Error> {
        self.queue.push_back(cofre.clone());
        Ok(())
    }

    /// Pop and return the next cofre in FIFO order, or `None` when the queue is empty.
    ///
    /// # Errors
    /// Never returns an error — see [`LoopbackError`].
    fn recv(&mut self) -> Result<Option<Cofre>, Self::Error> {
        Ok(self.queue.pop_front())
    }

    /// Record an acknowledgement for `cofre_id` (header-only; the payload is never consulted).
    ///
    /// # Errors
    /// Never returns an error — see [`LoopbackError`].
    fn ack(&mut self, cofre_id: [u8; 32]) -> Result<(), Self::Error> {
        self.acked.push(cofre_id);
        Ok(())
    }
}

/// AC-6 substrate-parity harness: the one acceptance flow every [`Substrate`] must pass.
///
/// Given a factory that builds a fresh substrate, this drives the canonical lifecycle — **send N → recv N in
/// FIFO order → ack each** — and asserts:
///
/// 1. every `recv` after a `send` yields a cofre, in send order, byte-for-byte equal to what was sent;
/// 2. the queue is drained (the `N+1`-th `recv` is `None`);
/// 3. each received cofre can be acked (by its `etiqueta.cofre_id`) without error.
///
/// [`LoopbackSubstrate`] passes it; later substrates (shmem / QUIC / S3) plug into the *same* function so the
/// suite is identical across substrates (`INV-SUBSTRATE-POLYMORPHIC`).
///
/// # Panics
/// Panics (as a test assertion) if any step deviates from the contract above, or if the substrate's `send` /
/// `recv` / `ack` returns an error.
pub fn substrate_conformance<S, E>(make: impl Fn() -> S)
where
    S: Substrate<Error = E>,
    E: core::fmt::Debug,
{
    const N: u64 = 8;

    let mut sub = make();
    let sent: Vec<Cofre> = (0..N).map(testsupport::cofre_seq).collect();

    for cofre in &sent {
        sub.send(cofre).expect("send must succeed");
    }

    for (i, expected) in sent.iter().enumerate() {
        let got = sub
            .recv()
            .expect("recv must succeed")
            .unwrap_or_else(|| panic!("recv #{i} returned None but a cofre was sent"));
        assert_eq!(&got, expected, "recv #{i} must return the sent cofre in order");
        sub.ack(got.etiqueta.cofre_id).expect("ack must succeed");
    }

    assert!(
        sub.recv().expect("recv must succeed").is_none(),
        "queue must be drained after N recvs"
    );
}

/// Shared cofre fixtures — `pub` so downstream substrate crates can reuse them in the same harness.
pub mod testsupport {
    use datarail_cofre::seal;
    use datarail_core::{AeadAlg, Cofre, Etiqueta};

    /// A deterministic signing seed for fixtures (test-only; not a real key).
    pub const FIXTURE_SEED: [u8; 32] = [7u8; 32];

    /// Build a fully-populated etiqueta with a given `seq` and all routing fields fixed.
    ///
    /// `cofre_id` / `signer_key_id` are placeholders overwritten by [`seal`].
    #[must_use]
    pub fn etiqueta_seq(seq: u64) -> Etiqueta {
        Etiqueta {
            route_id: [1; 16],
            stream_id: [2; 16],
            seq,
            cofre_id: [0; 32],
            idempotency_key: [4; 32],
            contract_fp: [5; 32],
            aead_alg: AeadAlg::Gcmsiv256,
            nonce: [6; 12],
            signer_key_id: [0; 32],
            eph_pk: [7; 32],
        }
    }

    /// A validly-sealed cofre for sequence `seq`, with a distinct payload per `seq`.
    #[must_use]
    pub fn cofre_seq(seq: u64) -> Cofre {
        let carga = format!("opaque-ciphertext-{seq:03}").into_bytes();
        seal(etiqueta_seq(seq), carga, &FIXTURE_SEED)
    }
}

#[cfg(test)]
mod tests {
    use super::{substrate_conformance, LoopbackSubstrate};
    use super::testsupport::{etiqueta_seq, FIXTURE_SEED};
    use datarail_cofre::{seal, verify};
    use datarail_core::{Cofre, Substrate};
    use datarail_crypto::verifying_key;

    #[test]
    fn ac6_loopback_passes_substrate_conformance() {
        // The reference substrate satisfies the polymorphic acceptance flow (AC-6).
        substrate_conformance(LoopbackSubstrate::new);
    }

    #[test]
    fn send_recv_is_fifo_and_ack_is_logged() {
        let mut sub = LoopbackSubstrate::new();
        let a = super::testsupport::cofre_seq(0);
        let b = super::testsupport::cofre_seq(1);
        sub.send(&a).unwrap();
        sub.send(&b).unwrap();
        assert_eq!(sub.queued_len(), 2);

        assert_eq!(sub.recv().unwrap().as_ref(), Some(&a));
        assert_eq!(sub.recv().unwrap().as_ref(), Some(&b));
        assert_eq!(sub.recv().unwrap(), None);

        sub.ack(a.etiqueta.cofre_id).unwrap();
        sub.ack(b.etiqueta.cofre_id).unwrap();
        assert_eq!(sub.acked(), &[a.etiqueta.cofre_id, b.etiqueta.cofre_id]);
    }

    /// Drive the full send→recv→ack lifecycle and distill the substrate's *decision structure* — the
    /// routing choices it makes — into a comparable trace.
    ///
    /// The substrate routes/acks from the [`Etiqueta`](datarail_core::Etiqueta) alone, so the opacity claim
    /// is that this *structure* is independent of the payload bytes. We deliberately do NOT record the
    /// absolute `cofre_id`/`lacre` here: those are payload-derived (`cofre_id = BLAKE3(carga)`,
    /// `lacre` signs over carga), so when two payloads legitimately differ they differ too — comparing them
    /// would conflate "carga was read" with "the cofre carries a different content-id". Instead we record:
    /// receive order/count, the routing fields the substrate may use, whether each recv'd cofre is echoed
    /// byte-for-byte (the pipe is faithful), whether each ack records exactly the handed-in id, and drain.
    fn observe(cofres: &[Cofre]) -> Trace {
        let mut sub = LoopbackSubstrate::new();
        for c in cofres {
            sub.send(c).unwrap();
        }
        let mut steps = Vec::new();
        let mut idx = 0usize;
        while let Some(got) = sub.recv().unwrap() {
            sub.ack(got.etiqueta.cofre_id).unwrap();
            steps.push(RecvStep {
                position: idx,
                // The header routing fields the substrate is allowed to read…
                route_id: got.etiqueta.route_id,
                stream_id: got.etiqueta.stream_id,
                seq: got.etiqueta.seq,
                // …the pipe is faithful: what came out equals what went in, byte-for-byte.
                echoed_input_verbatim: cofres.get(idx) == Some(&got),
                // …and the ack records exactly the id it was handed (header-driven, not carga-driven).
                acked_handed_in_id: sub.acked().last() == Some(&got.etiqueta.cofre_id),
            });
            idx += 1;
        }
        Trace {
            steps,
            ack_count: sub.acked().len(),
            drained: sub.recv().unwrap().is_none(),
        }
    }

    /// One observed routing decision: position, the header fields the substrate may use, and two opacity
    /// invariants (faithful echo + handed-in-id ack) — all computable without reading `carga`.
    #[derive(Debug, PartialEq, Eq)]
    struct RecvStep {
        position: usize,
        route_id: [u8; 16],
        stream_id: [u8; 16],
        seq: u64,
        echoed_input_verbatim: bool,
        acked_handed_in_id: bool,
    }

    /// The substrate's observable decision structure over a flow. Independent of `carga` by the opacity
    /// claim — that independence is exactly what the AC-1 metamorphic test asserts.
    #[derive(Debug, PartialEq, Eq)]
    struct Trace {
        steps: Vec<RecvStep>,
        ack_count: usize,
        drained: bool,
    }

    #[test]
    fn ac1_metamorphic_opacity_payload_swap_is_unobservable() {
        // AC-1 (metamorphic opacity): take a sealed cofre; build a SECOND one identical except its `carga`
        // is replaced by ANOTHER validly-sealed ciphertext of EQUAL LENGTH (re-sealed via datarail-cofre).
        // The substrate routes/acks on the header alone, so its observable send/recv/ack behavior must be
        // IDENTICAL for the two flows — i.e. independent of the payload bytes (INV-OPAQUE-CARGO).
        let etq = etiqueta_seq(99);

        let carga_x = b"AAAAAAAAAAAAAAAA".to_vec();
        let carga_y = b"ZQ7k-3p!9xLm_w2#".to_vec(); // different bytes…
        assert_eq!(carga_x.len(), carga_y.len(), "metamorphic precondition: equal length");

        // Two validly-sealed cofres with the SAME routing/header inputs, differing only in payload bytes.
        let cofre_x = seal(etq.clone(), carga_x.clone(), &FIXTURE_SEED);
        let cofre_y = seal(etq, carga_y.clone(), &FIXTURE_SEED);

        // Sanity: both are genuinely valid cofres (the swap is a re-seal, not a forgery)…
        let vk = verifying_key(&FIXTURE_SEED);
        verify(&cofre_x, &vk).expect("cofre_x is validly sealed");
        verify(&cofre_y, &vk).expect("cofre_y is validly sealed");
        // …and they really do differ only in the payload-derived parts (carga + its BLAKE3 id + the seal).
        assert_ne!(cofre_x.carga, cofre_y.carga);
        assert_eq!(cofre_x.carga.len(), cofre_y.carga.len());

        // The metamorphic relation: identical observable substrate behavior for the two flows.
        let trace_x = observe(std::slice::from_ref(&cofre_x));
        let trace_y = observe(std::slice::from_ref(&cofre_y));
        assert_eq!(
            trace_x, trace_y,
            "substrate behavior must be independent of the opaque payload bytes (INV-OPAQUE-CARGO)"
        );

        // And a stronger control: swapping carga UNDER A FIXED header (forcing cofre_id/lacre identical to
        // cofre_x) leaves the substrate's behavior bit-identical, proving carga is never read.
        let cofre_y_fixed_header = Cofre {
            etiqueta: cofre_x.etiqueta.clone(),
            carga: carga_y,
            lacre: cofre_x.lacre,
        };
        assert_eq!(
            observe(std::slice::from_ref(&cofre_x)),
            observe(std::slice::from_ref(&cofre_y_fixed_header)),
            "with header fields held fixed, payload bytes are invisible to the substrate"
        );
    }
}
