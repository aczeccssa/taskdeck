#!/usr/bin/env sh
# Exercise the curl | bash release installer against a local fake release.
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
REPO_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
IMAGE=${1:-taskdeck:install-smoke}
if [ "$#" -eq 0 ]; then
    docker build -t "$IMAGE" "$REPO_ROOT"
fi

version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$REPO_ROOT/Cargo.toml" | head -1)
[ -n "$version" ] || { printf '%s\n' 'could not read Cargo.toml version' >&2; exit 1; }
machine=$(docker run --rm --entrypoint uname "$IMAGE" -m)
case "$machine" in
    x86_64|amd64) target=x86_64-unknown-linux-gnu ;;
    aarch64|arm64) target=aarch64-unknown-linux-gnu ;;
    *) printf 'unsupported Docker architecture: %s\n' "$machine" >&2; exit 1 ;;
esac

run_id=$$
network="taskdeck-install-smoke-$run_id"
fixture_container="taskdeck-install-fixture-$run_id"
source_container="taskdeck-install-source-$run_id"
fixture_dir=$(mktemp -d "${TMPDIR:-/tmp}/taskdeck-install-smoke.XXXXXX")
archive_name="taskdeck-v${version}-${target}.tar.gz"
archive_dir="$fixture_dir/releases/download/v$version"
mkdir -p "$archive_dir/payload"

cleanup() {
    docker rm -f "$fixture_container" "$source_container" >/dev/null 2>&1 || true
    docker network rm "$network" >/dev/null 2>&1 || true
    rm -rf "$fixture_dir"
}
trap cleanup EXIT HUP INT TERM

docker create --name "$source_container" "$IMAGE" >/dev/null
docker cp "$source_container:/usr/local/bin/taskdeck" "$archive_dir/payload/taskdeck"
docker rm "$source_container" >/dev/null
tar -czf "$archive_dir/$archive_name" -C "$archive_dir/payload" taskdeck
if command -v sha256sum >/dev/null 2>&1; then
    (cd "$archive_dir" && sha256sum "$archive_name" > SHA256SUMS)
else
    (cd "$archive_dir" && shasum -a 256 "$archive_name" > SHA256SUMS)
fi
cp "$SCRIPT_DIR/install.sh" "$archive_dir/install.sh"
cat > "$fixture_dir/server.py" <<'PY'
import http.server
import os

version = os.environ["TASKDECK_FIXTURE_VERSION"]

class Handler(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory="/srv", **kwargs)

    def do_GET(self):
        if self.path == "/releases/latest":
            self.send_response(302)
            self.send_header("Location", f"/releases/tag/{version}")
            self.end_headers()
            return
        if self.path == "/install.sh":
            self.path = f"/releases/download/{version}/install.sh"
            return super().do_GET()
        if self.path == f"/releases/tag/{version}":
            self.send_response(200)
            self.end_headers()
            self.wfile.write(version.encode())
            return
        super().do_GET()

http.server.ThreadingHTTPServer(("0.0.0.0", 8080), Handler).serve_forever()
PY

docker network create "$network" >/dev/null
docker run -d --name "$fixture_container" --network "$network" --network-alias release-fixture \
    --mount "type=bind,src=$fixture_dir,dst=/srv,readonly" \
    --entrypoint python3 -e "TASKDECK_FIXTURE_VERSION=v$version" "$IMAGE" /srv/server.py >/dev/null
sleep 1

printf 'Testing curl | bash install for v%s (%s)...\n' "$version" "$target"
docker run --rm --network "$network" --entrypoint sh \
    -e "TASKDECK_RELEASES_URL=http://release-fixture:8080/releases" \
    -e "TASKDECK_EXPECTED_VERSION=$version" \
    "$IMAGE" -c 'curl -fsSL http://release-fixture:8080/install.sh | bash -s -- --install-dir /tmp/taskdeck-bin && /tmp/taskdeck-bin/taskdeck --version | grep -F "taskdeck $TASKDECK_EXPECTED_VERSION"'

printf '%064d  %s\n' 0 "$archive_name" > "$archive_dir/SHA256SUMS"
printf '%s\n' 'Testing that a checksum mismatch is rejected before installation...'
docker run --rm --network "$network" --entrypoint sh \
    -e "TASKDECK_RELEASES_URL=http://release-fixture:8080/releases" \
    -e "TASKDECK_VERSION=v$version" \
    "$IMAGE" -c 'if curl -fsSL http://release-fixture:8080/install.sh | bash -s -- --install-dir /tmp/taskdeck-rejected; then echo "installer accepted a bad checksum" >&2; exit 1; fi; test ! -e /tmp/taskdeck-rejected/taskdeck'

printf '%s\n' 'Docker installer smoke passed.'
