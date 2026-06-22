//! `datarail-rail` — the ephemeral, substrate-polymorphic transport (SPEC `07-rail-substrate.md`): it moves
//! cofres using only the authenticated header + seal (`INV-OPAQUE-CARGO`), never the payload.
//!
//! This crate provides the first concrete [`Substrate`] — an in-process [`LoopbackSubstrate`] — plus the
//! reusable [`substrate_conformance`] harness (AC-6) that every later substrate (shmem / QUIC / object-store)
//! must also pass. The loopback is the reference oracle: a `VecDeque<Cofre>` queue whose `send` enqueues,
//! `recv` dequeues in FIFO order, and `ack` records the acknowledged `cofre_id`s — routing and acking from
//! the [`Etiqueta`](datarail_core::Etiqueta) alone, **never** reading [`Cofre::carga`](datarail_core::Cofre).

#![forbid(unsafe_code)]

use std::collections::{HashSet, VecDeque};

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

/// A substrate that models a **resumable** link across a network partition (AC-8).
///
/// Every cofre handed to [`send`](Substrate::send) is retained in a source-side outbox until it is acked;
/// [`recv`](Substrate::recv) delivers forward from a cursor. [`partition`](Self::partition) severs the link
/// (delivery yields `None`); [`resume`](Self::resume) reconnects and rewinds the cursor to the **first
/// un-acked** cofre, so the source re-drives the stream from the last durable position. Redeliveries are
/// deduped downstream by the `datarail-once` gate, so the net effect across a partition is **0-loss /
/// 0-duplicate** at the sink (proven end-to-end in `datarail-acceptance`'s AC-8 test).
///
/// Like [`LoopbackSubstrate`] it holds no keys and reads only the header (`INV-DUMB-PIPE` / `INV-OPAQUE-CARGO`)
/// and cannot fail (its `Error` is the uninhabited [`LoopbackError`]).
#[derive(Debug, Default, Clone)]
pub struct ResumableSubstrate {
    /// Every cofre sent, retained until acked (the resend buffer).
    outbox: Vec<Cofre>,
    /// `cofre_id`s the destination has acknowledged.
    acked: HashSet<[u8; 32]>,
    /// Index of the next cofre `recv` will deliver.
    cursor: usize,
    /// While `true`, `recv` delivers nothing (the link is severed).
    partitioned: bool,
}

impl ResumableSubstrate {
    /// A fresh, connected, empty resumable substrate.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sever the link: until [`resume`](Self::resume), `recv` yields `None` (in-flight cofres are "lost").
    pub fn partition(&mut self) {
        self.partitioned = true;
    }

    /// Reconnect and resume from the last durable (acked) position: rewind the delivery cursor to the first
    /// un-acked cofre, so every sent-but-unacked cofre is re-driven.
    pub fn resume(&mut self) {
        self.partitioned = false;
        self.cursor = self
            .outbox
            .iter()
            .position(|c| !self.acked.contains(&c.etiqueta.cofre_id))
            .unwrap_or(self.outbox.len());
    }

    /// Number of cofres sent but not yet acked.
    #[must_use]
    pub fn inflight(&self) -> usize {
        self.outbox.len() - self.acked.len().min(self.outbox.len())
    }
}

impl Substrate for ResumableSubstrate {
    type Error = LoopbackError;

    /// Retain a clone of the cofre in the resend buffer.
    ///
    /// # Errors
    /// Never returns an error — [`LoopbackError`] is uninhabited.
    fn send(&mut self, cofre: &Cofre) -> Result<(), Self::Error> {
        self.outbox.push(cofre.clone());
        Ok(())
    }

    /// Deliver the next cofre from the cursor, or `None` while partitioned / drained.
    ///
    /// # Errors
    /// Never returns an error — see [`LoopbackError`].
    fn recv(&mut self) -> Result<Option<Cofre>, Self::Error> {
        if self.partitioned {
            return Ok(None);
        }
        if let Some(cofre) = self.outbox.get(self.cursor) {
            self.cursor += 1;
            Ok(Some(cofre.clone()))
        } else {
            Ok(None)
        }
    }

    /// Record an acknowledgement (header-only); acked cofres are not re-driven on [`resume`](Self::resume).
    ///
    /// # Errors
    /// Never returns an error — see [`LoopbackError`].
    fn ack(&mut self, cofre_id: [u8; 32]) -> Result<(), Self::Error> {
        self.acked.insert(cofre_id);
        Ok(())
    }
}

/// The shared core behind every **real byte-stream** [`Substrate`] (cross-process / cross-host): a duplex
/// transport `S` (anything `Read + Write` — a kernel socket, a TCP connection, later a QUIC stream).
///
/// Cofres are serialized with the canonical wire codec ([`encode`](datarail_cofre::encode) /
/// [`decode`](datarail_cofre::decode)), `u32`-length-prefixed, and moved over a transport that sees **only
/// opaque bytes** (`INV-OPAQUE-CARGO`) and holds **no keys** (`INV-DUMB-PIPE`). The very same
/// [`substrate_conformance`] harness passes over an in-memory queue, a Unix pipe, and a TCP connection
/// unchanged — `INV-SUBSTRATE-POLYMORPHIC` demonstrated identically across all of them.
///
/// `send` writes a framed cofre to the write half (`tx`); `recv` drains the read half (`rx`) into a buffer
/// and decodes one complete frame (returning `None` until a full frame has arrived). The read half is made
/// *non-immediately-blocking* by the concrete constructor — non-blocking for the separate-fd pair cases
/// ([`SocketSubstrate::pair`] / [`TcpSubstrate::loopback_pair`]) or a short read-timeout for the shared-fd
/// duplex cases ([`TcpSubstrate::connect`] / [`TcpSubstrate::accept`], where the write half must stay
/// blocking) — so `recv` returns promptly on an empty transport.
///
/// v1 note: a single connection with kernel-buffered sends — fine for bounded batches; a production substrate
/// would interleave send/recv or size the socket buffer to avoid back-pressure on huge bursts.
#[derive(Debug)]
pub struct StreamSubstrate<S> {
    tx: S,
    rx: S,
    buf: Vec<u8>,
    acked: Vec<[u8; 32]>,
}

impl<S> StreamSubstrate<S> {
    /// The `cofre_id`s acked so far, in ack order.
    #[must_use]
    pub fn acked(&self) -> &[[u8; 32]] {
        &self.acked
    }
}

impl<S: std::io::Read + std::io::Write> Substrate for StreamSubstrate<S> {
    type Error = std::io::Error;

    /// Serialize the cofre (wire codec) and write a `u32`-length-prefixed frame to the write half.
    ///
    /// # Errors
    /// [`std::io::Error`] on a write failure, or if the encoded cofre exceeds a `u32` frame length.
    fn send(&mut self, cofre: &Cofre) -> Result<(), Self::Error> {
        let bytes = datarail_cofre::encode(cofre);
        let len = u32::try_from(bytes.len()).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "cofre exceeds u32 frame")
        })?;
        self.tx.write_all(&len.to_le_bytes())?;
        self.tx.write_all(&bytes)?;
        Ok(())
    }

    /// Drain whatever is readable into the buffer, then decode one complete length-prefixed frame if present
    /// (`None` until a full frame has arrived). Routes from the bytes only — never inspects the plaintext.
    ///
    /// # Errors
    /// [`std::io::Error`] on a read failure, or `InvalidData` if a framed cofre fails to decode (corrupt pipe).
    fn recv(&mut self) -> Result<Option<Cofre>, Self::Error> {
        let mut tmp = [0u8; 8192];
        loop {
            match self.rx.read(&mut tmp) {
                Ok(0) => break,
                Ok(n) => self.buf.extend_from_slice(&tmp[..n]),
                // WouldBlock (non-blocking fd) and TimedOut (read-timeout fd) both mean "no more right now".
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    break
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        if self.buf.len() < 4 {
            return Ok(None);
        }
        let mut len_bytes = [0u8; 4];
        len_bytes.copy_from_slice(&self.buf[..4]);
        let len = u32::from_le_bytes(len_bytes) as usize;
        if self.buf.len() < 4 + len {
            return Ok(None); // frame not fully arrived yet
        }
        let frame = self.buf[4..4 + len].to_vec();
        self.buf.drain(..4 + len);
        let cofre = datarail_cofre::decode(&frame)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        Ok(Some(cofre))
    }

    /// Record an acknowledgement (header-only; the payload is never consulted).
    ///
    /// # Errors
    /// Never returns an error — recording is in-memory; the `Result` satisfies the [`Substrate`] contract.
    fn ack(&mut self, cofre_id: [u8; 32]) -> Result<(), Self::Error> {
        self.acked.push(cofre_id);
        Ok(())
    }
}

/// A **cross-process** [`Substrate`] over a connected Unix-domain-socket pair (same host — the first rung of
/// the cross-container ladder). A [`StreamSubstrate`] over [`UnixStream`](std::os::unix::net::UnixStream).
#[cfg(unix)]
pub type SocketSubstrate = StreamSubstrate<std::os::unix::net::UnixStream>;

#[cfg(unix)]
impl StreamSubstrate<std::os::unix::net::UnixStream> {
    /// Build a substrate over a fresh connected socket pair: bytes written by `send` (to `tx`) are read by
    /// `recv` (from `rx`). The read end is set non-blocking so `recv` can return `None` on an empty pipe.
    ///
    /// # Errors
    /// Returns the underlying [`std::io::Error`] if the socket pair cannot be created or set non-blocking.
    pub fn pair() -> std::io::Result<Self> {
        let (tx, rx) = std::os::unix::net::UnixStream::pair()?;
        rx.set_nonblocking(true)?;
        Ok(Self {
            tx,
            rx,
            buf: Vec::new(),
            acked: Vec::new(),
        })
    }
}

/// A **cross-host** [`Substrate`] over a TCP connection — the cross-cluster rung of the ladder, and the
/// literal **TCP baseline** the AC-8 WAN benchmark measures against. A [`StreamSubstrate`] over
/// [`TcpStream`](std::net::TcpStream).
///
/// The cofre is already sealed, so TCP is used purely as a dumb byte pipe — **no TLS is needed for
/// confidentiality** (`INV-OPAQUE-CARGO` already holds; this validates the DERP-style blind relay). Two
/// constructor shapes: [`loopback_pair`](Self::loopback_pair) holds both ends locally (conformance + same-host
/// hops); [`connect`](Self::connect) / [`accept`](Self::accept) hold a single duplex endpoint each, for a real
/// two-process / two-host transfer.
pub type TcpSubstrate = StreamSubstrate<std::net::TcpStream>;

impl StreamSubstrate<std::net::TcpStream> {
    /// A same-object loopback pair over `127.0.0.1` (both ends held locally): `send` writes the client end,
    /// `recv` reads the accepted server end. Used by the conformance harness and same-host hops; the two ends
    /// are *separate* sockets, so the read end can be set non-blocking without affecting writes.
    ///
    /// # Errors
    /// [`std::io::Error`] if binding, connecting, accepting, or socket configuration fails.
    pub fn loopback_pair() -> std::io::Result<Self> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let tx = std::net::TcpStream::connect(addr)?;
        tx.set_nodelay(true)?;
        let (rx, _peer) = listener.accept()?;
        rx.set_nonblocking(true)?;
        Ok(Self {
            tx,
            rx,
            buf: Vec::new(),
            acked: Vec::new(),
        })
    }

    /// Connect to a remote rail endpoint (the cross-host **source** side): one duplex connection where `send`
    /// writes and `recv` reads the same socket. The read half is given a short read-timeout (rather than
    /// non-blocking, which — sharing the file description with the write half — would make writes fail) so
    /// `recv` returns promptly when the peer has sent nothing, while `send` stays blocking.
    ///
    /// # Errors
    /// [`std::io::Error`] on connect / clone / socket-configuration failure.
    pub fn connect(addr: impl std::net::ToSocketAddrs) -> std::io::Result<Self> {
        let tx = std::net::TcpStream::connect(addr)?;
        tx.set_nodelay(true)?;
        let rx = tx.try_clone()?;
        rx.set_read_timeout(Some(std::time::Duration::from_millis(50)))?;
        Ok(Self {
            tx,
            rx,
            buf: Vec::new(),
            acked: Vec::new(),
        })
    }

    /// Accept one connection from `listener` (the cross-host **destination** side). Same single-duplex,
    /// read-timeout shape as [`connect`](Self::connect).
    ///
    /// # Errors
    /// [`std::io::Error`] on accept / clone / socket-configuration failure.
    pub fn accept(listener: &std::net::TcpListener) -> std::io::Result<Self> {
        let (tx, _peer) = listener.accept()?;
        tx.set_nodelay(true)?;
        let rx = tx.try_clone()?;
        rx.set_read_timeout(Some(std::time::Duration::from_millis(50)))?;
        Ok(Self {
            tx,
            rx,
            buf: Vec::new(),
            acked: Vec::new(),
        })
    }
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
    use super::{substrate_conformance, LoopbackSubstrate, ResumableSubstrate};
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
    fn ac6_resumable_passes_substrate_conformance() {
        // The resumable substrate (with no partition) satisfies the same polymorphic flow.
        substrate_conformance(ResumableSubstrate::new);
    }

    #[cfg(unix)]
    #[test]
    fn ac6_socket_passes_substrate_conformance() {
        // The SAME AC-6 flow, now over a REAL cross-process kernel transport (Unix domain socket) — the wire
        // codec round-trips through the kernel and the polymorphic harness is satisfied unchanged.
        substrate_conformance(|| super::SocketSubstrate::pair().expect("unix socket pair"));
    }

    #[test]
    fn ac6_tcp_passes_substrate_conformance() {
        // The SAME AC-6 flow over a REAL cross-host transport (TCP, loopback) — INV-SUBSTRATE-POLYMORPHIC now
        // holds over a network socket too, not only a kernel pipe. This substrate is the AC-8 TCP baseline.
        substrate_conformance(|| super::TcpSubstrate::loopback_pair().expect("tcp loopback pair"));
    }

    #[test]
    fn tcp_cross_endpoint_transfers_cofre_byte_for_byte() {
        // A genuine TWO-ENDPOINT transfer (separate connect + accept across threads, as a cross-host hop would
        // be): a cofre sealed at the source survives the TCP transport byte-for-byte at the destination.
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("local addr");
        let sent = super::testsupport::cofre_seq(42);
        let expected = sent.clone();

        let server = std::thread::spawn(move || {
            let mut dst = super::TcpSubstrate::accept(&listener).expect("accept");
            loop {
                if let Some(c) = dst.recv().expect("recv") {
                    dst.ack(c.etiqueta.cofre_id).expect("ack");
                    return c;
                }
            }
        });

        let mut src = super::TcpSubstrate::connect(addr).expect("connect");
        src.send(&sent).expect("send");
        let got = server.join().expect("server thread");
        assert_eq!(
            got, expected,
            "cofre survived a real cross-endpoint TCP transport byte-for-byte"
        );
    }

    #[test]
    fn resumable_partition_then_resume_redelivers_unacked() {
        let mut sub = ResumableSubstrate::new();
        let a = super::testsupport::cofre_seq(0);
        let b = super::testsupport::cofre_seq(1);
        let c = super::testsupport::cofre_seq(2);
        sub.send(&a).unwrap();
        sub.send(&b).unwrap();
        sub.send(&c).unwrap();

        // Deliver + ack `a`; deliver `b` but do NOT ack it (its ack is "lost").
        assert_eq!(sub.recv().unwrap().as_ref(), Some(&a));
        sub.ack(a.etiqueta.cofre_id).unwrap();
        assert_eq!(sub.recv().unwrap().as_ref(), Some(&b));
        assert_eq!(sub.inflight(), 2, "b and c are unacked");

        // Partition: nothing is delivered.
        sub.partition();
        assert_eq!(sub.recv().unwrap(), None);

        // Resume: re-drive from the first un-acked (`b`), then the never-delivered `c`, then drain.
        sub.resume();
        assert_eq!(sub.recv().unwrap().as_ref(), Some(&b), "unacked b is redelivered");
        assert_eq!(sub.recv().unwrap().as_ref(), Some(&c));
        assert_eq!(sub.recv().unwrap(), None);
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
