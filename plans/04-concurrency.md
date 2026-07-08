# 04. Concurrency & parallelism: bounded parallel `act`, nested Subagent fan-out, deterministic cancellation, and fair scheduling

Map the prototype's concurrency posture onto the real workspace. Today the engine
runs a model-driven loop whose only parallelism is a bounded tool-dispatch window,
and the Subagent child turn deliberately forbids nested delegation. The target keeps
tool concurrency inside the `act` step of the program-owned loop, opens nested
Subagent concurrency under explicit limits, makes cancellation deterministic through
the whole child tree, and lets a Workflow schedule fair, conflict-gated waves. This
is a design doc. No product Rust is written here.

---

## 0. Scope & Contracts

This doc covers the concurrency and parallelism surfaces of the harness:

- **Parallel tool execution** inside one step: the bounded fan-out window that today
  lives in the runner and that the 7-step loop (`02-engine-7step-loop.md`) relocates
  under `act`.
- **`TurnQueue`**: session-scoped admission (one active turn per session) plus the
  cooperative abort channel that carries cancellation.
- **`DoomLoopGuard`**: the doom-loop protection that stops repeated identical rounds,
  and its move from blind-retry to the `detect → diagnose → fix` posture.
- **Subagent / child concurrency**: the `task`-tool child turn, background child
  turns, and the currently-blocked nested spawn (a child cannot spawn a child).
- **Backpressure**: bounding in-flight work at three levels: tool fan-out, background
  child turns, and Workflow wave width.
- **Cancellation**: propagating one abort deterministically to the active turn, its
  fan-out, and every descendant child turn, with no leaked task or orphaned process.

Authoritative prototype sources read for this plan (read-only reference tree
`/cortex-code-prototype`):

- **`ORCHESTRATION.md`**: §1 the seven-step loop and the ownership rule "The program
  owns the loop... The model is a bounded component" (lines 7-38); §5 failure handling
  `detect → diagnose → fix`: "Repeated failure is not met with blind retry... The
  diagnostic step is required before another attempt is gated through" (lines 96-106);
  §6 Workflow control flow (line 110+).
- **`SUBAGENTS.md`**: the spawn mechanism and its single-agent default: "the default
  is one agent. Subagents is not reached for because parallelism sounds" (lines 7-38);
  a worker is spawned only with a contracted instruction, never a vague one (line 38).
- **`prompts/workflow/run-workflow.md`**: wave scheduling: a wave is "a batch of
  features spawned concurrently in one scheduling step, up to `workflowParallelism`",
  default `N=1` (line 22), the select→spawn→fold→record loop (lines 24-28), and the
  runner-owned env-port seam that keeps N>1 workers from colliding (line 25).
- **`prompts/workflow/concurrency/conflict-metric.md`**: the parallel go/no-go signal
 : a measured file-overlap rate, default threshold 15%, "guard first, then measure",
  and the rule that detection only *advises* while merge-back *guarantees*.
- **`prompts/workflow/concurrency/cut-line.md`**: the writable-surface partition that
  gives each worker a disjoint region (the SHARED / ISOLATED / append-only table).
- **`prompts/team/coordinator.md`** + **`prompts/team/task-board.md`**: deterministic
  coordination: "the core, not you, walks it and decides concrete run order and any
  parallelism" (coordinator lines 16, 24); the board state enum `OPEN → READY →
  claimed → CLOSED` where "The core moves items" (task-board lines 36-51).

Naming is drawn only from `/cortex-code-prototype/docs/rename/naming-map.md` and
`GLOSSARY.md`; no term is coined here. Surfaces referenced: Subagents / worker,
Workflow, Team, Coordinator, Orchestrator, Plan / Edit / Agent / Chat Mode, Reminders,
Verification (levels 0-4), Session Log, conflict metric, cut-line, intent reducer,
merge-back, worktree isolation, wave, Task Board.

**Locked contracts inherited (not re-litigated here):**
- The 7-step loop from `02-engine-7step-loop.md` owns step control; tool concurrency is
  an internal detail of its `act` phase. This doc refines *how* that fan-out is bounded,
  cancelled, and extended to nested children; it does not re-open the phase pipeline.
- The MODE GATE (doc 01) decides tool grants; a nested spawn is gated like any other
  action, so nested-concurrency limits are enforced *at the gate*, not invented here.
- The normalized runner event vocabulary (VAL-RUN-002 / VAL-RUN-007,
  `crates/engine/src/runner.rs:15-17`): deltas in order, one summed `Usage`, exactly
  one terminal `Done`. Concurrency must not perturb this order.

The only file this task creates or edits is this document. Nothing under
`crates/` is touched.

---

## 1. Current State (file:line)

Every anchor below was verified via `rg`/`sed` under `/projects/cortex-code/crates/`.
Where the prototype expects a mechanism that does not exist, it is marked **ABSENT**.

### 1.1 Parallel tool dispatch (the one live parallelism)

- **Concurrency ceiling.** `DEFAULT_TOOL_DISPATCH_MAX_CONCURRENCY: usize = 4` at
  `crates/engine/src/runner.rs:42`; stored as the `tool_dispatch_max_concurrency` field
  (`runner.rs:197`), defaulted (`runner.rs:213`), and overridable via
  `with_tool_dispatch_max_concurrency` which clamps to `.max(1)` (`runner.rs:226-227`).
- **The fan-out executor.** Inside the `'turn: loop` (`runner.rs:309`), the dispatch arm
  computes an effective window `max_concurrency = field.max(1).min(round.tool_calls.len())`
  (`runner.rs:580-584`), builds a `VecDeque` of `(index, call)` (`runner.rs:585`), a
  `futures_util::stream::FuturesUnordered` (`runner.rs:587`), and a `completed_results`
  slot vector sized to the round (`runner.rs:588`). It primes the window
  (`runner.rs:591-601`), then drains completions and refills one-for-one
  (`runner.rs:603-620`). This is a **bounded sliding window**, not a semaphore, and it
  is scoped to a single provider round.
- **Model owns the batch.** The number of concurrent tools is whatever the model emitted
  this round; the window only caps *in-flight* count. Results are re-injected **in call
  order** regardless of completion order (`runner.rs:621-642`), with the `skill` special
  case appending to system context (`runner.rs:634-636`).

### 1.2 Step budget and doom-loop protection

- **Step budget.** `DEFAULT_MAX_STEPS: usize = 25` (`runner.rs:40`), enforced at the top
  of the loop: on `steps >= self.max_steps` it emits a `step-limit-exceeded` diagnostic
  and breaks (`runner.rs:310-324`). This is the **only** budget; there is **ABSENT** any
  wall-clock or edited-lines ceiling (`ORCHESTRATION.md` §3 wants all three; this doc
  inherits that gap statement from `02-engine-7step-loop.md` and does not re-solve it).
- **Doom-loop guard.** `DOOM_LOOP_REPEAT_THRESHOLD: usize = 2` (`runner.rs:58`);
  `DoomLoopGuard::new` (`runner.rs:298`); struct at `runner.rs:1058`, impl `observe`
  at `runner.rs:1064`, signature hash `tool_round_signature` at `runner.rs:1101`. On
  N+1 identical rounds `observe` returns a stop detail; the loop routes it through an
  advisory `loop-safeguard` tool call (`runner.rs:538-570`) and, unless the safeguard is
  denied, stops with a `doom-loop` diagnostic. This is **blind-repeat detection**, not
  the `detect → diagnose → fix` pattern (`ORCHESTRATION.md` §5); **ABSENT** any required
  diagnostic step before re-admission.

### 1.3 Session admission and cancellation

- **`TurnQueue`.** `crates/engine/src/turn_queue.rs`: struct at `:17`, an
  `active: Arc<Mutex<HashMap<String, watch::Sender<bool>>>>` map (`:18`) that admits at
  most one active turn per session. `acquire` (`:27`) awaits the slot; `try_acquire`
  (`:57`) inserts a `watch::channel(false)` abort pair (`:65-66`) and hands back a
  `TurnLease` (`:67`).
- **Cooperative abort.** `TurnQueue::abort(session_id)` (`:34`) flips the sender and
  returns whether an active turn existed. `TurnLease` (`:128`) carries the receiver
  `abort_rx` (`:131`) surfaced by `abort_receiver()` (`:144-145`); `Drop for TurnLease`
  (`:149`) releases the slot. Verified by the unit test `abort_signals_only_active_turn`
  (`:234-240`).
- **What it does NOT do.** The abort is a single `watch<bool>` scoped to one session's
  outer turn. It is **ABSENT** any propagation into the in-step fan-out window
  (§1.1 never reads an abort receiver) or into descendant child turns (§1.4). Cancelling
  a parent does not deterministically cancel its Subagents today.

### 1.4 Subagent / child concurrency

- **The child turn.** `crates/subagent_runner/src/runner.rs` is the real work behind the
  `task` tool (module doc `:1-6`). `run_task` (`:69`) dispatches to `run_foreground`
  (`:86`) for a synchronous child, or `run_background` (`:129`) for a detached one.
- **Foreground child.** `run_foreground` (`:296`) opens a child DB session, builds a
  `Turn`, constructs the child tool surface via `ChildDispatcher::for_model` (`:350`),
  layers permissions / checkpoints / hooks / LSP (`:363-390`), then runs a nested
  `SessionRunner::default().run_with_tools(...)` (child loop) to completion.
- **Background child.** `run_background` (`:129`) `tokio::spawn`s the child turn
  (`:167`) and stores its `JoinHandle` in a `background_tasks: Arc<Mutex<BTreeMap<...>>>`
  map (`:53`), rejoinable by id. `drain_background` (`:99-127`) awaits every outstanding
  handle at parent shutdown so no child transcript is lost; the module comment explains
  the hazard it closes: a spawned child "is no longer polled and would be aborted when
  the runtime is dropped" (`:92-98`). Bound: each child turn is itself `MAX_STEPS` +
  transport-idle bounded (`:98`), so the drain terminates.
- **Nested spawn is BLOCKED.** `ChildDispatcher` (`crates/subagent_runner/src/bridges.rs:32`)
  is documented as advertising the built-in tool set but "deliberately omits MCP, skills,
  websearch, and nested `task` delegation, so a child subagent cannot recursively spawn
  further subagents" (`bridges.rs:29-31`). Its `for_model` (`:37`) selects a family-aware
  `ToolRegistry` (`:44-49`); its `ToolDispatcher::dispatch` (`:93`) routes only that
  registry. So today the spawn tree is exactly **one level deep**: the prototype's
  nested Subagent concurrency (SUBAGENTS.md) is **ABSENT** by construction.
- **Spawnability gate.** `crates/subagent_runner/src/catalog.rs:61` filters agents by
  `availability.spawnable()`, and `:73` checks the permission engine
  `classify("task", agent_name) == Action::Allow`. This is the seam a nested-limit policy
  would extend; today it only decides *whether* a top-level `task` may run, not *depth*.

### 1.5 Workflow / Team scheduling

- **ABSENT in-tree.** There is no runner-side wave scheduler, conflict metric,
  `parallel-metrics.jsonl` writer, cut-line partitioner, intent reducer, worktree
  isolation, or merge-back gate under `crates/`. `rg` for these surfaces returns only
  prototype references, not Rust. The whole Workflow/Team concurrency apparatus is a
  prototype contract not yet implemented; `03-workflow-team-board.md` owns building it
  and consumes the primitives this doc hardens.

**Net.** One real parallelism (the concurrency-4 tool window, model-controlled,
uncancellable, single-round), one real serialization (`TurnQueue`, one turn per
session, cooperatively abortable but only at the outer boundary), one real bound
(`MAX_STEPS` + `DoomLoopGuard`, blind-repeat), and a Subagent tree hard-capped at depth
one. Nested concurrency, cancellation propagation, backpressure beyond the single
window, and fair multi-wave scheduling are all ABSENT.

---

## 2. Target Design

The prototype's concurrency posture, mapped onto the loop and Subagent machinery.

### 2.1 Bounded parallel `act` (concurrency stays inside one step)

Per `02-engine-7step-loop.md`, the model no longer owns the batch: it proposes exactly
one action in `decide`. When that one decided action legitimately fans out into
independent sub-operations (a multi-path read, a parallel-safe query set), the existing
concurrency-4 sliding window (`runner.rs:580-620`) becomes an **internal `act` detail**,
not a model-controlled multi-tool round. The window keeps its shape (bounded in-flight,
in-order re-injection) but gains two properties it lacks today:

- **Cancellation-aware.** The fan-out reads the turn's abort receiver
  (`TurnLease::abort_receiver`, `turn_queue.rs:144`). On abort it stops priming new
  in-flight work and lets in-flight futures settle or time out, so a cancel is prompt and
  leak-free rather than "finish the whole round first."
- **Write-serialized.** A fan-out is admitted in parallel only for read-only /
  parallel-safe operations. Any write or irreversible action is one gated action at a
  time (this is the R2 default carried from `02-engine-7step-loop.md` §8: option (c) for
  independent reads, option (a) for writes). The gate (doc 01) makes this call; `act`
  honors it.

The effective width stays `tool_dispatch_max_concurrency` (default 4), now interpreted
as "max concurrent sub-operations within one `act`", surfaced as a per-mode-tunable
budget rather than a hard constant.

### 2.2 Nested Subagent concurrency, ENABLED under limits

The blanket block in `ChildDispatcher` (`bridges.rs:29-31`) is replaced by a **bounded**
policy so the prototype's Subagent model (SUBAGENTS.md: single-agent default, extra
agents only when earned) becomes expressible:

- **Depth limit.** A child may spawn a grandchild only up to a configured maximum depth
  (proposed default: 2, i.e. parent → child → grandchild, no deeper). Depth is carried on
  the child turn context and checked at the spawn gate (extending the
  `catalog.rs:73` `classify("task", …)` seam with a depth predicate). Beyond the limit,
  `task` is simply not granted, a `GateDecision::Refused`, recorded, never a panic.
- **Fan-out limit.** A single agent has a bounded number of concurrently live children
  (proposed default: mirror the tool window, 4), enforced by a per-agent semaphore over
  the `background_tasks` map (`subagent_runner/src/runner.rs:53`). Requests past the
  limit queue (backpressure, §2.4) rather than spawn unboundedly.
- **Global ceiling.** A process-wide cap on total live child turns guards against a wide
  tree multiplying (depth × fan-out) into resource exhaustion. This is the top-level
  backstop the single-writer merge model (cut-line) assumes exists.
- **Single-agent default preserved.** Enabling nesting does not change the default:
  SUBAGENTS.md's "the default is one agent" (lines 27-30) stays the norm; the limits only
  cap what is *possible*, not what is *encouraged*.

### 2.3 Deterministic cancellation across the whole tree

One abort must deterministically stop the active turn, its `act` fan-out, and every
descendant child turn, leaving no leaked task and no orphaned process:

- **Propagation.** `TurnQueue::abort` (`turn_queue.rs:34`) already signals the outer
  turn. The target threads that abort receiver into (a) the `act` fan-out (§2.1) and
  (b) each spawned child turn, so `run_foreground` / `run_background`
  (`subagent_runner/src/runner.rs:296` / `:129`) receive a child abort derived from the
  parent's. Cancelling a parent cancels the subtree.
- **Reaping.** `drain_background` (`:99-127`) is generalized from "drain at shutdown" to
  "drain on cancel": on abort, outstanding child handles are awaited-or-aborted
  deterministically, and their transcripts are still durably written (the existing
  no-orphan guarantee, `:96-98`), so a cancelled tree is rejoinable and clean.
- **Process cleanup.** Child tools that spawn OS processes are already reaped by
  exact-PID cleanup (module doc `subagent_runner/src/runner.rs:5`); cancellation routes
  through the same path so a cancel does not leak a subprocess.

### 2.4 Backpressure at three levels

Nothing may fan out unboundedly. Three nested bounds, outermost first:

1. **Workflow wave width**: `workflowParallelism` (default `N=1`), the batch size of a
   wave (`run-workflow.md:22`). Owned by `03-workflow-team-board.md`; this doc guarantees
   the per-worker primitives it caps.
2. **Per-agent child fan-out**: the semaphore of §2.2 over `background_tasks`.
3. **In-step tool fan-out**: the concurrency-4 window of §2.1.

When any level is saturated, new work **queues** (admission blocks, like `TurnQueue`'s
`acquire`, `turn_queue.rs:27`) rather than over-committing. Backpressure is admission
control, never silent drop.

### 2.5 Doom-loop protection as `detect → diagnose → fix`

`DoomLoopGuard` (`runner.rs:1058-1109`) keeps its signature-hashing detection but is
rewired (per `02-engine-7step-loop.md` §2.3 / M3) so a recurring same-class failure no
longer just stops: it forces a **required diagnostic action** at the next gate before any
retry is admitted (`ORCHESTRATION.md` §5, lines 96-106). Detection stays bounded (the
run still terminates within a finite step count, parity with the `MAX_STEPS`/doom bound),
but the response becomes "name it, hypothesize, gather evidence, change approach" instead
of a blind halt.

### 2.6 Fair scheduling

- **Per-session fairness.** `TurnQueue` (one active turn per session,
  `turn_queue.rs:57-73`) already prevents one session from starving others at the turn
  boundary; the target keeps it as the outer admission layer.
- **Wave fairness.** Within a Workflow, features are selected in array order
  (`run-workflow.md:24`), and parallelism only widens while the rolling **conflict
  metric** stays under threshold (default 15%), "guard first, then measure"
  (`conflict-metric.md`). The Team Coordinator emits a dependency graph and "the core...
  walks it and decides concrete run order and any parallelism"
  (`coordinator.md:16`); the board state enum `OPEN → READY → claimed → CLOSED` with
  "the core moves items" (`task-board.md:36-51`) makes claiming deterministic and
  starvation-free. This doc supplies the cancellable, depth-limited worker primitive the
  Coordinator schedules; the scheduler itself is `03-workflow-team-board.md`.

### 2.7 Operator surface

The concurrency dashboard is speced by the mockup at
`plans/assets/mockups/concurrency-metrics.txt`: per-wave conflict rate with the
WIDEN/HOLD decision against the 15% threshold, the merge-back branch states, and the
quarantine ref list. The events this doc adds (fan-out progress, child-spawn admitted /
refused / cancelled, per-agent live-child count) are the live feed that view renders; the
wave / merge-back rows are populated by `03-workflow-team-board.md`.

---

## 3. Gap Analysis

The controlling change is **cancellation and nesting**: today parallelism is a single
uncancellable window and the Subagent tree is capped at depth one. Each existing piece
folds forward rather than being discarded.

| Existing mechanism | Current role | Folds into |
|---|---|---|
| Concurrency-4 fan-out (`runner.rs:580-620`) | Model-controlled multi-tool round, uncancellable | Internal `act` fan-out, cancellation-aware, write-serialized (§2.1) |
| `TurnQueue` abort (`turn_queue.rs:34, :144`) | Cooperative abort of the outer turn only | Propagated into fan-out + child subtree (§2.3) |
| `DoomLoopGuard` (`runner.rs:1058-1109`) | Blind-repeat stop | Bounded `detect → diagnose → fix` at gate (§2.5) |
| `ChildDispatcher` nested-`task` omission (`bridges.rs:29-31`) | Hard block on any nested spawn | Depth/fan-out-limited nested spawn at the gate (§2.2) |
| `background_tasks` map + `drain_background` (`subagent_runner/src/runner.rs:53, :99-127`) | Rejoin/drain at shutdown | Per-agent fan-out semaphore + drain-on-cancel (§2.2, §2.3) |
| `catalog.rs` spawnability gate (`:61, :73`) | Whether a top-level `task` may run | Extended with a **depth** predicate (§2.2) |
| `MAX_STEPS` (`runner.rs:40, :310-324`) | Sole per-turn ceiling | Joined by wall-clock + edited-lines budgets (deferred to `02-engine-7step-loop.md`) |

**Genuine gaps (nothing exists to fold):**
- No cancellation path from a parent turn into its `act` fan-out or its children.
- No depth/fan-out/global limits on the Subagent tree (it is simply blocked at one).
- No per-agent or global child-turn semaphore (backpressure level 2).
- No `detect → diagnose → fix` diagnostic step (only blind-repeat stop).
- No Workflow wave scheduler / conflict metric in-tree (owned by
  `03-workflow-team-board.md`; this doc only hardens the primitives).

---

## 4. Crate / File Change Map

Grounded in the verified crate layout. No new crate is introduced.

| File | New / Extend | What changes |
|---|---|---|
| `crates/engine/src/runner.rs` | **extend** | The fan-out window (`:580-620`) becomes cancellation-aware: it reads the turn abort receiver and stops priming new in-flight work on abort. `DoomLoopGuard` (`:1058-1109`) rewired to require a diagnostic before re-admission (coordinated with `02`'s M3). Preserve the normalized event contract (`:15-17`). |
| `crates/engine/src/turn_queue.rs` | **extend** | Expose a child-abort derivation so a spawned child turn observes an abort derived from the parent's `watch<bool>` (`:65-66, :144`). Keep one-turn-per-session admission unchanged. |
| `crates/engine/src/event.rs` | **extend** | Add concurrency events: fan-out progress, `ChildSpawnAdmitted` / `ChildSpawnRefused` / `ChildCancelled`, and a per-agent live-child gauge, without disturbing the `Usage`/`Done` terminal contract. |
| `crates/subagent_runner/src/bridges.rs` | **extend** | Replace the blanket nested-`task` omission (`:29-31`) with a depth-limited grant: `ChildDispatcher::for_model` (`:37`) carries the child's depth and conditionally advertises `task` when depth < limit. |
| `crates/subagent_runner/src/runner.rs` | **extend** | Thread parent abort into `run_foreground` (`:296`) and `run_background` (`:129`); add a per-agent fan-out semaphore over `background_tasks` (`:53`); generalize `drain_background` (`:99-127`) to drain-on-cancel. |
| `crates/subagent_runner/src/catalog.rs` | **extend** | Extend the spawn gate (`:61, :73`) with a depth predicate and a global live-child ceiling check. |
| `crates/permissions/src/*` | **extend** | Add the nested-spawn depth / fan-out policy as classifiable rules so the gate (doc 01) enforces limits uniformly (the `classify("task", …)` seam, `catalog.rs:73`). |
| `crates/settings/src/*` | **extend** | Surface the tunables: `tool_dispatch_max_concurrency` per mode, `subagentMaxDepth`, `subagentMaxFanout`, `subagentGlobalCeiling`, `workflowParallelism` default. |

No files outside the above change. Nothing under `crates/` is edited by *this task*
(this table is the plan for a later implementation, not an edit performed now).

---

## 5. Milestones

Each milestone has a machine-checkable acceptance criterion (a command with a binary
pass/fail).

### M1: Cancellation-aware `act` fan-out
Thread the turn abort receiver into the concurrency window (`runner.rs:580-620`).
- **AC1**: a test that aborts mid-fan-out asserts no new in-flight future is primed after
  the signal and the turn ends promptly (bounded settle), with no leaked task.
- **AC2**: the normalized event order is unchanged for a non-cancelled turn (golden
  sequence: deltas → one `Usage` → one `Done`, parity with `runner.rs:15-17`).
- **AC3**: `cargo clippy -p engine -- -D warnings` clean.

### M2: Nested Subagent spawn, depth-limited
Replace the `ChildDispatcher` block (`bridges.rs:29-31`) with a depth-gated `task` grant.
- **AC1**: a child at depth < limit is advertised `task` and can spawn a grandchild
  (asserted via the child dispatcher's `definitions()`, `bridges.rs:93`).
- **AC2**: a child at the depth limit is **refused** `task` with a recorded
  `GateDecision::Refused` (never a panic), asserted through the `catalog.rs:73` gate.
- **AC3**: a fresh + existing config both resolve `subagentMaxDepth` (settings parse).

### M3: Per-agent fan-out + global ceiling (backpressure level 2)
Add the semaphore over `background_tasks` (`subagent_runner/src/runner.rs:53`) and the
global live-child ceiling (`catalog.rs`).
- **AC1**: with fan-out limit K, a K+1th concurrent child **queues** (admission blocks)
  rather than spawning; a test asserts at most K live handles at any instant.
- **AC2**: the global ceiling refuses the N+1th live child process-wide, recorded.

### M4: Deterministic tree cancellation + drain-on-cancel
Propagate abort parent → children and generalize `drain_background` (`:99-127`).
- **AC1**: aborting a parent with two live background children cancels both; a test
  asserts both handles are reaped and both transcripts are durably written (no orphan).
- **AC2**: an OS process spawned by a child tool is reaped on cancel (exact-PID cleanup,
  `subagent_runner/src/runner.rs:5`), asserted by a no-lingering-PID check.

### M5: Doom-loop → detect-diagnose-fix (coordinated with doc 02 M3)
Rewire `DoomLoopGuard` (`runner.rs:1058-1109`) to require a diagnostic before re-admission.
- **AC1**: >`DOOM_LOOP_REPEAT_THRESHOLD` identical failing rounds force a required
  diagnostic action at the next gate before any retry; a test asserts the diagnostic
  precedes re-admission and the run stops bounded (finite step count).

### M6: Concurrency event surface + dashboard feed
Add the events (`event.rs`) the concurrency dashboard renders.
- **AC1**: a `cortex run --jsonl` capture shows `child-spawn` (admitted/refused) and
  fan-out events in order; the shape matches the panels in
  `plans/assets/mockups/concurrency-metrics.txt` (conflict-rate feed + child-spawn feed).

---

## 6. TDD Plan

Every production change lands test-first: write the failing test (RED), make the
smallest change to pass (GREEN), then capture a real-surface artifact at the
agent-visible Verification level (L0-L1; L2-L4 held out / doc 05). Live-LLM tests are
gated behind an OpenRouter key: skip-if-absent, never mocked (offline tests use recorded
provider-stream fixtures only, never a faked user-facing surface).

**RED → GREEN scenarios (each an atomic pair):**

1. **Cancel mid-fan-out leaks nothing.**
   - RED: `abort_during_act_fanout_stops_priming`: drive a step whose one action fans
     out into several slow read sub-operations, fire `TurnQueue::abort`
     (`turn_queue.rs:34`) after the window is primed; assert no sub-operation started
     *after* the signal and the turn returns bounded. Fails initially (the window
     `runner.rs:580-620` never reads the abort receiver).
   - GREEN: thread `TurnLease::abort_receiver` (`turn_queue.rs:144`) into the drain loop.
   - L0-L1 artifact: test output RED→GREEN + a `cortex run --jsonl` capture showing the
     turn terminating without a trailing tool result after the abort.

2. **Nested spawn admitted below depth, refused at limit.**
   - RED: `child_spawns_grandchild_below_depth_limit` and
     `child_at_depth_limit_refused_task`: the first asserts a depth-1 child's
     `ChildDispatcher::definitions()` (`bridges.rs:93`) includes `task`; the second
     asserts a depth-limit child is refused with a recorded `GateDecision::Refused`.
     Both fail initially (nested `task` is blocked outright, `bridges.rs:29-31`).
   - GREEN: depth-carry in `for_model` (`bridges.rs:37`) + depth predicate at
     `catalog.rs:73`.
   - L0-L1 artifact: test output RED→GREEN; the refusal appears in the trajectory
     (contract handed to `07-intent-scorer.md`).

3. **Fan-out semaphore bounds live children.**
   - RED: `per_agent_fanout_never_exceeds_K`: request K+1 concurrent background children;
     assert at most K live handles at any sampled instant and the K+1th only starts after
     one completes. Fails initially (no semaphore over `background_tasks`,
     `subagent_runner/src/runner.rs:53`).
   - GREEN: add the per-agent semaphore + admission wait.
   - L0-L1 artifact: test output RED→GREEN asserting the peak live-count invariant.

4. **Race: two children, one edited surface (no lost update).**
   - RED: `two_children_disjoint_regions_no_lost_update`: spawn two children whose
     writes must land in disjoint regions (the cut-line contract); assert neither
     overwrites the other and both results are folded. Fails initially (no isolation/limit
     primitive). This is the primitive `03-workflow-team-board.md`'s reducer/merge-back
     builds on; here it is proven at the child-turn level only.
   - GREEN: per-agent isolation + serialized fold seam.
   - L0-L1 artifact: test output RED→GREEN + a diff dump showing both regions intact.

5. **Tree cancellation reaps every descendant.**
   - RED: `abort_parent_cancels_and_reaps_children`: abort a parent with two live
     background children; assert both handles reaped, both transcripts written, and no
     lingering PID. Fails initially (abort is not propagated;
     `drain_background` `:99-127` only runs at shutdown).
   - GREEN: parent→child abort derivation + drain-on-cancel.
   - L0-L1 artifact: test output RED→GREEN + a process-list check showing no orphan.

6. **Doom-loop forces a diagnostic (bounded).**
   - RED: `doom_loop_requires_diagnostic_before_retry`: drive
     >`DOOM_LOOP_REPEAT_THRESHOLD` (`runner.rs:58`) identical failing rounds; assert a
     required diagnostic action precedes any re-admission and the run stops bounded. Fails
     initially (guard only blind-stops, `runner.rs:538-570`).
   - GREEN: rewire `DoomLoopGuard` into the gate diagnostic requirement.
   - L0-L1 artifact: test output RED→GREEN asserting a finite step count.

7. **Live-LLM nested decide (gated).**
   - RED/GREEN: `nested_child_decide_live`: `#[ignore]`-style skip when
     `OPENROUTER_API_KEY` is absent; when present, a depth-1 child runs one real `decide`
     round and returns. Never mocked; skip-if-absent.
   - Artifact: test output SKIPPED without the key, PASS with it.

**Ladder mapping.** L0 = `cargo build -p engine -p subagent_runner -p permissions -p
settings` + `cargo clippy -- -D warnings` + `cargo fmt --check`; L1 = the suites above.
L2-L4 are held out (doc 05); this doc's obligation to them is that every concurrency
decision (spawn admitted/refused, cancel, doom-diagnostic) is recorded in the trajectory
that `02-engine-7step-loop.md`'s `record` step writes.

---

## 7. Wave / Dependency Position

**Depends on:**
- `02-engine-7step-loop.md`: the `act` step owns tool concurrency; the loop's abort
  boundary, `DoomLoopGuard` rewrite, and trajectory record substrate are defined there.
  This doc refines the concurrency internals of `act` and does not re-open the pipeline.
- `01-prompt-mode-system.md`: the MODE GATE enforces the nested-spawn depth / fan-out
  limits as classifiable actions; the gate is the single enforcement point.

**Referenced by:**
- `07-intent-scorer.md`: reads the recorded concurrency decisions (spawn refusals,
  cancellations, doom diagnostics) as part of scoring what the run actually did.
- `03-workflow-team-board.md`: builds the wave scheduler, conflict metric, cut-line,
  intent reducer, worktree isolation, and merge-back on top of the cancellable,
  depth-limited, backpressured worker primitive this doc hardens.

**Wave ordering:**
- Wave A: M1 (cancellation-aware fan-out) and M5 (doom diagnostic), internal to `engine`,
  can proceed alongside `02`'s loop work.
- Wave B: M2, M3, M4 (nesting, backpressure, tree cancellation), in `subagent_runner` /
  `permissions` / `settings`, depend on the abort-derivation from M1.
- Wave C: M6 (event surface + dashboard feed), then hand the hardened worker primitive to
  `03-workflow-team-board.md` and the recorded decisions to `07-intent-scorer.md`.

---

## 8. Risks & Mitigations

- **Deadlock (R1).** Nested admission (parent holds a fan-out slot while a child waits on
  the same global ceiling) can deadlock. Mitigation: admission is depth-ordered: a child
  never blocks on a resource its ancestor holds; the global ceiling counts *live* turns
  and a parent awaiting a child does not count against fan-out. A test drives a full-depth
  full-fan-out tree and asserts forward progress.
- **Starvation (R2).** A wide fast agent could monopolize the global child ceiling and
  starve siblings. Mitigation: `TurnQueue` fairness at the session boundary
  (`turn_queue.rs:57-73`) plus per-agent fan-out caps (§2.2) bound any single agent's
  share; Workflow selection stays array-ordered (`run-workflow.md:24`).
- **Unbounded fan-out (R3).** Enabling nesting risks depth × fan-out explosion.
  Mitigation: three independent bounds (wave width, per-agent fan-out, global ceiling,
  §2.4); each is a hard admission gate, and the conflict metric (`conflict-metric.md`,
  default 15%) holds parallelism from widening into a rising collision rate.
- **Cancellation leaks (R4).** A cancelled parent could leave a child polling, a
  transcript half-written, or a subprocess orphaned. Mitigation: drain-on-cancel
  generalized from `drain_background` (`:99-127`) guarantees every handle is reaped and
  every transcript durably written; exact-PID cleanup
  (`subagent_runner/src/runner.rs:5`) reaps subprocesses; M4 AC2 asserts no lingering PID.
- **Event-order regression (R5).** Concurrency changes could perturb the normalized
  `Usage`/`Done` contract (`runner.rs:15-17`, VAL-RUN-002/007). Mitigation: M1 AC2 golden
  sequence asserts byte-for-byte parity for the non-cancelled path before any new event is
  added.
- **False independence (R6).** Two children on "disjoint" files can still collide on a
  shared contract / config / schema, the leaky-proxy warning in `conflict-metric.md`.
  Mitigation: this doc guarantees only the *primitive* (isolated, cancellable children);
  the correctness guarantee is the serialized test-gated merge-back owned by
  `03-workflow-team-board.md`. Detection advises; the merge-gate guarantees.

---

## 9. Commit Sequence

Commits are authored and committed as `echobt` (per the commit-identity skill). One
logical commit per milestone; enforcement seams and settings land before the code that
uses them.

1. `engine: make act fan-out cancellation-aware (M1)`, thread the turn abort receiver
   into the concurrency window; golden-sequence parity test.
2. `engine: doom-loop requires a diagnostic before retry (M5)`, rewire `DoomLoopGuard`
   into the gate diagnostic requirement; bounded-stop test.
3. `settings,permissions: add nested-spawn depth/fan-out/ceiling policy`, the tunables
   and classifiable rules the gate enforces (M2/M3 prerequisite).
4. `subagent_runner: depth-limited nested task grant (M2)`, replace the `ChildDispatcher`
   block with a depth-gated `task`; admitted-below / refused-at-limit tests.
5. `subagent_runner: per-agent fan-out semaphore + global ceiling (M3)`, bound live
   children; peak-live-count test.
6. `engine,subagent_runner: deterministic tree cancellation + drain-on-cancel (M4)`,
   propagate abort parent→children; reap-and-no-orphan test.
7. `engine,events: concurrency event surface + dashboard feed (M6)`, spawn/cancel/fan-out
   events; `cortex run --jsonl` capture matching the concurrency-metrics mockup.

Each commit must leave `cargo build`, `cargo clippy -- -D warnings`, `cargo fmt --check`,
and `cargo test -p engine -p subagent_runner -p permissions -p settings` green
(Verification Level 0-1) before the next begins.
