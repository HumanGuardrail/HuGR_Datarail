#!/usr/bin/env bash
# Run the OMB workload against ONE system, single-node, then tear it down.
# Usage: ./run-omb.sh <kafka|rabbitmq|pulsar|datarail> <workload-file> <results-dir>
# CWD must be the OMB distribution dir (has bin/benchmark, workloads/, the local *.yaml driver configs, the shim).
set -uo pipefail

SYS="$1"; WL="$2"; RESULTS="$3"
SHIM_PID=""
export HEAP_OPTS="-Xms1G -Xmx2G"   # OMB client heap — modest so it coexists with a colocated broker

cleanup() {
  [ -n "$SHIM_PID" ] && kill "$SHIM_PID" 2>/dev/null || true
  docker rm -f omb-kafka omb-rabbit omb-pulsar >/dev/null 2>&1 || true
}
trap cleanup EXIT

case "$SYS" in
  kafka)
    docker run -d --name omb-kafka -p 9092:9092 \
      -e KAFKA_HEAP_OPTS="-Xmx2g -Xms512m" apache/kafka:3.8.0 >/dev/null
    echo "waiting for kafka..."
    for i in $(seq 1 60); do
      docker exec omb-kafka /opt/kafka/bin/kafka-topics.sh --bootstrap-server localhost:9092 --list >/dev/null 2>&1 && break
      sleep 2
    done
    DRIVER="driver-kafka-local.yaml" ;;
  rabbitmq)
    docker run -d --name omb-rabbit -p 5672:5672 rabbitmq:3.13 >/dev/null
    echo "waiting for rabbitmq..."
    for i in $(seq 1 60); do
      docker exec omb-rabbit rabbitmq-diagnostics -q ping >/dev/null 2>&1 && break
      sleep 2
    done
    DRIVER="driver-rabbitmq-local.yaml" ;;
  pulsar)
    docker run -d --name omb-pulsar -p 6650:6650 -p 8080:8080 \
      -e PULSAR_MEM="-Xms512m -Xmx2g" apachepulsar/pulsar:3.3.1 bin/pulsar standalone >/dev/null
    echo "waiting for pulsar..."
    for i in $(seq 1 90); do
      curl -sf localhost:8080/admin/v2/clusters >/dev/null 2>&1 && break
      sleep 2
    done
    DRIVER="driver-pulsar-local.yaml" ;;
  datarail)
    ./datarail-omb-shim >shim.log 2>&1 &
    SHIM_PID=$!
    sleep 3
    if ! kill -0 "$SHIM_PID" 2>/dev/null; then echo "shim failed to start:"; cat shim.log; exit 1; fi
    DRIVER="datarail.yaml" ;;
  *)
    echo "unknown system: $SYS"; exit 2 ;;
esac

echo "running OMB: driver=$DRIVER workload=$WL"
# No workers.yaml present => OMB uses the in-process LocalWorker (single-node).
bin/benchmark -d "$DRIVER" "workloads/$WL"
rc=$?

mkdir -p "$RESULTS"
mv ./*.json "$RESULTS/" 2>/dev/null || true
echo "$SYS done (rc=$rc)"
exit 0
