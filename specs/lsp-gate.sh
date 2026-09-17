#!/bin/sh
# 0057 VF05: the LSP wire queue model and its kept mutants. Coalescing
# across a barrier must violate exactly CoalesceLegal; a refused-open
# binding exactly BindingOnlyIfAdmitted; close-skips-cancel exactly
# WireOrdered; a stale reply application exactly LateNeverApplies;
# bound bypass exactly Bounded; a double settle exactly TerminalOnce.
# Tool/parser failures never count as kills.
set -eu
JAR="${TLA_TOOLS_JAR:-/tla/tla2tools.jar}"
WORK=$(mktemp -d "${TMPDIR:-/tmp}/strop-lspmodel.XXXXXX")
trap 'rm -rf "$WORK"' EXIT HUP INT TERM
. specs/check-model.sh
MODEL=specs/LspWire.tla

expect_clean lsp-wire-main specs/cfg/lsp-wire.cfg "$MODEL"

check_fault lsp-wire-barrier-coalesce specs/cfg/lsp-wire-barrier-coalesce.cfg "$MODEL" CoalesceLegal
check_fault lsp-wire-refused-binding specs/cfg/lsp-wire-refused-binding.cfg "$MODEL" BindingOnlyIfAdmitted
check_fault lsp-wire-close-skips-cancel specs/cfg/lsp-wire-close-skips-cancel.cfg "$MODEL" WireOrdered
check_fault lsp-wire-stale-reply specs/cfg/lsp-wire-stale-reply.cfg "$MODEL" LateNeverApplies
check_fault lsp-wire-unbounded specs/cfg/lsp-wire-unbounded.cfg "$MODEL" Bounded
check_fault lsp-wire-double-terminal specs/cfg/lsp-wire-double-terminal.cfg "$MODEL" TerminalOnce

for witness in WitnessNoCoalesce WitnessNoRefusal WitnessNoReopen WitnessNoCancelNote WitnessNoFlush WitnessNoLateRejected WitnessNoApplied WitnessNoRestart; do
    check_fault "lsp-wire-$witness" specs/cfg/lsp-wire-coverage.cfg "$MODEL" "$witness"
done
echo 'lsp wire model gate: bounded checks clean; six mutants rejected; eight witnesses reached'
