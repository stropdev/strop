#!/bin/sh
# VF01 candidate freeze (0057 §2): record the exact candidate — commit,
# tree, lockfile, shipped-helper digests, Dockerfile stage/tool pins —
# into verification/candidate.json. All work is in freeze.py (python3
# stdlib + git); this wrapper keeps the invocation boring.
#
#   sh verification/freeze.sh                 # write candidate.json
#   sh verification/freeze.sh --check         # candidate.json matches tree?
#   sh verification/freeze.sh --allow-dirty   # record dirt instead of refusing
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
exec python3 "$ROOT/verification/freeze.py" "$@"
