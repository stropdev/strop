#!/bin/sh
# 0058 S7: the filesystem-notification lifecycle model and its kept
# mutants. A hint applied as authority must violate exactly
# HintsNeverAuthority; acting on a stale identity exactly
# StaleIdentityNeverActs; a guessed-pair relocation exactly
# AmbiguousNeverRelocates; premature freshness exactly
# FreshRequiresCoverage; a silent queue-full drop exactly
# CoalesceNeverLosesStaleness; boundary erasure of newer invalidations
# exactly ScanNeverErasesNewer; a reload over a dirty buffer exactly
# DirtyNeverClobbered; a stale snapshot publish exactly
# StaleReloadNeverClears; an invisible-crawl substitution exactly
# CoverageHonest. Tool/parser failures never count as kills.
set -eu
JAR="${TLA_TOOLS_JAR:-/tla/tla2tools.jar}"
WORK=$(mktemp -d "${TMPDIR:-/tmp}/strop-notifymodel.XXXXXX")
trap 'rm -rf "$WORK"' EXIT HUP INT TERM
. specs/check-model.sh
MODEL=specs/Notify.tla

expect_clean notify-main specs/cfg/notify.cfg "$MODEL"

check_fault notify-hint-authority specs/cfg/notify-hint-authority.cfg "$MODEL" HintsNeverAuthority
check_fault notify-stale-identity specs/cfg/notify-stale-identity.cfg "$MODEL" StaleIdentityNeverActs
check_fault notify-guessed-relocation specs/cfg/notify-guessed-relocation.cfg "$MODEL" AmbiguousNeverRelocates
check_fault notify-premature-fresh specs/cfg/notify-premature-fresh.cfg "$MODEL" FreshRequiresCoverage
check_fault notify-silent-drop specs/cfg/notify-silent-drop.cfg "$MODEL" CoalesceNeverLosesStaleness
check_fault notify-boundary-erasure specs/cfg/notify-boundary-erasure.cfg "$MODEL" ScanNeverErasesNewer
check_fault notify-dirty-clobber specs/cfg/notify-dirty-clobber.cfg "$MODEL" DirtyNeverClobbered
check_fault notify-stale-reload specs/cfg/notify-stale-reload.cfg "$MODEL" StaleReloadNeverClears
check_fault notify-invisible-crawl specs/cfg/notify-invisible-crawl.cfg "$MODEL" CoverageHonest

for witness in WitnessNoOverflow WitnessNoRescan WitnessNoRestart WitnessNoStaleDrop WitnessNoDuplicate WitnessNoBoundaryNewer WitnessNoUnsupportedRefusal WitnessNoOnDemand WitnessNoExclusions WitnessNoControlPressure WitnessNoSaveRace WitnessNoDirtyExternal WitnessNoRelocation WitnessNoRescanChurn WitnessNoTwoEstablished; do
    check_fault "notify-$witness" specs/cfg/notify-coverage.cfg "$MODEL" "$witness"
done
echo 'notify model gate: bounded checks clean; nine mutants rejected; fifteen witnesses reached'
