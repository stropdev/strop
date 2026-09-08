#!/bin/sh
# Remote workspace and process safety, qualified progress, faults and witnesses.
set -eu
JAR="${TLA_TOOLS_JAR:-/tla/tla2tools.jar}"
WORK=$(mktemp -d "${TMPDIR:-/tmp}/strop-remotemodelgate.XXXXXX")
trap 'rm -rf "$WORK"' EXIT HUP INT TERM
. specs/check-model.sh
PROCESS=specs/RemoteProcess.tla
POOL=specs/RemoteWorkspace.tla

# Each targeted fault gets its own configuration; a first counterexample must
# violate exactly that invariant, not a parser failure or another property.
check_fault() {
    fault_label=$1 base_config=$2 fault_module=$3 invariant=$4
    selected="$WORK/$fault_label.cfg"
    sed -n '1,/^INVARIANTS/p' "$base_config" >"$selected"
    printf 'TypeOK\n%s\n' "$invariant" >>"$selected"
    expect_kills "$fault_label" "$selected" "$fault_module" no "$invariant"
}

expect_clean remote-process-main specs/cfg/remote-process.cfg "$PROCESS"
expect_clean remote-process-progress specs/cfg/remote-process-liveness.cfg "$PROCESS"
check_fault process-ungated specs/cfg/remote-process-ungated-launch.cfg "$PROCESS" LaunchGated
check_fault process-early-reap specs/cfg/remote-process-early-reap.cfg "$PROCESS" ReapAfterFinalKill
check_fault process-reused-pid specs/cfg/remote-process-early-reap.cfg "$PROCESS" SignalOwnedTarget
check_fault process-post-eof specs/cfg/remote-process-backpressure.cfg "$PROCESS" NoPostEofDelivery
check_fault process-premature-success specs/cfg/remote-process-premature-success.cfg "$PROCESS" OutcomeClean
check_fault process-unreaped-ssh specs/cfg/remote-process-premature-success.cfg "$PROCESS" NoUnownedSsh
expect_kills process-orphan-progress specs/cfg/remote-process-early-reap-live.cfg "$PROCESS" EventuallyRemoteSettled
expect_kills process-eof-progress specs/cfg/remote-process-backpressure-live.cfg "$PROCESS" EventuallyEofObserved
for witness in WitnessNoAdmission WitnessNoLaunch WitnessNoWorkerRun WitnessNoExitOutcome WitnessNoCancelDuringRun WitnessNoBackpressuredEof WitnessNoStragglerCleanup WitnessNoPartitionWindow; do
    check_fault "process-$witness" specs/cfg/remote-process-coverage.cfg "$PROCESS" "$witness"
done

expect_clean workspace-main specs/cfg/remote-workspace.cfg "$POOL"
expect_clean workspace-progress specs/cfg/remote-workspace-liveness.cfg "$POOL"
expect_kills workspace-epoch specs/cfg/remote-workspace-epoch.cfg "$POOL" no ReplyOwnsDispatch
expect_kills workspace-queued-cancel specs/cfg/remote-workspace-queued-cancel.cfg "$POOL" no NoQueuedSkipTeardown
expect_kills workspace-lease-release specs/cfg/remote-workspace-lease-release.cfg "$POOL" no LastLeaseSignalsStop
expect_kills workspace-stale-service specs/cfg/remote-workspace-stale-service.cfg "$POOL" no ServiceOwnership
for witness in WitnessNoReuse WitnessNoReconnect WitnessNoActiveCancel WitnessNoQueuedCancel WitnessNoLastLeaseShutdown WitnessNoRefusal WitnessNoServicePublication WitnessNoStoppedJob WitnessNoStaleBytesDiscarded WitnessNoReincarnation; do
    check_fault "workspace-$witness" specs/cfg/remote-workspace-coverage.cfg "$POOL" "$witness"
done
expect_clean workspace-follow specs/cfg/remote-workspace-follow.cfg "$POOL"
expect_kills workspace-stat-identity specs/cfg/remote-workspace-follow-stat-identity.cfg "$POOL" no FollowWindowHonest
for witness in WitnessNoFollowPublish WitnessNoFollowAppend WitnessNoFollowReset WitnessNoSameStatReplacement; do
    check_fault "follow-$witness" specs/cfg/remote-workspace-follow-coverage.cfg "$POOL" "$witness"
done
echo 'remote model gate: safety, qualified progress, exact fault rejection and witnesses checked'
