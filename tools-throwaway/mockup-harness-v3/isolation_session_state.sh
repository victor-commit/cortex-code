#!/usr/bin/env bash
# Resolve the mission-owned state root for immutable isolation bindings.
set -euo pipefail

RUNTIME_SETTINGS="${FACTORY_RUNTIME_SETTINGS_PATH:?isolation guard: FACTORY_RUNTIME_SETTINGS_PATH is required}"
RUNTIME_DIR="$(cd "$(dirname "$RUNTIME_SETTINGS")" && pwd)"
V3_ISOLATION_STATE_DIR="$RUNTIME_DIR/isolation-guard-v3"
V3_ISOLATION_STATE_MARKER="$RUNTIME_DIR/isolation-guard-v3.initialized"
V3_ISOLATION_STATE_ID="$V3_ISOLATION_STATE_DIR/state-id"

[[ "$V3_ISOLATION_STATE_DIR" = /* && "$V3_ISOLATION_STATE_MARKER" = /* ]] || {
    echo "isolation guard: runtime state paths must be absolute" >&2
    exit 2
}

isolation_require_state() {
    local marker
    local state_id

    [[ -f "$V3_ISOLATION_STATE_MARKER" && -d "$V3_ISOLATION_STATE_DIR" && -f "$V3_ISOLATION_STATE_ID" ]] || {
        echo "isolation guard: missing persistent mission state identity" >&2
        return 2
    }
    marker="$(<"$V3_ISOLATION_STATE_MARKER")"
    state_id="$(<"$V3_ISOLATION_STATE_ID")"
    [[ "$marker" =~ ^v1:[0-9a-f]{64}$ && "$state_id" == "$marker" ]] || {
        echo "isolation guard: persistent mission state identity mismatch" >&2
        return 2
    }
}

export V3_ISOLATION_STATE_DIR V3_ISOLATION_STATE_MARKER V3_ISOLATION_STATE_ID
