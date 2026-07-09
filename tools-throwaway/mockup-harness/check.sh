#!/usr/bin/env bash
# check.sh - conformance gate for the v2 mockup gallery.
#
# VERIFY-ONLY: scans the already-rendered artifacts under
# plans/assets/mockups-v2/ plus the compiled screen sources under
# src/screens/ - it NEVER re-renders anything (the harness binary is invoked
# only for `--list`, which writes no files). The parked v1 sources under
# src/screens_v1_reference/ are not compiled and are never scanned.
#
# Usage:
#   ./check.sh --all            check every registered screen
#   ./check.sh <screen>...      check one or more screens (see --list)
#   ./check.sh <category>...    check every screen in one or more categories
#   ./check.sh --list           print the registered screens and exit
#
# Per selected screen:
#   - <category>/<name>.txt AND <name>.png exist under plans/assets/mockups-v2/
#   - .txt holds only printable ASCII + the closed v2 glyph set
#     (no emoji codepoints, no other non-ASCII)
#   - no .txt line exceeds 120 columns (codepoints, same math as the harness)
#   - at most one bordered zone per .txt (a single U+256D top-left corner)
# Always:
#   - every src/screens/*.rs holds only ASCII + the closed glyph set
#   - no src/screens/*.rs file bypasses semantic tokens with raw Color::Rgb
#   - every non-ASCII glyph of the closed set is either in the DejaVu Sans
#     Mono cmap or synthesized by the PNG rasterizer (U+23BF -> draw_elbow;
#     the synthesized list is overridable via CHECK_FONT_SYNTH for testing)
#
# Exit: 0 clean, 1 violation(s) (file:line messages on stdout), 2 usage/error.

set -u
LC_ALL=C
export LC_ALL

HARNESS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HARNESS_DIR/../.." && pwd)"
OUT_REL="plans/assets/mockups-v2"
HARNESS_REL="tools-throwaway/mockup-harness"

usage() {
    cat >&2 <<'EOF'
usage: ./check.sh --all | <screen>... | <category>... | --list
Verify-only conformance gate over plans/assets/mockups-v2 (never re-renders).
EOF
}

all=0
list_only=0
wanted=()
for a in "$@"; do
    case "$a" in
        --all) all=1 ;;
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

# The registry is the single source of truth for screens; `--list` only
# prints it (no artifacts are written, so this is not a re-render).
list_out="$(cd "$HARNESS_DIR" && cargo run --quiet --features png -- --list)" || {
    echo "check.sh: could not obtain the screen registry (cargo run -- --list failed)" >&2
    exit 2
}

if [[ $list_only -eq 1 ]]; then
    printf '%s\n' "$list_out"
    exit 0
fi

cats=()
names=()
cur=""
while IFS= read -r line; do
    case "$line" in
        "  ["*"]") cur="${line#"  ["}"; cur="${cur%]}" ;;
        "    "*) cats+=("$cur"); names+=("${line#"    "}") ;;
    esac
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
    fi
done

src_count=0
for f in "$HARNESS_DIR"/src/screens/*.rs; do
    [[ -f "$f" ]] || continue
    scan_list+="src	$HARNESS_REL/src/screens/${f##*/}"$'\n'
    src_count=$((src_count + 1))
done

raw_color_lines="$(grep -nH 'Color::Rgb' "$HARNESS_DIR"/src/screens/*.rs || true)"
if [[ -n "$raw_color_lines" ]]; then
    printf '%s\n' "$raw_color_lines" |
        sed "s#${HARNESS_DIR}/#${HARNESS_REL}/#; s#$#: raw Color::Rgb is forbidden in screen sources#"
    violations=$((violations + $(printf '%s\n' "$raw_color_lines" | grep -c .)))
fi

py_out="$(
    cd "$REPO_ROOT" &&
    SCAN_LIST="$scan_list" \
    FONT_PATH="$HARNESS_REL/assets/DejaVuSansMono.ttf" \
    MAIN_RS="$HARNESS_REL/src/main.rs" \
    CHECK_FONT_SYNTH="${CHECK_FONT_SYNTH-23bf}" \
    python3 - <<'PY'
import os
import struct
import sys

ALLOWED = set("●○✓✗›→↑↓─│╭╮╰╯█░·⎿▸▾…")
# BMP dingbats/symbols (e.g. the decorative stars) are reported as forbidden
# glyphs, not emoji, so these ranges cover only unambiguous emoji machinery.
EMOJI_RANGES = (
    (0x1F000, 0x1FFFF),
    (0xFE00, 0xFE0F),
    (0x200D, 0x200D),
    (0x20E3, 0x20E3),
    (0xE0000, 0xE007F),
)

count = 0


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
    return f"forbidden glyph U+{o:04X} {ch!r} (outside the closed v2 glyph set)"


def scan(path, rendered):
    try:
        text = open(path, encoding="utf-8").read()
    except OSError as e:
        viol(f"{path}: unreadable ({e})")
        return
    except UnicodeDecodeError as e:
        viol(f"{path}: not valid UTF-8 ({e})")
        return
    corners = 0
    for ln, line in enumerate(text.splitlines(), 1):
        if rendered and len(line) > 120:
            viol(f"{path}:{ln}: line is {len(line)} columns (limit 120)")
        for col, ch in enumerate(line, 1):
            if 0x20 <= ord(ch) <= 0x7E or ch in ALLOWED:
                continue
            viol(f"{path}:{ln}:{col}: {classify(ch)}")
        if rendered:
            for _ in range(line.count("\u256d")):
                corners += 1
                if corners > 1:
                    viol(
                        f"{path}:{ln}: bordered zone #{corners}"
                        " ('\u256d' corner; at most one bordered zone per screen)"
                    )


def cmap_lookup(path):
    try:
        data = open(path, "rb").read()
    except OSError as e:
        viol(f"{path}: unreadable font ({e})")
        return None
    if len(data) < 12:
        viol(f"{path}: not a TrueType font")
        return None
    num_tables = struct.unpack_from(">H", data, 4)[0]
    cmap_off = None
    for i in range(num_tables):
        tag, _, off, _ = struct.unpack_from(">4sIII", data, 12 + 16 * i)
        if tag == b"cmap":
            cmap_off = off
            break
    if cmap_off is None:
        viol(f"{path}: font has no cmap table")
        return None
    n_sub = struct.unpack_from(">H", data, cmap_off + 2)[0]
    sub = None
    for i in range(n_sub):
        pid, eid, off = struct.unpack_from(">HHI", data, cmap_off + 4 + 8 * i)
        if (pid, eid) in ((3, 1), (0, 3), (0, 4), (3, 10)):
            if struct.unpack_from(">H", data, cmap_off + off)[0] == 4:
                sub = cmap_off + off
                break
    if sub is None:
        viol(f"{path}: font has no format-4 unicode cmap subtable")
        return None
    seg_x2 = struct.unpack_from(">H", data, sub + 6)[0]
    end0 = sub + 14
    start0 = end0 + seg_x2 + 2
    delta0 = start0 + seg_x2
    range0 = delta0 + seg_x2

    def has(cp):
        for i in range(seg_x2 // 2):
            end = struct.unpack_from(">H", data, end0 + 2 * i)[0]
            if cp > end:
                continue
            start = struct.unpack_from(">H", data, start0 + 2 * i)[0]
            if cp < start:
                return False
            delta = struct.unpack_from(">h", data, delta0 + 2 * i)[0]
            ro = struct.unpack_from(">H", data, range0 + 2 * i)[0]
            if ro == 0:
                return (cp + delta) & 0xFFFF != 0
            gid_off = range0 + 2 * i + ro + 2 * (cp - start)
            gid = struct.unpack_from(">H", data, gid_off)[0]
            return gid != 0 and (gid + delta) & 0xFFFF != 0
        return False

    return has


for entry in os.environ.get("SCAN_LIST", "").splitlines():
    if not entry.strip():
        continue
    kind, path = entry.split("\t", 1)
    scan(path, kind == "render")

font = os.environ["FONT_PATH"]
main_rs = os.environ["MAIN_RS"]
synth = {
    int(tok, 16)
    for tok in os.environ.get("CHECK_FONT_SYNTH", "23bf").split(",")
    if tok.strip()
}
has = cmap_lookup(font)
if has is not None:
    try:
        main_src = open(main_rs, encoding="utf-8").read().lower()
    except OSError as e:
        main_src = ""
        viol(f"{main_rs}: unreadable ({e})")
    for ch in sorted(ALLOWED):
        cp = ord(ch)
        if has(cp):
            continue
        if cp in synth:
            if "draw_elbow" in main_src and f"u{{{cp:x}}}" in main_src:
                continue
            viol(
                f"{font}: U+{cp:04X} {ch!r} missing from the font cmap;"
                f" declared synthesized but {main_rs} lacks the draw_elbow synthesis"
            )
        else:
            viol(
                f"{font}: U+{cp:04X} {ch!r} missing from the font cmap"
                " and not synthesized by the PNG rasterizer"
            )

sys.exit(0 if count == 0 else 1)
PY
)"
pyrc=$?
if [[ -n "$py_out" ]]; then
    printf '%s\n' "$py_out"
fi
py_count="$(printf '%s' "$py_out" | grep -c .)"
if [[ $pyrc -ne 0 && $py_count -eq 0 ]]; then
    echo "check.sh: content scanner failed (python exit $pyrc)" >&2
    exit 2
fi

total=$((violations + py_count))
if [[ $total -gt 0 ]]; then
    echo "check.sh: FAIL - $total violation(s) across ${#sel[@]} screen(s)"
    exit 1
fi
echo "check.sh: OK - ${#sel[@]} screen(s) clean (txt+png present, closed glyph set, no emoji, <=120 cols, single bordered zone), $src_count screen source(s) clean, font cmap covers the glyph set"
exit 0
