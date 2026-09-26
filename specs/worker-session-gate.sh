#!/bin/sh
# WorkerSession: live client/worker ownership, bounded stream credits,
# typed exits and uncertain Store reconciliation. Missing tools, parse
# errors or a mutant without its named invariant failure are failures.
set -eu
JAR="${TLA_TOOLS_JAR:-/tla/tla2tools.jar}"
WORK=$(mktemp -d "${TMPDIR:-/tmp}/strop-workersessiongate.XXXXXX")
trap 'rm -rf "$WORK"' EXIT HUP INT TERM
. specs/check-model.sh
MODEL=specs/WorkerSession.tla
BASE=specs/cfg/worker-session.cfg

fault() {
    mutation=$1 invariant=$2 label=$3 base=${4:-$BASE}
    selected="$WORK/$label.cfg"
    sed -n '1,/^INVARIANTS/p' "$base" | sed "s/MUTATION = 0/MUTATION = $mutation/" >"$selected"
    printf 'TypeOK\n%s\n' "$invariant" >>"$selected"
    expect_kills "$label" "$selected" "$MODEL" no "$invariant"
}
fault 1 NoStaleEffect worker-stale-admission
fault 2 DirtyUntilProof worker-lost-reply-ack
fault 3 BoundedOutstanding worker-free-credit
fault 4 ExitAfterOutput worker-early-exit
fault 5 ForeignStopRefused worker-foreign-stop specs/cfg/worker-session-streams.cfg
fault 6 RecoveryInNamespace worker-foreign-recovery
fault 7 NoStaleCommit worker-stale-commit

for witness in WitnessSuccess WitnessUncertain WitnessRecovery WitnessAbandoned \
               WitnessChangedNamespace WitnessBackpressured; do
    fault 0 "$witness" "worker-$witness"
done
fault 0 WitnessBothClients worker-WitnessBothClients specs/cfg/worker-session-clients.cfg
fault 0 WitnessStalePrepared worker-WitnessStalePrepared

expect_clean worker-session "$BASE" "$MODEL"
expect_clean worker-session-clients specs/cfg/worker-session-clients.cfg "$MODEL"
expect_clean worker-session-streams specs/cfg/worker-session-streams.cfg "$MODEL"
echo 'worker session gate: bounded safety, semantic mutants and recovery witnesses checked'
