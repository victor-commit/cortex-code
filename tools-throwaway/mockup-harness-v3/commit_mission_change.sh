#!/usr/bin/env bash
# Commit staged mission work and atomically attribute the resulting SHA.
set -euo pipefail

LC_ALL=C
export LC_ALL

HARNESS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HARNESS_DIR/../.." && pwd)"
LEDGER="$HARNESS_DIR/.isolation_state/mission_commits.ledger"
LOCK_DIR="$HARNESS_DIR/.isolation_state/.mission_commits.lock"

[[ $# -ge 1 ]] || {
    echo "usage: commit_mission_change.sh <git-commit-argument> [...]" >&2
    exit 2
}

[[ -f "$LEDGER" && ! -L "$LEDGER" ]] || {
    echo "commit_mission_change.sh: missing mission commit ledger" >&2
    exit 2
}

if ! mkdir -m 700 "$LOCK_DIR"; then
    echo "commit_mission_change.sh: mission commit ledger is locked or unavailable" >&2
    exit 2
fi
trap 'rmdir "$LOCK_DIR"' EXIT

# Git writes the new HEAD and its reflog entry as one ref update. Give this
# invocation a unique reflog action so its exact commit can be recovered even
# if an unrelated commit moves HEAD before the ledger append.
reflog_action="commit_mission_change.sh[$$-$(date +%s%N)]"
GIT_REFLOG_ACTION="$reflog_action" \
git -C "$REPO_ROOT" -c user.name="echobt" \
    -c user.email="154886644+echobt@users.noreply.github.com" \
    commit "$@"

reflog_entries="$(git -C "$REPO_ROOT" reflog --format='%H%x09%gs' HEAD)" || {
    echo "commit_mission_change.sh: could not read HEAD reflog to identify its commit" >&2
    exit 2
}
commit=""
matches=0
while IFS=$'\t' read -r candidate action; do
    if [[ "$action" == "$reflog_action:"* ]]; then
        commit="$candidate"
        matches=$((matches + 1))
    fi
done <<<"$reflog_entries"
if [[ "$matches" -ne 1 || ! "$commit" =~ ^[0-9a-f]{40,64}$ ]]; then
    echo "commit_mission_change.sh: could not uniquely identify the commit created by this operation; refusing to ledger HEAD" >&2
    exit 2
fi

if [[ -n "${COMMIT_MISSION_CHANGE_TEST_AFTER_COMMIT:-}" ]]; then
    "$COMMIT_MISSION_CHANGE_TEST_AFTER_COMMIT"
fi

MISSION_COMMIT_LOCK_HELD=1 "$HARNESS_DIR/record_mission_commit.sh" "$commit"
