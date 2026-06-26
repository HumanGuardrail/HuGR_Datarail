#!/usr/bin/env bash
# S3 (FASP-UDP-TRANSPORT.md): the REAL tc-netem goodput bench. Inject genuine kernel packet loss + delay on the
# loopback interface, then move the same payload over FaspLink (UDP, our delay-based controller) and over kernel
# TCP — a fair same-link comparison. Sweep the loss rate and print the goodput ratio. Linux + NET_ADMIN only.
#
# Usage: bench/wan/netem-fasp-vs-tcp.sh [seconds-per-transfer] [one-way-delay-ms]
set -euo pipefail

SECS="${1:-5}"        # fixed-DURATION transfer (iperf-style): bounds runtime even when a link collapses
DELAY_MS="${2:-25}"   # one-way on lo → ~2× RTT (request+response both traverse lo)

if ! command -v tc >/dev/null 2>&1; then
  echo "::error::tc (iproute2) not found — cannot inject netem loss. SKIPPING (no silent green)."
  exit 2
fi

echo "building bench (release)…"
cargo build --release --example fasp_wan_bench -p datarail-rail
BIN="target/release/examples/fasp_wan_bench"
[ -x "$BIN" ] || { echo "::error::bench binary not built at $BIN"; exit 2; }

cleanup() { sudo tc qdisc del dev lo root 2>/dev/null || true; }
trap cleanup EXIT
cleanup  # start from a clean qdisc

echo ""
echo "## FASP (real UDP, delay-based CC) vs kernel TCP under real tc-netem loss — ${SECS}s/transfer, ${DELAY_MS}ms one-way"
echo ""
echo "| loss | fasp MB/s | tcp MB/s | ratio (fasp/tcp) |"
echo "|---|---|---|---|"
for L in 0 5 15 30; do
  cleanup
  sudo tc qdisc add dev lo root netem loss "${L}%" delay "${DELAY_MS}ms"
  out="$(timeout 90 "$BIN" "$SECS" || echo "fasp_mbps=NaN tcp_mbps=NaN ratio=NaN")"
  cleanup
  f="$(printf '%s' "$out" | sed -E 's/.*fasp_mbps=([0-9.]+).*/\1/')"
  t="$(printf '%s' "$out" | sed -E 's/.*tcp_mbps=([0-9.]+).*/\1/')"
  r="$(printf '%s' "$out" | sed -E 's/.*ratio=([0-9.]+).*/\1/')"
  echo "| ${L}% | ${f} | ${t} | ${r} |"
done
echo ""
echo "Honest reading: at 0% loss kernel TCP wins (our user-space per-datagram UDP mover is slower on a clean"
echo "link); the FASP thesis is that as loss climbs, TCP's goodput collapses while the delay-based window holds,"
echo "so the ratio should climb with loss. The numbers above are reported exactly as measured."
