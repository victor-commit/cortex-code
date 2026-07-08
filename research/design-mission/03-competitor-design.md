# 03 — Competitor TUI Design Research

> Research for the Cortex Code v2 visual redesign. Direction already chosen: minimalist
> palette (one brand accent + semantic green/red), low visual noise, no emojis, no
> decorative glyphs, human-friendly.
>
> Sources: scraped 2026-07-08 (official docs, GitHub READMEs/issues, reviews, design
> essays — see "Sources" at the bottom) plus first-hand knowledge of these TUIs.
> Scraped raw material is cached in `.firecrawl/` at the repo root.

---

## A. Per-competitor visual conventions

### A1. Claude Code (Anthropic) — the benchmark for restraint

The most-copied design in the category. Its defining trait: **almost everything is the
terminal's default foreground or dim gray; color appears only where it carries meaning.**

**Layout / chrome**
- No fullscreen takeover by default: renders inline into normal scrollback (a "chat log
  that grows"), so native terminal scroll/search still work. Fullscreen transcript viewer
  is opt-in (`Ctrl+O`).
- One visual column. No sidebars, no panels. Hierarchy is done with indentation, not boxes.
- The **only box on screen is the input composer**: a single-line rounded border
  (`╭─...─╮ / │ > │ / ╰─...─╯`) in dim gray, with a `>` prompt char and a grayed-out
  placeholder suggestion ("Try 'fix typecheck errors'") mined from your git history.
- Below the composer: a two-column **hint area in dim gray** — `! for bash mode`,
  `/ for commands`, `@ for file paths`, `shift+tab to auto-accept edits`, etc. It rotates
  and shrinks as you learn (progressive disclosure).
- Footer row of "badges": cwd, git branch, PR link ("PR #446" with a colored underline —
  green approved, etc.), context indicator. A user-scriptable statusline row can render
  above the built-in badges (never replaces them).

**Tool-call rendering (the ⏺ system)**
- Every agent action is one line: `⏺ Read(src/main.rs)`, `⏺ Bash(cargo test)`,
  `⏺ Update(file.ts)`. The `⏺` bullet is the state pixel: colored by outcome
  (green = ok, red = failed) while assistant prose gets a plain/white `⏺`.
- Tool *results* collapse to one indented continuation line prefixed with the elbow glyph
  `⎿` : `⎿ Read 124 lines`, `⎿ Found 3 files`. Expanded detail lives behind `Ctrl+O`
  (transcript view); repeated MCP calls collapse to "Called slack 3 times".
- Net effect: a scannable "trace" where 1 action = 1 line = 1 status color.

**Spinner / progress**
- A single animated star/asterisk glyph (`✻`/`·✢✳✶` frames) in the brand terracotta,
  followed by a whimsical gerund verb — "Pondering…", "Reticulating…", "Honking…",
  "Simmering…" — plus live counters in dim gray: `(37s · ↑ 2.4k tokens · esc to interrupt)`.
  Personality lives *only* in the verb; geometry and color stay constant.
- Long-running bash gets the same treatment, not a progress bar.

**Permission prompts**
- A bordered dialog with the question in plain text ("Do you want to make this edit to
  `foo.ts`?"), a preview of the exact command/diff, and a **numbered vertical list**:
  `1. Yes  2. Yes, and don't ask again this session  3. No (esc)`. Arrow keys + number
  keys + Enter; `Esc` always closes. Left/Right cycles dialog tabs.
- Mode is otherwise a *persistent* setting cycled with `Shift+Tab` and shown as a small
  status badge under the composer: `⏸ manual mode on` (gray), `⏵⏵ accept edits on`,
  `⏸ plan mode on` (teal-ish). One glyph + three words — the entire "mode UI".

**Diffs**
- Unified diff with line numbers; added lines rendered as dark-green *background* blocks,
  removed lines dark-red background, at full line width — readable at a glance without
  +/- signs being the only cue (though they're present too).

**Todo list**
- `Ctrl+T` toggles a max-5-row checklist in the status area: `☐` pending / `☐` bold
  in-progress / `☒`(strikethrough, green) done. No percentages, no bars.

**Color budget (observed)**
- Terracotta/coral brand accent (Anthropic's Claude orange, ~`#D97757`) — used *only*
  for the welcome mark, spinner star and occasional emphasis.
- Green/red for success/failure + diff add/remove. A teal/cyan for plan-mode accents.
- Everything else: default foreground, `dim`, and one gray. No rainbow, no emoji
  anywhere in the chrome.

**Errors**
- Red text inline, prefixed by the same `⏺`/`⎿` structure — errors are part of the
  trace, not modal interruptions.

**Weakness worth learning from** (esveo review): hierarchy is *sometimes too flat* —
"not always obvious at a glance whether text is coming from the agent, a tool call, or
system messages", and tool-call expansion is global (all-or-nothing) which creates noise
when you only want one call's detail.

### A2. OpenAI Codex CLI

Rust + ratatui. Even *more* austere than Claude Code — to a fault.

- Inline (non-fullscreen) chat log like Claude Code; composer at the bottom with a `›`
  prompt marker; header block on start showing version, model, sandbox/approval mode, cwd.
- **Uses terminal default foreground for nearly all body text.** GitHub issue #21130
  documents the consequence: users on odd terminal palettes get "plain black text",
  and explicitly ask for "semantic colors across its terminal UI **like Claude Code**,
  so the interface has much better contrast and is easier to scan". `/theme` +
  `[tui] theme = "..."` in `config.toml` affect **syntax highlighting only**, not chrome.
  Lesson: *there is such a thing as too little color* — semantic slots must exist.
- Working state: a plain `• Working (Esc to interrupt)` line with elapsed seconds; less
  personality than Claude Code, considered "flat but professional".
- Approvals: `Allow command?`-style modal with y/n/always options tied to approval modes
  (`suggest` / `auto-edit` / `full-auto`); diff preview before file writes; magenta/cyan
  is used sparingly for selection highlight.
- Diffs: classic fg-colored +green / -red unified diff (no background blocks), `/diff`
  command to review the working tree.

### A3. Gemini CLI (Google)

The maximalist of the three majors — useful mostly as a "what to tone down" reference.

- **Huge ASCII-art "GEMINI" banner** in a blue→purple gradient on every launch; theme
  picker dialog on first run; "Tips for getting started" block.
- Ships ~17 named themes (Dracula, GitHub, Ayu, Solarized, ANSI, …) selectable via
  `/theme`, plus full custom-theme JSON in `settings.json` under `ui.customThemes` with
  keys like `background.primary`, `text.primary`, `text.secondary`, `accent.blue`,
  `accent.red`, plus `ui.*` overrides (`symbol`, `gradient`, comment, border, diff.added…).
  Structurally a good token system — visually the default is busy.
- Bordered input box; **footer line** with cwd + git branch, sandbox indicator (red
  "no sandbox"), model name and **"XX% context left"** on the right.
- Loading: spinner + elapsed time + rotating witty phrases; blue `✦` bullet for model
  responses, `✓`/`x` on tool cards, which are *boxed* — heavier chrome per action than
  Claude Code's single line.
- Reviewers consistently describe it as "colorful"; the gradient banner + boxed tool
  cards + persistent footer make it the noisiest of the majors.

### A4. opencode (sst) — widely praised as the best-*looking* TUI

- **Flat, editorial, square** — no rounded corners; solid full-width background blocks
  instead of line borders. Blocky half-block pixel-font wordmark on the empty state.
  Feels "designed" rather than "drawn with box characters".
- Layout: single centered column with generous side margins on wide terminals (~80-col
  content measure); messages separated by whitespace and a thin left rule rather than
  boxes.
- **Bottom statusline as colored blocks**: cwd on the left; mode badge (BUILD/PLAN) as an
  inverted color block; model + session cost/context on the right. High density, zero
  border characters.
- Composer: plain block with a hint row; leader-key model (`ctrl+x` prefix) keeps global
  chords to a minimum; `/commands`, `@` fuzzy file search, `!` shell — same vocabulary as
  Claude Code (this vocabulary is now a de-facto standard).
- **Best-in-class theme system** (docs/themes): JSON themes with a `defs` palette layer +
  semantic tokens (`primary`, `secondary`, `accent`, `error`, `warning`, `success`,
  `info`, `text`/`textMuted`, `background`/`backgroundPanel`/`backgroundElement`,
  `border`/`borderActive`/`borderSubtle`, `diffAdded`/`diffRemoved`…, full `syntax*` and
  `markdown*` sets). Supports hex, ANSI index, refs, per-dark/light values, and `"none"`
  (inherit terminal).
- The default `system` theme **generates its gray scale from the terminal's own
  background** and uses pure ANSI 0–15 for everything else, `none` for text/bg — the
  "blend in, don't take over" philosophy taken to its logical end. Requires truecolor
  for full fidelity, degrades to 256-color approximation.
- Tool detail is toggleable per session (`/details`), addressing the exact complaint
  reviewers have about Claude Code's all-or-nothing expansion.

### A5. Briefly: Aider, Amp, Cursor CLI, Crush

- **Aider** — pre-TUI generation: a Rich-based REPL, not a full-screen app. Markdown
  rendered inline, unified diffs, auto git commits announced as text. Colors are
  user-flags (`--user-input-color`, code themes). Utilitarian and noisy; its lasting
  contributions are workflow (repo map, watch mode), not visuals.
- **Amp (Sourcegraph)** — 2026 "Neo" rebuild of the CLI. Public material emphasizes
  architecture (remote control from ampcode.com, auto-compaction at 90% context,
  plugin UI hooks `ctx.ui.notify/confirm/input/select`) over chrome; the TUI itself is
  restrained, Claude-Code-like, with a small set of plugin-facing UI primitives — an
  interesting pattern: *the UI surface is an API*.
- **Cursor CLI (cursor-agent)** — follows the Claude Code template closely (inline log,
  bordered composer, dim hints, shift-tab-style mode cycling) with Cursor's brand accent;
  nothing novel to report visually.
- **Crush (Charm; the renamed original opencode)** — the anti-minimalist pole:
  lipgloss/BubbleTea showcase, purple/pink gradients, big bordered panels, "pretty (or
  garish, depending on your taste)" (TheNewStack). Gorgeous in screenshots, heavy in
  daily use; also a good reminder that in-terminal mouse handling and copy/paste often
  break inside fancier chrome (reviewer couldn't click-to-place cursor or copy text).

---

## B. What the best do — patterns worth adopting

1. **Inline scrollback, not fullscreen** (Claude Code, Codex): render into the normal
   terminal flow so native scroll, search, and copy keep working; make any fullscreen
   view (transcript, help) opt-in and transient.
2. **One box per screen** (Claude Code): the composer is the only bordered element;
   everything else uses indentation + whitespace. Boxes are for *focus*, and there is
   exactly one focus.
3. **1 action = 1 line = 1 status color** (Claude Code's `⏺ Tool(arg)` + `⎿ result`):
   collapsed-by-default tool trace where the bullet glyph doubles as a status LED.
4. **A dedicated "result elbow" glyph** (`⎿`) to visually attach results to their action
   without nesting boxes — hierarchy from two characters, not borders.
5. **Per-item expansion** (opencode `/details`; requested-by-users for Claude Code):
   let users expand one tool call, not toggle the entire session's verbosity.
6. **Status through a badge, not a banner** (Claude Code `⏵⏵ accept edits on`): mode =
   glyph + 2–3 lowercase words in one color, sitting under the composer. No modal, no box.
7. **Numbered vertical option lists for permissions** (Claude Code): `1. Yes  2. Yes,
   session  3. No (esc)` — number keys + arrows + esc; show the exact command/diff above
   the question.
8. **Background-block diffs** (Claude Code): full-width green/red *background* tint for
   add/remove with line numbers beats fg-only `+`/`-` for scannability; keep fg-only as
   the 16-color fallback (Codex style).
9. **Spinner = fixed glyph + rotating verb + live counters** (Claude Code): personality
   confined to the word ("Pondering…"), signal in the dim counters
   (`37s · ↑2.4k tokens · esc to interrupt`). Never bars for unbounded work.
10. **Dim hint footer with the top 3–6 keys, rotating** (Claude Code, jensroemer's
    write-up): progressive disclosure — footer floor, `?`/`/help` ceiling, docs last.
    Show hotkeys *where they act*, not in a manual.
11. **Placeholder-as-onboarding** (Claude Code): the empty composer teaches
    (`Try "fix typecheck errors"`) in gray; costs zero space.
12. **Semantic color tokens, not hex-in-code** (opencode themes, Gemini customThemes,
    Opaline essay): `text.muted`, `status.error`, `diff.added` — a palette→token→style
    pipeline makes restraint enforceable and theming free.
13. **`"none"` as a first-class color** (opencode `system` theme): inherit the user's
    terminal fg/bg by default; generate grays *relative to their background*. The app
    should look native in any terminal.
14. **Usable at 16 colors, beautiful at truecolor** (terminal-renaissance essay;
    opencode fallback behavior): design monochrome-first, layer ANSI-16 semantics, then
    truecolor polish. Respect `NO_COLOR` unconditionally.
15. **Shared vocabulary** (all): `/` commands, `@` files, `!` shell, `Esc` interrupt,
    `Shift+Tab` mode cycle, `Ctrl+O`-style detail toggle. Users arrive pre-trained —
    deviating is a cost, not a brand.
16. **Footer facts, right-aligned, dim** (Gemini, opencode statusline): cwd · branch ·
    model · context-remaining in one gray line; color only the one item that's abnormal
    (e.g. red "no sandbox", context < 20%).
17. **Checklist, not progress bar, for plans** (Claude Code todo): ≤5 visible rows,
    `☐/☒` + strikethrough; state is obvious without percentages.
18. **Errors stay in the trace** (Claude Code): red line under the failed action with the
    same layout as success — no popups; the transcript remains the single source of truth.
19. **Whitespace as separator** (opencode): blank line between turns, thin left rule for
    quoted/agent text; zero horizontal rules (`────`) between every message.
20. **Constrain the content measure** (opencode): on wide terminals, cap the text column
    (~80–100 cols) and center it; full-bleed lines are only for diffs/code.
21. **Color the underline/edge, not the word** (Claude Code's PR badge with colored
    underline): a way to encode status without another colored word in the text.
22. **Session recap on return** (Claude Code): one dim line summarizing what happened
    while you were away — human-friendly with zero chrome.

---

## C. What to avoid (observed in the wild)

- **ASCII-art banners on every launch** (Gemini): burns 6–10 rows, pure decoration,
  ages instantly. A one-line `cortex vX.Y — model · cwd` header is enough.
- **Gradients and rainbow palettes in chrome** (Crush, Gemini banner): screenshot candy,
  daily-use fatigue; breaks on 256-color/SSH; described as "garish" by reviewers.
- **Boxing every message/tool card** (Gemini tool cards, Crush panels): border characters
  become the loudest thing on screen; density collapses.
- **Too little semantic color** (Codex #21130): terminal-default-fg for *everything*
  makes the UI unscannable and hostage to the user's palette. Reserve color, don't
  abolish it.
- **All-or-nothing detail toggles** (Claude Code Ctrl+O criticism): global expansion
  floods the transcript; expansion must be per-item.
- **Emoji as UI vocabulary** (aider's marketing, many wrappers): inconsistent width and
  rendering across terminals/fonts; unserious next to code. (Direction already bans this.)
- **Flat hierarchy of speakers** (esveo on Claude Code): agent prose, tool output and
  system notices must be distinguishable *without reading* — use the bullet/elbow/dim
  system consistently.
- **Color as the only channel**: if the app is unusable with color stripped, the design
  is broken (renaissance essay). Glyph + position must carry state too — this is also
  the colorblind-safety rule.
- **Stacked emphasis** (bold+color+underline+reverse): one channel of emphasis at a time.
- **Fancy chrome that breaks terminal basics** (Crush in Warp): mouse-cursor placement,
  copy/paste, scrollback — if a visual choice breaks any of these, drop it.
- **Progress bars for unbounded work**; **percentages on todo lists**; **blinking**.
- **Decorative box-drawing dividers** (`════`, `────` after every section) — whitespace
  and dim indentation do the job at zero noise.

---

## D. Proposed minimalist color spec — Cortex Code v2

Philosophy: *the transcript is the product; color is a status LED, not paint.*
Dark-terminal-first, truecolor with a defined ANSI-16 fallback, `"none"` (terminal
default) wherever possible. Stripping all color must leave a fully usable UI.

### D1. Roles and values

| Token | Hex (dark) | ANSI-16 fallback | May be used for | Never used for |
|---|---|---|---|---|
| `text` | `none` (terminal fg) | default | body text, agent prose | — |
| `text.bright` | `#E6E9EF` | bright white (15) | headers, focused item, in-progress todo | body text |
| `text.muted` | `#9AA3AF` | bright black (8) | metadata, counters, timestamps, key hints, placeholders, collapsed results | anything the user must not miss |
| `text.faint` / `border` | `#4C5566` | bright black (8) + dim | composer border, separators, `⎿` elbows, disabled items | text longer than a few words |
| `bg.surface` | `#262B33` | — (skip at 16-color) | selection rows, statusline block, dialog fill (≈6% lighter than typical bg) | full-screen fills |
| **`accent` (brand)** | **`#D9825F` copper** | yellow (3) | spinner glyph, welcome mark, mode badge when elevated (auto/accept-edits), focused-option marker, brand wordmark | success/error meaning, body text, borders at rest |
| `ok` | `#7FB069` | green (2) | completed action bullet, diff added, pass counts, approved states | decoration, branding |
| `err` | `#D96C75` | red (1) | failed action bullet, diff removed, errors, destructive-confirm highlights | warnings, emphasis |
| `warn` *(optional, rare)* | `#D9B36C` | yellow (3) | genuinely dangerous states only (no-sandbox, context < 10%) | routine notices (use `text.muted`) |
| `info` *(optional)* | `#6C9BD9` | blue (4) | links, plan-mode badge | general accenting |

Copper `#D9825F` is deliberately adjacent-but-distinct from Claude's terracotta `#D97757`
— warm, legible on dark, and (being non-semantic) safe for red-green colorblind users
since it never has to be *distinguished from* `ok`/`err` to be understood.

### D2. Usage rules

1. **Accent budget:** ≤ ~5% of visible cells; at most one accent element per region
   (spinner *or* badge *or* focus marker — the eye should land on exactly one).
2. **Green and red are verdicts.** They may appear only after something *happened*
   (tool result, diff line, test outcome) or on the confirm button of a destructive
   action. Never in headers, borders, prompts, or branding.
3. **Chrome is gray.** Borders, hints, elbows, counters, statusline: `text.muted`/
   `text.faint`. The composer border may switch to `accent` only while a permission
   dialog or elevated mode demands attention.
4. **State = glyph + color, always both.** `●` ok/err bullets, `☐/☒` todos, `⎿` results
   carry the state in shape as well as hue (colorblind + NO_COLOR safety).
5. **Diffs:** added = `ok` fg on `#26331F`-tinted bg; removed = `err` fg on `#33211F` bg;
   at 16-color drop the backgrounds, keep fg + `+`/`-`.
6. **Emphasis ladder:** dim → default → bright → bold. Bold is for ≤1 phrase per screen;
   underline only for links; reverse video only for the statusline mode block; never
   stack more than one.
7. **Degradation:** honor `NO_COLOR` (strip everything, glyphs carry state); at
   `TERM=*-256color` map to nearest; at 16 colors use the fallback column and drop
   `bg.surface` tints.
8. **No other hues exist.** Syntax highlighting in code blocks is the single sanctioned
   exception and should itself be a muted 5-color scheme derived from the tokens above.

---

## Sources

- Claude Code docs: interactive mode, statusline, permission modes
  (code.claude.com/docs/en/{interactive-mode,statusline,permission-modes}) — scraped.
- J. Rømer, "TUI design" (jensroemer.com/writing/tui-design) — Claude Code composer/help
  reproduction, terminal.shop hotkey pattern — scraped.
- S. Jane, "The Terminal Renaissance" (hyperbliss.tech, 2026-04-04) — semantic color
  slots, three-tier color model, seven layout archetypes, progressive disclosure — scraped.
- opencode docs: /docs/tui, /docs/themes (theme JSON schema, `system` theme) — scraped.
- openai/codex GitHub README + issue #21130 ("Allow configuring semantic TUI colors
  beyond syntax highlighting") — scraped.
- google-gemini/gemini-cli docs/cli/themes.md (theme list, customThemes schema) — scraped.
- esveo, "Let's Try Out OpenCode UI" (2026-03-13) — Claude Code hierarchy criticism — scraped.
- TheNewStack, "Terminal User Interfaces: Review of Crush" — Charm aesthetic critique,
  terminal-basics breakage — scraped.
- ampcode.com/news/neo — Amp CLI rebuild, plugin UI primitives — scraped.
- LogRocket "Gemini CLI vs Codex CLI", gradually.ai 3-way comparison — scraped.
- Specific Claude Code glyph/spinner/color details (⏺/⎿, verb list, badge wording)
  corroborated from first-hand product knowledge current to early 2026; treat exact
  glyph choices as "observed convention", not published spec.
