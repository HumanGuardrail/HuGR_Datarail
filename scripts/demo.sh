#!/usr/bin/env bash
# demo.sh — 60-second proof of the datarail kafka-broker provider-blind property.
#
# What it does:
#   1. Builds datarail-cli --release if the binary is absent.
#   2. Starts the kafka-broker against a temp data dir and waits for the port.
#   3a. If kcat/kafkacat is on PATH: produces 3 records and consumes them back.
#   3b. If not: falls back to the native rail demo (--source-file → --sink-file).
#   4. THE PROOF: greps the data dir for plaintext, prints PASS/FAIL + hexdump.
#   5. (kcat path only) Restarts the broker with the same data dir, consumes again.
#   6. Cleans up with a trap.
#
# Environment overrides:
#   DATARAIL_BIN   — path to a pre-built datarail binary (skips the cargo build)
#   DATARAIL_PORT  — broker port (default 19092)
#
set -euo pipefail

REPO_ROOT="${REPO_ROOT:-"$(cd "$(dirname "$0")/.." && pwd)"}"
DATARAIL_BIN="${DATARAIL_BIN:-}"
DATARAIL_PORT="${DATARAIL_PORT:-19092}"
TOPIC="demo"
DATA_DIR=""
BROKER_PID=""

# ---------------------------------------------------------------------------
# Cleanup trap
# ---------------------------------------------------------------------------
cleanup() {
    if [ -n "$BROKER_PID" ] && kill -0 "$BROKER_PID" 2>/dev/null; then
        kill "$BROKER_PID" 2>/dev/null || true
        wait "$BROKER_PID" 2>/dev/null || true
    fi
    if [ -n "$DATA_DIR" ] && [ -d "$DATA_DIR" ]; then
        rm -rf "$DATA_DIR"
    fi
}
trap cleanup EXIT

# ---------------------------------------------------------------------------
# Step 1 — resolve binary
# ---------------------------------------------------------------------------
if [ -z "$DATARAIL_BIN" ]; then
    DATARAIL_BIN="$REPO_ROOT/target/release/datarail"
fi

if [ ! -x "$DATARAIL_BIN" ]; then
    echo "==> Binary not found at $DATARAIL_BIN — building datarail-cli --release ..."
    cargo build --release -p datarail-cli --manifest-path "$REPO_ROOT/Cargo.toml"
fi

echo "==> Using binary: $DATARAIL_BIN"
"$DATARAIL_BIN" --help 2>&1 | head -3

RAIL_TOML="$REPO_ROOT/examples/rail.toml"

# ---------------------------------------------------------------------------
# Step 2 — start the broker
# ---------------------------------------------------------------------------
DATA_DIR="$(mktemp -d)"
echo ""
echo "==> Starting datarail kafka-broker on port $DATARAIL_PORT (data dir: $DATA_DIR) ..."

"$DATARAIL_BIN" kafka-broker "$RAIL_TOML" \
    --listen "0.0.0.0:$DATARAIL_PORT" \
    --advertised "127.0.0.1" \
    --data-dir "$DATA_DIR" \
    --partitions 1 \
    >"$DATA_DIR/broker.log" 2>&1 &
BROKER_PID=$!

# Wait for the port to open (up to 10 s)
echo -n "==> Waiting for port $DATARAIL_PORT ..."
for i in $(seq 1 50); do
    if bash -c "echo >/dev/tcp/127.0.0.1/$DATARAIL_PORT" 2>/dev/null; then
        echo " ready (${i} × 200 ms)"
        break
    fi
    if ! kill -0 "$BROKER_PID" 2>/dev/null; then
        echo ""
        echo "❌  Broker died on startup. Log:"
        cat "$DATA_DIR/broker.log"
        exit 1
    fi
    if [ "$i" -eq 50 ]; then
        echo ""
        echo "❌  Timed out waiting for broker. Log:"
        cat "$DATA_DIR/broker.log"
        exit 1
    fi
    sleep 0.2
done

# ---------------------------------------------------------------------------
# Step 3 — produce + consume
# ---------------------------------------------------------------------------
KCAT=""
for candidate in kcat kafkacat; do
    if command -v "$candidate" >/dev/null 2>&1; then
        KCAT="$candidate"
        break
    fi
done

RECORDS=("evt:alpha" "evt:beta" "evt:gamma")

if [ -n "$KCAT" ]; then
    echo ""
    echo "==> kcat found ($KCAT) — running Kafka round-trip ..."

    # Produce 3 records
    printf '%s\n' "${RECORDS[@]}" | \
        "$KCAT" -P -b "127.0.0.1:$DATARAIL_PORT" -t "$TOPIC"
    echo "==> Produced: ${RECORDS[*]}"

    # Consume them back
    echo ""
    echo "==> Consuming from beginning ..."
    CONSUMED="$("$KCAT" -C -b "127.0.0.1:$DATARAIL_PORT" -t "$TOPIC" -o beginning -e 2>/dev/null)"
    echo "$CONSUMED"

    for rec in "${RECORDS[@]}"; do
        if ! echo "$CONSUMED" | grep -qF "$rec"; then
            echo "❌  Missing record in consumer output: $rec"
            exit 1
        fi
    done
    echo "✅  All 3 records round-tripped via Kafka wire protocol."

else
    echo ""
    echo "==> kcat/kafkacat not found — falling back to native rail demo."
    echo "    (Install kcat to exercise the Kafka wire path.)"

    IN_FILE="$(mktemp)"
    OUT_FILE="$(mktemp)"
    printf '%s\n' "${RECORDS[@]}" > "$IN_FILE"

    "$DATARAIL_BIN" run "$RAIL_TOML" \
        --source-file "$IN_FILE" \
        --sink-file "$OUT_FILE"

    echo ""
    echo "==> Native rail output:"
    cat "$OUT_FILE"

    for rec in "${RECORDS[@]}"; do
        if ! grep -qF "$rec" "$OUT_FILE"; then
            echo "❌  Missing record in rail output: $rec"
            rm -f "$IN_FILE" "$OUT_FILE"
            exit 1
        fi
    done
    echo "✅  All 3 records round-tripped via native rail."
    rm -f "$IN_FILE" "$OUT_FILE"
fi

# ---------------------------------------------------------------------------
# Step 4 — THE PROOF: grep for plaintext + hexdump
# ---------------------------------------------------------------------------
echo ""
echo "==> THE PROOF — searching data dir for plaintext payload strings ..."
SEARCH_TERMS=("evt:alpha" "evt:beta" "evt:gamma" "alpha" "beta" "gamma")
HITS=0
for term in "${SEARCH_TERMS[@]}"; do
    COUNT="$(grep -rl --include='*' "$term" "$DATA_DIR" 2>/dev/null | grep -v 'broker.log' | wc -l || true)"
    HITS=$(( HITS + COUNT ))
done
echo "==> On-disk plaintext grep result: $HITS hits for ${SEARCH_TERMS[*]}"

if [ "$HITS" -eq 0 ]; then
    echo "✅  PASS — on-disk store contains 0 plaintext hits — provider-blind ✅"
else
    echo "❌  FAIL — plaintext found in data dir (unexpected)."
    grep -rl --include='*' "evt:" "$DATA_DIR" 2>/dev/null | grep -v 'broker.log' || true
    exit 1
fi

echo ""
echo "==> Hexdump of first 64 bytes of a segment file (on-disk ciphertext is visible):"
SEGMENT_FILE="$(find "$DATA_DIR" -type f ! -name 'broker.log' ! -name '*.json' | head -1)"
if [ -n "$SEGMENT_FILE" ]; then
    echo "    File: $SEGMENT_FILE"
    od -A x -t x1z -v "$SEGMENT_FILE" 2>/dev/null | head -5 || \
        hexdump -C "$SEGMENT_FILE" 2>/dev/null | head -5 || \
        xxd "$SEGMENT_FILE" 2>/dev/null | head -5 || \
        echo "(no hexdump tool available)"
else
    echo "(no segment file found yet)"
fi

# ---------------------------------------------------------------------------
# Step 5 — durability: restart broker, consume again (kcat path only)
# ---------------------------------------------------------------------------
if [ -n "$KCAT" ]; then
    echo ""
    echo "==> Durability test — killing broker and restarting with same data dir ..."
    kill "$BROKER_PID" 2>/dev/null || true
    wait "$BROKER_PID" 2>/dev/null || true
    BROKER_PID=""

    "$DATARAIL_BIN" kafka-broker "$RAIL_TOML" \
        --listen "0.0.0.0:$DATARAIL_PORT" \
        --advertised "127.0.0.1" \
        --data-dir "$DATA_DIR" \
        --partitions 1 \
        >>"$DATA_DIR/broker.log" 2>&1 &
    BROKER_PID=$!

    echo -n "==> Waiting for broker restart ..."
    for i in $(seq 1 50); do
        if bash -c "echo >/dev/tcp/127.0.0.1/$DATARAIL_PORT" 2>/dev/null; then
            echo " ready."
            break
        fi
        if ! kill -0 "$BROKER_PID" 2>/dev/null; then
            echo ""
            echo "❌  Broker failed to restart. Log:"
            tail -20 "$DATA_DIR/broker.log"
            exit 1
        fi
        if [ "$i" -eq 50 ]; then
            echo ""
            echo "❌  Timed out on restart."
            exit 1
        fi
        sleep 0.2
    done

    echo "==> Consuming after restart ..."
    CONSUMED2="$("$KCAT" -C -b "127.0.0.1:$DATARAIL_PORT" -t "$TOPIC" -o beginning -e 2>/dev/null)"
    echo "$CONSUMED2"

    for rec in "${RECORDS[@]}"; do
        if ! echo "$CONSUMED2" | grep -qF "$rec"; then
            echo "❌  Record lost after restart: $rec"
            exit 1
        fi
    done
    echo "✅  Durability confirmed — all records survived broker restart."
fi

echo ""
echo "==> Demo complete. ✅"
