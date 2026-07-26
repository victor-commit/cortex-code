#!/usr/bin/env bash
# check.sh - verify-only conformance for the isolated v3 mockup gallery.
#
# Scans already-rendered artifacts under plans/assets/mockups-v3/ plus the
# compiled screen sources under src/screens. It NEVER re-renders. The harness
# binary is invoked only for --list, which writes no files.
#
# Usage:
#   ./check.sh --all            check every registered screen
#   ./check.sh --implemented    check every implemented screen (same as --all once complete)
#   ./check.sh <screen>...      check one or more screens (see --list)
#   ./check.sh <category>...    check every screen in one or more categories
#   ./check.sh --list           print the registered screens and exit
#
# Exit: 0 clean, 1 violation(s), 2 usage/error.

set -u
LC_ALL=C
export LC_ALL

HARNESS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HARNESS_DIR/../.." && pwd)"
OUT_REL="plans/assets/mockups-v3"
HARNESS_REL="tools-throwaway/mockup-harness-v3"

usage() {
  cat >&2 <<'EOF'
usage: ./check.sh --all | --implemented | <screen>... | <category>... | --list
Verify-only conformance over plans/assets/mockups-v3 (never re-renders).
EOF
}

all=0
list_only=0
wanted=()
for a in "$@"; do
  case "$a" in
    --all|--implemented) all=1 ;;
    --list) list_only=1 ;;
    -h|--help) usage; exit 0 ;;
    -*) echo "check.sh: unknown option '$a'" >&2; usage; exit 2 ;;
    *) wanted+=("$a") ;;
  esac
done
if [[ $all -eq 0 && $list_only -eq 0 && ${#wanted[@]} -eq 0 ]]; then
  usage
  exit 2
fi

list_out="$(cd "$HARNESS_DIR" && cargo run --quiet --features png --bin mockup-harness-v3 -- --list)" || {
  echo "check.sh: could not obtain the screen registry (cargo run -- --list failed)" >&2
  exit 2
}

if [[ $list_only -eq 1 ]]; then
  printf '%s\n' "$list_out"
  exit 0
fi

cats=()
names=()
while IFS= read -r line; do
  [[ -z "$line" ]] && continue
  # Public --list is TSV: category<TAB>id<TAB>status<TAB>flow<TAB>surface
  IFS=$'\t' read -r cat id _status _flow _surface <<<"$line"
  if [[ -z "${cat:-}" || -z "${id:-}" ]]; then
    echo "check.sh: unparsable --list row: $line" >&2
    exit 2
  fi
  cats+=("$cat")
  names+=("$id")
done <<<"$list_out"

if [[ ${#names[@]} -eq 0 ]]; then
  echo "check.sh: parsed zero screens out of --list; harness output format changed?" >&2
  exit 2
fi

sel=()
if [[ $all -eq 1 ]]; then
  for i in "${!names[@]}"; do sel+=("$i"); done
else
  declare -A seen=()
  for w in "${wanted[@]}"; do
    matched=0
    for i in "${!names[@]}"; do
      if [[ "$w" == "${names[$i]}" || "$w" == "${cats[$i]}" ]]; then
        matched=1
        if [[ -z "${seen[$i]+x}" ]]; then
          seen[$i]=1
          sel+=("$i")
        fi
      fi
    done
    if [[ $matched -eq 0 ]]; then
      echo "check.sh: unknown screen or category '$w'" >&2
      printf '%s\n' "$list_out" >&2
      exit 2
    fi
  done
fi

violations=0
scan_list=""
for i in "${sel[@]}"; do
  txt="$OUT_REL/${cats[$i]}/${names[$i]}.txt"
  png="$OUT_REL/${cats[$i]}/${names[$i]}.png"
  if [[ -f "$REPO_ROOT/$txt" ]]; then
    scan_list+="render	$txt"$'\n'
  else
    echo "$txt: missing rendered .txt for screen '${names[$i]}' (category '${cats[$i]}')"
    violations=$((violations + 1))
  fi
  if [[ ! -f "$REPO_ROOT/$png" ]]; then
    echo "$png: missing rendered .png for screen '${names[$i]}' (category '${cats[$i]}')"
    violations=$((violations + 1))
  else
    # Enforce faithful 1200x840 RGB PNG geometry without re-opening the renderer.
    if ! python3 - "$REPO_ROOT/$png" <<'PY'
import struct, sys
from pathlib import Path
path = Path(sys.argv[1])
data = path.read_bytes()
if data[:8] != b"\x89PNG\r\n\x1a\n" or data[12:16] != b"IHDR":
    print(f"{path}: not a PNG IHDR")
    raise SystemExit(1)
w, h, bit_depth, color_type = struct.unpack(">IIBB", data[16:26])
if (w, h) != (1200, 840):
    print(f"{path}: PNG is {w}x{h}, expected 1200x840")
    raise SystemExit(1)
if color_type not in (2, 6):
    print(f"{path}: PNG color type {color_type} is not RGB(A)")
    raise SystemExit(1)
PY
    then
      violations=$((violations + 1))
    fi
  fi
done

src_count=0
for f in "$HARNESS_DIR"/src/screens/*.rs; do
  [[ -f "$f" ]] || continue
  src_count=$((src_count + 1))
done

raw_color_lines="$(grep -nH 'Color::Rgb' "$HARNESS_DIR"/src/screens/*.rs 2>/dev/null || true)"
if [[ -n "$raw_color_lines" ]]; then
  printf '%s\n' "$raw_color_lines" |
    sed "s#${HARNESS_DIR}/#${HARNESS_REL}/#; s#\$#: raw Color::Rgb is forbidden in screen sources#"
  violations=$((violations + $(printf '%s\n' "$raw_color_lines" | grep -c .)))
fi

# Rendered-artifact scanner only: exact 120x40 UTF-8, closed glyph set, trailing newline.
# Screen sources are allowed long diagnostic comments; they are guarded by Color::Rgb
# scan above plus the design-system / semantic audit aggregate suites.
py_out="$(
  cd "$REPO_ROOT" &&
  SCAN_LIST="$scan_list" \
  python3 - <<'PY'
import os
import sys

from pathlib import Path

ALLOWED = set("●○✓✗›→↑↓─│╭╮╰╯█░·⎿▸▾…")
# Pure Cortex dump expands the closed set (prompt arrow, diamonds, braille logo,
# powerline branch). MC V2 premium adds Factory slot cells / mini ticks /
# selection bar blocks. Active only when pure ownership sidecar is present.
PURE_EXTRA = set("❯◆◇◈◉◎•⧉↗⇣❙⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏▮▯▰▱▌▐")
PURE_SIDECAR = Path("plans/assets/mockups-v3/.renderer-manifest-v1-pure-cortex")
PURE_MODE = PURE_SIDECAR.is_file()
EMOJI_RANGES = (
    (0x1F000, 0x1FFFF),
    (0xFE00, 0xFE0F),
    (0x200D, 0x200D),
    (0x20E3, 0x20E3),
    (0xE0000, 0xE007F),
)

count = 0

def pure_allowed(ch):
    o = ord(ch)
    return ch in ALLOWED or ch in PURE_EXTRA or 0x2800 <= o <= 0x28FF

def viol(msg):
    global count
    count += 1
    print(msg)

def classify(ch):
    o = ord(ch)
    if o < 0x20 or o == 0x7F:
        return f"control character U+{o:04X} (only printable ASCII is allowed)"
    for lo, hi in EMOJI_RANGES:
        if lo <= o <= hi:
            return f"emoji U+{o:04X} {ch!r} (emoji codepoints are forbidden)"
    mode = "pure-Cortex" if PURE_MODE else "closed v3"
    return f"forbidden glyph U+{o:04X} {ch!r} (outside the {mode} glyph set)"

def glyph_ok(ch):
    if 0x20 <= ord(ch) <= 0x7E:
        return True
    if PURE_MODE:
        return pure_allowed(ch)
    return ch in ALLOWED

def scan_render(path):
    try:
        text = open(path, encoding="utf-8").read()
    except OSError as e:
        viol(f"{path}: unreadable ({e})")
        return
    except UnicodeDecodeError as e:
        viol(f"{path}: not valid UTF-8 ({e})")
        return
    rows = text.splitlines()
    if len(rows) != 40:
        viol(f"{path}: row count is {len(rows)} (exact 40 required)")
    if not text.endswith("\n"):
        viol(f"{path}: missing trailing newline")
    if "\r" in text:
        viol(f"{path}: contains CR characters")
    for ln, line in enumerate(rows, 1):
        if len(line) != 120:
            viol(f"{path}:{ln}: line is {len(line)} columns (exact 120 required)")
        for col, ch in enumerate(line, 1):
            if glyph_ok(ch):
                continue
            viol(f"{path}:{ln}:{col}: {classify(ch)}")

for entry in os.environ.get("SCAN_LIST", "").splitlines():
    if not entry.strip():
        continue
    kind, path = entry.split("\t", 1)
    if kind == "render":
        scan_render(path)

sys.exit(0 if count == 0 else 1)
PY
)"
pyrc=$?
if [[ -n "$py_out" ]]; then
  printf '%s\n' "$py_out"
fi
py_count="$(printf '%s' "$py_out" | grep -c . || true)"
if [[ $pyrc -ne 0 && $py_count -eq 0 ]]; then
  echo "check.sh: content scanner failed (python exit $pyrc)" >&2
  exit 2
fi

total=$((violations + py_count))
if [[ $total -gt 0 ]]; then
  echo "check.sh: FAIL - $total violation(s) across ${#sel[@]} screen(s)"
  exit 1
fi
echo "check.sh: OK - ${#sel[@]} screen(s) clean (txt+png present, closed glyph set, exact 120x40, 1200x840 RGB), $src_count screen source(s) without raw Color::Rgb"
exit 0
