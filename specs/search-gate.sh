#!/bin/sh
# 0063 §6.4/§6.7: the search lifecycle model and its kept mutant.
# Unguarded publication must violate exactly RowsCurrent; acceptance
# without the revision re-check must violate exactly StaleAcceptsNever.
# Tool/parser failures never count as kills.
set -eu
JAR="${TLA_TOOLS_JAR:-/tla/tla2tools.jar}"
WORK=$(mktemp -d "${TMPDIR:-/tmp}/strop-searchmodel.XXXXXX}")
trap 'rm -rf "$WORK"' EXIT HUP INT TERM
. specs/check-model.sh

expect_clean "SearchLifecycle" specs/cfg/search-lifecycle.cfg specs/SearchLifecycle.tla

echo "[model gate] SearchLifecycle_Mutant: expect publication and stale-accept violations"
java -XX:+UseParallelGC -Xmx4g -jar "$JAR" -cleanup -continue \
    -metadir "$WORK/mutant.meta" -config specs/cfg/search-lifecycle-mutant.cfg \
    specs/SearchLifecycle_Mutant.tla >"$WORK/mutant.log" 2>&1 || true
if grep -qE 'Semantic errors:|Parsing or semantic analysis failed' "$WORK/mutant.log"; then
    echo "FAIL: the mutant did not parse"
    cat "$WORK/mutant.log"
    exit 1
fi
violations=$(grep -oE 'Invariant [A-Za-z0-9_]+ is violated' "$WORK/mutant.log" | sort -u)
expected="Invariant RowsCurrent is violated
Invariant StaleAcceptsNever is violated"
if [ "$violations" != "$expected" ]; then
    echo "FAIL: the mutant must violate only publication freshness and stale acceptance; got:"
    echo "$violations"
    exit 1
fi

echo "search lifecycle model gate: bounded checks clean; publication and stale-accept mutants rejected"
