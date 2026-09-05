#!/bin/sh
# Publish leader readiness onto the shared bus, then run the daemon.
set -eu

BUS_DIR=${TASKDECK_BUS_DIR:-/shared/bus}
READY_DIR=$BUS_DIR/ready
mkdir -p "$READY_DIR" "$BUS_DIR/commands" "$BUS_DIR/acks" "$BUS_DIR/report"

taskdeck daemon &
DAEMON_PID=$!

cleanup() {
    kill "$DAEMON_PID" 2>/dev/null || true
    wait "$DAEMON_PID" 2>/dev/null || true
}
trap cleanup INT TERM

i=0
while [ "$i" -lt 60 ]; do
    if curl --fail --silent "http://127.0.0.1:9837/healthz" >/dev/null 2>&1; then
        break
    fi
    i=$((i + 1))
    sleep 0.5
done
if [ "$i" -ge 60 ]; then
    echo "leader HTTP never became healthy" >&2
    exit 1
fi

tmp=$READY_DIR/leader.json.tmp
cat >"$tmp" <<JSON
{
  "role": "leader",
  "name": "${TASKDECK_NODE_NAME:-cluster-leader}",
  "url": "http://leader:9837",
  "healthz": "ok"
}
JSON
mv "$tmp" "$READY_DIR/leader.json"

wait "$DAEMON_PID"
