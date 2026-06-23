#!/usr/bin/env bash
# Run the OMB workload against ONE system, single-node, then tear it down.
# Usage: ./run-omb.sh <kafka|rabbitmq|pulsar|datarail> <workload-file> <results-dir>
# CWD must be the OMB distribution dir (has bin/benchmark, workloads/, the local *.yaml driver configs, the shim).
#
# Hardening (learned from a 17-min hang): every broker has a REAL readiness gate (TCP listener actually
# accepting), not just "node up"; the benchmark is wrapped in `timeout` so a stuck run cannot burn the budget;
# per-system stdout + the shim log are captured into the results dir; the per-system rc is reported, not
# silently swallowed.
set -uo pipefail

SYS="$1"; WL="$2"; RESULTS="$3"
SHIM_PID=""
RUN_TIMEOUT="${RUN_TIMEOUT:-900}"   # hard cap per system (seconds)
# Size the OMB client heap to the box: ~45% of total RAM (leaves room for the colocated broker + shim + OS).
# A small 7 GB runner gets ~3 GB; a 64 GB larger-runner gets ~28 GB — enough headroom for rate-discovery,
# which is what OOM'd the tiny runner. Overridable via HEAP_OPTS.
if [ -z "${HEAP_OPTS:-}" ]; then
  mem_kb=$(awk '/MemTotal/{print $2}' /proc/meminfo 2>/dev/null || echo 7000000)
  heap_g=$(( mem_kb * 45 / 100 / 1024 / 1024 )); [ "$heap_g" -lt 1 ] && heap_g=1
  export HEAP_OPTS="-Xms1G -Xmx${heap_g}G"
fi
echo "OMB client heap: $HEAP_OPTS"
mkdir -p "$RESULTS"
OUT="$RESULTS/$SYS.out"

cleanup() {
  [ -n "$SHIM_PID" ] && kill "$SHIM_PID" 2>/dev/null || true
  docker rm -f omb-kafka omb-rabbit omb-pulsar >/dev/null 2>&1 || true
}
trap cleanup EXIT

# Wait until a host TCP port actually accepts a connection (listener ready), or fail.
wait_port() {
  local host="$1" port="$2" tries="${3:-90}"
  for _ in $(seq 1 "$tries"); do
    if (exec 3<>"/dev/tcp/$host/$port") 2>/dev/null; then exec 3>&- 3<&-; return 0; fi
    sleep 2
  done
  return 1
}

case "$SYS" in
  kafka)
    if [ "${KAFKA_TLS:-0}" = "1" ]; then
      # Kafka WITH TLS (transport encryption). Self-signed cert via keytool (stock Kafka SSL config, no custom
      # crypto from us → low bias). Certs land on the HOST (/tmp/kcerts) so the host-side OMB client can read the
      # truststore. NOTE: TLS encrypts client↔broker in transit; the BROKER still sees plaintext (NOT
      # provider-blind like datarail). This measures the throughput cost of "encrypted Kafka" as commonly run.
      CERTS=/tmp/kcerts; rm -rf "$CERTS"; mkdir -p "$CERTS"
      docker run --rm -v "$CERTS":/certs apache/kafka:3.8.0 bash -c '
        keytool -genkeypair -alias broker -keyalg RSA -keysize 2048 -validity 3650 \
          -keystore /certs/server.keystore.jks -storepass changeit -keypass changeit \
          -dname "CN=localhost" -ext SAN=DNS:localhost,IP:127.0.0.1 &&
        keytool -exportcert -alias broker -keystore /certs/server.keystore.jks -storepass changeit -rfc -file /certs/broker.crt &&
        keytool -importcert -alias broker -keystore /certs/client.truststore.jks -storepass changeit -file /certs/broker.crt -noprompt &&
        chmod 644 /certs/*' || { echo "::error::keytool cert gen failed"; exit 1; }
      docker run -d --network host --name omb-kafka -v "$CERTS":/certs -e KAFKA_HEAP_OPTS="-Xmx2g -Xms512m" \
        -e KAFKA_LISTENERS="PLAINTEXT://:9092,CONTROLLER://:9093,SSL://:9094" \
        -e KAFKA_ADVERTISED_LISTENERS="PLAINTEXT://localhost:9092,SSL://localhost:9094" \
        -e KAFKA_LISTENER_SECURITY_PROTOCOL_MAP="PLAINTEXT:PLAINTEXT,CONTROLLER:PLAINTEXT,SSL:SSL" \
        -e KAFKA_SSL_KEYSTORE_LOCATION=/certs/server.keystore.jks -e KAFKA_SSL_KEYSTORE_PASSWORD=changeit \
        -e KAFKA_SSL_KEY_PASSWORD=changeit \
        -e KAFKA_SSL_TRUSTSTORE_LOCATION=/certs/client.truststore.jks -e KAFKA_SSL_TRUSTSTORE_PASSWORD=changeit \
        -e KAFKA_SSL_CLIENT_AUTH=none apache/kafka:3.8.0 >/dev/null
      echo "waiting for kafka SSL :9094 ..."
      if ! wait_port localhost 9094 150; then echo "::error::kafka never opened SSL :9094"; docker logs --tail 50 omb-kafka || true; exit 1; fi
      sleep 8
      DRIVER="driver-kafka-tls.yaml"
    else
      # --network host: kafka binds 9092 on the host; advertised localhost:9092 resolves for the colocated client.
      docker run -d --network host --name omb-kafka -e KAFKA_HEAP_OPTS="-Xmx2g -Xms512m" apache/kafka:3.8.0 >/dev/null
      echo "waiting for kafka :9092 ..."
      if ! wait_port localhost 9092 150; then echo "::error::kafka never opened :9092"; docker logs --tail 40 omb-kafka || true; exit 1; fi
      for _ in $(seq 1 30); do
        docker exec omb-kafka /opt/kafka/bin/kafka-topics.sh --bootstrap-server localhost:9092 --list >/dev/null 2>&1 && break
        sleep 2
      done
      DRIVER="driver-kafka-local.yaml"
    fi ;;
  rabbitmq)
    # Port-mapped (NOT --network host: that broke the erlang cookie). A non-`guest` user (guest is loopback-only,
    # which a -p gateway connection would reject) — the driver config authenticates as omb:omb over the URI.
    # A tmpfs data dir gives a fresh, correctly-permissioned /var/lib/rabbitmq so the erlang cookie is readable
    # by the rabbitmq user — the GH runner's overlay fs otherwise yields ".erlang.cookie: eacces" and the node
    # never starts. (Verified locally: with tmpfs the broker reaches "Server startup complete".)
    docker run -d -p 5672:5672 --tmpfs /var/lib/rabbitmq:rw,mode=1777 \
      -e RABBITMQ_ERLANG_COOKIE=omb-bench-cookie \
      -e RABBITMQ_DEFAULT_USER=omb -e RABBITMQ_DEFAULT_PASS=omb --name omb-rabbit rabbitmq:3.13 >/dev/null
    echo "waiting for rabbitmq node + :5672 ..."
    for _ in $(seq 1 90); do docker exec omb-rabbit rabbitmqctl await_startup >/dev/null 2>&1 && break; sleep 2; done
    if ! wait_port localhost 5672 90; then echo "::error::rabbitmq never opened :5672"; docker logs --tail 40 omb-rabbit || true; exit 1; fi
    DRIVER="driver-rabbitmq-local.yaml" ;;
  pulsar)
    docker run -d --network host --name omb-pulsar \
      -e PULSAR_MEM="-Xms512m -Xmx2g" apachepulsar/pulsar:3.3.1 bin/pulsar standalone >/dev/null
    echo "waiting for pulsar :6650 + admin ..."
    if ! wait_port localhost 6650 180; then echo "::error::pulsar never opened :6650"; docker logs --tail 40 omb-pulsar || true; exit 1; fi
    for _ in $(seq 1 45); do curl -sf localhost:8080/admin/v2/clusters >/dev/null 2>&1 && break; sleep 2; done
    DRIVER="driver-pulsar-local.yaml" ;;
  datarail)
    ./datarail-omb-shim >"$RESULTS/datarail-shim.log" 2>&1 &
    SHIM_PID=$!
    sleep 3
    if ! kill -0 "$SHIM_PID" 2>/dev/null; then echo "::error::shim failed to start"; cat "$RESULTS/datarail-shim.log"; exit 1; fi
    if ! wait_port 127.0.0.1 7701 15; then echo "::error::shim ingress :7701 not accepting"; cat "$RESULTS/datarail-shim.log"; exit 1; fi
    DRIVER="datarail.yaml" ;;
  *)
    echo "unknown system: $SYS"; exit 2 ;;
esac

echo "running OMB: driver=$DRIVER workload=$WL (cap ${RUN_TIMEOUT}s)"
# No workers.yaml present => OMB uses the in-process LocalWorker (single-node).
# OMB's bin/benchmark often does NOT exit after writing its result (lingering non-daemon driver threads),
# which would waste the whole timeout per system. So: run it in the background, and as soon as it has written
# the result JSON ("Writing test result into ..."), grant a short flush then stop it. A hard cap still bounds a
# genuine hang.
bin/benchmark -d "$DRIVER" "workloads/$WL" > "$OUT" 2>&1 &
bench_pid=$!
rc=0; done_ok=0
for _ in $(seq 1 "$RUN_TIMEOUT"); do
  if ! kill -0 "$bench_pid" 2>/dev/null; then wait "$bench_pid"; rc=$?; break; fi
  if grep -q "Writing test result into" "$OUT" 2>/dev/null; then
    done_ok=1; sleep 3            # let the JSON finish flushing to disk
    kill "$bench_pid" 2>/dev/null; pkill -P "$bench_pid" 2>/dev/null || true
    break
  fi
  sleep 1
done
if [ "$done_ok" -eq 0 ] && kill -0 "$bench_pid" 2>/dev/null; then
  echo "::warning::$SYS hit the ${RUN_TIMEOUT}s cap with no result — killing"
  kill -9 "$bench_pid" 2>/dev/null || true; rc=124
fi

mv ./*.json "$RESULTS/" 2>/dev/null || true
if [ "$done_ok" -eq 1 ]; then
  rc=0
elif [ "$rc" -ne 0 ]; then
  echo "::warning::$SYS exited rc=$rc — last lines:"; tail -20 "$OUT"
fi
echo "$SYS done (rc=$rc)"
exit "$rc"
