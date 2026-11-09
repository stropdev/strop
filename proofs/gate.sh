#!/bin/sh
# The `proofs` gate (plans/0086 §7), run at container start in the image
# proofs/Dockerfile builds: regenerate the translation and refuse drift
# against the committed proofs/Extracted/, build the proofs, audit every
# theorem's axioms, then calibrate with mutants that must fail.
set -eu
cd "$(dirname "$0")/.."
started="$(date +%s)"
stamp() { echo "[$(( $(date +%s) - started ))s] $*"; }

stamp "extract: charon $(charon version) / aeneas $(aeneas -version)"
rm -rf /tmp/gate-extract
sh proofs/extract.sh crates/strop-kernel /tmp/gate-extract

stamp "drift check against proofs/Extracted"
if ! diff -ru proofs/Extracted /tmp/gate-extract; then
    echo "FAIL: proofs/Extracted differs from a fresh translation of crates/strop-kernel;" >&2
    echo "      run 'sh proofs/refresh.sh' and commit the result" >&2
    exit 1
fi

stamp "lake build"
cd proofs
lake build
stamp "axiom audit"
lake env lean Audit.lean
cd ..

stamp "mutant calibration"
python3 proofs/mutants.py

stamp "proofs gate: OK"
