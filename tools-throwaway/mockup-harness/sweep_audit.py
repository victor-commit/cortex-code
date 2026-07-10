#!/usr/bin/env python3
"""Semantic, style-aware sweep verifier for mockup gallery invariants.

The rendered TXT files are canonical for copy and geometry, but deliberately
drop Ratatui styles. `mockup-harness --audit-json` exposes a deterministic
buffer projection, and this verifier combines it with the registry and source
call sites. Assertions fail closed: a new key-line role, selection consumer,
or wall-clock context must first be added to the inventory below.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections import Counter
from decimal import Decimal, InvalidOperation
from pathlib import Path
from typing import Any


ACCENT = "rgb(217,130,95)"
BRIGHT = "rgb(230,233,239)"
TXT = "rgb(200,205,214)"
MUTED = "rgb(154,163,175)"
FAINT = "rgb(76,85,102)"
SURFACE = "rgb(38,43,51)"
OK = "rgb(127,176,105)"
BAD = "rgb(217,108,117)"
WARN = "rgb(217,179,108)"
INFO = "rgb(108,155,217)"
LIGHT_ACCENT = "rgb(192,107,68)"
LIGHT_BRIGHT = "rgb(20,23,28)"
LIGHT_SURFACE = "rgb(232,228,220)"

# Machine-readable inventory for every source-level selection consumer. The
# `filter_required` flag is derived from its rendered row count. Any future
# `*_select_list`, choice, hunk, findings, or theme-gallery consumer that is
# not declared here fails the source/registry cross-check.
SELECTION_CONSUMERS: dict[str, dict[str, Any]] = {
    "auth/connect-method": {"source": "select_list", "filter_required": False},
    "auth/connect-provider": {"source": "connection_select_list", "filter_required": False},
    "auth/login": {"source": "sectioned_select_list", "filter_required": False},
    "input/agent-picker": {"source": "sectioned_select_list", "filter_required": False},
    "input/attachment-picker": {"source": "annotated_select_list", "filter_required": False},
    "input/command-palette": {"source": "select_list", "filter_required": False},
    "input/command-palette-submenu": {"source": "sectioned_select_list", "filter_required": False},
    "input/export-picker": {"source": "annotated_select_list", "filter_required": False},
    "input/file-mention": {"source": "fuzzy_file_list", "filter_required": False},
    "input/fork-picker": {"source": "timeline_select_list", "filter_required": False},
    "input/model-picker": {"source": "sectioned_select_list", "filter_required": True},
    "input/model-picker-filtered": {"source": "sectioned_select_list", "filter_required": False},
    "input/permission-file-edit": {"source": "permission_choices", "filter_required": False},
    "input/permission-shell": {"source": "permission_choices", "filter_required": False},
    "input/queue-manager": {"source": "numbered_select_list", "filter_required": False},
    "input/session-picker": {"source": "session_select_list", "filter_required": False},
    "input/skills-picker": {"source": "toggle_select_list", "filter_required": True},
    "input/slash-menu": {"source": "sectioned_select_list", "filter_required": True},
    "input/subagent-picker": {"source": "annotated_select_list", "filter_required": False},
    "input/theme-picker": {"source": "theme_select_list", "filter_required": False},
    "input/variant-picker": {"source": "select_list", "filter_required": False},
    "orchestration/mode-switch": {"source": "select_list", "filter_required": False},
    "orchestration/mode-switch-refused": {"source": "select_list", "filter_required": False},
    "views/diff-viewer-staging": {"source": "diff_hunk_header", "filter_required": False},
    "views/review": {"source": "review_findings_list", "filter_required": False},
    "views/theme-gallery-dark": {"source": "theme_gallery", "filter_required": False},
    "views/theme-gallery-light": {
        "source": "theme_gallery",
        "filter_required": False,
        "palette": "light",
    },
    "session/onboarding": {"source": "step_current", "filter_required": False},
}

# Live activity ownership is deliberately closed. Each entry records the exact
# render-time `(source, verb, elapsed, interrupt)` event multiset expected for
# a registered screen, then says whether the canonical band or inline row owns
# its activity. The event inventory is deliberately independent of the
# rendered wording, so helper-generated plain-English rows stay fail-closed.
# Shared renderers may service more than one registered screen, but each
# rendered screen still has its own explicit event inventory.
LIVE_ACTIVITY_CONSUMERS: dict[str, dict[str, Any]] = {
    "session/streaming": {
        "source_paths": ("screens/streaming.rs",),
        "events": (("working_band", "Composing", "24s", "band"),),
        "owner": "band",
        "verb": "Composing",
        "activity": "Composing",
        "interrupt": "band",
    },
    "session/tool-running": {
        "source_paths": ("screens/tool_running.rs",),
        "events": (
            ("tool_running", "Search", None, "band"),
            ("working_band", "Scouring", "4s", "band"),
        ),
        "owner": "band",
        "verb": "Scouring",
        "activity": "Scouring",
        "interrupt": "band",
    },
    "session/tool-calls": {
        "source_paths": ("screens/tool_calls.rs",),
        "events": (("tool_running", "Search", "12s", "none"),),
        "owner": "inline",
        "verb": "Search",
        "activity": "Search(pattern: \"poll_cadence\")",
        "interrupt": "none",
    },
    "session/tool-task": {
        "source_paths": ("screens/tool_task.rs",),
        "events": (("tool_running", "Task", "48s", "inline"),),
        "owner": "inline",
        "verb": "Task",
        "activity": "Task(build: add rate-limit tests)",
        "interrupt": "inline",
    },
    "session/shell": {
        "source_paths": ("screens/shell.rs",),
        "events": (("shell_running", "cargo test --workspace", "4s", "inline"),),
        "owner": "inline",
        "verb": "cargo test --workspace",
        "activity": "cargo test --workspace",
        "interrupt": "inline",
    },
    "session/todo-list": {
        "source_paths": ("screens/todo_list.rs",),
        "events": (("working_band", "Writing tests", "22s", "band"),),
        "owner": "band",
        "verb": "Writing tests",
        "activity": "Writing tests",
        "interrupt": "band",
    },
    "states/rate-limit": {
        "source_paths": ("screens/rate_limit.rs",),
        "events": (("working_band", "Retry in", "42s", "band"),),
        "owner": "band",
        "verb": "Retry in",
        "activity": "Retry in",
        "interrupt": "band",
    },
    "input/subagent-picker": {
        "source_paths": ("screens/subagent_picker.rs",),
        "events": (("inline_live_state", "running", "12s", "none"),),
        "owner": "inline",
        "verb": "running",
        "activity": "running",
        "interrupt": "none",
    },
    "views/sidebar-subagents": {
        "source_paths": ("screens/sidebar_subagents.rs",),
        "events": (("inline_live_state", "running", "12s", "none"),),
        "owner": "inline",
        "verb": "running",
        "activity": "running",
        "interrupt": "none",
    },
    "orchestration/workflow-monitor": {
        "source_paths": ("screens/workflow_monitor.rs",),
        "events": (("working_band", "Orchestrating", "4m12s", "band"),),
        "owner": "band",
        "verb": "Orchestrating",
        "activity": "Orchestrating",
        "interrupt": "band",
    },
    "orchestration/workflow-monitor-failure": {
        "source_paths": ("screens/workflow_monitor.rs",),
        "events": (("working_band", "Orchestrating", "4m12s", "band"),),
        "owner": "band",
        "verb": "Orchestrating",
        "activity": "Orchestrating",
        "interrupt": "band",
    },
}

# Keyboard lines are the semantic placement inventory. A role is not inferred
# from a filename or arbitrary nearby word: every candidate key line must map
# to a concrete rendered line and allowed placement. New rows therefore fail
# closed with a file, line, and role diagnostic.
KEY_LINE_ROLES: dict[str, dict[int, str]] = {
    "auth/connect-oauth": {16: "card", 25: "card"},
    "orchestration/concurrency-metrics": {3: "view_header", 24: "activity_attached"},
    "orchestration/intent-scorer-fail": {3: "view_header"},
    "orchestration/intent-scorer-pass": {3: "view_header"},
    "orchestration/mode-switch-refused": {22: "failure_adjacent"},
    "orchestration/reviewer-approved": {24: "interactive_attached"},
    "orchestration/reviewer-output": {25: "failure_adjacent"},
    "orchestration/team-board": {3: "view_header"},
    "orchestration/verification-ladder": {3: "view_header"},
    "orchestration/workflow-monitor": {3: "view_header"},
    "orchestration/workflow-monitor-failure": {3: "view_header"},
    "orchestration/workflow-launch-blocked": {10: "failure_adjacent"},
    "session/blocked-command": {36: "failure_adjacent"},
    "session/chat-long-scroll": {6: "interactive_attached"},
    "session/compact": {36: "interactive_attached"},
    "session/onboarding": {35: "interactive_attached"},
    "session/reasoning": {18: "interactive_attached"},
    "session/reasoning-collapsed": {24: "interactive_attached"},
    "session/rich-response": {28: "interactive_attached"},
    "session/shell": {36: "activity_attached"},
    "session/tool-calls": {32: "failure_adjacent"},
    "session/tool-failed": {32: "failure_adjacent"},
    "session/tool-success": {29: "interactive_attached"},
    "session/tool-success-expanded": {19: "interactive_attached"},
    "session/tool-task": {33: "interactive_attached", 36: "activity_attached"},
    "session/tool-task-expanded": {30: "interactive_attached"},
    "session/turn-error": {36: "failure_adjacent"},
    "states/error-context": {36: "interactive_attached"},
    "states/error-network": {36: "failure_adjacent"},
    "states/interrupt": {35: "interactive_attached", 36: "interactive_attached", 37: "interactive_attached"},
    "states/rate-limit": {36: "activity_attached"},
    "states/share": {36: "interactive_attached"},
    "states/toasts": {3: "failure_adjacent"},
    "views/diff-viewer-staging": {1: "view_header", 6: "interactive_attached", 13: "interactive_attached", 22: "interactive_attached"},
    "views/mcp": {3: "view_header", 16: "failure_adjacent"},
    "views/review": {1: "view_header"},
    "views/sidebar-queue": {9: "interactive_attached"},
    "views/status": {14: "interactive_attached"},
    "views/timeline": {4: "view_header"},
    "views/theme-gallery-dark": {1: "view_header"},
    "views/theme-gallery-light": {1: "view_header"},
}

DISABLED_KEYS = {
    ("orchestration/reviewer-output", 25, "a"),
}

# Wall clocks are valid only in these explicitly typed event/timeline tables.
WALL_CLOCK_CONTEXTS = {
    "input/fork-picker",
    "orchestration/concurrency-metrics",
    "orchestration/team-board",
    "orchestration/tool-envelope",
    "orchestration/verification-ladder",
    "orchestration/workflow-monitor",
    "orchestration/workflow-monitor-failure",
    "views/mcp",
    "views/status",
    "views/timeline",
}

# A colored heading dot is a structural semantic role rather than a hidden
# status. Tool dots are instead justified by a directly attached elbow
# evidence line. This closed list prevents "any nearby noun" from becoming a
# bypass for a color-only dot.
STRUCTURAL_DOT_PREFIXES = (
    "In short",
    "Recommendation",
    "Updated plan",
    "Conversation compacted",
    "Exported session",
    "Proposed an edit",
)
STATUS_WORDS = {
    "active",
    "claimed",
    "closed",
    "connected",
    "current",
    "done",
    "error",
    "failed",
    "full",
    "live",
    "match",
    "miss",
    "ok",
    "off",
    "on",
    "open",
    "partial",
    "pending",
    "queued",
    "refused",
    "review",
    "running",
    "skipped",
    "staged",
    "submitted",
    "widen",
}

# The alternatives are longest-first. In particular, an h+m token must be
# consumed whole before the engine can consider its m component separately.
DURATION_TOKEN = re.compile(
    r"(?<![A-Za-z0-9.])(?:"
    r"\d+\s*h\s*\d+\s*m(?:\s*\d+\s*s)?"
    r"|\d+\s*m\s*\d+\s*s"
    r"|\d+(?:\.\d+)?\s*(?:seconds?|secs?|sec|s|minutes?|mins?|min|m|hours?|hrs?|hr|h)"
    r")(?![A-Za-z0-9.])"
)
VALID_DURATION = re.compile(r"^(?:[0-5]?\d(?:\.\d)?s|\d+m[0-5]\ds|\d+h[0-5]\dm)$")
AGE = re.compile(r"^\d+[mhd] ago$")
CLOCK = re.compile(r"(?<!\d)\d{1,2}:\d{2}(?!\d)")
KEY = r"(?:ctrl\+(?:[a-z0-9]|↑|↓)|shift\+enter|↑↓|ctrl\+↑|ctrl\+↓|1-3|enter|esc|tab|space|[a-z](?:/[a-z])?(?![a-z]))"
KEY_PAIR = re.compile(rf"(?<![A-Za-z0-9/+])({KEY})(?=\s+[a-z])", re.IGNORECASE)
SOURCE_CONSUMER = re.compile(
    r"chrome::(?:[a-z_]*select_list|fuzzy_file_list|permission_choices|review_findings_list|diff_hunk_header|theme_gallery)"
)
STEP_CURRENT_CONSUMER = re.compile(r"StepState::Current\b")
LIVE_SOURCE_PATTERNS = {
    "working_band": re.compile(r"\bchrome::working_band\s*\("),
    "tool_running": re.compile(r"\bToolStatus::Running\s*\("),
    "shell_running": re.compile(r"\bShellStatus::Running\s*\("),
    "inline_live_state": re.compile(r"\bchrome::inline_live_state\s*\("),
}


class Audit:
    def __init__(self) -> None:
        self.errors: list[str] = []
        self.counts: Counter[str] = Counter()

    def fail(self, screen: str, line: int, role: str, message: str) -> None:
        self.errors.append(f"{screen}:{line}: [{role}] {message}")


def style_at(row: dict[str, Any], column: int) -> dict[str, Any] | None:
    return next(
        (
            span
            for span in row["spans"]
            if span["start"] <= column < span["end"]
        ),
        None,
    )


def rows_with_background(screen: dict[str, Any], background: str) -> list[dict[str, Any]]:
    return [
        row
        for row in screen["rows"]
        if any(span["bg"] == background for span in row["spans"])
    ]


def key_matches(row: dict[str, Any]) -> list[tuple[str, int]]:
    pairs: list[tuple[str, int]] = []
    for match in KEY_PAIR.finditer(row["text"]):
        chord = match.group(1)
        normalized = chord.lower()
        span = style_at(row, match.start())
        strong = normalized.startswith(("ctrl+", "shift+")) or normalized in {
            "enter",
            "esc",
            "tab",
            "space",
            "↑↓",
            "ctrl+↑",
            "ctrl+↓",
            "1-3",
        }
        if strong or (span is not None and span["fg"] in {MUTED, ACCENT}):
            pairs.append((chord, match.start()))
    return pairs


def is_key_line(row: dict[str, Any]) -> bool:
    line = row["text"]
    pairs = key_matches(row)
    if not pairs:
        return False
    starts_with_key = line.lstrip(" │").lower().startswith(tuple(chord.lower() for chord, _ in pairs))
    return starts_with_key and (" · " in line or len(pairs) > 1 or any(
        token in line.lower()
        for token in ("retry", "select", "cancel", "apply", "copy", "filter", "open", "close", "move", "claim", "interrupt")
    ))


def line_role(screen_id: str, row: dict[str, Any]) -> str | None:
    if screen_id == "fixtures/cross-002.txt":
        return "view_header"
    if "esc to interrupt" in row["text"] and "…" in row["text"]:
        return "working_band"
    if row["text"].startswith("│") and row["text"].rstrip().endswith("│"):
        return "card"
    return KEY_LINE_ROLES.get(screen_id, {}).get(row["line"])


def validate_chord(audit: Audit, screen_id: str, row: dict[str, Any], chord: str) -> None:
    if chord != chord.lower():
        audit.fail(screen_id, row["line"], "key", f"uppercase chord '{chord}'")
    if chord in {"?", "/"}:
        audit.fail(screen_id, row["line"], "key", f"forbidden footer-style chord '{chord}'")


def validate_key_lines(
    audit: Audit,
    screens: list[dict[str, Any]],
    check_inventory: bool = True,
) -> None:
    known_ids = {screen["id"] for screen in screens}
    if check_inventory:
        unknown_inventory = set(KEY_LINE_ROLES) - known_ids
        for screen_id in sorted(unknown_inventory):
            audit.fail(screen_id, 0, "inventory", "key placement inventory references no registry screen")

    for screen in screens:
        screen_id = screen["id"]
        for row in screen["rows"]:
            text = row["text"]
            if not is_key_line(row):
                continue
            role = line_role(screen_id, row)
            if screen_id == "views/help":
                role = "reference"
            if role is None:
                audit.fail(screen_id, row["line"], "key", "unclassified keyboard affordance placement")
                continue
            audit.counts[f"key:{role}"] += 1
            pairs = key_matches(row)
            if role == "reference":
                continue
            if role != "working_band" and len(pairs) > 5:
                audit.fail(screen_id, row["line"], role, f"{len(pairs)} key chords, maximum is 5")
            for chord, position in pairs:
                validate_chord(audit, screen_id, row, chord)
                span = style_at(row, position)
                expected_key_color = (
                    FAINT if (screen_id, row["line"], chord.lower()) in DISABLED_KEYS else MUTED
                )
                if role != "working_band" and (
                    span is None or span["fg"] != expected_key_color
                ):
                    audit.fail(
                        screen_id,
                        row["line"],
                        role,
                        f"key '{chord}' is not {'FAINT disabled' if expected_key_color == FAINT else 'MUTED'}",
                    )
                if (screen_id, row["line"], chord.lower()) not in DISABLED_KEYS and role != "working_band":
                    label_end = text.find("·", position + len(chord))
                    if label_end < 0:
                        label_end = len(text)
                    for label_position in range(position + len(chord), label_end):
                        label_char = text[label_position]
                        if label_char.isspace():
                            continue
                        label_span = style_at(row, label_position)
                        if label_span is None or label_span["fg"] != FAINT:
                            audit.fail(
                                screen_id,
                                row["line"],
                                role,
                                f"label for key '{chord}' is not FAINT",
                            )
                            break
            if len(pairs) > 1:
                between = text
                for (left, left_at), (right, right_at) in zip(pairs, pairs[1:]):
                    if right_at > left_at and "·" not in between[left_at:right_at]:
                        audit.fail(
                            screen_id,
                            row["line"],
                            role,
                            f"keys '{left}' and '{right}' lack a · separator",
                        )

            if role == "working_band":
                continue
            for band in screen["rows"]:
                if "…" not in band["text"]:
                    continue
                if "esc to interrupt" not in band["text"]:
                    continue
                if "esc to interrupt" in text:
                    audit.fail(
                        screen_id,
                        row["line"],
                        role,
                        "interrupt affordance duplicates the working band",
                    )

        status_row = screen["rows"][-1]
        previous = screen["rows"][-2]
        if is_key_line(previous):
            audit.fail(
                screen_id,
                previous["line"],
                "key",
                "global/footer key line neighbors the statusline",
            )
        if "?" in previous["text"] and "help" in previous["text"].lower():
            audit.fail(screen_id, previous["line"], "key", "global '? help' footer vocabulary")
        if "ctx" in status_row["text"] or "cost" in status_row["text"]:
            audit.fail(screen_id, status_row["line"], "statusline", "statusline metadata drift")

    if not check_inventory:
        return
    screen_rows = {
        (screen["id"], row["line"]): row
        for screen in screens
        for row in screen["rows"]
    }
    for screen_id, line, chord in DISABLED_KEYS:
        row = screen_rows.get((screen_id, line))
        if row is None:
            audit.fail(screen_id, line, "key", f"disabled key '{chord}' is absent")
            continue
        position = row["text"].find(chord)
        span = style_at(row, position) if position >= 0 else None
        if span is None or span["fg"] != FAINT:
            audit.fail(screen_id, line, "key", f"disabled key '{chord}' is not FAINT")


def duration_candidates(line: str) -> list[str]:
    tokens: list[str] = []
    for match in DURATION_TOKEN.finditer(line):
        token = match.group(0).strip()
        if match.start() > 0 and line[match.start() - 1] == "/":
            continue
        if line[match.end() :].lstrip().startswith("ago"):
            continue
        tokens.append(token)
    return sorted(tokens, key=len, reverse=True)


def canonical_seconds(token: str) -> Decimal | None:
    if not VALID_DURATION.fullmatch(token):
        seconds_only = re.fullmatch(r"(\d+(?:\.\d+)?)s", token)
        if seconds_only:
            try:
                return Decimal(seconds_only.group(1))
            except InvalidOperation:
                return None
        spaced_minutes = re.fullmatch(r"(\d+)m\s+(\d+)s", token)
        if spaced_minutes:
            return Decimal(int(spaced_minutes.group(1)) * 60 + int(spaced_minutes.group(2)))
        return None
    if token.endswith("s") and "m" not in token and "h" not in token:
        try:
            return Decimal(token[:-1])
        except InvalidOperation:
            return None
    if "h" in token:
        hours, minutes = token[:-1].split("h")
        return Decimal(int(hours) * 3600 + int(minutes) * 60)
    minutes, seconds = token[:-1].split("m")
    return Decimal(int(minutes) * 60 + int(seconds))


def validate_durations(audit: Audit, screens: list[dict[str, Any]]) -> None:
    for screen in screens:
        seen: dict[Decimal, str] = {}
        for row in screen["rows"]:
            text = row["text"]
            for token in duration_candidates(text):
                audit.counts["duration"] += 1
                if not VALID_DURATION.fullmatch(token):
                    audit.fail(screen["id"], row["line"], "duration", f"noncanonical duration '{token}'")
                seconds = canonical_seconds(token)
                if seconds is not None and seconds in seen and seen[seconds] != token:
                    audit.fail(
                        screen["id"],
                        row["line"],
                        "duration",
                        f"mixed equivalent duration '{seen[seconds]}' and '{token}'",
                    )
                elif seconds is not None:
                    seen[seconds] = token
            for clock in CLOCK.findall(text):
                if screen["id"] not in WALL_CLOCK_CONTEXTS:
                    audit.fail(
                        screen["id"],
                        row["line"],
                        "duration",
                        f"clock-like token '{clock}' lacks an inventory-approved event context",
                    )


def dot_context(text: str, column: int) -> str:
    return text[column + 1 : column + 40].strip().lower()


def attached_evidence(rows: list[dict[str, Any]], row_index: int) -> bool:
    if row_index + 1 >= len(rows):
        return False
    next_text = rows[row_index + 1]["text"].lstrip()
    return next_text.startswith("⎿")


def validate_dots(audit: Audit, screens: list[dict[str, Any]]) -> None:
    semantic_colors = {ACCENT, BRIGHT, TXT, MUTED, FAINT, OK, BAD, WARN, INFO}
    for screen in screens:
        for row_index, row in enumerate(screen["rows"]):
            for column, glyph in enumerate(row["text"]):
                if glyph not in {"●", "○"}:
                    continue
                span = style_at(row, column)
                if span is None or span["fg"] not in semantic_colors:
                    continue
                context = dot_context(row["text"], column)
                role: str | None = None
                if glyph == "○":
                    role = "shape"
                elif any(context.startswith(prefix.lower()) for prefix in STRUCTURAL_DOT_PREFIXES):
                    role = "structural"
                elif "✓" in row["text"] or "✗" in row["text"] or attached_evidence(screen["rows"], row_index):
                    role = "evidence"
                elif any(re.match(rf"{re.escape(word)}\b", context) for word in STATUS_WORDS):
                    role = "label"
                elif (
                    re.match(r"(?:[a-f]\s+\w+|d[1-8]\s+\w+)", context)
                    or re.match(r"(?:read|search|bash|task|edit)\(", context)
                    or re.match(r"(?:composing|scouring|writing|orchestrating|retry)\b", context)
                    or "in progress" in row["text"].lower()
                    or re.search(
                        r"\b(?:valid|waiting|reached|interrupted|clean|modified|warning|warnings|error|diagnostics|needs auth|info|high|med|low|complete|blocked|widen|current|build)\b",
                        row["text"].lower(),
                    )
                ):
                    role = "progress_shape"
                if role is None:
                    audit.fail(
                        screen["id"],
                        row["line"],
                        "dot",
                        f"colored {glyph} has no approved glyph, label, shape, or evidence role",
                    )
                else:
                    audit.counts[f"dot:{role}"] += 1


def source_consumers(source_root: Path, registry: dict[str, dict[str, Any]]) -> dict[str, str]:
    consumers: dict[str, str] = {}
    for source in sorted(source_root.glob("*.rs")):
        source_text = source.read_text()
        matched = SOURCE_CONSUMER.findall(source_text)
        if STEP_CURRENT_CONSUMER.search(source_text):
            matched.append("step_current")
        if not matched:
            continue
        name = source.stem.replace("_", "-")
        candidates = [screen_id for screen_id in registry if screen_id.endswith(f"/{name}")]
        if len(candidates) != 1:
            raise ValueError(f"{source}: cannot map selection consumer to exactly one registry screen")
        sources = {match.removeprefix("chrome::") for match in matched}
        if len(sources) != 1:
            raise ValueError(f"{source}: has multiple selection consumer types {sorted(sources)}")
        consumers[candidates[0]] = sources.pop()
    return consumers


def expected_palette(entry: dict[str, Any]) -> tuple[str, str, str]:
    if entry.get("palette") == "light":
        return LIGHT_ACCENT, LIGHT_BRIGHT, LIGHT_SURFACE
    return ACCENT, BRIGHT, SURFACE


def validate_selection(
    audit: Audit,
    screens: list[dict[str, Any]],
    source_root: Path | None,
    inventory: dict[str, dict[str, Any]] | None = None,
    source_actual: dict[str, str] | None = None,
) -> None:
    inventory = SELECTION_CONSUMERS if inventory is None else inventory
    registry = {screen["id"]: screen for screen in screens}
    if source_root is not None or source_actual is not None:
        actual = (
            source_consumers(source_root, registry)
            if source_root is not None
            else source_actual or {}
        )
        expected = {screen_id: spec["source"] for screen_id, spec in inventory.items()}
        for screen_id in sorted(set(actual) - set(expected)):
            audit.fail(screen_id, 0, "selection", f"source consumer '{actual[screen_id]}' is absent from inventory")
        for screen_id in sorted(set(expected) - set(actual)):
            audit.fail(screen_id, 0, "selection", "inventory consumer has no source call site")
        for screen_id in sorted(set(actual) & set(expected)):
            if actual[screen_id] != expected[screen_id]:
                audit.fail(
                    screen_id,
                    0,
                    "selection",
                    f"inventory expects '{expected[screen_id]}', source uses '{actual[screen_id]}'",
                )

    for screen_id, spec in inventory.items():
        screen = registry.get(screen_id)
        if screen is None:
            audit.fail(screen_id, 0, "selection", "inventory screen is absent from registry")
            continue
        accent, bright, surface = expected_palette(spec)
        selected_rows = rows_with_background(screen, surface)
        if len(selected_rows) != 1:
            audit.fail(
                screen_id,
                selected_rows[0]["line"] if selected_rows else 0,
                "selection",
                f"expected exactly one selected SURFACE row, found {len(selected_rows)}",
            )
            continue
        row = selected_rows[0]
        marker = row["text"].find("›")
        marker_span = style_at(row, marker) if marker >= 0 else None
        if marker_span is None or marker_span["fg"] != accent or marker_span["bg"] != surface:
            audit.fail(screen_id, row["line"], "selection", "selected marker is not ACCENT on SURFACE")
        bright_after_marker = any(
            span["fg"] == bright and span["bg"] == surface and span["start"] > marker
            for span in row["spans"]
        )
        if not bright_after_marker:
            audit.fail(screen_id, row["line"], "selection", "selected row lacks BRIGHT text on SURFACE")
        surface_spans = [span for span in row["spans"] if span["bg"] == surface]
        continuous = bool(surface_spans)
        if continuous:
            last_end = surface_spans[0]["end"]
            for span in surface_spans[1:]:
                if span["start"] != last_end:
                    continuous = False
                    break
                last_end = span["end"]
            continuous = (
                continuous
                and surface_spans[0]["start"] <= marker
                and last_end - surface_spans[0]["start"] >= 8
            )
        if spec["source"] == "theme_gallery":
            leading_end = 0
            for span in surface_spans:
                if span["start"] != leading_end:
                    break
                leading_end = span["end"]
            trailing_start = 120
            for span in reversed(surface_spans):
                if span["end"] != trailing_start:
                    break
                trailing_start = span["start"]
            continuous = (
                bool(surface_spans)
                and surface_spans[0]["start"] <= marker
                and leading_end >= 15
                and trailing_start <= 80
            )
        if not continuous:
            audit.fail(screen_id, row["line"], "selection", "selection wash is not a continuous row geometry")
        if spec["filter_required"] and not any("/ " in candidate["text"] for candidate in screen["rows"]):
            audit.fail(screen_id, row["line"], "selection", "list over eight rows lacks a top filter line")
        audit.counts["selection"] += 1


def live_source_types(source_text: str) -> set[str]:
    return {
        source_name
        for source_name, pattern in LIVE_SOURCE_PATTERNS.items()
        if pattern.search(source_text)
    }


def live_source_consumers(
    source_root: Path,
    registry: dict[str, dict[str, Any]],
) -> dict[str, set[str]]:
    consumers: dict[str, set[str]] = {}
    for source in sorted(source_root.glob("*.rs")):
        matched = live_source_types(source.read_text())
        if not matched:
            continue
        name = source.stem.replace("_", "-")
        candidates = [screen_id for screen_id in registry if screen_id.endswith(f"/{name}")]
        if len(candidates) != 1:
            raise ValueError(f"{source}: cannot map live activity to exactly one registry screen")
        consumers[candidates[0]] = matched
    return consumers


def working_bands(screen: dict[str, Any]) -> list[dict[str, Any]]:
    return [
        row
        for row in screen["rows"]
        if "…" in row["text"] and "esc to interrupt" in row["text"]
    ]


def active_elapsed(row: dict[str, Any]) -> str | None:
    return next(
        (token for token in duration_candidates(row["text"]) if VALID_DURATION.fullmatch(token)),
        None,
    )


def live_events(screen: dict[str, Any]) -> list[dict[str, Any]]:
    events = screen.get("events", [])
    if not isinstance(events, list):
        return []
    return [event for event in events if isinstance(event, dict)]


def is_live_shaped_row(row: dict[str, Any]) -> bool:
    dot_index = row["text"].find("●")
    if dot_index < 0 or active_elapsed(row) is None:
        return False
    dot_span = style_at(row, dot_index)
    return dot_span is not None and dot_span["fg"] in {ACCENT, MUTED}


def event_signature(event: dict[str, Any]) -> tuple[str, str, str | None, str]:
    return (
        str(event.get("source")),
        str(event.get("verb")),
        event.get("elapsed") if isinstance(event.get("elapsed"), str) else None,
        str(event.get("interrupt")),
    )


def inventory_event_sources(spec: dict[str, Any]) -> set[str]:
    return {source for source, _, _, _ in spec["events"]}


def matching_event(
    row: dict[str, Any],
    event: dict[str, Any],
) -> bool:
    verb = event.get("verb")
    elapsed = event.get("elapsed")
    return (
        isinstance(verb, str)
        and isinstance(elapsed, str)
        and verb in row["text"]
        and elapsed in row["text"]
    )


def scan_unregistered_inline_helper(
    audit: Audit,
    source_root: Path,
    inventory: dict[str, dict[str, Any]],
) -> None:
    for source in sorted(source_root.glob("*.rs")):
        if not LIVE_SOURCE_PATTERNS["inline_live_state"].search(source.read_text()):
            continue
        screen_id = next(
            (
                candidate
                for candidate, spec in inventory.items()
                if source.relative_to(source_root.parent).as_posix() in spec["source_paths"]
            ),
            source.stem.replace("_", "-"),
        )
        if screen_id not in inventory:
            audit.fail(
                screen_id,
                0,
                "live_activity",
                f"inline_live_state source {source.name} is absent from inventory",
            )


def validate_live_activity(
    audit: Audit,
    screens: list[dict[str, Any]],
    source_root: Path | None,
    inventory: dict[str, dict[str, Any]] | None = None,
    source_actual: dict[str, set[str]] | None = None,
) -> None:
    inventory = LIVE_ACTIVITY_CONSUMERS if inventory is None else inventory
    registry = {screen["id"]: screen for screen in screens}
    actual = (
        live_source_consumers(source_root, registry)
        if source_root is not None
        else (source_actual or {})
    )
    if source_root is not None:
        scan_unregistered_inline_helper(audit, source_root, inventory)

    for screen_id in sorted(set(actual) - set(inventory)):
        screen = registry.get(screen_id)
        event_rows = (
            [
                row
                for event in live_events(screen or {})
                for row in (screen or {}).get("rows", [])
                if is_live_shaped_row(row) and matching_event(row, event)
            ]
            if screen is not None
            else []
        )
        audit.fail(
            screen_id,
            event_rows[0]["line"] if event_rows else 0,
            "live_activity",
            f"source live activity {sorted(actual[screen_id])} is absent from inventory",
        )
    for screen_id in sorted(set(registry) - set(inventory)):
        screen = registry[screen_id]
        events = live_events(screen)
        if events:
            event_rows = [
                row
                for event in events
                for row in screen["rows"]
                if is_live_shaped_row(row) and matching_event(row, event)
            ]
            audit.fail(
                screen_id,
                event_rows[0]["line"] if event_rows else 0,
                "live_activity",
                f"rendered live events {sorted(event_signature(event) for event in events)} are absent from inventory",
            )
    for screen_id, spec in inventory.items():
        screen = registry.get(screen_id)
        if screen is None:
            audit.fail(screen_id, 0, "live_activity", "inventory screen is absent from registry")
            continue

        expected_events = Counter(spec["events"])
        expected_sources = inventory_event_sources(spec)
        observed_sources = actual.get(screen_id)
        if observed_sources is not None and observed_sources != expected_sources:
            audit.fail(
                screen_id,
                0,
                "live_activity",
                f"inventory sources {sorted(expected_sources)} differ from source {sorted(observed_sources)}",
            )

        if source_root is not None:
            declared_sources: set[str] = set()
            for relative_path in spec["source_paths"]:
                source_path = source_root.parent / relative_path
                declared_sources.update(live_source_types(source_path.read_text()))
            missing_sources = expected_sources - declared_sources
            extra_sources = declared_sources - expected_sources
            if missing_sources:
                audit.fail(
                    screen_id,
                    0,
                    "live_activity",
                    f"inventory sources {sorted(missing_sources)} have no declared source call site",
                )
            if extra_sources:
                audit.fail(
                    screen_id,
                    0,
                    "live_activity",
                    f"declared source has unowned live activity {sorted(extra_sources)}",
                )

        events = live_events(screen)
        observed_events = Counter(event_signature(event) for event in events)
        if observed_events != expected_events:
            audit.fail(
                screen_id,
                0,
                "live_activity",
                "inventory event multiplicity "
                f"{sorted(expected_events.items(), key=lambda item: repr(item[0]))} "
                "differs from render events "
                f"{sorted(observed_events.items(), key=lambda item: repr(item[0]))}",
            )

        event_pairs = [
            (event, row)
            for event in events
            for row in screen["rows"]
            if is_live_shaped_row(row) and matching_event(row, event)
        ]
        for event in events:
            matches = [row for candidate, row in event_pairs if candidate is event]
            if event.get("elapsed") is not None and len(matches) != 1:
                audit.fail(
                    screen_id,
                    matches[0]["line"] if matches else 0,
                    "live_activity",
                    f"rendered event {event_signature(event)} maps to {len(matches)} live-shaped rows",
                )
        for row in screen["rows"]:
            if not is_live_shaped_row(row):
                continue
            matches = [event for event, candidate in event_pairs if candidate is row]
            if len(matches) != 1:
                audit.fail(
                    screen_id,
                    row["line"],
                    "live_activity",
                    f"live-shaped row maps to {len(matches)} render events",
                )

        bands = [
            row
            for event, row in event_pairs
            if event.get("source") == "working_band"
        ]
        owner = spec["owner"]
        verb = spec["verb"]
        activity = spec["activity"]
        activity_rows = [
            row
            for event, row in event_pairs
            if event.get("verb") == verb and activity in row["text"]
        ]
        interrupt_rows = [
            row for row in screen["rows"] if "esc to interrupt" in row["text"]
        ]
        if len(activity_rows) != 1:
            audit.fail(
                screen_id,
                activity_rows[0]["line"] if activity_rows else 0,
                "live_activity",
                f"authoritative activity '{activity}' occurs on {len(activity_rows)} rows",
            )
            continue
        activity_row = activity_rows[0]
        activity_event = next(
            event for event, row in event_pairs if row is activity_row and event.get("verb") == verb
        )

        expected_interrupt = spec["interrupt"]
        if activity_event.get("interrupt") != expected_interrupt:
            audit.fail(
                screen_id,
                activity_row["line"],
                "live_activity",
                f"inventory interrupt '{expected_interrupt}' differs from render event '{activity_event.get('interrupt')}'",
            )

        if owner == "band":
            if len(bands) != 1:
                audit.fail(
                    screen_id,
                    bands[1]["line"] if len(bands) > 1 else 0,
                    "live_activity",
                    f"band owner requires exactly one working band, found {len(bands)}",
                )
                continue
            band = bands[0]
            elapsed = active_elapsed(band)
            if elapsed is None:
                audit.fail(
                    screen_id,
                    band["line"],
                    "live_activity",
                    "working band lacks canonical elapsed time",
                )
                continue
            if activity_row is not band:
                audit.fail(
                    screen_id,
                    activity_row["line"],
                    "live_activity",
                    f"band does not own activity '{activity}'",
                )
            if verb not in band["text"]:
                audit.fail(
                    screen_id,
                    band["line"],
                    "live_activity",
                    f"band does not own verb '{verb}'",
                )
            if activity_event.get("source") != "working_band":
                audit.fail(
                    screen_id,
                    band["line"],
                    "live_activity",
                    "band owner is not recorded by working_band",
                )
            elapsed_rows = [row for row in screen["rows"] if elapsed in row["text"]]
            if len(elapsed_rows) != 1:
                audit.fail(
                    screen_id,
                    band["line"],
                    "live_activity",
                    f"band elapsed '{elapsed}' occurs on {len(elapsed_rows)} rows",
                )
            audit.counts["working_band"] += 1
        else:
            if bands:
                audit.fail(
                    screen_id,
                    bands[0]["line"],
                    "live_activity",
                    "inline owner must not render a working band",
                )
            elapsed = active_elapsed(activity_row)
            if elapsed is None:
                audit.fail(
                    screen_id,
                    activity_row["line"],
                    "live_activity",
                    f"inline activity '{activity}' lacks canonical elapsed time",
                )
            else:
                elapsed_rows = [row for row in screen["rows"] if elapsed in row["text"]]
                if len(elapsed_rows) != 1:
                    audit.fail(
                        screen_id,
                        activity_row["line"],
                        "live_activity",
                        f"inline elapsed '{elapsed}' occurs on {len(elapsed_rows)} rows",
                    )
            if verb not in activity_row["text"]:
                audit.fail(
                    screen_id,
                    activity_row["line"],
                    "live_activity",
                    f"inline elapsed is not owned by verb '{verb}'",
                )
            if activity_event.get("source") == "working_band":
                audit.fail(
                    screen_id,
                    activity_row["line"],
                    "live_activity",
                    "inline owner is recorded by working_band",
                )
            audit.counts["inline_live_activity"] += 1

        interrupt_owner = expected_interrupt
        if interrupt_owner == "band":
            if len(interrupt_rows) != 1 or (bands and interrupt_rows[0] not in bands):
                audit.fail(
                    screen_id,
                    interrupt_rows[0]["line"] if interrupt_rows else 0,
                    "live_activity",
                    "working band does not exclusively own the interrupt affordance",
                )
        elif interrupt_owner == "inline":
            if len(interrupt_rows) != 1:
                audit.fail(
                    screen_id,
                    interrupt_rows[1]["line"] if len(interrupt_rows) > 1 else 0,
                    "live_activity",
                    "inline activity does not exclusively own the interrupt affordance",
                )
        elif interrupt_rows:
            audit.fail(
                screen_id,
                interrupt_rows[0]["line"],
                "live_activity",
                "activity without interrupt ownership renders an interrupt affordance",
            )


def run(report: dict[str, Any], source_root: Path | None, fixture_name: str | None = None) -> Audit:
    audit = Audit()
    screens = report.get("screens")
    if not isinstance(screens, list):
        audit.fail("report", 0, "inventory", "audit JSON has no screens array")
        return audit
    if fixture_name == "cross-002":
        validate_key_lines(audit, screens, check_inventory=False)
    elif fixture_name == "cross-003-duplicate":
        validate_live_activity(
            audit,
            screens,
            None,
            {
                "fixtures/cross-003.txt": {
                    "events": (("working_band", "Composing", "24s", "band"),),
                    "owner": "band",
                    "verb": "Composing",
                    "activity": "Composing",
                    "interrupt": "band",
                }
            },
        )
    elif fixture_name == "cross-003-unlisted":
        validate_live_activity(
            audit,
            screens,
            None,
            {},
            {"fixtures/unlisted-live.txt": {"working_band"}},
        )
    elif fixture_name == "cross-003-source-unlisted":
        validate_live_activity(
            audit,
            screens,
            None,
            {},
            {
                "fixtures/source-live.txt": live_source_types(
                    "chrome::tool_line(ToolStatus::Running(elapsed, LiveActivityInterrupt::None), \"Task\", \"audit\"); "
                    "chrome::shell_line(ShellStatus::Running(elapsed), \"cargo test\");"
                )
            },
        )
    elif fixture_name == "cross-003-inline-helper-unlisted":
        validate_live_activity(
            audit,
            screens,
            None,
            {},
            {
                "fixtures/helper-live.txt": live_source_types(
                    'render_catalogue_state() { chrome::inline_live_state("cataloguing in background", elapsed); }'
                )
            },
        )
    elif fixture_name in {"cross-004-malformed", "cross-004-mixed"}:
        validate_durations(audit, screens)
    elif fixture_name in {"cross-004-valid", "cross-004-fractional"}:
        validate_durations(audit, screens)
    elif fixture_name in {"cross-007-bright", "cross-007-txt"}:
        validate_dots(audit, screens)
    elif fixture_name == "cross-009-current-unlisted":
        validate_selection(
            audit,
            screens,
            None,
            {},
            {"session/onboarding": "step_current"},
        )
    elif fixture_name == "cross-009-current-style":
        validate_selection(
            audit,
            screens,
            None,
            {"session/onboarding": SELECTION_CONSUMERS["session/onboarding"]},
        )
    else:
        validate_key_lines(audit, screens)
        validate_live_activity(audit, screens, source_root)
        validate_durations(audit, screens)
        validate_dots(audit, screens)
        validate_selection(audit, screens, source_root)
    return audit


def fixture(name: str) -> dict[str, Any]:
    def span(start: int, end: int, fg: str = MUTED, bg: str = "reset") -> dict[str, Any]:
        return {"start": start, "end": end, "fg": fg, "bg": bg, "bold": False}

    def row(line: int, text: str, spans: list[dict[str, Any]]) -> dict[str, Any]:
        return {"line": line, "text": text, "spans": spans}

    status = row(40, " ~/projects/cortex-code · master · Claude Opus 4.8", [span(0, 55)])
    if name == "cross-002":
        text = "CTRL+P open / CTRL+K close / CTRL+L next / CTRL+M previous / CTRL+N add / CTRL+O remove"
        return {"screens": [{"id": "fixtures/cross-002.txt", "rows": [row(39, text, [span(0, len(text), BRIGHT)]), status]}]}
    if name == "cross-003-duplicate":
        band = " ● Composing… 24s · esc to interrupt"
        duplicate = "   ⎿ Composing still running · 24s · esc to interrupt"
        return {
            "screens": [
                {
                    "id": "fixtures/cross-003.txt",
                    "events": [
                        {
                            "source": "working_band",
                            "verb": "Composing",
                            "elapsed": "24s",
                            "interrupt": "band",
                        },
                        {
                            "source": "working_band",
                            "verb": "Composing",
                            "elapsed": "24s",
                            "interrupt": "band",
                        }
                    ],
                    "rows": [
                        row(1, band, [span(0, len(band), ACCENT)]),
                        row(2, duplicate, [span(0, len(duplicate), MUTED)]),
                        status,
                    ],
                }
            ]
        }
    if name == "cross-003-unlisted":
        band = " ● Unlisted… 24s · esc to interrupt"
        return {
            "screens": [
                {
                    "id": "fixtures/unlisted-live.txt",
                    "rows": [row(1, band, [span(0, len(band), ACCENT)]), status],
                }
            ]
        }
    if name == "cross-003-source-unlisted":
        return {
            "screens": [
                {
                    "id": "fixtures/source-live.txt",
                    "rows": [status],
                }
            ]
        }
    if name == "cross-003-inline-helper-unlisted":
        return {
            "screens": [
                {
                    "id": "fixtures/helper-live.txt",
                    "events": [
                        {
                            "source": "inline_live_state",
                            "verb": "cataloguing in background",
                            "elapsed": "12s",
                            "interrupt": "none",
                        }
                    ],
                    "rows": [
                        row(
                            1,
                            " ● cataloguing in background 12s",
                            [span(0, 2, MUTED), span(2, 32)],
                        ),
                        status,
                    ],
                }
            ]
        }
    if name == "cross-004-valid":
        text = " ● Retrying… 2h10m · esc to interrupt"
        return {
            "screens": [
                {
                    "id": "fixtures/cross-004.txt",
                    "rows": [row(1, text, [span(0, len(text), ACCENT)]), status],
                }
            ]
        }
    if name == "cross-004-fractional":
        text = " ● Retrying… 0.4s · 0.9s · esc to interrupt"
        return {
            "screens": [
                {
                    "id": "fixtures/cross-004.txt",
                    "rows": [row(1, text, [span(0, len(text), ACCENT)]), status],
                }
            ]
        }
    if name == "cross-004-malformed":
        text = " ● Retrying… 2h 10m · esc to interrupt"
        return {
            "screens": [
                {
                    "id": "fixtures/cross-004.txt",
                    "rows": [row(1, text, [span(0, len(text), ACCENT)]), status],
                }
            ]
        }
    if name == "cross-004-mixed":
        text = " ● Retrying… 2h10m · prior form 130m00s · esc to interrupt"
        return {
            "screens": [
                {
                    "id": "fixtures/cross-004.txt",
                    "rows": [row(1, text, [span(0, len(text), ACCENT)]), status],
                }
            ]
        }
    if name == "cross-007-bright":
        text = " ● arbitrary neighbor"
        return {
            "screens": [
                {
                    "id": "fixtures/cross-007.txt",
                    "rows": [row(1, text, [span(0, 2, BRIGHT), span(2, len(text))]), status],
                }
            ]
        }
    if name == "cross-007-txt":
        text = " ● arbitrary neighbor"
        return {
            "screens": [
                {
                    "id": "fixtures/cross-007.txt",
                    "rows": [row(1, text, [span(0, 2, TXT), span(2, len(text))]), status],
                }
            ]
        }
    if name == "cross-009-current-unlisted":
        current = " › Choose a model"
        return {
            "screens": [
                {
                    "id": "session/onboarding",
                    "rows": [
                        row(1, current, [span(0, len(current), ACCENT, SURFACE)]),
                        status,
                    ],
                }
            ]
        }
    if name == "cross-009-current-style":
        current = " › Choose a model"
        return {
            "screens": [
                {
                    "id": "session/onboarding",
                    "rows": [
                        row(1, current, [span(0, len(current), MUTED, SURFACE)]),
                        status,
                    ],
                }
            ]
        }
    raise ValueError(f"unknown fixture {name}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--report", type=Path)
    parser.add_argument("--source-root", type=Path)
    parser.add_argument(
        "--fixture",
        choices=(
            "cross-002",
            "cross-003-duplicate",
            "cross-003-unlisted",
            "cross-003-source-unlisted",
            "cross-003-inline-helper-unlisted",
            "cross-004-valid",
            "cross-004-fractional",
            "cross-004-malformed",
            "cross-004-mixed",
            "cross-007-bright",
            "cross-007-txt",
            "cross-009-current-unlisted",
            "cross-009-current-style",
        ),
    )
    args = parser.parse_args()
    if bool(args.report) == bool(args.fixture):
        parser.error("provide exactly one of --report or --fixture")
    report = json.loads(args.report.read_text()) if args.report else fixture(args.fixture)
    audit = run(report, None if args.fixture else args.source_root, args.fixture)
    if audit.errors:
        print("\n".join(audit.errors))
        return 1
    print(
        "sweep_audit.py: OK "
        + ", ".join(f"{name}={count}" for name, count in sorted(audit.counts.items()))
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
