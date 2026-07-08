# 02 — Cortex Code Page Inventory (design-mockup mission scoping)

Sources mined:
- Real TUI: `crates/tui/src/` — `slash.rs:19` (`BUILTINS`, 42 commands), `view.rs:41` (`ViewKind`: Status/Timeline/Queue/Mcp/Diff/Help/Review), `app.rs:178` (`DialogKind`: 10 modal pickers), `app.rs:213` (`SidebarPanel`: 7 tabs), `app.rs:713` (`PaletteScope`: 6 scopes), `connect.rs:77` (`Step`: 7-step connect flow), `app.rs:149` (`Entry` transcript kinds), plus view modules `onboarding.rs`, `auth_login.rs`, `motd.rs`, `gallery.rs`, `permission.rs`, `tool_card.rs`, `diff.rs`, `shell.rs`.
- Plans: `plans/08-cli-uiux-mockups.md` (9 orchestration frames §2.1–2.9), `plans/00-MASTER-PLAN.md` (subsystem surfaces).
- v1 gallery: `plans/assets/mockups/README.md` (says 42; **45 actually exist** — `rich-response`, `todo-list`, `tool-task` are beyond the README count) + the harness registry `tools-throwaway/mockup-harness/src/main.rs:39`.
- UX audit: `plans/assets/mockups/UX-AUDIT.md` (382 lines, audits all 45 PNGs).

---

## A. Master page table

Legend: **v1** = has a v1 mockup · **TUI** = exists in real TUI code · **Plan** = specified in plans only.

### Category 1 — session (core chat surface)

| # | Page | v1 | TUI | Source of truth |
|---|------|----|-----|-----------------|
| 1 | `home` | ✔ | ✔ | `motd.rs` (welcome card + meta panels atop scrollback) |
| 2 | `onboarding` | ✔ | ✔ | `onboarding.rs` (first-run pristine launch) |
| 3 | `chat-turn` | ✔ | ✔ | `app.rs:149` `Entry::User/Assistant` |
| 4 | `streaming` | ✔ | ✔ | `turn.rs` (TurnMessage stream → busy indicator) |
| 5 | `reasoning` | ✔ | ✔ | `Entry::Assistant` reasoning region |
| 6 | `rich-response` | ✔ | ✔ | markdown/table/code rendering in transcript |
| 7 | `tool-running` | ✔ | ✔ | `tool_card.rs` `ToolStatus::Running` |
| 8 | `tool-success` | ✔ | ✔ | `tool_card.rs` `ToolStatus::Completed` |
| 9 | `tool-failed` | ✔ | ✔ | `tool_card.rs` `ToolStatus::Failed` |
| 10 | `tool-task` (subagent delegation) | ✔ | ✔ | `app.rs:346` `SubagentState` |
| 11 | `diff-edit` (inline diff card) | ✔ | ✔ | `diff.rs` `Entry::Diff` |
| 12 | `shell` (`!` shell mode cards) | ✔ | ✔ | `shell.rs` `Entry::Shell` |
| 13 | `todo-list` (plan progress) | ✔ | ✔ | sidebar-todo data rendered in transcript |
| 14 | `compact` | ✔ | ✔ | `/compact` (`slash.rs:145`) |
| 15 | `blocked-command` (plan-mode hard-deny banner) | ✘ | ✔ | `app.rs:149` `Entry::Blocked` — **gap** |
| 16 | `turn-error` (inline ✦ Cortex ✗ error block) | ✘ | ✔ | `Entry::TurnError` (distinct from toast) — **gap** |
| 17 | `notice` (slash-ack / unknown-command notice) | ✘ | ✔ | `Entry::Notice` — **gap** (can fold into chat variants) |

### Category 2 — input & pickers (modal overlays)

| # | Page | v1 | TUI | Source of truth |
|---|------|----|-----|-----------------|
| 18 | `slash-menu` | ✔ | ✔ | `slash.rs:19` BUILTINS autocomplete |
| 19 | `command-palette` | ✔ | ✔ | `app.rs:713` `PaletteScope::Root` (Ctrl+P) |
| 20 | `model-picker` | ✔ | ✔ | `DialogKind::Models` (`/models`) |
| 21 | `agent-picker` | ✔ | ✔ | `DialogKind::Agents` (`/agents`) |
| 22 | `skills-picker` | ✔ | ✔ | `DialogKind::Skills` (`/skills`) |
| 23 | `session-picker` | ✔ | ✔ | `DialogKind::Sessions` (`/sessions`) |
| 24 | `permission-prompt` | ✔ | ✔ | `permission.rs` (allow once/always/deny) |
| 25 | `variant-picker` | ✘ | ✔ | `DialogKind::Variants` (`/variant`) — **gap** |
| 26 | `theme-picker` | ✘ | ✔ | `DialogKind::Themes` (`/themes`) — **gap** |
| 27 | `subagent-picker` | ✘ | ✔ | `DialogKind::Subagents` (`/subagents`) — **gap** |
| 28 | `attachment-picker` | ✘ | ✔ | `DialogKind::Attachments` (`/attach`, `attachments.rs`) — **gap** |
| 29 | `fork-picker` | ✘ | ✔ | `DialogKind::Fork` (`/fork`, timeline-backed) — **gap** |
| 30 | `export-picker` | ✘ | ✔ | `DialogKind::Export` (`/export`, 4 formats) — **gap** |
| 31 | `file-mention` (`@` fuzzy picker) | ✘ | ✔ | `mention.rs` — **gap** |
| 32 | `queue-manager` (queued prompts) | ✘ | ✔ | `/queue`, `PaletteScope::QueuedPrompts`, `ViewKind::Queue` — **gap** |
| 33 | `external-editor` handoff (`/editor`) | ✘ | ✔ | `slash.rs:153` — optional (transient state) |

### Category 3 — views (full-screen `ViewKind` + gallery)

| # | Page | v1 | TUI | Source of truth |
|---|------|----|-----|-----------------|
| 34 | `status` | ✔ | ✔ | `ViewKind::Status` (`view.rs:689` status_body_lines) |
| 35 | `timeline` | ✔ | ✔ | `ViewKind::Timeline` (`view.rs:767`) |
| 36 | `mcp` | ✔ | ✔ | `ViewKind::Mcp` (`view.rs:873`) |
| 37 | `diff-viewer` | ✔ | ✔ | `ViewKind::Diff`, `app.rs:493` `DiffViewMode` |
| 38 | `help` | ✔ | ✔ | `ViewKind::Help` (`view.rs:1053` 2×2 grid) |
| 39 | `review` (code review findings view) | (as reviewer-output) | ✔ | `ViewKind::Review`, `view.rs:576` `review_body_lines`, `Severity` — the real `/review` view differs from the planned L3/L4 `reviewer-output`; worth its own screen |
| 40 | `theme-gallery` | ✘ | ✔ | `gallery.rs` (`/theme-gallery`, VAL-THEME-001) — **gap** |
| 41 | `sidebar` (7 tabs: Context/Files/LSP/MCP/Todo/Subagents/Queue) | ✘ | ✔ | `app.rs:213` `SidebarPanel` — **gap** (biggest single omission; the v1 set has no sidebar at all) |

### Category 4 — auth

| # | Page | v1 | TUI | Source of truth |
|---|------|----|-----|-----------------|
| 42 | `login` (auth method choice) | ✔ | ✔ | `auth_login.rs` (`cortex auth login`) |
| 43 | `connect-provider` (catalogue) | ✔ | ✔ | `connect.rs` `Step::Provider` |
| 44 | `connect-method` (API key vs OAuth pick) | ✘ | ✔ | `Step::Method` — **gap** |
| 45 | `connect-apikey` | ✔ | ✔ | `Step::ApiKey` |
| 46 | `connect-oauth` | ✔ | ✔ | `Step::OAuthWaiting` |
| 47 | `connect-connected` (success confirm) | ✘ | ✔ | `Step::Connected` — **gap** |
| 48 | `connect-disconnect` (confirm + done) | ✘ | ✔ | `Step::ConfirmDisconnect`/`Disconnected` — **gap** |

### Category 5 — states & feedback

| # | Page | v1 | TUI | Source of truth |
|---|------|----|-----|-----------------|
| 49 | `empty` (no sessions) | ✔ | ✔ | manual empty state |
| 50 | `error-network` | ✔ | ✔ | turn failure path |
| 51 | `error-context` | ✔ | ✔ | context-window exhaustion |
| 52 | `rate-limit` | ✔ | ✔ | provider 429 path |
| 53 | `interrupt` | ✔ | ✔ | Esc during generation |
| 54 | `toasts` | ✔ | ✔ | `tui-components/toast.rs` |
| 55 | `checkpoint-undo-redo` (`/undo`,`/redo`) | (partial in toasts) | ✔ | `checkpoint.rs` — **gap** as its own screen |
| 56 | `share` (`/share`,`/unshare` + attached-server) | ✘ | ✔ | `remote.rs`, `slash.rs:157-163` — **gap** |

### Category 6 — orchestration (planned, doc 08 frames; none in real TUI yet)

| # | Page | v1 | TUI | Source of truth |
|---|------|----|-----|-----------------|
| 57 | `mode-switch` | ✔ | ✘ | 08 §2.1 (5 modes + grant matrix + detail) |
| 58 | `workflow-launch` | ✔ | ✘ | 08 §2.2 (preflight + features.json graph) |
| 59 | `workflow-monitor` | ✔ | ✘ | 08 §2.3 (A–F ribbon + verdict→status + waves) |
| 60 | `team-board` | ✔ | ✘ | 08 §2.4 (6-column kanban + BLOCKED + contract) |
| 61 | `reviewer-output` | ✔ | ✘ | 08 §2.5 (L3 Code Reviewer + L4 QA Validator) |
| 62 | `verification-ladder` | ✔ | ✘ | 08 §2.6 (L0–L4 visible/masked) |
| 63 | `intent-scorer-report` | ✔ | ✘ | 08 §2.7 (D1–D5 + PASS rule + repo-readiness) |
| 64 | `concurrency-metrics` | ✔ | ✘ | 08 §2.8 (waves + merge-back + quarantine) |
| 65 | `tool-envelope` | ✔ | ✘ | 08 §2.9 (status/payload/truncated contract) |

**Page count: 65 distinct pages** (56 real-TUI-backed, 9 plan-only orchestration, 45 with v1 mockups).

---

## B. State variants per page (target 80–100 total screens)

**E** = essential, **O** = optional. Baseline: every page gets 1 base screen; variants add on top.

### session (17 pages → 17 E screens)
- `home` — base **E**; returning-user variant O
- `onboarding` — base (step 2 of 3) **E**; all-steps-done O
- `chat-turn` — base **E**; long-scroll multi-turn **E** (audit: scroll/truncation affordance)
- `streaming` — base **E** (Working band as single canonical status source)
- `reasoning` — expanded **E**; collapsed O
- `rich-response` — base **E**
- `tool-running` — base **E**
- `tool-success` — base **E**; expanded detail (`/details`, ctrl+o) O
- `tool-failed` — base with `r retry` + `y apply fix` **E**
- `tool-task` — one done + one running (unified time format) **E**; expanded findings O
- `diff-edit` — pending-apply **E**; applied/confirmed O
- `shell` — exit-0 + running pair **E**
- `blocked-command` — plan-mode deny banner **E** (new)
- `turn-error` — inline setup/turn failure **E** (new)
- `todo-list` — in-progress **E**; all-done O
- `compact` — base **E**
- `notice` — O (fold into slash-menu/chat variants)

### input & pickers (16 pages → 18 E screens)
- `slash-menu` — base **E**; filtered (`/th…`) O
- `command-palette` — root **E**; submenu scope (queued prompts) **E** (audit: chord merge)
- `model-picker` — base **E**; filtered **E** (audit: neutral selection accent, cost/speed hints)
- `agent-picker` — base **E**
- `skills-picker` — base with apply affordance **E**
- `session-picker` — base **E**
- `permission-prompt` — shell command **E**; file edit **E**; deny-with-reason flow O; always-allow-chosen confirmation O
- `variant-picker` — base **E** (new)
- `theme-picker` — base **E** (new)
- `subagent-picker` — base **E** (new)
- `attachment-picker` — base **E** (new); invalid-file error O
- `fork-picker` — base **E** (new)
- `export-picker` — base **E** (new, 4 formats + default badge)
- `file-mention` — `@` fuzzy list open **E** (new)
- `queue-manager` — with queued prompts **E** (new)
- `external-editor` — O (transient; skip if budget tight)

### views (8 pages → 12 E screens)
- `status` — base **E** (audit: split FIELD columns, ctx bar color ramp)
- `timeline` — base **E** (audit: dot legend); expanded turn O
- `mcp` — base w/ 1 failed row **E** (audit: actionable error)
- `diff-viewer` — file 2/6 **E**; hunk-staging view **E** (audit HIGH: stage/skip per hunk)
- `help` — base **E**
- `review` — real ViewKind::Review findings list **E** (new; distinct from reviewer-output)
- `theme-gallery` — dark **E**; light **E** (new)
- `sidebar` — Context **E**, LSP **E**, Subagents **E**; Files O, Todo O, Queue O, MCP O (new)

### auth (7 pages → 7 E screens)
- `login` — base **E** (conditional welcome text)
- `connect-provider` — base **E** (audit: active vs connected badge)
- `connect-method` — base **E** (new)
- `connect-apikey` — valid-key state **E**; invalid-format error O
- `connect-oauth` — waiting w/ live countdown **E**
- `connect-connected` — success **E** (new)
- `connect-disconnect` — confirm **E** (new); disconnected-done O

### states (8 pages → 8 E screens)
- `empty` — base **E**
- `error-network` — base **E**
- `error-context` — base **E** (audit: red/amber ramp on 99% bar)
- `rate-limit` — base w/ live countdown **E**
- `interrupt` — base **E**
- `toasts` — severity-ordered, BAD toast with recovery hotkey **E**
- `checkpoint-undo-redo` — undo confirmation **E** (new)
- `share` — share-created **E** (new); unshare O

### orchestration (9 pages → 10 E screens)
- `mode-switch` — base **E**; gate-refused promotion O
- `workflow-launch` — preflight-green **E**; preflight-red (MISSING, exit 1) **E**
- `workflow-monitor` — Phase D running **E**; failure-escalated (failed feature pinned) O
- `team-board` — base w/ BLOCKED emphasized **E**
- `reviewer-output` — CHANGES REQUESTED + BLOCKED **E**; all-pass approved O
- `verification-ladder` — base **E**
- `intent-scorer-report` — PASS 8/10 **E**; FAIL (gate dim = 0) O
- `concurrency-metrics` — HOLD w/ quarantine **E**
- `tool-envelope` — base **E**

### Final count
| Category | Pages | Essential screens | Optional screens |
|---|---|---|---|
| session | 17 | 17 | 8 |
| input & pickers | 16 | 18 | 5 |
| views | 8 | 12 | 5 |
| auth | 7 | 7 | 2 |
| states | 8 | 8 | 1 |
| orchestration | 9 | 10 | 4 |
| **Total** | **65** | **72** | **25** |

**Recommendation: 72 essential screens + 8–28 optionals → the mission lands at 80–100 total (suggested cut: 90).**
Priority optionals (first 10 to add on top of the 72): permission deny-reason flow, reasoning-collapsed, tool-success expanded detail, tool-task findings expanded, sidebar-files, sidebar-todo, mode-switch gate-refused, reviewer-output approved, intent-scorer FAIL, workflow-monitor failure-escalated.

---

## C. UX-AUDIT checklist (v2 must fix)

### Cross-cutting themes (apply to every v2 screen)
1. **[HIGH] Red overload** — reserve red for error/deletion only; selection highlight gets a neutral accent (worst offender: `model-picker` selected-row ctx in red).
2. **[HIGH] Color-only status** — every status dot pairs with a redundant glyph (✓ / ● / ○ / ✗); `todo-list` already does it: make it the standard.
3. **[HIGH] Redundant status bands** — one canonical status line; kill the Working-band + inline-line + footer triplication (`shell`, `streaming`, `tool-running`, `tool-task`).
4. **[HIGH] Inconsistent time format** — unify everywhere (`1m04s` on task line vs `64s` in band, `tool-task`).
5. **[HIGH] Static counters look broken** — countdowns must read as live (`connect-oauth` "expires in 15:00", `rate-limit` "retry in 42s").
6. **[MED] Duplicated state tables** — one authoritative table + expansion (`concurrency-metrics`, `intent-scorer-report`, `verification-ladder` all list the same items twice).
7. **[MED] Failure gets routine visual weight** — escalate with bar/tint/severity sort (`team-board` BLOCKED, `workflow-monitor` failed 1, `reviewer-output`, `toasts` BAD).
8. **[MED] Ambiguous apply/persist in toggle pickers** — explicit `enter apply` or "changes apply immediately" (`skills-picker` has no confirm key).
9. **[MED] Weak selection affordance** — add a highlight-bar background, not just bold + `›` (`agent-picker`, `model-picker`).
10. **[MED] No unified inline-color convention** — one rule for identifiers vs HTTP codes vs links (`chat-turn`, `reasoning`, `compact`).
11. **[HIGH] Every failure state carries a keyboard recovery affordance** — blocking errors do this well; the BAD toast must get a hotkey ("press s to retry settings save").

### Per-screen findings

**auth/**
- `connect-apikey`: [MED] clarify whether the Provider line is editable or locked (dim if locked) · [LOW] reword "paste insert key" → `paste · insert key from clipboard` · [LOW] state the post-save expectation ("back to chat once verified").
- `connect-oauth`: [HIGH] live countdown for code expiry · [MED] tie `o open browser` visually to the step-1 URL (`[o] open` inline) · [LOW] copy affordance for the one-time code.
- `connect-provider`: [HIGH] distinguish "active" vs merely "connected" (Anthropic is active with no badge while others show green connected) · [MED] non-color indicators (`✓ connected` / `○ not connected`) · [LOW] show `d disconnect` only on connected rows.
- `login`: [MED] conditional "Welcome back" (contradicts first launch) · [MED] group the 3 options (2 quick paths vs full catalogue, add separator) · [LOW] use top canvas for logo/version identity.

**input/**
- `agent-picker`: [MED] highlight bar on the selected row (cursor ≠ active) · [LOW] add a filter line (consistency with model-picker) · [LOW] show the effect of selection ("grants: read-only").
- `command-palette`: [HIGH] merge the cryptic 2nd column (`H / L / G / F / 8`) into the chord (`Ctrl+G H`) as one readable shortcut · [MED] submenu affordance (`›`/`▸`) on scoped rows · [LOW] disambiguate stash/recall/restore.
- `diff-viewer`: [HIGH] hunk-level stage/skip actions + "stage all / finish review" · [MED] staged counter ("staged 1/6") · [LOW] dedupe the `summary +9 −4` line.
- `help`: [MED] pointer to the full palette ("Ctrl+P for all 28 actions" — this is a curated cheat sheet) · [LOW] inline filter · [LOW] getting-started hint on first run.
- `mcp`: [MED] actionable error note (check `playwright` binary on PATH / copyable command, not raw spawn error) · [LOW] clarify what `enter details` reveals · [LOW] last-connection-attempt timestamp per row.
- `model-picker`: [HIGH] selected-row ctx must not be red (reads as error) · [MED] add cost/token or speed/quality hint per model · [LOW] group/label by provider as the list grows.
- `permission-prompt`: [MED] after Deny, confirm whether a "reason" field appears (else drop "and tell the agent why") · [LOW] highlight the risky flag (`--all`) · [LOW] scope reminder on "Allow always" (per-command vs per-session).
- `session-picker`: [MED] auto-select/badge the session matching the query ("yesterday") · [LOW] fixed columns for time and message count · [LOW] light row actions (delete/rename/pin) with footer hint.
- `skills-picker`: [MED] persistence clarity — add `enter apply` or "changes apply immediately" · [LOW] live counter ("5 of 6 enabled") · [LOW] distinguish loaded-this-session vs available-but-off.
- `slash-menu`: [LOW] default-select the first item (`/help`) · [LOW] section separators beyond ~10 items · [LOW] inline shortcuts for commands that have them.
- `status`: [MED] vertical separator between the two FIELD/VALUE pairs · [LOW] ctx bar color ramp (green→amber→red) near 100% · [LOW] tie `c to compact` to the context row.
- `timeline`: [MED] legend for actor-dot colors · [MED] expandable turns (`enter` → findings/diff) · [LOW] clarify `f fork` result ("→ new session from this turn").

**orchestration/**
- `concurrency-metrics`: [HIGH] real hotkey for the quarantine-reattach action, surfaced in the footer · [MED] visually link the quarantined `feat-api-routes` to its wave-3 conflict row · [LOW] compress the 2 prose blocks into a 2-line legend.
- `intent-scorer-report`: [MED] mark gated dimensions inline (lock glyph) in the score table · [MED] merge/fold duplicated D1–D5 between scores and evidence · [LOW] `n/a` instead of `—` in the GATE column.
- `mode-switch`: [MED] rename `● active` on the Agent row to `● selected`/`requested` (the switch has not happened yet) · [LOW] move `Δ = unlocked` next to the matrix header · [LOW] tint the 2 newly-granted rows.
- `reviewer-output`: [HIGH] aggregate the 4 findings into one actionable checklist near the gate line ("re-post with: Q3, Q4, Q8, Q9") · [MED] align `file:line` refs in a real column · [LOW] dim `a accept` while blocking findings exist.
- `team-board`: [MED] sort the task table by pipeline stage (mirror the flow ribbon) · [MED] emphasize BLOCKED (red bar/row; the failed dep must stand out from CLOSED rows) · [LOW] mark READY rows eligible for `c claim`.
- `tool-envelope`: [MED] make the top counters (`ok 41 · error 2 · refused 3`) filtering · [LOW] inline the "error = ran & failed · refused = never ran" contrast near the status column · [LOW] balance the 2 JSON block heights.
- `verification-ladder`: [MED] fold the duplicated L0–L4 lists into one authoritative table · [MED] non-red color for "masked" (conflates with "fail") · [LOW] make the L3 continuation explicit ("3 questions remaining → enter").
- `workflow-launch`: [MED] color graph nodes by wave (link wave calendar to graph) · [LOW] dim the `abort form: MISSING…` note (edge case on a confirm screen) · [LOW] highlight the editable levers (`N=3 parallelism`, `budget $2.00`).
- `workflow-monitor`: [MED] escalate the failed feature (`feat-audit-log`): red bar / pinned on top so `failed 1` has an anchor · [MED] highlight the current position in the D1–D8 loop (D4) · [LOW] default `f filter` to unverified.

**session/**
- `chat-turn`: [MED] establish the inline color convention (identifier vs HTTP status vs link) and apply everywhere · [LOW] raise body-text gray contrast slightly.
- `compact`: [MED] clarify the target of `ctrl+o expand` ("expand summary" vs "full transcript") · [LOW] subtle separator between recap bullets and the metrics line.
- `diff-edit`: [MED] disambiguate the input line — show a `[y/n/edit]` key prompt instead of a text field that also accepts typing · [LOW] show old→new inline in the rationale (`None → Some(1)`).
- `home`: [MED] dedupe the two hint lines (welcome panel vs footer overlap) · [LOW] faint rule between welcome block and first turn.
- `onboarding`: [LOW] leader dots toward the right-hand status labels at 120 cols · [LOW] distinguish the current step more strongly.
- `reasoning`: [MED] "more content" affordance (`▾ more`/fade) — the list is visibly truncated with no scroll indicator · [LOW] raise the Thinking-trace gray contrast.
- `rich-response`: [MED] reduce redundancy between the table Verdict column and the `[best fit]/[caution]/[avoid]` bullets · [LOW] copy affordance on the code block (`y copy`) · [LOW] verdict symbols (●/▲/■) for color-blind users.
- `shell`: [MED] dedupe interrupt hints (running line + band + footer = 3×) · [MED] glyph next to the status dots · [LOW] align `exit 0 · 0.2s` in a consistent status column.
- `streaming`: [HIGH] fold the redundant "streaming… press esc to interrupt" line into the Working band — one status line · [LOW] avoid esc/interrupt doubles between band and footer.
- `todo-list`: [MED] visual emphasis (bold/background) on the current ▶ line · [LOW] merge the generic "working through the plan…" band into the Working band.
- `tool-failed`: [MED] explicit "apply fix" shortcut (`y apply`) since Cortex proposes a concrete fix · [LOW] subtle red left border on the failed block.
- `tool-running`: [MED] consolidate metrics — task line owns "matches", band owns elapsed/scanned, no repeated file count · [LOW] light progress fraction when the total is known.
- `tool-success`: [LOW] make file:line hits visibly actionable ("↵ open") · [LOW] consistent labels/units across states (221 vs 128 files).
- `tool-task`: [HIGH] unify the time format (`1m04s` vs `64s`, same value) · [MED] make `6 findings` discoverable/expandable (`ctrl+o`) · [LOW] reduce redundancy between the "2 of 3 files" line and the band label.

**states/**
- `empty`: [LOW] merge the two redundant sub-text lines · [LOW] raise secondary-gray contrast.
- `error-context`: [MED] red/amber ramp on the near-full bar so "99%" reads urgent · [LOW] color the reclaimed "~150k tokens" (make `/compact` the obvious choice) · [LOW] drop the "full" label doubling "99%".
- `error-network`: [LOW] tint "attempts 3 of 3" (retries exhausted) rather than neutral metadata · [LOW] clarify enter (continue) vs `r` (resend).
- `interrupt`: [LOW] "retype" → "rephrase" (matches "ask a different way") · [LOW] visual seam between the partial answer and the interrupt block.
- `rate-limit`: [MED] live countdown for "retry in 42s" · [LOW] visually demote `retry now` (screen self-heals; retrying can worsen).
- `toasts`: [HIGH] keyboard recovery affordance on the BAD toast ("press s to retry settings save") · [MED] elevate/persist the error toast while OK/INFO auto-dismiss · [MED] reduce success/failure whiplash ("Checkpoint saved" next to "Could not save settings" — move the failure near its causing action) · [LOW] order toasts by severity (BAD on top), not chronologically.

---

## D. Gaps — pages with NO v1 mockup

Real-TUI surfaces missing from the v1 gallery (20 pages, all shipping code today):

1. **`sidebar` (7 tabs: Context/Files/LSP/MCP/Todo/Subagents/Queue)** — the entire sidebar system (`app.rs:213`) has zero mockups; the biggest single omission.
2. **`variant-picker`** (`/variant`, `DialogKind::Variants`)
3. **`theme-picker`** (`/themes`, `DialogKind::Themes`)
4. **`subagent-picker`** (`/subagents`, `DialogKind::Subagents`)
5. **`attachment-picker`** (`/attach`, `DialogKind::Attachments`)
6. **`fork-picker`** (`/fork`, timeline-backed `DialogKind::Fork`)
7. **`export-picker`** (`/export`, 4 formats, `DialogKind::Export`)
8. **`file-mention`** (`@` fuzzy file picker, `mention.rs`)
9. **`queue-manager`** (`/queue`, `ViewKind::Queue`, `PaletteScope::QueuedPrompts`)
10. **`theme-gallery`** (`/theme-gallery`, `gallery.rs`, dark + light)
11. **`review`** — the real `ViewKind::Review` findings view (v1 only has the planned L3/L4 `reviewer-output`, a different surface)
12. **`blocked-command`** — plan-mode hard-deny banner (`Entry::Blocked`; only glimpsed inside the `shell` mockup)
13. **`turn-error`** — inline ✦ Cortex ✗ turn/setup failure block (`Entry::TurnError`)
14. **`connect-method`** — auth-method pick step (`connect.rs` `Step::Method`)
15. **`connect-connected`** — connect success confirmation (`Step::Connected`)
16. **`connect-disconnect`** — confirm-before-remove + disconnected note (`Step::ConfirmDisconnect`/`Disconnected`)
17. **`checkpoint-undo-redo`** — `/undo` `/redo` surfaces (`checkpoint.rs`; only a checkpoint block inside `toasts`)
18. **`share`** — `/share` `/unshare` local share references (`remote.rs`)
19. **`external-editor`** — `/editor` composer handoff (optional; transient)
20. **`notice`** — system notice transcript entry (optional; can fold into chat variants)

Plan-only **state gaps** (existing v1 pages missing key variants): `workflow-launch` preflight-red (MISSING → exit 1), `mode-switch` gate-refused promotion, `reviewer-output` approved, `intent-scorer-report` FAIL, `permission-prompt` file-edit + deny-reason, `command-palette` submenu scope, `diff-viewer` hunk staging, `model-picker` filtered, `chat-turn` long-scroll.
