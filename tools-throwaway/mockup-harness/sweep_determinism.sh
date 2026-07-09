#!/usr/bin/env bash
# Re-render the entire gallery twice and prove all artifact bytes are stable.

set -euo pipefail

HARNESS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HARNESS_DIR/../.." && pwd)"
OUT_DIR="$REPO_ROOT/plans/assets/mockups-v2"
first_manifest="$(mktemp)"
second_manifest="$(mktemp)"
trap 'rm -f "$first_manifest" "$second_manifest"' EXIT

render_manifest() {
    (
        cd "$HARNESS_DIR"
        cargo run --quiet --features png -- --all >/dev/null
    )
    (
        cd "$OUT_DIR"
        find . -type f \( -name '*.txt' -o -name '*.png' \) -print0 |
            sort -z |
            xargs -0 sha256sum
    )
}

render_manifest >"$first_manifest"
render_manifest >"$second_manifest"
diff -u "$first_manifest" "$second_manifest"

printf 'sweep_determinism.sh: OK, identical sha256 manifests for %d artifacts\n' \
    "$(wc -l <"$first_manifest")"
