#!/usr/bin/env bash
# Create the one-time immutable comparison baseline for the v3 design mission.
set -euo pipefail

LC_ALL=C
export LC_ALL

HARNESS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HARNESS_DIR/../.." && pwd)"
BASELINES_DIR="$HARNESS_DIR/baselines"
MANIFEST="$BASELINES_DIR/frozen-references.sha256"
METADATA="$BASELINES_DIR/frozen-references.json"
FORBIDDEN_STATUS="$BASELINES_DIR/forbidden-status.json"

for baseline in "$MANIFEST" "$METADATA" "$FORBIDDEN_STATUS"; do
    if [[ -e "$baseline" ]]; then
        echo "record_baseline.sh: refusing to overwrite $baseline" >&2
        exit 2
    fi
done

mkdir -p "$BASELINES_DIR"
manifest_tmp="$(mktemp "$BASELINES_DIR/.frozen-references.XXXXXX")"
metadata_tmp="$(mktemp "$BASELINES_DIR/.frozen-references.XXXXXX")"
status_tmp="$(mktemp "$BASELINES_DIR/.forbidden-status.XXXXXX")"
trap 'rm -f "$manifest_tmp" "$metadata_tmp" "$status_tmp"' EXIT

(
    cd "$REPO_ROOT"
    git ls-files -z -- \
        tools-throwaway/mockup-harness \
        plans/assets/mockups-v2 \
        plans/assets/mockups |
        sort -z |
        while IFS= read -r -d '' path; do
            sha256sum -- "$path"
        done
) >"$manifest_tmp"

file_count="$(wc -l <"$manifest_tmp" | tr -d ' ')"
if [[ "$file_count" -eq 0 ]]; then
    echo "record_baseline.sh: frozen reference inventory is empty" >&2
    exit 1
fi

head_commit="$(git -C "$REPO_ROOT" rev-parse HEAD)"
manifest_digest="$(sha256sum "$manifest_tmp" | cut -d' ' -f1)"

python3 - "$head_commit" "$file_count" "$manifest_digest" >"$metadata_tmp" <<'PY'
import json
import sys

head_commit, file_count, manifest_digest = sys.argv[1:]
json.dump(
    {
        "schema_version": 1,
        "head_commit": head_commit,
        "hash_algorithm": "sha256",
        "frozen_roots": [
            "tools-throwaway/mockup-harness",
            "plans/assets/mockups-v2",
            "plans/assets/mockups",
        ],
        "tracked_file_count": int(file_count),
        "manifest_sha256": manifest_digest,
        "manifest_format": "sha256sum, repository-relative path",
    },
    sys.stdout,
    indent=2,
    sort_keys=True,
)
sys.stdout.write("\n")
PY

python3 "$HARNESS_DIR/forbidden_path_baseline.py" \
    record "$REPO_ROOT" "$head_commit" >"$status_tmp"

mv "$manifest_tmp" "$MANIFEST"
mv "$metadata_tmp" "$METADATA"
mv "$status_tmp" "$FORBIDDEN_STATUS"
trap - EXIT

printf 'record_baseline.sh: recorded %s frozen tracked files and %s forbidden status entries\n' \
    "$file_count" \
    "$(python3 -c 'import json, sys; print(len(json.load(open(sys.argv[1]))["entries"]))' "$FORBIDDEN_STATUS")"
