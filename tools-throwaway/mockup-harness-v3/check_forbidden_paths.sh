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

REPO_ROOT="$REPO_ROOT" BASELINE="$BASELINE" python3 - <<'PY'
import json
import os
import subprocess
import sys

repository = os.environ["REPO_ROOT"]
baseline_path = os.environ["BASELINE"]
try:
    baseline = json.load(open(baseline_path, encoding="utf-8"))
except (OSError, json.JSONDecodeError) as error:
    raise SystemExit(f"check_forbidden_paths.sh: invalid baseline: {error}")

allowed_prefixes = (
    "tools-throwaway/mockup-harness-v3/",
    "plans/assets/mockups-v3/",
    "plans/assets/mockups-v3-gates/",
)
if baseline.get("schema_version") != 1:
    raise SystemExit("check_forbidden_paths.sh: unsupported schema_version")
if baseline.get("status_format") != "git status --porcelain=v1 -z":
    raise SystemExit("check_forbidden_paths.sh: unsupported status format")
if baseline.get("excluded_v3_prefixes") != list(allowed_prefixes):
    raise SystemExit("check_forbidden_paths.sh: v3 exclusions do not match contract")

def status_entries() -> list[dict[str, str]]:
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
            raise SystemExit(
                f"check_forbidden_paths.sh: invalid porcelain record: {field!r}"
            )
        status = field[:2].decode("ascii")
        path = field[3:].decode("utf-8", "surrogateescape")
        original_path = None
        if "R" in status or "C" in status:
            if index >= len(fields):
                raise SystemExit(
                    f"check_forbidden_paths.sh: missing original path for {path!r}"
                )
            original_path = fields[index].decode("utf-8", "surrogateescape")
            index += 1
        if path.startswith(allowed_prefixes):
            continue
        entry = {"path": path, "status": status}
        if original_path is not None:
            entry["original_path"] = original_path
        entries.append(entry)
    return sorted(
        entries,
        key=lambda entry: (
            entry["path"],
            entry["status"],
            entry.get("original_path", ""),
        ),
    )

expected = baseline.get("entries")
if not isinstance(expected, list):
    raise SystemExit("check_forbidden_paths.sh: baseline entries are missing")
actual = status_entries()
if actual != expected:
    print("check_forbidden_paths.sh: forbidden status baseline mismatch", file=sys.stderr)
    print(
        json.dumps({"expected": expected, "actual": actual}, indent=2, sort_keys=True),
        file=sys.stderr,
    )
    raise SystemExit(1)

print(
    f"check_forbidden_paths.sh: OK, {len(actual)} off-limits status entries match"
)
PY
