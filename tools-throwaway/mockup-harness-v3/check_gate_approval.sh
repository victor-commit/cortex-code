#!/usr/bin/env bash
# Validate the immutable six-flow human-gate approval evidence.
# Usage:
#   ./check_gate_approval.sh [--require-approved] <evidence/v3-human-gate>
#
# Exit codes:
#   0 - integrity satisfied for the requested mode
#   1 - missing/malformed/rejected/hash-stale/worker-authored evidence or bulk blocked
#   2 - usage error
set -euo pipefail

LC_ALL=C
export LC_ALL

HARNESS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HARNESS_DIR/../.." && pwd)"
export V3_REPO_ROOT="${V3_REPO_ROOT:-$REPO_ROOT}"
export V3_MISSION_DIR="${V3_MISSION_DIR:-/root/.factory/missions/7060a79b-ccfb-445c-9e82-ee4504b320dc}"

REQUIRE_APPROVED=0
ROOT=""

usage() {
  echo "usage: $0 [--require-approved] <evidence/v3-human-gate>" >&2
  exit 2
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --require-approved)
      REQUIRE_APPROVED=1
      shift
      ;;
    -h|--help)
      usage
      ;;
    *)
      if [[ -n "$ROOT" ]]; then
        usage
      fi
      ROOT="$1"
      shift
      ;;
  esac
done

if [[ -z "$ROOT" ]]; then
  usage
fi

if [[ ! -d "$ROOT" ]]; then
  echo "check_gate_approval.sh: human-gate root does not exist: $ROOT" >&2
  exit 1
fi

cd "$HARNESS_DIR"
export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-1}"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-16}"

ARGS=()
if [[ "$REQUIRE_APPROVED" -eq 1 ]]; then
  ARGS+=(--require-approved)
fi
ARGS+=("$ROOT")

# Preferred: compiled binary under target/debug after cargo build.
if cargo build --quiet --features png --bin check_gate_approval >/tmp/check_gate_approval_build.log 2>&1; then
  BIN="${CARGO_TARGET_DIR:-$HARNESS_DIR/target}/debug/check_gate_approval"
  if [[ -x "$BIN" ]]; then
    exec "$BIN" "${ARGS[@]}"
  fi
fi

echo "check_gate_approval.sh: could not build check_gate_approval binary" >&2
if [[ -f /tmp/check_gate_approval_build.log ]]; then
  cat /tmp/check_gate_approval_build.log >&2 || true
fi
exit 1
