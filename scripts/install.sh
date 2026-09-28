#!/bin/sh
# Install Taskdeck from a checksum-verified GitHub Release archive.
set -eu

REPOSITORY=${TASKDECK_REPOSITORY:-aczeccssa/taskdeck}
RELEASES_URL=${TASKDECK_RELEASES_URL:-https://github.com/$REPOSITORY/releases}
INSTALL_DIR=${TASKDECK_INSTALL_DIR:-${CARGO_HOME:-$HOME/.cargo}/bin}
VERSION=${TASKDECK_VERSION:-latest}

usage() {
    cat <<'EOF'
Usage: install.sh [--version VERSION] [--install-dir DIRECTORY]

Download and install the matching Taskdeck release for Linux or macOS.
The release archive is checked against its SHA256SUMS entry before install.

Options:
  --version VERSION      Release tag to install (default: latest)
  --install-dir DIRECTORY  Install directory (default: $CARGO_HOME/bin or ~/.cargo/bin)
  -h, --help             Show this help text.

Environment:
  TASKDECK_VERSION, TASKDECK_INSTALL_DIR, TASKDECK_RELEASES_URL,
  TASKDECK_REPOSITORY
EOF
}

fail() {
    printf 'error: %s\n' "$1" >&2
    exit 1
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --version)
            [ "$#" -ge 2 ] || fail '--version requires a value'
            VERSION=$2
            shift 2
            ;;
        --install-dir)
            [ "$#" -ge 2 ] || fail '--install-dir requires a value'
            INSTALL_DIR=$2
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *) fail "unknown option: $1" ;;
    esac
done

command -v curl >/dev/null 2>&1 || fail 'curl is required'
command -v tar >/dev/null 2>&1 || fail 'tar is required'
command -v awk >/dev/null 2>&1 || fail 'awk is required'

os=$(uname -s 2>/dev/null || printf unknown)
arch=$(uname -m 2>/dev/null || printf unknown)
case "$os/$arch" in
    Linux/x86_64|Linux/amd64) target=x86_64-unknown-linux-gnu; archive_ext=tar.gz ;;
    Linux/aarch64|Linux/arm64) target=aarch64-unknown-linux-gnu; archive_ext=tar.gz ;;
    Darwin/x86_64) target=x86_64-apple-darwin; archive_ext=tar.gz ;;
    Darwin/arm64|Darwin/aarch64) target=aarch64-apple-darwin; archive_ext=tar.gz ;;
    MINGW*/*|MSYS*/*|CYGWIN*/*|Windows_NT/*)
        fail 'Windows is not supported by this shell installer; use scripts/install-local.ps1 or download the Windows release archive'
        ;;
    *) fail "unsupported platform: $os/$arch" ;;
esac

if [ "$VERSION" = latest ]; then
    latest_url=$(curl --fail --silent --show-error --location --output /dev/null --write-out '%{url_effective}' "$RELEASES_URL/latest") \
        || fail 'could not resolve the latest GitHub Release'
    VERSION=${latest_url##*/}
fi
case "$VERSION" in
    v[0-9]*.[0-9]*.[0-9]*) ;;
    *) fail "invalid release version: $VERSION" ;;
esac
case "$VERSION" in *[!A-Za-z0-9.+_-]*) fail "invalid release version: $VERSION" ;; esac

archive_name="taskdeck-${VERSION}-${target}.${archive_ext}"
download_url="$RELEASES_URL/download/$VERSION"
tmp_root=${TMPDIR:-/tmp}
tmp_dir=$(mktemp -d "$tmp_root/taskdeck-install.XXXXXX") || fail 'could not create a temporary directory'
staged=''
cleanup() { rm -f "$tmp_dir/$archive_name" "$tmp_dir/SHA256SUMS" "$tmp_dir/taskdeck"; [ -z "$staged" ] || rm -f "$staged"; rmdir "$tmp_dir" 2>/dev/null || true; }
trap cleanup EXIT HUP INT TERM

printf 'Downloading Taskdeck %s for %s...\n' "$VERSION" "$target"
curl --fail --silent --show-error --location "$download_url/$archive_name" -o "$tmp_dir/$archive_name" \
    || fail "could not download release asset: $archive_name"
curl --fail --silent --show-error --location "$download_url/SHA256SUMS" -o "$tmp_dir/SHA256SUMS" \
    || fail 'could not download release checksums'

expected=$(awk -v name="$archive_name" '$2 == name { print $1; exit }' "$tmp_dir/SHA256SUMS")
case "$expected" in
    ''|*[!0-9a-fA-F]*) fail "release checksum is missing or invalid for $archive_name" ;;
esac
[ "${#expected}" -eq 64 ] || fail "release checksum is invalid for $archive_name"
if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$tmp_dir/$archive_name" | awk '{print $1}')
elif command -v shasum >/dev/null 2>&1; then
    actual=$(shasum -a 256 "$tmp_dir/$archive_name" | awk '{print $1}')
else
    fail 'sha256sum or shasum is required to verify the release archive'
fi
[ "$actual" = "$expected" ] || fail "SHA256 checksum mismatch for $archive_name"

tar -xzf "$tmp_dir/$archive_name" -C "$tmp_dir" taskdeck || fail 'could not extract taskdeck from the release archive'
[ -f "$tmp_dir/taskdeck" ] || fail 'release archive does not contain taskdeck'
chmod 755 "$tmp_dir/taskdeck"

mkdir -p "$INSTALL_DIR" || fail "could not create install directory: $INSTALL_DIR"
staged="$INSTALL_DIR/.taskdeck.new.$$"
cp "$tmp_dir/taskdeck" "$staged" || fail "could not stage Taskdeck in $INSTALL_DIR"
chmod 755 "$staged"
if command -v taskdeck >/dev/null 2>&1; then
    taskdeck shutdown >/dev/null 2>&1 || true
fi
mv -f "$staged" "$INSTALL_DIR/taskdeck" || fail "could not install Taskdeck in $INSTALL_DIR"

printf 'Taskdeck %s installed at %s/taskdeck\n' "$VERSION" "$INSTALL_DIR"
case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *) printf 'Add this directory to PATH to run taskdeck: %s\n' "$INSTALL_DIR" ;;
esac
