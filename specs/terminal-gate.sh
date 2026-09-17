#!/bin/sh
# 0057 VF12: the terminal lifecycle model and its kept mutants.
# Stale-frame acceptance must violate exactly FrameFresh; auto-install
# under inspection exactly PinnedNeverDragged; resurrection exactly
# TerminalAbsorbing (and ExitAnnouncedOnce); dead-session input exactly
# InputNeverToDead; an honored OSC host effect exactly
# OutputNeverMutates; a child-applied resize exactly GeometryOwned.
# Tool/parser failures never count as kills.
set -eu
JAR="${TLA_TOOLS_JAR:-/tla/tla2tools.jar}"
WORK=$(mktemp -d "${TMPDIR:-/tmp}/strop-terminalmodel.XXXXXX")
trap 'rm -rf "$WORK"' EXIT HUP INT TERM
. specs/check-model.sh
MODEL=specs/Terminal.tla

expect_clean terminal-main specs/cfg/terminal.cfg "$MODEL"

check_fault terminal-stale-frame specs/cfg/terminal-stale-frame.cfg "$MODEL" FrameFresh
check_fault terminal-dragged-pin specs/cfg/terminal-dragged-pin.cfg "$MODEL" PinnedNeverDragged
check_fault terminal-resurrect specs/cfg/terminal-resurrect.cfg "$MODEL" TerminalAbsorbing
check_fault terminal-resurrect-twice specs/cfg/terminal-resurrect.cfg "$MODEL" ExitAnnouncedOnce
check_fault terminal-dead-input specs/cfg/terminal-dead-input.cfg "$MODEL" InputNeverToDead
check_fault terminal-host-effect specs/cfg/terminal-host-effect.cfg "$MODEL" OutputNeverMutates
check_fault terminal-child-resize specs/cfg/terminal-child-resize.cfg "$MODEL" GeometryOwned
check_fault terminal-dead-input-boundary specs/cfg/terminal-dead-input.cfg "$MODEL" ExitBoundary

for witness in WitnessNoPinnedLag WitnessNoRefresh WitnessNoExitRetained WitnessNoDenied WitnessNoLateDrop WitnessNoReentry WitnessNoClosing WitnessNoTwoSessions WitnessNoPasteBoundary; do
    check_fault "terminal-$witness" specs/cfg/terminal-coverage.cfg "$MODEL" "$witness"
done
echo 'terminal model gate: bounded checks clean; six mutants rejected; nine witnesses reached'
