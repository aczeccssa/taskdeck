#!/usr/bin/env sh
set -eu

REPO_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
TEST_TMP=$(mktemp -d "${TMPDIR:-/tmp}/taskdeck-scripts-test.XXXXXX")

cleanup() {
    find "$TEST_TMP" -type f -exec unlink {} \; 2>/dev/null || true
    find "$TEST_TMP" -depth -type d -exec rmdir {} \; 2>/dev/null || true
}
trap cleanup EXIT HUP INT TERM

fail() {
    printf 'FAIL: %s\n' "$1" >&2
    exit 1
}

assert_contains() {
    haystack=$1
    needle=$2
    case "$haystack" in
        *"$needle"*) ;;
        *) fail "expected output to contain: $needle" ;;
    esac
}

assert_file_contains() {
    file=$1
    needle=$2
    grep -F "$needle" "$file" >/dev/null || fail "expected $file to contain: $needle"
}

test_help_from_repo_and_scripts_directories() {
    for script in install-local.sh deploy-compose.sh deploy-ssh.sh; do
        [ -x "$REPO_ROOT/scripts/$script" ] || fail "scripts/$script is missing or not executable"
        output=$(cd "$REPO_ROOT" && "./scripts/$script" --help)
        assert_contains "$output" "Usage:"
        output=$(cd "$REPO_ROOT/scripts" && "./$script" --help)
        assert_contains "$output" "Usage:"
    done
}

test_root_level_legacy_scripts_are_removed() {
    for script in install.sh deploy_compose.sh deploy_ssh.sh; do
        [ ! -e "$REPO_ROOT/$script" ] || fail "$script should live under scripts/ with its new name"
    done
    [ -f "$REPO_ROOT/scripts/install-local.ps1" ] || fail "scripts/install-local.ps1 is missing"
}

test_local_install_supports_windows() {
    mock_bin="$TEST_TMP/windows-bin"
    log="$TEST_TMP/windows-install.log"
    mkdir -p "$mock_bin"
    cat >"$mock_bin/uname" <<'EOF'
#!/usr/bin/env sh
case "${1:-}" in
    -s) printf '%s\n' MINGW64_NT-10.0 ;;
    -m) printf '%s\n' x86_64 ;;
esac
EOF
    cat >"$mock_bin/cargo" <<'EOF'
#!/usr/bin/env sh
printf 'cargo args=%s\n' "$*" >>"$TASKDECK_TEST_LOG"
EOF
    cat >"$mock_bin/taskdeck" <<'EOF'
#!/usr/bin/env sh
printf 'taskdeck args=%s\n' "$*" >>"$TASKDECK_TEST_LOG"
EOF
    cat >"$mock_bin/powershell.exe" <<'EOF'
#!/usr/bin/env sh
printf 'powershell args=%s\n' "$*" >>"$TASKDECK_TEST_LOG"
EOF
    chmod +x "$mock_bin/uname" "$mock_bin/cargo" "$mock_bin/taskdeck" "$mock_bin/powershell.exe"

    (cd "$REPO_ROOT" && TASKDECK_TEST_LOG="$log" PATH="$mock_bin:$PATH" ./scripts/install-local.sh)

    assert_file_contains "$log" "cargo args=install --locked --path $REPO_ROOT --force"
    assert_file_contains "$log" "taskdeck args=shutdown"
}

test_local_install_uses_repo_root_from_both_directories() {
    mock_bin="$TEST_TMP/install-bin"
    log="$TEST_TMP/install.log"
    mkdir -p "$mock_bin"
    cat >"$mock_bin/uname" <<'EOF'
#!/usr/bin/env sh
case "${1:-}" in
    -s) printf '%s\n' Linux ;;
    -m) printf '%s\n' x86_64 ;;
esac
EOF
    cat >"$mock_bin/cargo" <<'EOF'
#!/usr/bin/env sh
printf 'cargo cwd=%s args=%s\n' "$PWD" "$*" >>"$TASKDECK_TEST_LOG"
EOF
    cat >"$mock_bin/taskdeck" <<'EOF'
#!/usr/bin/env sh
printf 'taskdeck cwd=%s args=%s\n' "$PWD" "$*" >>"$TASKDECK_TEST_LOG"
EOF
    chmod +x "$mock_bin/uname" "$mock_bin/cargo" "$mock_bin/taskdeck"

    (cd "$REPO_ROOT" && TASKDECK_TEST_LOG="$log" PATH="$mock_bin:$PATH" ./scripts/install-local.sh)
    (cd "$REPO_ROOT/scripts" && TASKDECK_TEST_LOG="$log" PATH="$mock_bin:$PATH" ./install-local.sh)

    assert_file_contains "$log" "cargo cwd=$REPO_ROOT args=install --locked --path $REPO_ROOT --force"
    assert_file_contains "$log" "cargo cwd=$REPO_ROOT/scripts args=install --locked --path $REPO_ROOT --force"
    assert_file_contains "$log" "taskdeck cwd=$REPO_ROOT args=shutdown"
    assert_file_contains "$log" "taskdeck cwd=$REPO_ROOT/scripts args=shutdown"
}

create_compose_mocks() {
    mock_bin=$1
    mkdir -p "$mock_bin"
    cat >"$mock_bin/docker" <<'EOF'
#!/usr/bin/env sh
printf 'docker cwd=%s args=%s\n' "$PWD" "$*" >>"$TASKDECK_TEST_LOG"
exit 0
EOF
    chmod +x "$mock_bin/docker"
}

test_compose_deploy_uses_repo_root_from_both_directories() {
    mock_bin="$TEST_TMP/compose-bin"
    log="$TEST_TMP/compose.log"
    create_compose_mocks "$mock_bin"

    (cd "$REPO_ROOT" && TASKDECK_TEST_LOG="$log" PATH="$mock_bin:$PATH" ./scripts/deploy-compose.sh)
    (cd "$REPO_ROOT/scripts" && TASKDECK_TEST_LOG="$log" PATH="$mock_bin:$PATH" ./deploy-compose.sh)

    assert_file_contains "$log" "docker cwd=$REPO_ROOT"
    assert_file_contains "$log" "compose -f $REPO_ROOT/compose.yaml config --quiet"
    assert_file_contains "$log" "up --build --force-recreate --detach --wait taskdeck-leader"
}

test_sync_fast_forwards_before_compose_deploy() {
    mock_bin="$TEST_TMP/sync-bin"
    log="$TEST_TMP/sync.log"
    create_compose_mocks "$mock_bin"
    cat >"$mock_bin/git" <<'EOF'
#!/usr/bin/env sh
printf 'git args=%s\n' "$*" >>"$TASKDECK_TEST_LOG"
case "$*" in
    *"rev-parse --is-inside-work-tree"*) printf '%s\n' true ;;
    *"rev-parse --abbrev-ref --symbolic-full-name @{upstream}"*) printf '%s\n' origin/main ;;
esac
EOF
    chmod +x "$mock_bin/git"

    (cd "$REPO_ROOT/scripts" && TASKDECK_TEST_LOG="$log" PATH="$mock_bin:$PATH" ./deploy-compose.sh --sync)

    assert_file_contains "$log" "git args=-C $REPO_ROOT fetch --prune origin"
    assert_file_contains "$log" "git args=-C $REPO_ROOT merge --ff-only origin/main"
}

test_compose_deploy_targets_an_explicit_docker_context() {
    mock_bin="$TEST_TMP/context-bin"
    log="$TEST_TMP/context.log"
    create_compose_mocks "$mock_bin"

    (cd "$REPO_ROOT" && TASKDECK_TEST_LOG="$log" PATH="$mock_bin:$PATH" ./scripts/deploy-compose.sh --context remote-engine --no-build --no-wait)

    assert_file_contains "$log" "docker cwd=$REPO_ROOT args=--context remote-engine compose -f $REPO_ROOT/compose.yaml config --quiet"
    assert_file_contains "$log" "docker cwd=$REPO_ROOT args=--context remote-engine compose -f $REPO_ROOT/compose.yaml up --force-recreate --detach taskdeck-leader"
}

test_ssh_deploy_uses_repo_layout_from_both_directories() {
    fixture="$TEST_TMP/ssh-repo"
    mock_bin="$TEST_TMP/ssh-bin"
    log="$TEST_TMP/ssh.log"
    mkdir -p "$fixture/scripts/lib" "$fixture/src" "$mock_bin"
    fixture=$(CDPATH= cd -- "$fixture" && pwd)
    cp "$REPO_ROOT/scripts/deploy-ssh.sh" "$fixture/scripts/deploy-ssh.sh"
    cp "$REPO_ROOT/scripts/lib/common.sh" "$fixture/scripts/lib/common.sh"
    chmod +x "$fixture/scripts/deploy-ssh.sh" "$fixture/scripts/lib/common.sh"
    : >"$fixture/Cargo.toml"
    : >"$fixture/Cargo.lock"
    : >"$fixture/src/main.rs"

    cat >"$mock_bin/uname" <<'EOF'
#!/usr/bin/env sh
case "${1:-}" in
    -s) printf '%s\n' Darwin ;;
    -m) printf '%s\n' arm64 ;;
esac
EOF
    cat >"$mock_bin/cargo" <<'EOF'
#!/usr/bin/env sh
printf 'cargo cwd=%s args=%s\n' "$PWD" "$*" >>"$TASKDECK_TEST_LOG"
mkdir -p "$TASKDECK_TEST_REPO/target/release"
cat >"$TASKDECK_TEST_REPO/target/release/taskdeck" <<'BIN'
#!/usr/bin/env sh
printf '%s\n' 'taskdeck 0.1.0'
BIN
chmod +x "$TASKDECK_TEST_REPO/target/release/taskdeck"
EOF
    cat >"$mock_bin/ssh" <<'EOF'
#!/usr/bin/env sh
printf 'ssh cwd=%s args=%s\n' "$PWD" "$*" >>"$TASKDECK_TEST_LOG"
case "$*" in
    *"uname -s"*) printf '%s\n' Darwin ;;
    *"uname -m"*) printf '%s\n' arm64 ;;
    *"--version"*) printf '%s\n' 'taskdeck 0.1.0' ;;
esac
EOF
    cat >"$mock_bin/scp" <<'EOF'
#!/usr/bin/env sh
printf 'scp cwd=%s args=%s\n' "$PWD" "$*" >>"$TASKDECK_TEST_LOG"
EOF
    chmod +x "$mock_bin/uname" "$mock_bin/cargo" "$mock_bin/ssh" "$mock_bin/scp"

    (cd "$fixture" && TASKDECK_TEST_LOG="$log" TASKDECK_TEST_REPO="$fixture" TASKDECK_SSH_BIN=ssh TASKDECK_SCP_BIN=scp PATH="$mock_bin:$PATH" ./scripts/deploy-ssh.sh --remote-path /opt/taskdeck/bin/taskdeck dev@example.test)
    (cd "$fixture/scripts" && TASKDECK_TEST_LOG="$log" TASKDECK_TEST_REPO="$fixture" TASKDECK_SSH_BIN=ssh TASKDECK_SCP_BIN=scp PATH="$mock_bin:$PATH" ./deploy-ssh.sh --remote-path /opt/taskdeck/bin/taskdeck dev@example.test)

    assert_file_contains "$log" "cargo cwd=$fixture args=build --locked --release --manifest-path $fixture/Cargo.toml"
    assert_file_contains "$log" "scp cwd=$fixture args="
    assert_file_contains "$log" "scp cwd=$fixture/scripts args="
    assert_file_contains "$log" "/opt/taskdeck/bin/taskdeck"
}

test_ssh_deploy_uses_powershell_for_windows() {
    fixture="$TEST_TMP/windows-ssh-repo"
    mock_bin="$TEST_TMP/windows-ssh-bin"
    log="$TEST_TMP/windows-ssh.log"
    mkdir -p "$fixture/scripts/lib" "$fixture/src" "$mock_bin"
    fixture=$(CDPATH= cd -- "$fixture" && pwd)
    cp "$REPO_ROOT/scripts/deploy-ssh.sh" "$fixture/scripts/deploy-ssh.sh"
    cp "$REPO_ROOT/scripts/lib/common.sh" "$fixture/scripts/lib/common.sh"
    chmod +x "$fixture/scripts/deploy-ssh.sh" "$fixture/scripts/lib/common.sh"
    : >"$fixture/Cargo.toml"
    : >"$fixture/Cargo.lock"
    : >"$fixture/src/main.rs"

    cat >"$mock_bin/uname" <<'EOF'
#!/usr/bin/env sh
case "${1:-}" in
    -s) printf '%s\n' Darwin ;;
    -m) printf '%s\n' arm64 ;;
esac
EOF
    cat >"$mock_bin/ssh" <<'EOF'
#!/usr/bin/env sh
printf 'ssh args=%s\n' "$*" >>"$TASKDECK_TEST_LOG"
last=''
for argument in "$@"; do
    last=$argument
done
case "$*" in
    *-EncodedCommand*)
        decoded=$(printf '%s' "${last##* }" | base64 -d | iconv -f UTF-16LE -t UTF-8)
        printf 'powershell decoded=%s\n' "$decoded" >>"$TASKDECK_TEST_LOG"
        case "$decoded" in
            *PROCESSOR_ARCHITECTURE*) printf '%s' AMD64 ;;
            *LOCALAPPDATA*) printf '%s' 'C:\Users\22407\AppData\Local' ;;
            *"Write('windows')"*) printf '%s' windows ;;
            *'--version'*) printf '%s\n' 'taskdeck 0.1.0' ;;
        esac
        ;;
esac
EOF
    cat >"$mock_bin/scp" <<'EOF'
#!/usr/bin/env sh
printf 'scp args=%s\n' "$*" >>"$TASKDECK_TEST_LOG"
EOF
    cat >"$mock_bin/rustup" <<'EOF'
#!/usr/bin/env sh
printf 'rustup args=%s\n' "$*" >>"$TASKDECK_TEST_LOG"
mkdir -p "$TASKDECK_TEST_REPO/target/x86_64-pc-windows-msvc/release"
: >"$TASKDECK_TEST_REPO/target/x86_64-pc-windows-msvc/release/taskdeck.exe"
EOF
    cat >"$mock_bin/cargo-xwin" <<'EOF'
#!/usr/bin/env sh
exit 0
EOF
    chmod +x "$mock_bin/uname" "$mock_bin/ssh" "$mock_bin/scp" "$mock_bin/rustup" "$mock_bin/cargo-xwin"

    (cd "$fixture" && TASKDECK_TEST_LOG="$log" TASKDECK_TEST_REPO="$fixture" TASKDECK_SSH_BIN=ssh TASKDECK_SCP_BIN=scp PATH="$mock_bin:$PATH" ./scripts/deploy-ssh.sh 22407@company-hp66)

    assert_file_contains "$log" "PROCESSOR_ARCHITECTURE"
    assert_file_contains "$log" "rustup args=run stable cargo xwin build --locked --release --target x86_64-pc-windows-msvc"
    assert_file_contains "$log" "taskdeck.exe 22407@company-hp66:taskdeck."
    assert_file_contains "$log" "Copy-Item"
    assert_file_contains "$log" "SetEnvironmentVariable('Path'"
    assert_file_contains "$log" "C:\Users\22407\AppData\Local/Taskdeck/taskdeck.exe"
}

test_help_from_repo_and_scripts_directories
test_root_level_legacy_scripts_are_removed
test_local_install_supports_windows
test_local_install_uses_repo_root_from_both_directories
test_compose_deploy_uses_repo_root_from_both_directories
test_sync_fast_forwards_before_compose_deploy
test_compose_deploy_targets_an_explicit_docker_context
test_ssh_deploy_uses_repo_layout_from_both_directories
test_ssh_deploy_uses_powershell_for_windows

printf '%s\n' 'scripts tests passed'
