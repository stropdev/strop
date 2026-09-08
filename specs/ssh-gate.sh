#!/bin/sh
# Bounded SSH/SFTP checks, exact mutant kills, and non-vacuity witnesses.
set -eu
JAR="${TLA_TOOLS_JAR:-/tla/tla2tools.jar}"
WORK=$(mktemp -d "${TMPDIR:-/tmp}/strop-sshmodelgate.XXXXXX")
trap 'rm -rf "$WORK"' EXIT HUP INT TERM
WIRE=specs/SftpWire.tla
LIFE=specs/RemoteRead.tla

. specs/check-model.sh

expect_clean sftp-wire-main specs/cfg/sftp-wire.cfg "$WIRE"
expect_kills sftp-wire-wrong-id specs/cfg/sftp-wire-wrong-id.cfg "$WIRE" no ReplyIdentity
expect_kills sftp-wire-over-read specs/cfg/sftp-wire-over-read.cfg "$WIRE" no SnapshotBounded
expect_kills sftp-wire-unbounded-alloc specs/cfg/sftp-wire-unbounded-alloc.cfg "$WIRE" no PacketBounded
expect_kills sftp-wire-success-witness specs/cfg/sftp-wire-coverage.cfg "$WIRE" no WitnessNoSuccess
expect_kills sftp-wire-utf8-witness specs/cfg/sftp-wire-utf8.cfg "$WIRE" no WitnessNoUtf8Failure
expect_clean remote-read-main specs/cfg/remote-read.cfg "$LIFE"
expect_clean remote-read-liveness specs/cfg/remote-read-liveness.cfg "$LIFE"
expect_kills remote-read-stale-publish specs/cfg/remote-read-stale-publish.cfg "$LIFE" no FreshPublication
expect_kills remote-read-duplicate-terminal specs/cfg/remote-read-duplicate-terminal.cfg "$LIFE" no TerminalOnce
expect_kills remote-read-pid-reuse specs/cfg/remote-read-pid-reuse.cfg "$LIFE" no SignalOwnedPid
expect_kills remote-read-cleanup specs/cfg/remote-read-cleanup.cfg "$LIFE" no NoUnownedChild
expect_kills remote-read-cleanup-live specs/cfg/remote-read-cleanup-live.cfg "$LIFE" EventuallyReaped
for witness in WitnessNoAdmission WitnessNoPublication WitnessNoCancelledChild WitnessNoDroppedSuccess; do
    config="$WORK/$witness.cfg"
    grep -v '^ *Witness' specs/cfg/remote-read-coverage.cfg >"$config"
    printf '    %s\n' "$witness" >>"$config"
    expect_kills "remote-read-$witness" "$config" "$LIFE" no "$witness"
done

echo 'ssh model gate: bounded safety and qualified progress checked; seven faulty modes rejected; witnesses reached'
