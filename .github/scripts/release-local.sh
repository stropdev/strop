#!/bin/sh
# Local Linux/WSL release entrypoint (plans/0079-local-linux-release.md).
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$ROOT"

COMMAND=${1:?"usage: release-local.sh qualify|package|draft|publish|verify VERSION"}
VERSION=${2#v}
TAG="v$VERSION"
TARGET="x86_64-unknown-linux-musl"
DIST="$ROOT/dist/$TAG"
ARCHIVE="$DIST/strop-$VERSION-$TARGET.tar.gz"
CANDIDATE="$DIST/completion-$TARGET.json"
FROZEN="$DIST/completion-candidate.json"
REPO="stropdev/strop"

case "$VERSION" in
    ''|*[!0-9.]*|*.*.*.*|*..*) echo "invalid release version: $VERSION" >&2; exit 2 ;;
esac
case "$COMMAND" in
    qualify|package|draft|publish|verify) ;;
    *) echo "unknown release command: $COMMAND" >&2; exit 2 ;;
esac

clean_source() {
    dirty=$(git status --porcelain)
    if [ -n "$dirty" ]; then
        printf '%s\n' "release requires an exact clean checkout:" "$dirty" >&2
        exit 1
    fi
    python3 - "$VERSION" <<'PY'
import sys, tomllib
with open("Cargo.toml", "rb") as stream:
    actual = tomllib.load(stream)["workspace"]["package"]["version"]
if actual != sys.argv[1]:
    raise SystemExit(f"workspace version {actual} does not match release {sys.argv[1]}")
PY
}

catalog() {
    protocol=$(sed -n 's/^pub const PROTOCOL_VERSION: u32 = \([0-9][0-9]*\);$/\1/p' \
        crates/strop-worker-protocol/src/message.rs)
    min_editor=$(sed -n 's/^pub const MIN_EDITOR_VERSION: &str = "\([^"]*\)";$/\1/p' \
        crates/strop-worker-deploy/src/lib.rs)
    [ -n "$protocol" ] && [ -n "$min_editor" ]
    python3 .github/scripts/release-catalog.py catalog \
        --tag "$TAG" --dist "$DIST" --out "$DIST/catalog.json" \
        --worker-protocol "$protocol" --worker-min-editor "$min_editor"
}

case "$COMMAND" in
qualify)
    clean_source
    python3 verification/check.py --release
    sh tests/release-catalog.sh
    sh tests/install.sh
    docker compose run --build --rm test
    ;;
package)
    clean_source
    if [ -e "$ARCHIVE" ]; then
        echo "refusing to rebuild over existing artifact: $ARCHIVE" >&2
        exit 1
    fi
    mkdir -p "$DIST"
    STROP_DIST="$DIST" docker compose run --build --rm \
        -e "VERSION=$VERSION" -e "TARGET=$TARGET" release
    (cd "$DIST" && sha256sum -c "$(basename "$ARCHIVE").sha256")

    WORK=$(mktemp -d "${TMPDIR:-/tmp}/strop-release-$VERSION.XXXXXX")
    trap 'rm -rf "$WORK"' EXIT HUP INT TERM
    tar -xzf "$ARCHIVE" -C "$WORK"
    BINARY="$WORK/strop-$VERSION-$TARGET/strop"
    file "$BINARY"
    file "$BINARY" | grep -qE "static-pie linked|statically linked"
    if readelf -d "$BINARY" | grep -q NEEDED; then
        echo "dynamic dependency in static release" >&2
        exit 1
    fi
    "$BINARY" --version
    printf 'fn main() { let x = 1; }\n' > "$WORK/smoke.rs"
    printf 'frame\n' > "$WORK/smoke.txt"
    "$BINARY" --headless "$WORK/smoke.txt" "$WORK/smoke.rs" | grep -q "fn main"
    timeout 15 "$BINARY" --worker-stdio </dev/null
    python3 -B verification/qualify_completion.py \
        --binary "$BINARY" --profile release-musl-stripped \
        --build "docker compose release; $TARGET" --out "$CANDIDATE"
    python3 verification/freeze.py --out "$FROZEN"
    python3 verification/freeze.py --check --out "$FROZEN"
    catalog
    python3 .github/scripts/changelog-notes.py "$TAG" > "$DIST/release-notes.md"
    echo "qualified local release directory: $DIST"
    ;;
draft)
    clean_source
    [ -f "$ARCHIVE" ] && [ -f "$ARCHIVE.sha256" ] && [ -f "$CANDIDATE" ] \
        && [ -f "$FROZEN" ] && [ -f "$DIST/catalog.json" ] && [ -f "$DIST/release-notes.md" ]
    python3 verification/freeze.py --check --out "$FROZEN"
    local_commit=$(git rev-parse HEAD)
    remote_commit=$(git ls-remote origin "refs/tags/$TAG" | cut -f1)
    if [ -z "$remote_commit" ] || [ "$local_commit" != "$remote_commit" ]; then
        echo "remote tag $TAG does not point at qualified HEAD $local_commit" >&2
        exit 1
    fi
    gh release create "$TAG" --repo "$REPO" --verify-tag --draft \
        --title "strop $TAG" --notes-file "$DIST/release-notes.md" \
        "$ARCHIVE" "$ARCHIVE.sha256" "$CANDIDATE" "$FROZEN" \
        "$DIST/catalog.json"
    gh release view "$TAG" --repo "$REPO"
    ;;
publish)
    clean_source
    python3 verification/freeze.py --check --out "$FROZEN"
    local_commit=$(git rev-parse HEAD)
    remote_commit=$(git ls-remote origin "refs/tags/$TAG" | cut -f1)
    if [ -z "$remote_commit" ] || [ "$local_commit" != "$remote_commit" ]; then
        echo "remote tag $TAG does not point at qualified HEAD $local_commit" >&2
        exit 1
    fi
    if [ "$(gh release view "$TAG" --repo "$REPO" --json isDraft --jq .isDraft)" != "true" ]; then
        echo "release $TAG is not an existing draft" >&2
        exit 1
    fi
    gh release view "$TAG" --repo "$REPO" --json isDraft,assets,url
    gh release edit "$TAG" --repo "$REPO" --draft=false --latest
    ;;
verify)
    WORK=$(mktemp -d "${TMPDIR:-/tmp}/strop-public-$VERSION.XXXXXX")
    trap 'rm -rf "$WORK"' EXIT HUP INT TERM
    curl -fsSL -o "$WORK/catalog.json" \
        "https://github.com/$REPO/releases/latest/download/catalog.json"
    python3 .github/scripts/release-catalog.py verify \
        --local "$DIST/catalog.json" --public "$WORK/catalog.json"
    env STROP_INSTALL_DIR="$WORK/bin" STROP_INSTALL_YES=1 sh install.sh
    "$WORK/bin/strop" --version | grep -q "strop $VERSION"
    "$WORK/bin/strop" --version
    ;;
esac
