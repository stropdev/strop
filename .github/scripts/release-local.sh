#!/bin/sh
# Local Linux/WSL release entrypoint (plans/0079, plans/0086 §8).
# qualify: every blocking gate on the exact clean source, including the
# GitHub ci run for HEAD and the local-only Lean proofs gate.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$ROOT"

COMMAND=${1:?"usage: release-local.sh qualify|package|draft|publish|verify VERSION"}
VERSION=${2#v}
TAG="v$VERSION"
TARGET="x86_64-unknown-linux-musl"
DIST="$ROOT/dist/$TAG"
ARCHIVE="$DIST/strop-$VERSION-$TARGET.tar.gz"
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
    python3 .github/scripts/release-catalog.py catalog \
        --tag "$TAG" --dist "$DIST" --out "$DIST/catalog.json"
}

case "$COMMAND" in
qualify)
    clean_source
    head=$(git rev-parse HEAD)
    conclusion=$(gh run list --repo "$REPO" --workflow ci.yml --commit "$head" \
        --json conclusion --jq '.[0].conclusion // "missing"')
    if [ "$conclusion" != "success" ]; then
        echo "ci for $head is '$conclusion', not success — push and wait for green" >&2
        exit 1
    fi
    sh tests/release-catalog.sh
    sh tests/install.sh
    docker compose run --build --rm test
    docker compose run --build --rm model
    docker compose run --build --rm proofs
    ;;
package)
    clean_source
    mkdir -p "$DIST"
    if [ -e "$ARCHIVE" ]; then
        echo "reusing existing artifact without rebuilding: $ARCHIVE"
    else
        STROP_DIST="$DIST" docker compose run --build --rm \
            -e "VERSION=$VERSION" -e "TARGET=$TARGET" release
    fi
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
    # The find budgets (plans/0093 §7): finder paint and per-key list,
    # search first hit and completion. Either exits non-zero past its budget.
    TMPDIR="$WORK" "$BINARY" --bench find
    TMPDIR="$WORK" "$BINARY" --bench search
    catalog
    python3 .github/scripts/changelog-notes.py "$TAG" > "$DIST/release-notes.md"
    echo "qualified local release directory: $DIST"
    ;;
draft)
    clean_source
    [ -f "$ARCHIVE" ] && [ -f "$ARCHIVE.sha256" ] \
        && [ -f "$DIST/catalog.json" ] && [ -f "$DIST/release-notes.md" ]
    (cd "$DIST" && sha256sum -c "$(basename "$ARCHIVE").sha256")
    local_commit=$(git rev-parse HEAD)
    remote_commit=$(git ls-remote origin "refs/tags/$TAG^{}" | cut -f1)
    if [ -z "$remote_commit" ]; then
        remote_commit=$(git ls-remote origin "refs/tags/$TAG" | cut -f1)
    fi
    if [ -z "$remote_commit" ] || [ "$local_commit" != "$remote_commit" ]; then
        echo "remote tag $TAG does not point at qualified HEAD $local_commit" >&2
        exit 1
    fi
    gh release create "$TAG" --repo "$REPO" --verify-tag --draft \
        --title "strop $TAG" --notes-file "$DIST/release-notes.md" \
        "$ARCHIVE" "$ARCHIVE.sha256" "$DIST/catalog.json"
    gh release view "$TAG" --repo "$REPO"
    ;;
publish)
    clean_source
    (cd "$DIST" && sha256sum -c "$(basename "$ARCHIVE").sha256")
    local_commit=$(git rev-parse HEAD)
    remote_commit=$(git ls-remote origin "refs/tags/$TAG^{}" | cut -f1)
    if [ -z "$remote_commit" ]; then
        remote_commit=$(git ls-remote origin "refs/tags/$TAG" | cut -f1)
    fi
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
