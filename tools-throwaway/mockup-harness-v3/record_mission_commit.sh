#!/usr/bin/env bash
# Append one newly created mission commit to the append-only local ledger.
set -euo pipefail

LC_ALL=C
export LC_ALL
umask 077

HARNESS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HARNESS_DIR/../.." && pwd)"
LEDGER_DIR="$HARNESS_DIR/.isolation_state"
LEDGER="$LEDGER_DIR/mission_commits.ledger"
LOCK_DIR="$LEDGER_DIR/.mission_commits.lock"

[[ $# -eq 1 ]] || {
    echo "usage: record_mission_commit.sh <commit-sha>" >&2
    exit 2
}
commit="$1"
[[ "$commit" =~ ^[0-9a-f]{40,64}$ ]] || {
    echo "record_mission_commit.sh: commit SHA must be a full lowercase SHA" >&2
    exit 2
}
[[ -f "$LEDGER" && ! -L "$LEDGER" ]] || {
    echo "record_mission_commit.sh: missing mission commit ledger" >&2
    exit 2
}
if [[ "${MISSION_COMMIT_LOCK_HELD:-}" == 1 ]]; then
    [[ -d "$LOCK_DIR" && ! -L "$LOCK_DIR" ]] || {
        echo "record_mission_commit.sh: caller does not hold the mission commit lock" >&2
        exit 2
    }
else
    if ! mkdir -m 700 "$LOCK_DIR"; then
        echo "record_mission_commit.sh: mission commit ledger is locked or unavailable" >&2
        exit 2
    fi
    trap 'rmdir "$LOCK_DIR"' EXIT
fi

canonical="$(git -C "$REPO_ROOT" rev-parse --verify "${commit}^{commit}" 2>/dev/null)" || {
    echo "record_mission_commit.sh: commit $commit is unreachable" >&2
    exit 2
}
[[ "$canonical" == "$commit" ]] || {
    echo "record_mission_commit.sh: commit SHA must be canonical" >&2
    exit 2
}
git -C "$REPO_ROOT" merge-base --is-ancestor "$commit" HEAD >/dev/null 2>&1 || {
    echo "record_mission_commit.sh: commit $commit is not reachable from HEAD" >&2
    exit 2
}
grep -Fqx -- "$commit" "$LEDGER" && {
    echo "record_mission_commit.sh: commit $commit is already ledgered" >&2
    exit 2
}

printf '%s\n' "$commit" >>"$LEDGER" || {
    echo "record_mission_commit.sh: could not append mission commit ledger" >&2
    exit 2
}
grep -Fqx -- "$commit" "$LEDGER" || {
    echo "record_mission_commit.sh: ledger append verification failed" >&2
    exit 2
}

if [[ "${MISSION_COMMIT_LOCK_HELD:-}" != 1 ]]; then
    trap - EXIT
    rmdir "$LOCK_DIR"
fi
printf 'record_mission_commit.sh: ledgered %s\n' "$commit"
