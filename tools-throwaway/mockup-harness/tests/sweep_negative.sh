#!/usr/bin/env bash
# Isolated semantic-audit bypass proofs. Each fixture is an in-memory buffer
# report, so gallery artifacts and registered screen sources remain untouched.

set -euo pipefail

HARNESS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
failures=0

expect_failure() {
    local fixture="$1"
    local role="$2"
    local output
    output="$(mktemp)"
    if python3 "$HARNESS_DIR/sweep_audit.py" --fixture "$fixture" >"$output" 2>&1; then
        printf 'sweep_negative.sh: %s unexpectedly passed\n' "$fixture"
        failures=$((failures + 1))
    elif ! grep -Eq ':[0-9]+: \['"$role"'\]' "$output"; then
        printf 'sweep_negative.sh: %s did not report file:line [%s]\n' "$fixture" "$role"
        cat "$output"
        failures=$((failures + 1))
    fi
    rm -f "$output"
}

expect_success() {
    local fixture="$1"
    local output
    output="$(mktemp)"
    if ! python3 "$HARNESS_DIR/sweep_audit.py" --fixture "$fixture" >"$output" 2>&1; then
        printf 'sweep_negative.sh: %s unexpectedly failed\n' "$fixture"
        cat "$output"
        failures=$((failures + 1))
    fi
    rm -f "$output"
}

expect_failure cross-002 key
expect_failure cross-003-unlisted live_activity
expect_failure cross-003-source-unlisted live_activity
expect_failure cross-003-inline-helper-unlisted live_activity
expect_failure cross-003-duplicate live_activity
expect_success cross-004-valid
expect_success cross-004-fractional
expect_failure cross-004-malformed duration
expect_failure cross-004-mixed duration
expect_failure cross-007-bright dot
expect_failure cross-007-txt dot
expect_failure cross-009-current-unlisted selection
expect_failure cross-009-current-style selection

if [[ $failures -gt 0 ]]; then
    printf 'sweep_negative.sh: FAIL, %d fixture(s) lacked diagnostics\n' "$failures"
    exit 1
fi

printf 'sweep_negative.sh: OK, semantic bypass fixtures rejected with file:line:role diagnostics\n'
