#!/bin/sh
# 0059 C02/C04/C05/C09: independent epoch/selection ownership, physical
# outstanding requests, actual synchronization flush and native retirement.
set -eu
JAR="${TLA_TOOLS_JAR:-/tla/tla2tools.jar}"
WORK=$(mktemp -d "${TMPDIR:-/tmp}/strop-completion-model.XXXXXX")
trap 'rm -rf "$WORK"' EXIT HUP INT TERM
. specs/check-model.sh

expect_clean completion-ownership specs/cfg/completion-ownership.cfg specs/CompletionOwnership.tla
expect_clean completion-wire specs/cfg/completion-wire.cfg specs/CompletionWire.tla

check_fault completion-query-backlog specs/cfg/completion-ownership-mutant-1.cfg specs/CompletionOwnership.tla LatestOnly
check_fault completion-index-restart specs/cfg/completion-ownership-mutant-2.cfg specs/CompletionOwnership.tla IndexNotRestartedByQuery
check_fault completion-stale-publication specs/cfg/completion-ownership-mutant-3.cfg specs/CompletionOwnership.tla NativePublicationOwned
check_fault completion-stale-presentation specs/cfg/completion-ownership-mutant-4.cfg specs/CompletionOwnership.tla NoStalePresentationOrApply
check_fault completion-ui-destructor specs/cfg/completion-ownership-mutant-5.cfg specs/CompletionOwnership.tla NativeFinalDestruction
check_fault completion-unbounded-retention specs/cfg/completion-ownership-mutant-6.cfg specs/CompletionOwnership.tla BoundedRetention
check_fault completion-stale-acceptance specs/cfg/completion-ownership-mutant-7.cfg specs/CompletionOwnership.tla NoStalePresentationOrApply

check_fault completion-cancel-releases-slot specs/cfg/completion-wire-mutant-1.cfg specs/CompletionWire.tla PhysicalSlotsRetained
check_fault completion-crosses-flush specs/cfg/completion-wire-mutant-2.cfg specs/CompletionWire.tla WireSnapshotOrdered
check_fault completion-double-terminal specs/cfg/completion-wire-mutant-3.cfg specs/CompletionWire.tla TerminalOnce
check_fault completion-unbounded-physical specs/cfg/completion-wire-mutant-4.cfg specs/CompletionWire.tla PhysicalOutstandingBounded
check_fault completion-close-loses-request specs/cfg/completion-wire-mutant-5.cfg specs/CompletionWire.tla ClosedSettlesAll

for witness in WitnessNoApply WitnessNoRetire WitnessNoCancel; do
    check_fault "completion-ownership-$witness" specs/cfg/completion-ownership.cfg specs/CompletionOwnership.tla "$witness"
done
for witness in WitnessNoReply WitnessNoCancelledSent WitnessNoPhysicalFlush; do
    check_fault "completion-wire-$witness" specs/cfg/completion-wire.cfg specs/CompletionWire.tla "$witness"
done
echo 'completion model gate: two models clean; twelve mutants rejected; six witnesses reached'
