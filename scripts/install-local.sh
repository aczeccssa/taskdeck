#!/usr/bin/env sh
# Install the current Taskdeck checkout into Cargo's executable environment.
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$SCRIPT_DIR/lib/common.sh"

usage() {
    cat <<'EOF'
Usage:
  install-local.sh [--sync]

Build and install Taskdeck on this macOS or Linux machine using cargo install.
The running Taskdeck daemon is stopped after installation so the next command
starts the newly installed version.

Options:
  --sync    Fast-forward the clean Git worktree from its configured upstream.
  -h, --help
            Show this help text.
EOF
}

sync_git=false
while [ "$#" -gt 0 ]; do
    case "$1" in
        --sync) sync_git=true ;;
        -h|--help) usage; exit 0 ;;
        *) taskdeck_error "unknown option: $1" ;;
    esac
    shift
done

local_os=$(taskdeck_local_os)
local_arch=$(taskdeck_local_arch)

case "$local_os" in
    linux|macos) ;;
    windows) taskdeck_error 'Windows is not supported; use macOS or Linux' ;;
    *) taskdeck_error "unsupported local operating system: $(uname -s 2>/dev/null || printf unknown)" ;;
esac
[ "$local_arch" != unknown ] || taskdeck_error 'could not identify the local CPU architecture'

[ "$sync_git" = false ] || taskdeck_git_sync
taskdeck_require cargo

printf 'Installing Taskdeck for %s/%s from %s...\n' "$local_os" "$local_arch" "$TASKDECK_REPO_ROOT"
cargo install --locked --path "$TASKDECK_REPO_ROOT" --force

if command -v taskdeck >/dev/null 2>&1 && taskdeck shutdown >/dev/null 2>&1; then
    printf '%s\n' 'Stopped the running Taskdeck daemon.'
else
    printf '%s\n' 'No running Taskdeck daemon needed to be stopped.'
fi

printf '%s\n' 'Local installation completed.'
