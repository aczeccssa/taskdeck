#!/usr/bin/env sh
# Run the private leader/worker/middleman Compose cluster.
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$SCRIPT_DIR/lib/common.sh"

usage() {
    cat <<'EOF'
Usage:
  run-internal-cluster.sh [options]

Bring up an internal Docker network with one leader, one worker, and one
middleman. The worker enrolls itself with the leader. Middleman then drives
local and remote task actions over a shared file bus, compares leader/worker
state, and writes a report.

Modes:
  self-check   Run, report, destroy the stack, keep only the report (default)
  observer     Run, report, leave the stack running

Options:
  --mode MODE           self-check or observer (default: self-check)
  --project-name NAME   Compose project name
  --no-build            Reuse the current taskdeck:local image
  --keep-stack          Leave Compose running even in self-check mode
  --token TOKEN         Enrollment token (default: TASKDECK_ENROLLMENT_TOKEN or a lab token)
  -h, --help            Show this help text

Environment:
  TASKDECK_ENROLLMENT_TOKEN
  TASKDECK_INTERNAL_COMPOSE_PROJECT
  TASKDECK_IMAGE
EOF
}

mode=${TASKDECK_CLUSTER_MODE:-self-check}
project_name=${TASKDECK_INTERNAL_COMPOSE_PROJECT:-taskdeck-internal}
build_image=true
keep_stack=false
token=${TASKDECK_ENROLLMENT_TOKEN:-}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --mode)
            [ "$#" -ge 2 ] || taskdeck_error '--mode requires a value'
            mode=$2
            shift
            ;;
        --project-name)
            [ "$#" -ge 2 ] || taskdeck_error '--project-name requires a value'
            project_name=$2
            shift
            ;;
        --no-build) build_image=false ;;
        --keep-stack) keep_stack=true ;;
        --token)
            [ "$#" -ge 2 ] || taskdeck_error '--token requires a value'
            token=$2
            shift
            ;;
        -h|--help) usage; exit 0 ;;
        *) taskdeck_error "unknown option: $1" ;;
    esac
    shift
done

case "$mode" in
    self-check|observer) ;;
    *) taskdeck_error "unknown mode: $mode (expected self-check or observer)" ;;
esac

taskdeck_require docker
docker compose version >/dev/null 2>&1 || taskdeck_error 'Docker Compose v2 is required'

compose_file="$TASKDECK_REPO_ROOT/compose/internal-cluster/compose.yaml"
[ -f "$compose_file" ] || taskdeck_error "Compose file not found: $compose_file"

if [ -z "$token" ]; then
    token=taskdeck-internal-lab-token
fi

stamp=$(date -u +%Y%m%dT%H%M%SZ)
report_host_dir="$TASKDECK_REPO_ROOT/output/internal-cluster/$stamp"
mkdir -p "$report_host_dir"

cd "$TASKDECK_REPO_ROOT"
export TASKDECK_ENROLLMENT_TOKEN=$token
export TASKDECK_CLUSTER_MODE=$mode
export TASKDECK_INTERNAL_COMPOSE_PROJECT=$project_name
export TASKDECK_IMAGE=${TASKDECK_IMAGE:-taskdeck:local}

printf 'Validating %s...\n' "$compose_file"
docker compose -f "$compose_file" --project-name "$project_name" config --quiet

down_stack() {
    docker compose -f "$compose_file" --project-name "$project_name" down --volumes --remove-orphans >/dev/null 2>&1 || true
}

# Always start from a clean private lab network and fresh bus volume. This avoids
# stale command/ack files after interrupted self-check or observer runs.
printf 'Resetting internal cluster project %s...\n' "$project_name"
down_stack

copy_report() {
    # The middleman exits immediately after writing its final report, so include
    # stopped containers when resolving the copy source.
    cid=$(docker compose -f "$compose_file" --project-name "$project_name" ps -aq middleman 2>/dev/null || true)
    tmp_report="$report_host_dir/report.json.tmp"
    rm -f "$tmp_report"
    if [ -n "$cid" ]; then
        if docker cp "$cid":/shared/bus/report/latest.json "$tmp_report" 2>/dev/null && [ -s "$tmp_report" ]; then
            mv "$tmp_report" "$report_host_dir/report.json"
        else
            rm -f "$tmp_report"
        fi
        docker cp "$cid":/shared/bus/phase.json "$report_host_dir/phase.json" 2>/dev/null || true
    fi
    if [ ! -s "$report_host_dir/report.json" ]; then
        vol=${TASKDECK_INTERNAL_BUS:-taskdeck-internal-bus}
        helper=taskdeck-internal-report-copy
        docker rm -f "$helper" >/dev/null 2>&1 || true
        if docker run --rm --name "$helper" -v "$vol":/shared/bus busybox cat /shared/bus/report/latest.json >"$tmp_report" 2>/dev/null && [ -s "$tmp_report" ]; then
            mv "$tmp_report" "$report_host_dir/report.json"
        else
            rm -f "$tmp_report"
        fi
    fi
}

trap 'status=$?; copy_report; if [ "$mode" = self-check ] && [ "$keep_stack" = false ]; then down_stack; fi; exit $status' EXIT INT TERM

if [ "$build_image" = true ]; then
    printf 'Building and starting internal cluster (%s)...\n' "$mode"
    docker compose -f "$compose_file" --project-name "$project_name" up --build --force-recreate --detach --wait leader worker
else
    printf 'Starting internal cluster (%s)...\n' "$mode"
    docker compose -f "$compose_file" --project-name "$project_name" up --force-recreate --detach --wait leader worker
fi
docker compose -f "$compose_file" --project-name "$project_name" up --no-deps --force-recreate --detach middleman

printf 'Waiting for middleman report...\n'
i=0
middleman_status=1
while [ "$i" -lt 180 ]; do
    copy_report
    if [ -f "$report_host_dir/report.json" ]; then
        if python3 -c 'import json,sys; report=json.load(open(sys.argv[1],encoding="utf-8")); raise SystemExit(0 if report.get("ok") else 1)' "$report_host_dir/report.json"; then
            middleman_status=0
            break
        else
            middleman_status=1
            # Keep waiting until middleman exits or a complete failed report exists.
            cid=$(docker compose -f "$compose_file" --project-name "$project_name" ps -q middleman 2>/dev/null || true)
            if [ -n "$cid" ]; then
                running=$(docker inspect -f '{{.State.Running}}' "$cid" 2>/dev/null || printf false)
                if [ "$running" = false ]; then
                    break
                fi
            fi
        fi
    else
        cid=$(docker compose -f "$compose_file" --project-name "$project_name" ps -q middleman 2>/dev/null || true)
        if [ -n "$cid" ]; then
            running=$(docker inspect -f '{{.State.Running}}' "$cid" 2>/dev/null || printf false)
            if [ "$running" = false ] && [ "$mode" = self-check ]; then
                break
            fi
        fi
    fi
    i=$((i + 1))
    sleep 1
done

if [ ! -f "$report_host_dir/report.json" ]; then
    taskdeck_error "middleman did not write a report; inspect docker compose logs"
fi

python3 -c 'import json,sys
report=json.load(open(sys.argv[1],encoding="utf-8"))
print("report:", sys.argv[1])
print("ok:", report.get("ok"))
print("mode:", report.get("mode"))
failed=[item["name"] for item in report.get("checks") or [] if not item.get("ok")]
print("failed:", ", ".join(failed) if failed else "(none)")' "$report_host_dir/report.json"

if [ "$mode" = observer ]; then
    printf 'Observer mode: stack is still running as project %s\n' "$project_name"
    printf 'Leader UI is on the internal network only; use docker compose port mapping if needed.\n'
    printf 'Stop with: docker compose -f %s --project-name %s down --volumes\n' "$compose_file" "$project_name"
fi

[ "$middleman_status" -eq 0 ] || exit 1
