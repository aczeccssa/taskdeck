#!/usr/bin/env sh

TASKDECK_SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
TASKDECK_REPO_ROOT=$(CDPATH= cd -- "$TASKDECK_SCRIPT_DIR/.." && pwd)

taskdeck_error() {
    printf 'error: %s\n' "$1" >&2
    exit 1
}

taskdeck_require() {
    command -v "$1" >/dev/null 2>&1 || taskdeck_error "$1 is required but was not found in PATH"
}

taskdeck_normalize_os() {
    case "$1" in
        Darwin|darwin|MacOS|macOS) printf '%s\n' macos ;;
        Linux|linux) printf '%s\n' linux ;;
        MINGW*|MSYS*|CYGWIN*|Windows_NT|Windows|windows) printf '%s\n' windows ;;
        *) printf '%s\n' unknown ;;
    esac
}

taskdeck_normalize_arch() {
    case "$1" in
        x86_64|amd64|x64) printf '%s\n' x86_64 ;;
        arm64|aarch64) printf '%s\n' aarch64 ;;
        armv7l|armv7) printf '%s\n' armv7 ;;
        *) printf '%s\n' unknown ;;
    esac
}

taskdeck_local_os() {
    taskdeck_normalize_os "$(uname -s 2>/dev/null || printf unknown)"
}

taskdeck_local_arch() {
    taskdeck_normalize_arch "$(uname -m 2>/dev/null || printf unknown)"
}

taskdeck_git_sync() {
    taskdeck_require git

    git -C "$TASKDECK_REPO_ROOT" rev-parse --is-inside-work-tree >/dev/null 2>&1 \
        || taskdeck_error "$TASKDECK_REPO_ROOT is not a Git worktree"

    dirty=$(git -C "$TASKDECK_REPO_ROOT" status --porcelain)
    [ -z "$dirty" ] || taskdeck_error 'Git sync requires a clean worktree; commit or stash local changes first'

    upstream=$(git -C "$TASKDECK_REPO_ROOT" rev-parse --abbrev-ref --symbolic-full-name '@{upstream}' 2>/dev/null) \
        || taskdeck_error 'current branch has no upstream; configure one before using --sync'
    remote=${upstream%%/*}
    [ "$remote" != "$upstream" ] || taskdeck_error "could not determine the remote from upstream '$upstream'"

    printf 'Syncing Git worktree from %s...\n' "$upstream"
    git -C "$TASKDECK_REPO_ROOT" fetch --prune "$remote"
    git -C "$TASKDECK_REPO_ROOT" merge --ff-only "$upstream"
}
