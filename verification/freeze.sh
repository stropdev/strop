#!/bin/sh
# Freeze the committed worker candidate and its exact source, lock,
# inventory, baseline archive and pinned tool identities. The output
# lives in ignored dist/worker-candidate.json, not inside the commit
# whose hash it records.
#
#   sh verification/freeze.sh                 # write candidate artifact
#   sh verification/freeze.sh --check         # match current clean tree
#   sh verification/freeze.sh --allow-dirty   # diagnostic, unqualified
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
exec python3 "$ROOT/verification/freeze.py" "$@"
