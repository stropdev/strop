#!/bin/sh
# 0057 VF16: the install/update publication model and its kept mutants.
# Unverified publish must violate exactly VerifiedBeforeActivation;
# in-place publication exactly InstalledComplete (and
# NoClobberOnFailure); path-inferred channel exactly ChannelGate; a
# failed-transaction receipt exactly ReceiptNeverAhead; a stale catalog
# exactly StaleNeverPublishes. Tool/parser failures never count as kills.
set -eu
JAR="${TLA_TOOLS_JAR:-/tla/tla2tools.jar}"
WORK=$(mktemp -d "${TMPDIR:-/tmp}/strop-installmodel.XXXXXX")
trap 'rm -rf "$WORK"' EXIT HUP INT TERM
. specs/check-model.sh
MODEL=specs/Install.tla

expect_clean install-main specs/cfg/install.cfg "$MODEL"

check_fault install-unverified-publish specs/cfg/install-unverified-publish.cfg "$MODEL" VerifiedBeforeActivation
check_fault install-inplace-partial specs/cfg/install-inplace-publish.cfg "$MODEL" InstalledComplete
check_fault install-inplace-clobber specs/cfg/install-inplace-publish.cfg "$MODEL" NoClobberOnFailure
check_fault install-path-channel specs/cfg/install-path-channel.cfg "$MODEL" ChannelGate
check_fault install-receipt-ahead specs/cfg/install-receipt-ahead.cfg "$MODEL" ReceiptNeverAhead
check_fault install-stale-catalog specs/cfg/install-stale-catalog.cfg "$MODEL" StaleNeverPublishes

for witness in WitnessNoUpdate WitnessNoDigestRefusal WitnessNoInterrupt WitnessNoManagerRefusal WitnessNoUnknownRefusal WitnessNoStaleNoop WitnessNoReceiptCarry WitnessNoReceiptLag WitnessNoFreshInstall; do
    check_fault "install-$witness" specs/cfg/install-coverage.cfg "$MODEL" "$witness"
done
echo 'install model gate: bounded checks clean; five mutants rejected; nine witnesses reached'
