#!/bin/sh
#
# strop installer
#
# Usage:
#   curl -fsSL https://strop.dev/install.sh | sh
#
# Environment variables:
#   STROP_VERSION      Version to install (default: latest)
#   STROP_INSTALL_DIR  Where to install the binary (default: $HOME/.local/bin)
#   STROP_INSTALL_YES  Skip the confirmation prompt (default: prompt if a tty is attached)
#   STROP_BASE_URL     Release download base URL (default: GitHub releases; for mirrors/testing)

set -e

REPO="stropdev/strop"
BIN="strop"
INSTALL_DIR="${STROP_INSTALL_DIR:-$HOME/.local/bin}"
BASE_URL="${STROP_BASE_URL:-https://github.com/$REPO/releases/download}"

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

# Resolve version
VERSION="${STROP_VERSION:-}"
if [ -z "$VERSION" ]; then
    info "Resolving latest release..."
    RELEASES_URL="https://api.github.com/repos/${REPO}/releases/latest"
    TAG=$(fetch "$RELEASES_URL" | grep -o '"tag_name": *"[^"]*"' | head -1 | sed 's/.*"\(v[^"]*\)".*/\1/')
    [ -n "$TAG" ] || err "could not resolve latest release from $RELEASES_URL"
    VERSION="${TAG#v}"
fi
# Normalize: strip leading v if the user passed v0.1.1
VERSION="${VERSION#v}"

ARCHIVE="${BIN}-${VERSION}-${TARGET}.tar.gz"
URL="${BASE_URL}/v${VERSION}/${ARCHIVE}"

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
trap 'rm -rf "$TMP"' EXIT

fetch_to "$URL" "$TMP/$ARCHIVE"

# Verify the checksum; macOS has no sha256sum — fall back to shasum
# rather than silently skipping verification.
if fetch_to "${URL}.sha256" "$TMP/$ARCHIVE.sha256" 2>/dev/null; then
    if command -v sha256sum >/dev/null 2>&1; then
        (cd "$TMP" && sha256sum -c "$ARCHIVE.sha256") || err "checksum mismatch — aborting"
    elif command -v shasum >/dev/null 2>&1; then
        (cd "$TMP" && shasum -a 256 -c "$ARCHIVE.sha256") || err "checksum mismatch — aborting"
    fi
fi

info "Extracting..."
tar -xzf "$TMP/$ARCHIVE" -C "$TMP"

info "Installing to $INSTALL_DIR..."
mkdir -p "$INSTALL_DIR"
install -m 0755 "$TMP/${BIN}-${VERSION}-${TARGET}/$BIN" "$INSTALL_DIR/$BIN" 2>/dev/null \
    || install -m 0755 "$TMP/$BIN" "$INSTALL_DIR/$BIN" 2>/dev/null \
    || cp "$TMP/${BIN}-${VERSION}-${TARGET}/$BIN" "$INSTALL_DIR/$BIN"

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
