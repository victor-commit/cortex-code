#!/usr/bin/env bash
# Verify mission-authored commits remain within approved roots.
set -euo pipefail

LC_ALL=C
export LC_ALL

HARNESS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HARNESS_DIR/../.." && pwd)"
source "$HARNESS_DIR/isolation_session_state.sh"
STATE_DIR="$V3_ISOLATION_STATE_DIR"
CURRENT_SESSION="$STATE_DIR/current-session"
isolation_require_state || exit $?
if [[ -n "${V3_ISOLATION_SESSION_ID:-}" ]]; then
    SESSION_ID="$V3_ISOLATION_SESSION_ID"
elif [[ -r "$CURRENT_SESSION" ]]; then
    SESSION_ID="$(<"$CURRENT_SESSION")"
else
    echo "audit_mission_commits.sh: missing active worker-session identity" >&2
    exit 2
fi
[[ "$SESSION_ID" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$ ]] || {
    echo "audit_mission_commits.sh: invalid worker session identifier" >&2
    exit 2
}
SNAPSHOT="$STATE_DIR/snapshots/$SESSION_ID.json"
LEDGER_DIR="$HARNESS_DIR/.isolation_state"
LEDGER="$LEDGER_DIR/mission_commits.ledger"

[[ -f "$SNAPSHOT" ]] || {
    echo "audit_mission_commits.sh: missing active worker-session snapshot" >&2
    exit 2
}
reported_commits=()
while [[ $# -gt 0 ]]; do
    case "$1" in
        --reported-commit)
            [[ $# -ge 2 ]] || {
                echo "audit_mission_commits.sh: --reported-commit requires a SHA" >&2
                exit 2
            }
            reported_commits+=("$2")
            shift 2
            ;;
        *)
            echo "usage: audit_mission_commits.sh [--reported-commit <SHA> ...]" >&2
            exit 2
            ;;
    esac
done

python3 "$HARNESS_DIR/audit_mission_commits.py" \
    "$REPO_ROOT" "$SNAPSHOT" "$SESSION_ID" "$LEDGER" "${reported_commits[@]}"
