# Mockup Harness Architecture — Deep Dive

Investigation of `/projects/cortex-code/tools-throwaway/mockup-harness/` to prepare the v2 mockup gallery mission. Build verified working 2026-07-08 (see §7).

---

## 1. Overall architecture

**Location & isolation.** The harness lives at `tools-throwaway/mockup-harness/` and is its **own workspace root** — `Cargo.toml:7` declares an empty `[workspace]` table so it is fully detached from the parent Cortex Code workspace (root `cargo build` never sees it). Deps are pinned (`ratatui =0.29.0`, `crossterm =0.28.1`) to versions already vendored in the parent `Cargo.lock` so the crate cache is reused offline.

**Three source files + assets:**

| Path | Role |
|---|---|
| `src/main.rs` (390 lines) | CLI entry, screen registry, render→txt→png pipeline, PNG rasterizer module |
| `src/chrome.rs` (745 lines) | Shared palette, glyphs, layout frames, transcript/table/card helpers |
| `src/screens/` (45 files + `mod.rs`) | One `pub fn render(f: &mut Frame)` per screen, static stub data |
| `assets/DejaVuSansMono.ttf` (343 KB), `assets/DejaVuSansMono-Bold.ttf` (334 KB) | Embedded via `include_bytes!` for PNG glyph rasterization |

**Registry.** `registry()` at `src/main.rs:37` returns a `Vec<Entry>` where `Entry = (&'static str category, &'static str screen-name, RenderFn)` (`main.rs:30-33`). It is a **fixed-order Vec, not a HashMap — deterministic output ordering** is an explicit design goal. 45 screens across 5 categories:

- `orchestration` (9): mode-switch, workflow-launch, workflow-monitor, team-board, reviewer-output, intent-scorer-report, concurrency-metrics, verification-ladder, tool-envelope
- `session` (14): home, onboarding, chat-turn, streaming, reasoning, tool-running, tool-success, tool-failed, diff-edit, tool-task, rich-response, shell, compact, todo-list
- `input` (12): slash-menu, command-palette, model-picker, agent-picker, skills-picker, session-picker, permission-prompt, diff-viewer, status, timeline, mcp, help
- `auth` (4): login, connect-provider, connect-apikey, connect-oauth
- `states` (6): error-network, error-context, rate-limit, empty, toasts, interrupt

Screen names use kebab-case in the registry; module files use snake_case (`chat-turn` → `screens/chat_turn.rs`). `src/screens/mod.rs` is a flat list of `pub mod` declarations grouped by category comment. **Adding a screen = 3 touches:** new file in `screens/`, one `pub mod` line in `mod.rs`, one registry tuple in `main.rs`.

**Render pipeline** (`main.rs:106-145`):
1. `render_buffer()` (`main.rs:106`): constructs a `ratatui::backend::TestBackend` at fixed **120×40** (`WIDTH`/`HEIGHT`, `main.rs:27-28`), wraps it in `Terminal`, calls `terminal.draw(|f| render(f))`, and clones the resulting `Buffer`. No real terminal is touched; crossterm is only a ratatui dependency.
2. `buffer_to_text()` (`main.rs:117`): walks the buffer row-major, pushing each `cell.symbol()`; each line is `trim_end_matches(' ')`-ed (trailing-space trim only, so assets diff cleanly), then `\n`. Exactly HEIGHT lines.
3. `write_screen()` (`main.rs:135`): `create_dir_all(base/cat)`, writes `<name>.txt`, then `maybe_write_png()`.
4. `maybe_write_png()` (`main.rs:147` with png feature; `:162` no-op stub without): **soft-fail** — PNG errors only print `note: png ... skipped` to stderr; `.txt` is declared the source of truth.

**Determinism contract** (doc comment `main.rs:1-18`): fixed 120×40, static seed data, no clock, no randomness, no env, no network → byte-identical `.txt` on re-run.

**PNG rasterizer** (`mod png`, `main.rs:246-395`, only compiled under `--features png`):
- Crates: `image =0.25.9` (RgbImage output) + `fontdue =0.9.3` (pure-Rust TrueType rasterizer).
- Cell metrics: `PX=17.0` font px, cell `CW=10 × CH=21` px, `BASELINE=16` (`main.rs:256-259`). Frame = 120·10 × 40·21 = **1200×840 px** (verified on rendered `home.png`).
- Per cell: fill bg rect (`fill_cell`), then rasterize the cell's first char with fontdue (bold font if `Modifier::BOLD`) and alpha-blend fg over bg using the coverage bitmap (`draw_glyph`, `main.rs:324`). fontdue metrics are y-up; glyph top = `BASELINE - ymin - height`.
- Defaults: `DEFAULT_BG = [13,13,13]` (= CANVAS), `DEFAULT_FG = [208,208,206]` (= TXT) at `main.rs:261-262`.
- `color_to_rgb()` (`main.rs:367`): passes `Color::Rgb` through verbatim, maps the 16 named ANSI colors to hardcoded RGB approximations of the palette (e.g. `Red → [224,96,96]` = ERR, `Green → [96,204,110]` = OKG, `Yellow → [242,183,120]` = AMBER), and falls back to the default for `Indexed(_)` — never panics.

**CLI surface** (`main.rs:181`): `--list` (grouped listing), `--all`, `<screen-name>`, `<category>` (renders whole group), `-h/--help`. Selection filters the registry by exact name-or-category match, preserving registry order. No args → usage + exit 2.

---

## 2. Full theme / palette (chrome.rs:22-75)

The palette is documented as "an exact mirror of `frame::style` (mono-orange on near-black). ORANGE is the only UI hue; green/red/blue are reserved for status dots."

### Base tokens

| Const | Line | RGB | Hex | Role |
|---|---|---|---|---|
| `CANVAS` | chrome.rs:25 | 13,13,13 | `#0d0d0d` | Page background (every frame variant paints this first) |
| `ORANGE` | :27 | 232,146,86 | `#e89256` | Primary brand accent: `›` prompts, key-hint keys, active nav pill bg, filled progress cells, streaming caret |
| `ORANGE_D` | :29 | 150,92,54 | `#965c36` | Dim orange: composer border, `card()` borders, top rule of `page()` |
| `AMBER` | :31 | 242,183,120 | `#f2b778` | Lighter accent: titles (`agent_head`, card titles, rule_head), WARN alias, active-call dot |
| `TXT` | :33 | 208,208,206 | `#d0d0ce` | Body text; also PNG default fg |
| `BRIGHT` | :35 | 242,242,240 | `#f2f2f0` | Emphasized/selected text, "Cortex Code" wordmark, user command text, model name in footer |
| `DIM` | :37 | 140,140,140 | `#8c8c8c` | Secondary text, labels, MUTED alias |
| `FAINT` | :39 | 92,92,98 | `#5c5c62` | Tertiary text, placeholders, ghost composer text, `·` separators, table borders (`vsep`, `table_rule`) |
| `GHOST` | :41 | 60,60,66 | `#3c3c42` | Rules/dividers, empty progress track, footer rule |
| `PANEL` | :43 | 22,22,26 | `#16161a` | Panel surface *(defined, **unused by any screen**)* |
| `PANEL2` | :45 | 28,28,33 | `#1c1c21` | Alt panel surface *(unused)* |
| `BAR` | :47 | 20,20,24 | `#141418` | Status-bar bg *(unused)* |
| `SELBG` | :49 | 44,35,26 | `#2c231a` | Warm selected-row bg *(unused by screens)* |
| `BORDER_BRAND` | :51 | 92,66,46 | `#5c422e` | Brand card border *(unused)* |
| `OKG` | :53 | 96,204,110 | `#60cc6e` | Success green — status dots, diff `+`, ✓ glyph, "live" indicator |
| `ERR` | :55 | 224,96,96 | `#e06060` | Error red — status dots, diff `−`, ✗ glyph |
| `INFO_DOT` | :57 | 96,170,224 | `#60aae0` | Info blue — status dots, hunk headers, worker ids |
| `ADD_BG` | :59 | 18,40,26 | `#12281a` | Dark green wash behind `+` diff rows |
| `DEL_BG` | :61 | 48,22,22 | `#301616` | Dark red wash behind `−` diff rows |
| `SOFT` | :62 | 34,28,22 | `#221c16` | Warm soft bg *(unused)* |

Plus `PETALS` (chrome.rs:64): spinner glyph array `["·","✢","✻","✽"]` — currently unused by any screen.

### Semantic aliases (chrome.rs:68-75) — what screens actually import

| Alias | → Token | Occurrences in screens/ |
|---|---|---|
| `ACCENT` | ORANGE | 107 |
| `TITLE` | AMBER | 108 |
| `OK` | OKG | 133 |
| `WARN` | AMBER | 57 |
| `BAD` | ERR | 58 |
| `INFO` | INFO_DOT | 49 |
| `MUTED` | DIM | 51 |
| `LOCKED` | ORANGE_D | 4 (rare) |

Raw-token usage in screens: DIM 130, TXT 87, FAINT 71, BRIGHT 65, AMBER 26, GHOST 22, ORANGE 19, ADD_BG/DEL_BG 4 each, ORANGE_D 4; OKG/ERR/INFO_DOT/CANVAS ≈0 (reached via aliases).

### v2 palette-migration implications

- Screens overwhelmingly use the **semantic aliases**, so re-pointing `ACCENT/TITLE/OK/WARN/BAD/INFO/MUTED` in chrome.rs restyles ~90% of the gallery without touching screen files. A minimalist Claude Code-style palette (1 accent + semantic green/red) mostly means: `TITLE` and `WARN` stop aliasing AMBER (today the accent family bleeds into titles and warnings), `INFO` blue either collapses into gray or the single accent, and ORANGE/ORANGE_D/AMBER collapse to one accent + at most one dim variant.
- Places that hardcode the orange family directly and would need edits: `page()` nav pill + top rule, `composer()` border (ORANGE_D), `keyhints()` keys (ORANGE), `footer()`, `user_cmd()`/`call_*` markers, `bar_spans()` fill — plus the 19 raw `ORANGE` uses inside screens (e.g. the streaming caret `streaming.rs:57`, model_picker search prompt).
- The **PNG named-color fallback table** (`main.rs:367-390`) hardcodes palette-approximating RGBs for ANSI names; screens never use named colors, so it only matters if v2 does.
- Dead tokens (PANEL, PANEL2, BAR, SELBG, BORDER_BRAND, SOFT, PETALS) can simply be dropped in v2 — `#![allow(dead_code)]` at chrome.rs:11 hides them today.

---

## 3. Shared chrome helpers (chrome.rs)

### Span constructors (chrome.rs:130-152)
- `t(s) -> Span` — TXT body span.
- `m(s) -> Span` — DIM muted span.
- `c(s, color) -> Span` — colored span.
- `cb(s, color) -> Span` — bold colored span.

### Progress / misc
- `bar(filled, total) -> String` (:89) — `█`/`░` string.
- `bar_spans(filled, total) -> Vec<Span>` (:103) — fill in ORANGE, track in GHOST.
- `fmt_elapsed(secs) -> String` (:114) — `42s` / `4m 12s` / `1h 02m 03s`.

### Frame variants (each paints CANVAS bg over `f.area()` first, returns body Rect(s))

1. **`page(f, active_nav, title, hints) -> Rect`** (:165) — "FLAT PAGE MANNER": top rule (ORANGE_D `─`×width) · brand+title row (` Cortex Code · <title>` left, `mockup · stub data` right) · nav tab row (`NAV: [&str;8]` at :160 = Mode/Workflow/Team/Review/Intent/Metrics/Verify/Tools; active tab is an inverted CANVAS-on-ORANGE pill) · spacer · **body (Min)** · statusbar row · GHOST footer rule · key hints. **Currently used by 0 screens** — all 45 have migrated to the CLI manner below; `page()`, `statusbar()`, `NAV`, `two_col()`, `section()` are legacy.
2. **`cli_page(f, _title, composer_ghost, hints) -> Rect`** (:361) — "REAL-CLI MANNER" with composer. Rows: transcript body (Min; **35 rows** at 120×40) · composer(3) · footer(1). Used by **26 screens** (all session + states + some orchestration).
3. **`cli_view(f, _title, hints) -> Rect`** (:385) — no composer; body(Min) · footer(1). For read-only monitoring/overlay views ("you read them, you don't type"). Used by **8 screens** (workflow_monitor, team_board, concurrency_metrics, intent_scorer_report, verification_ladder, tool_envelope, onboarding, empty).
4. **`cli_action(f, _title, dock_h, hints) -> (Rect body, Rect dock)`** (:430) — composer replaced by an **action dock** `dock_h` rows tall; the caller draws a `card()` into the dock. This is the N-7 convention (cortex-code-design skill log): permission prompts and pickers dock in place of the input line, **never** a centered floating modal. Used by **11 screens** (all pickers, permission-prompt, slash-menu, command-palette, login, connect-*).

Note: `_title` is ignored by all three CLI variants; `cli_header()` (:327) exists (brand row + `session wf-42 · agent mode` right meta) but is **not called** by them — the CLI frames deliberately have no top chrome so they read as inline scrollback output, not a full-screen TUI.

### Private frame internals
- `statusbar(f, area, active_nav)` (:248) — page()-only session line: `● live · <nav> · model claude-opus · ctx 47% · cost $0.0184` left, `~/projects/cortex-code` right.
- `keyhints(hints, _width) -> Paragraph` (:293) — `key desc` pairs, keys in ORANGE, descs in DIM, ` · ` FAINT separators.
- `composer(f, area, ghost)` (:455) — rounded ORANGE_D-bordered box, `› ` ORANGE bold prompt + FAINT ghost text.
- `footer(f, area, hints)` (:475) — hints left; right side `claude-opus · 47% · $0.0184 · ● live` (BRIGHT/TXT/OKG).

### Layout utilities
- `two_col(area, left_pct) -> (Rect, Rect)` (:280) — horizontal split with a **2-col gutter** (unused today).
- `section(f, area, title, accent) -> Rect` (:535) — bold title row (2-space indent), body below, no box (unused).
- `rule_head(f, area, title, accent) -> Rect` (:555) — ` title ` bold in accent + GHOST `─` rule to the edge; consumes 1 row. Used by 6 screens as the standard CLI block divider.
- `card(f, area, title, accent, hints) -> Rect` (:574) — rounded ORANGE_D border, embedded ` title ` in accent bold, optional key hints drawn inside the bottom row of the inner area, **no filled surface**. Used by 15 screens (docks, boxed panels).

### Transcript idiom (the CLI vocabulary)
- `user_cmd(cmd, time) -> Line` (:495) — ` › cmd` (ORANGE marker, BRIGHT bold text) + FAINT `· hh:mm:ss` inline after 4 spaces. Used in 31 screens.
- `agent_head(title) -> Line` (:511) — 3-space pad + AMBER bold heading. Convention: **agent prose indents to column 3** under it. 20 screens.
- `transcript(f, body, lines)` (:524) — **bottom-anchors** the lines in the body Rect (spare space at the top, newest line hugs the composer, like real scrollback). 37 screens — the universal body renderer.
- `info(label, value, color) -> Line` (:594) — `  label: value`, DIM label; empty label = continuation row.
- `pill(text) -> Span` (:607) — inverted CANVAS-on-ORANGE badge.
- `status(color, text) -> Vec<Span>` (:618) — `● text` both in the state hue. 19 screens.
- `dot(color)` (:626) / `state_glyph(color)` (:630 — OKG→`✓`, ERR→`✗`, INFO_DOT→`○`, else `●`) / `chip(text, color)` (:640).

### Tool-call turns
- `call_collapsed(state, reason, summary)` (:647) — `   ● reason   › summary` (unused today).
- `call_expanded(state, reason)` (:659) — `   ● reason` BRIGHT bold head.
- `call_detail(spans)` (:671) — `     → …` detail row at 5-col indent.
- `call_head(state, title, rest)` (:680) — 1-space-indent variant.
- `call_active(header, elapsed)` (:696) — ` ● Working  (6s · Esc to interrupt)` AMBER dot; the live-turn band. 5 screens.
- `call_branch(spans, first)` (:711) — `   └ …` for the first branch row, 5-space pad for continuations.
- `call_result(ok, code, detail)` (:724) — ` ✓ (0) · detail` / ` ✗ (1) · …`.

### Tables
- `table_rule(widths, left, mid, right) -> Line` (:406) — builds `┌┬┐`/`├┼┤`/`└┴┘` rules from **content widths + 2 padding**, so junctions land exactly under the `│` separators. FAINT.
- `vsep() -> Span` (:416) — FAINT `│`. Both used by 11 screens; screens define their own `cell()` wrapper (1-space pad each side + vsep) and fixed `COLS: [usize; N]` arrays chosen so the total row width is exactly 120 (e.g. `workflow_monitor.rs:20-21`: `1 + Σ(w+2) + n = 1 + 113 + 6 = 120`).

### Layout conventions summary
- Margins: chrome rows lead with a single space (` Cortex Code`, ` › cmd`, ` enter send`); transcript prose indents 3 (`   `); call details indent 5; info rows indent 2.
- Transcript column: everything under `agent_head` aligns at col 3; prose is hand-wrapped by the screen author to fit 120 cols (no automatic wrapping anywhere).
- Glyph set is deliberately DejaVu-safe (chrome.rs:78-86): `›` marker (never `▶`), `·` separator, `█`/`░` progress, `●`/`○`/`◉` state dots, `✓`/`✗`, box-drawing.

---

## 4. Output paths

`out_dir()` (`main.rs:91-104`): takes `option_env!("CARGO_MANIFEST_DIR")` (compile-time = `<repo>/tools-throwaway/mockup-harness`), walks up **two ancestors** to the repo root, and joins **`plans/assets/mockups`**. Fallback: CWD-relative `plans/assets/mockups` if the env var was unset at compile time.

Per screen: `write_screen()` creates `<out>/<category>/` and writes `<screen>.txt` (+ `<screen>.png` with the feature). Current tree: `plans/assets/mockups/{orchestration,session,input,auth,states}/` — 45 `.txt` + 45 `.png`, plus hand-written `README.md` and `UX-AUDIT.md` at the root of `mockups/` (**not generated** by the harness; a v2 output root would not carry them over automatically).

**Changing to `plans/assets/mockups-v2`** = editing two string literals inside `out_dir()`: the `join("plans/assets/mockups")` at `main.rs:100` and the fallback `PathBuf::from("plans/assets/mockups")` at `main.rs:102` (plus the usage doc comment at `main.rs:17`). Category subfolders are created on demand. The path is baked at **compile time** via `option_env!`, so it resolves to the repo checkout wherever it lives; no runtime env is read.

---

## 5. Anatomy of a screen file (walkthroughs)

Every screen: a `//!` doc comment stating the scenario, ratatui imports, `use crate::chrome::{...}` picking span helpers + semantic colors, one `pub fn render(f: &mut Frame)`, optional private line-builder fns and layout consts. All data is inline string literals — no state, no args.

### 5a. `screens/chat_turn.rs` (simple transcript screen, 97 lines)
1. Doc comment: one completed exchange, no spinner.
2. `render()` calls `chrome::cli_page(f, "Chat", "reply to Cortex, or ask a follow-up…", &[("enter","send"),("↑","edit last"),("/","commands"),("q","quit")])` → transcript body Rect.
3. Builds a `Vec<Line>`: `chrome::user_cmd("how does the rate limiter…", "14:23:41")` + blank; `chrome::agent_head("Cortex")` + blank; hand-wrapped prose paragraphs via a local `para()` helper that prepends `Span::raw("   ")` (the 3-col alignment) — mixing `t()` body text with `c(…, TITLE)` inline code terms and `c(…, BRIGHT)` numbers; blank; a takeaway row `● In short …` (`dot(OK)` + OK-colored lead + TXT tail).
4. `chrome::transcript(f, body, lines)` bottom-anchors the whole thing.

Pattern: **screen = frame call + Vec<Line> assembly + transcript()**. No Rect math at all.

### 5b. `screens/diff_edit.rs` (diff idiom, 133 lines)
1. `cli_page()` with a y/n composer ghost `"[y/n/edit]  apply, skip, or ask for a change…"` and y/n/ctrl+o hints.
2. `user_cmd(…)`; `call_expanded(OK, "Proposed an edit to crates/server/src/models/session.rs")`; `call_detail([c("+2",OK), …, c("−1",BAD), m("· 1 hunk")])`.
3. Diff block via local helpers over `diff_row(old, new, sign, sign_color, code_color, code, bg)`: fixed gutters `OLD_W=3`, `NEW_W=3`, band width `DIFF_W=108`; `ctx()` rows DIM/no bg, `add()` rows OK on `Some(ADD_BG)`, `rem()` rows BAD on `Some(DEL_BG)`, `hunk()` header INFO bold. When a bg wash is present the row is **manually padded**: `vis = 2+3+1+3+1+2+code.chars().count()`, then a `" ".repeat(DIFF_W − vis)` span carrying the bg so the tint spans a fixed 108-col band.
4. Two prose rows explaining the edit, then `transcript()`.

Complex region screens (cf. `workflow_monitor.rs`): `cli_view()` + a `Layout` vertical split of the body (`Length`s + `Min(0)`), `rule_head()` per region, side-by-side panels via `Layout::horizontal` percentage split, and fixed `COLS` arrays with `format!("{:<w$}")` padding for grids.

### 5c. Dock screens (third shape)
`permission_prompt.rs` / `model_picker.rs`: `cli_action(f, _, dock_h=11, hints)` → dim `backdrop()` lines into the body via `transcript()`, then `card(f, dock, "Permission required"/"Select model", AMBER, &[])` and hand-built option/list rows (selection = `› ` ACCENT marker + BRIGHT bold padded label; current item = `● current` badge in OK; right-aligned metadata padded with `width.saturating_sub(used + meta_len + 1)`).

---

## 6. Brittleness & noteworthy details

- **All width math is `chars().count()` / `format!("{:<w$}")`** — char count, *not display width*. Safe only because the chrome glyph set is strictly single-width (`›·█░●○◉✓✗→└─│`, box drawing). **Any emoji, CJK, or double-width char silently breaks column alignment and the 120-col table math.** No unicode-width crate anywhere. v2 must keep the DejaVu-safe single-width glyph discipline (or add `unicode-width`).
- **No text wrapping.** ratatui `Paragraph` is used without `.wrap()`; every prose line is hand-broken by the author to fit (~117 usable cols at 3-col indent). Overlong lines are clipped by the buffer, not wrapped.
- **Table widths are hand-balanced to exactly 120** (`workflow_monitor.rs:20-21` and `:262` document the arithmetic). Changing any column width means redoing the sum by hand.
- **Bottom-anchored transcript** (`transcript()`, chrome.rs:524): if lines exceed the body height the Paragraph gets the full body Rect and simply **truncates the overflow** — no scrolling. Screens must keep line counts ≤ 35 (cli_page) to avoid silent truncation.
- `buffer_to_text` trims trailing spaces per line; **bg-only cells (ADD_BG/DEL_BG washes) are invisible in `.txt`** — the PNG is the only artifact that shows background washes.
- **PNG cell drawing uses only the first char** of a cell symbol (`main.rs:295`) — fine for single-width glyphs, wrong for grapheme clusters. Cell aspect is hardcoded 10×21 px @ 17 px DejaVu.
- `_title` params on `cli_page`/`cli_view`/`cli_action` are accepted and ignored; `cli_header()`, `page()`, `statusbar()`, `NAV`, `two_col()`, `section()`, `call_collapsed()`, `PETALS` and several palette tokens are dead/legacy code kept intentionally — chrome.rs:9-11 explains the module mirrors the full design-base inventory and sets `#![allow(dead_code)]`.
- Footer/status stub data (`claude-opus · 47% · $0.0184`, session `wf-42`, `~/projects/cortex-code`) is hardcoded inside chrome.rs (`footer()` :475, `statusbar()` :248, `cli_header()` :327) — one place to change for a v2 restyle.
- The design authority is the **`cortex-code-design` skill** (the owner's do/don't log): referenced at `permission_prompt.rs:3-5` and chrome.rs:437 as rule "N-7" (decisions dock in place of the composer, never floating modals). Read it before any v2 visual work.

---

## 7. Build & render commands (verified 2026-07-08)

```sh
cd /projects/cortex-code/tools-throwaway/mockup-harness

cargo run --quiet --features png -- --list      # 45 screens in 5 categories
cargo run --quiet --features png -- home        # one screen
cargo run --quiet --features png -- session     # a whole category
cargo run --quiet --features png -- --all       # everything
cargo run --quiet -- --all                      # .txt only (no png feature)
```

- **Clean build with `--features png`: ~7.1 s wall** (34.8 s user; warm crate cache, deps resolve offline against the pinned versions shared with the parent workspace lockfile). Incremental rebuild after touching `src/main.rs`: **~0.3 s**. A `--list` run: ~0.05 s.
- The png feature needs **nothing special** beyond the two TTFs already in `assets/` (embedded at compile time via `include_bytes!`, `main.rs:264-265`) — no system fonts, no network, pure-Rust rasterization (`fontdue` + `image`).
- Verified render: `cargo run --quiet --features png -- home` wrote both
  `/projects/cortex-code/plans/assets/mockups/session/home.txt` (1,510 B) and
  `/projects/cortex-code/plans/assets/mockups/session/home.png` (108,856 B, 1200×840 px).
- PNG failure is soft (stderr `note:`), never fails the run; an unwritable output dir is a hard `exit(1)`.
