#!/usr/bin/env bash
# Verify mission-authored commits remain within approved roots.
set -euo pipefail

LC_ALL=C
export LC_ALL

HARNESS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HARNESS_DIR/../.." && pwd)"
SNAPSHOT="$HARNESS_DIR/.isolation-session/active.json"

[[ -f "$SNAPSHOT" ]] || {
    echo "audit_mission_commits.sh: missing active worker-session snapshot" >&2
    exit 2
}

python3 "$HARNESS_DIR/audit_mission_commits.py" "$REPO_ROOT" "$SNAPSHOT"
