#!/bin/sh
# Conditional remote writes: atomicity, ownership, metadata, privacy and limits.
set -eu
JAR="${TLA_TOOLS_JAR:-/tla/tla2tools.jar}"
WORK=$(mktemp -d "${TMPDIR:-/tmp}/strop-savemodelgate.XXXXXX")
trap 'rm -rf "$WORK"' EXIT HUP INT TERM
. specs/check-model.sh
MODEL=specs/RemoteSave.tla
BASE=specs/cfg/remote-save.cfg

expect_clean save-main "$BASE" "$MODEL"
expect_clean save-concurrency specs/cfg/remote-save-concurrency.cfg "$MODEL"
expect_clean save-progress specs/cfg/remote-save-progress.cfg "$MODEL"

save_fault() {
    mutation=$1 invariant=$2 label=$3
    selected="$WORK/$label.cfg"
    sed -n '1,/^INVARIANTS/p' "$BASE" | sed "s/MUTATION = 0/MUTATION = $mutation/" >"$selected"
    printf 'TypeOK\n%s\n' "$invariant" >>"$selected"
    expect_kills "$label" "$selected" "$MODEL" no "$invariant"
}
save_fault 1 NoBlindOverwrite save-stat-only
save_fault 2 NoPartialOriginal save-in-place
save_fault 3 MetadataPreserved save-metadata-loss
save_fault 4 NoLinkWrites save-follow-link
save_fault 5 PrivateDraft save-readable-stage
save_fault 7 OwnedAcknowledgment save-stale-ack
save_fault 8 DurableAcknowledgment save-unsynced-ack
expect_kills save-lock-replacement specs/cfg/remote-save-lock-namespace.cfg "$MODEL" no SingleLockDomain

# This counterexample documents a deliberately UNCLAIMED guarantee: advisory
# exclusion cannot prevent a nonparticipant racing the final check and rename.
expect_kills save-nonparticipant-limit specs/cfg/remote-save-nonparticipant.cfg "$MODEL" no NoBlindOverwrite
for witness in WitnessNoCommit WitnessNoConfirmed WitnessNoCancelledCommit WitnessNoPrivateOrphan WitnessNoVerification; do
    save_fault 0 "$witness" "save-$witness"
done
echo 'remote save model gate: conditional safety, progress, fault rejection and witnesses checked'
