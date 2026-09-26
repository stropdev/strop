#!/bin/sh
# Native worker cache lock, final-path admission and scoped retirement.
set -eu
JAR="${TLA_TOOLS_JAR:-/tla/tla2tools.jar}"
WORK=$(mktemp -d "${TMPDIR:-/tmp}/strop-workercachegate.XXXXXX")
trap 'rm -rf "$WORK"' EXIT HUP INT TERM
. specs/check-model.sh
MODEL=specs/WorkerCacheGC.tla
BASE=specs/cfg/worker-cache-gc.cfg

fault() {
    mutation=$1 invariant=$2 label=$3
    selected="$WORK/$label.cfg"
    sed -n '1,/^INVARIANTS/p' "$BASE" | sed "s/MUTATION = 0/MUTATION = $mutation/" >"$selected"
    printf 'TypeOK\n%s\n' "$invariant" >>"$selected"
    expect_kills "$label" "$selected" "$MODEL" no "$invariant"
}
fault 1 NoRetireLive worker-cache-unlocked-welcome
fault 2 NoRetireLive worker-cache-ignored-lease
fault 3 ScopedRetirement worker-cache-foreign-context
fault 4 LiveObject worker-cache-unchecked-executable

for witness in WitnessConcurrentReady WitnessBlockedWelcome WitnessRetired \
               WitnessCrashPinned WitnessDifferentBuilds; do
    fault 0 "$witness" "worker-cache-$witness"
done
expect_clean worker-cache-gc "$BASE" "$MODEL"
echo 'worker cache gate: lock/snapshot/retire graph checked'
