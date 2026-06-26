# FASP-UDP-TRANSPORT — wire the DelayController to a real lossy network (design, pre-code)

> **Status:** DESIGN (pre-code) · **Owner:** TechLead · **Confidence of the claim it unlocks:** DIRECTIONAL→PROVEN
>
> **Why this exists.** A read-only exploration of the WAN code found an honest gap that a hostile reviewer would
> land: the FASP delay-based congestion controller (`congestion::DelayController`, `datarail-rail`) is real, pure
> logic — but the "16–42× the goodput of loss-based TCP" number comes from an **in-process seeded-loss Monte-Carlo
> simulation** (`tests/fasp_vs_lossbased.rs`, a `SplitMix64` PRNG dropping packets), and the controller is **not
> wired to any real transport**. datarail's real socket path today is `TcpSubstrate` (kernel TCP) → on a real lossy
> link it would inherit **kernel TCP's loss collapse**, getting *none* of the FASP benefit. So the WAN moat is
> "algorithm-proven-in-sim, unwired-to-reality." This doc designs the build that closes it — **before** any code,
> because reliable transport is exactly the kind of subtle work that gave the WAL 5 data-loss bugs.

## The goal (one sentence)
A minimal **reliable-UDP mover paced by the `DelayController`**, benchmarked under **real `tc netem` packet loss**
against a **real kernel-TCP** bulk transfer over the *same* impaired link — turning the 16–42× from a simulation
into a real-socket, real-kernel-loss measurement (or honestly reporting whatever real number it yields).

## Contract (freeze before code)

### New component: `FaspLink` (crate: `datarail-rail`, behind no new external dep — `std::net::UdpSocket` only)
```
FaspLink::connect(local: SocketAddr, peer: SocketAddr, cfg: FaspCfg) -> Result<FaspLink, FaspError>
FaspLink::send(&mut self, payload: &[u8]) -> Result<(), FaspError>   // reliable, ordered, paced by window
FaspLink::recv(&mut self, buf: &mut Vec<u8>) -> Result<usize, FaspError>
FaspLink::stats(&self) -> FaspStats   // delivered, retransmits, rtt_base, window, losses
```
- **Wire frame** (UDP datagram, little-endian, CRC-tailed like the WAL): `[seq:u64][kind:u8][len:u32][bytes][crc:u32]`
  where `kind ∈ {DATA, ACK}`. An `ACK` echoes `[ack_seq:u64][echo_send_micros:u64]` for an RTT sample.
- **Pacing:** the sender may have at most `floor(DelayController.window())` un-acked frames in flight. After each
  ACK → `on_rtt_sample(now − echo_send_micros)`; on a retransmit timeout → `on_loss()` (which, per FASP physics,
  does **NOT** cut the window — only RTT inflation does).
- **Reliability:** sliding send window with per-frame retransmit timeout (`base_rtt × k`, k≈2, floored); receiver
  buffers out-of-order frames by `seq` and delivers in order; duplicate `seq` dropped (effectively-once preserved).

### Invariants (INV — verified by gates, not asserted)
- **INV-FASP-RELIABLE** — every byte delivered exactly once, in order, even at 30% loss (the whole point).
- **INV-FASP-SIGNAL** — loss does **not** shrink the window; only measured queue/RTT growth does (FASP physics).
  A gate asserts the window stays ≈BDP under pure loss and backs off under injected RTT inflation.
- **INV-FASP-BOUNDED** — memory is O(window), never O(total bytes moved). Send/recv buffers are window-bounded.
- **Charter** — `#![forbid(unsafe_code)]`, clippy `deny(all+pedantic)` no `#[allow]`, no panic/unwrap/expect in
  non-test, zero new external deps (std `UdpSocket` + existing CRC/`Cofre`).

## The benchmark (the honest number)
`bench/wan/netem-fasp-vs-tcp.sh` (Linux/CI only — needs `NET_ADMIN`):
1. `sudo tc qdisc add dev lo root netem loss <L>% delay 50ms` — REAL kernel loss+latency on loopback.
2. Move a fixed payload (e.g. 256 MiB) **A→B over `FaspLink`** (UDP, our controller); record goodput + retransmits.
3. Move the *same* payload **A→B over kernel TCP** (a plain `TcpStream` bulk copy, or `iperf3`); record goodput.
4. `sudo tc qdisc del dev lo root`. Sweep `L ∈ {0,5,15,30}`. Report `fasp_goodput / tcp_goodput` per loss rate.
- **Honesty pre-commitments:** (a) if `tc`/NET_ADMIN is unavailable on the runner, the bench SKIPS loudly (no
  silent green); (b) the real ratio is reported *as measured* even if it's far below the sim's 16–42× (real UDP
  has syscall + ACK overhead the sim ignores); (c) FaspLink gets its **own adversarial audit** (a `FaspLink` round
  of the skeptic loop) before any PROVEN label — reliable transport is bug-prone (cf. the WAL's 5 loss windows).

## Slicing (build order — each a verifiable increment)
1. **S1 — frame + loopback echo:** `FaspLink` over `UdpSocket` on `127.0.0.1`, no loss, prove reliable ordered
   delivery of N frames + RTT samples feeding the controller. Gate: INV-FASP-RELIABLE on a clean socket.
2. **S2 — retransmit + reorder under in-process drop:** inject drops at the socket-send call (test-only) → prove
   recovery + INV-FASP-SIGNAL (window holds under loss, backs off under added RTT). Gate: 0-loss at 30% drop.
3. **S3 — real netem bench:** the `tc netem` harness + the kernel-TCP baseline; the real ratio sweep.
4. **S4 — adversarial audit of `FaspLink`** ✅ DONE (`AUDIT-FASP.md`): 3 skeptics found 2 CRITICAL (unbounded
   reorder buffer → OOM; `send` infinite-block) + 1 HIGH (off-path spoof) + 1 LOW — **all fixed at root with
   regression gates** (`f1`/`f3`/`f4` in `reliable_udp::s4_gates`). Exactly-once core independently verified.
   Claim stays DIRECTIONAL (loopback netem, n=2, modest throughput) — PROVEN still needs a real-WAN field number.

## What this deliberately is NOT (scope honesty)
- Not a full QUIC/production transport (no path MTU discovery, no multipath, no encryption *here* — the cofre is
  already sealed end-to-end above this layer; FaspLink moves opaque bytes).
- Not multi-stream/connection-migration (that's QuicSubstrate's lane).
- Not a claim that we beat tuned BBR — the comparison is honestly vs **kernel TCP (loss-based)**, which is what
  Kafka MirrorMaker actually uses, so it's the fair incumbent baseline.
