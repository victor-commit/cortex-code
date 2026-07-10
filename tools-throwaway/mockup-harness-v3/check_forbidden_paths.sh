#!/usr/bin/env bash
# Verify that no path outside the three mission-owned v3 roots changed status.
set -euo pipefail

LC_ALL=C
export LC_ALL

HARNESS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HARNESS_DIR/../.." && pwd)"
BASELINE="$HARNESS_DIR/baselines/forbidden-status.json"

[[ -f "$BASELINE" ]] || {
    echo "check_forbidden_paths.sh: missing forbidden-path status baseline" >&2
    exit 2
}

python3 "$HARNESS_DIR/forbidden_path_baseline.py" \
    verify "$REPO_ROOT" "$BASELINE"
