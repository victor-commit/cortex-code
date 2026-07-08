# 03 — Workflow, Team & Task Board

> Design doc only. No product code. Maps the prototype's multi-agent **Workflow** (long-running, plan-driven build), **Team** (persistent Coordinator + claiming Workers), and **Task Board** (shared, core-owned work queue) onto the real Rust tree.
>
> **Naming**: this doc uses only professional-generic names — **Workflow**, **Team**, **Task Board**, **Coordinator**, **Worker**, **Orchestrator**. The frozen prototype terms are noted once here for migration traceability and never used as live names: *Voyage → Workflow*, *Fleet → Team*, *deckhand → Worker*.

---

## 0. Scope & Contracts

**In scope.** A design for three cooperating capabilities that sit *above* the single-turn engine loop and the single-level subagent dispatcher:

1. **Workflow** — a durable, plan-driven build session. An Orchestrator plans a dependency-ordered feature graph; Workers each take one feature, run a bounded micro-loop, and return a typed result; milestones validate against an acceptance contract.
2. **Team** — a persistent Coordinator that plans, posts, and reconciles work but never edits or runs code, plus Workers that claim exactly one Task Board item at a time.
3. **Task Board** — a shared, **core-owned** queue whose item state transitions are deterministic (moved by the core, not by any model).

**Out of scope (owned by sibling docs).**
- Nested/parallel subagent execution and worktree isolation → `04-concurrency.md`.
- Reviewer gating and evidence/verification model → `05-reviewers-verification.md`.
- Subagent persona catalog, skills, and dispatch mechanics → `06-subagents-skills.md`.
- The 7-step per-turn loop itself → `02-engine-7step-loop.md`.

**Contracts (binding).**

| # | Contract | Enforced where (target) |
|---|----------|-------------------------|
| C1 | A stop is always honest; completion is never fabricated. Every Workflow/Team run ends in a core-owned terminal state, never a model-asserted "done". | Runtime handoff state machine (§2.4) |
| C2 | Task Board item state transitions are performed by the **core**, never by a model. Models only *request* transitions via typed messages. | Board reducer (§2.3) |
| C3 | Every state transition (Workflow phase, Board item state, handoff) is durably recorded before it is acted on. | Append-only log (§2.5) |
| C4 | A Worker receives a **typed briefing** and returns a **typed result**; free-form "I finished" is not a completion signal. | Briefing/result schemas (§2.2) |
| C5 | Milestone validation checks black-box **acceptance assertions** (`ACC-<AREA>-NNN`), not "the model says it works". | Milestone validator (§2.6) |
| C6 | Workflow/Team never bypass the engine's turn queue or step budget; each Worker turn is an ordinary engine turn. | Reuse `engine::runner` + `TurnQueue` |

**Scenario contract (proved in §6).**
- **S1 (happy)**: Preflight passes with all Phase-C artifacts present → Workflow seeds integration branch, empty progress log, `intents/`, and enters the runtime loop.
- **S2 (edge)**: Preflight with a missing artifact (e.g. no `acceptance-contract`) → hard stop with a named, honest error; no branch seeded (C1).
- **S3 (edge/concurrency-gated)**: Two Workers request to claim the same `READY` board item → core admits exactly one to `CLAIMED`, the other observes `CLAIMED` and is re-offered another item (C2).
- **S4 (regression)**: An ordinary single subagent `task` dispatch (today's behavior) is unchanged by the new surfaces (C6, `06-subagents-skills.md`).

---

## 1. Current State (grounded)

Every path below was verified with `rg`/`ls` under `crates/`. Surfaces the prototype needs that do **not** exist are marked **ABSENT**.

### 1.1 What exists — the single-turn, single-level substrate

- **Per-turn loop.** `crates/engine/src/runner.rs:1` — "the durable session runner: the outer activity loop with the inner per-step loop"; step budget `MAX_STEPS` (`crates/engine/src/runner.rs:39`) and concurrent tool-call cap (`:41`). This is the 7-step loop of `02-engine-7step-loop.md`.
- **Turn serialization.** `crates/engine/src/turn_queue.rs:17` `TurnQueue`; `:57` `try_acquire` — one active turn per session, others queued. Any Worker turn must pass through here (C6).
- **Agent personas.** `crates/engine/src/agent.rs:13` `enum Agent { Build, Plan, General, Explore }`; `:30` `ALL`; `:34` `SUBAGENTS = [Plan, General, Explore]`; `:67` `is_subagent`. These are the only named behavioral modes today. There is **no** Orchestrator, Coordinator, or Worker persona.
- **Subagent dispatch (single level).** `crates/tools/src/task.rs:16` `TaskTool` exposes `task` to the model (`subagent_type`, `task_id` resume, `background`). Backing types: `crates/tools/src/tool.rs:343` `TaskTarget`, `:363` `TaskRequest` (`:375` `background`), `:398` `trait TaskRunner` / `:400` `run_task`.
- **Child turns are deliberately capped at one level.** `crates/subagent_runner/src/bridges.rs:30` — `ChildDispatcher` "deliberately omits MCP, skills, websearch, and nested `task` delegation, so a child subagent cannot recursively spawn further subagents" (`:31`). This is the single biggest structural gap for Workflow/Team, and its lift is owned by `04-concurrency.md`.
- **Catalog & wiring.** `crates/subagent_runner/src/catalog.rs:14` `SubagentCatalog` (`:16` `targets`), `:44` `permitted_catalog`, `:61` filters to `agent.availability.spawnable()`. Runner config `crates/subagent_runner/src/runner.rs:31` `CliTaskRunnerConfig`. CLI wiring: `crates/cli/src/run.rs:646` builds the permitted catalog, `:692` constructs the task runner, `:716` `with_task_runner`, `:790` `runner.drain_background()` at shutdown.
- **Durable session store.** `crates/db/src/migrations/0001_initial.sql:3` `sessions` + `:10` `messages`; `0007_session_parent_revert.sql:4` adds `parent_id` (child sessions listable) + index (`:10`). Todos: `crates/db/src/migrations/0003_todos.sql:5`. Session/todo/message row types: `crates/db/src/{session.rs,todo.rs,message.rs}`.
- **Background draining exists** (`run.rs:790`) but only for the flat set of background subagent tasks; there is no coordinator that re-plans or reconciles across them.

### 1.2 What is ABSENT (needed by the prototype, no match under `crates/`)

- **ABSENT — no match under `crates/` for a `Workflow` type, workflow runner, or `run_workflow` entrypoint.** Recon `rg -n "workflow|orchestrat|coordinat|task.?board|feature.?graph|acceptance" crates/ -i` returns only incidental matches (tui geometry/borders/scrollbar, markdown tables, plugin bridge, `agent.rs`) — no domain surface.
- **ABSENT — no match under `crates/` for a Workflow lifecycle/phase state (Planning / Worker-design / Artifact / Runtime / Milestone / End).**
- **ABSENT — no match under `crates/` for a Preflight step verifying Phase-C artifacts** (architecture, acceptance-contract, acceptance-state, features, boundaries, services, init, skills).
- **ABSENT — no match under `crates/` for an acceptance contract / `ACC-<AREA>-NNN` assertion type or a milestone validator.**
- **ABSENT — no match under `crates/` for a Team `Coordinator` persona or a persistent coordinator loop.**
- **ABSENT — no match under `crates/` for a `Task Board`, board item, board state enum (OPEN/READY/CLAIMED/ACTIVE/SUBMITTED/CLOSED/BLOCKED), or a claim/reconcile reducer.**
- **ABSENT — no match under `crates/` for typed Worker briefing / `end_feature`-style result schemas.**
- **ABSENT — no match under `crates/` for an append-only progress/handoff log (`progress-log`, `intents/`) or its writer.**
- **ABSENT — no match under `crates/` for nested/parallel Worker execution** (blocked today by `ChildDispatcher`, `bridges.rs:30`).

**Net:** the engine turn, turn queue, single-level subagent dispatch, persona enum, and session/todo persistence are all reusable substrate. Everything specific to Workflow, Team, and the Task Board is greenfield.

---

## 2. Target Design

The prototype defines two operating shapes on one shared spine. The spine is a **core-owned state machine + append-only log**; Workflow and Team are two front-ends over it.

### 2.1 Workflow lifecycle (6 phases)

Ports the prototype `prompts/workflow/lifecycle.md`:

| Phase | Name | Owner | Output |
|-------|------|-------|--------|
| A | Planning | Orchestrator | dependency-ordered feature graph |
| B | Worker design | Orchestrator | per-feature typed briefings |
| C | Artifact creation | Orchestrator | architecture, acceptance-contract, acceptance-state, features, boundaries, services, init, skills |
| D | Runtime loop | **core** (`run_workflow`) | Workers execute features; blocking driver |
| E | Milestone validation | core validator | acceptance assertions checked |
| F | End gate | core | honest terminal state |

**Preflight = Step 0** (before Phase D): verify all Phase-C artifacts exist and parse; then seed a `workflow/<id>/integration` branch, an empty append-only `progress-log`, and an `intents/` area. A missing or unparseable artifact is a **hard, named stop** (C1, scenario S2) — no branch is seeded.

The runtime loop (Phase D) is the ordinary engine turn per Worker, one feature at a time in dependency order (parallelism deferred to `04-concurrency.md`). See mockups `plans/assets/mockups/workflow-launch.txt` (Preflight → seed → enter loop) and `plans/assets/mockups/workflow-monitor.txt` (per-feature progress + milestone state).

### 2.2 Worker briefing & result (typed)

Each Worker gets a **typed briefing** (feature spec, boundaries, expected artifact, verification tied to acceptance assertions it fulfills) and runs a three-phase micro-loop **Bootstrap → Work → End**, returning a typed `end_feature`-shaped result (schema authority: prototype `worker-base.md`). Free-form completion is not accepted (C4). A Worker is an ordinary subagent turn (`06-subagents-skills.md`); the Orchestrator/Coordinator are new personas layered onto `engine::agent::Agent`.

### 2.3 Task Board (Team)

A shared queue of items. Item schema fields (from prototype `team/task-board.md`): `id`, `state`, `task`, `expected_artifact`, `tool_whitelist`, `forbidden`. State enum (columns in `plans/assets/mockups/team-board.txt`): `OPEN → READY → CLAIMED → ACTIVE → SUBMITTED → CLOSED`, plus `BLOCKED`.

**All transitions are a core reducer** (C2): a Worker *requests* "claim item X"; the core admits at most one claimant to `CLAIMED` and rejects the rest, which are re-offered another `READY` item (scenario S3). The Coordinator (persistent) plans, posts new items, and reconciles `SUBMITTED`/`BLOCKED` items — it never edits or runs code.

### 2.4 Runtime handoffs (deterministic, core-owned)

A small set of core-owned control states drives both shapes: the core decides when a Worker starts, stops, hands back, or is superseded. "A stop is always honest; completion is never fabricated" (C1). Models emit typed *requests*; the core owns the transition.

### 2.5 Append-only recording

Every phase change, board transition, and handoff is appended to a durable log **before** it is acted on (C3). This reuses the DB substrate (`crates/db`) as a new table alongside `sessions`/`messages`/`todos`, plus an on-disk `progress-log`/`intents/` mirror for Workflow (matching the prototype's file layout).

### 2.6 Milestone validation

At milestone boundaries the core validator checks the black-box acceptance assertions (`ACC-<AREA>-NNN`: title, behavioral description, tool/surface, evidence requirement). A milestone passes only when its assertions pass with captured evidence (C5) — this defers evidence semantics to `05-reviewers-verification.md`.

---

## 3. Gap Analysis

| Capability | Current | Target | Gap |
|-----------|---------|--------|-----|
| Long-running plan-driven session | none | Workflow runner (`run_workflow`) | new crate/module + Phase A–F state machine |
| Orchestrator / Coordinator / Worker personas | `Agent` = Build/Plan/General/Explore (`agent.rs:13`) | + Orchestrator, Coordinator, Worker | extend persona enum + personas + `spawnable` policy |
| Nested/parallel Worker turns | blocked (`bridges.rs:30`) | many Workers | **DEPENDS on `04-concurrency.md`** |
| Typed briefing/result | free-form `TaskRequest` (`tool.rs:363`) | typed briefing + `end_feature` result | new schemas; wrap `TaskRunner` |
| Task Board + core reducer | none | board + deterministic transitions | new board store + reducer |
| Acceptance contract + milestone validator | none | `ACC-*` assertions + validator | new types + validator (evidence via `05`) |
| Honest handoffs | queue only (`turn_queue.rs:17`) | control-state machine | new handoff states over the queue |
| Durable phase/board/handoff log | sessions/messages/todos (`db`) | + append-only transition log | new migration + writer + file mirror |

---

## 4. Crate / File Change Map

All paths verified to exist unless marked **(new)**.

**New crate — `crates/workflow/` (new).** Home for the shared spine and both front-ends:
- `src/lifecycle.rs` (new) — Workflow Phase A–F state machine; Preflight/Step-0 artifact verification and branch/log/intents seeding.
- `src/board.rs` (new) — Task Board item type, state enum, and the **core reducer** (claim/submit/close/block); satisfies C2/S3.
- `src/briefing.rs` (new) — typed Worker briefing + `end_feature` result schemas (C4).
- `src/handoff.rs` (new) — core-owned control states (C1).
- `src/acceptance.rs` (new) — `ACC-<AREA>-NNN` assertion type + milestone validator scaffold (evidence hooks to `05-reviewers-verification.md`).
- `src/log.rs` (new) — append-only transition writer (DB + file mirror; C3).
- `src/runner.rs` (new) — `run_workflow` blocking driver for Phase D; drives Worker turns through the existing engine.

**`crates/engine/src/agent.rs` (existing).** Extend `enum Agent` (`:13`), `ALL` (`:30`), and `SUBAGENTS` (`:34`) with `Orchestrator`, `Coordinator`, `Worker`; add personas beside `PLAN_PERSONA`/`GENERAL_PERSONA`/`EXPLORE_PERSONA` (`:90`) and `is_subagent`/`spawnable` treatment (`:67`).

**`crates/subagent_runner/src/` (existing).** `catalog.rs:44` `permitted_catalog` / `:61` spawnable filter must admit the new personas; `runner.rs:31` `CliTaskRunnerConfig` gains a workflow/board handle so Worker child turns report transitions. `bridges.rs:30` `ChildDispatcher` is the nested-dispatch blocker — **modified only per `04-concurrency.md`**, not here.

**`crates/tools/src/` (existing).** `tool.rs:363` `TaskRequest` / `:398` `TaskRunner` wrapped (not replaced) so a Worker dispatch carries a typed briefing and returns a typed result; `task.rs:16` `TaskTool` unchanged for ordinary single subagents (protects S4).

**`crates/db/` (existing).** New migration `crates/db/src/migrations/00NN_workflow.sql` (new) for the transition log + board tables alongside `sessions`/`messages`/`todos` (`0001`, `0003`); row types in `crates/db/src/workflow.rs` (new).

**`crates/cli/src/run.rs` (existing).** Wire the workflow runner beside the existing catalog/runner setup (`:646`, `:692`, `:716`) and drain (`:790`).

---

## 5. Milestones

1. **M1 — Spine types & log.** `crates/workflow` skeleton: board item + state enum, briefing/result schemas, handoff states, append-only log + migration. No behavior yet. (C3, C4 types.)
2. **M2 — Board reducer.** Deterministic claim/submit/close/block; core-owned transitions. Proves S3. (C2.)
3. **M3 — Personas & dispatch.** Orchestrator/Coordinator/Worker in `agent.rs`; `permitted_catalog` admits them; Worker dispatch carries typed briefing/result. Protects S4.
4. **M4 — Workflow Preflight + seed.** Step-0 artifact verification, branch/`progress-log`/`intents/` seeding. Proves S1 & S2; ref mockup `plans/assets/mockups/workflow-launch.txt`.
5. **M5 — Runtime loop (serial).** `run_workflow` drives Workers feature-by-feature in dependency order; monitor surface matches `plans/assets/mockups/workflow-monitor.txt`. (Parallelism deferred to `04`.)
6. **M6 — Milestone validation.** `ACC-*` assertions checked at milestone boundaries; evidence hooks to `05-reviewers-verification.md`. (C5.)
7. **M7 — Team front-end.** Persistent Coordinator over the same board; ref `plans/assets/mockups/team-board.txt`.

---

## 6. TDD Plan

Test-first per scenario; RED → GREEN → surface artifact.

- **S1 (happy) — Preflight passes → seeds.** RED: `workflow::lifecycle` test with a fixture dir containing all Phase-C artifacts asserts `preflight()` returns `Ready` and seeds branch/log/intents; fails (no impl). GREEN: implement Step-0. Surface: CLI run against fixture shows integration branch + empty `progress-log` created.
- **S2 (edge) — missing artifact → honest hard stop.** RED: fixture missing `acceptance-contract`; assert `preflight()` returns a named error and seeds **nothing**. GREEN: verification order + no-side-effect guarantee. Surface: CLI exits non-zero with the named error; `git`/fs shows no branch (C1).
- **S3 (edge) — double claim → one winner.** RED: two `claim(item)` requests on one `READY` item; assert exactly one `CLAIMED`, the other re-offered. GREEN: core reducer. Surface: board dump (matching `team-board.txt` columns) shows one `CLAIMED`, no dupes (C2).
- **S4 (regression) — ordinary subagent unchanged.** RED/GREEN: existing `subagent_runner` tests (`crates/subagent_runner/src/tests.rs`) stay green; add one asserting a plain `task` dispatch needs no briefing. Surface: single-subagent CLI run behaves exactly as `06-subagents-skills.md` documents.
- **Log invariant (C3).** Test: a transition is appended **before** it is observable; crash-injection between append and act leaves the log ahead of state, never behind.

Each production change is preceded by its failing test; full `subagent_runner`/`engine`/`db` suites plus `lsp_diagnostics` on changed files must stay green.

---

## 7. Wave / Dependency Position

- **DEPENDS on `02-engine-7step-loop.md`** — Worker turns are ordinary engine turns; Workflow/Team never bypass the loop or turn queue (C6).
- **DEPENDS on `04-concurrency.md`** — nested/parallel Workers are blocked today by `ChildDispatcher`; serial M1–M5 land first, parallel Worker execution unlocks only after `04`.
- **DEPENDS on `05-reviewers-verification.md`** — milestone acceptance evidence semantics (M6) come from the verification model.
- **DEPENDS on `06-subagents-skills.md`** — Worker = subagent; the persona catalog and single-level dispatch mechanics are defined there.

Position: **after** `02`/`06` (substrate), **alongside/after** `04` (serial before parallel), **before** `05` closes the evidence loop for M6.

---

## 8. Risks & Mitigations

| Risk | Impact | Mitigation |
|------|--------|-----------|
| Nested dispatch stays blocked (`bridges.rs:30`) longer than expected | No parallel Workers | Ship serial M1–M5 fully usable; parallelism is additive behind `04` |
| Model asserts "done" and bypasses honest stop | Fabricated completion (violates C1) | All terminal/board transitions are core-reducer only; models emit requests, never states |
| Board race on claim | Duplicate work / corruption | Single core reducer, one admitted claimant; proved by S3 |
| Log written after action | Unrecoverable state on crash | Append-before-act invariant; crash-injection test (§6) |
| Persona sprawl breaks existing dispatch | Regression in plain subagents | S4 pins current behavior; new personas gated by `spawnable` (`catalog.rs:61`) |
| Acceptance evidence under-specified here | Milestones pass on weak proof | Defer evidence semantics to `05`; M6 only scaffolds `ACC-*` |
| Scope creep into product code | Violates "design only" | This doc adds **no** code; all changes are proposals in §4 |

---

## 9. Commit Sequence

Each commit is test-first (§6) and independently green.

1. `workflow: scaffold crate with board/briefing/handoff/log types (no behavior)` — M1.
2. `db: add workflow transition-log + board migration and row types` — M1/C3.
3. `workflow: core board reducer with deterministic claim/submit/close` — M2, proves S3.
4. `engine: add Orchestrator/Coordinator/Worker personas` — M3.
5. `subagent_runner: admit workflow personas in permitted_catalog` — M3, protects S4.
6. `tools: carry typed briefing/result on Worker dispatch (ordinary task unchanged)` — M3.
7. `workflow: Preflight Step-0 artifact verification + branch/log/intents seeding` — M4, proves S1/S2.
8. `workflow: serial run_workflow runtime loop (Phase D)` — M5.
9. `workflow: milestone acceptance validator scaffold (ACC-*)` — M6 (evidence via `05`).
10. `workflow: persistent Coordinator Team front-end over the board` — M7.
11. `cli: wire workflow runner beside subagent catalog/runner` — integration.
