#!/usr/bin/env bash
# Re-render the complete v3 gallery twice and prove canonical + gate digests match.
#
# A complete run is `--all` plus every public `--flow <name>` so the gate root
# carries the exact 35 TXT + 35 PNG ordered aliases. Ownership manifests and
# SHA-256 path digests are compared for both roots; any content or membership
# drift fails closed.

set -euo pipefail
LC_ALL=C
export LC_ALL

HARNESS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HARNESS_DIR/../.." && pwd)"
CANONICAL_ROOT="$REPO_ROOT/plans/assets/mockups-v3"
GATE_ROOT="$REPO_ROOT/plans/assets/mockups-v3-gates"
FLOWS=(
  daily-work
  input-and-approval
  work-configuration
  provider-and-mcp-connection
  recovery
  mission
)

first_dir="$(mktemp -d)"
second_dir="$(mktemp -d)"
trap 'rm -rf "$first_dir" "$second_dir"' EXIT

harness() {
  (
    cd "$HARNESS_DIR"
    cargo run --quiet --features png --bin mockup-harness-v3 -- "$@"
  )
}

render_complete() {
  harness --all >/dev/null
  local flow
  for flow in "${FLOWS[@]}"; do
    harness --flow "$flow" >/dev/null
  done
}

artifact_manifest() {
  local root="$1"
  (
    cd "$root"
    find . -type f \( -name '*.txt' -o -name '*.png' \) -print0 |
      sort -z |
      xargs -0 sha256sum
  )
}

scope_digest() {
  local file="$1"
  sha256sum "$file" | awk '{print $1}'
}

render_complete
artifact_manifest "$CANONICAL_ROOT" >"$first_dir/canonical.sha256"
artifact_manifest "$GATE_ROOT" >"$first_dir/gate.sha256"
cp "$CANONICAL_ROOT/.renderer-manifest-v1" "$first_dir/canonical.ownership"
cp "$GATE_ROOT/.renderer-manifest-v1" "$first_dir/gate.ownership"
cp "$CANONICAL_ROOT/README.md" "$first_dir/README.md"
(
  cd "$HARNESS_DIR"
  cargo run --quiet --features png --bin mockup-harness-v3 -- --audit-json --all
) >"$first_dir/audit.json"

render_complete
artifact_manifest "$CANONICAL_ROOT" >"$second_dir/canonical.sha256"
artifact_manifest "$GATE_ROOT" >"$second_dir/gate.sha256"
cp "$CANONICAL_ROOT/.renderer-manifest-v1" "$second_dir/canonical.ownership"
cp "$GATE_ROOT/.renderer-manifest-v1" "$second_dir/gate.ownership"
cp "$CANONICAL_ROOT/README.md" "$second_dir/README.md"
(
  cd "$HARNESS_DIR"
  cargo run --quiet --features png --bin mockup-harness-v3 -- --audit-json --all
) >"$second_dir/audit.json"

diff -u "$first_dir/canonical.sha256" "$second_dir/canonical.sha256"
diff -u "$first_dir/gate.sha256" "$second_dir/gate.sha256"
diff -u "$first_dir/canonical.ownership" "$second_dir/canonical.ownership"
diff -u "$first_dir/gate.ownership" "$second_dir/gate.ownership"
diff -u "$first_dir/README.md" "$second_dir/README.md"
diff -u "$first_dir/audit.json" "$second_dir/audit.json"

canonical_count="$(wc -l <"$first_dir/canonical.sha256" | tr -d ' ')"
gate_count="$(wc -l <"$first_dir/gate.sha256" | tr -d ' ')"
if [[ "$canonical_count" -ne 172 ]]; then
  echo "sweep_determinism.sh: expected 172 canonical artifacts, found $canonical_count" >&2
  exit 1
fi
if [[ "$gate_count" -ne 70 ]]; then
  echo "sweep_determinism.sh: expected 70 gate artifacts, found $gate_count" >&2
  exit 1
fi

first_digest="$(
  {
    scope_digest "$first_dir/canonical.sha256"
    scope_digest "$first_dir/gate.sha256"
    scope_digest "$first_dir/canonical.ownership"
    scope_digest "$first_dir/gate.ownership"
    scope_digest "$first_dir/README.md"
    scope_digest "$first_dir/audit.json"
  } | sha256sum | awk '{print $1}'
)"
second_digest="$(
  {
    scope_digest "$second_dir/canonical.sha256"
    scope_digest "$second_dir/gate.sha256"
    scope_digest "$second_dir/canonical.ownership"
    scope_digest "$second_dir/gate.ownership"
    scope_digest "$second_dir/README.md"
    scope_digest "$second_dir/audit.json"
  } | sha256sum | awk '{print $1}'
)"
if [[ "$first_digest" != "$second_digest" ]]; then
  echo "sweep_determinism.sh: combined deliverable digest drifted" >&2
  exit 1
fi

printf 'sweep_determinism.sh: OK, identical SHA-256 manifests for %s canonical + %s gate artifacts (combined %s)\n' \
  "$canonical_count" "$gate_count" "$first_digest"
