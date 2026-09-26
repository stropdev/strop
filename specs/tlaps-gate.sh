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

echo "[tlaps gate] WorkerSessionProofs: expect symbolic induction and safety"
if ! (cd specs && "$TLAPM" -I . --cleanfp WorkerSessionProofs.tla) \
        >"$WORK/worker.log" 2>&1; then
    echo "FAIL: TLAPS did not verify the generalized WorkerSession safety proofs"
    cat "$WORK/worker.log"
    exit 1
fi
if grep -qE '\[ERROR\]|Could not parse|Expression not supported' "$WORK/worker.log" \
        || ! grep -q 'obligations proved' "$WORK/worker.log"; then
    echo "FAIL: WorkerSession proof run was not a clean verification"
    cat "$WORK/worker.log"
    exit 1
fi
grep -E 'All [0-9]+ obligations proved' "$WORK/worker.log"

echo "[tlaps gate] WorkerDeployProofs: expect symbolic induction and safety"
if ! (cd specs && "$TLAPM" -I . --cleanfp WorkerDeployProofs.tla) \
        >"$WORK/deploy.log" 2>&1; then
    echo "FAIL: TLAPS did not verify the generalized WorkerDeploy safety proofs"
    cat "$WORK/deploy.log"
    exit 1
fi
if grep -qE '\[ERROR\]|Could not parse|Expression not supported' "$WORK/deploy.log" \
        || ! grep -q 'obligations proved' "$WORK/deploy.log"; then
    echo "FAIL: WorkerDeploy proof run was not a clean verification"
    cat "$WORK/deploy.log"
    exit 1
fi
grep -E 'All [0-9]+ obligations proved' "$WORK/deploy.log"

echo "[tlaps gate] WorkerCacheGC: conditional scoped retirement theorems"
if ! (cd specs && "$TLAPM" -I . --cleanfp WorkerCacheGCProofs.tla) \
        >"$WORK/cache.log" 2>&1; then
    echo "FAIL: cache retirement does not preserve its scoped snapshot premises"
    cat "$WORK/cache.log"
    exit 1
fi
if grep -qE '\[ERROR\]|Could not parse|Expression not supported' "$WORK/cache.log" \
        || ! grep -q 'obligations proved' "$WORK/cache.log"; then
    echo "FAIL: conditional cache retirement proof was not clean"
    cat "$WORK/cache.log"
    exit 1
fi
grep -E 'All [0-9]+ obligations proved' "$WORK/cache.log"

echo "[tlaps gate] WorkerCacheGC_InductionProofs: generalized lock/snapshot safety"
if ! (cd specs && "$TLAPM" -I . --cleanfp WorkerCacheGC_InductionProofs.tla) \
        >"$WORK/cache-induction.log" 2>&1; then
    echo "FAIL: native cache lock/snapshot safety is not inductive"
    cat "$WORK/cache-induction.log"
    exit 1
fi
if grep -qE '\[ERROR\]|Could not parse|Expression not supported' "$WORK/cache-induction.log" \
        || ! grep -q 'obligations proved' "$WORK/cache-induction.log"; then
    echo "FAIL: generalized cache lock/snapshot proof was not clean"
    cat "$WORK/cache-induction.log"
    exit 1
fi
grep -E 'All [0-9]+ obligations proved' "$WORK/cache-induction.log"

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

echo "[tlaps gate] WorkerSession Commit: matched healthy proof"
if ! (cd specs && "$TLAPM" -I . --cleanfp WorkerSession_CleanProofs.tla) \
        >"$WORK/worker-clean.log" 2>&1; then
    echo "FAIL: the guarded Commit action lost its positive proof control"
    cat "$WORK/worker-clean.log"
    exit 1
fi
if grep -qE '\[ERROR\]|Could not parse|Expression not supported' "$WORK/worker-clean.log" \
        || ! grep -q 'obligations proved' "$WORK/worker-clean.log"; then
    echo "FAIL: the guarded Commit control was not cleanly proved"
    cat "$WORK/worker-clean.log"
    exit 1
fi
grep -E 'All [0-9]+ obligations proved' "$WORK/worker-clean.log"

echo "[tlaps gate] WorkerSession stale Commit: expect safety proof failure"
if (cd specs && "$TLAPM" -I . --cleanfp WorkerSession_MutantProofs.tla) \
        >"$WORK/worker-mutant.log" 2>&1; then
    echo "FAIL: TLAPS verified the stale-commit mutant"
    cat "$WORK/worker-mutant.log"
    exit 1
fi
if grep -qE 'Could not parse|Semantic errors|Unexpected|Expression not supported' "$WORK/worker-mutant.log" \
        || ! grep -q 'obligations failed' "$WORK/worker-mutant.log" \
        || ! grep -q 'Could not prove or check' "$WORK/worker-mutant.log" \
        || ! grep -q 'NoStaleCommit' "$WORK/worker-mutant.log" \
        || ! grep -q 'BadInputs' "$WORK/worker-mutant.log"; then
    echo "FAIL: stale-commit mutant was not rejected on its safety obligation"
    cat "$WORK/worker-mutant.log"
    exit 1
fi
grep -E 'obligations failed' "$WORK/worker-mutant.log"

echo "[tlaps gate] WorkerDeploy Collect: matched healthy proof"
if ! (cd specs && "$TLAPM" -I . --cleanfp WorkerDeploy_CleanProofs.tla) \
        >"$WORK/deploy-clean.log" 2>&1; then
    echo "FAIL: the scoped Collect action lost its positive proof control"
    cat "$WORK/deploy-clean.log"
    exit 1
fi
if grep -qE '\[ERROR\]|Could not parse|Expression not supported' "$WORK/deploy-clean.log" \
        || ! grep -q 'obligations proved' "$WORK/deploy-clean.log"; then
    echo "FAIL: the scoped Collect control was not cleanly proved"
    cat "$WORK/deploy-clean.log"
    exit 1
fi
grep -E 'All [0-9]+ obligations proved' "$WORK/deploy-clean.log"

echo "[tlaps gate] WorkerDeploy cross-context Collect: expect safety proof failure"
if (cd specs && "$TLAPM" -I . --cleanfp WorkerDeploy_MutantProofs.tla) \
        >"$WORK/deploy-mutant.log" 2>&1; then
    echo "FAIL: TLAPS verified the cross-context-cleanup mutant"
    cat "$WORK/deploy-mutant.log"
    exit 1
fi
if grep -qE 'Could not parse|Semantic errors|Unexpected|Expression not supported' "$WORK/deploy-mutant.log" \
        || ! grep -q 'obligations failed' "$WORK/deploy-mutant.log" \
        || ! grep -q 'Could not prove or check' "$WORK/deploy-mutant.log" \
        || ! grep -q 'OwnedCleanup' "$WORK/deploy-mutant.log" \
        || ! grep -q 'BadInputs' "$WORK/deploy-mutant.log"; then
    echo "FAIL: cross-context mutant was not rejected on its safety obligation"
    cat "$WORK/deploy-mutant.log"
    exit 1
fi
grep -E 'obligations failed' "$WORK/deploy-mutant.log"

echo "[tlaps gate] WorkerCacheGC foreign-context retirement: expect scoped theorem failure"
if (cd specs && "$TLAPM" -I . --cleanfp WorkerCacheGC_MutantProofs.tla) \
        >"$WORK/cache-mutant.log" 2>&1; then
    echo "FAIL: TLAPS verified the foreign-context cache-retirement mutant"
    cat "$WORK/cache-mutant.log"
    exit 1
fi
if grep -qE 'Could not parse|Semantic errors|Unexpected|Expression not supported' "$WORK/cache-mutant.log" \
        || ! grep -q 'obligations failed' "$WORK/cache-mutant.log" \
        || ! grep -q 'Could not prove or check' "$WORK/cache-mutant.log" \
        || ! grep -q 'ScopedRetirement' "$WORK/cache-mutant.log" \
        || ! grep -q 'BadInputs' "$WORK/cache-mutant.log"; then
    echo "FAIL: cache mutant failed for tooling, not the scoped retirement theorem"
    cat "$WORK/cache-mutant.log"
    exit 1
fi
grep -E 'obligations failed' "$WORK/cache-mutant.log"
echo "tlaps gate: search, worker-session, deploy and conditional cache safety proved; paired mutants rejected"
