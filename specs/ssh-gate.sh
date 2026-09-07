#!/bin/sh
# Bounded SSH/SFTP checks, exact mutant kills, and non-vacuity witnesses.
set -eu
JAR="${TLA_TOOLS_JAR:-/tla/tla2tools.jar}"
WORK=$(mktemp -d "${TMPDIR:-/tmp}/strop-sshmodelgate.XXXXXX")
trap 'rm -rf "$WORK"' EXIT HUP INT TERM
WIRE=specs/SftpWire.tla
LIFE=specs/RemoteRead.tla

run_model() {
    name=$1
    result=0
    java -XX:+UseParallelGC -Xmx4g -jar "$JAR" -noTE -cleanup \
        -metadir "$WORK/$name.meta" -config "$2" "$3" >"$WORK/$name.log" 2>&1 || result=$?
    # Stop at the first fault/witness: exhaustive safety is a separate clean run.
    # Classify all error lines so parser/tool failures cannot count as a kill.
    unexpected=$(grep '^Error:' "$WORK/$name.log" | grep -vE \
        '^Error: (Invariant [A-Za-z0-9_]+ is violated\.|The behavior up to this point is:|Temporal properties were violated\.|Temporal property [A-Za-z0-9_]+ was violated\.|The following behavior constitutes a counter-example:)$' || true)
    if [ -n "$unexpected" ] || \
       grep -qE 'Semantic errors:|Exception|OutOfMemory|Parsing or semantic analysis failed' "$WORK/$name.log" || \
       ! grep -q 'states generated' "$WORK/$name.log" || \
       ! grep -q 'Finished in' "$WORK/$name.log"; then
        echo "FAIL: $name did not finish a valid model check (exit $result)"
        cat "$WORK/$name.log"
        exit 1
    fi
}

expect_clean() {
    echo "[ssh model gate] $1: expect clean safety/progress checks"
    run_model "$1" "$2" "$3"
    if [ "$result" -ne 0 ] || grep -q '^Error:' "$WORK/$name.log" || \
       ! grep -q 'Model checking completed. No error' "$WORK/$name.log"; then
        cat "$WORK/$name.log"
        exit 1
    fi
    grep -E 'states generated|depth of the complete|Finished in' "$WORK/$name.log"
}

expect_kills() {
    label=$1 config=$2 module=$3 temporal=$4
    shift 4
    echo "[ssh model gate] $label: expect $*${temporal:+ (temporal=$temporal)}"
    run_model "$label" "$config" "$module"
    got=$(grep -oE 'Invariant [A-Za-z0-9_]+ is violated' "$WORK/$name.log" | sort -u)
    want=""
    if [ "$#" -gt 0 ]; then want=$(printf 'Invariant %s is violated\n' "$@" | sort -u); fi
    if [ "$got" != "$want" ]; then
        echo "FAIL: $label violated a different invariant set"
        cat "$WORK/$name.log"
        exit 1
    fi
    if [ "$temporal" != no ]; then
        grep -q "Temporal property $temporal was violated" "$WORK/$name.log" || {
            echo "FAIL: $label did not kill its sole configured temporal property"
            cat "$WORK/$name.log"; exit 1;
        }
    elif grep -q 'Temporal propert' "$WORK/$name.log"; then
        echo "FAIL: unexpected temporal violation in $label"
        cat "$WORK/$name.log"; exit 1
    fi
    grep -E 'states generated|Finished in' "$WORK/$name.log"
}

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
