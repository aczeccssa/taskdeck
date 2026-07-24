#!/usr/bin/env sh
# Build a Taskdeck binary for a remote host and install it over SSH.
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$SCRIPT_DIR/lib/common.sh"

REMOTE_INSTALL_PATH=${TASKDECK_REMOTE_PATH:-}
SSH_CONNECT_TIMEOUT=${TASKDECK_SSH_CONNECT_TIMEOUT:-10}
SSH_CONTROL_PATH="${TMPDIR:-/tmp}/taskdeck-ssh.$$.sock"
SSH_BIN=${TASKDECK_SSH_BIN:-ssh}
SCP_BIN=${TASKDECK_SCP_BIN:-scp}
if [ -z "${TASKDECK_SSH_BIN:-}" ] && [ "$(taskdeck_local_os)" = macos ] && [ -x /usr/bin/ssh ]; then
    SSH_BIN=/usr/bin/ssh
fi
if [ -z "${TASKDECK_SCP_BIN:-}" ] && [ "$(taskdeck_local_os)" = macos ] && [ -x /usr/bin/scp ]; then
    SCP_BIN=/usr/bin/scp
fi

usage() {
    cat <<'EOF'
Usage:
  deploy-ssh.sh [options] <username>@<host>
  deploy-ssh.sh [options] <host> <username>

Detect the remote operating system and CPU architecture, reuse a current
matching binary when possible, otherwise build one, then install it over SSH.
SSH and sudo prompt for passwords interactively when required.

Options:
  --sync              Fast-forward a clean Git worktree before building.
  --force-build       Rebuild even when a current matching artifact exists.
  --remote-path PATH  Installation path (default: /usr/local/bin/taskdeck).
  --timeout SECONDS   SSH connection timeout (default: 10).
  -h, --help          Show this help text.

Environment:
  TASKDECK_REMOTE_PATH          Alternative absolute installation path.
  TASKDECK_SSH_CONNECT_TIMEOUT  Alternative SSH connection timeout.
  TASKDECK_SSH_BIN              Alternative ssh executable.
  TASKDECK_SCP_BIN              Alternative scp executable.
EOF
}

sync_git=false
force_build=false
while [ "$#" -gt 0 ]; do
    case "$1" in
        --sync) sync_git=true; shift ;;
        --force-build) force_build=true; shift ;;
        --remote-path)
            [ "$#" -ge 2 ] || taskdeck_error '--remote-path requires a path'
            REMOTE_INSTALL_PATH=$2
            shift 2
            ;;
        --timeout)
            [ "$#" -ge 2 ] || taskdeck_error '--timeout requires seconds'
            SSH_CONNECT_TIMEOUT=$2
            shift 2
            ;;
        -h|--help) usage; exit 0 ;;
        --) shift; break ;;
        -*) taskdeck_error "unknown option: $1" ;;
        *) break ;;
    esac
done

if [ "$#" -eq 1 ]; then
    case "$1" in
        *@*) REMOTE_TARGET=$1 ;;
        *) usage >&2; exit 2 ;;
    esac
elif [ "$#" -eq 2 ]; then
    REMOTE_HOST=$1
    REMOTE_USER=$2
    [ -n "$REMOTE_HOST" ] || taskdeck_error 'remote host is empty'
    [ -n "$REMOTE_USER" ] || taskdeck_error 'remote username is empty'
    REMOTE_TARGET="$REMOTE_USER@$REMOTE_HOST"
else
    usage >&2
    exit 2
fi

case "$SSH_CONNECT_TIMEOUT" in
    ''|*[!0-9]*) taskdeck_error '--timeout must be a positive integer' ;;
    0) taskdeck_error '--timeout must be greater than zero' ;;
esac

taskdeck_require "$SSH_BIN"
taskdeck_require "$SCP_BIN"
[ "$sync_git" = false ] || taskdeck_git_sync

build_dir=''
remote_tmp=''
remote_os=''

cleanup() {
    if [ -n "$remote_tmp" ]; then
        if [ "$remote_os" = windows ]; then
            powershell_remote "Remove-Item -LiteralPath (Join-Path \$HOME '$remote_tmp') -Force -ErrorAction SilentlyContinue" >/dev/null 2>&1 || true
        else
            "$SSH_BIN" -o ControlPath="$SSH_CONTROL_PATH" -o ConnectTimeout="$SSH_CONNECT_TIMEOUT" "$REMOTE_TARGET" "rm -f '$remote_tmp'" >/dev/null 2>&1 || true
        fi
    fi
    "$SSH_BIN" -O exit -o ControlPath="$SSH_CONTROL_PATH" "$REMOTE_TARGET" >/dev/null 2>&1 || true
    if [ -n "$build_dir" ]; then
        [ ! -e "$build_dir/taskdeck" ] || unlink "$build_dir/taskdeck"
        [ ! -d "$build_dir" ] || rmdir "$build_dir"
    fi
}
trap cleanup EXIT HUP INT TERM

ssh_remote() {
    "$SSH_BIN" \
        -o ControlMaster=auto \
        -o ControlPersist=60 \
        -o ControlPath="$SSH_CONTROL_PATH" \
        -o ConnectTimeout="$SSH_CONNECT_TIMEOUT" \
        "$REMOTE_TARGET" "$@"
}

powershell_remote() {
    script="\$ProgressPreference = 'SilentlyContinue'; $1"
    encoded=$(printf '%s' "$script" | iconv -f UTF-8 -t UTF-16LE | base64 | tr -d '\r\n')
    ssh_remote "powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand $encoded"
}

local_os=$(taskdeck_local_os)
local_arch=$(taskdeck_local_arch)

windows_probe=''
if command -v iconv >/dev/null 2>&1 \
    && command -v base64 >/dev/null 2>&1 \
    && windows_probe=$(powershell_remote "[Console]::Out.Write('windows')" 2>/dev/null) \
    && [ "$windows_probe" = windows ]; then
    remote_os=windows
else
    remote_os_raw=''
    if remote_os_raw=$(ssh_remote 'uname -s 2>/dev/null' 2>/dev/null); then
        :
    fi
    remote_os=$(taskdeck_normalize_os "$(printf '%s' "$remote_os_raw" | sed -n '1p')")
fi

[ "$remote_os" != unknown ] || taskdeck_error 'could not identify the remote OS with PowerShell or uname'

remote_arch_raw=''
if [ "$remote_os" = windows ]; then
    if remote_arch_raw=$(powershell_remote '[Console]::Out.Write($env:PROCESSOR_ARCHITECTURE)' 2>/dev/null); then
        :
    fi
else
    if remote_arch_raw=$(ssh_remote 'uname -m 2>/dev/null' 2>/dev/null); then
        :
    fi
fi
remote_arch=$(taskdeck_normalize_arch "$(printf '%s' "$remote_arch_raw" | sed -n '1p')")
[ "$remote_arch" != unknown ] || taskdeck_error 'could not identify the remote architecture'

case "$remote_os/$remote_arch" in
    macos/x86_64) target_triple=x86_64-apple-darwin ;;
    macos/aarch64) target_triple=aarch64-apple-darwin ;;
    linux/x86_64) target_triple=x86_64-unknown-linux-gnu ;;
    linux/aarch64) target_triple=aarch64-unknown-linux-gnu ;;
    windows/x86_64) target_triple=x86_64-pc-windows-msvc ;;
    windows/aarch64) target_triple=aarch64-pc-windows-msvc ;;
    *) taskdeck_error "unsupported target platform: $remote_os/$remote_arch" ;;
esac

if [ -z "$REMOTE_INSTALL_PATH" ]; then
    if [ "$remote_os" = windows ]; then
        remote_local_app_data=$(powershell_remote '[Console]::Out.Write($env:LOCALAPPDATA)')
        [ -n "$remote_local_app_data" ] || taskdeck_error 'PowerShell did not report LOCALAPPDATA'
        REMOTE_INSTALL_PATH="$remote_local_app_data/Taskdeck/taskdeck.exe"
    else
        REMOTE_INSTALL_PATH=/usr/local/bin/taskdeck
    fi
fi
if [ "$remote_os" = windows ]; then
    case "$REMOTE_INSTALL_PATH" in
        [A-Za-z]:[\\/]*.exe) ;;
        *) taskdeck_error 'Windows installation path must be an absolute .exe path' ;;
    esac
    case "$REMOTE_INSTALL_PATH" in
        *"'"*) taskdeck_error "Windows installation path cannot contain '" ;;
    esac
else
    case "$REMOTE_INSTALL_PATH" in
        /*) ;;
        *) taskdeck_error 'remote installation path must be absolute' ;;
    esac
    case "$REMOTE_INSTALL_PATH" in
        *[!A-Za-z0-9_./-]*) taskdeck_error 'remote installation path contains unsupported characters' ;;
    esac
fi

printf 'Remote target: %s (%s)\n' "$remote_os" "$remote_arch"

artifact_name=taskdeck
[ "$remote_os" != windows ] || artifact_name=taskdeck.exe
artifact="$TASKDECK_REPO_ROOT/target/$target_triple/release/$artifact_name"
if [ "$remote_os" = "$local_os" ] && [ "$remote_arch" = "$local_arch" ]; then
    artifact="$TASKDECK_REPO_ROOT/target/release/$artifact_name"
fi

artifact_is_current() {
    [ "$force_build" = false ] || return 1
    [ -f "$1" ] || return 1
    [ "$remote_os" = windows ] || [ -x "$1" ] || return 1
    if find "$TASKDECK_REPO_ROOT/src" "$TASKDECK_REPO_ROOT/Cargo.toml" "$TASKDECK_REPO_ROOT/Cargo.lock" -type f -newer "$1" -print -quit | grep -q .; then
        return 1
    fi
    return 0
}

if artifact_is_current "$artifact"; then
    printf 'Using current artifact: %s\n' "$artifact"
else
    printf 'Compiling %s...\n' "$target_triple"
    if [ "$remote_os" = "$local_os" ] && [ "$remote_arch" = "$local_arch" ]; then
        taskdeck_require cargo
        cargo build --locked --release --manifest-path "$TASKDECK_REPO_ROOT/Cargo.toml"
    elif [ "$remote_os" = windows ]; then
        taskdeck_require rustup
        taskdeck_require cargo-xwin
        rustup run stable cargo xwin build \
            --locked \
            --release \
            --target "$target_triple" \
            --manifest-path "$TASKDECK_REPO_ROOT/Cargo.toml"
    elif [ "$remote_os" = linux ] && command -v docker >/dev/null 2>&1 && docker buildx version >/dev/null 2>&1; then
        build_dir=$(mktemp -d "${TMPDIR:-/tmp}/taskdeck-build.XXXXXX")
        case "$remote_arch" in
            x86_64) docker_platform=linux/amd64 ;;
            aarch64) docker_platform=linux/arm64 ;;
        esac
        docker buildx build \
            --platform "$docker_platform" \
            --target artifact \
            --output "type=local,dest=$build_dir" \
            "$TASKDECK_REPO_ROOT"
        built_artifact="$build_dir/taskdeck"
        [ -x "$built_artifact" ] || taskdeck_error "Docker build completed without $built_artifact"
        mkdir -p "$(dirname "$artifact")"
        cp "$built_artifact" "$artifact"
        chmod 0755 "$artifact"
    else
        taskdeck_require cargo
        taskdeck_require rustup
        rustup target list --installed | grep -qx "$target_triple" || rustup target add "$target_triple"
        cargo build --locked --release --target "$target_triple" --manifest-path "$TASKDECK_REPO_ROOT/Cargo.toml"
    fi
    [ -f "$artifact" ] || taskdeck_error "build did not produce $artifact"
    [ "$remote_os" = windows ] || [ -x "$artifact" ] || taskdeck_error "build artifact is not executable: $artifact"
fi

if [ "$remote_os" = windows ]; then
    remote_tmp="taskdeck.$$.exe"
else
    remote_tmp="/tmp/taskdeck.$$.bin"
fi

printf 'Uploading %s to %s...\n' "$(basename "$artifact")" "$REMOTE_TARGET"
"$SCP_BIN" \
    -o ControlMaster=auto \
    -o ControlPersist=60 \
    -o ControlPath="$SSH_CONTROL_PATH" \
    -o ConnectTimeout="$SSH_CONNECT_TIMEOUT" \
    "$artifact" "$REMOTE_TARGET:$remote_tmp"

if [ "$remote_os" = windows ]; then
    printf 'Installing at %s...\n' "$REMOTE_INSTALL_PATH"
    powershell_remote "\$ErrorActionPreference = 'Stop'
\$source = Join-Path \$HOME '$remote_tmp'
\$target = '$REMOTE_INSTALL_PATH'
New-Item -ItemType Directory -Path (Split-Path -Parent \$target) -Force | Out-Null
if (Test-Path -LiteralPath \$target) {
    & \$target shutdown *> \$null
    \$targetPath = [IO.Path]::GetFullPath(\$target)
    Get-Process taskdeck -ErrorAction SilentlyContinue |
        Where-Object { [IO.Path]::GetFullPath(\$_.Path) -eq \$targetPath } |
        Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 1200
}
Copy-Item -LiteralPath \$source -Destination \$target -Force
Remove-Item -LiteralPath \$source -Force
\$installDir = Split-Path -Parent \$target
\$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
\$pathEntries = @(\$userPath -split ';' | Where-Object { \$_ })
if (\$pathEntries -notcontains \$installDir) {
    [Environment]::SetEnvironmentVariable('Path', ((\$pathEntries + \$installDir) -join ';'), 'User')
}"
else
    printf 'Installing at %s (sudo may prompt for a password)...\n' "$REMOTE_INSTALL_PATH"
    "$SSH_BIN" -tt \
        -o ControlMaster=auto \
        -o ControlPersist=60 \
        -o ControlPath="$SSH_CONTROL_PATH" \
        -o ConnectTimeout="$SSH_CONNECT_TIMEOUT" \
        "$REMOTE_TARGET" "set -eu
install_dir='${REMOTE_INSTALL_PATH%/*}'
if [ \"\$(id -u)\" -eq 0 ]; then
    mkdir -p \"\$install_dir\"
    install -m 0755 '$remote_tmp' '$REMOTE_INSTALL_PATH'
elif command -v sudo >/dev/null 2>&1; then
    sudo mkdir -p \"\$install_dir\"
    sudo install -m 0755 '$remote_tmp' '$REMOTE_INSTALL_PATH'
else
    printf '%s\\n' 'error: remote user is not root and sudo is unavailable' >&2
    exit 1
fi
rm -f '$remote_tmp'"
fi

printf 'Installed version: '
if [ "$remote_os" = windows ]; then
    powershell_remote "& '$REMOTE_INSTALL_PATH' --version"
else
    ssh_remote "$REMOTE_INSTALL_PATH --version"
fi
printf '%s\n' 'Remote installation completed.'
