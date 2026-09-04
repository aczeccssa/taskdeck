#!/usr/bin/env sh
# Rebuild and replace a Taskdeck service with Docker Compose.
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$SCRIPT_DIR/lib/common.sh"

usage() {
    cat <<'EOF'
Usage:
  deploy-compose.sh [options]

Validate the Compose configuration, rebuild the image, recreate the target
service, wait for it to become healthy, and print its final status.

Options:
  --sync              Fast-forward a clean Git worktree before deployment.
  --context NAME      Deploy through the named Docker context.
  --file PATH         Compose file (default: compose.yaml).
  --service NAME      Compose service (default: taskdeck-leader).
  --no-build          Reuse the current Compose image.
  --no-wait           Do not wait for the service health check.
  -h, --help          Show this help text.

Environment:
  TASKDECK_COMPOSE_FILE       Alternative default Compose file.
  TASKDECK_COMPOSE_SERVICE    Alternative default service name.
  TASKDECK_DOCKER_CONTEXT     Alternative default Docker context.
  TASKDECK_ENROLLMENT_TOKEN   Required by the supplied compose.yaml.
  TASKDECK_NODE_NAME          Node name passed through Compose.
  TASKDECK_PORT               Published Web/MCP port.
EOF
}

sync_git=false
build_image=true
wait_for_health=true
compose_file=${TASKDECK_COMPOSE_FILE:-compose.yaml}
service=${TASKDECK_COMPOSE_SERVICE:-taskdeck-leader}
docker_context=${TASKDECK_DOCKER_CONTEXT:-}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --sync) sync_git=true ;;
        --context)
            [ "$#" -ge 2 ] || taskdeck_error '--context requires a name'
            docker_context=$2
            shift
            ;;
        --file)
            [ "$#" -ge 2 ] || taskdeck_error '--file requires a path'
            compose_file=$2
            shift
            ;;
        --service)
            [ "$#" -ge 2 ] || taskdeck_error '--service requires a name'
            service=$2
            shift
            ;;
        --no-build) build_image=false ;;
        --no-wait) wait_for_health=false ;;
        -h|--help) usage; exit 0 ;;
        *) taskdeck_error "unknown option: $1" ;;
    esac
    shift
done

case "$compose_file" in
    /*) ;;
    *) compose_file="$TASKDECK_REPO_ROOT/$compose_file" ;;
esac
[ -f "$compose_file" ] || taskdeck_error "Compose file not found: $compose_file"
[ -n "$service" ] || taskdeck_error 'Compose service name is empty'

[ "$sync_git" = false ] || taskdeck_git_sync
taskdeck_require docker

docker_compose() {
    if [ -n "$docker_context" ]; then
        docker --context "$docker_context" compose "$@"
    else
        docker compose "$@"
    fi
}

docker_compose version >/dev/null 2>&1 || taskdeck_error 'Docker Compose v2 is required'

cd "$TASKDECK_REPO_ROOT"
printf 'Validating %s...\n' "$compose_file"
docker_compose -f "$compose_file" config --quiet

printf 'Deploying Compose service %s...\n' "$service"
if [ "$build_image" = true ] && [ "$wait_for_health" = true ]; then
    docker_compose -f "$compose_file" up --build --force-recreate --detach --wait "$service"
elif [ "$build_image" = true ]; then
    docker_compose -f "$compose_file" up --build --force-recreate --detach "$service"
elif [ "$wait_for_health" = true ]; then
    docker_compose -f "$compose_file" up --force-recreate --detach --wait "$service"
else
    docker_compose -f "$compose_file" up --force-recreate --detach "$service"
fi

docker_compose -f "$compose_file" ps "$service"
printf '%s\n' 'Compose deployment completed.'
