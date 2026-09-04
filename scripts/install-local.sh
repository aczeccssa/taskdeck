#!/usr/bin/env sh
# Install the current Taskdeck checkout into Cargo's executable environment.
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$SCRIPT_DIR/lib/common.sh"

usage() {
    cat <<'EOF'
Usage:
  install-local.sh [--sync]

Build and install Taskdeck using cargo install.
The running Taskdeck daemon is stopped before installation so its executable
can be replaced safely on every supported platform.

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
    linux|macos|windows) ;;
    *) taskdeck_error "unsupported local operating system: $(uname -s 2>/dev/null || printf unknown)" ;;
esac
[ "$local_arch" != unknown ] || taskdeck_error 'could not identify the local CPU architecture'

[ "$sync_git" = false ] || taskdeck_git_sync
taskdeck_require cargo

if command -v taskdeck >/dev/null 2>&1 && taskdeck shutdown >/dev/null 2>&1; then
    printf '%s\n' 'Stopped the running Taskdeck daemon.'
else
    printf '%s\n' 'No running Taskdeck daemon needed to be stopped.'
fi
if [ "$local_os" = windows ]; then
    powershell.exe -NoLogo -NoProfile -NonInteractive -Command \
        'Get-Process taskdeck -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue'
    sleep 2
fi

printf 'Installing Taskdeck for %s/%s from %s...\n' "$local_os" "$local_arch" "$TASKDECK_REPO_ROOT"
cargo install --locked --path "$TASKDECK_REPO_ROOT" --force

if [ "$local_os" = windows ]; then
    powershell.exe -NoLogo -NoProfile -NonInteractive -Command \
        '$cargoRoot = if ($env:CARGO_INSTALL_ROOT) { $env:CARGO_INSTALL_ROOT } elseif ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE ".cargo" }; $installDir = Join-Path $cargoRoot "bin"; $userPath = [Environment]::GetEnvironmentVariable("Path", "User"); $pathEntries = @($userPath -split ";" | Where-Object { $_ }); if ($pathEntries -notcontains $installDir) { [Environment]::SetEnvironmentVariable("Path", (($pathEntries + $installDir) -join ";"), "User") }'
fi

printf '%s\n' 'Local installation completed.'
