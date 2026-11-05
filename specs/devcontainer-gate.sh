#!/bin/sh
# Plans 0037/0081: bounded Dev Container lifecycle safety and kept mutants.
# A mutant counts only when TLC finishes and violates exactly its named
# invariant; parser, semantic, runtime and unrelated invariant failures fail
# the gate. The clean configuration checks all safety properties together.
set -eu
JAR="${TLA_TOOLS_JAR:-/tla/tla2tools.jar}"
WORK=$(mktemp -d "${TMPDIR:-/tmp}/strop-devcontainer-model.XXXXXX")
trap 'rm -rf "$WORK"' EXIT HUP INT TERM
. specs/check-model.sh
MODEL=specs/DevContainerLifecycle.tla

expect_clean devcontainer-lifecycle-clean \
    specs/cfg/devcontainer-lifecycle.cfg "$MODEL"
check_fault devcontainer-lifecycle-omit-trust \
    specs/cfg/devcontainer-lifecycle-omit-trust.cfg "$MODEL" NoUntrustedEffects
check_fault devcontainer-lifecycle-ignore-incarnation \
    specs/cfg/devcontainer-lifecycle-ignore-incarnation.cfg "$MODEL" NoStaleWorkspacePublication
check_fault devcontainer-lifecycle-publish-before-hooks \
    specs/cfg/devcontainer-lifecycle-publish-before-hooks.cfg "$MODEL" NoReadyBeforeRequiredHooks
check_fault devcontainer-lifecycle-stop-unowned \
    specs/cfg/devcontainer-lifecycle-stop-unowned.cfg "$MODEL" OnlyOwnedResourcesStopped

echo 'devcontainer lifecycle model gate: clean safety passed; four intended mutants killed by their named invariants'
