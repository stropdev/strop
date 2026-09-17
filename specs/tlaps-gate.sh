#!/bin/sh
# 0057 VF17 / 0063 §6.6: the TLAPS proof lane for the search lifecycle.
# The proofs module must verify; the kept-mutant mirror must be REJECTED
# (the negative control). Tool/parser failures never count as a kill, and
# a cached fingerprint never counts as a proof (--cleanfp re-proves all).
set -eu
TLAPM="${TLAPM:-tlapm}"
WORK=$(mktemp -d "${TMPDIR:-/tmp}/strop-tlaps.XXXXXX}")
trap 'rm -rf "$WORK"' EXIT HUP INT TERM

echo "[tlaps gate] SearchLifecycleProofs: expect every obligation proved"
if ! (cd specs && "$TLAPM" -I . --cleanfp SearchLifecycleProofs.tla) \
        >"$WORK/main.log" 2>&1; then
    echo "FAIL: TLAPS did not verify the SearchLifecycle safety proofs"
    cat "$WORK/main.log"
    exit 1
fi
if grep -qE '\[ERROR\]|Could not parse' "$WORK/main.log" \
        || ! grep -q 'obligations proved' "$WORK/main.log"; then
    echo "FAIL: the proof run was not a clean full verification"
    cat "$WORK/main.log"
    exit 1
fi
grep -E 'All [0-9]+ obligations proved' "$WORK/main.log"

echo "[tlaps gate] SearchLifecycle_MutantProofs: expect rejection (negative control)"
if (cd specs && "$TLAPM" -I . --cleanfp SearchLifecycle_MutantProofs.tla) \
        >"$WORK/mutant.log" 2>&1; then
    echo "FAIL: TLAPS verified the kept mutant — the negative control is dead"
    cat "$WORK/mutant.log"
    exit 1
fi
# The rejection must be unproved proof obligations on the mutation's own
# steps — never a parse or tool failure.
if grep -qE 'Could not parse|Semantic errors|Unexpected' "$WORK/mutant.log" \
        || ! grep -q 'obligations failed' "$WORK/mutant.log" \
        || ! grep -q 'Could not prove or check' "$WORK/mutant.log"; then
    echo "FAIL: the mutant was rejected for tooling reasons, not unprovable obligations"
    cat "$WORK/mutant.log"
    exit 1
fi
grep -E 'obligations failed' "$WORK/mutant.log"

echo "tlaps gate: lifecycle safety proved inductively; kept mutant rejected"
