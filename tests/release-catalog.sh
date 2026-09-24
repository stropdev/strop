#!/bin/sh
# Shell-level fixtures for .github/scripts/release-catalog.py (0056 AR12,
# 0058 WK05):
#   - catalog generation is deterministic for fixed inputs
#   - the WK05 wire shape is pinned: worker manifest (protocol, min_editor,
#     targets) + per-artifact byte sizes
#   - a tampered checksum sidecar fails generation
#   - invalid worker facts (protocol < 1, min_editor postdating the release)
#     fail generation, and worker divergence fails public verification
#   - verify accepts matching local/public catalogs and rejects divergence
#   - the promotion ledger is idempotent per step (resumable re-records)
# Run: sh tests/release-catalog.sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
WORK=$(mktemp -d "${TMPDIR:-/tmp}/strop-catalog-test.XXXXXX")
trap 'rm -rf "$WORK"' EXIT HUP INT TERM

fail() { echo "FAIL: $1" >&2; exit 1; }
ok() { echo "ok: $1"; }

command -v python3 >/dev/null 2>&1 || { echo "skip: python3 required"; exit 0; }

SCRIPT="$ROOT/.github/scripts/release-catalog.py"
VERSION="9.9.9"
TARGETS="x86_64-unknown-linux-musl aarch64-apple-darwin"

mkdir -p "$WORK/dist"
for target in $TARGETS; do
    printf 'fake strop %s %s\n' "$VERSION" "$target" \
        > "$WORK/dist/strop-$VERSION-$target.tar.gz"
    if command -v sha256sum >/dev/null 2>&1; then
        digest=$(sha256sum "$WORK/dist/strop-$VERSION-$target.tar.gz" | cut -d' ' -f1)
    else
        digest=$(shasum -a 256 "$WORK/dist/strop-$VERSION-$target.tar.gz" | cut -d' ' -f1)
    fi
    printf '%s  %s\n' "$digest" "strop-$VERSION-$target.tar.gz" \
        > "$WORK/dist/strop-$VERSION-$target.tar.gz.sha256"
done

gen() { # out
    python3 "$SCRIPT" catalog --tag "v$VERSION" --dist "$WORK/dist" \
        --published-at "2026-01-01T00:00:00Z" \
        --worker-protocol 1 --worker-min-editor "9.0.0" --out "$1" >/dev/null
}

# --- deterministic generation ---------------------------------------------
gen "$WORK/a.json"
gen "$WORK/b.json"
cmp -s "$WORK/a.json" "$WORK/b.json" || fail "catalog generation is not deterministic"
grep -q "\"version\": \"$VERSION\"" "$WORK/a.json" || fail "catalog version missing"
grep -q '"tag": "v9.9.9"' "$WORK/a.json" || fail "catalog tag missing"
grep -q '"target": "x86_64-unknown-linux-musl"' "$WORK/a.json" \
    || fail "catalog artifact target missing"
grep -q '"published_at": "2026-01-01T00:00:00Z"' "$WORK/a.json" \
    || fail "catalog published_at missing"
ok "deterministic catalog generation"

# --- WK05 wire shape: worker manifest + artifact byte sizes -----------------
# This is the exact shape strop-worker-deploy parses; both ends pin it.
python3 - "$WORK/a.json" <<'PIN' || fail "catalog wire shape drifted"
import json, sys
catalog = json.load(open(sys.argv[1], encoding="utf-8"))
assert set(catalog) == {"schema", "product", "version", "tag",
                        "published_at", "artifacts", "worker"}, set(catalog)
worker = catalog["worker"]
assert worker == {"protocol": 1,
                  "min_editor": "9.0.0",
                  "targets": sorted(a["target"] for a in catalog["artifacts"])}, \
    worker
assert isinstance(worker["protocol"], int)
for artifact in catalog["artifacts"]:
    assert set(artifact) == {"target", "name", "sha256", "bytes", "url"}, artifact
    assert isinstance(artifact["bytes"], int) and artifact["bytes"] > 0, artifact
PIN
ok "WK05 worker manifest wire shape pinned"

# --- invalid worker facts fail generation -----------------------------------
if python3 "$SCRIPT" catalog --tag "v$VERSION" --dist "$WORK/dist" \
    --published-at "2026-01-01T00:00:00Z" \
    --worker-protocol 0 --worker-min-editor "9.0.0" \
    --out "$WORK/bad1.json" 2>/dev/null; then
    fail "worker protocol 0 did not fail catalog generation"
fi
if python3 "$SCRIPT" catalog --tag "v$VERSION" --dist "$WORK/dist" \
    --published-at "2026-01-01T00:00:00Z" \
    --worker-protocol 1 --worker-min-editor "9.9.10" \
    --out "$WORK/bad2.json" 2>/dev/null; then
    fail "min_editor postdating the release did not fail catalog generation"
fi
ok "invalid worker facts rejected"

# --- tampered sidecar fails generation -------------------------------------
SIDECAR="$WORK/dist/strop-$VERSION-x86_64-unknown-linux-musl.tar.gz.sha256"
cp "$SIDECAR" "$WORK/sidecar-good"
printf '0000000000000000000000000000000000000000000000000000000000000000  %s\n' \
    "strop-$VERSION-x86_64-unknown-linux-musl.tar.gz" \
    > "$SIDECAR"
if python3 "$SCRIPT" catalog --tag "v$VERSION" --dist "$WORK/dist" \
    --published-at "2026-01-01T00:00:00Z" \
    --worker-protocol 1 --worker-min-editor "9.0.0" \
    --out "$WORK/c.json" 2>/dev/null; then
    fail "tampered sidecar did not fail catalog generation"
fi
cp "$WORK/sidecar-good" "$SIDECAR"
ok "tampered sidecar rejected"

# --- verify: match passes, divergence fails ---------------------------------
gen "$WORK/local.json"
cp "$WORK/local.json" "$WORK/public.json"
python3 "$SCRIPT" verify --local "$WORK/local.json" --public "$WORK/public.json" \
    >/dev/null || fail "verify rejected identical catalogs"
sed 's/"sha256": "[0-9a-f]*"/"sha256": "deadbeef"/' "$WORK/local.json" \
    > "$WORK/public-diverged.json"
if python3 "$SCRIPT" verify --local "$WORK/local.json" \
    --public "$WORK/public-diverged.json" 2>/dev/null; then
    fail "verify accepted a diverged public catalog"
fi
sed 's/"protocol": 1/"protocol": 2/' "$WORK/local.json" \
    > "$WORK/public-worker-diverged.json"
if python3 "$SCRIPT" verify --local "$WORK/local.json" \
    --public "$WORK/public-worker-diverged.json" 2>/dev/null; then
    fail "verify accepted a diverged worker manifest"
fi
ok "public catalog verification"

# --- ledger: idempotent per step, resumable ---------------------------------
LEDGER="$WORK/promotion.json"
python3 "$SCRIPT" ledger --file "$LEDGER" --tag "v$VERSION" \
    --step crates.io --status ok --at "2026-01-01T00:00:00Z" >/dev/null
python3 "$SCRIPT" ledger --file "$LEDGER" --tag "v$VERSION" \
    --step github-release --status ok --at "2026-01-01T00:01:00Z" >/dev/null
python3 "$SCRIPT" ledger --file "$LEDGER" --tag "v$VERSION" \
    --step crates.io --status failed --detail "crates.io 500" \
    --at "2026-01-01T00:02:00Z" >/dev/null
[ "$(grep -c '"step": "crates.io"' "$LEDGER")" -eq 1 ] \
    || fail "re-recorded step duplicated in the ledger"
grep -q '"status": "failed"' "$LEDGER" || fail "re-recorded status not updated"
grep -q '"detail": "crates.io 500"' "$LEDGER" || fail "ledger detail missing"
grep -q '"at": "2026-01-01T00:02:00Z"' "$LEDGER" || fail "ledger timestamp not updated"
[ "$(grep -c '"step":' "$LEDGER")" -eq 2 ] || fail "unexpected ledger step count"
ok "promotion ledger is idempotent and resumable"

echo "release-catalog fixtures: all passed"
