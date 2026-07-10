#!/usr/bin/env bash
# Cross-gallery, verify-only checks for the rendered mockup artifacts.

set -euo pipefail
LC_ALL=C
export LC_ALL

HARNESS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HARNESS_DIR/../.." && pwd)"
OUT_DIR="$REPO_ROOT/plans/assets/mockups-v2"
README="$OUT_DIR/README.md"
EXPECTED_CATEGORIES=(session input views auth states orchestration)
EXPECTED_STATUS=" ~/projects/cortex-code · master · Claude Opus 4.8"

failures=0
fail() {
    printf 'sweep_checks.sh: %s\n' "$*"
    failures=$((failures + 1))
}

list_out="$(cd "$HARNESS_DIR" && cargo run --quiet --features png -- --list)"
categories=()
screens=()
current=""
while IFS= read -r line; do
    case "$line" in
        "  ["*"]")
            current="${line#"  ["}"
            current="${current%]}"
            ;;
        "    "*)
            categories+=("$current")
            screens+=("${line#"    "}")
            ;;
    esac
done <<<"$list_out"

if [[ ${#screens[@]} -ne 86 ]]; then
    fail "registry count is ${#screens[@]}, expected 86"
fi

actual_categories="$(find "$OUT_DIR" -mindepth 1 -maxdepth 1 -type d -printf '%f\n' | sort)"
expected_categories="$(printf '%s\n' "${EXPECTED_CATEGORIES[@]}" | sort)"
if ! diff -u <(printf '%s\n' "$expected_categories") <(printf '%s\n' "$actual_categories"); then
    fail "output categories differ from the required six"
fi

expected_artifacts="$(mktemp)"
actual_artifacts="$(mktemp)"
trap 'rm -f "$expected_artifacts" "$actual_artifacts"' EXIT
for i in "${!screens[@]}"; do
    printf '%s/%s.txt\n%s/%s.png\n' \
        "${categories[$i]}" "${screens[$i]}" "${categories[$i]}" "${screens[$i]}"
done | sort >"$expected_artifacts"
find "$OUT_DIR" -mindepth 2 -maxdepth 2 -type f \( -name '*.txt' -o -name '*.png' \) \
    -printf '%P\n' | sort >"$actual_artifacts"
if ! diff -u "$expected_artifacts" "$actual_artifacts"; then
    fail "rendered artifact set differs from the registry"
fi

while IFS=$'\t' read -r category screen; do
    file="$OUT_DIR/$category/$screen.txt"
    status="$(tail -n 1 "$file")"
    if [[ "$status" != "$EXPECTED_STATUS" ]] &&
        [[ ! "$status" =~ ^${EXPECTED_STATUS//\//\\/}[[:space:]]+(plan|elevated|active)$ ]]; then
        fail "$category/$screen has a non-canonical statusline: $status"
    fi
    if [[ "$status" =~ (ctx|cost|idle|busy|live|claude-opus|opus-4-8) ]]; then
        fail "$category/$screen statusline has forbidden metadata: $status"
    fi

    previous="$(tail -n 2 "$file" | head -n 1)"
    if [[ "$previous" =~ (ctrl\+|shift\+|(^|[[:space:]])(enter|esc|tab|[a-z][[:space:]])) ]] &&
        [[ "$previous" != *"╰"* ]]; then
        fail "$category/$screen has a footer-like key row beside the statusline"
    fi

    interrupt_rows="$(grep -c 'esc to interrupt' "$file" || true)"
    if [[ "$interrupt_rows" -gt 1 ]]; then
        fail "$category/$screen repeats the interrupt affordance"
    fi
done < <(for i in "${!screens[@]}"; do printf '%s\t%s\n' "${categories[$i]}" "${screens[$i]}"; done)

if grep -RInE '\? help|/[[:space:]]+commands[[:space:]]*·' \
    "$OUT_DIR"/*/*.txt; then
    fail "global footer hint vocabulary remains in a rendered screen"
fi

if grep -RIn 'Color::Rgb' "$HARNESS_DIR/src/screens"; then
    fail "screen source bypasses semantic color aliases"
fi

if grep -RIn 'SELECT_MARKER' "$HARNESS_DIR/src/screens"; then
    fail "screen source hand-rolls a selection marker"
fi

for source in "$HARNESS_DIR"/src/screens/*picker*.rs; do
    [[ -f "$source" ]] || continue
    if ! grep -Eq 'chrome::[a-z_]*select_list' "$source"; then
        fail "picker source lacks a shared selection-list primitive: ${source##*/}"
    fi
done
for source in "$HARNESS_DIR"/src/screens/permission_shell.rs \
    "$HARNESS_DIR"/src/screens/permission_file_edit.rs; do
    if ! grep -q 'chrome::permission_choices' "$source"; then
        fail "permission source lacks shared choices: ${source##*/}"
    fi
done

sidebar_tabs="$(for file in "$OUT_DIR"/views/sidebar-*.txt; do
    grep -m 1 'Ctx · Files · LSP · MCP · Todo · Subs · Queue' "$file" || true
done | sort -u)"
if [[ "$(printf '%s\n' "$sidebar_tabs" | sed '/^$/d' | wc -l)" -ne 1 ]]; then
    fail "sidebar tab strips are not byte-identical"
fi

if ! README_PATH="$README" OUT_DIR="$OUT_DIR" python3 - <<'PY'
import os
import re
import sys
from collections import Counter
from pathlib import Path

allowed = set("●○✓✗›→↑↓─│╭╮╰╯█░·⎿▸▾…")
out_dir = Path(os.environ["OUT_DIR"])
readme = Path(os.environ["README_PATH"])
errors = []

duration = re.compile(r"(?<![A-Za-z0-9.])(\d+(?:\.\d+)?s|\d+m\d{2}s|\d+h\d{2}m)(?![A-Za-z0-9.])")
valid_duration = re.compile(r"^(?:[0-5]?\d(?:\.\d)?s|\d+m[0-5]\ds|\d+h[0-5]\dm)$")
spaced_duration = re.compile(r"(?<![A-Za-z0-9])\d+\s+(?:s|sec|secs|second|seconds|min|mins|minute|minutes|h|hr|hrs|hour|hours)\b", re.I)
paired_dot = re.compile(r"(?:[●○]\s*\S+|\S+\s*[●○])")

inventory = Counter()
for path in sorted(out_dir.glob("*/*.txt")):
    lines = path.read_text().splitlines()
    for line_no, line in enumerate(lines, 1):
        for char in line:
            if ord(char) > 127:
                inventory[char] += 1
                if char not in allowed:
                    errors.append(f"{path.relative_to(out_dir)}:{line_no}: unauthorized U+{ord(char):04X} {char!r}")
        for match in duration.finditer(line):
            token = match.group(1)
            if line[match.end():].startswith(" ago"):
                continue
            if not valid_duration.fullmatch(token):
                errors.append(f"{path.relative_to(out_dir)}:{line_no}: invalid duration {token}")
        if spaced_duration.search(line):
            errors.append(f"{path.relative_to(out_dir)}:{line_no}: spaced duration syntax")

        if ("●" in line or "○" in line) and not paired_dot.search(line):
            errors.append(f"{path.relative_to(out_dir)}:{line_no}: unpaired state glyph")

print("codepoint inventory:", " ".join(f"U+{ord(ch):04X}={count}" for ch, count in sorted(inventory.items())))
if errors:
    print("\n".join(errors))
    sys.exit(1)
PY
then
    fail "glyph, duration, or status-pairing scan failed"
fi

# Text alone cannot prove Ratatui foreground/background ownership or whether a
# key line belongs to the dock rather than a footer. The harness emits a
# deterministic buffer projection, and sweep_audit.py cross-checks that
# projection against the registry and source-level selection-consumer inventory.
audit_json="$(mktemp)"
trap 'rm -f "$expected_artifacts" "$actual_artifacts" "$audit_json"' EXIT
if ! (
    cd "$HARNESS_DIR"
    cargo run --quiet --features png -- --audit-json >"$audit_json"
    python3 ./sweep_audit.py --report "$audit_json" --source-root ./src/screens
); then
    fail "semantic key/status/duration/dot/selection audit failed"
fi

if ! bash "$HARNESS_DIR/tests/sweep_negative.sh"; then
    fail "semantic negative fixtures failed"
fi

if [[ ! -f "$README" ]]; then
    fail "gallery README is missing"
else
    readme_entries="$(grep -Eo '^\| `[a-z0-9-]+` \|' "$README" | sed -E 's/^\| `([^`]+)` \|/\1/' | sort)"
    registered_entries="$(printf '%s\n' "${screens[@]}" | sort)"
    if ! diff -u <(printf '%s\n' "$registered_entries") <(printf '%s\n' "$readme_entries"); then
        fail "README screen entries differ from the registry"
    fi
    for category in "${EXPECTED_CATEGORIES[@]}"; do
        if ! grep -q "^## ${category^} " "$README"; then
            fail "README is missing the $category section"
        fi
    done
fi

if [[ $failures -gt 0 ]]; then
    printf 'sweep_checks.sh: FAIL, %d issue(s)\n' "$failures"
    exit 1
fi

printf 'sweep_checks.sh: OK, %d registered screens, six categories, no stale artifacts\n' "${#screens[@]}"
