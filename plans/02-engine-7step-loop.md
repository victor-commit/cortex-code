# 02. Engine: the PROGRAM-owned 7-step loop + trajectory/Session-Log record substrate

Invert the engine from a model-driven ReAct loop (the model decides how many tools
to call and when the turn ends) into the prototype's program-owned control loop,
where the core walks a fixed seven-step pipeline every step and the model is a
bounded component that owns only `decide`. Add the trajectory + Session Log record
substrate that this loop writes and that later docs read.

---

## 0. Scope & Prototype Contracts

Authoritative prototype sources read for this plan (read-only reference tree
`/cortex-code-prototype`):

- **`ORCHESTRATION.md`**: THE seven-step loop. §1 fixes the order and the ownership
  rule: `sense → decide → gate → act → check → remind → record`; "The program owns
  the loop... The model is a bounded component inside that loop: it proposes one
  action at a time and explains why. It does not drive the run" (lines 7-12,
  25-39). §2 mode gate (lines 45-60), §3 budgets (steps / wall-clock / edited-lines,
  lines 64-79), §4 trajectory record (lines 83-92), §5 failure handling
  detect→diagnose→fix (lines 96-106).
- **`REMINDERS.md`**: reminder mechanism and triggers. §2 the three trigger classes:
  event, periodic, drift (lines 42-68); §3 content rule "a slice, not a dump"
  (lines 72-85); §4 verbatim strings (lines 89-126); §5 "Reminders are the sixth
  step of the control loop" (lines 130-137).
- **`prompts/reminders/catalog.md`**: the mechanical string table the scheduler
  draws from: 11 reminder keys with trigger event + target mode(s) (index lines
  18-32; strings lines 34-104): `pre_edit`, `drift`, `post_verification_failure`,
  `budget`, `mode_promotion`, `objective_refresh`, `post_compaction`,
  `external_file_changed`, `todo_stale`, `session_complete`, `injection_warning`.
- **`VERIFICATION.md`**: the verification ladder. Agent-visible portion consumed
  here: Level 0 Deterministic (compile/type/lint/format) and Level 1 Agent-visible
  tests (lines 21-24, 33-42). Levels 2-4 (Held-out, Intent audit, Process check)
  are core-only and held out from the model (lines 25-31); Level 4 reads the
  trajectory this doc writes (lines 56-61). L2-L4 wiring is doc 05's scope, not this
  doc's.
- **`MEMORY.md`**: the record substrate's durable side. Session Log = per-workspace,
  git-ignored rolling record of decisions/findings/dead-ends/verification results
  that survives compaction and restart (§3, lines 58-73); CORTEX.md = repo-committed
  durable facts, human-reviewed (§2, lines 37-54); context governor summarizes into
  the Session Log and never silently truncates (§4, lines 78-89).
- **`prompts/06-orchestration-mindset.md`**: the stance for Workflow/Team planning:
  verifiable units, single-agent by default, deterministic core-owned coordination,
  contract every worker (lines 12-24). Relevant because docs 03/06 wrap this loop.

Naming is drawn only from `/cortex-code-prototype/docs/rename/naming-map.md` and
`GLOSSARY.md`; no term is coined here. Surfaces referenced: Plan Mode / Edit Mode /
Agent Mode / Chat Mode / Headless, Reminders, Verification (levels 0-4), Session Log,
CORTEX.md, Reviewers, Intent Scorer, Subagents, Workflow, Team, worker.

Sibling-doc contracts:
- **Depends on doc 01 (MODE GATE, `plans/01-*.md`)** for the `gate` step: mode tool
  grant + plan scope + safety refusal live there; this doc consumes its gate
  interface.
- **Depends on** a memory substrate (Session Log / CORTEX.md) as defined in `MEMORY.md`.
- **Depended on by doc 05** (Verification / Reviewers): its Level 4 process check and
  Reviewers read the trajectory + Session Log the `record` step writes.
- **Depended on by doc 07** (Intent Scorer, `plans/07-*.md`): intent scoring consumes
  the trajectory.
- **Depended on by doc 03** (Workflow / workers, `plans/03-*.md`) and **doc 06**
  (Subagents, `plans/06-*.md`): each worker / subagent instance runs this same loop.

No product Rust is written by this plan. The only file this task creates or edits is
this document.

---

## 1. Current State (file:line)

The engine today is a **model-driven** loop. The model, not the program, decides how
many tools run and when the turn stops.

- **The loop itself.** `crates/engine/src/runner.rs:309` opens `'turn: loop` inside
  `SessionRunner::run_inner` (`runner.rs:275-668`). Each iteration is one provider
  round-trip. The model emits any number of tool calls; the runner executes them and
  re-injects results; the turn ends only when the model returns a round with no tool
  calls (`runner.rs:525-527`). There is no `sense/decide/gate/act/check/remind/record`
  decomposition; those concerns are inlined and interleaved.
- **Step budget (`MAX_STEPS`).** `DEFAULT_MAX_STEPS: usize = 25` at `runner.rs:40`,
  enforced at `runner.rs:310-324` by emitting a `step-limit-exceeded` diagnostic and
  breaking. This is the only budget; there is no wall-clock or edited-lines ceiling
  (`ORCHESTRATION.md` §3 requires all three).
- **Parallel tool dispatch, concurrency = 4.** `DEFAULT_TOOL_DISPATCH_MAX_CONCURRENCY:
  usize = 4` at `runner.rs:42`; the fan-out executor is `runner.rs:580-619` using a
  `futures_util::stream::FuturesUnordered` with a bounded in-flight window; results are
  re-injected in call order at `runner.rs:621-642`. The model controls the batch: many
  calls per round is normal.
- **Doom-loop guard.** `DOOM_LOOP_REPEAT_THRESHOLD: usize = 2` at `runner.rs:58`;
  `DoomLoopGuard::new` at `runner.rs:298`; struct + `observe` at `runner.rs:1056-1109`.
  It hashes the tool-call round signature (`tool_round_signature`, `runner.rs:1101`)
  and, on N identical rounds, routes through an advisory `loop-safeguard` tool
  (`runner.rs:538-570`) and stops with a `doom-loop` diagnostic. This is blind-retry
  detection, not the `detect → diagnose → fix` pattern of `ORCHESTRATION.md` §5.
- **Tool re-injection.** `runner.rs:352-369` (compaction record surfacing) and
  `runner.rs:621-642` (tool results pushed back as `Message::tool_result`, with the
  `skill` special case at `runner.rs:634-636`).
- **Session admission.** `crates/engine/src/turn_queue.rs:16-123`: `TurnQueue`
  admits at most one active turn per session (`try_acquire`, `turn_queue.rs:57-73`),
  queues the rest, and carries a cooperative abort receiver (`TurnLease`,
  `turn_queue.rs:128-153`). It wraps turns; it knows nothing about the internal loop.
- **Compaction.** `crates/engine/src/compaction.rs` (policy + `estimate_messages_tokens`
  + `compact_to_budget`); driven from `runner.rs:345-378` (summary path) and
  `runner.rs:418-432` (overflow recovery). Compaction is a pre-send concern today,
  not a `sense`-phase input, and it does not fire a `post_compaction` reminder.
- **Persisted parts model.** `crates/protocol/src/message.rs:70-162` :
  `MessagePartBody` variants: `Text`, `Reasoning`, `Tool`, `Step`, `Patch`, `File`,
  `Agent`, `Subtask`, `Compaction`, `Snapshot`, `Retry`, `Usage`. `Step`
  (`message.rs:94-109`) carries per-round-trip lifecycle but there is **no trajectory
  or gate-decision or reminder-fired variant**.
- **Runner events.** `crates/engine/src/event.rs:90-172`: `RunnerEvent` enum
  (`TextDelta`, `Reasoning`, `ToolCall`, `ToolResult`, `ToolSetChanged`, `Usage`,
  `Retry`, `Error`, `Done`). No `GateDecision`, `Reminder`, or `PhaseTransition`.
- **DB.** `crates/db/src/`: `MessageRepo::create_with_parts` (`message.rs:73-114`)
  persists parts into the `message_parts` table (`migrations/0005_message_parts.sql`);
  `SessionRepo` (`session.rs:48-`). Migrations run through `0008`
  (`migrations/0008_projects_accounts.sql`). No trajectory table.
- **Memory.** `crates/memory/src/lib.rs`: `ProjectMemory` discovery/render
  (`lib.rs:42-85`, `discover` `lib.rs:157`) and `DynamicMemoryTracker`
  (`lib.rs:96-155`). This is instruction-file discovery only; there is **no Session
  Log reader/writer** as `MEMORY.md` §3 describes.
- **Events crate.** `crates/events/src/event.rs` + `families.rs`: a typed
  publish/subscribe bus (`EventBus`) with turn lifecycle events. No reminder or gate
  or trajectory events.

Net: the current substrate has all the raw machinery (bounded loop, tool executor,
concurrency, doom guard, compaction, part persistence) but arranged model-first, and
it lacks a trajectory record and a reminder scheduler.

---

## 2. Target State

The **program owns the loop**. Every step walks the fixed seven-phase pipeline; the
model is called exactly once per step, inside `decide`, and returns exactly one
proposed action plus a reason. All other phases are core code.

### 2.1 Phase pipeline

A new module `engine::control_loop` (Rust reserves `loop` as a keyword, so the module
is `control_loop`, not `loop`; documented deviation from the brief's `engine::loop`
label) holds a `LoopController` that drives one `Step` through seven phases. Each phase
is a small struct with a single typed entry point and a typed output that feeds the
next; the controller owns sequencing and short-circuiting.

```
Step {
  sense   : SensePhase   -> SensedState   // files touched, last result, pending
                                          // reminders, live-objective slice, compaction
  decide  : DecidePhase  -> ProposedAction // THE ONE model round-trip: one action + reason
  gate    : GatePhase    -> GateDecision   // doc 01: mode grant + plan scope + safety + budget
  act     : ActPhase     -> ActResult      // execute the single allowed action
  check   : CheckPhase   -> CheckResult    // real result: exit/output/diff + L0-L1
  remind  : RemindPhase  -> Vec<Reminder>  // fire catalog reminders on triggers
  record  : RecordPhase  -> TrajectoryEntry // persist step to db + append Session Log
}
```

- **`sense`** assembles the state the model needs: the working message set, the set of
  files touched so far, the last `CheckResult`, any reminders queued from the previous
  step's `remind`, and the live-objective slice. `compaction.rs` folds in here:
  compact-if-needed runs as a `sense` sub-step, and a compaction fires a
  `post_compaction` reminder (queued for the next step's context).
- **`decide`** is the only model interaction. It issues one provider round-trip
  (reusing the existing streaming path: `stream_lifecycle.rs` + `build_request`,
  `runner.rs:672-688`) constrained to emit exactly one action plus a natural-language
  reason. The model does not execute, validate, or judge; it proposes.
- **`gate`** validates the one proposed action against three things (per
  `ORCHESTRATION.md` §2): the current mode's tool grant, the plan's scope, and the
  safety rules. This calls the MODE GATE interface from **doc 01**. It also enforces
  budgets here (`ORCHESTRATION.md` §3): steps, wall-clock, edited-lines. A refused
  action never reaches `act` and its refusal is recorded.
- **`act`** executes the single allowed action against the environment. The existing
  tool executor folds in here: when one decided action fans out into independent
  sub-operations (for example a read of several paths), the concurrency=4
  `FuturesUnordered` executor (`runner.rs:580-619`) becomes an internal `act` detail,
  not a model-controlled batch. The model still decides one action.
- **`check`** inspects the real result: tool exit status, captured output, and diff,
  then runs the agent-visible ladder rungs (Verification Level 0 Deterministic and
  Level 1 Agent-visible tests) where the action warrants it. The `DoomLoopGuard` folds
  in here and in `gate`: a recurring same-class failure switches the controller from
  blind retry into the `detect → diagnose → fix` pattern (`ORCHESTRATION.md` §5), which
  the `gate` then requires before another attempt is admitted.
- **`remind`** is the reminder scheduler (new module `engine::reminders`). After the
  action is checked and before the next decision is recorded, it evaluates the catalog
  triggers and queues the matching reminder slice for the next `sense`. Reminders are
  ephemeral in context but their firing is recorded.
- **`record`** appends one `TrajectoryEntry` for the step (state sensed, action
  proposed + reason, gate decision, act result, reminder fired, outcome) to the `db`
  trajectory store and appends the human-readable delta to the Session Log via `memory`.

The step-budget check (old `MAX_STEPS`, `runner.rs:310-324`) becomes a `gate` budget
check; `TurnQueue` stays as the outer session-admission layer wrapping the whole loop,
unchanged.

### 2.2 Reminder scheduler (`engine::reminders`)

A `ReminderScheduler` owns the catalog (the 11 keys from
`prompts/reminders/catalog.md`) and the three trigger classes from `REMINDERS.md` §2:

- **Event triggers**: `pre_edit` before `edit_span`/`write_file`, `mode_promotion`
  before a mode-promotion request, `post_verification_failure` after a red level,
  `external_file_changed` on out-of-band file change, `injection_warning` on untrusted
  content entering context.
- **Periodic triggers**: `objective_refresh` every N steps (N mode-dependent, tightest
  in Agent Mode), `post_compaction` after the context governor compacts, `todo_stale`
  when the plan artifact lags, `session_complete` on final turn.
- **Drift trigger**: `drift` when the set of touched files diverges from the plan's
  named set beyond tolerance.

Each fired reminder is a **slice, not a dump** (`REMINDERS.md` §3): at most the
one-line objective, the current plan task, its verification check, and (for drift /
budget) the specific discrepancy. The verbatim strings are the exact catalog texts
with `{braces}` filled by the core at fire time; the model never authors a reminder.
Firing is recorded so a reviewer can later see intent was re-asserted (`REMINDERS.md`
§5). A new `RunnerEvent::Reminder { key, target_mode, rendered }` and a
`MessagePartBody::Reminder { key, rendered }` variant surface reminders on the event
bus and in persisted parts.

### 2.3 `check` phase and the agent-visible ladder

`check` inspects `act`'s real result and, where the action changed code, runs
Verification Level 0 (compile/type/lint/format) and Level 1 (agent-visible tests) only
(`VERIFICATION.md` lines 33-42). A red Level 0 or 1 closes roll-forward: the controller
queues a `post_verification_failure` reminder and refuses new-scope actions at the next
`gate` until the level is green again. Levels 2-4 (held-out, intent audit, process
check) are **not** run here; they are the core's business and are doc 05's scope. This
doc only guarantees the trajectory the Level 4 process check will later read.

### 2.4 Record substrate (trajectory + Session Log)

The `record` phase writes two artifacts (`ORCHESTRATION.md` §4, `MEMORY.md` §3):

1. **Trajectory**: one structured `TrajectoryEntry` per step, persisted to a new `db`
   table via a new `TrajectoryRepo`. Fields: `session_id`, `step` ordinal, `sensed`
   (summary), `action` + `reason`, `gate_decision` (allow/refuse + cause), `act_result`
   (exit/summary/diff ref), `reminder_fired` (key or none), `outcome`, `created_at`.
   This is the queryable record consumed by **doc 05** (Level 4 process check +
   Reviewers) and **doc 07** (Intent Scorer). It reuses the existing part model where
   natural (a `Step` part already exists at `protocol/src/message.rs:94-109`) and adds
   a dedicated trajectory row so consumers can query by step without walking parts.
2. **Session Log**: a per-workspace, git-ignored rolling file appended by `memory`:
   decision + reason, findings tied to `file:line`, dead ends, and verification
   results (`MEMORY.md` §3). The context governor summarizes into it on compaction and
   records the compaction event (`MEMORY.md` §4); a `post_compaction` reminder points
   the model back into it.

Both are plain, inspectable artifacts a human can open. Nothing important lives only in
the model's transient context.

---

## 3. Gap Analysis

The **control inversion** is the core change: today the model owns iteration
(`runner.rs:309` loop ends when the model stops calling tools); the target has the
program own iteration and confines the model to `decide`. Rather than discard the
existing machinery, each piece folds into a phase:

| Existing mechanism | Current role | Folds into |
|---|---|---|
| `'turn: loop` (`runner.rs:309`) | Model-driven round-trip loop | The `LoopController` step loop; the model call becomes only the `decide` phase |
| `MAX_STEPS = 25` (`runner.rs:40`, check `:310`) | Sole per-turn ceiling | A `gate` **step budget**, joined by new wall-clock + edited-lines budgets (`ORCHESTRATION.md` §3) |
| Parallel dispatch, concurrency=4 (`runner.rs:42`, `:580-619`) | Model-controlled multi-tool batch | An **internal `act` executor** for a single decided action's fan-out; no longer model-controlled batching |
| Tool re-injection (`runner.rs:621-642`) | Feed results back for next model round | Moves into `act` -> `check`; results feed `sense` of the next step |
| `DoomLoopGuard` (`runner.rs:298`, `:1056-1109`) | Stop N identical tool rounds | `check`/`gate` **detect -> diagnose -> fix** (`ORCHESTRATION.md` §5): a required diagnostic step before re-admission |
| `TurnQueue` (`turn_queue.rs:16-123`) | One active turn per session | Unchanged; wraps the whole loop as the outer session-admission layer |
| Compaction (`compaction.rs`, `runner.rs:345-378`) | Pre-send shrink | A `sense` sub-step that also fires `post_compaction` and records the compaction (`MEMORY.md` §4) |
| `MessagePartBody` (`message.rs:70-162`) | Persisted parts | Extended with `Reminder`; trajectory rows added alongside (not replacing) `Step` |
| `RunnerEvent` (`event.rs:90-172`) | Streamed turn events | Extended with `GateDecision`, `Reminder`, `PhaseTransition` |

Genuine gaps (nothing exists to fold):
- No reminder scheduler and no reminder catalog binding at all.
- No `gate` step (arrives via doc 01 MODE GATE).
- No trajectory table / repo and no Session Log reader/writer.
- No wall-clock or edited-lines budget.
- `DoomLoopGuard` is blind-retry detection, not `detect -> diagnose -> fix`.

---

## 4. Crate/File Change Map

| File | New / Extend | What changes |
|---|---|---|
| `crates/engine/src/control_loop.rs` | **new** | `LoopController`, `Step`, the seven phase structs (`SensePhase` … `RecordPhase`), typed phase outputs, budget checks. The inversion lives here. |
| `crates/engine/src/reminders.rs` | **new** | `ReminderScheduler`, `ReminderKey` (11 catalog keys), trigger evaluation (event / periodic / drift), slice rendering with `{braces}` fill. |
| `crates/engine/src/trajectory.rs` | **new** | `TrajectoryEntry` type + builder; maps a completed `Step` into a persistable record; the `record`-phase writer glue over `db` + `memory`. |
| `crates/engine/src/runner.rs` | **extend** | `SessionRunner` delegates the turn body to `LoopController`. `MAX_STEPS` check (`:310-324`) becomes the `gate` step budget; parallel dispatch (`:580-619`) moves under `act`; `DoomLoopGuard` (`:1056-1109`) rewired to `detect->diagnose->fix`. Keep the normalized event stream contract (`:16-18`). |
| `crates/engine/src/event.rs` | **extend** | Add `RunnerEvent::GateDecision`, `RunnerEvent::Reminder`, `RunnerEvent::PhaseTransition`. |
| `crates/engine/src/compaction.rs` | **extend** | Expose a `sense`-callable compact hook that returns a compaction record so `remind` can fire `post_compaction` and `record` can log it. |
| `crates/engine/src/lib.rs` | **extend** | `mod control_loop; mod reminders; mod trajectory;` + re-exports. |
| `crates/protocol/src/message.rs` | **extend** | Add `MessagePartBody::Reminder { key, rendered }` (variant list `:70-162`). |
| `crates/db/src/migrations/0009_trajectory.sql` | **new** | `trajectory` table: `id`, `session_id`, `step`, `phase_json`, `action`, `reason`, `gate_decision`, `act_result`, `reminder_fired`, `outcome`, `created_at`; indexed by `(session_id, step)`. |
| `crates/db/src/trajectory.rs` | **new** | `TrajectoryRepo` (`append`, `list_for_session`, `get_step`); mirrors `MessageRepo` shape (`message.rs:44-`). |
| `crates/db/src/lib.rs` + `migrations.rs` | **extend** | Register `trajectory` module + re-export; bump migration set to `0009`. |
| `crates/memory/src/lib.rs` | **extend** | Add `SessionLog` reader/writer (per-workspace, git-ignored file): `append(entry)`, `read()`, and a compaction-summary sink (`MEMORY.md` §3-4). |
| `crates/events/src/event.rs` + `families.rs` | **extend** | Add reminder-fired and gate-decision events for the bus / SSE. |

No files outside the above change. No new crate is introduced.

---

## 5. Milestones

Each milestone has machine-checkable acceptance criteria (a command whose output is
a binary pass/fail).

### M1: Phase scaffolding around existing tool execution
Introduce `control_loop.rs` with the seven phase structs and `LoopController`, and
route `SessionRunner::run_inner` through it, preserving today's behavior (feature-flag
or drop-in). `decide` wraps the existing model round-trip; `act` wraps the existing tool
executor; `record` is a no-op stub.
- **AC1**: `cargo test -p engine` green, including every pre-existing runner test
  (VAL-RUN-002/003/007 event-order tests still pass).
- **AC2**: a trivial turn through `LoopController` emits the identical `RunnerEvent`
  sequence (deltas -> one `Usage` -> one `Done`) as the legacy path, asserted by a
  golden-sequence test.
- **AC3**: `cargo clippy -p engine -- -D warnings` clean (Verification Level 0).

### M2: Gate integration (consumes doc 01 MODE GATE)
Wire the `gate` phase to the MODE GATE interface from doc 01: validate the one proposed
action against mode grant + plan scope + safety, and enforce the step / wall-clock /
edited-lines budgets.
- **AC1**: a test invoking a tool not granted by the active mode yields a
  `GateDecision::Refused` before `act` runs (act executor never called), asserted via a
  spy dispatcher.
- **AC2**: exceeding the step budget produces the existing `step-limit-exceeded`
  diagnostic through the gate path (parity with `runner.rs:310-324`).
- **AC3**: a refusal is recorded (once M5 lands) with its cause; until M5, the refusal
  surfaces as a `RunnerEvent::GateDecision`.

### M3: Check phase (real result + agent-visible ladder + failure handling)
`check` inspects tool exit / output / diff, runs Verification Level 0-1 where the action
changed code, and rewires `DoomLoopGuard` into `detect -> diagnose -> fix`.
- **AC1**: a red Level 0/1 result closes roll-forward: the next `gate` refuses a
  new-scope action and a `post_verification_failure` reminder is queued (asserted).
- **AC2**: N identical failing actions trigger a required diagnostic before a further
  attempt is admitted; the doom-loop remains bounded (a test drives >N identical rounds
  and asserts a bounded stop, parity with `runner.rs:1089-1097`).
- **AC3**: `check` output carries the real exit code / diff reference, asserted against
  a fixture tool result.

### M4: Remind scheduler (`engine::reminders`)
Implement `ReminderScheduler` with the 11 catalog keys and the three trigger classes.
- **AC1**: `pre_edit` fires immediately before an `edit_span`/`write_file` action, with
  the exact catalog string and `{task}`/`{check}` filled (asserted verbatim against
  `prompts/reminders/catalog.md`).
- **AC2**: `objective_refresh` fires every N steps (N mode-dependent); a run of >N steps
  asserts exactly the expected fire count.
- **AC3**: `drift` fires when touched files diverge from the plan's named set beyond
  tolerance; a test with an off-plan edit asserts one `drift` fire.
- **AC4**: every fired reminder is a slice (asserted: contains objective + current task
  + check, and does not contain the full plan text).

### M5: Record -> trajectory + Session Log
Implement the `db` trajectory table + `TrajectoryRepo`, the `memory` Session Log
writer, and the `record` phase that writes both per step.
- **AC1**: after an N-step run, `TrajectoryRepo::list_for_session` returns exactly N
  entries, each with a non-empty action, reason, gate decision, and outcome; verified
  by `sqlite3 <db> "select count(*) from trajectory where session_id=?"`.
- **AC2**: the Session Log file exists, is git-ignored, and contains one appended delta
  per step (decision + result), verified by reading the file.
- **AC3**: a gate refusal (M2) and a fired reminder (M4) both appear as trajectory
  fields, so doc 05's Level 4 process check and doc 07's Intent Scorer can read them
  (asserted: the refusal step's `gate_decision` = refused and `reminder_fired` matches).
- **AC4**: migration `0009` applies idempotently on a fresh and an existing db
  (`Database::open` re-open is a no-op), parity with `db/src/lib.rs` open contract.

---

## 6. TDD Plan

Every production change lands test-first: write the failing test (RED), make the
smallest change to pass (GREEN), then capture a real-surface artifact at the
appropriate agent-visible Verification level (L0-L1; L2-L4 are held out / doc 05).
Live-LLM tests are gated behind an OpenRouter key: skip-if-absent, **never mocked**
(offline tests use recorded fixtures for the provider stream only, never a fake
user-facing surface).

**RED → GREEN examples (each is one atomic pair):**

1. **Disallowed action blocked at gate.**
   - RED: `gate_refuses_ungranted_tool`, construct a step whose `ProposedAction`
     invokes a tool absent from the active mode's grant; assert `GateDecision::Refused`
     and that the `act` executor (spy dispatcher) is never called. Fails initially
     because `gate` does not yet consult the mode grant.
   - GREEN: implement `GatePhase` calling the doc 01 MODE GATE interface.
   - L0-L1 artifact: `cargo test -p engine gate_refuses_ungranted_tool` output showing
     RED then GREEN; `cargo clippy -p engine -- -D warnings` clean.

2. **Reminder fires on its trigger.**
   - RED: `pre_edit_reminder_fires_before_write`, drive a step whose action is
     `write_file`; assert a `RunnerEvent::Reminder { key: pre_edit, .. }` is emitted
     before the act, with the rendered text equal to the catalog `pre_edit` string with
     fields filled. Fails initially (no scheduler).
   - GREEN: implement `ReminderScheduler` event-trigger path + catalog binding.
   - L0-L1 artifact: test output RED→GREEN; a `cortex run --jsonl` capture showing the
     `reminder` event in the live event stream (real surface).

3. **Record produces a trajectory entry.**
   - RED: `record_appends_one_trajectory_entry_per_step`, run a 3-step loop against a
     fixture provider; assert `TrajectoryRepo::list_for_session` returns 3 entries with
     populated fields. Fails initially (no repo / no record phase).
   - GREEN: implement migration `0009`, `TrajectoryRepo`, and `RecordPhase`.
   - L0-L1 artifact: test output RED→GREEN; `sqlite3 <db> "select step, gate_decision,
     reminder_fired from trajectory"` dump (real DB surface) + `cat` of the Session Log
     file showing 3 appended deltas.

4. **DoomLoop still bounded after inversion.**
   - RED: `doom_loop_bounded_under_phase_loop`, drive >`DOOM_LOOP_REPEAT_THRESHOLD`
     identical failing actions through `LoopController`; assert the run stops bounded
     with a `doom-loop` diagnostic and that a diagnostic step was required before any
     re-admission (detect→diagnose→fix). Fails initially because the guard is not yet
     wired into `check`/`gate`.
   - GREEN: rewire `DoomLoopGuard` into `check` + require a diagnostic at `gate`.
   - L0-L1 artifact: test output RED→GREEN asserting a finite step count (parity with
     the `MAX_STEPS`/doom bound invariant, `runner.rs:1089-1097`).

5. **Live-LLM decide step (gated).**
   - RED/GREEN: `decide_emits_single_action_live`, `#[ignore]`-style skip when
     `OPENROUTER_API_KEY` is absent; when present, one real `decide` round-trip returns
     exactly one action + reason. Never mocked; skip-if-absent.
   - Artifact: test output showing SKIPPED without the key and PASS with it.

**Ladder mapping for this doc's own changes:** L0 = `cargo build -p engine -p db -p
protocol -p memory -p events` + `cargo clippy -- -D warnings` + `cargo fmt --check`;
L1 = the `cargo test` suites above. L2-L4 are held out and belong to doc 05; this doc's
only obligation to them is that the trajectory + Session Log it writes are complete and
queryable (M5 AC3).

---

## 7. Wave/Dependency Position

**Upstream (must land first):**
- **doc 01 (MODE GATE)**: supplies the `gate` step interface. M2 blocks on it. M1,
  M4, M5 do not.
- **memory substrate** (`MEMORY.md`): the Session Log side of `record`. M5 depends on
  the Session Log writer, added in the `memory` crate here but conforming to doc-defined
  memory semantics.

**Downstream (blocked on this doc):**
- **doc 05 (Verification / Reviewers)**: Level 4 process check and Reviewers read the
  trajectory + Session Log; the record substrate (M5) is their input.
- **doc 07 (Intent Scorer)**: consumes the trajectory (M5).
- **doc 03 (Workflow / workers)** and **doc 06 (Subagents)**: each worker / subagent
  instance runs this `LoopController`; they wrap it, so they block on M1-M5.

**Wave ordering:**
- Wave A: M1 (phase scaffolding), no external dependency; can start immediately in
  parallel with doc 01.
- Wave B: M2 (needs doc 01) and M3, M4, M5 in parallel (M3/M4/M5 are internal-only).
  M5's trajectory schema is the shared contract for docs 05/07, so land it early in
  Wave B to unblock them.
- Wave C: integration, full seven-phase run with all phases live; hand the trajectory
  + Session Log contract to docs 05/07 and the `LoopController` entry point to docs
  03/06.

---

## 8. Risks & Open Questions

- **Behavior parity during inversion (R1).** Routing `run_inner` through
  `LoopController` risks changing the normalized event order (the VAL-RUN-002/003/007
  contract, `runner.rs:16-18`). Mitigation: M1 golden-sequence test asserts byte-for-byte
  event parity before any phase gains new behavior.
- **One action per `decide` vs today's multi-call rounds (R2).** Providers routinely
  return several tool calls per round. Open question: does `decide` (a) hard-constrain
  the model to one call, or (b) accept the first and defer the rest to later steps, or
  (c) treat a multi-call round as one fan-out action under `act`? Proposed default: (c)
  for read-only independent calls (preserves the concurrency=4 executor), (a) for any
  write/irreversible action (one gated action at a time). Needs confirmation against doc
  01's gate granularity.
- **Trajectory storage shape (R3).** Trajectory could reuse `message_parts`
  (`Step`/new part) or a dedicated `trajectory` table. Chosen: dedicated table, so docs
  05/07 query by `(session_id, step)` without walking parts. Open question: do docs
  05/07 need per-step diffs inlined, or a reference to a diff blob? Proposed: store a
  diff reference, keep the row small.
- **Session Log location + git-ignore (R4).** `MEMORY.md` §3 says per-workspace and
  git-ignored. Open question: exact path (`.cortex/session-log/…`?) and rotation
  policy. Must not collide with the existing `.cortex/` config dir
  (`memory/src/lib.rs` `PROJECT_CONFIG_DIR`).
- **Budget sources (R5).** Wall-clock and edited-lines budgets are new
  (`ORCHESTRATION.md` §3). Open question: defaults and per-mode overrides; where they
  are configured (settings crate). Steps budget reuses `DEFAULT_MAX_STEPS = 25`.
- **Reminder cadence N per mode (R6).** `REMINDERS.md` §2.2 says N is mode-dependent
  and tightest in Agent Mode; the concrete N per mode is unspecified. Proposed initial:
  Plan Mode loose, Agent Mode tight; exact values set with doc 01 mode definitions.
- **`detect → diagnose → fix` shape (R7).** `ORCHESTRATION.md` §5 requires a diagnostic
  step before re-admission but does not fix its representation. Proposed: a required
  `ProposedAction` of kind "diagnose" that `gate` demands after a repeated same-class
  failure before any retry is admitted.

---

## 9. Commit Sequence

Commits are authored and committed as `echobt` (per the commit-identity skill). One
logical commit per milestone; scaffolding and schema land before the code that uses
them.

1. `engine: scaffold control_loop phase pipeline (M1)`, new `control_loop.rs` with the
   seven phase structs + `LoopController`; route `run_inner` through it with behavior
   parity; golden-sequence test. (M1)
2. `protocol,engine,events: add gate/reminder/phase event surfaces`, `MessagePartBody::
   Reminder`, `RunnerEvent::{GateDecision,Reminder,PhaseTransition}`, bus events. (M2/M4
   prerequisite)
3. `engine: wire gate phase to MODE GATE + budgets (M2)`, `GatePhase` over doc 01;
   step/wall-clock/edited-lines budgets; refusal path. (M2)
4. `engine: check phase + detect-diagnose-fix + L0/L1 (M3)`, `CheckPhase`, rewired
   `DoomLoopGuard`, roll-forward closure on red level. (M3)
5. `engine: reminder scheduler over catalog (M4)`, `reminders.rs`, 11 keys, three
   trigger classes, slice rendering; verbatim-string tests. (M4)
6. `db: add 0009 trajectory table + TrajectoryRepo`, migration + repo + re-exports. (M5)
7. `memory: add Session Log reader/writer + compaction sink`, per-workspace git-ignored
   log. (M5)
8. `engine: record phase writes trajectory + Session Log (M5)`, `trajectory.rs`,
   `RecordPhase`, compaction record + `post_compaction` reminder. (M5)
9. `engine: integrate full seven-phase loop + parity/QA capture (Wave C)`, end-to-end
   run, `cortex run --jsonl` event-stream capture, trajectory + Session Log artifacts.

Each commit must leave `cargo build`, `cargo clippy -- -D warnings`, `cargo fmt
--check`, and `cargo test -p engine -p db -p protocol -p memory -p events` green
(Verification Level 0-1) before the next begins.
