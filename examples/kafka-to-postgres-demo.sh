#!/usr/bin/env bash
# kafka-to-postgres-demo.sh — see the flagship datarail flow on your laptop in ~30s:
#   an UNMODIFIED Kafka producer (kcat) → datarail kafka-ingest → SEALED rail → Postgres.
# Requires: docker, and the datarail binary (the script builds it). No Kafka broker needed — datarail IS the
# Kafka-compatible endpoint. This is the local twin of the kafka-ingest.yml CI gate.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
CARGO_BIN="$HOME/.rustup/toolchains/stable-x86_64-apple-darwin/bin"
[ -d "$CARGO_BIN" ] && PATH="$CARGO_BIN:$PATH"

PG_PORT="${PG_PORT:-5444}"          # host port for the demo Postgres (avoid clashing with a local 5432)
KAFKA_PORT="${KAFKA_PORT:-9092}"
ADVERTISED="${ADVERTISED:-host.docker.internal}"   # what the containerized kcat must reach datarail at
CONTAINER="dr-demo-pg"

cleanup() {
  [ -n "${INGEST_PID:-}" ] && kill "$INGEST_PID" 2>/dev/null || true
  docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
}
trap cleanup EXIT

echo "==> building the datarail CLI (release)"
cargo build -q -p datarail-cli --release

echo "==> starting a throwaway Postgres (docker, port $PG_PORT)"
docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
docker run -d --name "$CONTAINER" -e POSTGRES_HOST_AUTH_METHOD=trust -p "$PG_PORT:5432" postgres:16 >/dev/null
for _ in $(seq 1 30); do docker exec "$CONTAINER" pg_isready -U postgres >/dev/null 2>&1 && break; sleep 1; done
docker exec "$CONTAINER" psql -U postgres -c "CREATE TABLE events (data text);" >/dev/null

echo "==> starting datarail kafka-ingest (Kafka wire :$KAFKA_PORT → sealed rail → Postgres)"
./target/release/datarail kafka-ingest examples/rail.toml \
  --listen "0.0.0.0:$KAFKA_PORT" --advertised "$ADVERTISED" \
  --sink-postgres "host=127.0.0.1,port=$PG_PORT,user=postgres,db=postgres,table=events,column=data" &
INGEST_PID=$!
sleep 3

echo "==> producing with an UNMODIFIED Kafka client (kcat in docker) — no datarail-specific code"
printf 'evt:order-1001\nevt:order-1002\nevt:order-1003\n' \
  | docker run --rm -i edenhill/kcat:1.7.1 -b "$ADVERTISED:$KAFKA_PORT" -t events -P
sleep 2

echo "==> what landed in Postgres (sealed through the rail, then opened at the sink):"
docker exec "$CONTAINER" psql -U postgres -c "SELECT data FROM events ORDER BY data;"
ROWS="$(docker exec "$CONTAINER" psql -U postgres -t -A -c 'SELECT count(*) FROM events;')"
echo "==> $ROWS rows landed from a real Kafka producer, provider-blind in transit. ✅"
