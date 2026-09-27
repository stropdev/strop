#!/bin/sh
# Concurrent worker artifact deployment and cache retirement. This is
# safety under authenticated provider observations, not SHA/OS proof.
set -eu
JAR="${TLA_TOOLS_JAR:-/tla/tla2tools.jar}"
WORK=$(mktemp -d "${TMPDIR:-/tmp}/strop-workerdeploygate.XXXXXX")
trap 'rm -rf "$WORK"' EXIT HUP INT TERM
. specs/check-model.sh
MODEL=specs/WorkerDeploy.tla
BASE=specs/cfg/worker-deploy.cfg

fault() {
    mutation=$1 invariant=$2 label=$3
    selected="$WORK/$label.cfg"
    sed -n '1,/^INVARIANTS/p' "$BASE" | sed "s/MUTATION = 0/MUTATION = $mutation/" >"$selected"
    printf 'TypeOK\n%s\n' "$invariant" >>"$selected"
    expect_kills "$label" "$selected" "$MODEL" no "$invariant"
}
fault 1 ConsentBeforeUpload worker-upload-without-consent
fault 2 VerifiedActivation worker-corrupt-activation
fault 3 ChosenContext worker-default-context-retarget
fault 4 OwnedCleanup worker-foreign-stage-cleanup
fault 5 NoRetireLive worker-live-lease-retirement
fault 6 VerifiedActivation worker-wrong-executed-object
fault 7 OwnedCleanup worker-cross-context-cache-cleanup

for witness in WitnessReady WitnessPublishedNotReady WitnessBothInstallers \
               WitnessLiveLease WitnessConcurrentInstallers WitnessCorruptUnready \
               WitnessDifferentContexts; do
    fault 0 "$witness" "worker-deploy-$witness"
done

expect_clean worker-deploy "$BASE" "$MODEL"
expect_clean worker-deploy-targets specs/cfg/worker-deploy-targets.cfg "$MODEL"
sh specs/worker-cache-gate.sh
echo 'worker deployment gate: atomic abstraction and native refinement checked'
