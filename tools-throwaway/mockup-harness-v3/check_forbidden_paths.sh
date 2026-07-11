#!/usr/bin/env bash
# Verify no forbidden path changed during the active worker session.
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
    echo "check_forbidden_paths.sh: missing active worker-session identity" >&2
    exit 2
fi
[[ "$SESSION_ID" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$ ]] || {
    echo "check_forbidden_paths.sh: invalid worker session identifier" >&2
    exit 2
}
SNAPSHOT="$STATE_DIR/snapshots/$SESSION_ID.json"

[[ -f "$SNAPSHOT" ]] || {
    echo "check_forbidden_paths.sh: missing active worker-session snapshot" >&2
    exit 2
}

python3 "$HARNESS_DIR/isolation_snapshot.py" \
    verify "$REPO_ROOT" "$SNAPSHOT" "$SESSION_ID"
