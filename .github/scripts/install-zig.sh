#!/bin/sh
# Explicit build-tool provisioning; build.rs never downloads a compiler.
set -eu
if [ "$#" -ne 1 ]; then
    echo "usage: install-zig.sh NEW_INSTALL_DIRECTORY" >&2
    exit 2
fi
destination=$1
if [ -e "$destination" ]; then
    echo "refusing to replace existing Zig directory: $destination" >&2
    exit 1
fi
case "$(uname -s)/$(uname -m)" in
    Linux/x86_64) platform=x86_64-linux; digest=70e49664a74374b48b51e6f3fdfbf437f6395d42509050588bd49abe52ba3d00 ;;
    *) echo "unsupported Zig build host (supported: Linux/WSL x86_64)" >&2; exit 1 ;;
esac
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT HUP INT TERM
archive="$scratch/zig.tar.xz"
curl --fail --location --proto '=https' --tlsv1.2 \
    "https://ziglang.org/download/0.16.0/zig-$platform-0.16.0.tar.xz" -o "$archive"
if command -v sha256sum >/dev/null 2>&1; then
    printf '%s  %s\n' "$digest" "$archive" | sha256sum -c -
else
    printf '%s  %s\n' "$digest" "$archive" | shasum -a 256 -c -
fi
mkdir -p "$destination"
tar -xJf "$archive" --strip-components=1 -C "$destination"
test "$("$destination/zig" version)" = 0.16.0
printf 'Zig 0.16.0 installed at %s/zig\n' "$destination"
