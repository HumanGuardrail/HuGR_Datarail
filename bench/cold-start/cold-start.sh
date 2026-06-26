#!/usr/bin/env bash
# Reproduces COLD-START-RESULTS.md: time-to-ready for datarail vs Kafka.
#  - datarail: spawn the prebuilt shim binary → measure until ingress :7701 accepts (bare-process cold start).
#  - Kafka (if docker present): `docker run` apache/kafka → measure until the broker answers `kafka-topics --list`
#    (measured INSIDE the container, so it works regardless of host↔VM port mapping; excludes the one-time image pull).
# Honest scope is in COLD-START-RESULTS.md (bare-binary vs JVM-container; not byte-identical readiness bars).
#
# Usage: bench/cold-start/cold-start.sh [datarail-trials] [kafka-trials]
set -euo pipefail
DR_TRIALS="${1:-10}"
KF_TRIALS="${2:-3}"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

echo "building datarail shim (release)…"
cargo build --release -p datarail-omb-shim --bin datarail-omb-shim
BIN="target/release/datarail-omb-shim"
[ -x "$BIN" ] || { echo "::error::shim binary not built at $BIN"; exit 2; }

echo ""
echo "## datarail shim cold-start → ingress :7701 ready (n=$DR_TRIALS)"
python3 - "$BIN" "$DR_TRIALS" <<'PY'
import subprocess, socket, time, statistics as st, sys, signal
BIN, trials = sys.argv[1], int(sys.argv[2])
def ready():
    s=socket.socket(); s.settimeout(0.05)
    try: s.connect(("127.0.0.1",7701)); s.close(); return True
    except OSError:
        try: s.close()
        except OSError: pass
        return False
def one():
    while ready(): time.sleep(0.05)
    t0=time.perf_counter()
    p=subprocess.Popen([BIN], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    while not ready() and time.perf_counter()-t0 < 10: time.sleep(0.0005)
    dt=(time.perf_counter()-t0)*1000
    p.send_signal(signal.SIGKILL); p.wait(); time.sleep(0.15)
    return dt
for _ in range(2): one()  # discard warmup (first-spawn page-in)
times=sorted(one() for _ in range(trials))
def pct(v,q): return v[min(len(v)-1, int(q*len(v)))]
print(f"datarail: p50={st.median(times):.0f} ms  p10={pct(times,0.10):.0f}  p90={pct(times,0.90):.0f}  min={min(times):.0f}  max={max(times):.0f}  (n={trials}, warmup-discarded)")
PY

if command -v docker >/dev/null 2>&1 && docker version >/dev/null 2>&1; then
  echo ""
  echo "## Kafka 3.8 (KRaft) cold-start → broker answers topic-list (n=$KF_TRIALS, image pre-pulled)"
  docker pull -q apache/kafka:3.8.0 >/dev/null 2>&1 || true
  python3 - "$KF_TRIALS" <<'PY'
import subprocess, time, statistics as st, sys
trials=int(sys.argv[1])
def ready():
    r=subprocess.run(["docker","exec","cs-kafka","/opt/kafka/bin/kafka-topics.sh",
                      "--bootstrap-server","localhost:9092","--list"],capture_output=True,timeout=15)
    return r.returncode==0
ts=[]
for _ in range(trials):
    subprocess.run(["docker","rm","-f","cs-kafka"],capture_output=True)
    t0=time.perf_counter()
    subprocess.run(["docker","run","-d","--name","cs-kafka","-e","KAFKA_HEAP_OPTS=-Xmx2g -Xms512m",
                    "apache/kafka:3.8.0"],capture_output=True)
    ok=False
    while time.perf_counter()-t0 < 180:
        try:
            if ready(): ok=True; break
        except Exception: pass
        time.sleep(0.25)
    if ok: ts.append(time.perf_counter()-t0)
    subprocess.run(["docker","rm","-f","cs-kafka"],capture_output=True); time.sleep(1)
if ts: print(f"kafka: p50={st.median(ts):.1f} s  min={min(ts):.1f}  max={max(ts):.1f}  (n={len(ts)})")
else:  print("kafka: did not come up within 180s")
PY
else
  echo ""
  echo "(docker not available — skipping the Kafka side; see COLD-START-RESULTS.md for the recorded 5.5s)"
fi
echo ""
echo "Reading: datarail ms vs Kafka seconds ⇒ scale-to-zero viable vs always-on mandatory (COLD-START-RESULTS.md)."
