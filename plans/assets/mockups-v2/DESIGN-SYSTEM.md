# Cortex Code Design System v2

The human-readable spec for every screen in the v2 mockup gallery
(`plans/assets/mockups-v2/`). The binding source of truth is the mission
`architecture.md` (sections 3.1-3.4); this document restates it for screen authors and
adds the working detail you need to build a compliant screen from scratch. If the two
ever disagree, architecture.md wins: stop and escalate, never improvise.

The whole system exists as code in `tools-throwaway/mockup-harness/src/chrome.rs`.
Screens compose those primitives (`tool_line`/`elbow`, `diff_lines`, `select_list`,
`working_band`, `bar`, `statusbar`, `hints`, `banner`, `timefmt`, `cli_page`/`cli_view`/
`cli_action`, `card`) and never hand-roll their own versions. Every example below is a
literal excerpt of what those primitives render; spaces are significant.

Design stance in one line: **the transcript is the product; color is a status LED, not
paint.** Chrome is gray, one box per screen, green/red are verdicts, personality lives
in a single word.

---

## 1. Palette

Thirteen semantic tokens, defined once in `chrome.rs`. Screens import tokens and never
write a raw `Color::Rgb` (the conformance gate greps `src/screens/` for it). Theme-picker
swatch hues and the light palette also live in `chrome.rs` tables (`theme_previews()`,
`light_palette()`), so even the theme gallery never touches raw color.

| Token     | Hex       | Role                | Usage rules |
|-----------|-----------|---------------------|-------------|
| `CANVAS`  | `#0d0d0d` | background          | The background everywhere. Nothing else fills large areas. |
| `ACCENT`  | `#D9825F` | copper brand accent | Wordmark, spinner, active mode badge, focus/selection marker `›`. Budget: <=5% of visible cells, ONE accent element per region, and no more than ~3 accent-colored elements per screen. Never carries success/error meaning, never body text, never borders at rest. |
| `OK`      | `#7FB069` | success verdicts    | Verdicts only: tool success dot/check, diff additions, pass states, connected. Never decorative, never branding. |
| `BAD`     | `#D96C75` | failure verdicts    | Verdicts only: failures, diff deletions, blocking errors. NEVER for selection or emphasis. |
| `WARN`    | `#D9B36C` | rare attention      | Degraded/attention states only (rate-limit, near-full context). Routine notices stay `MUTED`. |
| `INFO`    | `#6C9BD9` | rare informational  | Links and informational badges (the plan-mode badge is the single INFO badge). |
| `BRIGHT`  | `#E6E9EF` | emphasis text       | Headers, focused row text, key data. Not body text. |
| `TXT`     | `#C8CDD6` | body text           | Default body text. |
| `MUTED`   | `#9AA3AF` | secondary text      | Hints, counters, metadata, timestamps. |
| `FAINT`   | `#4C5566` | chrome              | Borders, rules `─`, disabled items, ghost text. Never text longer than a few words. |
| `SURFACE` | `#262B33` | selection fill      | Fill bg ONLY for the selected row in lists and the current item in plan/step lists, nothing else. Statusline and cards keep the `CANVAS` bg (rule N-1). |
| `ADD_BG`  | `#26331F` | diff wash           | Added-line background wash in diffs. |
| `DEL_BG`  | `#33211F` | diff wash           | Removed-line background wash in diffs. |

Hard rules:

- **Chrome is always gray.** Borders, rules, labels, hints: `FAINT`/`MUTED`. Color means
  meaning.
- **Green and red are verdicts.** They appear only after something HAPPENED (a tool
  result, a diff line, a test outcome). Never decorative, never in headers or prompts.
- **State is always glyph + color, never color alone** (colorblind safety). A dot always
  has a word or an outcome glyph next to it.
- **One emphasis channel at a time**: bold OR color OR background, never stacked. Bold
  is for at most one phrase per screen.
- **Red is reserved** for error/deletion. Selection uses `SURFACE` bg + the `›` marker
  (marker glyph in `ACCENT`), never red, never a filled color bar.
- **Accent budget, operational check:** no more than ~3 accent-colored elements per
  screen, one per region. The <=5% of visible cells figure is judged approximately by
  vision review; the ~3-element count is the rule you design against.

---

## 2. Glyphs

### 2.1 Authorized set (closed and exhaustive)

Functional glyphs only. Nothing outside this table is allowed, in source or in rendered
output. Every glyph is single-width and DejaVu-safe: the harness aligns by
`chars().count()`, so a double-width character silently breaks the 120-column math.

| Glyph     | Name / role |
|-----------|-------------|
| `●`       | state dot (running/done/failed/live, always paired with a word or outcome) |
| `○`       | pending / empty dot |
| `✓`       | success |
| `✗`       | failure |
| `›`       | prompt / selection marker |
| `→`       | arrow / transition |
| `↑` `↓`   | scroll / nav hints |
| `─` `│` `╭` `╮` `╰` `╯` | rules and the single border (rounded corners only) |
| `█` `░`   | progress bars (filled / empty track) |
| `·`       | separator (also stands in for the forbidden em dash) |
| `⎿`       | result elbow: attaches tool output to its tool line |
| `▸` `▾`   | disclosure (collapsed / expanded) |
| `…`       | ellipsis (working verbs, truncation) |
| `+` `-`   | diff markers, ASCII only; diff counts are written `+9 -4` |
| `@`       | file mention |
| `!`       | shell prefix |

Plus plain ASCII text. Key chords render lowercase everywhere: `ctrl+p`, `shift+tab`,
`esc`, `enter`.

### 2.2 Forbidden (auto-fail, enforced by check.sh)

These may never appear in `src/screens/` or in any rendered `.txt` (this document names
them once, here, for reference only):

- **All emojis**, any codepoint in the emoji ranges.
- **All decorative glyphs**: `✦ ✧ ★ ☆ ❖ ◆ ✱` (stars, sparkles, ornamental diamonds).
- **Em dash `—`** (U+2014): use `·` or `-` instead.
- **Bullet `•`** (U+2022): mask secrets with `·` or `█`.
- **`⇥`** (U+21E5): write `tab` in words.
- **`Δ ◉ ◐ ■ ▶`**: no deltas, no alternate dots, no filled squares or arrows.
- **Minus sign `−`** (U+2212): diff counts are ASCII `+9 -4`.
- **Any glyph not listed in 2.1**, including square box-drawing corners and junctions.
- **Double-width characters** (CJK, emoji) anywhere, including mock data: they break the
  `chars().count()` column alignment.

---

## 3. The 13 core patterns

### Pattern 1: tool call = one line + elbow result

One agent action = one line. The `●` bullet is the status LED: `MUTED` while running
(with elapsed time appended when no working band is shown), `OK` when done, `BAD` when
failed. The result attaches below with the `⎿` elbow; multi-line output indents under
the elbow at the same content column. Never a box around a tool call.

```
 ● Read(crates/tui/src/app.rs)
   ⎿ 214 lines

 ● Search(pattern: "poll_cadence")
   ⎿ crates/tui/src/app.rs:412
     crates/tui/src/poll.rs:88

 ● Bash(cargo build) · 12s
```

Anatomy: dot at column 2, tool name in `BRIGHT`, `(args)` in `MUTED`; elbow `⎿` at
column 4 in `FAINT`, result content at column 6; continuation rows indent 5 spaces.

### Pattern 2: one bordered zone per screen

Exactly one bordered element exists on any screen: the composer (rounded, `FAINT`
border, `ACCENT` `›` prompt, `FAINT` ghost text), OR the action dock that replaces it
(rule F-5: permissions, pickers and decisions anchor where the composer was, drawn as a
`card()`), OR the single framed zone of a full-height view. Never a floating centered
modal (rule N-7). Everything else separates by whitespace and `─` rules.

```
╭────────────────────────────────────────────╮
│ › try "fix the failing poll test"          │
╰────────────────────────────────────────────╯
```

The dock variant (a `card()` with an embedded title, no filled surface, the card's keys
on ONE MUTED line INSIDE the card, active option marked by `›` + clear text):

```
╭ Permission required ───────────────────────────╮
│  Cortex wants to run: cargo test --workspace   │
│                                                │
│  › yes, run it                                 │
│    yes, and don't ask again this session       │
│    no                                          │
│                                                │
│  1-3 choose · enter confirm · esc deny         │
╰────────────────────────────────────────────────╯
```

### Pattern 3: statusline (bottom)

One dim gray row, the last row of the screen, NO filled background (rule N-1) and
MINIMAL content (rule N-8): `~/path · branch · <Model Display Name>` in `MUTED` with
`FAINT` separators, NOTHING else. No context percentage, no cost, no state dot: those
live in `/status`. Liveness is conveyed by the working band or on-screen content, never
by the statusline. The single optional right-side element is the mode badge, shown only
when a non-default mode is active: a colored label in the fixed right-side slot while
the cell background remains `CANVAS`; the plan-mode label is the single `INFO` badge and
any elevated/active mode label is `ACCENT`. The model always renders as its human display name ("Claude Opus 4.8"), never
a raw id like `claude-opus` (rule N-10/F-7) in the statusline, pickers, and `/status`
alike.

```
 ~/projects/cortex-code · master · Claude Opus 4.8
```

With the plan-mode badge active:

```
 ~/projects/cortex-code · master · Claude Opus 4.8                                                                plan
```

### Pattern 4: hint placement (no global hint row)

There is NO global footer hint row, on any screen (rule N-9): the old
`key label · key label` row under the statusline is gone, and so is the `? help` escape
hatch. Keyboard affordances render where they act:

- **Dock/picker cards** carry ONE `MUTED` key line INSIDE the card (max ~5 items),
  rendered by `hints(&[...])` on the card's last interior row.
- **Failure blocks** carry their recovery keys ADJACENT to the block (same line or the
  line below), rendered with `key_span`, e.g. `r retry` under a failed tool call.
- **Full-screen views** put navigation keys, where needed, on ONE `MUTED` line inside
  the view header or directly attached to the interactive element.
- **The composer placeholder** may carry at most one inline hint.

Keys are `MUTED`, labels `FAINT`, `·` separators, chords lowercase. The full key
reference lives in `/help`.

```
│  enter select · esc cancel · type to filter    │   (inside a picker card)

   ⎿ ✗ 2 of 38 tests failed · exit 101
     r retry                                         (adjacent to the failure)
```

### Pattern 5: one canonical status band (elapsed once)

A working/streaming state announces itself ONCE: the working band above the composer.
Never repeat the same status inline and in the band. A screen with an in-flight
tool/shell/task shows at most one band; showcase screens may omit it. Elapsed time
renders exactly once per activity: in the band when one is shown (the tool line then
omits it), on the tool line when no band is shown.

```
 ● Bash(cargo test --workspace)
   ⎿ 3 of 5 suites complete

 ● Testing… 42s · esc to interrupt
```

The band dot is the single `ACCENT` spinner glyph; the verb is `TXT`; counters and the
interrupt hint stay dim (`MUTED`/`FAINT`).

### Pattern 6: diffs, tinted lines

Tinted background washes with line numbers and ASCII sign markers. Added rows wash
`ADD_BG` with an `OK` `+`; removed rows wash `DEL_BG` with a `BAD` `-`; context rows
stay `MUTED` on the canvas. Code text stays `TXT` on the wash, only the sign is colored
(one emphasis channel); word-level emphasis is optional via bold. Gutter numbers
(`FAINT`) advance automatically. The wash band spans a fixed 108 columns from column 3;
it is invisible in the `.txt`, so verify washes on the PNG. Diff counts are `+9 -4`,
never with U+2212.

```
    14   14   fn poll(&mut self) {
    15      -     let delay = 250;
         15 +     let delay = self.cadence_ms();
    16   16       schedule(delay)
```

### Pattern 7: selection in lists

The selected row gets the `SURFACE` background wash, an `ACCENT` `›` marker, and
`BRIGHT` name text. Unselected rows: `TXT` name, `MUTED` detail, `FAINT` meta. Columns
are fixed and aligned (name / detail / meta, meta right-aligned). A `/` filter line is
REQUIRED when the list is longer than 8 rows, optional below that, never forbidden.
Selection is never red and never a filled color bar.

```
  / type to filter
 › Claude Opus 4.8    thinking · 200k ctx                             current
   Claude Sonnet 4.6  fast · 200k ctx
   GPT-5              balanced · 400k ctx
```

### Pattern 8: failure escalation + borderless left-bar toasts

Failed/blocked items sort first or get a `BAD` left bar `│`. Every failure state carries
a keyboard recovery affordance (`r retry`, `y apply fix`, `o open`). Toasts are
borderless compact blocks with a `│` left bar, never bordered chips; the bar is
severity-colored on the BAD toast only (other toasts keep a `FAINT` bar).

```
 │ Rate limited
 │ retrying in 42s · esc to cancel

 │ Merge conflict in crates/tui/src/app.rs
 │ y apply fix · r retry · esc dismiss
```

### Pattern 9: time, age and countdown grammar

One duration formatter (`timefmt`) is used everywhere: `0.4s`, `12s`, `1m04s`, `2h10m`.
Bare seconds appear only below 60; minutes zero-pad seconds; hours drop seconds.
Relative ages use the distinct grammar `Nm ago / Nh ago / Nd ago / yesterday`.
Wall-clock `HH:MM` appears only for schedule timestamps. Countdowns render as live
off-round values (`14m32s`, never `15m00s`) with a `⎿ refreshing` hint; a round static
placeholder is a design bug.

```
duration    0.4s   12s   1m04s   2h10m
age         5m ago   2h ago   3d ago   yesterday
schedule    14:30
countdown   code expires in 14m32s
            ⎿ refreshing
```

### Pattern 10: spinner personality

A single `ACCENT` glyph + one working verb + dim counters. Personality is confined to
the verb; geometry and color never change. Never a progress bar for unbounded work.

```
 ● Distilling… 12s · 3.1k tokens · esc to interrupt
```

### Pattern 11: plain-English copy

Errors and statuses are plain English, human phrasing, reduced to the essential
(rules N-4/N-5, F-2/F-3). No snake_case tokens, no error codes as headlines, no
shouting. Key chords lowercase.

```
say                            not
Merge conflict                 merge_conflict
Rate limited · retry in 42s    ERR_RATE_LIMIT (429)
press esc to cancel            Press ESC To Cancel
```

### Pattern 12: bottom-anchored chat vs full-height views

The chat REPL anchors to the bottom: the newest turn hugs the composer and spare space
sits at the top. An empty top canvas is intentional, never a bug to fill. Full-screen
views (help, status, sidebar tabs, theme gallery, orchestration boards) may use the full
height, with a `title ─` header rule at the top and no composer: you read them, you
don't type.

Chat frame (`cli_page`), bottom-anchored:

```
 (empty top canvas: intentional)

 › fix the failing poll test  · 2m ago
   Cortex
   Done. The cadence test passes now.

╭────────────────────────────────────────────╮
│ › reply to Cortex, or ask a follow-up…     │
╰────────────────────────────────────────────╯
 ~/projects/cortex-code · master · Claude Opus 4.8
```

Full-height view (`cli_view`):

```
 status ──────────────────────────────────────────────────────────

 agent      Claude Opus 4.8
 context    ████████░░░░░░░░░░░░  42% of 200k
 mcp        ● live · 3 servers

 ~/projects/cortex-code · master · Claude Opus 4.8
```

### Pattern 13: the ASCII "Code" banner

`home` and `onboarding` open with a droid-style ASCII-art wordmark reading "Code" in
`ACCENT` copper (rule F-6): 6 rows tall, built ONLY from authorized glyphs/ASCII (block
letters in `█`), rendered by the single shared chrome primitive `banner()` at the
column-4 text edge, with product version and greeting nearby. It replaces the old
one-line wordmark on those two screens and is the only large brand mark in the gallery:
NO other screen carries a banner. On home and onboarding the banner counts as ONE accent
element, and the accent budget is amended there to the banner + at most 2 other accent
elements.

```
    ██████              ██
   ██                   ██
   ██        ████    █████   ████
   ██       ██  ██  ██  ██  ██████
   ██       ██  ██  ██  ██  ██
    ██████   ████    █████   █████

   Welcome back.  v0.3.0
```

---

## 4. Density and spacing

- **Canvas is fixed at 120x40 cells.** Every rendered line is <=120 columns, measured by
  `chars().count()`. Where a border column appears (the composer or dock spans the full
  width), it lands at column 120 on every row it occupies; check the rendered `.txt`.
- **Content measure is ~100 columns.** Hand-wrap prose at roughly 96-104 columns even
  though the canvas is 120: long lines are hard to scan. Full-bleed width is reserved
  for the single border, header rules, the statusline and diff wash bands (108 columns).
- **The indent grid** (all columns 1-based):
  - column 1: one-space left margin on every chrome row (`›` turns, `●` tool lines,
    toasts, the statusline, in-card key lines);
  - column 2: markers (`●` tool dot, `›` user marker, `│` toast bar, band spinner);
  - column 3: two-space indent rows (`label: value` info rows, the `/` filter line, the
    diff block's left edge);
  - column 4: primary content (tool name after the dot, user command after the marker,
    agent heading and agent prose under their 3-space pad);
  - column 4: the `⎿` elbow, with its content at column 6; elbow continuation rows
    indent 5 spaces to the same content column.
- **Whitespace separates regions**: one blank line between turns, a ` title ` + `─` rule
  (`rule_head`) between sections. Never boxes, never decorative divider rows.
- **No automatic wrapping exists.** Prose is hand-broken by the author; tables are
  hand-summed so each row is exactly the intended width. Transcript overflow silently
  truncates from the top: keep `cli_page` transcript content within its 36-row body
  (40 rows minus composer 3, statusline 1; there is no hint row, rule N-9).
- **Trailing spaces are trimmed** in the `.txt`; never rely on trailing pads for layout
  proofs, and verify background washes (`SURFACE`, `ADD_BG`, `DEL_BG`) on the PNG.
- **One emphasis channel at a time**; bold for at most one phrase per screen.
- Keep screens sparse: cut every element that does not earn its place (rules N-3/N-4,
  F-2/F-4). If a region needs more than one accent element, redesign the region.

---

## 5. File naming and gallery structure

- Output root: `plans/assets/mockups-v2/`. Each screen writes exactly two artifacts:
  `<category>/<screen>.txt` and `<category>/<screen>.png`.
- **Screen names are kebab-case** (`chat-turn`, `theme-gallery-dark`); state variants
  are standalone screens with a kebab-case suffix (`workflow-launch-preflight-red`).
  Module files are snake_case: `chat-turn` renders from `src/screens/chat_turn.rs`.
- **Six categories** = six output subdirectories, 86 screens at mission end:
  `session/` (21), `input/` (20), `views/` (16), `auth/` (7), `states/` (8),
  `orchestration/` (14).
- The gallery root carries `DESIGN-SYSTEM.md` (this document) and, at mission end, a
  `README.md` index. The v1 gallery `plans/assets/mockups/` is frozen: never re-render,
  edit or delete it.
- **The `.txt` is the source of truth**: 40 lines, trailing-space trimmed, byte-identical
  on re-render (no randomness, no clock reads, fixed English mock data). The `.png`
  (1200x840, DejaVu Sans Mono at 10x21 px cells) is the faithful raster of the same
  frame and the only artifact that shows background washes.
- Adding a screen takes three touches in the harness: a new file in `src/screens/`, one
  `pub mod` line in `src/screens/mod.rs`, one registry tuple in `src/main.rs`
  (categories keep registry order; output folders are created on demand).
- Render and verify from `tools-throwaway/mockup-harness/`:

```
cargo run --quiet --features png -- --list        list registered screens
cargo run --quiet --features png -- <name>        render one screen
cargo run --quiet --features png -- <category>    render a category
cargo run --quiet --features png -- --all         render everything
./check.sh <name|category|--all>                  conformance gate
```

Every screen change ends with: rebuild, render, `./check.sh <name>`, read the `.txt`,
vision-review the `.png`.
