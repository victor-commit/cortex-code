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

REPO_ROOT="$REPO_ROOT" HEAD_COMMIT="$head_commit" python3 - >"$status_tmp" <<'PY'
import json
import os
import subprocess
import sys

repository = os.environ["REPO_ROOT"]
allowed_prefixes = (
    "tools-throwaway/mockup-harness-v3/",
    "plans/assets/mockups-v3/",
    "plans/assets/mockups-v3-gates/",
)
raw = subprocess.check_output(
    ["git", "-C", repository, "status", "--porcelain=v1", "-z"]
)
fields = raw.split(b"\0")
entries = []
index = 0
while index < len(fields):
    field = fields[index]
    index += 1
    if not field:
        continue
    if len(field) < 4:
        raise SystemExit(f"invalid porcelain-v1 record: {field!r}")
    status = field[:2].decode("ascii")
    path = field[3:].decode("utf-8", "surrogateescape")
    original_path = None
    if "R" in status or "C" in status:
        if index >= len(fields):
            raise SystemExit(f"missing original path for {path!r}")
        original_path = fields[index].decode("utf-8", "surrogateescape")
        index += 1
    if path.startswith(allowed_prefixes):
        continue
    entry = {"path": path, "status": status}
    if original_path is not None:
        entry["original_path"] = original_path
    entries.append(entry)

entries.sort(key=lambda entry: (
    entry["path"],
    entry["status"],
    entry.get("original_path", ""),
))
json.dump(
    {
        "schema_version": 1,
        "head_commit": os.environ["HEAD_COMMIT"],
        "status_format": "git status --porcelain=v1 -z",
        "excluded_v3_prefixes": list(allowed_prefixes),
        "entries": entries,
    },
    sys.stdout,
    indent=2,
    sort_keys=True,
)
sys.stdout.write("\n")
PY

mv "$manifest_tmp" "$MANIFEST"
mv "$metadata_tmp" "$METADATA"
mv "$status_tmp" "$FORBIDDEN_STATUS"
trap - EXIT

printf 'record_baseline.sh: recorded %s frozen tracked files and %s forbidden status entries\n' \
    "$file_count" \
    "$(python3 -c 'import json, sys; print(len(json.load(open(sys.argv[1]))["entries"]))' "$FORBIDDEN_STATUS")"
