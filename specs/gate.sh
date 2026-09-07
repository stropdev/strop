#!/bin/sh
# Model gate: the real protocol must satisfy every invariant. Removing
# delivery freshness must violate NoMisapply and NoWrongDocument, and no
# structural accounting invariant. Parser/tool failures never count as kills.
set -eu
JAR="${TLA_TOOLS_JAR:-/tla/tla2tools.jar}"
WORK=$(mktemp -d "${TMPDIR:-/tmp}/strop-modelgate.XXXXXX")
trap 'rm -rf "$WORK"' EXIT HUP INT TERM

echo "[model gate] EditorProtocol: expect a clean check"
java -jar "$JAR" -cleanup -config specs/cfg/editor-protocol.cfg \
    specs/EditorProtocol.tla >"$WORK/main.log" 2>&1 || {
    echo "FAIL: TLC did not accept EditorProtocol"
    cat "$WORK/main.log"
    exit 1
}
grep -q "Model checking completed. No error" "$WORK/main.log" || {
    echo "FAIL: EditorProtocol did not finish clean"
    cat "$WORK/main.log"
    exit 1
}

echo "[model gate] EditorProtocol_Mutant: expect freshness violations"
# Continue to distinguish expected freshness failures from accounting failures.
java -jar "$JAR" -cleanup -continue -config specs/cfg/editor-protocol-mutant.cfg \
    specs/EditorProtocol_Mutant.tla >"$WORK/mutant.log" 2>&1 || true
violations=$(grep -oE "Invariant [A-Za-z]+ is violated" "$WORK/mutant.log" | sort -u)
expected="Invariant NoMisapply is violated
Invariant NoWrongDocument is violated"
if [ "$violations" != "$expected" ]; then
    echo "FAIL: the mutant must violate only the two freshness invariants; got:"
    echo "$violations"
    exit 1
fi

echo "editor model gate: bounded checks clean; freshness mutant rejected"
sh specs/ssh-gate.sh
