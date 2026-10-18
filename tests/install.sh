#!/bin/sh
# Shell-level fixtures for install.sh (0056 AR11/AR12):
#   - "latest" resolves through the generated release catalog
#   - the staged verified transaction preserves any old binary when the
#     install is interrupted (failure at publish, SIGTERM mid-transaction)
#     and leaves no partial state behind
#   - an installation receipt is written, and a replaced install's version
#     is carried forward as previous_version
#   - a digest that does not match the downloaded bytes refuses to install
#   - a catalog without an artifact for this target refuses to install
#   - unsupported platforms refuse before fetch, staging, or install mutation
# Hermetic: everything is served from file:// fixtures, no network.
# Run: sh tests/install.sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
WORK=$(mktemp -d "${TMPDIR:-/tmp}/strop-install-test.XXXXXX")
trap 'rm -rf "$WORK"' EXIT HUP INT TERM

fail() { echo "FAIL: $1" >&2; exit 1; }
ok() { echo "ok: $1"; }

command -v curl >/dev/null 2>&1 || { echo "skip: curl required"; exit 0; }
command -v python3 >/dev/null 2>&1 || { echo "skip: python3 required (catalog fixture)"; exit 0; }

if command -v sha256sum >/dev/null 2>&1; then
    sha() { sha256sum "$1" | cut -d' ' -f1; }
elif command -v shasum >/dev/null 2>&1; then
    sha() { shasum -a 256 "$1" | cut -d' ' -f1; }
else
    echo "skip: sha256sum/shasum required"
    exit 0
fi

# Host target triple, same mapping as install.sh.
case "$(uname -s)/$(uname -m)" in
    Linux/x86_64|Linux/amd64) TARGET="x86_64-unknown-linux-musl" ;;
    *) echo "skip: no prebuilt target for $(uname -s)/$(uname -m)"; exit 0 ;;
esac

OLD_VERSION="9.9.8"
NEW_VERSION="9.9.9"

# Fixture tarballs; the payload text doubles as the version identity.
make_tarball() { # version destdir
    pkg="$WORK/pkg-$1"
    mkdir -p "$pkg/strop-$1-$TARGET" "$2"
    printf 'fake strop binary %s\n' "$1" > "$pkg/strop-$1-$TARGET/strop"
    tar -czf "$2/strop-$1-$TARGET.tar.gz" -C "$pkg" "strop-$1-$TARGET"
    digest=$(sha "$2/strop-$1-$TARGET.tar.gz")
    printf '%s  %s\n' "$digest" "strop-$1-$TARGET.tar.gz" \
        > "$2/strop-$1-$TARGET.tar.gz.sha256"
}

SRV="$WORK/srv"
make_tarball "$OLD_VERSION" "$SRV/v$OLD_VERSION"
make_tarball "$NEW_VERSION" "$SRV/v$NEW_VERSION"

# The real generator builds the catalog fixture from real build outputs;
# the tarball only exists at the URL the catalog records, so a working
# install proves catalog-driven resolution end to end.
LATEST="$SRV/latest/download"
mkdir -p "$LATEST/v$NEW_VERSION" "$WORK/catalog-dist"
cp "$SRV/v$NEW_VERSION/strop-$NEW_VERSION-$TARGET.tar.gz"* "$WORK/catalog-dist/"
cp "$SRV/v$NEW_VERSION/strop-$NEW_VERSION-$TARGET.tar.gz" "$LATEST/v$NEW_VERSION/"
python3 "$ROOT/.github/scripts/release-catalog.py" catalog \
    --tag "v$NEW_VERSION" --dist "$WORK/catalog-dist" \
    --base-url "file://$LATEST" \
    --published-at "2026-01-01T00:00:00Z" \
    --worker-protocol 1 --worker-min-editor "$NEW_VERSION" \
    --out "$LATEST/catalog.json" >/dev/null

run_install() { # installdir logfile [extra env as NAME=VALUE ...]
    dir="$1"
    log="$2"
    shift 2
    env STROP_INSTALL_YES=1 STROP_INSTALL_DIR="$dir" \
        STROP_CATALOG_URL="file://$LATEST/catalog.json" \
        STROP_BASE_URL="file://$SRV" \
        "$@" sh "$ROOT/install.sh" >"$log" 2>&1
}

# --- 1. catalog-driven fresh install -------------------------------------
run_install "$WORK/bin" "$WORK/out1.log" || fail "fresh install: $(cat "$WORK/out1.log")"
[ "$(cat "$WORK/bin/strop")" = "fake strop binary $NEW_VERSION" ] \
    || fail "installed binary is not the catalog artifact"
[ -x "$WORK/bin/strop" ] || fail "installed binary is not executable"
RECEIPT="$WORK/bin/.strop-install.json"
[ -f "$RECEIPT" ] || fail "no installation receipt written"
grep -q '"channel": "tarball"' "$RECEIPT" || fail "receipt channel is not tarball"
grep -q "\"version\": \"$NEW_VERSION\"" "$RECEIPT" || fail "receipt version wrong"
grep -q '"method": "install.sh"' "$RECEIPT" || fail "receipt method wrong"
grep -q "\"install_root\": \"$WORK/bin\"" "$RECEIPT" || fail "receipt install_root wrong"
ok "catalog-driven install + receipt"

# --- 2. failure at publish preserves the old binary, no partial state ----
mkdir -p "$WORK/bin2" "$WORK/shim-fail"
printf 'fake strop binary OLD\n' > "$WORK/bin2/strop"
chmod 755 "$WORK/bin2/strop"
printf '#!/bin/sh\nexit 1\n' > "$WORK/shim-fail/mv"
chmod +x "$WORK/shim-fail/mv"
if run_install "$WORK/bin2" "$WORK/out2.log" PATH="$WORK/shim-fail:$PATH"; then
    fail "publish failure unexpectedly succeeded"
fi
[ "$(cat "$WORK/bin2/strop")" = "fake strop binary OLD" ] \
    || fail "old binary clobbered by failed publish"
if ls -A "$WORK/bin2" | grep -q '^\.strop-'; then
    fail "staging leftovers after failed publish: $(ls -A "$WORK/bin2")"
fi
ok "failed publish preserves old binary, leaves no partial state"

# --- 3. SIGTERM mid-transaction preserves the old binary ------------------
mkdir -p "$WORK/bin3" "$WORK/shim-term"
printf 'fake strop binary OLD\n' > "$WORK/bin3/strop"
chmod 755 "$WORK/bin3/strop"
printf '#!/bin/sh\nkill -TERM "$PPID"\nsleep 5\n' > "$WORK/shim-term/mv"
chmod +x "$WORK/shim-term/mv"
if run_install "$WORK/bin3" "$WORK/out3.log" PATH="$WORK/shim-term:$PATH"; then
    fail "terminated install unexpectedly succeeded"
fi
[ "$(cat "$WORK/bin3/strop")" = "fake strop binary OLD" ] \
    || fail "old binary clobbered by terminated install"
if ls -A "$WORK/bin3" | grep -q '^\.strop-'; then
    fail "staging leftovers after terminated install: $(ls -A "$WORK/bin3")"
fi
ok "terminated install preserves old binary, leaves no partial state"

# --- 4. pinned install, then catalog install carries previous_version -----
run_install "$WORK/bin4" "$WORK/out4.log" STROP_VERSION="$OLD_VERSION" \
    || fail "pinned install: $(cat "$WORK/out4.log")"
[ "$(cat "$WORK/bin4/strop")" = "fake strop binary $OLD_VERSION" ] \
    || fail "pinned install got the wrong artifact"
grep -q "\"version\": \"$OLD_VERSION\"" "$WORK/bin4/.strop-install.json" \
    || fail "pinned install receipt version wrong"
run_install "$WORK/bin4" "$WORK/out4b.log" \
    || fail "catalog install over pinned: $(cat "$WORK/out4b.log")"
[ "$(cat "$WORK/bin4/strop")" = "fake strop binary $NEW_VERSION" ] \
    || fail "catalog install over pinned got the wrong artifact"
RECEIPT4="$WORK/bin4/.strop-install.json"
grep -q "\"version\": \"$NEW_VERSION\"" "$RECEIPT4" || fail "receipt not refreshed"
grep -q "\"previous_version\": \"$OLD_VERSION\"" "$RECEIPT4" \
    || fail "previous_version not carried forward"
ok "pinned install + previous_version carried forward"

# --- 5. digest mismatch refuses to install --------------------------------
ZERO=0000000000000000000000000000000000000000000000000000000000000000
awk -v zero="$ZERO" '
    !done && /"sha256": "/ { sub(/"[0-9a-f]+"/, "\"" zero "\""); done = 1 }
    { print }
' "$LATEST/catalog.json" > "$WORK/bad-catalog.json"
[ -s "$WORK/bad-catalog.json" ] || fail "could not build bad catalog fixture"
if run_install "$WORK/bin5" "$WORK/out5.log" \
    STROP_CATALOG_URL="file://$WORK/bad-catalog.json"; then
    fail "digest mismatch unexpectedly installed"
fi
[ ! -e "$WORK/bin5/strop" ] || fail "unverified bytes were installed"
ok "digest mismatch refuses to install"

# --- 6. catalog without an artifact for this target refuses ---------------
# Deliberately foreign historical catalog, not an active release output.
cat > "$WORK/foreign-catalog.json" <<EOF
{
  "version": "$NEW_VERSION",
  "artifacts": [
    {
      "target": "aarch64-apple-darwin",
      "name": "strop-$NEW_VERSION-aarch64-apple-darwin.tar.gz",
      "sha256": "$ZERO",
      "url": "file://$WORK/foreign.tar.gz"
    }
  ]
}
EOF
if run_install "$WORK/bin6" "$WORK/out6.log" \
    STROP_CATALOG_URL="file://$WORK/foreign-catalog.json"; then
    fail "foreign-target catalog unexpectedly installed"
fi
[ ! -e "$WORK/bin6/strop" ] || fail "installed from a foreign-target catalog"
ok "catalog without a matching artifact refuses to install"

# --- 7. unsupported targets refuse before any fetch or mutation -----------
mkdir -p "$WORK/shim-platform"
cat > "$WORK/shim-platform/uname" <<'UNAME'
#!/bin/sh
case "$1" in
    -s) printf '%s\n' "$STROP_TEST_SYSTEM" ;;
    -m) printf '%s\n' "$STROP_TEST_MACHINE" ;;
    *) exit 1 ;;
esac
UNAME
cat > "$WORK/shim-platform/refuse" <<'REFUSE'
#!/bin/sh
: > "$STROP_TEST_SIDE_EFFECT"
exit 1
REFUSE
chmod +x "$WORK/shim-platform/uname" "$WORK/shim-platform/refuse"
for command in curl wget mktemp mkdir cp mv chmod tar; do
    cp "$WORK/shim-platform/refuse" "$WORK/shim-platform/$command"
done
for platform in Linux/aarch64 Linux/arm64 Linux/i686 Linux/riscv64 \
    Darwin/x86_64 Darwin/amd64 Darwin/arm64 Darwin/aarch64 \
    Windows_NT/x86_64 MINGW64_NT-10.0/x86_64 FreeBSD/x86_64; do
    system=${platform%/*}
    machine=${platform#*/}
    name="$system-$machine"
    mkdir -p "$WORK/existing-$name"
    printf 'old binary\n' > "$WORK/existing-$name/strop"
    printf 'old receipt\n' > "$WORK/existing-$name/.strop-install.json"
    for version in "" "$NEW_VERSION"; do
        for state in fresh existing; do
            if run_install "$WORK/$state-$name" "$WORK/$name.log" \
                PATH="$WORK/shim-platform:$PATH" STROP_TEST_SYSTEM="$system" \
                STROP_TEST_MACHINE="$machine" STROP_VERSION="$version" \
                STROP_TEST_SIDE_EFFECT="$WORK/side-effect-$name"; then
                fail "$platform unexpectedly installed"
            fi
            grep -q "unsupported platform $platform" "$WORK/$name.log" \
                || fail "$platform did not report the unsupported target"
            [ ! -e "$WORK/side-effect-$name" ] || fail "$platform attempted fetch or mutation"
            [ ! -e "$WORK/fresh-$name" ] || fail "$platform created an install directory"
            [ "$(cat "$WORK/existing-$name/strop")" = "old binary" ] \
                || fail "$platform changed the existing binary"
            [ "$(cat "$WORK/existing-$name/.strop-install.json")" = "old receipt" ] \
                || fail "$platform changed the existing receipt"
        done
    done
    if env PATH="$WORK/shim-platform:$PATH" STROP_TEST_SYSTEM="$system" \
        STROP_TEST_MACHINE="$machine" STROP_TEST_SIDE_EFFECT="$WORK/zig-side-effect-$name" \
        sh "$ROOT/.github/scripts/install-zig.sh" "$WORK/zig-$name" \
        >"$WORK/zig-$name.log" 2>&1; then
        fail "$platform unexpectedly provisioned a compiler"
    fi
    [ ! -e "$WORK/zig-side-effect-$name" ] || fail "$platform attempted compiler fetch or mutation"
    [ ! -e "$WORK/zig-$name" ] || fail "$platform created a compiler directory"
done
ok "unsupported targets refuse before fetching or writing"

# Both uname spellings select the same Linux artifact; WSL reports Linux too.
mkdir -p "$WORK/shim-linux"
cp "$WORK/shim-platform/uname" "$WORK/shim-linux/uname"
for machine in x86_64 amd64; do
    run_install "$WORK/linux-$machine" "$WORK/linux-$machine.log" \
        PATH="$WORK/shim-linux:$PATH" STROP_TEST_SYSTEM=Linux STROP_TEST_MACHINE="$machine" \
        || fail "Linux/$machine refused: $(cat "$WORK/linux-$machine.log")"
    [ "$(cat "$WORK/linux-$machine/strop")" = "fake strop binary $NEW_VERSION" ] \
        || fail "Linux/$machine selected the wrong artifact"
done
ok "Linux x86_64 and amd64 select the same artifact"

echo "install.sh fixtures: all passed"
