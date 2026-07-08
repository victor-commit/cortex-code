# 08. CLI / TUI UI/UX: the operator surface over the new subsystems

Give every new subsystem shipped by docs 01–07 a concrete terminal surface. The
engine, Workflow runner, Team board, Reviewers, Verification ladder, Intent Scorer,
concurrency accounting, and the tool response contract all now exist as core
machinery; this doc specifies **how an operator sees and drives them** from the CLI
and the TUI. It is anchored to nine real rendered terminal frames (captured by a
throwaway `ratatui` harness under `tools-throwaway/mockup-harness/`, mirrored into
`plans/assets/mockups/`), each pasted verbatim below and annotated for layout,
interaction model, keybindings, and states.

This is a UI/UX design plan. No product Rust is written by this task; the only file
it creates is this document. Nothing under `crates/` is touched.

---

## 0. Scope & Contracts

This doc covers the CLI/TUI surface — and only the surface — for every new subsystem:

- **Mode switch** — picking Plan / Edit / Agent / Chat / Headless and seeing the
  per-mode tool grant (the `mode-switch` frame).
- **Workflow launch** — the `run_workflow` preflight (Step 0) and the feature
  dependency graph read from `features.json` (the `workflow-launch` frame).
- **Workflow monitor** — the Phase A–F lifecycle, per-feature verdict→status board,
  and wave scheduling while `run_workflow` blocks (the `workflow-monitor` frame).
- **Team board** — the shared Task Board columns, item contract, and Coordinator
  assignments (the `team-board` frame).
- **Reviewer output** — the Code Reviewer (L3 intent audit) and QA Validator (L4
  process check) verdicts side by side (the `reviewer-output` frame).
- **Verification ladder** — L0–L4 with the visible-vs-held-out split (the
  `verification-ladder` frame).
- **Intent-scorer report** — the five-dimension effectiveness score plus the
  Repo Readiness signal (the `intent-scorer-report` frame).
- **Concurrency metrics** — per-wave conflict rate, merge-back, and quarantine read
  from `parallel-metrics.jsonl` (the `concurrency-metrics` frame).
- **Tool envelope** — the fixed 3-field response contract shared by all 11 tools
  (the `tool-envelope` frame).

**Locked contracts inherited** (this doc renders them, it does not redefine them):

- **Modes and the mode gate** come from `plans/01-*.md` (Plan / Edit / Agent / Chat /
  Headless; the tool grant per mode). This doc draws the picker; the gate itself is
  doc 01's.
- **The seven-step loop** `sense → decide → gate → act → check → remind → record` and
  the trajectory + Session Log record substrate come from `plans/02-engine-7step-loop.md`.
  The monitor and reviewer surfaces read what `record` writes; they add no new store.
- **Workflow / workers / `run_workflow` / waves / merge-back / quarantine** come from
  `plans/03-*.md`.
- **Verification levels 0–4, Code Reviewer, QA Validator** come from `plans/05-*.md`.
- **Subagents / Skills** come from `plans/06-subagents-skills.md`.
- **Intent Scorer / Repo Readiness report** come from `plans/07-*.md`.
- **The tool response contract** (`status / payload / truncated`, 11 tools) is the
  tool-layer contract; this doc renders it, it does not author it.

Naming is drawn only from `/cortex-code-prototype/docs/rename/naming-map.md` and
`/cortex-code-prototype/GLOSSARY.md`; no term is coined here. Every surface named —
Plan/Edit/Agent/Chat/Headless Mode, Workflow, Team, Task Board, Coordinator,
Orchestrator, worker, Reviewers (Code Reviewer, QA Validator), Verification, Intent
Scorer, Repo Readiness report, Subagents, Skills, `run_workflow`, wave, conflict
metric, `parallel-metrics.jsonl`, merge-back, quarantine ref, integration branch,
intent reducer — is a professional-generic register term from those authorities. The
default UI convention is `design-guidelines.md`: reuse the project's existing
components, tokens, and layout patterns; do not introduce a new palette, glyph set,
or widget library when one already exists. The build procedure is the TUI Application
Playbook (vertical interaction slices, render/state seam); the verification half is
the tui-testing Playbook.

---

## 1. Current State (file:line)

The CLI/TUI today is a **single-agent chat client**. It has personas, modes,
model/session resume, a slash-command menu, and a component kit — but **no surface for
any of the eight new subsystems**. Anchors below are all verified with `rg`/`sed`
against `/projects/cortex-code/crates/`.

### 1.1 What exists

- **Agent personas (`--agent`).** `crates/engine/src/agent.rs:11-13` defines the
  `Agent` enum selectable with `cortex run --agent <name>`; `agent.rs:26-34` fixes
  `DEFAULT = Build`, `ALL = [Build, Plan, General, Explore]`, and the non-primary
  `SUBAGENTS = [Plan, General, Explore]`; `agent.rs:41-55` gives each its canonical
  lowercase name and one-line description (the values a discovery UI shows).
- **CLI arg parsing (`crates/cli/`).** `crates/cli/src/cli.rs:36` is the top-level
  `enum Command` (variants at `cli.rs:47-336`: `Run`, `Tui`, `Serve`, `Acp`, `Config`,
  `Session`, `Export`, `Import`, `Stats`, `Search`, `Replay`, `Agent`, `Auth`,
  `Models`, `Generate`, `Plugin`, `Provider`, `Project`, `Worktree`, `Account`, `Env`,
  `Net`, `Undo`, `Redo`, `Snapshot`, `Mcp`, `Lsp`, `Formatter`, `Attach`, …). The
  headless turn surface is `struct RunArgs` at `cli.rs:1178`: `--model` (`cli.rs:1203`),
  `--agent` (`cli.rs:1207`), `--mode` (`cli.rs:1211`), `--session` (`cli.rs:1215`),
  `-c/--continue` (`cli.rs:1219`), plus `--format`/`--jsonl` (`cli.rs:1187`/`1191`).
  Argument resolution and dispatch live in `crates/cli/src/run.rs:72` (`run`), with
  the persona/mode precedence chain at `run.rs:140-173`.
- **Custom modes (`cortex run --mode`).** `crates/cli/src/custom_modes.rs:1-3`
  discovers markdown "mode overlays"; `custom_modes.rs:40-70` is `ActiveMode` with
  `base_agent`, `prompt`, and `permissions` accessors. Modes today are permission /
  persona overlays, not the five named operating modes of doc 01.
- **Session model + resume (`crates/db/src/session.rs`).** The `model` field is
  `session.rs:20-23` (set via the TUI `/models` picker, honored on a headless
  `--session <id>` resume); `session.rs:79` documents `--session` as a durable
  get-or-create id; `session.rs:98-106` documents `--continue` attaching to the most
  recently active session.
- **TUI slash-command surface.** `crates/tui/src/slash.rs:19` is the `BUILTINS`
  registry (single source of truth for what commands exist). Relevant present
  commands: `/models`,`/model` (`slash.rs:33-38`), `/agents` (`slash.rs:45`),
  `/skills` (`slash.rs:49`), `/subagents` (`slash.rs:81`), `/variant` (`slash.rs:53`),
  `/review` "Run a code review" (`slash.rs:137-138`), `/diff` (`slash.rs:141`),
  `/status` (`slash.rs:69`), `/timeline` (`slash.rs:73`), `/fork` (`slash.rs:77`),
  `/compact` (`slash.rs:145`), and the `sidebar-*` panels (`slash.rs:89-118`).
- **TUI keybindings.** `crates/tui/src/keybind.rs:16` is `KeybindAction`:
  `CommandPalette` (`keybind.rs:34`), `ModelCycleNext` (`keybind.rs:40`),
  `AgentCycleNext` (`keybind.rs:42`), `VariantCycleNext` (`keybind.rs:44`), and the
  palette openers `PaletteModels`/`PaletteSkills`/`PaletteVariants`/`PaletteSubagents`/
  `PaletteQueuedPrompts` (`keybind.rs:62-70`).
- **TUI app + component kit.** `crates/tui/src/app.rs:20` pulls `engine::{Agent,
  RunnerEvent, …}`; `app.rs:43-45` already renders a plan-mode "blocked" banner
  ("shell is disabled while planning"), the one existing mode-aware affordance. The
  reusable widgets the new frames map onto live in `crates/tui-components/src/`:
  `panel.rs`, `card.rs`, `page_layout.rs`, `selection_list.rs`, `list.rs`,
  `borders.rs`, `key_hints.rs`, `toast.rs`, `spinner.rs`, `color_scheme.rs`.

### 1.2 What is ABSENT

Verified: `rg -in "workflow|coordinator|task.?board|verification.?ladder|
intent.?scor|concurrency.?metric|tool.?envelope"` across `crates/cli/src` and
`crates/tui/src` returns **no matches**. Concretely:

- **Mode switcher UI — ABSENT.** There is no picker for the five named modes and no
  per-mode tool-grant table. `--mode` (`cli.rs:1211`) and custom modes
  (`custom_modes.rs`) are overlays, not the Plan/Edit/Agent/Chat/Headless switch of
  doc 01. No `/mode` slash command, no `ModeCycle*` keybind.
- **Workflow launch UI — ABSENT.** No `Workflow` variant in `enum Command`
  (`cli.rs:36`), no `run_workflow` CLI/TUI entry, no preflight or `features.json`
  graph renderer.
- **Workflow monitor UI — ABSENT.** No Phase A–F lifecycle view, no verdict→status
  feature board, no wave scheduler view.
- **Team board UI — ABSENT.** No Task Board columns, no item-contract panel, no
  Coordinator/assignments surface.
- **Reviewer output UI — ABSENT.** `/review` (`slash.rs:137-138`) names a code review
  but there is no Code-Reviewer/QA-Validator verdict view; the L3/L4 split is unrendered.
- **Verification ladder UI — ABSENT.** No L0–L4 visualization, no visible-vs-held-out
  masking.
- **Intent-scorer report UI — ABSENT.** No dimension/score table, no Repo Readiness
  panel.
- **Concurrency metrics UI — ABSENT.** No `parallel-metrics.jsonl` reader, no per-wave
  conflict-rate view, no merge-back/quarantine surface.
- **Tool envelope UI — ABSENT.** The `status/payload/truncated` contract is not
  rendered anywhere in the tool cards (`crates/tui/src/tool_card.rs` shows results but
  not the fixed-order envelope contract).

Net: the client has the frame, component kit, mode/persona plumbing, and event stream
to host these surfaces, but every one of the eight subsystem views is greenfield.

---

## 2. Target UI/UX Design

Nine real rendered frames follow. Each is captured terminal output at 120×40 from the
throwaway `ratatui` harness and is pasted **verbatim** inside a fence, then annotated.
All nine share the doc 01 chrome: a title bar (`╭ Cortex Code · <view> … MOCKUP · stub
data ╮`), a context strip (`session cx-0001   workspace /work/acme-api   model
cortex-large`), a body of bordered panels, and a footer hint line ending `· q quit`.
That chrome is the existing `page_layout.rs` + `panel.rs` + `key_hints.rs`
composition; the frames reuse it rather than inventing new chrome.

### 2.1 Mode Switch

```
╭ Cortex Code · Mode Switch ─────────────────────────────────────────────────────────────────────── MOCKUP · stub data ╮
│session cx-0001   workspace /work/acme-api   model cortex-large                                                       │
│──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────│
│╭ Modes ───────────────────────────────╮╭ Tool grants by mode ───────────────────────────────────────────────────────╮│
││  the current mode decides your tools ││tool            Plan    Edit   Agent                                        ││
││                                      ││──────────────────────────────────────────────                              ││
││  Plan Mode                           ││find            yes     yes     yes                                         ││
││    [read-only]                       ││grep_code       yes     yes     yes                                         ││
││                                      ││read_span       yes     yes     yes                                         ││
││  Edit Mode                           ││list_tree       yes     yes     yes                                         ││
││    [read + write + run]              ││run           read-only gated   gated                                       ││
││                                      ││edit_span        —      yes     yes                                         ││
││▶ Agent Mode                          ││write_file       —      yes     yes                                         ││
││    [read + write + run  + subagents] ││web_lookup     gated   gated   gated                                        ││
││                                      ││ask             yes     yes     yes                                         ││
││  Chat Mode                           ││plan_note       yes      —      yes                                         ││
││    [interactive body]                ││spawn_subagent   —       —      yes                                         ││
││                                      ││                                                                            ││
││  Headless                            ││                                                                            ││
││    [non-interactive single-shot]     │╰────────────────────────────────────────────────────────────────────────────╯│
││                                      │╭ Agent Mode · detail ───────────────────────────────────────────────────────╮│
││  active → Agent Mode                 ││purpose  long-horizon autonomous execution toward a goal, across many steps.││
││                                      ││grants   everything Edit Mode grants [read + write + run]                   ││
││                                      ││         plus [+ spawn scoped Subagents]                                    ││
││                                      ││caution  runs under the tightest discipline of any mode.                    ││
││                                      ││headless terminal status is one of [done] [partial] [blocked]               ││
││                                      ││                                                                            ││
││                                      ││                                                                            ││
││                                      ││                                                                            ││
││                                      ││                                                                            ││
││                                      ││                                                                            ││
││                                      ││                                                                            ││
││                                      ││                                                                            ││
││                                      ││                                                                            ││
││                                      ││                                                                            ││
││                                      ││                                                                            ││
│╰──────────────────────────────────────╯╰────────────────────────────────────────────────────────────────────────────╯│
│──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────│
│↑↓ pick mode · enter request promotion · the runtime grants the mode, not the model · q quit                          │
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
```

- **Layout.** Three panels: a left mode list, a right upper tool-grant matrix, a right
  lower detail card for the highlighted mode. The mode list carries all five doc 01
  modes in canonical order (Plan, Edit, Agent, Chat, Headless), each with a one-line
  capability tag. The matrix columns are the three tool-bearing interactive modes
  (Plan/Edit/Agent); Chat and Headless are bodies, not tool tiers, so they get a tag,
  not a column. Rows are the 11 tools in envelope order (§2.9).
- **Interaction model.** A single-select list. The cursor `▶` marks the highlighted
  row; `active → Agent Mode` states the currently effective mode (distinct from the
  highlight). Highlighting a mode repaints the detail card below; pressing `enter`
  issues a **mode-promotion request** (not a silent switch) — the runtime, not the
  model, grants the mode, so a promotion is gated exactly like doc 01's mode gate.
- **Keybindings.** `↑`/`↓` move the highlight; `enter` requests promotion to the
  highlighted mode; `q` quits. This maps to a new `ModeSwitcher` view driven by the
  `keybind.rs` action set; it parallels the existing `AgentCycleNext`
  (`keybind.rs:42`) but for the operating mode rather than the persona.
- **States.** highlighted (≠ active); active; requested-but-gate-pending;
  gate-refused (reuses the `app.rs:43-45` blocked-banner treatment). The grant cells
  render four states: `yes`, `—` (ungranted), `gated` (ask-before-run), and
  `read-only` (the `run` cell under Plan).

### 2.2 Workflow Launch (`run_workflow`)

```
╭ Cortex Code · Workflow Launch · run_workflow ──────────────────────────────────────────────────── MOCKUP · stub data ╮
│session cx-0001   workspace /work/acme-api   model cortex-large                                                       │
│──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────│
│╭ Preflight (Step 0) ────────────────────────────────────╮╭ Feature dependency graph · features.json ────────────────╮│
││  author-time artifacts (Phases B/C) must all exist     ││  node = one feature   edge = a real precondition         ││
││  [ok] architecture.md                                  ││                                                          ││
││  [ok] acceptance-contract.md                           ││  m1-auth                                                 ││
││  [ok] acceptance-state.json                            ││   feat-db-schema  → ACC-AUTH-002                         ││
││  [ok] features.json                                    ││        │                                                 ││
││  [ok] boundaries.md                                    ││        ├──┐                                              ││
││  [ok] services.yaml                                    ││        │  feat-auth-login  → ACC-AUTH-001                ││
││  [ok] init.sh                                          ││        │        │   pre: [feat-db-schema]                ││
││  [ok] skills/                                          ││  m2-api                                                  ││
││                                                        ││        └─ feat-api-routes  → ACC-API-010                 ││
││  runner-seeded surfaces (created by Step 0)            ││                 │   pre: [feat-db-schema]                ││
││  [ok] workflow/wf-42/integration                       ││                 └──┐                                     ││
││  [ok] progress-log.jsonl                               ││                     feat-web-ui  → ACC-WEB-003           ││
││  [ok] intents/                                         ││             pre: [feat-auth-login, feat-api-routes]      ││
││                                                        ││                                                          ││
││──────────────────────────────────────────────────────  ││  coverage: each ACC-<AREA>-NNN has exactly one fulfills  ││
││  preflight OK — spawning worker #1                     ││  wave 1 (parallel-safe): [feat-db-schema]                ││
││  abort form: MISSING <artifact>; exit 1                ││                                                          ││
││                                                        ││                                                          ││
││                                                        ││                                                          ││
││                                                        ││                                                          ││
││                                                        ││                                                          ││
││                                                        ││                                                          ││
││                                                        ││                                                          ││
││                                                        ││                                                          ││
││                                                        ││                                                          ││
││                                                        ││                                                          ││
││                                                        ││                                                          ││
││                                                        ││                                                          ││
││                                                        ││                                                          ││
││                                                        ││                                                          ││
││                                                        ││                                                          ││
│╰────────────────────────────────────────────────────────╯╰──────────────────────────────────────────────────────────╯│
│──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────│
│a missing artifact aborts here · no worker is spawned · workflowParallelism N=1 · q quit                              │
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
```

- **Layout.** Two panels. Left: the Step 0 preflight checklist, split into author-time
  artifacts (Phases B/C) and runner-seeded surfaces, with a divider and the launch
  verdict. Right: the `features.json` dependency graph drawn as an ASCII tree, grouped
  by milestone (`m1-auth`, `m2-api`), each node showing the feature id, its fulfilled
  acceptance id (`ACC-<AREA>-NNN`), and its `pre:` precondition list.
- **Interaction model.** A gate screen, not a free editor. It renders the pre-launch
  state and blocks: if any artifact is missing, it prints `MISSING <artifact>` and
  exits 1 — no worker spawns. On a full green it shows `preflight OK — spawning
  worker #1`. `wave 1 (parallel-safe)` is computed from the graph roots.
- **Keybindings.** Read-only launch view: `↑`/`↓` scroll the graph, `enter` confirms
  launch when preflight is green, `q` quits. The footer states the invariant: a
  missing artifact aborts here; `workflowParallelism N=1` is the default fan-out.
- **States.** preflight-green (launch enabled); preflight-red (one `[ok]` becomes
  `MISSING`, launch disabled, exit-1 abort form shown); post-launch (hands off to the
  Workflow Monitor, §2.3).

### 2.3 Workflow Monitor

```
╭ Cortex Code · Workflow Monitor · Phase D runtime loop ─────────────────────────────────────────── MOCKUP · stub data ╮
│session cx-0001   workspace /work/acme-api   model cortex-large                                                       │
│──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────│
│╭ Lifecycle phases ──────────────────────────────────────────────────────────────────────────────────────────────────╮│
││  ● A Planning  ──  ● B Worker design  ──  ● C Artifact creation                                                    ││
││  ◉ D Runtime loop  ──  ○ E Milestone validation  ──  ○ F End gate                                                  ││
││  now: Phase D — run_workflow is blocking; the runner drives features.json to finish                                ││
││  events: D1 Spawn · D2 Worktree · D4 Work · D6 end_feature · D7 Reduce · D8 Merge-back                             ││
│╰────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯│
│╭ Features · worker verdict → queue status ──────────────────────────────────────────────────────────────────────────╮│
││  feature           milestone verdict    status     fulfills                                                        ││
││  ──────────────────────────────────────────────────────────────────                                                ││
││  feat-db-schema    m1-auth   verified   completed  ACC-AUTH-002                                                    ││
││  feat-auth-login   m1-auth   verified   completed  ACC-AUTH-001                                                    ││
││  feat-api-routes   m2-api    partial    partial    ACC-API-010                                                     ││
││  feat-web-ui       m2-api    —          pending    ACC-WEB-003                                                     ││
││  feat-audit-log    m2-api    failed     failure    ACC-AUD-004                                                     ││
││                                                                                                                    ││
││  legend  verdict: [verified][partial][blocked][failed]   queue: completed/partial/failure/cancelled                ││
││                                                                                                                    ││
││                                                                                                                    ││
││                                                                                                                    ││
││                                                                                                                    ││
││                                                                                                                    ││
││                                                                                                                    ││
││                                                                                                                    ││
││                                                                                                                    ││
││                                                                                                                    ││
││                                                                                                                    ││
│╰────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯│
│╭ Wave scheduling · parallel-metrics.jsonl ──────────────────────────────────────────────────────────────────────────╮│
││  wave 2   pairs 3   intersecting 0   rate 0.0%   threshold 15%                                                     ││
││  conflict rate  ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  below threshold → keep expanding parallelism                       ││
││  worker  ws-3f9a2c on branch workflow/wf-42/feat-api-routes                                                        ││
││  end gate: every acceptance assertion passed [2 / 4 passed]  + README present                                      ││
││                                                                                                                    ││
│╰────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯│
│──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────│
│phases A-F · verdict→status two-level model · conflict rate vs 15% · q quit                                           │
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
```

- **Layout.** Three stacked panels. Top: the A–F lifecycle ribbon with per-phase
  glyphs and a `now:` line plus the D-phase event legend (`D1…D8`). Middle: the
  feature table — the doc 03 **two-level model** where a worker `verdict` (verified /
  partial / blocked / failed) is distinct from the queue `status`
  (completed/partial/failure/cancelled), joined to each feature's `fulfills`
  acceptance id. Bottom: live wave scheduling read from `parallel-metrics.jsonl`, with
  a conflict-rate bar, the active worker/branch, and the end-gate tally.
- **Interaction model.** A live dashboard that repaints from the `record` trajectory
  and `progress-log.jsonl` while `run_workflow` blocks. It is read-only: the operator
  observes; the runner drives.
- **Keybindings.** `↑`/`↓` select a feature row (to drill into its worker), `q` quits.
  No mutation keys — the core moves work, not the operator.
- **States.** Phase glyphs: `●` done (A/B/C), `◉` active (D), `○` pending (E/F). Per
  feature: the five verdict/status rows shown, including a `—`/`pending` (unstarted)
  and a `failed`/`failure` row. Conflict bar: below-threshold (widen) vs at/over
  (hold). End gate: `[2 / 4 passed]` partial vs all-passed.

### 2.4 Team Board

```
╭ Cortex Code · Task Board · Team ───────────────────────────────────────────────────────────────── MOCKUP · stub data ╮
│session cx-0001   workspace /work/acme-api   model cortex-large                                                       │
│──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────│
│╭ OPEN ────────────╮╭ READY ──────────╮╭ CLAIMED ─────────╮╭ ACTIVE ──────────╮╭ SUBMITTED ──────╮╭ CLOSED ──────────╮│
││T-107             ││T-104            ││T-103             ││T-102             ││T-101            ││T-100             ││
││                  ││T-105            ││owner set         ││owner set         ││owner set        ││T-099             ││
││                  ││                 ││                  ││                  ││                 ││                  ││
││                  ││                 ││                  ││                  ││                 ││                  ││
││                  ││                 ││                  ││                  ││                 ││                  ││
││                  ││                 ││                  ││                  ││                 ││                  ││
││                  ││                 ││                  ││                  ││                 ││                  ││
││                  ││                 ││                  ││                  ││                 ││                  ││
││                  ││                 ││                  ││                  ││                 ││                  ││
│╰──────────────────╯╰─────────────────╯╰──────────────────╯╰──────────────────╯╰─────────────────╯╰──────────────────╯│
│╭ BLOCKED ───────────────────────────────────────────────────────────────────────────────────────────────────────────╮│
││  T-106  reason: failed dependency   (also: missing contract field · external blocker)                              ││
│╰────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯│
│╭ Item T-102 · contract ────────────────────────────────────╮╭ Coordinator · assignments ────────────────────────────╮│
││  id                T-102                                  ││  posts contracts · workers claim · core moves         ││
││  state             ACTIVE                                 ││                                                       ││
││  task              add rate-limit middleware to /login    ││  post → claim → submit → accept                       ││
││  expected_artifact 429 on 6th attempt; test green         ││                                                       ││
││  tool_whitelist    read_span, edit_span, run              ││  [single ownership]  one per CLAIMED/ACTIVE           ││
││  forbidden         crates/db/**, prod config              ││  [no self-closing]  never closes own item             ││
││  context_slice     crates/server/src/auth/*               ││  [reconcile]  vs expected → re-post                   ││
││  depends_on        T-100 (CLOSED)                         ││                                                       ││
││  owner             ws-77b1e0                              ││  assignments   ws-3f9a2c→T-103   ws-77b1e0→T-102      ││
││  result            status + artifact ref + error          ││                ws-2c4d10→T-101 (reconciling)          ││
││                                                           ││                                                       ││
││                                                           ││                                                       ││
││                                                           ││                                                       ││
││                                                           ││                                                       ││
││                                                           ││                                                       ││
││                                                           ││                                                       ││
││                                                           ││                                                       ││
││                                                           ││                                                       ││
│╰───────────────────────────────────────────────────────────╯╰───────────────────────────────────────────────────────╯│
│──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────│
│the core moves items · claim only READY · no self-closing · q quit                                                    │
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
```

- **Layout.** A six-column kanban (`OPEN · READY · CLAIMED · ACTIVE · SUBMITTED ·
  CLOSED`) across the top, a full-width `BLOCKED` lane beneath it, and two detail
  panels below: the selected item's contract and the Coordinator/assignments panel.
  The contract panel renders the full Task Board item schema: `id`, `state`, `task`,
  `expected_artifact`, `tool_whitelist`, `forbidden`, `context_slice`, `depends_on`,
  `owner`, `result`.
- **Interaction model.** The operator navigates; **the core moves items**. The
  Coordinator posts contracts, workers claim `READY` items, and the core transitions
  states — the operator does not drag cards. The Coordinator panel states the three
  invariants: single ownership (one per CLAIMED/ACTIVE), no self-closing (never closes
  its own item), and reconcile-vs-expected (re-post on mismatch).
- **Keybindings.** `←`/`→` move between columns, `↑`/`↓` within a column, `enter`
  opens the item contract in the detail panel, `q` quits. Claiming is not an operator
  action (workers claim); the board is an inspection surface.
- **States.** Column membership is the item lifecycle (`OPEN → READY → CLAIMED →
  ACTIVE → SUBMITTED → CLOSED`), plus the out-of-flow `BLOCKED` lane carrying a
  `reason:` (failed dependency / missing contract field / external blocker). An
  assignment can be `(reconciling)`.

### 2.5 Reviewer Output (L3 + L4)

```
╭ Cortex Code · Reviewer output · L3 + L4 ───────────────────────────────────────────────────────── MOCKUP · stub data ╮
│session cx-0001   workspace /work/acme-api   model cortex-large                                                       │
│──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────│
│╭ Code Reviewer · L3 intent audit ───────────────────────────────╮╭ QA Validator · L4 process check ─────────────────╮│
││  verdict  [CHANGES REQUESTED]   diff 6 files / +214 -37        ││  verdict  [BLOCKED]                              ││
││                                                                ││  motto    “Un-run is not done.”                  ││
││  ✓ Q1 Scope fidelity  pass                                     ││                                                  ││
││      changes map to the requested task; nothing extra          ││  ✓ tests were executed (not just written)        ││
││  ✓ Q2 Correctness  pass                                        ││      cargo test  42 passed                       ││
││      logic matches acceptance ACC-API-010                      ││  ✓ real surface exercised                        ││
││  ✗ Q3 Dead weight  fail                                        ││      curl :8080/login  429 on 6th                ││
││      unused helper build_stub() left in server/util.rs:88      ││  ✗ evidence artifact captured                    ││
││  ✗ Q4 Swallowed failure  fail                                  ││      run-log.txt missing for feat-audit-log      ││
││      catch{} drops error in routes.rs:142 — masks 500          ││  ✗ regression suite re-run after fix             ││
││  ✓ Q5 Honest report  pass                                      ││                                                  ││
││      summary claims match the actual diff                      ││  outcome  pass / fail / blocked  ← here          ││
││                                                                ││  reason   cannot confirm without evidence        ││
││  gate: any FAIL → re-post to worker (no merge)                 ││                                                  ││
││                                                                ││                                                  ││
││                                                                ││                                                  ││
││                                                                ││                                                  ││
││                                                                ││                                                  ││
││                                                                ││                                                  ││
││                                                                ││                                                  ││
││                                                                ││                                                  ││
││                                                                ││                                                  ││
││                                                                ││                                                  ││
││                                                                ││                                                  ││
││                                                                ││                                                  ││
││                                                                ││                                                  ││
││                                                                ││                                                  ││
││                                                                ││                                                  ││
││                                                                ││                                                  ││
││                                                                ││                                                  ││
││                                                                ││                                                  ││
││                                                                ││                                                  ││
│╰────────────────────────────────────────────────────────────────╯╰──────────────────────────────────────────────────╯│
│──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────│
│reviewers read the diff, not the summary · un-run is not done · q quit                                                │
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
```

- **Layout.** Two side-by-side reviewer panels. Left: the **Code Reviewer** L3 intent
  audit with its top-line verdict + diff stat and the five checks Q1–Q5 (Scope
  fidelity, Correctness, Dead weight, Swallowed failure, Honest report), each a
  pass/fail with a one-line justification citing `file:line`. Right: the **QA
  Validator** L4 process check with its verdict, the "Un-run is not done." motto, and
  its four process gates (tests executed, real surface exercised, evidence artifact
  captured, regression re-run).
- **Interaction model.** A read-only verdict view rendered from the reviewer output
  the core produces after a worker submits. It states the gate rule: any `FAIL` →
  re-post to worker, no merge. The two panels are independent verdicts; both must pass
  to advance.
- **Keybindings.** `tab` toggles focus between the two reviewer panels, `↑`/`↓`
  scroll within the focused panel (to jump to a cited `file:line`), `q` quits.
- **States.** Verdict per panel: `[CHANGES REQUESTED]` / `[BLOCKED]` / (implied)
  approved. Per-check: `✓ pass` / `✗ fail`. QA outcome tri-state: `pass / fail /
  blocked` (here `blocked`, "cannot confirm without evidence").

### 2.6 Verification Ladder (L0–L4)

```
╭ Cortex Code · Verification ladder · L0–L4 ─────────────────────────────────────────────────────── MOCKUP · stub data ╮
│session cx-0001   workspace /work/acme-api   model cortex-large                                                       │
│──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────│
│╭ Levels · visible vs held-out ──────────────────────────────────────────────────────────────────────────────────────╮│
││  the worker can see L0/L1 to self-correct; L2–L4 stay held-out so scores are earned, not gamed                     ││
││                                                                                                                    ││
││  ◉ L0 Deterministic gates   [visible]                                                                              ││
││       spec  fmt · lint · type-check · build                                                                        ││
││       note  compiler & formatter — pass/fail is mechanical                                                         ││
││  ◉ L1 Agent-visible tests   [visible]                                                                              ││
││       spec  unit + integration the worker may read and run                                                         ││
││       note  RED → GREEN loop feeds back into the runtime                                                           ││
││  ■ L2 Held-out checks       [masked]                                                                               ││
││       spec  ███ withheld ███                                                                                       ││
││       note  run by the core after submit; worker never sees cases                                                  ││
││  ■ L3 Intent audit          [masked]                                                                               ││
││       spec  ███ withheld ███                                                                                       ││
││       note  scope / correctness / dead weight / swallowed / honesty                                                ││
││  ■ L4 Process check         [masked]                                                                               ││
││       spec  ███ withheld ███                                                                                       ││
││       note  “un-run is not done” · evidence required                                                               ││
││                                                                                                                    ││
││  [visible] worker-facing        [masked] held-out from the worker context                                          ││
││                                                                                                                    ││
││                                                                                                                    ││
││                                                                                                                    ││
││                                                                                                                    ││
││                                                                                                                    ││
││                                                                                                                    ││
││                                                                                                                    ││
││                                                                                                                    ││
││                                                                                                                    ││
││                                                                                                                    ││
││                                                                                                                    ││
││                                                                                                                    ││
││                                                                                                                    ││
│╰────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯│
│──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────│
│visible levels guide the worker · masked levels resist gaming · q quit                                                │
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
```

- **Layout.** A single tall panel listing the five levels L0–L4 top to bottom, each
  with a level glyph, a `[visible]`/`[masked]` badge, a `spec` line, and a `note`
  line. The intro line states the design principle; a legend at the bottom defines the
  two badges.
- **Interaction model.** A read-only explainer bound to the doc 05 ladder. Its whole
  purpose is the **visible-vs-held-out split**: L0/L1 render their real spec because
  the worker may read them; L2/L3/L4 render `███ withheld ███` for `spec` because the
  cases are held out — the note still describes *what* the level does without leaking
  *which* cases.
- **Keybindings.** `↑`/`↓` scroll the level list, `q` quits. No reveal key: masked
  specs stay masked in the worker-facing surface by design.
- **States.** Level glyph: `◉` visible/agent-facing (L0, L1), `■` masked/held-out (L2,
  L3, L4). Each level is independently pass/fail when populated with a run, but the
  masked levels never show their cases in this surface.

### 2.7 Intent-Scorer Report

```
╭ Cortex Code · Intent scorer · effectiveness report ────────────────────────────────────────────── MOCKUP · stub data ╮
│session cx-0001   workspace /work/acme-api   model cortex-large                                                       │
│──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────│
│╭ Dimensions · 0 miss / 1 partial / 2 full ──────────────────────────────────────────────────────────────────────────╮│
││  subject  feat-api-routes @ workflow/wf-42                                                                         ││
││                                                                                                                    ││
││  dimension                 score  gate   evidence                                                                  ││
││  ──────────────────────────────────────────────────────────────────────────────                                    ││
││  D1 Intent fidelity        2      ok     did exactly what was asked                                                ││
││  D2 Execution-verified     2      –      tests + curl surface captured                                             ││
││  D3 Scope discipline       1      ok     one unrequested helper added                                              ││
││  D4 Report honesty         2      ok     claims match the diff                                                     ││
││  D5 Reliability            1      –      one flaky retry observed                                                  ││
││                                                                                                                    ││
││  total    8/10   ████████░░                                                                                        ││
││  effectiveness  [PASS]   (PASS / FAIL / PARTIAL)                                                                   ││
││  repo-readiness [PASS]   (PASS / FAIL / N/A / unknown)                                                             ││
││                                                                                                                    ││
││  note  D3 gate met at ≥1; trim dead weight to reach full marks                                                     ││
││                                                                                                                    ││
││                                                                                                                    ││
││                                                                                                                    ││
││                                                                                                                    ││
││                                                                                                                    ││
││                                                                                                                    ││
││                                                                                                                    ││
││                                                                                                                    ││
││                                                                                                                    ││
││                                                                                                                    ││
││                                                                                                                    ││
││                                                                                                                    ││
││                                                                                                                    ││
││                                                                                                                    ││
││                                                                                                                    ││
││                                                                                                                    ││
││                                                                                                                    ││
│╰────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯│
│──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────│
│PASS requires total ≥ 8 AND every gate dimension ≥ 1 · q quit                                                         │
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
```

- **Layout.** One panel. A `subject` line names what was scored (a feature at a
  workflow ref). A five-row table gives each dimension D1–D5 a `score` (0 miss / 1
  partial / 2 full), a `gate` marker (`ok` for gate dimensions, `–` for non-gating),
  and one-line `evidence`. Below: a `total` with a bar, the `effectiveness` verdict,
  and the separate `repo-readiness` signal, then a `note`.
- **Interaction model.** A read-only scorecard produced by the doc 07 Intent Scorer,
  consuming the trajectory from doc 02's `record`. It renders the exact PASS rule:
  `total ≥ 8 AND every gate dimension ≥ 1`. The `repo-readiness` line is the distinct
  Repo Readiness report signal (a repository evaluation, not the change score).
- **Keybindings.** `↑`/`↓` scroll dimensions, `enter` on a dimension expands its
  evidence (into the note area), `q` quits.
- **States.** Per dimension: score `0`/`1`/`2`, gate `ok`/`–`. Verdicts:
  effectiveness `PASS`/`FAIL`/`PARTIAL`; repo-readiness `PASS`/`FAIL`/`N/A`/`unknown`.
  The bar reflects `total/10`.

### 2.8 Concurrency Metrics

```
╭ Cortex Code · Concurrency metrics · parallel-metrics.jsonl ────────────────────────────────────── MOCKUP · stub data ╮
│session cx-0001   workspace /work/acme-api   model cortex-large                                                       │
│──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────│
│╭ Conflict rate · per wave ──────────────────────────────────────────────────────────────────────────────────────────╮│
││  wave  pairs  intersecting  rate     decision                                                                      ││
││  ──────────────────────────────────────────────────────────                                                        ││
││  1     1      0              0.0%   [WIDEN]                                                                        ││
││  2     3      0              0.0%   [WIDEN]                                                                        ││
││  3     6      1             16.7%   [HOLD]                                                                         ││
││                                                                                                                    ││
││  threshold 15%   wave 3 ██░░░░░░░░░░ 16.7% → hold parallelism at N                                                 ││
│╰────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯│
│╭ Merge-back · D8 ────────────────────────────────────────╮╭ Quarantine ─────────────────────────────────────────────╮│
││  worker branch → integration branch                     ││  ref  refs/workflow/quarantine/feat-api-routes          ││
││                                                         ││                                                         ││
││  feat-db-schema      [completed]                        ││  [infra_merge_conflict]  overlapping edits in routes.rs ││
││  feat-auth-login     [completed]                        ││  [infra_test_regression]  suite red after merge         ││
││  feat-api-routes     [merge_blocked]                    ││                                                         ││
││                                                         ││  disposition  held for triage; out of end gate          ││
││  on conflict: core sets merge_blocked,                  ││  reattach     re-post as fresh READY item once resolved ││
││  never force-resolves; routes to quarantine             ││                                                         ││
││                                                         ││                                                         ││
││                                                         ││                                                         ││
││                                                         ││                                                         ││
││                                                         ││                                                         ││
││                                                         ││                                                         ││
││                                                         ││                                                         ││
││                                                         ││                                                         ││
││                                                         ││                                                         ││
││                                                         ││                                                         ││
││                                                         ││                                                         ││
││                                                         ││                                                         ││
││                                                         ││                                                         ││
││                                                         ││                                                         ││
││                                                         ││                                                         ││
││                                                         ││                                                         ││
│╰─────────────────────────────────────────────────────────╯╰─────────────────────────────────────────────────────────╯│
│──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────│
│conflict rate < 15% → widen · ≥ 15% → hold · q quit                                                                   │
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
```

- **Layout.** Top: the per-wave conflict-rate table (`wave`, `pairs`, `intersecting`,
  `rate`, `decision`) read from `parallel-metrics.jsonl`, with a highlighted bar for
  the latest wave against the 15% threshold. Bottom two panels: **Merge-back (D8)** —
  each worker branch's merge status into the integration branch — and **Quarantine** —
  the failed-merge parking ref, its failure classes, disposition, and reattach path.
- **Interaction model.** A read-only accounting surface for the doc 03 conflict metric
  and merge-back/quarantine machinery. It states the go/no-go rule directly: conflict
  rate `< 15%` → widen parallelism, `≥ 15%` → hold. Merge-back is serialized and
  test-gated; on conflict the core sets `merge_blocked` and routes to quarantine — it
  never force-resolves.
- **Keybindings.** `↑`/`↓` select a wave row or a branch row, `enter` on a
  quarantined item shows its failure detail, `q` quits.
- **States.** Per wave decision: `[WIDEN]` / `[HOLD]`. Per branch merge:
  `[completed]` / `[merge_blocked]`. Quarantine failure classes:
  `[infra_merge_conflict]` / `[infra_test_regression]`; disposition held-for-triage
  with a reattach path back to `READY`.

### 2.9 Tool Envelope

```
╭ Cortex Code · Tool envelope · response contract ───────────────────────────────────────────────── MOCKUP · stub data ╮
│session cx-0001   workspace /work/acme-api   model cortex-large                                                       │
│──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────│
│╭ Contract · fixed field order ──────────────────────────────────────────────────────────────────────────────────────╮│
││  1. status     ok | error | refused  outcome discriminant                                                          ││
││  2. payload    object                result, or reason on error/refused                                            ││
││  3. truncated  bool                  true when output was clipped                                                  ││
││                                                                                                                    ││
││  order is stable across all 11 tools — find grep_code read_span list_tree run edit_span                            ││
││                                              write_file web_lookup ask plan_note spawn_subagent                    ││
│╰────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯│
│╭ Example · status = ok ──────────────────────────────────╮╭ Example · status = refused ─────────────────────────────╮│
││  [ok]                                                   ││  [refused]  policy stopped the call                     ││
││  {                                                      ││  {                                                      ││
││    "status": "ok",                                      ││    "status": "refused",                                 ││
││    "payload": {                                         ││    "payload": {                                         ││
││      "matches": 3,                                      ││      "applied": false,                                  ││
││      "file": "routes.rs"                                ││      "reason": "explore-before-edit"                    ││
││    },                                                   ││    },                                                   ││
││    "truncated": false,                                  ││    "truncated": false,                                  ││
││  }                                                      ││  }                                                      ││
││                                                         ││                                                         ││
││  reader checks status first, then truncated             ││  [error] = tool ran & failed   [refused] = never ran    ││
││                                                         ││                                                         ││
││                                                         ││                                                         ││
││                                                         ││                                                         ││
││                                                         ││                                                         ││
││                                                         ││                                                         ││
││                                                         ││                                                         ││
││                                                         ││                                                         ││
││                                                         ││                                                         ││
││                                                         ││                                                         ││
││                                                         ││                                                         ││
││                                                         ││                                                         ││
││                                                         ││                                                         ││
││                                                         ││                                                         ││
│╰─────────────────────────────────────────────────────────╯╰─────────────────────────────────────────────────────────╯│
│──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────│
│every tool returns the same 3 fields, in order · q quit                                                               │
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
```

- **Layout.** Top: the contract card — the three fields in fixed order (`1. status`,
  `2. payload`, `3. truncated`) with type and role, and the full list of the 11 tools
  the order holds for (`find`, `grep_code`, `read_span`, `list_tree`, `run`,
  `edit_span`, `write_file`, `web_lookup`, `ask`, `plan_note`, `spawn_subagent`).
  Bottom: two worked examples side by side — a `status = ok` response and a
  `status = refused` response.
- **Interaction model.** A read-only contract explainer that governs how the TUI tool
  cards (`crates/tui/src/tool_card.rs`) render every result: read `status` first, then
  `truncated`. It fixes the crucial distinction: `[error]` = the tool ran and failed;
  `[refused]` = the tool never ran (policy stopped it).
- **Keybindings.** `tab` toggles the two examples, `↑`/`↓` scroll, `q` quits.
- **States.** `status` tri-state: `ok` / `error` / `refused`. `truncated` boolean
  (drives a "output clipped" affordance on the card). These three fields are the only
  contract every tool card must render, in order.

---

## 3. Gap Analysis

The current client renders exactly one subsystem — the single-agent chat turn — and
none of the eight new ones. Each frame is a net-new view; nothing is refactored away.

| Subsystem surface | Current CLI/TUI | Target (frame) | Gap |
|---|---|---|---|
| Mode switch | `--mode` overlay (`cli.rs:1211`), custom modes (`custom_modes.rs`), plan-blocked banner (`app.rs:43-45`); no picker, no grant matrix | §2.1 five-mode picker + tool-grant matrix + detail card | **New view + `/mode` command + mode-cycle keybind** |
| Workflow launch | none; no `Workflow` in `enum Command` (`cli.rs:36`) | §2.2 preflight checklist + `features.json` graph | **New `run_workflow` CLI subcommand + launch view** |
| Workflow monitor | none | §2.3 A–F lifecycle + verdict→status table + wave panel | **New live monitor reading trajectory/`progress-log.jsonl`** |
| Team board | none | §2.4 six-column board + item contract + Coordinator | **New Team board view + item-contract panel** |
| Reviewer output | `/review` label only (`slash.rs:137-138`) | §2.5 Code Reviewer L3 + QA Validator L4 verdicts | **New two-panel verdict view; wire `/review`** |
| Verification ladder | none | §2.6 L0–L4 visible/held-out list | **New ladder explainer view** |
| Intent-scorer report | none | §2.7 D1–D5 scorecard + repo-readiness | **New scorecard view** |
| Concurrency metrics | none | §2.8 per-wave conflict + merge-back + quarantine | **New metrics view reading `parallel-metrics.jsonl`** |
| Tool envelope | tool cards render results (`tool_card.rs`) but not the fixed contract | §2.9 contract card + ok/refused examples | **New contract view + card conformance to status/payload/truncated** |

Cross-cutting gaps:

- **No mode operating-switch** — modes exist as config overlays, not as a first-class
  runtime switch with a picker and gate feedback.
- **No Workflow/Team entry points** — neither a `run_workflow` subcommand nor a Team
  board command exists.
- **No read model for the new artifacts** — the TUI has no reader for `features.json`,
  `parallel-metrics.jsonl`, `progress-log.jsonl`, or the reviewer/intent-scorer output;
  the monitor/metrics/reviewer/scorer views all need one.
- **Tool cards do not encode the envelope contract** — results render ad hoc, not
  through the fixed `status → payload → truncated` reader order.

What is reusable (no gap): the page/panel/card/list chrome
(`crates/tui-components/src/`), the slash registry (`slash.rs:19`), the keybind action
set (`keybind.rs:16`), the `RunnerEvent` stream (`app.rs:20`), and the persona/mode
plumbing (`agent.rs`, `custom_modes.rs`). Every new view composes the existing kit.

---

## 4. Crate / File Change Map

Grounded in the verified layout: `crates/cli/src/` (flat command modules + `cli.rs`
arg tree), `crates/tui/src/` (views + `app.rs`/`slash.rs`/`keybind.rs`), and
`crates/tui-components/src/` (the widget kit). New view modules follow the existing
one-file-per-view convention (`gallery.rs`, `diff.rs`, `onboarding.rs` are siblings).

| File | New / Extend | What changes |
|---|---|---|
| `crates/tui/src/mode_switch.rs` | **new** | The §2.1 mode picker: five-mode list, tool-grant matrix, detail card; renders the doc 01 grant, issues a promotion request via the gate. |
| `crates/tui/src/workflow_launch.rs` | **new** | The §2.2 preflight checklist + `features.json` graph renderer; green/red gate, wave-1 computation. |
| `crates/tui/src/workflow_monitor.rs` | **new** | The §2.3 A–F lifecycle ribbon, verdict→status feature table, wave panel; reads trajectory + `progress-log.jsonl` + `parallel-metrics.jsonl`. |
| `crates/tui/src/team_board.rs` | **new** | The §2.4 six-column board + `BLOCKED` lane + item-contract panel + Coordinator/assignments. |
| `crates/tui/src/reviewer_output.rs` | **new** | The §2.5 Code Reviewer (L3) + QA Validator (L4) two-panel verdict view. |
| `crates/tui/src/verification_ladder.rs` | **new** | The §2.6 L0–L4 list with visible/masked badges. |
| `crates/tui/src/intent_scorer.rs` | **new** | The §2.7 D1–D5 scorecard + effectiveness + repo-readiness. |
| `crates/tui/src/concurrency_metrics.rs` | **new** | The §2.8 per-wave conflict table + merge-back + quarantine panels. |
| `crates/tui/src/tool_envelope.rs` | **new** | The §2.9 contract card + ok/refused examples. |
| `crates/tui/src/tool_card.rs` | **extend** | Render every tool result through the fixed `status → payload → truncated` reader order; distinguish `error` (ran & failed) from `refused` (never ran). |
| `crates/tui/src/slash.rs` | **extend** | Add builtins: `/mode`, `/workflow`, `/monitor`, `/board`, `/reviewers`, `/ladder`, `/score`, `/metrics`, `/envelope`; wire `/review` (`slash.rs:137-138`) to `reviewer_output.rs`. |
| `crates/tui/src/keybind.rs` | **extend** | Add `KeybindAction::ModeCycleNext` (sibling of `AgentCycleNext` at `keybind.rs:42`) and palette openers for the new views. |
| `crates/tui/src/app.rs` | **extend** | Route the new views into the app view stack; subscribe them to the relevant `RunnerEvent`/artifact updates (imports at `app.rs:20`). |
| `crates/tui/src/view.rs` | **extend** | Register the nine new views in the view enum/dispatch. |
| `crates/tui/src/lib.rs` | **extend** | `mod` declarations for the nine new view modules. |
| `crates/cli/src/cli.rs` | **extend** | Add a `Workflow(WorkflowArgs)` variant to `enum Command` (`cli.rs:36`) for the headless `run_workflow` launch/monitor path (parity with `RunArgs` at `cli.rs:1178`). |
| `crates/cli/src/workflow_cmd.rs` | **new** | Dispatch for the `Workflow` subcommand (launch/status), mirroring the shape of `run.rs`/`session.rs`. |
| `crates/tui-components/src/key_hints.rs` | **extend** | Footer hint rows for the new views (the `· q quit` line and per-view keys). |

No file outside the above changes. No new crate is introduced; every new view lives in
the existing `crates/tui` and `crates/cli` crates and composes `crates/tui-components`.

---

## 5. Milestones

Milestones are cut as **vertical interaction slices** (TUI Application Playbook): each
renders, reacts to at least one key, and updates state, and is verifiable on its own
against its captured frame. Each has a binary acceptance check.

### M1: Shared chrome + tool-envelope conformance
Establish the common view chrome (title bar / context strip / footer) as a reusable
composition and make tool cards render the envelope contract.
- **AC1**: the §2.9 `tool_envelope.rs` view renders and matches the captured frame at
  120×40 under the terminal driver.
- **AC2**: `tool_card.rs` renders `ok`/`error`/`refused` in fixed field order; a driver
  test asserts a `refused` result shows `[refused]` and never a partial payload.
- **AC3**: `cargo clippy -p tui -p tui-components -- -D warnings` clean (Verification L0).

### M2: Mode switch
Ship the §2.1 picker over the doc 01 grant.
- **AC1**: five modes render in canonical order with the grant matrix; the highlight
  moves with `↑`/`↓`.
- **AC2**: `enter` issues a mode-promotion request (not a silent switch); a gate-refused
  promotion shows the blocked treatment (parity with `app.rs:43-45`).
- **AC3**: `/mode` opens the view; `ModeCycleNext` cycles the active mode.

### M3: Workflow launch + monitor
Ship §2.2 and §2.3 as one slice (launch hands off to monitor).
- **AC1**: preflight renders green and red; a missing artifact disables launch and
  shows the `MISSING <artifact>; exit 1` abort form.
- **AC2**: the `features.json` graph and computed `wave 1 (parallel-safe)` render for a
  fixture graph.
- **AC3**: the monitor renders the A–F ribbon and the verdict→status table for a fixture
  run and repaints on a trajectory update.

### M4: Team board
Ship §2.4.
- **AC1**: the six columns + `BLOCKED` lane render item ids in the right columns for a
  fixture board.
- **AC2**: selecting an item populates the contract panel with all ten fields; the
  Coordinator panel shows assignments including a `(reconciling)` entry.
- **AC3**: the board is inspection-only — a driver test asserts no key mutates item state.

### M5: Reviewer output + verification ladder
Ship §2.5 and §2.6.
- **AC1**: the two reviewer panels render Q1–Q5 and the four QA gates with per-check
  pass/fail; `tab` toggles focus.
- **AC2**: the ladder renders L0/L1 `[visible]` with real spec and L2–L4 `[masked]` with
  `███ withheld ███`; a driver test asserts masked specs are never revealed.

### M6: Intent-scorer report + concurrency metrics
Ship §2.7 and §2.8.
- **AC1**: the scorecard renders D1–D5 with score/gate/evidence, the `total/10` bar, and
  both verdicts; a test asserts the PASS rule (`total ≥ 8 AND every gate ≥ 1`).
- **AC2**: the metrics view renders the per-wave table with correct `[WIDEN]`/`[HOLD]`
  decisions against the 15% threshold, plus merge-back and quarantine panels.
- **AC3**: the concurrency view reads a fixture `parallel-metrics.jsonl` and matches the
  captured frame.

Every milestone's slice is done only when: headless state tests pass, the
terminal-driver scenario passes against a captured frame, and it renders cleanly at
more than one terminal size (§8 resize matrix).

---

## 6. TDD Plan

Test-first per the tui-testing Playbook prototype (`playbooks/tui-testing.md`): split
each view along the **render/state seam** — a state layer with no terminal dependency
(unit-tested headless) and a render/interaction layer (snapshot-tested by driving a
pseudo-terminal and asserting on the captured frame). A rendering bug is invisible to
unit tests, so the captured-frame assertion is mandatory for every slice. This section
states test intent only; no test code is written here.

**State-layer scenarios (headless, no terminal):**

1. **Mode grant projection.** Given the doc 01 grant, the mode-switch state projects
   the 11-tool × 3-mode matrix with the four cell states (`yes`/`—`/`gated`/`read-only`).
   Assert the `run`-under-Plan cell is `read-only` and `spawn_subagent` is `—` outside
   Agent Mode.
2. **Preflight verdict.** Given an artifact set, the launch state returns green iff all
   required artifacts exist, else the first `MISSING <artifact>` and an exit-1 intent.
3. **Wave-1 computation.** Given a fixture `features.json`, the launch state computes the
   parallel-safe root set (`[feat-db-schema]`).
4. **Verdict→status mapping.** The monitor state keeps worker `verdict` and queue
   `status` as two independent axes; assert a `partial` verdict with `partial` status
   and a `failed`/`failure` row coexist.
5. **Conflict decision.** The metrics state maps a per-wave rate to `[WIDEN]` below 15%
   and `[HOLD]` at/above; assert the boundary at exactly 15% and the 16.7% → `[HOLD]`.
6. **Intent-scorer PASS rule.** The scorer state returns `PASS` iff `total ≥ 8` AND
   every gate dimension `≥ 1`; assert a 8/10 with all gates met is `PASS` and a
   gate-dimension `0` flips it to `FAIL` even at total ≥ 8.
7. **Envelope reader order.** The tool-card state exposes `status` before `truncated`;
   assert a `refused` never carries a success payload and `error` ≠ `refused`.
8. **Ladder masking.** The ladder state marks L0/L1 `visible` and L2–L4 `masked`;
   assert masked levels expose a note but never a spec string.

**Render/interaction scenarios (pseudo-terminal driver, assert on captured frame):**

- **Frame parity** for each of the nine views: launch the view at 120×40, render, and
  assert the captured frame equals the corresponding `plans/assets/mockups/*.txt`
  fixture (byte-for-byte on the content region).
- **Mode highlight move**: send `↓`, assert the `▶` cursor advances and the detail card
  repaints for the newly highlighted mode.
- **Reviewer focus toggle**: send `tab`, assert focus moves between the L3 and L4 panels.
- **Board immutability**: send every navigation and action key, assert no item changes
  column (the core moves items, not the operator).
- **Resize**: render each view at 80×24, 120×40, and 200×50 (§8 matrix); assert no panel
  overflows, no border misaligns, and the footer stays pinned.

**Ladder mapping for this doc's own changes:** L0 = `cargo build -p tui -p
tui-components -p cli` + `cargo clippy -- -D warnings` + `cargo fmt --check`; L1 = the
state-layer + driver suites above. L2–L4 are held out (doc 05). Poll for the expected
frame — never a fixed `sleep` — to avoid flaky-by-construction captures.

---

## 7. Wave / Dependency Position

This is the **UI layer over docs 01–07**: it renders their surfaces and adds no new
core semantics. It is the last subsystem doc before `plans/00-MASTER-PLAN.md`.

**Upstream (must land first):**
- `plans/01-*.md` (Prompt / Mode system): supplies the mode set and the mode gate the
  §2.1 switcher renders and calls.
- `plans/02-engine-7step-loop.md`: supplies the trajectory + Session Log the monitor
  (§2.3), reviewer (§2.5), and intent-scorer (§2.7) views read.
- `plans/03-*.md` (Workflow / workers): supplies `run_workflow`, `features.json`, waves,
  merge-back, and quarantine rendered by §2.2/§2.3/§2.8.
- `plans/05-*.md` (Verification / Reviewers): supplies the L0–L4 ladder and the Code
  Reviewer / QA Validator verdicts rendered by §2.5/§2.6.
- `plans/06-subagents-skills.md` (Subagents / Skills): supplies `spawn_subagent` and the
  scoped-worker model reflected in the §2.1 grant and the §2.4 item contract.
- `plans/07-*.md` (Intent Scorer): supplies the effectiveness dimensions and the Repo
  Readiness report rendered by §2.7.

**Downstream:**
- `plans/00-MASTER-PLAN.md`: this doc is its final input; the master doc sequences all
  subsystem docs and this UI layer closes the set.

**Wave ordering:**
- Wave A: M1 (shared chrome + tool envelope) — depends only on the tool contract; can
  start as soon as the component kit is stable.
- Wave B: M2 (needs doc 01 gate) and M5 (needs doc 05) in parallel; M3 (needs doc 03)
  and M6-metrics (needs doc 03 concurrency) in parallel.
- Wave C: M4 (Team board, needs doc 03 Team) and M6-scorer (needs doc 07); then
  integration — all nine views wired into `view.rs`/`app.rs`, resize matrix green, and
  the frame-parity suite passing against `plans/assets/mockups/*.txt`.

---

## 8. Risks & Mitigations

- **Terminal width (R1).** The frames are authored at 120 columns; panels are
  side-by-side and will overflow narrow terminals. Mitigation: define a responsive
  breakpoint — below ~100 columns, stack side-by-side panels vertically (the monitor
  and metrics views already stack); the resize matrix (80×24 / 120×40 / 200×50) is a
  gating test (§6). Never hardcode one width (Playbook pitfall).
- **Unicode / CJK width (R2).** Box-drawing (`╭─│╰`), block glyphs (`█░`), the mode
  glyphs (`●◉○■▶`), and curly quotes in the mottos are wide/ambiguous-width; a mis-measured
  cell drifts every border to its right. Mitigation: measure display width with the same
  width table the buffer uses (`crates/tui-buffer`, `crates/tui-text`), and add a CJK
  content-string case to the frame-parity suite (visual-QA CJK precision) so
  double-width text never clips a panel edge.
- **Color / no-color (R3).** Verdict/decision badges (`[WIDEN]`/`[HOLD]`,
  `[verified]`/`[failed]`, `✓`/`✗`) must not rely on color alone. Mitigation: every
  state carries a text token in addition to any color (the frames already do this);
  honor `NO_COLOR` and the `color_scheme.rs` palette rather than raw ANSI, per
  `design-guidelines.md` (no new palette).
- **Resize mid-run (R4).** The live monitor/metrics views repaint from streaming
  artifacts while the terminal may resize. Mitigation: recompute layout on every resize
  event and re-flow from the state layer (which is terminal-independent), never from a
  cached frame; poll for the settled frame in tests rather than sleeping.
- **Held-out leakage (R5).** The ladder (§2.6) and reviewer (§2.5) views must never
  render L2–L4 held-out cases in a worker-facing surface. Mitigation: masking lives in
  the state layer (`███ withheld ███` is the only value the render layer ever receives),
  and a driver test asserts no masked spec string appears in any capture.
- **Read-model drift (R6).** The monitor/metrics/reviewer/scorer views read artifacts
  (`features.json`, `parallel-metrics.jsonl`, `progress-log.jsonl`, reviewer/scorer
  output) authored by docs 02/03/05/07; a schema change upstream silently breaks a view.
  Mitigation: parse through shared typed readers with fixture-backed tests, and fail
  visibly (an in-panel error state) rather than rendering stale or partial data.

---

## 9. Commit Sequence

Commits are authored and committed as `echobt` (per the commit-identity skill). One
logical commit per slice; shared chrome and readers land before the views that use
them. Each commit leaves `cargo build`, `cargo clippy -- -D warnings`, `cargo fmt
--check`, and `cargo test -p tui -p tui-components -p cli` green (Verification L0–L1)
before the next begins.

1. `tui: shared view chrome + tool-envelope contract rendering (M1)` — the reusable
   title/context/footer composition, `tool_envelope.rs`, and `tool_card.rs`
   conformance to `status → payload → truncated`; frame-parity + `refused≠error` tests.
2. `tui,cli: mode switch view + /mode + ModeCycleNext (M2)` — `mode_switch.rs` over the
   doc 01 grant, `slash.rs`/`keybind.rs` wiring, promotion-request + gate-refused states.
3. `cli: run_workflow subcommand scaffold (M3 prerequisite)` — `Workflow` variant in
   `enum Command`, `workflow_cmd.rs` dispatch; parity shape with `run.rs`.
4. `tui: workflow launch + monitor views (M3)` — `workflow_launch.rs` (preflight +
   `features.json` graph + wave-1) and `workflow_monitor.rs` (A–F ribbon +
   verdict→status table + wave panel).
5. `tui: team board view + item contract + coordinator (M4)` — `team_board.rs`, six
   columns + `BLOCKED` lane + contract panel; inspection-only immutability test.
6. `tui: reviewer output + verification ladder views (M5)` — `reviewer_output.rs`
   (L3+L4) and `verification_ladder.rs` (visible/masked); no-leak masking test.
7. `tui: intent-scorer report + concurrency metrics views (M6)` — `intent_scorer.rs`
   (D1–D5 + PASS rule) and `concurrency_metrics.rs` (per-wave conflict + merge-back +
   quarantine) reading `parallel-metrics.jsonl`.
8. `tui: register all nine views + resize matrix + frame-parity suite (integration)` —
   `view.rs`/`app.rs`/`lib.rs` wiring, `slash.rs` remaining builtins, `key_hints.rs`
   footers; the full 80×24/120×40/200×50 resize matrix and byte-for-byte parity against
   `plans/assets/mockups/*.txt`.
