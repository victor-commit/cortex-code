#!/usr/bin/env bash
# Verify no forbidden path changed during the active worker session.
set -euo pipefail

LC_ALL=C
export LC_ALL

HARNESS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HARNESS_DIR/../.." && pwd)"
SNAPSHOT="$HARNESS_DIR/.isolation-session/active.json"

[[ -f "$SNAPSHOT" ]] || {
    echo "check_forbidden_paths.sh: missing active worker-session snapshot" >&2
    exit 2
}

python3 "$HARNESS_DIR/isolation_snapshot.py" \
    verify "$REPO_ROOT" "$SNAPSHOT"
