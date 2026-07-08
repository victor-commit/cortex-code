# 06 — Subagents & Skills: Nested Delegation, Scoped Capability Inheritance, Skill Gating

> Planning doc. Produces no product code by itself. Implementation lands via the commit
> sequence in §9. All terminology is drawn from `docs/rename/naming-map.md` and `GLOSSARY.md`;
> no new terms are coined here.

## 0. Scope & Prototype Contracts

### 0.1 What this doc plans

Three coupled capabilities on top of the existing single-level `task` delegation:

1. **Nested delegation** — a worker (subagent instance) may itself call the `task` tool to
   spawn a further worker, **only when the root turn is in Agent Mode**, under a depth-bounded
   budget. Today this is categorically forbidden (see §1).
2. **Scoped capability inheritance** — a spawned worker inherits a *narrowed* slice of the
   parent's capabilities (MCP server-qualified tools, Skills), never a wider one, per the
   worker contract's `tool_whitelist` / `forbidden_actions`. Today a worker inherits *none*
   of MCP/Skills/websearch.
3. **Skill gating** — Skills are dispatched lazily by trigger and `mode-availability`, and
   their `tools` whitelist restricts (never widens) the surface of whatever mode/worker is
   running.

### 0.2 Prototype sources (authoritative; cited, not copied)

| Contract | Source | What it binds |
|---|---|---|
| Off by default; 14 failure modes; single-agent default | `SUBAGENTS.md` §1 (F10/P12 · arXiv:2503.13657, arXiv:2511.15755) | Nesting/parallelism must be *earned*, not default-on |
| The worker contract | `SUBAGENTS.md` §2 (F10 · arXiv:2503.13657) | `task`, `expected_artifact`, `tool_whitelist`, `forbidden_actions`, `context_slice`, `budget`, `return_to` — enforced by the harness, not advisory |
| Deterministic coordination | `SUBAGENTS.md` §3 (F10/P12) | The core schedules and merges; no "manager" model. Handoffs pass through the core |
| Sanctioned use = independent verifier | `SUBAGENTS.md` §4 (arXiv:2601.17223, arXiv:2603.23448) | The one clearly-supported nested use is a reviewer that did not author the change |
| Typed communication contracts | `SUBAGENTS.md` §5 (arXiv:2601.13671) | Worker↔worker handoffs are typed artifacts, validated before use — never free-form chat |
| Skills = scoped, lazy, tool-narrowing | `SKILLS.md` §1, §2, §4 (F1/P6, F7/P8 · arXiv:2604.13120, arXiv:2405.15793) | Frontmatter `name` / `when-to-activate` / `tools` / `mode-availability`; core owns exposure; `tools` only *restricts* |
| Named worker roles | `prompts/subagents/{explorer,general-purpose,specialized,config-generator,diff-summarizer}.md` | `specialized` is a leaf (no recursion); `explorer` read-only; `general-purpose` r/w/run |
| Canonical vocabulary | `GLOSSARY.md`, `docs/rename/naming-map.md` | Agent Mode, Subagents, worker, general-purpose, Explorer, Diff Summarizer, Config Generator, specialized, Reviewers, Skills, `spawn_subagent` |

### 0.3 Prototype behavioral contracts this plan MUST satisfy

- **C-NEST-1** — Nested `task` is available **only** when the *root* operating mode is Agent
  Mode (`SUBAGENTS.md` §1; `GLOSSARY.md`: "Agent Mode … may spawn Subagents"). In Plan / Edit
  / Chat Mode the `task` tool is not advertised at any level.
- **C-NEST-2** — Every nested spawn is depth-bounded and step/wall-clock-bounded per the
  worker contract `budget` (`SUBAGENTS.md` §2). Recursion terminates deterministically.
- **C-NEST-3** — A `specialized` worker is a **leaf**: it may never spawn (`prompts/subagents/specialized.md`).
- **C-CAP-1** — A child's capability set is the **most-restrictive** intersection of
  (parent capabilities) ∩ (worker contract `tool_whitelist`) ∩ (per-agent rules). A child can
  never exceed its parent — mirrors existing permission `parent_cap` narrowing (§1).
- **C-CAP-2** — MCP tools and Skills a worker receives are an *explicit, narrowed subset*, not
  the whole parent surface (`SUBAGENTS.md` §2 `context_slice`; `SKILLS.md` §1 "narrow tools").
- **C-COORD-1** — Coordination is deterministic and core-owned; no worker overrides another,
  nothing is dropped in a handoff gap (`SUBAGENTS.md` §3, §5).
- **C-SKILL-1** — A Skill loads only when `when-to-activate` matches **and** the active mode is
  in `mode-availability`; its `tools` whitelist restricts the running surface for as long as the
  trigger holds, then is dropped (`SKILLS.md` §4).

### 0.4 Non-goals (deferred to sibling docs)

- Full Workflow/Team orchestration, waves, worktree isolation, merge-back — **doc 03**
  (workflow workers) and Team docs. This doc provides the nesting + capability primitives
  those build on.
- Reviewer role prompt content and the independent-verifier flow — **doc 05** (reviewers),
  which *depends on* the nesting primitive defined here (the reviewer is a nested worker).
- The Agent-Mode gate mechanism itself — **doc 01** (mode gate), a **dependency** of this doc
  (C-NEST-1 consumes it).
- The agentic loop / step budget engine — **doc 02** (loop).

## 1. Current State (file:line)

All paths under `crates/`. Line numbers verified against the tree at authoring time.

### 1.1 Nesting is categorically forbidden

- `subagent_runner/src/bridges.rs:28-31` — `ChildDispatcher` doc comment: *"deliberately omits
  MCP, skills, websearch, and nested `task` delegation, so a child subagent cannot recursively
  spawn further subagents."*
- `subagent_runner/src/bridges.rs:32-56` — `ChildDispatcher` struct + `for_model`. Builder
  methods present: `with_audit`, `with_permissions`, `with_checkpoints`, `with_hooks`,
  `with_lsp_service` (67-80+). **Absent:** any `with_skills`, `with_mcp`, `with_task_runner`.
- `subagent_runner/src/runner.rs:350-356` — the child dispatcher build site: `ChildDispatcher::for_model(...).with_audit(...)`,
  then permissions/checkpoints/hooks/lsp are layered (358-390). No task-runner, MCP, or skills
  are ever attached to a child.

### 1.2 Parent turn wires the full surface (the asymmetry)

- `cli/src/run.rs:121-125` — `skills::discover(&config_inputs)` → `discovered_skill_list`.
- `cli/src/run.rs:643-647` — `parent_permission_cap = engine.ruleset().clone()`;
  `task_catalog = permitted_catalog(&discovered_agents, permission_engine)`.
- `cli/src/run.rs:657-659` — `dispatcher.with_skills(discovered_skill_list.clone())`.
- `cli/src/run.rs:688-691` — `dispatcher.with_mcp(manager)` (server-qualified MCP tools).
- `cli/src/run.rs:692-718` — builds `CliTaskRunner` via `task_catalog.runner(CliTaskRunnerConfig{…})`
  then `dispatcher.with_task_runner(task_catalog.targets, runner)`. **Only the parent gets a task runner.**
- `cli/src/run.rs:789-791` — `runner.drain_background()` at shutdown.
- `tui/src/turn.rs:575-590` — the TUI mirrors this exact wiring (`permitted_catalog` + `CliTaskRunner`).

### 1.3 Capability-inheritance building blocks that already exist

- `subagent_runner/src/runner.rs:31-48` — `CliTaskRunnerConfig` fields: `provider`,
  `provider_audit`, `plugin_provider`, `audit`, `db_path`, `cwd`, `parent_session_id`,
  `parent_cap: Option<Ruleset>`, `parent_model_id`, `formatter_config`, `lsp_config`,
  `config_inputs`, `project_memory`, `bus`. **No depth field, no MCP handle, no skills list.**
- `subagent_runner/src/runner.rs:487-518` — `child_permissions`: composes builtin base rules +
  per-agent config ruleset (`load_agent_rulesets(...).for_agent(name)`) + agent's own rules +
  builtin restriction rules + trust ruleset, then `PermissionEngine::new(ruleset).with_parent_cap(parent_cap)`
  (514-516). **This is the existing most-restrictive parent-cap narrowing** (the model for C-CAP-1).
- `subagent_runner/src/catalog.rs:44-70` — `permitted_catalog`: offers `engine::Agent::SUBAGENTS`
  (builtin) + `discovery.agents` where `availability.spawnable()`, each filtered by
  `task_allowed(...)` = `classify("task", name) == Allow` (72-73).
- `subagent_runner/src/catalog.rs:28-34` — `runner()` constructs `CliTaskRunner` with empty
  `background_tasks`.
- `engine/src/agent.rs` — `Agent { Build, Plan, General, Explore }`; `SUBAGENTS: [Agent;3] = [Plan, General, Explore]`
  (Build excluded as primary); `is_subagent()` ~line 67.
- `tools/src/task.rs` — `TaskTool` / `TaskRequest { prompt, task_id, background, description }`;
  permission-gated by `task:<agent>`. **No `depth` / recursion field.**
- `tools/src/skill.rs` — `SkillTool` built from discovered `Skill`s; advertises names; returns
  guidance on engage; `FILE_LIMIT = 24`.
- `permissions/src/agent.rs` — `AgentRulesets`, `load_agent_rulesets`, `for_agent` (per-agent
  override rules).
- `skills/src/lib.rs:229-238` — builtin skill dirs (`…/cortex/skills`, `~/.claude/skills`,
  `~/.agents/skills`); `240-255` `configured_skill_dirs` (`skill_dirs`, `skills`, `skill.{dirs,directories,path,paths}`);
  `257-280` `configured_skill_urls`.

### 1.4 What the current state means

The engine already enforces **most-restrictive permission narrowing** parent→child
(`with_parent_cap`), but the *tool/capability surface* is all-or-nothing per surface:
**parent = everything, child = built-ins only, grandchild = impossible.** There is no depth
budget because depth is hard-capped at 1. Skills and MCP simply do not cross the parent→child
boundary. This plan converts the hard cap into a **contracted, depth-bounded, mode-gated,
capability-narrowing** delegation.

## 2. Target State

### 2.1 Nested delegation (Agent Mode only)

- The `task` tool is advertised to a worker **iff** the root operating mode is Agent Mode
  (C-NEST-1) **and** the current depth `< max_depth`. `ChildDispatcher` gains an optional
  task-runner attachment; when depth is exhausted or the worker is `specialized`, no task
  runner is attached and `task` is absent from that worker's tool set.
- `CliTaskRunnerConfig` gains `depth: u32` and `max_depth: u32` (default from config,
  conservative — see §8 R-1). Each nested spawn constructs the child config with
  `depth = parent.depth + 1`; `run` refuses to spawn when `depth >= max_depth`, returning a
  typed, non-fatal `ToolOutput` the parent worker can act on (C-NEST-2).
- `specialized` workers never receive a task runner regardless of depth (C-NEST-3).

### 2.2 Scoped capability inheritance

- `CliTaskRunnerConfig` carries the parent's **capability handles**: an `Option<Arc<McpManager>>`
  and the parent's `Vec<Skill>` (or a resolved handle), *plus* the worker contract's
  `tool_whitelist` / `forbidden_actions` slice.
- When building the child dispatcher, the runner attaches a **narrowed** MCP view and a
  **narrowed** skills list: `child_caps = parent_caps ∩ contract.tool_whitelist \ contract.forbidden_actions`.
  The narrowing is applied *before* advertisement, so an omitted capability is physically absent,
  not merely discouraged (C-CAP-1, C-CAP-2; mirrors `SUBAGENTS.md` §2 "physically lacks any
  capability not granted").
- Permission narrowing continues to run underneath via the existing `with_parent_cap` path
  (`runner.rs:514-516`); capability-surface narrowing and rule narrowing are **both** enforced and
  compose to most-restrictive.

### 2.3 Skill gating

- Skill dispatch honors `mode-availability` and `when-to-activate` (C-SKILL-1). A worker sees a
  Skill only if the Skill is available in the *effective* mode of that worker and the Skill's
  `tools` whitelist is intersected with the worker's already-narrowed tool surface (never widened).
- The `SkillTool` surface passed to a child is filtered by the same `child_caps` intersection.

### 2.4 Deterministic, typed coordination

- Handoffs continue to pass through the core/runner (existing `background_tasks` map,
  `drain_background`, rejoin-by-id). Nested results return to the spawning worker as a typed
  `ToolOutput` artifact; the worker contract `expected_artifact.shape` is recorded so a
  malformed/missing handoff is detectable, not silent (C-COORD-1; `SUBAGENTS.md` §5).

## 3. Gap Analysis

| # | Contract | Current | Gap to close |
|---|---|---|---|
| G1 | C-NEST-1 (Agent-Mode-only nesting) | `task` never reaches a child (`bridges.rs:28-31`) | Attach an optional task runner to `ChildDispatcher`, gated on root=Agent Mode (consumes doc 01) |
| G2 | C-NEST-2 (depth + budget bound) | Depth hard-capped at 1; no `depth` field (`runner.rs:31-48`) | Add `depth`/`max_depth` to config; refuse spawn at ceiling; typed non-fatal refusal |
| G3 | C-NEST-3 (`specialized` is a leaf) | N/A (nothing nests) | Suppress task runner for `specialized` role |
| G4 | C-CAP-1 / C-CAP-2 (narrowed MCP) | Child gets no MCP (`bridges.rs:28-31`) | Thread `Option<Arc<McpManager>>` into config; attach narrowed MCP view to child |
| G5 | C-CAP-1 / C-CAP-2 (narrowed Skills) | Child gets no Skills | Thread parent Skills into config; attach `child_caps`-filtered `SkillTool` |
| G6 | C-CAP-1 (most-restrictive intersection) | Permission narrowing exists (`runner.rs:514-516`); capability narrowing does not | Add capability-surface intersection alongside existing rule narrowing |
| G7 | C-SKILL-1 (mode-availability + tools restrict) | `SkillTool` advertises discovered skills without mode/worker filtering | Filter Skills by effective mode + intersect `tools` with worker surface |
| G8 | C-COORD-1 (typed handoff, nothing dropped) | Single-level handoff via `background_tasks` works | Preserve for nested; record `expected_artifact.shape` for validation |
| G9 | Parity across surfaces | Only CLI + TUI wire the runner | Ensure nested config flows identically in `cli/src/run.rs` and `tui/src/turn.rs` |

## 4. Crate/File Change Map

| File | New / Extend | What changes |
|---|---|---|
| `crates/subagent_runner/src/runner.rs` | Extend | Add `depth`/`max_depth` + capability handles (`mcp: Option<Arc<McpManager>>`, `skills: Vec<Skill>`, `contract_slice`) to `CliTaskRunnerConfig` (31-48). In `run`/`run_foreground` (74-87, 330-390): refuse when `depth >= max_depth`; when allowed and root=Agent Mode and role≠`specialized`, build a **child** `CliTaskRunner` (config with `depth+1`, narrowed caps) and attach it to the child `ChildDispatcher`. Compute `child_caps` intersection alongside existing `child_permissions` (487-518). |
| `crates/subagent_runner/src/bridges.rs` | Extend | Update `ChildDispatcher` doc (28-31) to reflect conditional nesting. Add builder methods `with_task_runner`, `with_mcp`, `with_skills` that layer onto `ToolContext`/registry exactly as the parent `RegistryDispatcher` does, but only when caps permit. |
| `crates/subagent_runner/src/catalog.rs` | Extend | `permitted_catalog` gains awareness of depth/mode so a child catalog is empty at the leaf; `runner()` (28-34) threads new config fields. |
| `crates/subagent_runner/src/custom_agents.rs` | Extend | Expose whether a role is a leaf (`specialized`) so the runner can suppress its task runner (C-NEST-3). |
| `crates/subagent_runner/src/lib.rs` | Extend | Re-export any new public config/handle types used by CLI/TUI. |
| `crates/tools/src/task.rs` | Extend | `TaskRequest`/`TaskTarget` carry the worker-contract slice (`tool_whitelist`, `forbidden_actions`, `expected_artifact.shape`, `budget`) so the runner can enforce C-CAP-2 and record C-COORD-1. Gate advertisement on Agent Mode + depth. |
| `crates/tools/src/skill.rs` | Extend | `SkillTool` accepts an effective-mode + tool-surface filter so Skills are gated by `mode-availability` and their `tools` whitelist intersects the running surface (C-SKILL-1). |
| `crates/skills/src/lib.rs` | Extend (maybe) | Ensure discovered `Skill` carries `mode-availability` + `tools` metadata needed for gating (frontmatter per `SKILLS.md` §2). Add parsing only if absent. |
| `crates/permissions/src/agent.rs` | Verify / minimal | Confirm `for_agent` + `parent_cap` compose for grandchild depth; add a helper if capability-surface intersection needs a rule-side counterpart. No behavior change unless a gap is found. |
| `crates/engine/src/agent.rs` | Verify | Confirm `SUBAGENTS` / `is_subagent()` semantics hold for a nested spawner; no persona change expected. |
| `crates/cli/src/run.rs` | Extend | Populate the new `CliTaskRunnerConfig` fields (depth=0 at root, `max_depth` from config, MCP handle from `mcp_manager` 688-691, skills from `discovered_skill_list` 657-659, root mode). |
| `crates/tui/src/turn.rs` | Extend | Mirror the CLI changes (575-590) so nested delegation behaves identically in the TUI. |
| `crates/*/config` (settings) | Extend | Add `subagents.max_depth` (and optional `subagents.nested_enabled`) with conservative defaults; documented in config reference. |

> No file outside this map is edited. This doc edits **no product code at all** — it only
> specifies the map.

## 5. Milestones

Each milestone ends with a machine-checkable acceptance gate (a command + a binary observable).

- **M1 — Config + depth plumbing (no behavior yet).**
  Add `depth`/`max_depth` + capability-handle fields to `CliTaskRunnerConfig`; populate from
  `cli/src/run.rs` and `tui/src/turn.rs`. Default `max_depth` conservative; nested still disabled.
  **Accept:** `cargo build -p subagent_runner -p cli -p tui` exits 0; a unit test asserts a
  root runner has `depth == 0` and `max_depth == <configured>`.

- **M2 — Agent-Mode gate for `task` advertisement.**
  `task` is advertised to a worker only when root mode is Agent Mode and `depth < max_depth`.
  **Accept:** unit tests — in Agent Mode a depth-0 child's tool set contains `task`; in Plan/Edit/Chat
  Mode it does not; at `depth == max_depth` it does not. `cargo test -p subagent_runner nested_gate` green.

- **M3 — Nested spawn executes and terminates.**
  A worker in Agent Mode spawns a grandchild; the runner builds a child `CliTaskRunner` with
  `depth+1`; a spawn at the ceiling returns a typed non-fatal refusal (not a panic/hang).
  **Accept:** `cargo test -p subagent_runner nested_depth_terminates` green; test proves a
  3-level chain stops at `max_depth` and the refusal `ToolOutput` is well-formed.

- **M4 — `specialized` is a leaf.**
  A `specialized` worker never receives a task runner.
  **Accept:** `cargo test -p subagent_runner specialized_is_leaf` green (its advertised tools
  never include `task`, at any depth).

- **M5 — Scoped MCP inheritance.**
  Child receives a `tool_whitelist`-narrowed MCP view; a server-qualified tool omitted by the
  contract is physically absent from the child.
  **Accept:** `cargo test -p subagent_runner mcp_narrowed` green — child advertises only the
  whitelisted MCP tools; asserts `parent_tools ⊇ child_tools` and forbidden tools absent.

- **M6 — Scoped Skill inheritance + gating.**
  Skills are filtered by `mode-availability`; a Skill's `tools` intersect the child surface;
  a non-whitelisted Skill is absent.
  **Accept:** `cargo test -p tools skill_mode_gate` + `cargo test -p subagent_runner skill_narrowed`
  green.

- **M7 — Most-restrictive composition proof.**
  Capability-surface narrowing and existing permission `parent_cap` rule-narrowing compose so a
  grandchild never exceeds its parent on either axis.
  **Accept:** `cargo test -p subagent_runner most_restrictive_compose` green (grandchild caps ⊆
  child caps ⊆ parent caps, and rules likewise).

- **M8 — Surface parity + live end-to-end (gated).**
  CLI and TUI wire identical nested behavior; a live Agent-Mode run performs one sanctioned
  nested delegation (independent verifier shape).
  **Accept:** L3/L4 real-surface artifacts captured (see §6); `cargo test --workspace` green;
  `cargo clippy --workspace -- -D warnings` clean.

## 6. TDD Plan (RED → GREEN → real-surface L0–L4)

**Discipline:** every behavior change is preceded by a failing test that fails for the *right*
reason. Live-LLM tests are **gated behind an OpenRouter key and skipped when absent — never
mocked** (a missing key skips; it does not fake a pass). Non-LLM unit tests use in-repo fixtures
(fixtures are allowed; user-facing mocks are not).

### RED→GREEN unit layer (no network)

| Test id | RED asserts | GREEN when |
|---|---|---|
| `nested_gate::task_absent_outside_agent_mode` | child in Edit Mode advertises `task` → fail | gate suppresses `task` unless root=Agent Mode (M2) |
| `nested_gate::task_present_in_agent_mode` | child in Agent Mode lacks `task` → fail | task runner attached at depth<max (M2) |
| `nested_depth::refuse_at_ceiling` | spawn at `depth==max_depth` panics/hangs → fail | typed non-fatal refusal returned (M3) |
| `nested_depth::terminates` | 3-level chain recurses unbounded → fail | chain stops at `max_depth` (M3) |
| `specialized_is_leaf::no_task_runner` | `specialized` advertises `task` → fail | leaf suppression (M4) |
| `mcp_narrowed::forbidden_absent` | child sees a non-whitelisted MCP tool → fail | `child_caps` intersection applied (M5) |
| `skill_mode_gate::unavailable_mode_hidden` | Skill shows in a mode absent from `mode-availability` → fail | mode filter applied (M6) |
| `skill_narrowed::tools_intersect` | Skill widens the child tool surface → fail | `tools` intersect not union (M6) |
| `most_restrictive_compose::grandchild_subset` | grandchild caps ⊄ parent → fail | double narrowing composes (M7) |

### Real-surface levels

- **L0 build** — `cargo build --workspace` exit 0.
- **L1 lint/type** — `cargo clippy --workspace -- -D warnings` clean on changed crates; no new warnings.
- **L2 unit/integration** — the RED→GREEN table above, `cargo test --workspace` green.
- **L3 CLI real surface** — headless `cortex run` in Agent Mode with a prompt that legitimately
  earns one nested delegation; capture stdout/JSONL transcript showing: parent `task` call →
  child turn → child's own `task` call → grandchild turn → typed artifact returned →
  `drain_background` clean shutdown. Capture the transcript file path. Repeat with a
  Plan-Mode run and assert **no** `task` tool is offered.
- **L4 TUI real surface** — drive the TUI (`turn.rs` path) through the same nested delegation via
  the QA harness; capture the session transcript proving parity with L3. Confirm no orphaned
  child sessions and all nested transcripts are rejoinable by id.

**L3/L4 LLM gating:** these require a live provider. Guard with the OpenRouter key env; when the
key is absent the test harness **skips** L3/L4 and records `SKIPPED (no OPENROUTER key)` — it
must never substitute a mock provider to manufacture a green.

**Evidence captured per milestone:** RED output + GREEN output (unit) **and** the L3/L4
transcript artifact path (integration). Both are required before a milestone is "done".

## 7. Wave / Dependency Position

```
doc 01 (mode gate) ──────────────┐  (provides Agent-Mode signal → C-NEST-1)
                                  ▼
doc 02 (loop) ────────────►  06 THIS DOC (subagents & skills primitives)
                                  │
                     ┌────────────┼─────────────┐
                     ▼            ▼              ▼
               doc 03        doc 05         (Workflow/Team,
            (workflow        (reviewers      later docs)
             workers)         = nested
                              verifier)
```

- **Depends on:** doc 01 (mode gate — the Agent-Mode boolean C-NEST-1 consumes) and doc 02
  (loop — the bounded step engine the `budget` ceiling rides on).
- **Depended on by:** doc 03 (workflow workers spawn via this nesting primitive) and doc 05
  (the sanctioned independent-verifier reviewer is a *nested worker* built on this doc).
- **Wave:** implementable immediately after 01/02 land; 03 and 05 must not start their
  spawn-path work until M3–M7 here are green.

## 8. Risks & Open Questions

### 8.1 The 14 multi-agent failure modes (SUBAGENTS.md §1 — arXiv:2503.13657 MAST) and how this design addresses each

The prototype explicitly grounds "off by default" in the fourteen-mode failure taxonomy. Each
mode below is named with the concrete mechanism in this plan that mitigates it. This is a binding
design checklist, not commentary.

**Category A — Specification & system design**
1. **Disobey task specification** → worker contract `task` + `expected_artifact.shape` recorded and
   validated on return (§2.4, C-COORD-1); malformed artifact is detectable.
2. **Disobey role specification** → per-role capability narrowing; `specialized` is a hard leaf
   (C-NEST-3, M4); a role physically lacks tools outside its whitelist (C-CAP-2).
3. **Step repetition** → deterministic core-owned scheduling (C-COORD-1); depth budget prevents
   re-spawn loops (M3).
4. **Loss of conversation history** → `context_slice` is an explicit, recorded input; child
   sessions are durable and rejoinable by id (existing `background_tasks`/`drain_background`).
5. **Unaware of termination conditions** → `budget` (steps/wall-clock) + `max_depth` ceiling with a
   typed refusal at the edge (C-NEST-2, M3).

**Category B — Inter-agent misalignment**
6. **Conversation reset** → handoffs pass through the core, not a re-initialized side channel
   (C-COORD-1).
7. **Fail to ask for clarification** → under-specified spawn is rejected by the contract's required
   fields (`SUBAGENTS.md` §2: a worker is "never spawned with a vague instruction").
8. **Task derailment** → narrowed tool/skill surface bounds what a worker *can* do to its contract
   (C-CAP-1/2).
9. **Information withholding** → typed `expected_artifact` return; the spawner validates shape
   before acting (§2.4, `SUBAGENTS.md` §5).
10. **Ignored other agent's input** → deterministic merge by the core under fixed rules; no worker
    silently overrides another (`SUBAGENTS.md` §3).
11. **Reasoning–action mismatch** → the harness enforces the whitelist regardless of the model's
    stated intent (capability is physical, not advisory).

**Category C — Task verification & termination**
12. **Premature termination** → `drain_background` guarantees in-flight nested turns complete and
    persist before the root exits (`cli/src/run.rs:789-791`).
13. **No / incomplete verification** → the sanctioned nested use is precisely the independent
    verifier (doc 05); L3/L4 evidence gates each milestone (§6).
14. **Incorrect verification** → the verifier is a *separate* agent that did not author the change
    (`SUBAGENTS.md` §4), removing the self-review incentive.

### 8.2 Risks

- **R-1 — Depth blow-up / cost.** Even bounded recursion multiplies token cost. *Mitigation:*
  conservative default `max_depth` (recommend 2, i.e. root→worker→leaf) and `subagents.nested_enabled`
  defaulting off; nesting is opt-in per `SUBAGENTS.md` §1.
- **R-2 — Capability-narrowing bugs that *widen* a child.** A logic error could grant a child a tool
  the parent lacks. *Mitigation:* M7 subset-assertion tests on both axes; capability narrowing runs
  *before* advertisement so the failure mode is "too few", not "too many".
- **R-3 — Surface drift** between `cli/src/run.rs` and `tui/src/turn.rs`. *Mitigation:* the config
  struct is the single source of truth; M8 parity test drives both paths through the same scenario.
- **R-4 — Skill frontmatter gaps.** If discovered `Skill`s don't currently carry `mode-availability`
  / `tools`, gating can't be enforced. *Mitigation:* M6 depends on confirming/adding that parsing in
  `skills/src/lib.rs` first (Crate map).

### 8.3 Open questions

- **Q-1** — Does doc 01 expose the *root* operating mode to the runner, or only the current turn's
  mode? C-NEST-1 needs the **root** mode. Confirm the signal shape with doc 01.
- **Q-2** — Should `max_depth` be a global config, per-agent frontmatter, or both? Leaning global
  with optional per-agent lower override (never higher — most-restrictive).
- **Q-3** — Is the MCP manager cheaply clonable/narrowable, or does a narrowed view need a wrapper?
  Determines whether G4 is a thread-through or a new adapter type in `mcp/`.
- **Q-4** — Typed handoff validation (C-COORD-1): validate `expected_artifact.shape` structurally
  now, or defer strict schema validation to doc 05? Proposal: record shape now, enforce in 05.

## 9. Commit Sequence

Each commit is independently green (build + relevant tests) and maps to milestones in §5.

1. **`subagents: add depth + capability-handle fields to CliTaskRunnerConfig`** — struct fields,
   root population in `cli/src/run.rs` + `tui/src/turn.rs`, defaults. (M1) — no behavior change.
2. **`subagents: gate task advertisement on Agent Mode + depth`** — `ChildDispatcher.with_task_runner`,
   advertisement gate, RED→GREEN `nested_gate` tests. (M2)
3. **`subagents: bounded nested spawn with typed ceiling refusal`** — child runner construction at
   `depth+1`, refusal at `max_depth`, `nested_depth` tests. (M3)
4. **`subagents: specialized worker is a non-spawning leaf`** — leaf suppression + `specialized_is_leaf`
   test. (M4)
5. **`subagents: inherit narrowed MCP view into child`** — thread MCP handle, `child_caps` intersection,
   `mcp_narrowed` test. (M5)
6. **`skills: gate by mode-availability and intersect tools; inherit narrowed skills`** — `skill.rs`
   filter, `skills/src/lib.rs` frontmatter parsing if needed, `skill_mode_gate` + `skill_narrowed`
   tests. (M6)
7. **`subagents: prove most-restrictive composition parent→child→grandchild`** — `most_restrictive_compose`
   test; any permission helper in `permissions/src/agent.rs`. (M7)
8. **`subagents: CLI/TUI parity + live nested delegation evidence`** — parity wiring, L3/L4 harness
   scenarios (key-gated), captured transcripts, config docs. (M8)
9. **`docs: config reference for subagents.max_depth / nested_enabled`** — settings documentation
   only.

> Commit identity for all commits follows the repo's commit-identity rule.
