#!/bin/sh
# Exercise the actual package/draft/publish entrypoints without Docker or GitHub.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
W=$(mktemp -d "${TMPDIR:-/tmp}/strop-candidate-test.XXXXXX")
trap 'rm -rf "$W"' EXIT HUP INT TERM
export W HOME="$W/home" GIT_CONFIG_NOSYSTEM=1 PATH="$W/bin:$PATH"
VERSION=9.9.9
TAG=v$VERSION
PKG=strop-$VERSION-x86_64-unknown-linux-musl
export PKG TAG
mkdir -p "$HOME" "$W/bin" "$W/repo/.github/scripts" "$W/repo/tests" "$W/export/$PKG"
cd "$W/repo"
cp "$ROOT"/.github/scripts/release-local.sh "$ROOT"/.github/scripts/release-catalog.py \
    "$ROOT"/.github/scripts/changelog-notes.py .github/scripts/
# The tiny repository has no other suites (and must not recursively run this one).
for suite in release-catalog release-candidate install; do
    printf 'echo "fixture gate: %s"\n' "$suite" > "tests/$suite.sh"
done
printf '/dist/\n' > .gitignore
printf '[workspace.package]\nversion = "%s"\n' "$VERSION" > Cargo.toml
printf '## %s\n\nFixture release notes.\n' "$VERSION" > CHANGELOG.md
cat > strop <<'BIN'
#!/bin/sh
case "$1" in
    --version) echo 'strop 9.9.9' ;;
    headless) cat "$3" ;;
    bench) echo "fixture bench: $2" ;;
    *) exit 1 ;;
esac
BIN
chmod +x strop
# Compose exports a real tarball and sidecar, containing this checkout's payload.
cat > "$W/bin/docker" <<'DOCKER'
#!/bin/sh
set -eu
for last do :; done
case "$last" in
    test|proofs) echo "fixture compose: $last" ;;
    release)
        git rev-parse HEAD >> "$W/builds"
        cp strop "$W/export/$PKG/strop"
        cd "$STROP_DIST"
        tar -czf "$PKG.tar.gz" -C "$W/export" "$PKG"
        sha256sum "$PKG.tar.gz" > "$PKG.tar.gz.sha256" ;;
    *) exit 1 ;;
esac
DOCKER
# Only ELF inspection is simulated; extraction, catalog, notes, checksums,
# git object and remote tag lookups and all byte comparisons are real.
printf '#!/bin/sh\necho "fixture: statically linked"\n' > "$W/bin/file"
printf '#!/bin/sh\necho "fixture: no dynamic section"\n' > "$W/bin/readelf"
cat > "$W/bin/gh" <<'GH'
#!/bin/sh
set -eu
op="$1 $2"; shift 2
case "$op" in
    'run list') echo success ;;
    'release create')
        mkdir "$W/assets"
        shift # tag
        while [ "$#" -gt 0 ]; do
            case "$1" in
                --notes-file) cp "$2" "$W/body"; shift 2 ;;
                --title|--repo) shift 2 ;;
                --verify-tag|--draft) shift ;;
                *) cp "$1" "$W/assets/"; shift ;;
            esac
        done ;;
    'release view')
        if [ -f "$W/body" ]; then
            python3 -c 'import json, sys; print(json.dumps({"isDraft": True, "body": open(sys.argv[1]).read()}))' "$W/body"
        else
            exit 1
        fi ;;
    'release download')
        while [ "$1" != --dir ]; do shift; done
        cp "$W/assets/"* "$2/" ;;
    'release edit') touch "$W/published" ;;
    *) exit 1 ;;
esac
GH
chmod +x "$W/bin/"*
git init -q
git config user.name 'Release Fixture'
git config user.email 'release@fixture.invalid'
git init -q --bare "$W/origin"
git remote add origin "$W/origin"
git add . && git commit -qm A
A=$(git rev-parse HEAD)
release() { sh .github/scripts/release-local.sh "$1" "$VERSION"; }
refuse() {
    if release "$1" > "$W/refusal" 2>&1; then
        echo "unexpected success: $1 ($2)" >&2; exit 1
    fi
}

release package
set -- "dist/$TAG/$A/"*
A_DIR=$1
cmp strop "$W/export/$PKG/strop"
tar -xOzf "$A_DIR/$PKG.tar.gz" "$PKG/strop" | cmp - strop
# Also leave an old-style archive + valid sidecar directly under this version.
cp "$A_DIR/$PKG.tar.gz" "$A_DIR/$PKG.tar.gz.sha256" "dist/$TAG/"
printf '# candidate B source correction\n' >> strop
git add strop && git commit -qm B
B=$(git rev-parse HEAD)
git tag "$TAG"
git push -q origin "$TAG"
refuse draft 'only A has been packaged'
[ ! -e "$W/assets" ]
# Even copying the entire A bundle under the tagged commit cannot promote it.
cp -r "dist/$TAG/$A" "dist/$TAG/$B"
refuse draft 'copied A bundle is not B'
mv "dist/$TAG/$B" "$W/copied-A"

release package
set -- "dist/$TAG/$B/"*
B_DIR=$1
tar -xOzf "$B_DIR/$PKG.tar.gz" "$PKG/strop" | cmp - strop
for suffix in tar.gz tar.gz.sha256; do
    cmp "$A_DIR/$PKG.$suffix" "dist/$TAG/$PKG.$suffix"
done
refuse package 'same candidate outputs are preserved'
git checkout -q --detach "$A"
refuse draft 'remote tag points to B, not A'
git checkout -q --detach "$B"
# A self-consistent replacement tarball/sidecar still is not the qualified one.
cp -r "$B_DIR" "$W/saved-B"
cp "$A_DIR/$PKG.tar.gz" "$A_DIR/$PKG.tar.gz.sha256" "$B_DIR/"
refuse draft 'modified local bytes'
[ ! -e "$W/assets" ]
cp "$W/saved-B/"* "$B_DIR/"
release draft
for asset in "$PKG.tar.gz" "$PKG.tar.gz.sha256" catalog.json; do
    cmp "$B_DIR/$asset" "$W/assets/$asset"
done
cmp "$B_DIR/release-notes.md" "$W/body"
cp "$A_DIR/$PKG.tar.gz" "$W/assets/"
refuse publish 'remote archive is A'
[ ! -e "$W/published" ]
cp "$B_DIR/$PKG.tar.gz" "$W/assets/"
touch "$W/assets/unexpected"
refuse publish 'unexpected asset'
[ ! -e "$W/published" ]
rm "$W/assets/unexpected"
printf 'wrong body\n' > "$W/body"
refuse publish 'changed release notes'
[ ! -e "$W/published" ]
cp "$B_DIR/release-notes.md" "$W/body"
release publish
[ -f "$W/published" ]
printf '%s\n' "$A" "$B" | cmp - "$W/builds"
echo 'release-candidate fixtures: all passed'
