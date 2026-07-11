#!/usr/bin/env bash
# Commit staged mission work and atomically attribute the resulting SHA.
set -euo pipefail

LC_ALL=C
export LC_ALL

HARNESS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HARNESS_DIR/../.." && pwd)"
LEDGER="$HARNESS_DIR/.isolation_state/mission_commits.ledger"

[[ $# -ge 1 ]] || {
    echo "usage: commit_mission_change.sh <git-commit-argument> [...]" >&2
    exit 2
}

[[ -f "$LEDGER" && ! -L "$LEDGER" ]] || {
    echo "commit_mission_change.sh: missing mission commit ledger" >&2
    exit 2
}
git -C "$REPO_ROOT" -c user.name="echobt" \
    -c user.email="154886644+echobt@users.noreply.github.com" \
    commit "$@"
commit="$(git -C "$REPO_ROOT" rev-parse HEAD)"
"$HARNESS_DIR/record_mission_commit.sh" "$commit"
