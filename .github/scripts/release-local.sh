#!/bin/sh
# Local Linux/WSL release entrypoint (plans/0079, plans/0086 §8).
# qualify: every blocking gate on the exact clean source, including the
# GitHub ci run for HEAD and the local-only Lean proofs gate.
# package: builds into a fresh directory and gives it its qualified name
# only after the extracted bytes pass the gates. Promotion never rebuilds.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$ROOT"

COMMAND=${1:?"usage: release-local.sh qualify|package|draft|publish|verify VERSION"}
VERSION=${2#v}
TAG="v$VERSION"
TARGET="x86_64-unknown-linux-musl"
COMMIT=$(git rev-parse HEAD)
CANDIDATE="$ROOT/dist/$TAG/$COMMIT"
ARCHIVE="strop-$VERSION-$TARGET.tar.gz"
REPO="stropdev/strop"

case "$VERSION" in
    ''|*[!0-9.]*|*.*.*.*|*..*) echo "invalid release version: $VERSION" >&2; exit 2 ;;
esac
case "$COMMAND" in
    qualify|package|draft|publish|verify) ;;
    *) echo "unknown release command: $COMMAND" >&2; exit 2 ;;
esac

clean_source() {
    if [ "$(git rev-parse HEAD)" != "$COMMIT" ]; then
        echo "checkout moved away from release candidate $COMMIT" >&2
        exit 1
    fi
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

# Bind all four outputs, including the release body, to the source. The
# directory name is local only; neither a receipt nor a new public asset.
bundle_id() (
    cd "$1"
    sums=$(sha256sum "$ARCHIVE" "$ARCHIVE.sha256" catalog.json release-notes.md)
    digest=$(printf '%s\n' "$COMMIT" "$sums" | sha256sum)
    printf '%s\n' "${digest%% *}"
)

qualified_bundle() {
    set -- "$CANDIDATE"/*
    if [ "$#" -ne 1 ] || [ ! -d "$1" ]; then
        echo "no unique qualified bundle for $TAG at $COMMIT; run package" >&2
        exit 1
    fi
    DIST=$1
    expected=$(bundle_id "$DIST")
    if [ "${DIST##*/}" != "$expected" ]; then
        echo "release bytes do not match the qualified bundle: $DIST" >&2
        exit 1
    fi
}

tagged_bundle() {
    clean_source
    qualified_bundle
    remote_commit=$(git ls-remote origin "refs/tags/$TAG^{}" | cut -f1)
    if [ -z "$remote_commit" ]; then
        remote_commit=$(git ls-remote origin "refs/tags/$TAG" | cut -f1)
    fi
    if [ "$remote_commit" != "$COMMIT" ]; then
        echo "remote tag $TAG does not point at qualified HEAD $COMMIT" >&2
        exit 1
    fi
}

qualify() {
    clean_source
    conclusion=$(gh run list --repo "$REPO" --workflow ci.yml --commit "$COMMIT" \
        --json conclusion --jq '.[0].conclusion // "missing"')
    if [ "$conclusion" != "success" ]; then
        echo "ci for $COMMIT is '$conclusion', not success — push and wait for green" >&2
        exit 1
    fi
    sh tests/release-catalog.sh
    sh tests/release-candidate.sh
    sh tests/install.sh
    docker compose run --build --rm test
    docker compose run --build --rm proofs
    clean_source
}

case "$COMMAND" in
qualify)
    qualify
    ;;
package)
    if [ -e "$CANDIDATE" ]; then
        echo "need a fresh candidate directory; move existing outputs aside: $CANDIDATE" >&2
        exit 1
    fi
    qualify
    mkdir -p "$(dirname "$CANDIDATE")"
    mkdir "$CANDIDATE"
    DIST="$CANDIDATE/.building"
    mkdir "$DIST"
    STROP_DIST="$DIST" docker compose run --build --rm \
        -e "VERSION=$VERSION" -e "TARGET=$TARGET" release
    (cd "$DIST" && sha256sum -c "$ARCHIVE.sha256")

    WORK=$(mktemp -d "${TMPDIR:-/tmp}/strop-release-$VERSION.XXXXXX")
    trap 'rm -rf "$WORK"' EXIT HUP INT TERM
    tar -xzf "$DIST/$ARCHIVE" -C "$WORK"
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
    "$BINARY" headless "$WORK/smoke.txt" "$WORK/smoke.rs" | grep -q "fn main"
    # The find budgets (plans/0093 §7): finder paint and per-key list,
    # search first hit and completion; the symbols budgets (plans/0099
    # §7); the history budgets (plans/0101 §5.1, needs git); and the
    # in-buffer search and large-file open budgets (2026-11-28 D4, D7), and the
    # structural-indent budget (2026-11-28 D2).
    # Each exits non-zero past its budget.
    TMPDIR="$WORK" "$BINARY" bench find
    TMPDIR="$WORK" "$BINARY" bench search
    TMPDIR="$WORK" "$BINARY" bench symbols
    TMPDIR="$WORK" "$BINARY" bench history
    TMPDIR="$WORK" "$BINARY" bench nav_100k
    TMPDIR="$WORK" "$BINARY" bench search_500mb
    TMPDIR="$WORK" "$BINARY" bench open_100mb
    TMPDIR="$WORK" "$BINARY" bench indent
    python3 .github/scripts/release-catalog.py catalog \
        --tag "$TAG" --dist "$DIST" --out "$DIST/catalog.json"
    python3 .github/scripts/changelog-notes.py "$TAG" > "$DIST/release-notes.md"
    clean_source
    id=$(bundle_id "$DIST")
    mv -T "$DIST" "$CANDIDATE/$id"
    echo "qualified local release directory: $CANDIDATE/$id"
    ;;
draft)
    tagged_bundle
    gh release create "$TAG" --repo "$REPO" --verify-tag --draft \
        --title "strop $TAG" --notes-file "$DIST/release-notes.md" \
        "$DIST/$ARCHIVE" "$DIST/$ARCHIVE.sha256" "$DIST/catalog.json"
    gh release view "$TAG" --repo "$REPO"
    ;;
publish)
    tagged_bundle
    gh release view "$TAG" --repo "$REPO" --json isDraft,body | python3 -c '
import json, pathlib, sys
release = json.load(sys.stdin)
if not release["isDraft"]:
    sys.exit("release is not an existing draft")
if release["body"].encode("utf-8") != pathlib.Path(sys.argv[1]).read_bytes():
    sys.exit("draft release notes differ from the qualified bundle")
' "$DIST/release-notes.md"
    WORK=$(mktemp -d "${TMPDIR:-/tmp}/strop-draft-$VERSION.XXXXXX")
    trap 'rm -rf "$WORK"' EXIT HUP INT TERM
    gh release download "$TAG" --repo "$REPO" --dir "$WORK"
    for asset in "$ARCHIVE" "$ARCHIVE.sha256" catalog.json; do
        cmp "$DIST/$asset" "$WORK/$asset"
        rm "$WORK/$asset"
    done
    # All three assets matched, and no unexpected assets may go public.
    rmdir "$WORK"
    gh release edit "$TAG" --repo "$REPO" --draft=false --latest
    ;;
verify)
    qualified_bundle
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
