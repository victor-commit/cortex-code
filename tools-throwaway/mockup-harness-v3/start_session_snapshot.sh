#!/usr/bin/env bash
# Record the active worker-session snapshot without reading untracked contents.
set -euo pipefail

LC_ALL=C
export LC_ALL

HARNESS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HARNESS_DIR/../.." && pwd)"
STATE_DIR="$HARNESS_DIR/.isolation-session"
SNAPSHOT="$STATE_DIR/active.json"
TEMP="$STATE_DIR/.active.json.$$"

mkdir -p "$STATE_DIR"
trap 'rm -f "$TEMP"' EXIT

HEAD_COMMIT="$(git -C "$REPO_ROOT" rev-parse HEAD)"
python3 "$HARNESS_DIR/isolation_snapshot.py" record "$REPO_ROOT" "$HEAD_COMMIT" >"$TEMP"
mv "$TEMP" "$SNAPSHOT"
trap - EXIT

printf 'start_session_snapshot.sh: recorded active worker session at %s\n' "$HEAD_COMMIT"
