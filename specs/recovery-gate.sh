#!/bin/sh
# 0057 VF13: the recovery cohort model and its kept mutants.
# Half-cohort mixing must violate exactly CohortCoherent (and
# DurableMatchesLastComplete); premature cleanup exactly
# DurableMatchesLastComplete; checkpoint-as-save exactly
# SaveRetiresOnlySuperseded; restore-to-disk exactly
# RestoreNeverWritesDisk; checkpoint-cleans exactly CheckpointNeverCleans;
# consent bypass exactly RemoteConsentGated. Tool/parser failures never
# count as kills.
set -eu
JAR="${TLA_TOOLS_JAR:-/tla/tla2tools.jar}"
WORK=$(mktemp -d "${TMPDIR:-/tmp}/strop-recoverymodel.XXXXXX")
trap 'rm -rf "$WORK"' EXIT HUP INT TERM
. specs/check-model.sh
MODEL=specs/Recovery.tla

expect_clean recovery-main specs/cfg/recovery.cfg "$MODEL"

check_fault recovery-half-cohort specs/cfg/recovery-half-cohort.cfg "$MODEL" CohortCoherent
check_fault recovery-half-cohort-image specs/cfg/recovery-half-cohort.cfg "$MODEL" DurableMatchesLastComplete
check_fault recovery-premature-cleanup specs/cfg/recovery-premature-cleanup.cfg "$MODEL" DurableMatchesLastComplete
check_fault recovery-checkpoint-as-save specs/cfg/recovery-checkpoint-as-save.cfg "$MODEL" SaveRetiresOnlySuperseded
check_fault recovery-restore-clobbers specs/cfg/recovery-restore-clobbers.cfg "$MODEL" RestoreNeverWritesDisk
check_fault recovery-checkpoint-cleans specs/cfg/recovery-checkpoint-cleans.cfg "$MODEL" CheckpointNeverCleans
check_fault recovery-consent-ignored specs/cfg/recovery-consent-ignored.cfg "$MODEL" RemoteConsentGated

for witness in WitnessNoDurable WitnessNoMultiDocCohort WitnessNoFailedPublish WitnessNoConflictRestore WitnessNoCrashRestore WitnessNoConsentCycle WitnessNoQueueReplace WitnessNoRetirement WitnessNoDiscard WitnessNoMemoryOnly; do
    check_fault "recovery-$witness" specs/cfg/recovery-coverage.cfg "$MODEL" "$witness"
done
echo 'recovery model gate: bounded checks clean; six mutants rejected; ten witnesses reached'
