# 05. Reviewers + the held-out Verification ladder

Give the program a verification stage it owns and the model cannot see. Today the
engine has no reviewer dispatch and no program-owned verification: the turn ends when
the model stops calling tools, and "verification" survives only as advisory prose in a
prompt. This plan maps the prototype's Reviewers (Code Reviewer, QA Validator) and the
held-out rungs of the Verification ladder onto the real Rust workspace, wired into the
7-step loop's `check` step and dispatched through the Subagents machinery.

---

## 0. Scope & Contracts

Authoritative prototype sources read for this plan (read-only reference tree
`/cortex-code-prototype`):

- **`prompts/reviewers/code-reviewer.md`**: the Code Reviewer, the independent
  code-scrutiny Reviewer. It reads the request, the diff, and the author's closing
  report, answers five scrutiny questions (scope fidelity, correctness, dead weight,
  swallowed failure, honest report), and returns one pass/fail verdict with evidence at
  `file:line`. It reads code; it never runs the surface, and it never authors the fix
  (lines 9-26, 46-83).
- **`prompts/reviewers/code-reviewer-feature.md`**: the per-feature scoped subagent a
  Code Reviewer dispatches when a change spans many independent features. It catches
  scope drift and "building to the test" for one feature slice and returns evidence to
  the parent, which owns the aggregate call (lines 9-27, 29-58).
- **`prompts/reviewers/qa-validator.md`**: the QA Validator, the user-facing behavior
  Reviewer. It exercises the real surface (CLI, HTTP, UI, TUI) the way a user would,
  captures an artifact per behavior, and judges pass/fail/blocked. "Un-run is not done"
  (lines 9-38, 48-57).
- **`prompts/reviewers/qa-validator-flow.md`**: the per-flow scoped subagent a QA
  Validator dispatches for one multi-step flow end to end, inside a fixed isolation
  boundary, capturing an artifact per step and perturbing the happy path (lines 9-24,
  27-55).
- **`VERIFICATION.md`** (via `GLOSSARY.md` lines 67-72): the ladder. Level 0
  Deterministic (compile/type/lint/format), Level 1 Agent-visible tests, Level 2
  Held-out, Level 3 Intent audit, Level 4 Process check. Levels 2-4 are run by the core
  and held out from the model.

Naming is drawn only from `/cortex-code-prototype/GLOSSARY.md` and
`/cortex-code-prototype/docs/rename/naming-map.md`; no term is coined here. Live
surfaces referenced: **Reviewers**, **Code Reviewer**, **QA Validator**, **Intent
Scorer**, **Verification** (Levels 0-4), **Subagents**, **worker**, **Session Log**,
**CORTEX.md**. For provenance only, the rename map records that these professional
names replaced earlier working labels (Reviewers was "Inspectors"; Code Reviewer,
QA Validator, Intent Scorer, and Verification replaced their prior working labels per
`docs/rename/naming-map.md` lines 35-41). Those prior labels are never used as live
terms in this doc.

### Locked contracts inherited

1. **The verification ladder is held out from the model.** Level 0 (Deterministic) and
   Level 1 (Agent-visible tests) are worker-facing and already settled inside the loop's
   `check` step (see `02-engine-7step-loop.md`, which owns L0-L1). This doc owns the
   **held-out** rungs, Levels 2-4: held-out checks, intent audit, and process check.
   The worker never sees L2-L4 cases or verdicts before submit, so scores are earned,
   not gamed (mockup framing in `plans/assets/mockups/verification-ladder.txt`).
2. **Reviewers are off by default.** Subagents are off by default and spawned only where
   an ablation earns the extra agent (`GLOSSARY.md` line 64). Code Reviewer and QA
   Validator inherit that default: the held-out ladder runs the deterministic and
   held-out-test rungs always, but the reviewer rungs (L3 Code Reviewer, L4 QA
   Validator) activate only when configured on or when the change size warrants them.
3. **One-term-one-surface (`GLOSSARY.md` lines 12-16, 82).** Code Reviewer scrutinizes
   *code and change* (static read). QA Validator validates *running behavior* (dynamic
   execution). Repo Readiness report evaluates the *repository* and is Intent-Scorer
   side, out of scope here. These three surfaces stay strictly apart.

**Rung reconciliation.** The informal "self / tests / reviewer / QA" rungs map onto the
frozen VERIFICATION.md numbering as: self-check and agent-visible tests are Levels 0-1
(worker-facing, owned by `02-engine-7step-loop.md`); held-out tests are Level 2; the
independent reviewer / intent audit is Level 3 (Code Reviewer); the process check and
behavior validation are Level 4 (QA Validator plus trajectory reconciliation). The two
reviewer-output panels label them exactly this way: "Code Reviewer · L3 intent audit"
and "QA Validator · L4 process check"
(`plans/assets/mockups/reviewer-output.txt` lines 4-16).

No product Rust is written by this plan. The only file this task creates or edits is
this document. Nothing under `crates/` is touched.

---

## 1. Current State (file:line)

Reviewers and the held-out verification ladder are **absent** today. The engine runs a
model-driven loop with no program-owned verification stage and no reviewer dispatch; the
only trace of "verification" anywhere in the engine is a sentence inside a prompt string.

**Reviewer dispatch: ABSENT.**

- **The built-in subagent roster has no reviewer.** `crates/subagent_runner/src/custom_agents.rs:601-613`
  defines exactly four built-in agents: `build`, `plan`, `general`, `explore`. There is
  no Code Reviewer, no QA Validator, no code-reviewer-feature, no qa-validator-flow. The
  availability enum (`custom_agents.rs:34`, with `spawnable` at `:52`) that would gate a
  reviewer as a spawn-only subagent exists, but no reviewer is registered against it.
- **The task tool exposes no reviewer target.** `crates/tools/src/task.rs:229-230`
  builds the default target set as `explore` and `general` only. A caller cannot dispatch
  a reviewer because none is a `TaskTarget`.
- **The catalog cannot surface one.** `crates/subagent_runner/src/catalog.rs:44`
  (`permitted_catalog`) filters the discovered agents by `availability.spawnable()`
  (`catalog.rs:61`); with no reviewer in the roster, the filtered `SubagentCatalog`
  (`catalog.rs:14`) never contains one.
- ABSENT: no match under `crates/` for a Code Reviewer or QA Validator agent, tool, or
  dispatch path.

**Program-owned verification stage: ABSENT.**

- **The loop is model-driven, not gate-driven.** `crates/engine/src/runner.rs:309` opens
  `'turn: loop` inside `SessionRunner`; the turn ends only when the model returns a round
  with no tool calls. The bounded machinery around it is a step budget
  (`DEFAULT_MAX_STEPS: usize = 25`, `runner.rs:40`), a parallel tool executor
  (`DEFAULT_TOOL_DISPATCH_MAX_CONCURRENCY: usize = 4`, `runner.rs:42`; fan-out at
  `runner.rs:587` via `FuturesUnordered`), a `DoomLoopGuard`
  (`runner.rs:58`, `:298`, struct at `:1058`), and the outer `TurnQueue`
  (`crates/engine/src/turn_queue.rs:17`). None of these is a verification stage: there is
  no point in the loop where the program runs held-out tests, an intent audit, or a
  process check against a submitted change.
- **"Verification" exists only as prompt prose.** The single hit for the concept under
  `crates/engine/src/` is `crates/engine/src/agent.rs:163`, a plan-mode instruction
  string telling the model to "describe ... how the result would be verified." That is
  advisory text handed to the model, not a program-owned check. ABSENT: no match under
  `crates/engine/src/` for a reviewer, a held-out test runner, an intent audit, or a
  process check.
- **The tool registry has no verification tool.** `crates/tools/src/registry.rs:51-88`
  registers the built-in tool set (read, write, edit, bash, glob, grep, lsp, todo,
  webfetch, question, plan, and the rest). There is no reviewer tool and no
  verification-ladder tool.

**Held-out substrate: ABSENT.**

- **No verdict or verification-result persistence.** `crates/db/src/migrations/` runs
  `0001`-`0008` (initial, usage, todos, session_model, message_parts, todo_priority,
  session_parent_revert, projects_accounts). There is no reviewer-verdict table and no
  verification-result table. `crates/db/src/lib.rs:39` re-exports the migration
  machinery; nothing registers a verification schema.
- **No trajectory / Session Log for a process check to read.** The Level 4 process check
  reconciles the self-report against captured artifacts, and the trajectory + Session Log
  it reads do not exist yet; they are introduced by `02-engine-7step-loop.md`. This doc
  depends on that substrate being present.
- **No held-out test config.** `crates/settings/src/schema.rs:25` validates `max_steps`
  and the rest of the current schema; there is no reviewers-enabled flag and no held-out
  test-set configuration.

Net: every piece is missing. The workspace has the scaffolding to dispatch a scoped
subagent and to persist parts, but no reviewer is registered, no held-out ladder runs,
and no verdict is recorded.

---

## 2. Target Design

The core owns a **held-out verification stage** that runs after a change is submitted,
inside the loop's `check` step (`02-engine-7step-loop.md`). The stage walks the held-out
rungs of the ladder in order and, where configured, dispatches Reviewers as scoped
Subagents. The worker never sees the held-out cases or the verdicts until the stage
returns a decision.

### 2.1 The ladder this doc owns (Levels 2-4)

Levels 0-1 are worker-facing and already run in `check` (doc 02). This doc adds the
held-out driver for the rest:

- **Level 2 Held-out checks.** Tests and perturbation checks the worker never sees, run
  by the core against the submitted change. A red L2 fails the stage before any reviewer
  is spawned; there is no point auditing code that does not pass the checks it could not
  read.
- **Level 3 Intent audit (Code Reviewer).** An independent static review of the diff
  against the original request. It answers the five scrutiny questions and returns a
  binary verdict with `file:line` evidence. On a large multi-feature change it fans out
  one code-reviewer-feature subagent per feature and synthesizes one verdict.
- **Level 4 Process check (QA Validator).** A trajectory-and-behavior reconciliation:
  the QA Validator exercises the real surface, captures an artifact per behavior, and the
  core reconciles the worker's self-report against the captured trajectory + Session Log.
  "Un-run is not done": a behavior with no artifact is reported blocked, never passing.
  It fans out one qa-validator-flow subagent per multi-step flow inside an isolation
  boundary.

The ladder short-circuits on the first red rung: L2 red skips L3/L4; an L3 fail can gate
merge before L4 runs, per the reviewer gate below.

### 2.2 Reviewers as Subagents

The four reviewer prompts become built-in Subagents (off by default), registered against
the existing availability machinery so the task tool and catalog can dispatch them:

- **Code Reviewer** and **QA Validator** register as spawnable agents (they are also the
  L3 and L4 drivers the core invokes directly).
- **code-reviewer-feature** and **qa-validator-flow** register as scoped, spawn-only
  subagents that the parent reviewers fan out to. Each receives a narrow slice (a
  feature's request + diff, or a flow's behaviors + isolation boundary) and returns
  evidence; the parent owns the aggregate verdict.

They dispatch through the same `CliTaskRunner` / `permitted_catalog` path every subagent
uses, so tool whitelists, permission filtering, and process cleanup are inherited, not
reinvented.

### 2.3 The reviewer gate and verdicts

A verdict is binary with evidence. The reviewer-output surface
(`plans/assets/mockups/reviewer-output.txt`) fixes the shape: Code Reviewer returns
per-question pass/fail with `file:line` evidence and a diff summary (for example
`✗ Q3 Dead weight fail, unused helper left in server/util.rs:88`); QA Validator returns
per-behavior pass/fail/blocked with the captured artifact path. The gate is strict:
**any FAIL re-posts to the worker with the concrete list of what must change, and no
merge happens** (mockup line 18, `code-reviewer.md` lines 75-83). A "looks good but"
is a fail. Each verdict is recorded (Session Log + a verdict row) so a re-run after
fixes reviews only what changed.

The verification-ladder surface (`plans/assets/mockups/verification-ladder.txt`) is the
worker-facing view: it shows L0-L1 as `[visible]` and L2-L4 as `[masked]`, making the
held-out boundary legible without leaking the cases.

---

## 3. Gap Analysis

Delta between §1 (nothing exists) and §2 (held-out stage + reviewers), itemized.

| # | Target capability (§2) | Current state (§1) | Gap |
|---|---|---|---|
| G1 | Held-out verification driver invoked from `check` | No verification stage; only prompt prose (`agent.rs:163`) | Build the whole L2-L4 driver; nothing to fold |
| G2 | Level 2 held-out checks run by the core | ABSENT | New held-out test runner + held-out test-set config |
| G3 | Level 3 Code Reviewer (intent audit) | No reviewer registered (`custom_agents.rs:601-613`) | New built-in agent + L3 driver invoking it |
| G4 | Level 4 QA Validator (process check) | ABSENT | New built-in agent + L4 driver reconciling trajectory + artifacts |
| G5 | Fan-out subagents (feature / flow reviewers) | No such targets (`task.rs:229-230`) | Register two scoped spawn-only subagents |
| G6 | Reviewer dispatch through the catalog | Catalog filters roster that has no reviewer (`catalog.rs:44,61`) | Add reviewers to the roster; gate by an enable flag |
| G7 | Reviewer gate: any FAIL blocks merge, re-posts | No gate exists | New gate wired into the loop's post-`check` decision |
| G8 | Verdict persistence + Session Log record | Migrations stop at `0008`; no verdict table | New migration + verdict repo; Session Log verdict append |
| G9 | Off-by-default toggle for reviewers | `settings/schema.rs:25` has no such key | New settings keys (reviewers-enabled, size threshold) |
| G10 | Worker-facing masked/visible ladder view | No ladder UI | New TUI surface per the two mockups |

Two dependencies are consumed, not built here: the loop's `check` step and its
trajectory + Session Log substrate come from `02-engine-7step-loop.md`; the subagent
dispatch machinery (`CliTaskRunner`, `permitted_catalog`, availability enum) already
exists and is extended, not replaced.

---

## 4. Crate / File Change Map

Every path below is verified present under `/projects/cortex-code/crates/`.

| Crate::path | Change | What changes | Why |
|---|---|---|---|
| `engine::verification` (`crates/engine/src/verification.rs`) | **new** | The held-out ladder driver: runs L2 held-out checks, invokes L3 Code Reviewer and L4 QA Validator, short-circuits on the first red rung, returns a `LadderOutcome`. | G1, G2. No verification stage exists today; this is the program-owned held-out portion. |
| `engine::reviewers` (`crates/engine/src/reviewers.rs`) | **new** | Reviewer orchestration: build the request/diff/report bundle a Code Reviewer needs, the behavior/isolation bundle a QA Validator needs, fan-out into feature/flow subagents, synthesize one verdict each. | G3, G4, G5. Turns the four reviewer prompts into a dispatchable stage. |
| `crates/engine/src/runner.rs` | **modify** | The loop's `check` step (post-`act`) calls `verification::run` on a submitted change and applies the reviewer gate before recording; a FAIL closes roll-forward and re-posts. Reuses the existing bounded loop (`:309`), budget (`:40`), and doom guard (`:1058`); adds no new model-driven iteration. | G1, G7. `check` is where reviewers fire (per doc 02). |
| `crates/engine/src/lib.rs` | **modify** | `mod verification; mod reviewers;` + re-exports of `LadderOutcome` / verdict types. | G1. Register the new modules. |
| `crates/engine/src/event.rs` | **modify** | Add runner events for a ladder-rung result and a reviewer verdict so the bus/SSE and TUI can render them. | G7, G10. Verdicts must surface. |
| `crates/subagent_runner/src/custom_agents.rs` | **modify** | Register four built-in agents at the roster (`:601-613`): Code Reviewer, QA Validator (both `spawnable`), code-reviewer-feature, qa-validator-flow (spawn-only, subagent availability). Ship each prompt body from the corresponding `prompts/reviewers/*.md`. | G3, G4, G5. The roster has none today. |
| `crates/subagent_runner/src/catalog.rs` | **modify** | `permitted_catalog` (`:44`) includes reviewer agents only when reviewers are enabled and the change warrants them; still filtered by `availability.spawnable()` (`:61`). | G6, G9. Off-by-default gating. |
| `crates/tools/src/task.rs` | **modify** | Extend the default target set (`:229-230`) so a reviewer can be a `TaskTarget` when enabled; unavailable otherwise (the existing unavailable-target diagnostic path applies). | G5. Dispatch surface for fan-out. |
| `crates/protocol/src/message.rs` | **modify** | Add a `MessagePartBody` variant for a reviewer verdict / ladder result so a verdict persists as a message part alongside the change. | G8. Verdicts are durable, inspectable parts. |
| `crates/db/src/migrations/0009_verification.sql` | **new** | `verification_verdict` table: `id`, `session_id`, `step`, `level` (2/3/4), `reviewer` (code-reviewer / qa-validator / none), `outcome` (pass/fail/blocked), `evidence_ref`, `created_at`; indexed by `(session_id, step)`. Numbered after the current `0008`. | G8. No verdict persistence exists. |
| `crates/db/src/verification.rs` | **new** | `VerdictRepo` (`append`, `list_for_session`, `latest_for_step`); mirrors the existing repo shape. | G8. Query verdicts by step for re-review. |
| `crates/db/src/lib.rs` | **modify** | Register the `verification` module and re-export `VerdictRepo`; the migration set picks up `0009` through the existing loader (`:39`). | G8. Wire the new schema + repo. |
| `crates/memory/src/lib.rs` | **modify** | Append reviewer verdicts to the Session Log (the writer added by doc 02) so a re-run reviews only what changed. | G8. Durable narrative of verdicts. |
| `crates/settings/src/schema.rs` | **modify** | Add validated keys next to `max_steps` (`:25`): `reviewers_enabled` (default off), `reviewer_fanout_threshold`, held-out test-set location. | G9. Off-by-default configuration. |
| `crates/cli/src/run.rs` | **modify** | Surface ladder-rung results and reviewer verdicts in the run output / JSONL stream so a headless caller sees the gate decision. | G7, G10. Verdicts reach the operator. |
| `crates/tui/src/view.rs` (+ a new `tui-components` panel) | **modify / new** | Render the two mockups: the masked/visible ladder view and the dual reviewer-output panels. | G10. Worker-facing ladder + verdict UI. |

No new crate is introduced. No file outside this table changes.

---

## 5. Milestones

Each milestone has a machine-checkable acceptance criterion (a command whose output is
binary pass/fail). Milestones are ordered so the schema and the off-by-default gate land
before the code that depends on them.

### M1: Verdict substrate (schema + repo + part)
Add migration `0009_verification.sql`, `VerdictRepo`, and the `MessagePartBody` verdict
variant.
- **AC1**: migration `0009` applies idempotently on a fresh and an existing db, parity
  with the `db` open contract; `sqlite3 <db> ".schema verification_verdict"` shows the
  table.
- **AC2**: `VerdictRepo::append` then `list_for_session` round-trips a verdict with all
  fields populated (`cargo test -p db`).
- **AC3**: `cargo clippy -p db -p protocol -- -D warnings` clean (Level 0).

### M2: Reviewers registered as off-by-default Subagents
Register Code Reviewer, QA Validator, code-reviewer-feature, and qa-validator-flow in the
roster with the correct availability; gate them behind `reviewers_enabled`.
- **AC1**: with reviewers disabled, `permitted_catalog` contains no reviewer (parity with
  today's `explore`/`general` set); with reviewers enabled, it contains Code Reviewer and
  QA Validator, and the two fan-out agents are spawn-only.
- **AC2**: a `task` call naming a reviewer target errors with the existing
  unavailable-target diagnostic when disabled, and dispatches when enabled.
- **AC3**: each registered reviewer's prompt body equals the corresponding
  `prompts/reviewers/*.md` (asserted against the shipped source).

### M3: Level 2 held-out checks
Implement the held-out test runner and its config; run it in the `check` step against a
submitted change, before any reviewer.
- **AC1**: a change that passes agent-visible L0-L1 but fails a held-out case yields a
  `LadderOutcome` red at Level 2, and no reviewer is spawned (asserted via a spy
  dispatcher).
- **AC2**: the held-out cases are never present in the model-facing context of the run
  (asserted: the rendered prompt/messages contain none of the held-out fixtures).

### M4: Level 3 Code Reviewer (intent audit) + fan-out
Drive the Code Reviewer over the request + diff + closing report; fan out one
code-reviewer-feature per feature on a large change and synthesize one verdict.
- **AC1**: a diff with an unused helper and a swallowed error yields a Code Reviewer
  **fail** whose verdict names both at `file:line` (fixture diff), matching the
  reviewer-output shape (`plans/assets/mockups/reviewer-output.txt`).
- **AC2**: a multi-feature change spawns one feature subagent per feature and produces a
  single aggregate verdict (asserted via dispatch spy).
- **AC3**: any FAIL blocks merge and re-posts the change list to the worker; no merge
  event fires (asserted).

### M5: Level 4 QA Validator (process check) + fan-out
Drive the QA Validator over the real surface; reconcile the self-report against the
trajectory + Session Log; fan out one qa-validator-flow per flow inside an isolation
boundary.
- **AC1**: a behavior executed against the real surface with a captured artifact is
  reported **pass**; the same behavior with no artifact is reported **blocked**, never
  passing ("un-run is not done").
- **AC2**: a self-report claim with no backing trajectory entry yields a process-check
  fail (fixture trajectory).
- **AC3**: verdicts from M4 and M5 persist via `VerdictRepo` and append to the Session
  Log; `sqlite3 <db> "select level, reviewer, outcome from verification_verdict"` shows
  L3 and L4 rows.

### M6: Ladder + reviewer-output surfaces
Render the worker-facing masked/visible ladder and the dual reviewer-output panels.
- **AC1**: the ladder view shows L0-L1 as visible and L2-L4 as masked, matching
  `plans/assets/mockups/verification-ladder.txt` (snapshot test).
- **AC2**: the reviewer-output view shows the Code Reviewer L3 and QA Validator L4 panels
  with per-question / per-behavior verdicts, matching
  `plans/assets/mockups/reviewer-output.txt` (snapshot test).

---

## 6. TDD Plan

Every production change lands test-first: write the failing test (RED), make the smallest
change to pass (GREEN), then capture a real-surface artifact at the agent-visible
Verification levels (L0-L1). The held-out rungs (L2-L4) this doc builds are exercised
through fixtures and a spy dispatcher offline, and through a live surface where an
OpenRouter key is present (skip-if-absent, never mocked).

**RED → GREEN scenarios per milestone (intent only, no code):**

1. **M1 verdict round-trips.**
   - RED: `verdict_repo_round_trips`, append a Code Reviewer fail verdict then list it;
     fails because `VerdictRepo` / table do not exist.
   - GREEN: add migration `0009`, `VerdictRepo`, and the verdict part variant.
   - Artifact: `cargo test -p db` RED→GREEN; `sqlite3 <db> ".schema verification_verdict"`.

2. **M2 reviewer off by default.**
   - RED: `catalog_excludes_reviewer_when_disabled` and
     `catalog_includes_reviewer_when_enabled`; both fail because no reviewer is in the
     roster (`custom_agents.rs:601-613`).
   - GREEN: register the four reviewers; gate on `reviewers_enabled`.
   - Artifact: test RED→GREEN; a `task`-target listing capture showing reviewers absent
     when disabled and present when enabled.

3. **M3 held-out check gates before review.**
   - RED: `held_out_red_skips_reviewer`, feed a change passing L0-L1 but failing an L2
     case; assert `LadderOutcome` red at L2 and the reviewer dispatcher (spy) is never
     called. Fails: no L2 driver.
   - GREEN: implement the held-out runner in `verification.rs` and its short-circuit.
   - Artifact: test RED→GREEN asserting zero reviewer dispatches.

4. **M4 Code Reviewer fails on dead weight + swallowed failure.**
   - RED: `code_reviewer_fails_dead_and_swallowed`, hand a fixture diff with an unused
     helper and a dropped error; assert a fail verdict naming both at `file:line` and
     that merge is blocked. Fails: no L3 driver / reviewer.
   - GREEN: implement `reviewers.rs` L3 path + the gate in `runner.rs` `check`.
   - Artifact: test RED→GREEN; the rendered verdict compared to the reviewer-output
     panel shape.

5. **M5 QA Validator: un-run is not done.**
   - RED: `qa_validator_blocks_without_artifact`, assert a behavior with a captured
     artifact is pass and the same behavior with none is blocked; and
     `process_check_fails_unbacked_claim` against a fixture trajectory. Fails: no L4
     driver.
   - GREEN: implement the L4 path + trajectory reconciliation over doc 02's substrate.
   - Artifact: test RED→GREEN; a live `cortex run --jsonl` capture (key present) showing
     the L4 verdict event; `sqlite3` dump of L3+L4 verdict rows.

6. **M6 surfaces match the mockups.**
   - RED: snapshot tests `ladder_view_masks_l2_l4` and `reviewer_output_dual_panels`
     compared to the two mockup files; fail until the views exist.
   - GREEN: implement the TUI panels.
   - Artifact: snapshot diff clean against `plans/assets/mockups/verification-ladder.txt`
     and `plans/assets/mockups/reviewer-output.txt`.

**Ladder mapping for this doc's own changes:** L0 = `cargo build` + `cargo clippy -- -D
warnings` + `cargo fmt --check` across `engine db protocol memory settings tools
subagent_runner`; L1 = the `cargo test` suites above. The held-out rungs this doc builds
(L2-L4) are the product feature, exercised through fixtures/spies offline and a live
surface where a key is present.

---

## 7. Wave / Dependency Position

**Upstream (must land first):**
- **`02-engine-7step-loop.md`**: supplies the `check` step (where reviewers fire) and the
  trajectory + Session Log substrate the Level 4 process check reconciles against. M3-M5
  block on it; M1-M2 (schema + roster) do not.
- **`06-subagents-skills.md`**: supplies the Subagents dispatch model under which the four
  reviewers are registered and fanned out. M2, M4, M5 consume its dispatch contract.

**Downstream (blocked on this doc):**
- The Intent Scorer plan consumes reviewer verdicts and the trajectory alongside its own
  repo-readiness scoring; the verdict substrate (M1) and the L3/L4 outcomes (M4/M5) are
  its inputs. Referenced by filename only; not built here.

**Wave ordering:**
- Wave A: M1 (verdict substrate) and M2 (roster registration, off by default); no
  dependency on the loop, can start in parallel with `02-engine-7step-loop.md`.
- Wave B: M3 (held-out checks), then M4 and M5 once doc 02's `check` step and trajectory
  land; M4/M5 share the reviewer gate and can proceed in parallel after M3.
- Wave C: M6 surfaces, and hand the verdict + ladder-outcome contract to the Intent
  Scorer plan.

---

## 8. Risks & Mitigations

- **R1 Held-out leakage.** If the held-out cases (L2) or verdicts ever enter the
  model-facing context, the "earned, not gamed" contract breaks. Mitigation: keep the
  held-out fixtures and the driver strictly core-side; M3 AC2 asserts none appear in the
  rendered prompt/messages; verdicts persist only after submit.
- **R2 Reviewer surface-blurring.** A Code Reviewer that starts running the surface, or a
  QA Validator that starts reading code, collapses one-term-one-surface. Mitigation: the
  two drivers take disjoint inputs (diff/report vs behaviors/isolation boundary) and the
  shipped prompt bodies enforce the boundary (`code-reviewer.md` lines 22-26,
  `qa-validator.md` lines 16-22); a test asserts each reviewer's tool whitelist matches
  its surface.
- **R3 Cost of always-on reviewers.** Spawning reviewers on every trivial change wastes
  agents against the off-by-default principle. Mitigation: `reviewers_enabled` defaults
  off and `reviewer_fanout_threshold` gates fan-out to changes large enough to warrant it
  (`code-reviewer.md` lines 66-73); small changes review in one pass or not at all.
- **R4 Gate too soft.** A verdict softened into a suggestion lets a failing change merge.
  Mitigation: the gate is binary; any FAIL blocks merge and re-posts (mockup line 18);
  M4 AC3 asserts no merge event on fail; a "looks good but" is encoded as fail.
- **R5 Migration ordering.** `0009` must slot cleanly after the current `0008` and not
  collide with doc 02's own new migration if both land in the same window. Mitigation:
  coordinate the next free migration number with doc 02 at integration; each migration is
  idempotent and independently reversible (M1 AC1).
- **R6 Flaky QA behavior validation.** Real-surface execution can flake on broken
  infrastructure. Mitigation: the QA Validator marks affected behaviors **blocked** after
  non-disruptive recovery and never tampers with shared infrastructure
  (`qa-validator.md` lines 52-54), so a flake is reported honestly rather than silently
  passed.

---

## 9. Commit Sequence

Commits are authored and committed as `echobt` (per the commit-identity skill). One
logical commit per milestone; substrate and off-by-default gating land before the code
that uses them. Each commit leaves `cargo build`, `cargo clippy -- -D warnings`, `cargo
fmt --check`, and the relevant `cargo test` suites green (Verification Levels 0-1) before
the next begins.

1. `db,protocol: add 0009 verification verdict table + VerdictRepo + verdict part (M1)`,
   migration, repo, `MessagePartBody` variant, re-exports.
2. `subagent_runner,settings: register reviewers off by default (M2)`, four built-in
   reviewer agents with correct availability, `reviewers_enabled` + threshold keys,
   catalog gating.
3. `engine: held-out Level 2 checks in the check step (M3)`, `verification.rs` driver,
   held-out runner + config, short-circuit before review.
4. `engine: Code Reviewer intent audit + fan-out + gate (M4)`, `reviewers.rs` L3 path,
   feature fan-out, the strict FAIL-blocks-merge gate in `runner.rs`.
5. `engine,memory: QA Validator process check + verdict persistence (M5)`, L4 path,
   trajectory reconciliation over doc 02's substrate, verdict rows + Session Log append.
6. `tui,cli: ladder + reviewer-output surfaces (M6)`, masked/visible ladder view and dual
   reviewer-output panels, run/JSONL verdict surfacing, snapshot tests against the two
   mockups.
