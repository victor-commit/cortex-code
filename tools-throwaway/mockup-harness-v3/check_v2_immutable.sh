#!/usr/bin/env bash
# Verify the one-time v1/v2 tracked-file hash baseline without writing files.
set -euo pipefail

LC_ALL=C
export LC_ALL

HARNESS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HARNESS_DIR/../.." && pwd)"
BASELINES_DIR="$HARNESS_DIR/baselines"
MANIFEST="$BASELINES_DIR/frozen-references.sha256"
METADATA="$BASELINES_DIR/frozen-references.json"

[[ -f "$MANIFEST" ]] || {
    echo "check_v2_immutable.sh: missing frozen reference manifest" >&2
    exit 2
}
[[ -f "$METADATA" ]] || {
    echo "check_v2_immutable.sh: missing frozen reference metadata" >&2
    exit 2
}

python3 - "$REPO_ROOT" "$MANIFEST" "$METADATA" <<'PY'
import hashlib
import json
import subprocess
import sys
from pathlib import Path

repository = Path(sys.argv[1])
manifest_path = Path(sys.argv[2])
metadata_path = Path(sys.argv[3])
roots = (
    "tools-throwaway/mockup-harness/",
    "plans/assets/mockups-v2/",
    "plans/assets/mockups/",
)

try:
    metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
except (OSError, json.JSONDecodeError) as error:
    raise SystemExit(f"invalid frozen reference metadata: {error}")

if metadata.get("schema_version") != 1:
    raise SystemExit("frozen reference metadata has unsupported schema_version")
if metadata.get("hash_algorithm") != "sha256":
    raise SystemExit("frozen reference metadata does not declare sha256")
if metadata.get("frozen_roots") != [root.rstrip("/") for root in roots]:
    raise SystemExit("frozen reference metadata roots do not match the contract")

manifest_bytes = manifest_path.read_bytes()
if metadata.get("manifest_sha256") != hashlib.sha256(manifest_bytes).hexdigest():
    raise SystemExit("frozen reference metadata manifest digest does not match")

manifest_paths = []
for number, line in enumerate(manifest_bytes.decode("utf-8").splitlines(), 1):
    try:
        digest, path = line.split("  ", 1)
    except ValueError:
        raise SystemExit(f"manifest line {number} is not sha256sum format")
    if len(digest) != 64 or any(char not in "0123456789abcdef" for char in digest):
        raise SystemExit(f"manifest line {number} has an invalid SHA-256 digest")
    if not path.startswith(roots):
        raise SystemExit(f"manifest line {number} is outside frozen roots: {path}")
    manifest_paths.append(path)

if manifest_paths != sorted(manifest_paths):
    raise SystemExit("frozen reference manifest paths are not sorted")
if len(manifest_paths) != len(set(manifest_paths)):
    raise SystemExit("frozen reference manifest contains duplicate paths")
if metadata.get("tracked_file_count") != len(manifest_paths):
    raise SystemExit("frozen reference metadata file count does not match")

tracked = subprocess.check_output(
    [
        "git",
        "-C",
        str(repository),
        "ls-files",
        "-z",
        "--",
        *[root.rstrip("/") for root in roots],
    ]
).split(b"\0")
tracked_paths = sorted(
    path.decode("utf-8", "surrogateescape") for path in tracked if path
)
if manifest_paths != tracked_paths:
    raise SystemExit("frozen reference manifest paths differ from tracked files")
PY

(
    cd "$REPO_ROOT"
    sha256sum --check --status "$MANIFEST"
)

printf 'check_v2_immutable.sh: OK, %s tracked frozen references match\n' \
    "$(wc -l <"$MANIFEST" | tr -d ' ')"
