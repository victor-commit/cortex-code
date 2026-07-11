#!/usr/bin/env bash
# Create the mission-local, append-only commit ledger exactly once.
set -euo pipefail

LC_ALL=C
export LC_ALL
umask 077

HARNESS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HARNESS_DIR/../.." && pwd)"
LEDGER_DIR="$HARNESS_DIR/.isolation_state"
LEDGER="$LEDGER_DIR/mission_commits.ledger"
SEED="$HARNESS_DIR/mission_commits.seed"

[[ ! -L "$LEDGER_DIR" ]] || {
    echo "initialize_mission_ledger.sh: ledger state directory must not be a symlink" >&2
    exit 2
}
if [[ -e "$LEDGER_DIR" ]]; then
    [[ -d "$LEDGER_DIR" ]] || {
        echo "initialize_mission_ledger.sh: ledger state path is not a directory" >&2
        exit 2
    }
else
    mkdir -m 700 "$LEDGER_DIR"
fi

if [[ -e "$LEDGER" || -L "$LEDGER" ]]; then
    [[ -f "$LEDGER" && ! -L "$LEDGER" ]] || {
        echo "initialize_mission_ledger.sh: mission commit ledger is not a regular file" >&2
        exit 2
    }
else
    temporary="$(mktemp "$LEDGER_DIR/.mission_commits.XXXXXX")"
    trap 'rm -f "$temporary"' EXIT
    : >"$temporary"
    chmod 600 "$temporary"
    if ! ln "$temporary" "$LEDGER"; then
        [[ -f "$LEDGER" && ! -L "$LEDGER" ]] || {
            echo "initialize_mission_ledger.sh: could not create mission commit ledger" >&2
            exit 2
        }
    fi
    rm -f "$temporary"
    trap - EXIT
fi

[[ -f "$SEED" && ! -L "$SEED" ]] || {
    echo "initialize_mission_ledger.sh: missing trusted mission commit seed" >&2
    exit 2
}
mapfile -t seed_commits <"$SEED"
for commit in "${seed_commits[@]}"; do
    [[ "$commit" =~ ^[0-9a-f]{40,64}$ ]] || {
        echo "initialize_mission_ledger.sh: invalid trusted mission commit seed" >&2
        exit 2
    }
    canonical="$(git -C "$REPO_ROOT" rev-parse --verify "${commit}^{commit}" 2>/dev/null)" || {
        echo "initialize_mission_ledger.sh: trusted mission commit $commit is unreachable" >&2
        exit 2
    }
    [[ "$canonical" == "$commit" ]] || {
        echo "initialize_mission_ledger.sh: trusted mission commit seed is not canonical" >&2
        exit 2
    }
    git -C "$REPO_ROOT" merge-base --is-ancestor "$commit" HEAD >/dev/null 2>&1 || {
        echo "initialize_mission_ledger.sh: trusted mission commit $commit is not reachable from HEAD" >&2
        exit 2
    }
done

if (( ${#seed_commits[@]} == 0 )); then
    expected=""
else
    expected="$(printf '%s\n' "${seed_commits[@]}")"
fi
current="$(cat "$LEDGER")"
if [[ -z "$current" && -n "$expected" ]]; then
    printf '%s\n' "$expected" >>"$LEDGER" || {
        echo "initialize_mission_ledger.sh: could not seed mission commit ledger" >&2
        exit 2
    }
elif [[ "$current" != "$expected" && "$current" != "$expected"$'\n'* ]]; then
    echo "initialize_mission_ledger.sh: mission commit ledger does not match trusted seed" >&2
    exit 2
fi

printf 'initialize_mission_ledger.sh: initialized %s with %s migration commits\n' \
    "$LEDGER" "${#seed_commits[@]}"
