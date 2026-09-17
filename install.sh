#!/bin/sh
#
# strop installer
#
# Usage:
#   curl -fsSL https://strop.dev/install.sh | sh
#
# Environment variables:
#   STROP_VERSION      Version to install (default: latest, resolved through the release catalog)
#   STROP_INSTALL_DIR  Where to install the binary (default: $HOME/.local/bin)
#   STROP_INSTALL_YES  Skip the confirmation prompt (default: prompt if a tty is attached)
#   STROP_BASE_URL     Release download base URL for pinned versions (default: GitHub releases)
#   STROP_CATALOG_URL  Release catalog URL for latest resolution (default: GitHub releases)

set -e

REPO="stropdev/strop"
BIN="strop"
INSTALL_DIR="${STROP_INSTALL_DIR:-$HOME/.local/bin}"
BASE_URL="${STROP_BASE_URL:-https://github.com/$REPO/releases/download}"
CATALOG_URL="${STROP_CATALOG_URL:-https://github.com/$REPO/releases/latest/download/catalog.json}"
RECEIPT_NAME=".strop-install.json"

# Color setup: only emit escapes when stdout is a tty and NO_COLOR is unset
if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
    BOLD=$(printf '\033[1m')
    RED=$(printf '\033[31m')
    GREEN=$(printf '\033[32m')
    CYAN=$(printf '\033[36m')
    RESET=$(printf '\033[0m')
else
    BOLD=""
    RED=""
    GREEN=""
    CYAN=""
    RESET=""
fi

err() {
    printf '%sError:%s %s\n' "$RED$BOLD" "$RESET" "$1" >&2
    exit 1
}

info() {
    printf '%s\n' "$1"
}

fetch() {
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$1"
    elif command -v wget >/dev/null 2>&1; then
        wget -qO- "$1"
    else
        err "neither curl nor wget found"
    fi
}

fetch_to() {
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL -o "$2" "$1"
    elif command -v wget >/dev/null 2>&1; then
        wget -qO "$2" "$1"
    else
        err "neither curl nor wget found"
    fi
}

# Detect OS/arch — the release matrix (plan 0002 §1).
OS=$(uname -s)
ARCH=$(uname -m)
case "$OS/$ARCH" in
    Linux/x86_64|Linux/amd64)
        TARGET="x86_64-unknown-linux-musl"
        ;;
    Linux/aarch64|Linux/arm64)
        TARGET="aarch64-unknown-linux-musl"
        ;;
    Darwin/x86_64)
        TARGET="x86_64-apple-darwin"
        ;;
    Darwin/arm64)
        TARGET="aarch64-apple-darwin"
        ;;
    *)
        err "no prebuilt binary for $OS/$ARCH — try: brew install stropdev/tap/strop (builds from source) or cargo install strop-editor --locked"
        ;;
esac

# Resolve version + artifact facts. "latest" goes through the generated
# release catalog (0056 AR12) — the single source of truth for version,
# artifact names and digests, shared with the updater and the site; no
# independent API scraping. A pinned STROP_VERSION falls back to the
# release's own checksum sidecar.
VERSION="${STROP_VERSION:-}"
DIGEST=""
if [ -z "$VERSION" ]; then
    info "Resolving latest release..."
    CATALOG=$(fetch "$CATALOG_URL") || err "could not fetch release catalog at $CATALOG_URL"
    [ -n "$CATALOG" ] || err "empty release catalog at $CATALOG_URL"
    VERSION=$(printf '%s\n' "$CATALOG" | sed -n 's/^  "version": "\([^"]*\)".*$/\1/p' | head -1)
    [ -n "$VERSION" ] || err "no version in release catalog at $CATALOG_URL"
    # The catalog pretty-prints one object per artifact, one field per
    # line; extract name/sha256/url from the block for this target.
    FACTS=$(printf '%s\n' "$CATALOG" | awk -v target="$TARGET" '
        $0 ~ "\"target\": \"" target "\"" { found = 1 }
        found && /"name": "/   { v = $0; sub(/.*"name": "/, "", v);   sub(/".*/, "", v); name = v }
        found && /"sha256": "/ { v = $0; sub(/.*"sha256": "/, "", v); sub(/".*/, "", v); sha = v }
        found && /"url": "/    { v = $0; sub(/.*"url": "/, "", v);    sub(/".*/, "", v); url = v }
        found && name && sha && url { print name; print sha; print url; exit }
    ')
    ARCHIVE=$(printf '%s\n' "$FACTS" | sed -n '1p')
    DIGEST=$(printf '%s\n' "$FACTS" | sed -n '2p')
    URL=$(printf '%s\n' "$FACTS" | sed -n '3p')
    { [ -n "$ARCHIVE" ] && [ -n "$DIGEST" ] && [ -n "$URL" ]; } \
        || err "release catalog has no artifact for $TARGET"
else
    # Normalize: strip leading v if the user passed v0.1.1
    VERSION="${VERSION#v}"
    ARCHIVE="${BIN}-${VERSION}-${TARGET}.tar.gz"
    URL="${BASE_URL}/v${VERSION}/${ARCHIVE}"
fi

# Show install plan and confirm
info ""
info "${BOLD}About to install:${RESET}"
info "  Package:  $BIN ${CYAN}$VERSION${RESET}"
info "  Target:   $TARGET"
info "  Source:   $URL"
info "  Dest:     $INSTALL_DIR/$BIN"
info ""

if [ -z "$STROP_INSTALL_YES" ] && [ -r /dev/tty ]; then
    printf '%sContinue? [Y/n]%s ' "$BOLD" "$RESET"
    read -r answer </dev/tty
    case "$answer" in
        ""|y|Y|yes|YES|Yes) ;;
        *) err "aborted by user" ;;
    esac
fi

info "Downloading ${ARCHIVE}..."

TMP=$(mktemp -d)
STAGED=""
trap 'rm -rf "$TMP"; [ -z "$STAGED" ] || rm -f "$STAGED"' EXIT
trap 'exit 1' HUP INT TERM

fetch_to "$URL" "$TMP/$ARCHIVE"

# Verify the checksum — mandatory (0023: install and update share one
# policy; unverified bytes never install). The digest comes from the
# release catalog for latest, or from the release's checksum sidecar for
# a pinned version; a missing sidecar or checker aborts.
if [ -n "$DIGEST" ]; then
    printf '%s  %s\n' "$DIGEST" "$ARCHIVE" > "$TMP/$ARCHIVE.sha256"
elif ! fetch_to "${URL}.sha256" "$TMP/$ARCHIVE.sha256"; then
    err "no checksum sidecar at ${URL}.sha256 — refusing to install unverified"
fi
if command -v sha256sum >/dev/null 2>&1; then
    (cd "$TMP" && sha256sum -c "$ARCHIVE.sha256") || err "checksum mismatch — aborting"
elif command -v shasum >/dev/null 2>&1; then
    (cd "$TMP" && shasum -a 256 -c "$ARCHIVE.sha256") || err "checksum mismatch — aborting"
else
    err "no sha256sum/shasum available — refusing to install unverified"
fi

info "Extracting..."
tar -xzf "$TMP/$ARCHIVE" -C "$TMP"
SRC="$TMP/${BIN}-${VERSION}-${TARGET}/$BIN"
[ -f "$SRC" ] || SRC="$TMP/$BIN"
[ -f "$SRC" ] || err "archive did not contain $BIN"

info "Installing to $INSTALL_DIR..."
mkdir -p "$INSTALL_DIR"

# One staged verified transaction (0056 AR11), the same contract the
# native updater keeps: stage in the destination directory (same
# filesystem), verify the staged bytes against the checksum-verified
# extraction, then publish with an atomic rename. Interrupt at any point
# before the rename and any old binary is untouched; the install/cp
# fallback chain is gone.
STAGED=$(mktemp "$INSTALL_DIR/.strop-stage.XXXXXX") || err "cannot stage in $INSTALL_DIR"
cat "$SRC" > "$STAGED" || err "failed to stage $BIN"
chmod 0755 "$STAGED" || err "failed to mark staged $BIN executable"
cmp -s "$SRC" "$STAGED" || err "staged bytes diverged — aborting"
mv -f "$STAGED" "$INSTALL_DIR/$BIN" || err "failed to publish $INSTALL_DIR/$BIN"
STAGED=""

# Installation receipt (0056 AR11): channel/version/root/method facts the
# updater reads instead of guessing from path substrings. Pre-receipt
# installs stay honestly unknown until reinstalled; a replaced install's
# version is remembered as previous_version for recovery/rollback.
RECEIPT="$INSTALL_DIR/$RECEIPT_NAME"
PREVIOUS=""
if [ -f "$RECEIPT" ]; then
    PREVIOUS=$(grep -o '"version": *"[^"]*"' "$RECEIPT" | head -1 | sed 's/.*"\([^"]*\)"$/\1/')
fi
if RSTAGE=$(mktemp "$INSTALL_DIR/.strop-receipt.XXXXXX"); then
    {
        printf '{\n'
        printf '  "schema": 1,\n'
        printf '  "channel": "tarball",\n'
        printf '  "method": "install.sh",\n'
        printf '  "version": "%s",\n' "$VERSION"
        [ -z "$PREVIOUS" ] || printf '  "previous_version": "%s",\n' "$PREVIOUS"
        printf '  "install_root": "%s",\n' "$INSTALL_DIR"
        printf '  "installed_at": %s\n' "$(date +%s)"
        printf '}\n'
    } > "$RSTAGE" && mv -f "$RSTAGE" "$RECEIPT" || {
        rm -f "$RSTAGE"
        info "warning: could not write installation receipt — strop update will report this install as unknown"
    }
else
    info "warning: could not write installation receipt — strop update will report this install as unknown"
fi

info ""
info "${GREEN}${BOLD}strop $VERSION installed${RESET} → $INSTALL_DIR/$BIN"
case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *) info "${CYAN}note:${RESET} $INSTALL_DIR is not on your PATH" ;;
esac
info ""
info "  ${BOLD}strop${RESET}              open the welcome card"
info "  ${BOLD}strop file.rs${RESET}      see the cut before you make it"
info "  ${BOLD}strop update${RESET}       self-update (tarball installs)"
info ""
