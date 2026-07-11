#!/usr/bin/env bash
# Record the active worker-session snapshot without reading untracked contents.
set -euo pipefail

LC_ALL=C
export LC_ALL

HARNESS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HARNESS_DIR/../.." && pwd)"
source "$HARNESS_DIR/isolation_session_state.sh"
STATE_DIR="$V3_ISOLATION_STATE_DIR"
STATE_MARKER="$V3_ISOLATION_STATE_MARKER"
STATE_ID="$V3_ISOLATION_STATE_ID"
SESSION_ID="${V3_ISOLATION_SESSION_ID:?start_session_snapshot.sh: V3_ISOLATION_SESSION_ID is required}"
[[ "$SESSION_ID" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$ ]] || {
    echo "start_session_snapshot.sh: invalid worker session identifier" >&2
    exit 2
}
SNAPSHOT="$STATE_DIR/snapshots/$SESSION_ID.json"
SESSION_BINDING="$STATE_DIR/session-bindings/$SESSION_ID.json"
CURRENT_SESSION="$STATE_DIR/current-session"

"$HARNESS_DIR/initialize_mission_ledger.sh"

MARKER_TEMP="$(mktemp "$(dirname "$STATE_MARKER")/.isolation-guard-v3.initialized.XXXXXX")"
STATE_ID_TEMP="$(mktemp "$(dirname "$STATE_MARKER")/.isolation-guard-v3.state-id.XXXXXX")"
trap 'rm -f "$MARKER_TEMP" "$STATE_ID_TEMP"' EXIT
HEAD_COMMIT="$(git -C "$REPO_ROOT" rev-parse HEAD)"
if [[ ! -e "$STATE_MARKER" ]]; then
    if [[ -e "$STATE_DIR" ]]; then
        echo "start_session_snapshot.sh: missing persistent mission state marker" >&2
        exit 2
    fi
    mkdir -m 700 "$STATE_DIR"
    STATE_ID_VALUE="v1:$(printf '%s\0%s\n' "$HEAD_COMMIT" "$STATE_ID_TEMP" | sha256sum | cut -d' ' -f1)"
    printf '%s\n' "$STATE_ID_VALUE" >"$STATE_ID_TEMP"
    if ! ln "$STATE_ID_TEMP" "$STATE_ID"; then
        [[ -e "$STATE_ID" ]] || {
            echo "start_session_snapshot.sh: could not initialize persistent mission state" >&2
            exit 2
        }
    fi
    printf '%s\n' "$STATE_ID_VALUE" >"$MARKER_TEMP"
    if ! ln "$MARKER_TEMP" "$STATE_MARKER"; then
        [[ -e "$STATE_MARKER" ]] || {
            echo "start_session_snapshot.sh: could not initialize persistent mission state" >&2
            exit 2
        }
    fi
else
    isolation_require_state || exit $?
fi
rm -f "$MARKER_TEMP"
rm -f "$STATE_ID_TEMP"
trap - EXIT
mkdir -p -m 700 "$STATE_DIR/snapshots" "$STATE_DIR/session-bindings"
if [[ -e "$SNAPSHOT" || -e "$SESSION_BINDING" ]]; then
    echo "start_session_snapshot.sh: worker session $SESSION_ID is already bound to an immutable snapshot" >&2
    exit 2
fi
if [[ -r "$CURRENT_SESSION" ]] && [[ "$(<"$CURRENT_SESSION")" == "$SESSION_ID" ]]; then
    echo "start_session_snapshot.sh: worker session $SESSION_ID has a missing immutable snapshot" >&2
    exit 2
fi

TEMP="$(mktemp "$STATE_DIR/.${SESSION_ID}.XXXXXX")"
CURRENT_TEMP="$(mktemp "$STATE_DIR/.current-session.XXXXXX")"
trap 'rm -f "$TEMP" "$CURRENT_TEMP"' EXIT
python3 "$HARNESS_DIR/isolation_snapshot.py" \
    record "$REPO_ROOT" "$HEAD_COMMIT" "$SESSION_ID" >"$TEMP"

if ! ln "$TEMP" "$SESSION_BINDING"; then
    echo "start_session_snapshot.sh: worker session $SESSION_ID is already bound to an immutable snapshot" >&2
    exit 2
fi
if ! ln "$SESSION_BINDING" "$SNAPSHOT"; then
    echo "start_session_snapshot.sh: could not publish immutable worker session $SESSION_ID" >&2
    exit 2
fi
rm -f "$TEMP"
printf '%s\n' "$SESSION_ID" >"$CURRENT_TEMP"
mv -f "$CURRENT_TEMP" "$CURRENT_SESSION"
trap - EXIT

printf 'start_session_snapshot.sh: recorded immutable worker session %s at %s\n' \
    "$SESSION_ID" "$HEAD_COMMIT"
