#!/usr/bin/env bash
# Prove every commit reported by this worker is in the mission commit ledger.
set -euo pipefail

LC_ALL=C
export LC_ALL

HARNESS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

[[ $# -ge 1 ]] || {
    echo "usage: check_mission_handoff.sh <reported-commit-sha> [...]" >&2
    exit 2
}

arguments=()
for commit in "$@"; do
    arguments+=(--reported-commit "$commit")
done

"$HARNESS_DIR/audit_mission_commits.sh" "${arguments[@]}"
