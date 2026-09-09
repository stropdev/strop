#!/bin/sh
# Shared strict result classification; callers own JAR, WORK and cleanup.
run_model() {
    name=$1
    result=0
    java -XX:+UseParallelGC -Xmx4g -jar "$JAR" -cleanup \
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
    echo "[model gate] $1: expect clean safety/progress checks"
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
    echo "[model gate] $label: expect $*${temporal:+ (temporal=$temporal)}"
    run_model "$label" "$config" "$module"
    if [ "$result" -eq 0 ]; then
        echo "FAIL: $label reported success instead of rejecting the fault"
        cat "$WORK/$name.log"; exit 1
    fi
    got=$(grep -oE 'Invariant [A-Za-z0-9_]+ is violated' "$WORK/$name.log" | sort -u)
    want=""
    if [ "$#" -gt 0 ]; then want=$(printf 'Invariant %s is violated\n' "$@" | sort -u); fi
    if [ "$got" != "$want" ]; then
        echo "FAIL: $label violated a different invariant set"
        cat "$WORK/$name.log"
        exit 1
    fi
    if [ "$temporal" != no ]; then
        # Stable TLC reports a collective temporal violation. Attribute it only
        # when the fault config ends with exactly the one expected property.
        declared_properties=$(sed -n '/^[[:space:]]*PROPERT[YI]/,$p' "$config" \
            | tr '\n\t' '  ' | sed 's/^ *//; s/ *$//; s/  */ /g')
        if [ "$declared_properties" != "PROPERTY $temporal" ]; then
            echo "FAIL: $label must configure only the final PROPERTY $temporal"
            exit 1
        fi
        grep -q '^Error: Temporal properties were violated\.$' "$WORK/$name.log" || {
            echo "FAIL: $label did not reject its sole configured temporal property"
            cat "$WORK/$name.log"; exit 1;
        }
    elif grep -q 'Temporal propert' "$WORK/$name.log"; then
        echo "FAIL: unexpected temporal violation in $label"
        cat "$WORK/$name.log"; exit 1
    fi
    grep -E 'states generated|Finished in' "$WORK/$name.log"
}
