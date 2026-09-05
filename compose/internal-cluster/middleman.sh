#!/bin/sh
# Drive the cluster scenario over the shared bus, then compare leader vs worker.
set -eu

BUS_DIR=${TASKDECK_BUS_DIR:-/shared/bus}
READY_DIR=$BUS_DIR/ready
COMMAND_DIR=$BUS_DIR/commands
ACK_DIR=$BUS_DIR/acks
REPORT_DIR=${TASKDECK_REPORT_DIR:-$BUS_DIR/report}
LEADER_URL=${TASKDECK_LEADER_URL:-http://leader:9837}
WORKER_URL=${TASKDECK_WORKER_URL:-http://worker:9837}
SESSION=${TASKDECK_SESSION:-cluster-lab}
TASK=${TASKDECK_TASK:-ticker}
MODE=${TASKDECK_CLUSTER_MODE:-self-check}

mkdir -p "$READY_DIR" "$COMMAND_DIR" "$ACK_DIR" "$REPORT_DIR"

log() { printf '[middleman] %s\n' "$*"; }

set_phase() {
    python3 -c 'import json,sys; open(sys.argv[1],"w",encoding="utf-8").write(json.dumps({"phase":sys.argv[2],"mode":sys.argv[3]})+"\n")' "$BUS_DIR/phase.json" "$1" "$MODE"
    log "phase=$1"
}

wait_file() {
    path=$1
    i=0
    while [ "$i" -lt 90 ]; do
        [ -f "$path" ] && return 0
        i=$((i + 1))
        sleep 0.5
    done
    echo "timed out waiting for $path" >&2
    return 1
}

http_json() { curl --fail --silent --show-error "$@"; }

set_phase waiting_for_nodes
wait_file "$READY_DIR/leader.json"
wait_file "$READY_DIR/worker.json"

i=0
WORKER_NODE_ID=
while [ "$i" -lt 60 ]; do
    nodes=$(http_json "$LEADER_URL/api/nodes")
    WORKER_NODE_ID=$(printf '%s' "$nodes" | python3 -c 'import json,sys
payload=json.load(sys.stdin)
for node in payload.get("data") or []:
    if node.get("is_self"):
        continue
    if node.get("online") and "cluster-lab" in (node.get("sessions") or []):
        print(node.get("id",""))
        break')
    [ -n "$WORKER_NODE_ID" ] && break
    i=$((i + 1))
    sleep 0.5
done
[ -n "$WORKER_NODE_ID" ] || { echo "leader never listed an online worker with session $SESSION" >&2; exit 1; }
log "worker_node_id=$WORKER_NODE_ID"

issue_command() {
    seq=$1
    op=$2
    extra=${3:-}
    id=$(printf '%03d-%s' "$seq" "$op")
    python3 -c 'import json,sys
path, ident, op, session, task, extra = sys.argv[1:7]
payload={"id":ident,"op":op,"session":session}
if extra:
    key,value=extra.split("=",1)
    payload[key]=value
if op in {"action","snapshot"} and "task" not in payload:
    payload["task"]=task
open(path,"w",encoding="utf-8").write(json.dumps(payload)+"\n")' "$COMMAND_DIR/$id.json" "$id" "$op" "$SESSION" "$TASK" "$extra"
    echo "$id"
}

wait_ack() {
    id=$1
    path=$ACK_DIR/$id.json
    i=0
    while [ "$i" -lt 60 ]; do
        if [ -f "$path" ]; then
            python3 -c 'import json,sys
payload=json.load(open(sys.argv[1],encoding="utf-8"))
if not payload.get("ok"):
    raise SystemExit("command failed: %s" % payload)
print(payload.get("message","ok"))' "$path"
            return 0
        fi
        i=$((i + 1))
        sleep 0.5
    done
    echo "timed out waiting for ack $id" >&2
    return 1
}

leader_action() {
    action=$1
    body=$(python3 -c 'import json,sys; print(json.dumps({"node":sys.argv[1],"session":sys.argv[2],"task":sys.argv[3],"action":sys.argv[4]}))' "$WORKER_NODE_ID" "$SESSION" "$TASK" "$action")
    response=$(curl --silent --show-error -H 'content-type: application/json' -d "$body" "$LEADER_URL/api/action")
    printf '%s' "$response" | python3 -c 'import json,sys; payload=json.load(sys.stdin); raise SystemExit(0 if payload.get("ok") else 1)' || {
        echo "leader action $action failed: $response" >&2
        return 1
    }
    printf '%s
' "$response"
}

wait_task_status() {
    url=$1
    expected=$2
    i=0
    snapshot=
    while [ "$i" -lt 40 ]; do
        snapshot=$(http_json "$url")
        status=$(printf '%s' "$snapshot" | python3 -c 'import json,sys
task=sys.argv[1]
payload=json.load(sys.stdin)
print((((payload.get("data") or {}).get("tasks") or {}).get(task) or {}).get("status",""))' "$TASK")
        if [ "$status" = "$expected" ]; then
            echo "$snapshot"
            return 0
        fi
        i=$((i + 1))
        sleep 0.5
    done
    echo "task $TASK on $url never reached $expected" >&2
    echo "$snapshot" >&2
    return 1
}

set_phase ping
id=$(issue_command 1 ping)
wait_ack "$id" >/dev/null
log "worker ping ack ok"

set_phase worker_local_start
id=$(issue_command 2 action action=start)
wait_ack "$id" >/dev/null
wait_task_status "$WORKER_URL/api/sessions/$SESSION?tail=5" running >/dev/null
wait_task_status "$LEADER_URL/api/sessions/$SESSION?node=$WORKER_NODE_ID&tail=5" running >/dev/null
log "ticker running on worker and leader inventory"

set_phase worker_local_stop
id=$(issue_command 3 action action=stop)
wait_ack "$id" >/dev/null
wait_task_status "$WORKER_URL/api/sessions/$SESSION?tail=5" idle >/dev/null
wait_task_status "$LEADER_URL/api/sessions/$SESSION?node=$WORKER_NODE_ID&tail=5" idle >/dev/null
log "ticker stopped by worker-local action"

set_phase leader_remote_start
leader_action start >/dev/null
wait_task_status "$WORKER_URL/api/sessions/$SESSION?tail=5" running >/dev/null
wait_task_status "$LEADER_URL/api/sessions/$SESSION?node=$WORKER_NODE_ID&tail=5" running >/dev/null
log "ticker started by leader remote action"

set_phase leader_remote_stop
leader_action stop >/dev/null
wait_task_status "$WORKER_URL/api/sessions/$SESSION?tail=5" idle >/dev/null
wait_task_status "$LEADER_URL/api/sessions/$SESSION?node=$WORKER_NODE_ID&tail=5" idle >/dev/null
log "ticker stopped by leader remote action"

set_phase compare
python3 -c 'import json,os,sys,time,urllib.request
leader_url, worker_url, session, task, worker_node, mode, report_path = sys.argv[1:8]

def load(url):
    with urllib.request.urlopen(url, timeout=10) as response:
        payload = json.load(response)
    return payload

def task_status(snapshot):
    return (((snapshot.get("data") or {}).get("tasks") or {}).get(task) or {}).get("status")

leader_nodes = load(leader_url + "/api/nodes")
worker_nodes = load(worker_url + "/api/nodes")
leader_snapshot = load("%s/api/sessions/%s?node=%s&tail=20" % (leader_url, session, worker_node))
worker_snapshot = load("%s/api/sessions/%s?tail=20" % (worker_url, session))
leader_audit = load(leader_url + "/api/audit?page_size=100")
worker_audit = load(worker_url + "/api/audit?page_size=100")
leader_items = ((leader_audit.get("data") or {}).get("items") or [])
worker_items = ((worker_audit.get("data") or {}).get("items") or [])
worker_local = [item for item in worker_items if item.get("operation") in ("start","stop") and item.get("source") in ("cli","web","tui")]
leader_replicated = [item for item in leader_items if item.get("executor_node_id")==worker_node and item.get("origin_node_id")==worker_node]
leader_origin = [item for item in leader_items if item.get("executor_node_id")==worker_node and item.get("origin_node_id")!=worker_node]
worker_remote = [item for item in worker_items if item.get("transport")=="agent" or (item.get("operation") in ("start","stop") and item.get("source") in ("web","internal"))]
worker_by_corr = {}
for item in worker_items:
    worker_by_corr.setdefault(item.get("correlation_id"), []).append(item)
twins = sum(1 for item in leader_origin if item.get("correlation_id") in worker_by_corr)
online_worker = next((node for node in (leader_nodes.get("data") or []) if node.get("id")==worker_node), None)
checks = [
    {"name":"leader_sees_online_worker","ok":bool(online_worker and online_worker.get("online")),"detail":online_worker},
    {"name":"leader_and_worker_task_status_match","ok":task_status(leader_snapshot)==task_status(worker_snapshot),"detail":{"leader":task_status(leader_snapshot),"worker":task_status(worker_snapshot)}},
    {"name":"worker_has_local_action_audit","ok":bool(worker_local),"detail":len(worker_local)},
    {"name":"leader_has_replicated_worker_audit","ok":bool(leader_replicated),"detail":len(leader_replicated)},
    {"name":"leader_has_origin_remote_audit","ok":bool(leader_origin),"detail":len(leader_origin)},
    {"name":"worker_has_executor_remote_audit","ok":bool(worker_remote),"detail":len(worker_remote)},
    {"name":"leader_worker_correlation_present","ok":twins>0,"detail":twins},
]
ok = all(item["ok"] for item in checks)
report = {
    "ok": ok,
    "mode": mode,
    "generated_at_ms": int(time.time()*1000),
    "worker_node_id": worker_node,
    "session": session,
    "task": task,
    "checks": checks,
    "leader_nodes": leader_nodes.get("data"),
    "worker_nodes": worker_nodes.get("data"),
    "leader_task_status": task_status(leader_snapshot),
    "worker_task_status": task_status(worker_snapshot),
    "leader_audit_total": (leader_audit.get("data") or {}).get("total"),
    "worker_audit_total": (worker_audit.get("data") or {}).get("total"),
}
os.makedirs(os.path.dirname(report_path) or ".", exist_ok=True)
tmp = report_path + ".tmp"
open(tmp,"w",encoding="utf-8").write(json.dumps(report, indent=2)+"\n")
os.replace(tmp, report_path)
print(json.dumps({"ok":ok,"report":report_path,"failed":[c["name"] for c in checks if not c["ok"]]}))
raise SystemExit(0 if ok else 1)' "$LEADER_URL" "$WORKER_URL" "$SESSION" "$TASK" "$WORKER_NODE_ID" "$MODE" "$REPORT_DIR/latest.json"

set_phase complete
log "report written to $REPORT_DIR/latest.json"

if [ "$MODE" = "observer" ]; then
    log "observer mode: keeping compose active"
    while true; do
        sleep 3600
    done
fi

log "self-check mode: middleman exiting"
