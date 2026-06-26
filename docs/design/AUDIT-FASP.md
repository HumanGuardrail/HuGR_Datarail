# AUDIT-FASP — adversarial audit of the FaspLink reliable-UDP transport (S4)

> **Method.** Three independent skeptics, each told to assume the code is buggy and PROVE it, cold-read
> `crates/datarail-rail/src/reliable_udp.rs` (lenses: exactly-once correctness · unbounded-memory/DoS ·
> framing/integer/parse-panic). Every finding was lead-cold-verified against the source before fixing. Reliable
> transport is exactly where subtle bugs hide — the S2 gate had already caught a real Karn's-algorithm bug, and
> the WAL audit found 5 data-loss windows; this round found more. **All CRITICAL/HIGH findings fixed at the root
> with regression gates before any PROVEN label.**

## Findings & fixes

| # | Lens | Severity | Finding (lead-verified) | Root fix | Gate |
|---|---|---|---|---|---|
| **F1** | memory/DoS | **CRITICAL** | **Unbounded reorder buffer.** `on_datagram` stored any `seq >= next_deliver` with no cap; a peer/off-path spoofer sending huge-gap seqs pins them forever → OOM. `INV-FASP-BOUNDED` was enforced on the *send* side only. (Found independently by 2 of 3 auditors.) | Receive-window flow control: drop (and do **not** ACK) any frame outside `[base, base+recv_window)` where `base` = lowest un-consumed seq; the sender retransmits when the window slides. Bounds reorder + ready to O(`recv_window`). | `f1_reorder_buffer_is_bounded_by_recv_window_under_hostile_gap_seqs` |
| **F3** | memory/liveness | **CRITICAL** | **`send()` blocks forever** if the peer stops ACKing — `while !can_send() { pump; sleep }` had no deadline. The doc comment "never sleeps indefinitely" was false; this is the same liveness class that hung a >10-min CI run. | `send()` honors `cfg.send_timeout`; returns `TimedOut` instead of wedging the caller. Doc corrected. | `f3_send_times_out_instead_of_blocking_forever_when_peer_never_acks` |
| **F4** | exactly-once/spoof | **HIGH** | **Forged datagram from any source.** Socket wasn't `connect`-ed and `on_datagram` didn't check `from`; CRC is an error-detector, not a MAC. An off-path spoofer could forge an ACK (retire an undelivered frame → silent loss) or inject DATA. | Drop datagrams whose `from != peer` once the peer is set — defeats blind off-path spoofing cheaply. On-path integrity remains the sealed cofre layer's job (stated honestly). | `f4_datagram_from_a_non_peer_source_is_dropped` |
| **F1b** | memory/DoS | HIGH | **Unbounded `ready` queue** if the app never `recv()`s. | Same receive-window cap (F1) bounds `ready` too (the window is anchored at the lowest *un-consumed* seq). | covered by F1 gate |
| **F8** | framing | LOW | `decode_frame` didn't enforce `MAX_FASP_PAYLOAD` on receive — a peer could push ~2 KB payloads past the 1200-byte contract. | Reject `len > MAX_FASP_PAYLOAD` in `decode_frame`. | — |

## What the audit confirmed is SOUND (no bug — verified, not assumed)
- **Exactly-once state machine:** dedup (`seq >= next_deliver && !reorder.contains_key`), contiguous drain, and
  ACK-retire are correct; each seq is buffered at most once and delivered exactly once; post-delivery retransmits
  are dropped (and re-ACKed so the sender stops). No honest-network loss or duplication path.
- **`decode_frame` is panic-safe** — proven index-by-index against crafted short/garbage datagrams; every slice
  is bounded by the `len >= HEADER_LEN+CRC_LEN` gate; `body_len - HEADER_LEN` cannot underflow. A jumbo frame >
  the 2048 buffer is truncated → CRC-fails → safe drop.
- **CRC-32** is correct reflected IEEE (poly `0xEDB88320`), covering seq‖kind‖len‖payload.
- **`inflight`** is genuinely bounded by `max_inflight` via `can_send()` (the one place the invariant already held).
- **Karn's algorithm** (S2) is correctly applied — a retransmitted frame's ACK never samples RTT.

## Honest residue (NOT fixed here — noted, not hidden)
- **No MAC on the wire** — F4's source-check stops *off-path blind* spoofing, not an *on-path* attacker. Full
  integrity needs a per-link MAC (the session key from the sealed layer). Out of scope for this transport slice;
  the cofre's AEAD above this layer already authenticates the payload. Documented, not silently assumed away.
- **No exponential RTO backoff** (Karn's second half) — `rto` is fixed; a perf/congestion nicety, not a
  correctness bug. Future.
- **`frame.clone()` per retransmit** — a borrow-checker workaround; bounded by `max_inflight`, so churn not
  amplification. Future cleanup.

## Status after S4
The transport is now **memory-bounded on both sides, live (no infinite block), and off-path-spoof-resistant**,
with the exactly-once core independently verified. The WAN *claim* stays **DIRECTIONAL** — the S4 audit removes
the "unaudited" caveat but the remaining ones stand: modest absolute throughput, n=2, loopback-`netem`-not-WAN.
No PROVEN label until a real-WAN field number with firmer n.
