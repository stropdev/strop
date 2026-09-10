#!/bin/sh
# Shared change plans (0043): revision-checked partial application and
# honest receipts, fault rejection and reachability witnesses.
set -eu
JAR="${TLA_TOOLS_JAR:-/tla/tla2tools.jar}"
WORK=$(mktemp -d "${TMPDIR:-/tmp}/strop-changemodelgate.XXXXXX")
trap 'rm -rf "$WORK"' EXIT HUP INT TERM
. specs/check-model.sh
MODEL=specs/ChangePlan.tla
BASE=specs/cfg/change-plan.cfg

expect_clean change-main "$BASE" "$MODEL"
expect_clean change-progress specs/cfg/change-plan-progress.cfg "$MODEL"

plan_fault() {
    mutation=$1 invariant=$2 label=$3
    selected="$WORK/$label.cfg"
    sed -n '1,/^INVARIANTS/p' "$BASE" | sed "s/MUTATION = 0/MUTATION = $mutation/" >"$selected"
    printf 'TypeOK\n%s\n' "$invariant" >>"$selected"
    expect_kills "$label" "$selected" "$MODEL" no "$invariant"
}
plan_fault 1 NoStaleApplication change-unchecked-apply
plan_fault 2 HonestReceipt change-dropped-refusal
plan_fault 3 PartialProgressHonest change-phantom-applied
for witness in WitnessNoFullSuccess WitnessNoPartial WitnessNoInvalidatedAll WitnessNoCancellation; do
    plan_fault 0 "$witness" "change-$witness"
done
echo 'change plan model gate: revision safety, receipt honesty, fault rejection and witnesses checked'
