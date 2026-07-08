# 01 — Prompt + Mode System

> Implementation plan for the modular prompt-block assembler and the 5 modes with a
> schema-level **MODE GATE**. Part of the `plans/` set; see `plans/00-*` (master index)
> for the doc map. Downstream consumers: `plans/02-*` (the 7-step loop consumes the MODE
> GATE at its `gate` phase), `plans/06-*` (Agent-mode subagent grant / Skills), and
> `plans/05-*` (Reviewers / Verification levels). Cross-references use sibling filenames.

---

## 0. Scope & Prototype Contracts

**In scope.** (1) A program-owned, ordered **PromptBlock** assembler that replaces the
current fixed concat; (2) the five modes and the two composing surfaces loaded as data;
(3) a **MODE GATE** that filters the tool schema set exposed to the model per mode and
returns the 3-field refusal envelope when a disallowed tool is invoked; (4) CLI / TUI /
slash wiring so a user can select and switch modes.

**Out of scope (owned elsewhere).** The 7-step turn loop that *drives* the assembler and
*calls* the gate → `plans/02-*`. Subagent spawning / Skills loading behind Agent Mode →
`plans/06-*`. Reviewers and the held-out Verification levels → `plans/05-*`. Workflow /
Team orchestration → `plans/03-*`. Concurrency → `plans/04-*`. Intent Scorer → `plans/07-*`.
CLI UI/UX surface polish (pickers, palette) → `plans/08-*`. This doc delivers the prompt
assembly + mode capability substrate those docs build on.

**Authoritative prototype sources read for this plan (cited by name):**

- `PROMPT-DISPATCH.md` — the block order and per-turn assembly rules. Canonical block
  sequence (§1): `[00-core-identity][mode-body][instruction-blocks][runtime-context][task-reminder][tool-cards]`;
  body selection (§2), instruction-block injection matrix (§3), the `<runtime>` context
  block (§4), and the autonomy GATE (§6).
- `prompts/00-core-identity.md` — the always-present identity + honesty contract layer.
- `prompts/01-plan.md`, `prompts/02-edit.md`, `prompts/03-agent.md`,
  `prompts/04-chat.md`, `prompts/05-headless.md`, `prompts/06-orchestration-mindset.md`
  — the five mode/surface bodies plus the orchestration-mindset body.
- `prompts/instruction-blocks/*.md` — the nine injectable blocks: `persistence.md`,
  `plan-guidelines.md`, `tool-usage-rules.md`, `plan-note-guidelines.md`,
  `markdown-spec.md`, `cli-preference.md`, `unstaged-changes.md`, `coding-guidelines.md`,
  `design-guidelines.md`.
- `TOOLS.md` — the mode/tool matrix (§"Mode / tool matrix"), the three-field return
  envelope `{status, payload, truncated}`, and the harness-enforced rules (mandatory
  execution, explore-before-edit, lazy role-scoped exposure).
- `GLOSSARY.md` — frozen surface vocabulary (one-term-one-surface).
- `docs/rename/naming-map.md` — frozen rename reference. All surface names in this plan
  are picked from these two files; **no new domain terms are coined.**

**Prototype contract restated (binding on this plan):**

- Block order is fixed and **program-owned**; the model never composes its own system
  prompt (`PROMPT-DISPATCH.md` §1, §preamble).
- `00-core-identity` is always present; exactly **one** mode body is sent per turn — Plan
  Mode XOR Edit Mode XOR Agent Mode — and the Chat Mode / Headless / Orchestration-mindset
  bodies **compose with** the active mode rather than adding a fourth mode
  (`PROMPT-DISPATCH.md` §2).
- The three real modes and their tool grants (`TOOLS.md` mode/tool matrix): **Plan Mode**
  = read-only; **Edit Mode** = read + write + `run`; **Agent Mode** = Edit-mode grant plus
  `spawn_subagent`.
- The MODE GATE enforces tool grants at the **schema level** for Plan / Edit / Agent — a
  read-only surface physically cannot write because the tool is not in its exposed set
  (`PROMPT-DISPATCH.md` §6.1, `TOOLS.md` "Lazy, role-scoped exposure" / F5).
- Every tool result — including a gate refusal — uses the three-field envelope
  `{ "status": "ok|error|refused", "payload": {…}, "truncated": false }`; a refused call
  carries `payload.reason` (`TOOLS.md` "The return envelope").
- Mode promotion (Plan → Edit, entry to Agent) is **core-granted**, never a tool the model
  invokes on itself (`PROMPT-DISPATCH.md` §6.5).

**Banned-term note.** Per `MISSION-BRIEF.md` §0, the prior-art TypeScript agent name is
never written in this repository; where prior art must be referenced it is called "the
reference TypeScript agent." No such reference is required in this plan.

---

## 1. Current State (file:line)

Verified against current HEAD (line numbers spot-checked in `context.rs`, `run.rs`,
`permission.rs`; remainder confirmed by exploration).

**Prompt assembly (engine crate).**

- `crates/engine/src/family.rs` — model-family selection only, **not** a multi-part
  concat. `enum ModelFamily { Anthropic, Gpt, Codex, Gemini, Default }` (lines 10-22);
  `from_model_id` (31-48); `base_prompt(self) -> &'static str` (53-62) returns one of five
  `const *_PROMPT` single strings: `DEFAULT_PROMPT` (65-69), `GEMINI_PROMPT` (71-74),
  `ANTHROPIC_PROMPT` (76-79), `GPT_PROMPT` (81-84), `CODEX_PROMPT` (86-89).
- `crates/engine/src/context.rs` — **the actual fixed concat.** `struct SystemContext
  { family, environment, project_memory: Option<String>, agent: Agent }` (16-27);
  builders `new` (32-39), `for_model` (43-45), `with_agent` (49-52),
  `with_project_memory` (56-59); `render(&self) -> String` (68-83) concatenates, in a
  hard-coded order: `base_prompt` + `"\n\n"` + `environment.render()` (69-73), then
  optional `project_memory` (74-77), then optional `agent.persona_prompt()` (78-81). Four
  fixed parts, no block registry, no per-mode instruction blocks, no reminder slot, no
  tool-cards slot.
- `crates/engine/src/environment.rs` — the runtime-context precursor. `enum GitStatus`
  (19-31) + `describe()` (36-50); `struct EnvironmentBlock { cwd, platform, os, arch,
  date, git }` (55-68); `from_process()` (77-92); `render()` (99-114) emits an
  `<environment>…</environment>` block with cwd / platform / date / git — **no** `mode`,
  `surface`, `budgets`, or `net` fields.
- `crates/engine/src/agent.rs` — an existing persona enum overlapping the mode concept.
  `enum Agent { Build, Plan, General, Explore }` (13-23); `DEFAULT = Build` (27);
  `SUBAGENTS = [Plan, General, Explore]` (34); `from_name` (71-82); `persona_prompt`
  (86-95); `const PLAN_PERSONA` (158-161). Module doc calls these "behavioral modes …
  system-prompt fragment."
- `crates/engine/src/message_parts.rs:32` — `MessagePartBody::Agent { … mode: None }`; a
  `mode` field already exists on agent message parts (currently unused).

**CLI wiring.**

- `crates/cli/src/cli.rs` — `struct RunArgs` (1178); `--agent` → `pub agent:
  Option<String>` (1206-1207); `--mode` → `pub mode: Option<String>` (1209-1211). Both are
  free-form `Option<String>`; **no clap enum** constrains mode values (contrast
  `OutputFormat` value-enum ~1170).
- `crates/cli/src/main.rs` — `main()` (66); dispatch `match cli.command` (99);
  `Command::Run(args) => run::run(...)` (112).
- `crates/cli/src/run.rs` — `run(args, bus, pure_plugins)` (72); mode discovery
  `custom_modes::discover` (117); resolution `custom_modes::resolve` (178-184); the
  system-prompt composition entry `render_system_context(model_id, agent, mode, skills,
  project_memory)` (980-1007) which computes `effective_base_agent` (987-989), calls
  `SystemContext::for_model(...).with_project_memory(...).with_agent(...).render()`
  (990-993), then string-appends `"Custom agent instructions:"` (994-997) and
  `"Mode instructions:"` (998-1001). Mode permissions merged at 311-321.
- `crates/cli/src/custom_modes.rs` — an existing **mode overlay**. `struct CustomMode {
  … base_agent: Option<Agent> … }` (18-24); `struct ActiveMode` (42-44) with `name` (52),
  `base_agent` (58-59), `prompt` (64), `permissions() -> &[Rule]` (70); `discover` (105);
  `resolve(...) -> Result<Option<ActiveMode>, UnknownMode>` (153-181); `UnknownMode` (77).
- `crates/cli/src/permission.rs` — the current per-mode enforcement, **at the permission
  layer, not the schema layer**. `struct BuiltinAgentPermissionRules { base_rules,
  restriction_rules }` (110-114); `builtin_agent_permission_rules(agent)` (122-137):
  `Plan | Explore` get `read_only_allow_rules()` (139-144: allow `read, glob, grep, lsp,
  webfetch, websearch`) plus `mutation_deny_rules()` (146-151: hard-deny `write, edit,
  apply_patch, format, bash`); `Build | General` get permissive defaults. Mutating tools
  stay **registered and exposed**; they are denied at execution time.

**TUI / tools / permissions engine.**

- `crates/tui/src/slash.rs` — `struct SlashCommand { name, description }` (10-16);
  `const BUILTINS: &[SlashCommand]` (19-184) includes `/agents` (44-47) but **no `/mode`
  or `/modes`**; `filter(query)` (193-209). Dispatch owned by `crate::app::App`.
- `crates/tools/src/registry.rs` — `struct ToolRegistry { tools: Vec<Box<dyn Tool>> }`
  (34-37); `builtin()` (51-58, includes edit/write), `gpt()` (69-75, apply_patch, no
  edit/write), `register_utility_tools` (80-94), `definitions()` (166-175), `execute()`
  (182-197). **No per-mode filtering** — the only split is by model family. Transient
  "planning mode" exists as per-turn tool state (`plan.rs`, `plan_enter.rs`,
  `plan_exit.rs`; `tool.rs:669-702` "refuse mutating while planning active"), distinct
  from the Agent/mode enum.
- `crates/permissions/src/lib.rs` — engine API: `Rule { tool, pattern, action }`, `Ruleset`
  (last-matching-rule-wins, default `Ask`), `PermissionEngine`, `Action`, capability consts
  incl. `CAPABILITY_PLAN_ENTER/EXIT/EDIT/BASH/READ` (66-72). No plan/mode logic in
  `defaults.rs`.

---

## 2. Target State

A **program-owned ordered PromptBlock assembler** in the engine crate, five mode bodies
loaded as embedded data, and a **MODE GATE** that scopes the exposed tool schema per mode
and refuses disallowed calls with the three-field envelope.

**2.1 PromptBlock assembler (`crates/engine/src/prompt/`).**

- A `PromptBlock` value = `{ kind: BlockKind, body: Cow<'static, str> }` and an ordered
  `enum BlockKind` fixing the prototype order:
  `CoreIdentity → ModeBody → InstructionBlocks → RuntimeContext → TaskReminder → ToolCards`
  (`PROMPT-DISPATCH.md` §1). Ordering is a property of the type, not of call sites.
- A `PromptAssembler` builds the ordered `Vec<PromptBlock>` for `(mode, surface, env,
  reminder?, tool_cards?)` and renders to one system string. It **subsumes** the current
  `SystemContext::render` concat: `CoreIdentity` = `00-core-identity`; `ModeBody` = the one
  selected body; `InstructionBlocks` = the per-mode set (§2.3); `RuntimeContext` = the
  extended env block (§2.4); `TaskReminder` / `ToolCards` are optional slots owned/filled
  by `plans/02-*` and the registry respectively (this doc defines the slots + empty
  behavior, not the reminder scheduler).
- `family.rs::base_prompt` becomes the **model-family adapter** feeding the `CoreIdentity`
  block, preserving family selection while removing the ad-hoc concat.

**2.2 Modes and surfaces as first-class data.**

- `enum Mode { Plan, Edit, Agent }` (the three real operating modes; Plan XOR Edit XOR
  Agent) and `enum Surface { Chat, Headless, Orchestration, None }` (composing bodies),
  in the engine crate. Names taken verbatim from `GLOSSARY.md`.
- The six bodies (`00`–`06`) are embedded via `include_str!` from a resources directory
  vendored into the engine crate (real data, not mocks; user-facing behavior stays real
  per `MISSION-BRIEF.md` §0). `Mode`/`Surface` → body is a deterministic table mirroring
  `PROMPT-DISPATCH.md` §2. Exactly one `ModeBody` is emitted; `Surface` appends its
  composing body (Chat/Headless/Orchestration) without adding a second `ModeBody`.
- Reconciliation with existing types (decided here, executed in M2): `Mode` is the new
  authority for tool grants + body selection. The existing `engine::Agent` persona layer
  is retained but **derives** from `Mode` for built-in modes (Plan Mode → read-only grant;
  Edit Mode → read+write+run; Agent Mode → +subagents), and the existing
  `custom_modes::ActiveMode` overlay maps its `base_agent` onto a `Mode`. The free-form
  `--mode` string resolves to a `Mode` (built-in) or a `CustomMode` overlay carrying a
  `Mode`. No renaming of the `--mode` flag is required.

**2.3 Instruction-block registry.**

- Each of the nine `instruction-blocks/*.md` is embedded and tagged with the modes it
  injects into, exactly per `PROMPT-DISPATCH.md` §3:
  - `tool-usage-rules`, `markdown-spec`: Plan + Edit + Agent.
  - `plan-guidelines`: Plan only.
  - `plan-note-guidelines`: Plan + Agent.
  - `persistence`, `cli-preference`, `unstaged-changes`, `coding-guidelines`,
    `design-guidelines`: Edit + Agent.
- The assembler injects only the blocks whose tag set contains the active mode; the rest
  stay out of context (lean/modular composition).

**2.4 Extended runtime-context block.**

- `EnvironmentBlock::render` gains the `<runtime>`-equivalent fields from
  `PROMPT-DISPATCH.md` §4: `mode: <plan|edit|agent>`, `surface: <chat|headless|
  orchestration|none>`, `budgets: steps/wall-clock/edited-lines`, `net: <granted|off>`,
  alongside existing cwd/branch. Regenerated every turn (never carried stale). Budget
  values are program-owned inputs supplied by the loop (`plans/02-*`); this doc adds the
  fields + rendering and defaults them to observable placeholders when the loop has not
  set them.

**2.5 MODE GATE (`crates/engine/src/mode/gate.rs` + tools/permissions integration).**

- `Mode::exposed_tools(&self) -> &'static [&'static str]` — the allowlist per mode,
  expressed over the **current registry tool names** and mapped to the `TOOLS.md`
  mode/tool matrix:
  - Plan Mode: `read, glob, grep, lsp, webfetch, websearch, ask, plan` (read-only;
    corresponds to prototype `find, grep_code, read_span, list_tree, run`(read-only),
    `web_lookup`, `ask`, `plan_note`).
  - Edit Mode: Plan set **plus** `write, edit, apply_patch, bash, format` (corresponds to
    `edit_span, write_file, run`(gated)); `plan_note` drops out (matrix: Plan/Agent only).
  - Agent Mode: Edit set **plus** `task` (subagent spawn; prototype `spawn_subagent`) and
    re-adds `plan`.
  - A prototype→registry tool-name mapping table is recorded in this doc's Appendix within
    M3; a full tool **rename** to the `TOOLS.md` names is flagged as an open question (§8)
    and is **not** performed here.
- `ModeGate::filter_schema(registry_defs, mode) -> Vec<ToolDefinition>` — returns only the
  definitions whose name is in `mode.exposed_tools()`. This is the schema set the model
  sees (schema-level exposure; F5). Consumed by `plans/02-*` when it builds the turn's
  tool list, and by `plans/06-*` for Agent-mode subagent grants.
- `ModeGate::check(mode, tool_name) -> Result<(), Refusal>` — defense-in-depth: if a
  disallowed tool is invoked anyway, returns `Refusal { reason: "mode-not-permitted" }`.
  The tool executor renders this as the three-field envelope
  `{ "status": "refused", "payload": { "reason": "mode-not-permitted", "tool": <name>,
  "mode": <mode> }, "truncated": false }` (`TOOLS.md` return envelope). A shared
  `refusal_envelope(reason, extra)` helper produces this shape so every refusal path
  (mode, explore-before-edit, path/net policy, budget) is uniform.
- The existing `permission.rs` layer is **retained as the execution-time backstop** for
  gated tools (Plan-mode `run`/`bash` read-only enforcement, network, forbidden paths) and
  is derived from `Mode` instead of `Agent`. Division of labor: **MODE GATE = schema
  exposure** (model never sees the tool); **permission engine = runtime gate** for tools
  that are exposed but conditionally allowed. Both emit the same refusal shape.
- Mode promotion stays core-granted: a `promote(from, to)` core function (not a tool),
  surfaced to the user via CLI/TUI (M4). The model may request promotion in prose; it
  cannot self-promote.

**2.6 CLI / TUI / slash wiring.**

- `--mode` resolves to a built-in `Mode` or a custom overlay; an unknown value returns the
  existing `UnknownMode` error path (extended to built-ins).
- TUI gains a `/mode` slash command (list + switch) and a `/plan`↔`/edit`↔`/agent`
  promotion affordance dispatched in `app::App`; the active mode is shown in the runtime
  status and drives the exposed tool schema for the next turn.

---

## 3. Gap Analysis

| # | Prototype contract | Current state | Gap to close |
|---|---|---|---|
| G1 | Ordered block registry `[00-core-identity][mode-body][instruction-blocks][runtime-context][task-reminder][tool-cards]` | Fixed 4-part concat in `context.rs:68-83` | New `PromptAssembler` + `BlockKind` ordering type; port `render` onto it |
| G2 | Exactly one mode body per turn, chosen by core; Chat/Headless/Orchestration compose | Only `Agent` persona fragment appended; no body-per-mode selection | `Mode`/`Surface` enums + embedded bodies `00`–`06` + selection table |
| G3 | Per-mode instruction-block injection (9 blocks, matrix in §3) | None exist in codebase | Embed 9 blocks + mode-tag registry + injection logic |
| G4 | `<runtime>` block with mode/surface/budgets/net | `<environment>` has cwd/platform/date/git only | Extend `EnvironmentBlock` fields + `render` |
| G5 | Tool grants enforced at **schema** level per mode (F5) | Tools always registered; blocked only at permission layer (`permission.rs:146-151`) | `Mode::exposed_tools` + `ModeGate::filter_schema` schema filter |
| G6 | Disallowed call → `{status:"refused", payload.reason, truncated}` | No uniform refusal envelope for mode violations | `refusal_envelope` helper + `ModeGate::check` + executor integration |
| G7 | 3 real modes (Plan/Edit/Agent) as program-owned type | Overlapping `Agent` enum (Build/Plan/General/Explore) + free-form `--mode` string | Introduce `Mode`; derive `Agent`/permissions from it; map custom overlays |
| G8 | Mode promotion is core-granted, not self-invoked | No promotion path; `--mode` is per-run only | `promote()` core fn + CLI/TUI affordance |
| G9 | User can select/switch mode from TUI | No `/mode` slash command (`slash.rs`) | Add `/mode` + dispatch in `app::App` |
| G10 | Unknown mode is an explicit error | `UnknownMode` exists for custom only | Extend resolution to built-ins with same error |

---

## 4. Crate/File Change Map

| File path | Change kind | What changes |
|---|---|---|
| `crates/engine/src/prompt/mod.rs` | new | `PromptBlock`, `enum BlockKind` (fixed order), `PromptAssembler`; `render()` producing the ordered system string |
| `crates/engine/src/prompt/blocks.rs` | new | Embedded `00-core-identity` + 9 `instruction-blocks/*.md` via `include_str!`; per-block mode-tag table (matrix §3) |
| `crates/engine/resources/prompts/` | new | Vendored copies of `00`–`06` bodies + `instruction-blocks/*.md` (real data; single source for `include_str!`) |
| `crates/engine/src/mode/mod.rs` | new | `enum Mode { Plan, Edit, Agent }`, `enum Surface { Chat, Headless, Orchestration, None }`; `Mode::body()`, `Mode::instruction_blocks()`, `Mode::exposed_tools()`; `from_name`/`Display`; `UnknownMode` |
| `crates/engine/src/mode/gate.rs` | new | `ModeGate::filter_schema`, `ModeGate::check`, `struct Refusal`, `refusal_envelope(reason, extra)`; `promote(from, to)` core fn |
| `crates/engine/src/context.rs` | extend | `SystemContext::render` (68-83) reimplemented atop `PromptAssembler`; add `mode`/`surface` fields; keep public API stable |
| `crates/engine/src/environment.rs` | extend | Add `mode`, `surface`, `budgets`, `net` to `EnvironmentBlock` (55-68) + `render` (99-114) → `<runtime>` fields |
| `crates/engine/src/family.rs` | extend | `base_prompt` repositioned as the `CoreIdentity` model-family adapter (no behavior loss) |
| `crates/engine/src/agent.rs` | extend | `Agent` derived from `Mode` for built-ins; keep persona fragments; add `Mode → Agent` mapping helper |
| `crates/tools/src/registry.rs` | extend | `definitions()` (166-175) gains a mode-filtered variant `definitions_for(mode)` calling `ModeGate::filter_schema`; `execute()` (182-197) consults `ModeGate::check` and returns the refusal envelope |
| `crates/tools/src/tool.rs` | extend | Refusal envelope helper wired into `ToolContext` result path; unify with existing planning-mode refusal (669-702) |
| `crates/cli/src/cli.rs` | extend | `--mode` (1209-1211) documented against built-in `Mode` names; validation delegates to resolver (stays `Option<String>`) |
| `crates/cli/src/run.rs` | extend | `render_system_context` (980-1007) delegates to `PromptAssembler`; resolve `--mode` to `Mode`; pass mode into schema filter + runtime block |
| `crates/cli/src/custom_modes.rs` | extend | `ActiveMode.base_agent` maps to `Mode`; built-in modes resolvable; `resolve` (153-181) covers built-ins with `UnknownMode` |
| `crates/cli/src/permission.rs` | extend | `builtin_agent_permission_rules` (122-137) re-keyed from `Mode`; read-only/mutation rules become the execution backstop for the gate |
| `crates/tui/src/slash.rs` | extend | Add `/mode` (and `/plan`,`/edit`,`/agent`) to `BUILTINS` (19-184) |
| `crates/tui/src/app.rs` | extend | Dispatch `/mode`: list/switch/promote; reflect active mode in runtime status |

---

## 5. Milestones

Each milestone has machine-checkable acceptance criteria (a command whose exit status /
asserted output is the pass condition).

### M1 — Block registry + fixed order

Deliver `PromptBlock`/`BlockKind`/`PromptAssembler`; port `SystemContext::render` onto it
with zero output change for the default (Build/Edit-equivalent) path.

- **AC1.1** `cargo test -p engine prompt::assembler` green, including a block-order
  snapshot test asserting the emitted `BlockKind` sequence is exactly
  `[CoreIdentity, ModeBody, InstructionBlocks, RuntimeContext, TaskReminder, ToolCards]`.
- **AC1.2** A golden test asserts the pre-existing `SystemContext::render` output for the
  default agent is **byte-identical** before/after the port (regression lock).
- **AC1.3** `cargo build -p engine` exits 0; `cargo clippy -p engine -- -D warnings` clean.

### M2 — Mode bodies + surfaces as data

Deliver `Mode`/`Surface`, embedded bodies `00`–`06`, the selection table, and the
instruction-block injection matrix.

- **AC2.1** Test asserts `Mode::Plan.body()` contains the Plan Mode body and Plan Mode
  injects `plan-guidelines` + `tool-usage-rules` + `plan-note-guidelines` + `markdown-spec`
  and does **not** inject `persistence`/`coding-guidelines` (matrix `PROMPT-DISPATCH.md` §3).
- **AC2.2** Test asserts Edit Mode injects `persistence`+`cli-preference`+`coding-guidelines`
  +`design-guidelines`+`unstaged-changes`+`tool-usage-rules`+`markdown-spec` and **not**
  `plan-guidelines`.
- **AC2.3** Test asserts exactly one `ModeBody` block is present for any `(Mode, Surface)`,
  and that `Surface::Chat|Headless|Orchestration` appends its composing body without a
  second `ModeBody`.
- **AC2.4** `Mode::from_name("frobnicate")` returns `UnknownMode`; `from_name("plan")` →
  `Mode::Plan`. `cargo test -p engine mode::` green.

### M3 — MODE GATE enforcement + refusal envelope

Deliver `exposed_tools`, `filter_schema`, `check`, and the three-field refusal envelope;
wire into the tools registry/executor.

- **AC3.1** Test: `ModeGate::filter_schema(defs, Mode::Plan)` excludes `write`, `edit`,
  `apply_patch`, `bash`, `format`; includes `read`, `grep`, `glob`, `lsp`. Edit/Agent
  include the write set. Agent additionally includes `task`.
- **AC3.2** RED→GREEN: invoking `edit`/`write`/`bash`(writing) under Plan Mode via the
  registry executor returns exactly
  `{"status":"refused","payload":{"reason":"mode-not-permitted",…},"truncated":false}`
  (assert all three top-level keys, the enum value, and `reason`).
- **AC3.3** Test: the refusal envelope for a mode violation, a path violation, and a
  budget stop all share the identical three-field shape (one helper).
- **AC3.4** Test: `promote(Mode::Plan, Mode::Edit)` succeeds via the core fn; there is **no
  tool** named for self-promotion in any mode's `exposed_tools` (asserted by scanning the
  allowlists).
- **AC3.5** `cargo test -p engine mode::gate` and `cargo test -p tools registry` green.

### M4 — CLI / TUI / slash wiring

Deliver end-to-end mode selection and switching.

- **AC4.1** `cortex run --mode plan -p "edit README"` runs; the transcript shows the model
  offered a read-only tool set and any attempted write returns the refused envelope
  (captured to a transcript artifact). `--mode bogus` exits non-zero with an `UnknownMode`
  message.
- **AC4.2** `--json` output for a Plan-mode run that attempts a write contains a
  `status":"refused"` record (grep the JSON stream).
- **AC4.3** TUI `/mode` lists Plan/Edit/Agent and switches the active mode; the runtime
  status line shows `mode:` and the next turn's exposed tool set reflects the switch
  (asserted via a TUI-level test or captured session).
- **AC4.4** `cargo test --workspace` green; `cargo clippy --workspace -- -D warnings` clean.

---

## 6. TDD Plan

Test-first throughout: RED (failing test naming the behavior) → GREEN (smallest change) →
real surface artifact laddered against the Verification levels (`VERIFICATION.md` /
`GLOSSARY.md`): **L0 Deterministic** (compile/clippy/fmt), **L1 Agent-visible tests** (the
unit/integration tests below), **L2 Held-out** (perturbation checks the gate cannot game),
**L3 Intent audit** (reviewer pass, `plans/05-*`), **L4 Process check** (self-report vs
captured artifacts). Live-LLM tests are gated behind `OPENROUTER_API_KEY` and **skip when
absent — never mocked** (`MISSION-BRIEF.md` §0/§5: OpenRouter is the live reference; real
LLM/tools, fixtures allowed in tests only).

**RED cases (write first, watch fail for the right reason):**

1. `mode_gate_plan_refuses_write` — invoke `write`/`edit`/`bash` under `Mode::Plan` →
   expect `status == "refused"`, `payload.reason == "mode-not-permitted"`, `truncated ==
   false`. (Prototype example: "Plan mode invoking the `run`/write tool → `status:refused`.")
2. `block_order_snapshot` — assembler `BlockKind` sequence equals the fixed six-block order.
3. `unknown_mode_errors` — `Mode::from_name("frobnicate")` / `--mode frobnicate` →
   `UnknownMode` (no silent fallback).
4. `plan_mode_schema_excludes_mutators` — `filter_schema(defs, Plan)` omits the write set;
   Edit/Agent include it; Agent includes `task`.
5. `instruction_block_matrix_plan` / `_edit` — per-mode injected block sets match
   `PROMPT-DISPATCH.md` §3 exactly (positive + negative assertions).
6. `single_mode_body` — exactly one `ModeBody` for any `(Mode, Surface)`; Chat/Headless/
   Orchestration compose without a second body.
7. `runtime_block_has_mode_and_regenerates` — `<runtime>` render includes `mode`/`surface`/
   `budgets`/`net`; two renders across a mode switch differ in the `mode` field.
8. `refusal_envelope_shape_uniform` — mode / path / budget refusals share the identical
   three-field JSON shape.
9. `render_regression_golden` — default-path `SystemContext::render` byte-identical after
   the assembler port (M1 lock).
10. `no_self_promotion_tool` — no mode's `exposed_tools` contains a promotion tool;
    `promote()` is the only path.

**GREEN:** implement the minimal `PromptAssembler` / `Mode` table / `ModeGate` /
`EnvironmentBlock` fields to flip each test, one at a time; if a GREEN needs a large diff,
split the RED finer.

**Real-surface artifacts per milestone (the ceiling, in addition to L1):**

- M1/M2: `cargo test -p engine` output showing the snapshot + matrix tests green (L0+L1).
- M3: `cargo test -p tools registry` + a captured executor run printing the refused
  envelope JSON (L1 + real surface).
- M4: a captured `cortex run --mode plan …` transcript + the `--json` stream containing
  `status":"refused"` (real CLI surface); a TUI session capture of `/mode` switching (L1 +
  real surface). L2 held-out: a perturbation test that renames a tool and asserts the gate
  still excludes it by allowlist membership, not by string coincidence.
- Live gate (skip-if-absent): one `#[ignore]`-by-default or env-gated test that, with
  `OPENROUTER_API_KEY` set, runs a Plan-mode turn against OpenRouter and asserts the model
  never receives a mutating tool schema (schema captured from the request) — real LLM, no
  mock.

---

## 7. Wave/Dependency Position

- **Upstream dependency:** none inside `plans/`; this doc defines the substrate. It depends
  only on the existing engine/tools/permissions crates.
- **Wave:** foundational — schedule in the **first wave** with `plans/00-*` (index). M1→M2→M3
  are strictly sequential (assembler before bodies before gate); M4 depends on M2+M3.
- **Downstream consumers (must land after M3):**
  - `plans/02-*` (7-step loop) consumes `ModeGate` at its `gate` phase and fills the
    `TaskReminder` + `ToolCards` slots this doc leaves open.
  - `plans/06-*` (Subagents/Skills) consumes Agent-mode `exposed_tools` (the `task`/subagent
    grant) and the instruction-block registry.
  - `plans/05-*` (Reviewers/Verification) consumes the Verification-level mapping used in §6
    and the refusal-envelope shape for its audits.
  - `plans/08-*` (CLI UI/UX) builds richer mode pickers atop the `/mode` command from M4.
- **Parallelizable with:** `plans/04-*` (concurrency) and `plans/07-*` (intent scorer),
  which do not touch the prompt/mode substrate.

---

## 8. Risks & Open Questions

1. **Mode vs Agent overlap (highest).** The codebase already ships `engine::Agent`
   (Build/Plan/General/Explore) and `custom_modes::ActiveMode`. Introducing `Mode` risks a
   third overlapping concept. *Mitigation:* `Mode` is the single authority; `Agent` derives
   from it for built-ins and remains only the persona-fragment layer; custom overlays carry
   a `Mode`. Decision recorded in §2.2 — confirm with `plans/00-*` for cross-doc consistency.
2. **Tool-name divergence (open question).** `TOOLS.md` uses `find/grep_code/read_span/
   list_tree/run/edit_span/write_file/web_lookup/plan_note/spawn_subagent`; the registry
   uses `read/grep/glob/lsp/write/edit/apply_patch/bash/webfetch/websearch/plan/task`. This
   plan keys the gate on the **current registry names** and records the mapping. **OQ:** is a
   full tool rename to the `TOOLS.md` set owned by another doc (tool layer)? If yes, the gate
   allowlists must be updated in lockstep; the allowlist is the single change point.
3. **Schema filter vs permission engine duplication.** Two enforcement layers (schema
   exposure + permission runtime) could drift. *Mitigation:* both derive from `Mode`; the
   permission layer covers only *exposed-but-gated* tools (`run` read-only in Plan, net,
   paths); a test asserts every mutator excluded by the gate is also hard-denied by
   permissions (belt-and-suspenders, no contradiction).
4. **Golden-output regression fragility (M1).** Byte-identical assertion may fight
   whitespace. *Mitigation:* port `render` as a pure re-expression first (M1), only then add
   blocks (M2) so the golden lock is meaningful.
5. **Budget fields ownership.** `<runtime>` budgets are filled by the loop (`plans/02-*`).
   This doc renders observable placeholders; **OQ:** exact budget ceiling defaults per mode
   — deferred to `plans/02-*`.
6. **Promotion UX.** Whether promotion is a slash command, a permission prompt, or both is a
   UI question shared with `plans/08-*`; §2.5 fixes only the core-granted `promote()`
   semantics, not the surface.

---

## 9. Commit Sequence

Atomic commits, each independently building + testing green (L0+L1), authored per the
repository commit-identity policy.

1. `engine: add PromptBlock/BlockKind ordered assembler (M1)` — new `prompt/` module +
   block-order snapshot test; no call-site change yet.
2. `engine: port SystemContext::render onto PromptAssembler (M1)` — reimplement `render`
   (context.rs:68-83) atop the assembler + byte-identical golden regression test.
3. `engine: vendor prompt bodies 00–06 + instruction-blocks resources (M2)` — resources dir
   + `include_str!` wiring; no logic change.
4. `engine: add Mode/Surface enums + body selection table (M2)` — `mode/mod.rs`,
   `from_name`/`UnknownMode`, single-body test.
5. `engine: inject per-mode instruction blocks by matrix (M2)` — block-tag registry +
   matrix tests (plan/edit).
6. `engine: extend EnvironmentBlock with runtime mode/surface/budgets/net (M2)` — fields +
   render + regeneration test.
7. `engine: add ModeGate schema filter + exposed_tools allowlists (M3)` — `mode/gate.rs`,
   `filter_schema` tests.
8. `engine,tools: refusal envelope + ModeGate::check in executor (M3)` — shared
   `refusal_envelope`, registry `execute` integration, RED→GREEN Plan-refuses-write.
9. `engine: core-granted promote() + no-self-promotion test (M3)`.
10. `cli: resolve --mode to Mode, drive assembler + schema filter (M4)` — `run.rs`
    `render_system_context` delegation; `custom_modes`/`permission.rs` re-keyed to `Mode`.
11. `tui: add /mode slash command + dispatch and runtime status (M4)` — `slash.rs` +
    `app.rs`; TUI switch test.
12. `test: env-gated live Plan-mode schema assertion via OpenRouter (M4)` — skip-if-absent,
    no mock.
