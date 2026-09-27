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
    Linux/aarch64|Linux/arm64) TARGET="aarch64-unknown-linux-musl" ;;
    Darwin/arm64) TARGET="aarch64-apple-darwin" ;;
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
if [ "$TARGET" = "aarch64-apple-darwin" ]; then
    FOREIGN_TARGET="x86_64-unknown-linux-musl"
else
    FOREIGN_TARGET="aarch64-apple-darwin"
fi
mkdir -p "$WORK/foreign-dist"
pkg="$WORK/pkg-foreign"
mkdir -p "$pkg/strop-$NEW_VERSION-$FOREIGN_TARGET"
printf 'fake foreign binary\n' > "$pkg/strop-$NEW_VERSION-$FOREIGN_TARGET/strop"
tar -czf "$WORK/foreign-dist/strop-$NEW_VERSION-$FOREIGN_TARGET.tar.gz" -C "$pkg" \
    "strop-$NEW_VERSION-$FOREIGN_TARGET"
digest=$(sha "$WORK/foreign-dist/strop-$NEW_VERSION-$FOREIGN_TARGET.tar.gz")
printf '%s  %s\n' "$digest" "strop-$NEW_VERSION-$FOREIGN_TARGET.tar.gz" \
    > "$WORK/foreign-dist/strop-$NEW_VERSION-$FOREIGN_TARGET.tar.gz.sha256"
python3 "$ROOT/.github/scripts/release-catalog.py" catalog \
    --tag "v$NEW_VERSION" --dist "$WORK/foreign-dist" \
    --base-url "file://$LATEST" \
    --published-at "2026-01-01T00:00:00Z" \
    --worker-protocol 1 --worker-min-editor "$NEW_VERSION" \
    --out "$WORK/foreign-catalog.json" >/dev/null
if run_install "$WORK/bin6" "$WORK/out6.log" \
    STROP_CATALOG_URL="file://$WORK/foreign-catalog.json"; then
    fail "foreign-target catalog unexpectedly installed"
fi
[ ! -e "$WORK/bin6/strop" ] || fail "installed from a foreign-target catalog"
ok "catalog without a matching artifact refuses to install"

# --- 7. Intel macOS refuses before any fetch or installation --------------
mkdir -p "$WORK/shim-intel"
cat > "$WORK/shim-intel/uname" <<'UNAME'
#!/bin/sh
case "$1" in
    -s) printf 'Darwin\n' ;;
    -m) printf '%s\n' "$STROP_TEST_MACHINE" ;;
    *) exit 1 ;;
esac
UNAME
cat > "$WORK/shim-intel/curl" <<'FETCH'
#!/bin/sh
: > "$STROP_TEST_FETCHED"
exit 1
FETCH
cp "$WORK/shim-intel/curl" "$WORK/shim-intel/wget"
chmod +x "$WORK/shim-intel/uname" "$WORK/shim-intel/curl" "$WORK/shim-intel/wget"
for machine in x86_64 amd64; do
    if run_install "$WORK/intel-$machine" "$WORK/intel-$machine.log" \
        PATH="$WORK/shim-intel:$PATH" STROP_TEST_MACHINE="$machine" \
        STROP_TEST_FETCHED="$WORK/fetched-$machine"; then
        fail "Intel macOS unexpectedly installed for $machine"
    fi
    [ ! -e "$WORK/fetched-$machine" ] || fail "Intel macOS attempted a download"
    [ ! -e "$WORK/intel-$machine" ] || fail "Intel macOS created an install directory"
    if env PATH="$WORK/shim-intel:$PATH" STROP_TEST_MACHINE="$machine" \
        STROP_TEST_FETCHED="$WORK/fetched-zig-$machine" \
        sh "$ROOT/.github/scripts/install-zig.sh" "$WORK/zig-$machine" \
        >"$WORK/zig-$machine.log" 2>&1; then
        fail "Intel macOS unexpectedly provisioned a compiler"
    fi
    [ ! -e "$WORK/fetched-zig-$machine" ] || fail "Intel macOS downloaded a compiler"
    [ ! -e "$WORK/zig-$machine" ] || fail "Intel macOS created a compiler directory"
done
ok "Intel macOS refuses before fetching or writing"

echo "install.sh fixtures: all passed"
