#!/bin/sh
# 0057 VF14: the UI-stdio session model and its kept mutants. A stale
# base applied must violate exactly StaleNeverActs; a future base
# exactly FutureNeverActs; a foreign incarnation's base exactly
# ForeignNeverActs; a poisoned client's action exactly
# PoisonedNeverActs; a delta clearing poisoning exactly
# PoisonedUntilSnapshot; an unchanged publication exactly
# NoEmptyPublication; a re-emitted clipboard effect exactly
# EffectExactlyOnce; a bye without the final ack exactly
# ByeAfterFinalAck; a post-bye publication exactly NoPostByePublication.
# Tool/parser failures never count as kills.
set -eu
JAR="${TLA_TOOLS_JAR:-/tla/tla2tools.jar}"
WORK=$(mktemp -d "${TMPDIR:-/tmp}/strop-uimodel.XXXXXX")
trap 'rm -rf "$WORK"' EXIT HUP INT TERM
. specs/check-model.sh
MODEL=specs/UiSession.tla

expect_clean ui-main specs/cfg/ui.cfg "$MODEL"

check_fault ui-stale-base specs/cfg/ui-stale-base.cfg "$MODEL" StaleNeverActs
check_fault ui-future-base specs/cfg/ui-future-base.cfg "$MODEL" FutureNeverActs
check_fault ui-foreign-base specs/cfg/ui-foreign-base.cfg "$MODEL" ForeignNeverActs
check_fault ui-poisoned-act specs/cfg/ui-poisoned-act.cfg "$MODEL" PoisonedNeverActs
check_fault ui-delta-unpoison specs/cfg/ui-delta-unpoison.cfg "$MODEL" PoisonedUntilSnapshot
check_fault ui-empty-publish specs/cfg/ui-empty-publish.cfg "$MODEL" NoEmptyPublication
check_fault ui-double-effect specs/cfg/ui-double-effect.cfg "$MODEL" EffectExactlyOnce
check_fault ui-early-bye specs/cfg/ui-early-bye.cfg "$MODEL" ByeAfterFinalAck
check_fault ui-post-bye-publish specs/cfg/ui-post-bye-publish.cfg "$MODEL" NoPostByePublication

for witness in WitnessNoDrop WitnessNoPoison WitnessNoRecovery WitnessNoEffect WitnessNoStaleRefusal WitnessNoFutureRefusal WitnessNoForeignRefusal WitnessNoBye WitnessNoOutrun WitnessNoSameGenDelta; do
    check_fault "ui-$witness" specs/cfg/ui-coverage.cfg "$MODEL" "$witness"
done
echo 'ui model gate: bounded checks clean; nine mutants rejected; ten witnesses reached'
