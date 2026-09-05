#!/bin/sh
# Register the lab session, connect to the leader, then execute bus commands.
set -eu

BUS_DIR=${TASKDECK_BUS_DIR:-/shared/bus}
READY_DIR=$BUS_DIR/ready
COMMAND_DIR=$BUS_DIR/commands
ACK_DIR=$BUS_DIR/acks
PROJECT=${TASKDECK_PROJECT:-/work/project}
SESSION=${TASKDECK_SESSION:-cluster-lab}

mkdir -p "$READY_DIR" "$COMMAND_DIR" "$ACK_DIR"

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
    echo "worker HTTP never became healthy" >&2
    exit 1
fi

json_get() {
    python3 -c 'import json,sys; data=json.load(open(sys.argv[1], encoding="utf-8")); value=data
for part in sys.argv[2].split("."):
    value=value[part]
print("" if value is None else ("true" if value is True else ("false" if value is False else value)))' "$1" "$2"
}

register_out=$(taskdeck --project "$PROJECT" --session "$SESSION" register)
echo "$register_out"

i=0
while [ "$i" -lt 60 ]; do
    nodes=$(curl --fail --silent http://leader:9837/api/nodes || true)
    if printf '%s' "$nodes" | grep -q '"online":true' && printf '%s' "$nodes" | grep -q "$SESSION"; then
        break
    fi
    i=$((i + 1))
    sleep 0.5
done

tmp=$READY_DIR/worker.json.tmp
cat >"$tmp" <<JSON
{
  "role": "worker",
  "name": "${TASKDECK_NODE_NAME:-cluster-worker}",
  "url": "http://worker:9837",
  "session": "$SESSION",
  "project": "$PROJECT",
  "healthz": "ok"
}
JSON
mv "$tmp" "$READY_DIR/worker.json"

processed=
while kill -0 "$DAEMON_PID" 2>/dev/null; do
    for command in "$COMMAND_DIR"/*.json; do
        [ -e "$command" ] || continue
        base=$(basename "$command")
        case " $processed " in
            *" $base "*) continue ;;
        esac
        ack=$ACK_DIR/$base
        [ -f "$ack" ] && continue
        id=$(json_get "$command" id || printf unknown)
        op=$(json_get "$command" op || printf unknown)
        session=$(json_get "$command" session || printf '%s' "$SESSION")
        task=$(json_get "$command" task || true)
        action=$(json_get "$command" action || true)
        ok=false
        message=""
        data="{}"
        case "$op" in
            ping)
                if curl --fail --silent http://127.0.0.1:9837/healthz >/dev/null; then
                    ok=true
                    message="pong"
                else
                    message="worker healthz failed"
                fi
                ;;
            action)
                if [ -z "$action" ]; then
                    message="missing action"
                else
                    body='{"session":"'"$session"'","action":"'"$action"'"'
                    if [ -n "$task" ]; then
                        body=$body',"task":"'"$task"'"'
                    fi
                    body=$body'}'
                    response=$(curl --fail --silent -H 'content-type: application/json' -d "$body" http://127.0.0.1:9837/api/action || true)
                    if printf '%s' "$response" | grep -q '"ok":true'; then
                        ok=true
                    fi
                    message=$(printf '%s' "$response" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("message",""))' 2>/dev/null || true)
                    data=$(printf '%s' "$response" || printf '{}')
                fi
                ;;
            snapshot)
                response=$(curl --fail --silent "http://127.0.0.1:9837/api/sessions/$session?tail=20" || true)
                if printf '%s' "$response" | grep -q '"ok":true'; then
                    ok=true
                fi
                message=$(printf '%s' "$response" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("message",""))' 2>/dev/null || true)
                data=$(printf '%s' "$response" || printf '{}')
                ;;
            *)
                message="unsupported op: $op"
                ;;
        esac
        python3 -c 'import json,sys,time
path, ident, op, ok, message, data_raw = sys.argv[1:7]
try:
    data = json.loads(data_raw) if data_raw else {}
except json.JSONDecodeError:
    data = {"raw": data_raw}
payload = {"id": ident, "op": op, "ok": ok == "true", "message": message, "data": data, "timestamp_ms": int(time.time() * 1000)}
open(path, "w", encoding="utf-8").write(json.dumps(payload) + "\n")' "$ack.tmp" "$id" "$op" "$ok" "$message" "$data"
        mv "$ack.tmp" "$ack"
        processed="$processed $base"
    done
    sleep 0.2
done

wait "$DAEMON_PID"
