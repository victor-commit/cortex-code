# 07 — Intent Scorer: Intent Scoring, Effectiveness Report, Repo Readiness Signals + Report

> Planning doc. Produces no product code by itself. Implementation lands via the commit
> sequence in §9. All terminology is drawn from `docs/rename/naming-map.md` and `GLOSSARY.md`;
> no new terms are coined here. The frozen migration renamed Arbiter → **Intent Scorer** and
> Seaworthiness → **Repo Readiness**; only the new names are live terms below, and the old
> names appear only in "was X" migration notes.

## 0. Scope & Contracts

### 0.1 What this doc plans

Three coupled evaluators that judge finished work, all downstream of and independent from
the builder:

1. **Intent scoring** — a five-dimension completion rubric that answers the one question
   test pass/fail cannot: *did this change actually do what the user asked for?* It emits a
   binary **PASS / FAIL** with per-dimension sub-scores and evidence, never a bare number.
2. **Effectiveness report** — the aggregation template that folds the Intent Scorer's intent
   score, the Reviewers' findings, and (for repo-level runs) the Repo Readiness result into
   one readable verdict. It makes no research claim of its own; it is a format sheet.
3. **Repo Readiness signals + report** — a catalog of mechanically-decidable readiness
   checks plus the harness-first report that evaluates a *repository* (not a diff, not a
   running feature) and rolls the signals into a maturity picture with a ranked gap list.

### 0.2 Prototype sources (authoritative; cited, not copied)

Read-only reference tree `/cortex-code-prototype`:

| Contract | Source | What it binds |
|---|---|---|
| The five-dimension rubric, gates, threshold, procedure, worked example | `prompts/intent-scorer/intent-scoring.md` | D1 Intent fidelity (gating) · D2 Execution-verified · D3 Scope discipline (gating) · D4 Report honesty (gating) · D5 Reliability; verdict = gates first, then sum ≥ 8 with no dim < 1 |
| Aggregation template + section order | `prompts/intent-scorer/effectiveness-report.md` | Header · Overall verdict (verdict-first) · Intent Scorer table · Reviewers · Repo Readiness · Required actions · Evidence index; a FAIL from any gating source is not averaged away |
| The 82-signal readiness catalog | `prompts/intent-scorer/repo-readiness-signals.md` | 44 repository + 38 application signals; each row = signal / scope / check / PASS-when / detect / remedy; `N/A` excluded from its group score |
| Harness-first readiness framing + aggregation stance | `prompts/intent-scorer/repo-readiness-report.md` | Repo measured by how much of the agent's harness it supplies; weight toward build/tests/memory/containment; per-group standing, never one collapsed grade |
| Canonical vocabulary | `GLOSSARY.md`, `docs/rename/naming-map.md` | Intent Scorer, Repo Readiness report, Repo Readiness signals, Reviewers (Code Reviewer + QA Validator), Session Log, CORTEX.md, Verification, Subagents, worker, Workflow |

### 0.3 Locked contracts this plan MUST satisfy

- **C-SCORE-1** — The Intent Scorer scores against the *restated ask*, not against whatever
  the diff happens to accomplish. Each dimension is `0 / 1 / 2` (`absent` / `partial` /
  `full`) with quoted evidence (`file:line`, run-log line, report sentence); a score without
  evidence is invalid (`intent-scoring.md`).
- **C-SCORE-2** — **D1, D3, D4 are gating.** Any gate at `0` ⇒ **FAIL** regardless of total.
  Otherwise **PASS requires total ≥ 8 with no dimension below 1**; a single `1` is tolerated
  only on a non-gating dimension (D2 or D5) and must be named on the pass. A "looks good
  but…" is a FAIL, not a soft pass (`intent-scoring.md`).
- **C-SCORE-3** — The verdict is **PASS / FAIL**, never a bare number that hides where the
  change is weak; a FAIL names the failed dimension and the concrete remediation.
- **C-REPORT-1** — The effectiveness report states the verdict **first**, then the detail.
  A FAIL from any gating source (Intent Scorer gate tripped, a Code Reviewer fail, a QA
  Validator that did not reproduce the asked-for behavior) makes the overall result FAIL
  (`effectiveness-report.md`).
- **C-REPORT-2** — Omit a section only when its evaluator did not run, and say so explicitly
  rather than leaving a silent gap; every verdict's evidence appears in the evidence index.
- **C-READY-1** — Repo Readiness evaluates the **repository**, not a diff (Code Reviewer) and
  not a running surface (QA Validator). One-term-one-surface holds (`GLOSSARY.md`).
- **C-READY-2** — Every signal is decided **PASS / FAIL / N/A** with cited evidence; a PASS is
  never inferred from the *absence* of a failure; `N/A` is excluded from its group's score,
  never counted as a failure (`repo-readiness-report.md`).
- **C-READY-3** — Report a per-group standing, weighted **harness-first** (build/tests/memory/
  containment dominate), never one collapsed grade; every recommendation is actionable and
  names signal → observed gap → next step.
- **C-INPUT-1** — The Intent Scorer reads the **original request**, the **change**, the
  **captured run evidence**, and the builder's **closing report**; it never authors the fix.
  Its structured inputs are the trajectory and Session Log produced by the 7-step loop.

### 0.4 Non-goals (deferred to sibling docs)

- The trajectory + Session Log **record substrate** itself — **doc 02** (`02-engine-7step-loop.md`).
  This doc *consumes* those artifacts; it does not define them.
- The **Reviewers** (Code Reviewer, QA Validator) prompt content and their verifier flow — a
  reviewers doc; this doc only *aggregates* their verdicts in the effectiveness report.
- The **Verification** ladder (levels 0–4) — a verification doc; the Intent Scorer is the
  level-3-style intent audit consumer, not the ladder's owner.
- Concurrent execution of the scorer as a nested worker — the primitive is **doc 06**
  (`06-subagents-skills.md`); the concurrency envelope is **doc 04** (`04-concurrency.md`).

---

## 1. Current State (file:line)

**The Intent Scorer subsystem is ABSENT today.** There is no intent scoring, no
effectiveness report, no repo-readiness signals or report anywhere under `crates/`. A
targeted grep confirms it:

- `rg -ni "intent scorer|readiness|effectiveness|arbiter|seaworthiness" crates/` returns
  **only** unrelated `InstallReadiness` LSP-server states (`crates/lsp/src/feedback.rs:1160`,
  `:1238`) — nothing about repository or intent readiness.
- `rg -ni "\bscor(e|ing|er)\b" crates/` returns **only** fuzzy-match scoring for edit
  anchoring (`crates/tools/src/edit.rs:572`, `:673`) and file-mention ranking
  (`crates/tui/src/mention.rs:233`, `:265`). Neither is intent completion scoring.

So every §2 surface is new. What follows are the **nearest existing surfaces** the scorer
will read from or hook into, each verified against the tree at authoring time.

### 1.1 Where a post-turn score would trigger

- `crates/engine/src/runner.rs:275` — `SessionRunner::run_inner`, the turn body.
  `crates/engine/src/runner.rs:309` opens `'turn: loop`; the single terminal
  `crates/engine/src/runner.rs:655` `sink(RunnerEvent::Done { … })` is where a run finishes.
  This is the natural point after which an intent score would be requested. **ABSENT — no
  match under `crates/` for any post-turn scoring hook at this site.**
- `crates/engine/src/event.rs:90` — `RunnerEvent` enum: `ToolResult` (`:116`), `Usage`
  (`:137`), `Done` (`:164`). There is **no** `IntentScore`, `EffectivenessReport`, or
  `RepoReadiness` event variant.

### 1.2 The structured inputs the scorer needs (owned by doc 02)

- **Trajectory: ABSENT.** `crates/engine/src/` contains `runner.rs`, `event.rs`,
  `compaction.rs`, `history.rs`, `message_parts.rs`, … but **no `trajectory.rs` and no
  `reminders.rs`.** DB migrations stop at `crates/db/src/migrations/0008_projects_accounts.sql`
  — **there is no `trajectory` table.** The trajectory + Session Log substrate is planned by
  **doc 02** (`02-engine-7step-loop.md` §2.4, M5, migration `0009_trajectory.sql`); this doc
  reads it once it exists (C-INPUT-1).
- **Session Log: ABSENT.** `crates/memory/src/lib.rs:42` is `ProjectMemory` (instruction-file
  discovery + `render` at `:75`) and `crates/memory/src/lib.rs:96` `DynamicMemoryTracker`.
  There is **no Session Log reader/writer** — the per-workspace rolling record the scorer
  records its verdict to is added by doc 02 (`MEMORY.md` §3).
- **Persisted parts.** `crates/protocol/src/message.rs:72` `MessagePartBody` variants:
  `Text`, `Reasoning`, `Tool`, `Step` (`:94`), `Patch` (`:110`), `Snapshot` (`:138`),
  `Usage` (`:155`). There is **no** `IntentScore` / `Report` part variant to persist a
  verdict as a first-class message part.

### 1.3 The only per-session evaluative record that exists today

- `crates/db/src/usage.rs:16` `UsageRecord`; `crates/db/src/usage.rs:62` `UsageRepo` with
  `record` (`:76`) and `session_totals` (`:120`). Token/cost telemetry is the *only* thing
  scored-per-session today. An intent score is a different, richer per-session artifact and
  does not belong in `usage`; it needs its own store (see §4). **ABSENT — no match for an
  intent-score or report repo.**
- `crates/db/src/session.rs:48` `SessionRepo`: `create` (`:62`), `create_child` (`:215`),
  `children` (`:229`). `create_child` is the existing seam for spawning the scorer as its own
  child session (the independent-evaluator shape) without polluting the builder's session.
- `crates/db/src/message.rs:44` `MessageRepo`: `create_with_parts` (`:73`),
  `list_for_session` (`:115`) — how the scorer would read the change conversation.

### 1.4 Where the scorer registers as an evaluator

- `crates/engine/src/agent.rs:13` `Agent { Build, Plan, General, Explore }`; `ALL` (`:30`),
  `SUBAGENTS: [Agent;3] = [Plan, General, Explore]` (`:34`). There is **no** `intent-scorer`
  or `repo-readiness` persona — the Reviewers/Intent-Scorer family of evaluators does not
  exist as an agent yet.
- `crates/subagent_runner/src/catalog.rs:44` `permitted_catalog` and `:28` `runner()` — the
  registration point where a scorer/evaluator subagent would be offered. `custom_agents.rs`
  `discover` (`crates/subagent_runner/src/custom_agents.rs:182`) discovers config-defined
  agents; a repo-committed scorer role would surface here. **ABSENT — no reviewer/evaluator
  agent is registered.**

### 1.5 The nearest report-surface stub

- `crates/tui/src/view.rs:41` `ViewKind`, with `ViewKind::Review` (`:67`), crumb `"/review"`
  (`:80`), title `"Code review"` (`:94`); slash entry `"Run a code review"` at
  `crates/tui/src/slash.rs:138`. This is a **UI stub label only** — there is no reviewer
  engine behind it and **no** effectiveness-report, intent-score, or repo-readiness view. It
  is the closest existing place an effectiveness report would eventually render.
- `crates/events/src/families.rs:183` `SessionStatusEvent` and `crates/events/src/event.rs:74`
  `LifecyclePhase` / `:94` `Lifecycle` — the session/turn lifecycle bus a "score ready" event
  would join. No score/report lifecycle event exists.

**Net:** the substrate exposes a clean turn-completion point (`runner.rs:655`), a child-session
seam (`session.rs:215`), a per-session record pattern (`usage.rs`), an evaluator registration
point (`catalog.rs:44`), and a report-view stub (`view.rs:67`) — but the scoring rubric, the
report format, the readiness catalog, and their storage are all ABSENT. The scorer's two
structured inputs (trajectory, Session Log) are being built by doc 02.

---

## 2. Target Design

The Intent Scorer is an **independent evaluator**: it reads the request, the change, the
captured run evidence, and the closing report, scores them against a fixed rubric, and emits
a binary verdict with evidence. It never authors the fix (C-INPUT-1). It runs as its own
evaluator session (reusing `session.rs:215` `create_child`) so its judgment is separated from
the builder's context.

### 2.1 Intent scoring (the rubric engine)

A new crate-local module holds the five-dimension rubric exactly as
`prompts/intent-scorer/intent-scoring.md` fixes it. The scorer first **restates the ask** in
one sentence (recording any assumption made on ambiguity), then scores each dimension `0/1/2`
with quoted evidence (C-SCORE-1):

| Dim | Name | Gating | Asks |
|---|---|---|---|
| D1 | Intent fidelity | **yes** | Does the change make the requested thing true *as the user meant it*, visible in something other than a test the author could have fitted? |
| D2 | Execution-verified | no | Was the changed surface actually run, and did the run show the intended behavior? |
| D3 | Scope discipline | **yes** | Did the change stay inside the ask — nothing required missing, nothing unrequested added? |
| D4 | Report honesty | **yes** | Does the closing report match what the change and run evidence actually show? |
| D5 | Reliability | no | Would the result hold under reasonable perturbation (renames, reordering, boundary inputs), or is it fitted to one path? |

**Verdict computation (C-SCORE-2):** gates first — if D1, D3, or D4 is `0`, the verdict is
**FAIL** regardless of the total. Otherwise sum all five (0–10): **PASS requires total ≥ 8
with no dimension below 1**; a lone `1` is allowed only on D2 or D5 and is named on the pass.
The output is **PASS / FAIL** (C-SCORE-3) plus the five sub-scores, a one-line reason, and —
on FAIL — a concrete remediation list. Each score carries its evidence pointer; a score
without evidence is emitted as `unknown`, never rounded up.

The scorer's structured inputs are the doc-02 **trajectory** (per-step action + reason + gate
decision + act result) and the **Session Log** (decisions, findings tied to `file:line`,
verification results). The trajectory is precisely the "captured run evidence" and the
"closing report vs artifacts" reconciliation the rubric requires — D2 and D4 read it directly.

### 2.2 Effectiveness report (the aggregation template)

A structured report type mirrors the section order of
`prompts/intent-scorer/effectiveness-report.md`, verdict-first (C-REPORT-1):

1. **Header** — work item, commit/range, evaluated-at, which evaluators ran.
2. **Overall verdict** — `PASS | FAIL | PARTIAL`, one-line reason, and the gating failure (if
   any). A FAIL from any gating source is not averaged away.
3. **Intent Scorer** — the D1–D5 table with scores + evidence, total `/10`, and the scorer's
   PASS/FAIL.
4. **Reviewers** — Code Reviewer verdict + findings, QA Validator verdict + what was exercised
   + observed-vs-expected. Rendered `not run` explicitly when a reviewer did not run (C-REPORT-2).
5. **Repo Readiness** — included only for repo-level runs: standing, per-group pass rates,
   top gaps.
6. **Required actions** — ordered, concrete, owned; empty only when the overall verdict is PASS.
7. **Evidence index** — every artifact referenced above, so the report is self-contained; a
   verdict whose evidence is not indexed is treated as unbacked (C-REPORT-2).

The completed report is recorded to the Session Log; a re-evaluation reuses the template and
re-checks only items that were not already PASS. Its rendered form is the report UI mocked in
[`plans/assets/mockups/intent-scorer-report.txt`](assets/mockups/intent-scorer-report.txt),
which shows the exact shape this doc targets: a `0 miss / 1 partial / 2 full` dimensions table
with a per-dimension `gate` column, a `total N/10` bar, and two headline verdicts —
`effectiveness [PASS/FAIL/PARTIAL]` and `repo-readiness [PASS/FAIL/N/A/unknown]` — over the
footer rule "PASS requires total ≥ 8 AND every gate dimension ≥ 1". The mockup is the binding
visual contract for the TUI surface in §5 M5.

### 2.3 Repo Readiness signals (the check catalog)

A committed catalog encodes the **82 signals** from
`prompts/intent-scorer/repo-readiness-signals.md` — **44 repository** + **38 application** —
each carrying: signal, scope (`repository` / `application`), what it checks, PASS condition,
detection method, and suggested remedy. Repository signals are evaluated once for the whole
repo; application signals are evaluated per shippable service/package. A signal that genuinely
cannot apply is marked `N/A` and excluded from its group's score (C-READY-2). The catalog is
data, not prose to be restated in the report.

### 2.4 Repo Readiness report (the harness-first evaluator)

The Repo Readiness report evaluates the **repository itself** — whether, if an agent were
dropped in tomorrow, the repo supplies the harness the agent needs (build, runnable tests,
durable memory, honest verification) or leaves it to flail. Its loop
(`repo-readiness-report.md`): scope the repo → evaluate every applicable signal as
**PASS/FAIL/N/A** with cited evidence → group and roll up by scope and maturity tier →
**rank the gaps by how much they block agentic work** → emit one readiness picture plus a
prioritized, actionable gap list. Maturity tiers (table-stakes → infrastructure → hardening →
autonomy-enabling) are a **map, not a gate**; the judgment is weighted harness-first so a repo
strong on build/tests/memory/containment is workable even with peripheral gaps, and one weak
on those is not ready however many peripheral signals pass (C-READY-3). Standing is reported
per group, never as one collapsed grade; unknowns are named, not rounded to PASS.

### 2.5 Boundary (one-term-one-surface)

Three distinct surfaces, never conflated (C-READY-1, `GLOSSARY.md`): **Code Reviewer**
scrutinizes the *code and change*; **QA Validator** validates *running behavior*; **Repo
Readiness report** evaluates the *repository*. Code Reviewer and QA Validator are Reviewers;
the Repo Readiness report and the intent score are the Intent-Scorer-side surfaces. The
effectiveness report is the one place all four meet.

---

## 3. Gap Analysis

| # | Contract | Current | Gap to close |
|---|---|---|---|
| G1 | C-SCORE-1/2/3 (rubric + gates + binary verdict) | No scoring rubric exists (`rg` §1) | New rubric module: restate ask, score D1–D5 with evidence, apply gates then threshold, emit PASS/FAIL + remediation |
| G2 | C-INPUT-1 (reads trajectory + Session Log) | Trajectory + Session Log ABSENT (doc 02 owns them; migrations stop at `0008`) | Consume doc-02 `TrajectoryRepo` + Session Log reader; block scorer integration on doc-02 M5 |
| G3 | Persist a verdict as a first-class artifact | `MessagePartBody` (`message.rs:72`) has no score/report variant; only `usage` records per session | New DB table + repo for intent scores / effectiveness reports (mirrors `UsageRepo` shape) |
| G4 | Trigger scoring at turn completion | Turn ends at `runner.rs:655` `Done`; no scoring hook | Add a post-`Done` evaluation hook + a `RunnerEvent` score/report variant (`event.rs:90`) |
| G5 | Run scorer as an independent evaluator | `create_child` exists (`session.rs:215`) but no evaluator persona (`agent.rs:34`) or catalog entry (`catalog.rs:44`) | Register a repo-committed evaluator role; spawn it as a child session; it never authors the fix |
| G6 | C-READY-1/2/3 (readiness catalog + harness-first report) | No signal catalog, no readiness report (`rg` §1) | Commit the 82-signal catalog as data; new report builder: evaluate → group → rank gaps |
| G7 | C-REPORT-1/2 (verdict-first aggregation) | No report type; nearest is the `/review` UI stub (`view.rs:67`) | New effectiveness-report type folding intent score + Reviewers + Repo Readiness; explicit `not run` sections |
| G8 | Render the report | `/review` view is a label with no engine (`view.rs:67`, `slash.rs:138`) | A report view matching `assets/mockups/intent-scorer-report.txt`; a "score ready" lifecycle event |
| G9 | Concurrency-safe execution | Scorer must be able to run concurrently (doc 04) | Scorer session is read-only over the change; write only its own verdict rows — no contention with builder writes |

Genuine gaps (nothing exists to fold): the rubric, the report template, the 82-signal
catalog, the readiness report, their storage, and the evaluator persona are all net-new. The
only reuse is structural: `create_child` for the evaluator session, the `UsageRepo` shape for
the verdict store, the `Done` turn-completion point for the trigger, and the doc-02 trajectory
for the inputs.

---

## 4. Crate / File Change Map

Every path below is grounded in the real crate layout (`ls crates/`); no new crate is
introduced. This doc edits **no product code** — it specifies the map only.

| File | New / Extend | What changes |
|---|---|---|
| `crates/engine/src/intent_scorer.rs` | **new** | The rubric engine: restate-ask, `Dimension`/`Score` types, D1–D5 scoring, gate-then-threshold verdict (`IntentVerdict::{Pass,Fail}` + sub-scores + evidence + remediation). Reads the doc-02 trajectory + Session Log; authors no fix. |
| `crates/engine/src/effectiveness_report.rs` | **new** | The aggregation type mirroring `effectiveness-report.md`: header, overall verdict (verdict-first), intent-score table, Reviewers block, Repo Readiness block, required actions, evidence index. Explicit `not run` markers (C-REPORT-2). |
| `crates/engine/src/repo_readiness.rs` | **new** | The harness-first report builder: scope repo → evaluate signals → group + roll up → rank gaps → emit standing + gap list. Reads the signal catalog. |
| `crates/engine/src/repo_readiness_signals.rs` | **new** | The 82-signal catalog as data (44 repository + 38 application): `Signal { id, scope, checks, pass_when, detect, remedy }`. Sourced verbatim from `repo-readiness-signals.md`; no signal invented. |
| `crates/engine/src/agent.rs` | **extend** | Register the evaluator persona alongside `Agent { Build, Plan, General, Explore }` (`:13`, `ALL` `:30`, `SUBAGENTS` `:34`) so the scorer runs as its own role that never edits. |
| `crates/engine/src/event.rs` | **extend** | Add a score/report `RunnerEvent` variant (enum at `:90`, near `Done` `:164`) so a completed verdict surfaces on the stream. |
| `crates/engine/src/runner.rs` | **extend** | Add the post-`Done` evaluation hook after `sink(RunnerEvent::Done { … })` (`:655`) that requests an intent score for the finished turn (opt-in; never blocks the builder's turn). |
| `crates/engine/src/lib.rs` | **extend** | `mod intent_scorer; mod effectiveness_report; mod repo_readiness; mod repo_readiness_signals;` + re-exports. |
| `crates/protocol/src/message.rs` | **extend** | Add an `IntentScore` / `EffectivenessReport` `MessagePartBody` variant (list at `:72`) so a verdict persists as a first-class part. |
| `crates/db/src/migrations/0010_intent_scores.sql` | **new** | `intent_scores` (+ `effectiveness_reports`) table(s): `session_id`, subject ref, D1–D5, gate flags, total, verdict, reason, evidence refs, `created_at`; indexed by `session_id`. Follows the current head `0008_projects_accounts.sql` — **doc 02 claims `0009_trajectory.sql`, so this doc takes `0010`** (see §8 R-4). |
| `crates/db/src/intent_score.rs` | **new** | `IntentScoreRepo` (`record`, `get_for_session`, `list_for_session`), mirroring `UsageRepo` shape (`usage.rs:62`) and `MessageRepo` shape (`message.rs:44`). |
| `crates/db/src/lib.rs` + `crates/db/src/migrations.rs` | **extend** | Register the `intent_score` module + re-export; add migration `0010` to the ordered set. |
| `crates/subagent_runner/src/catalog.rs` | **extend** | `permitted_catalog` (`:44`) offers the evaluator role so it can be spawned as a child evaluator session (`session.rs:215`); `runner()` (`:28`) threads any evaluator config. |
| `crates/subagent_runner/src/custom_agents.rs` | **extend** | Let a repo-committed evaluator role surface through `discover` (`:182`) so a project can pin its scorer. |
| `crates/memory/src/lib.rs` | **extend (consumes doc 02)** | Read the Session Log to record the verdict + evidence and re-check-only-what-changed on re-score. Depends on the doc-02 Session Log writer; no independent Session Log defined here. |
| `crates/events/src/event.rs` + `crates/events/src/families.rs` | **extend** | Add a "score/report ready" lifecycle event beside `SessionStatusEvent` (`families.rs:183`) / `Lifecycle` (`event.rs:94`) for the bus/SSE. |
| `crates/tui/src/view.rs` + `crates/tui/src/slash.rs` | **extend** | Add an effectiveness-report view beside `ViewKind::Review` (`view.rs:67`) rendering the layout in `assets/mockups/intent-scorer-report.txt`; a slash entry beside `slash.rs:138`. |
| `crates/settings/src/schema.rs` | **extend** | Config for scoring: enable/disable post-turn scoring, whether repo-readiness runs for repo-level jobs, evaluator model. Conservative defaults (off unless requested). |

No file outside this map is edited. `crates/` is not touched by this planning doc itself.

---

## 5. Milestones

Each milestone ends with a machine-checkable acceptance gate (a command + a binary observable).

- **M1 — Rubric engine (pure, offline).**
  `intent_scorer.rs`: restate-ask, D1–D5 scoring types, gate-then-threshold verdict. No I/O;
  scores a fixture `(request, change, evidence, report)`.
  **Accept:** `cargo test -p engine intent_rubric` green; unit tests prove the worked example
  from `intent-scoring.md` (D4=0 gate ⇒ FAIL even at total 6) and the threshold boundary
  (total 8, no dim < 1 ⇒ PASS; a `1` on D1 ⇒ still evaluated by gate). `cargo clippy -p engine
  -- -D warnings` clean.

- **M2 — Verdict persistence.**
  Migration `0010`, `IntentScoreRepo`, and the `MessagePartBody` variant. A scored turn writes
  one verdict row.
  **Accept:** `cargo test -p db intent_score_repo` green; `sqlite3 <db> "select verdict,total
  from intent_scores where session_id=?"` returns the recorded verdict; migration `0010`
  applies idempotently on a fresh and an existing db (parity with `db/src/lib.rs` open
  contract).

- **M3 — Scorer as an independent evaluator over the trajectory (consumes doc 02).**
  Register the evaluator persona (`agent.rs`), spawn it as a child session (`session.rs:215`),
  read the doc-02 trajectory + Session Log as its evidence, and add the post-`Done` hook
  (`runner.rs:655`) + score `RunnerEvent`.
  **Accept:** `cargo test -p engine scorer_reads_trajectory` green — a fixture 3-step
  trajectory yields a verdict whose D2/D4 evidence pointers reference real trajectory entries;
  the evaluator session performs **no writes to the change** (asserted via a read-only spy).

- **M4 — Repo Readiness signals + report.**
  The 82-signal catalog as data; the harness-first report builder (evaluate → group → rank).
  **Accept:** `cargo test -p engine repo_readiness_catalog` asserts exactly **82** signals
  (**44** repository + **38** application) with no duplicate id; `repo_readiness_report` green
  — a fixture repo yields per-group standings, an `N/A` signal is excluded from its group
  score (not counted as failure), and the gap list is ordered harness-first (a missing
  runnable-test signal ranks above a cosmetic one).

- **M5 — Effectiveness report + render.**
  The aggregation type (verdict-first, explicit `not run`) and the TUI report view matching
  the mockup.
  **Accept:** `cargo test -p engine effectiveness_verdict_first` green — a Code-Reviewer FAIL
  forces overall FAIL (not averaged); a not-run evaluator renders an explicit `not run`, never
  a silent gap; every cited artifact appears in the evidence index. A TUI snapshot test asserts
  the rendered report matches the structure of
  [`plans/assets/mockups/intent-scorer-report.txt`](assets/mockups/intent-scorer-report.txt)
  (dimensions table with gate column, `total N/10`, `effectiveness` + `repo-readiness`
  headline verdicts, footer rule).

- **M6 — Live end-to-end + concurrency (gated).**
  A live run scores a real finished turn; the scorer runs concurrently with follow-on builder
  work without contention (doc 04 envelope).
  **Accept:** `cargo test --workspace` green; `cargo clippy --workspace -- -D warnings` clean;
  a headless run captures a real effectiveness report artifact (path recorded); a concurrency
  test proves the evaluator session and a builder session interleave with no write conflict
  (scorer writes only `intent_scores` rows). LLM-dependent legs are key-gated and **skip** when
  the provider key is absent — never mocked to manufacture a green.

---

## 6. TDD Plan

Every behavior change lands test-first: write the failing test (RED) that fails for the
*right* reason, make the smallest change to pass (GREEN), then capture a real-surface artifact
at the appropriate agent-visible Verification level (L0–L1). LLM-dependent tests are gated
behind the provider key and **skipped when absent — never mocked** (offline tests use recorded
fixtures for the provider stream and in-repo trajectory fixtures, never a fake user-facing
surface).

**RED → GREEN scoring scenarios (each one atomic pair; test intent only, no code):**

| Test id | RED asserts | GREEN when |
|---|---|---|
| `intent_rubric::gate_zero_forces_fail` | a change with D1=2 D2=2 D3=2 D5=2 but **D4=0** scores PASS → fail | gate check makes any gating-dim `0` ⇒ FAIL regardless of total (C-SCORE-2) |
| `intent_rubric::worked_example_fail` | the `intent-scoring.md` rate-limit example (D1=1 D2=2 D3=2 D4=0 D5=1) scores PASS → fail | verdict = FAIL, decided by the D4 gate, total 6 (parity with the prototype worked example) |
| `intent_rubric::threshold_boundary` | total 8 with no dim < 1 scores FAIL, or a `1` on **D1** is tolerated → fail | PASS iff total ≥ 8, no dim < 1, and any lone `1` is on D2/D5 only, named on the pass |
| `intent_rubric::evidence_required` | a dimension scored with no evidence pointer is accepted → fail | a scoreless-evidence dimension is emitted `unknown`, never counted toward PASS (C-SCORE-1) |
| `scorer::reads_trajectory_not_authors` | the evaluator session issues a write to the change → fail | evaluator is read-only over the change; writes only its own verdict rows (C-INPUT-1, G9) |
| `scorer::d2_d4_cite_trajectory` | D2/D4 evidence pointers are empty on a real fixture trajectory → fail | D2 (execution-verified) and D4 (report-honesty) resolve to real trajectory entries |
| `intent_score_repo::roundtrip` | a scored turn has no persisted row → fail | `IntentScoreRepo::record` writes one row; `get_for_session` returns the verdict + sub-scores (M2) |
| `repo_readiness::catalog_is_82` | catalog length ≠ 82 or a duplicate id → fail | exactly 44 repository + 38 application signals, unique ids (C-READY-2) |
| `repo_readiness::na_excluded_from_score` | an `N/A` signal counts as a failure in its group → fail | `N/A` is excluded from its group's applicable-signal score, never a failure (C-READY-2) |
| `repo_readiness::gaps_ranked_harness_first` | gap list orders a cosmetic gap above a missing runnable-test signal → fail | gaps ranked by how much they block agentic work, harness-first (C-READY-3) |
| `effectiveness::verdict_first_and_gate` | a Code-Reviewer FAIL is averaged into an overall PASS → fail | any gating source FAIL ⇒ overall FAIL; verdict stated first (C-REPORT-1) |
| `effectiveness::not_run_is_explicit` | a not-run evaluator renders as a silent gap → fail | the section renders explicit `not run`; evidence index lists every cited artifact (C-REPORT-2) |

**Ladder mapping for this doc's own changes:** L0 = `cargo build -p engine -p db -p protocol
-p memory -p events -p tui` + `cargo clippy -- -D warnings` + `cargo fmt --check`; L1 = the
`cargo test` suites above. The scorer *is* the level-3-style intent audit for other work; the
Verification ladder itself is a sibling doc's scope. This doc's obligation is that its verdicts
are evidence-backed and persisted (M2/M3).

---

## 7. Wave / Dependency Position

```
doc 02 (7-step loop) ──────────────┐  (trajectory + Session Log = scorer inputs)
                                    ▼
doc 04 (concurrency) ─────────►  07 THIS DOC (Intent Scorer / Effectiveness / Repo Readiness)
                                    │
                          ┌─────────┴─────────┐
                          ▼                   ▼
                   effectiveness        Reviewers doc
                   report consumers      (Code Reviewer +
                   (CI / TUI)            QA Validator feed in)
```

- **Depends on `02-engine-7step-loop.md`** — the Intent Scorer reads the **loop trajectory**
  (per-step action + reason + gate decision + act result) and the Session Log as its structured
  evidence for D2 (execution-verified) and D4 (report honesty). M3 blocks on doc 02's record
  substrate (its M5 / migration `0009_trajectory.sql`); M1, M2, M4 are internal and do not.
- **Depends on `04-concurrency.md`** — the scorer **may run concurrently** with follow-on
  builder work as its own read-only evaluator session; M6's concurrency proof rides on the
  doc-04 execution envelope so the scorer's verdict writes never contend with builder writes.
- **Wave A:** M1 (rubric engine), M2 (persistence), M4 (readiness catalog + report) — all
  internal, start immediately in parallel.
- **Wave B:** M3 (scorer over the trajectory) after doc 02's trajectory lands; M5
  (effectiveness report + render) after M1/M4.
- **Wave C:** M6 (live end-to-end + concurrency) after doc 04's envelope lands; hand the
  effectiveness-report artifact contract to downstream consumers.

---

## 8. Risks & Mitigations

- **R-1 — Scoring against the diff instead of the ask.** The most common rubric failure is
  scoring what the change accomplished rather than what was requested. *Mitigation:* the
  restate-ask step is mandatory and its output is recorded; D1 tests assert the score tracks
  the restated ask, and the ambiguity-assumption is logged (C-SCORE-1).
- **R-2 — Evidence-free scores.** A dimension scored from reading alone silently degrades the
  verdict's trust. *Mitigation:* `intent_rubric::evidence_required` — a dimension with no
  `file:line` / run-log / report pointer is emitted `unknown`, never counted toward PASS.
- **R-3 — Gate averaging.** Aggregation could quietly average a gating FAIL into a soft pass.
  *Mitigation:* gates are applied *before* the threshold in code, and
  `effectiveness::verdict_first_and_gate` asserts a gating-source FAIL forces overall FAIL.
- **R-4 — Migration number collision with doc 02.** Doc 02 claims `0009_trajectory.sql`; a
  naive `0009` here would collide. *Mitigation:* this doc takes **`0010_intent_scores.sql`**;
  if doc 02 lands a different number, renumber to the next free slot before merge (single
  source of truth = the ordered `migrations.rs` set).
- **R-5 — Trajectory not yet available.** M3 depends on a doc-02 substrate that may lag.
  *Mitigation:* M1/M2/M4 are trajectory-independent and proceed first; M3 consumes a stable
  `TrajectoryRepo` interface and is fixture-tested against a recorded trajectory until the live
  one exists.
- **R-6 — Readiness signal drift.** The 82-signal catalog could diverge from
  `repo-readiness-signals.md` over time. *Mitigation:* `repo_readiness::catalog_is_82` pins the
  count and unique ids; signals are sourced verbatim and never invented.
- **R-7 — Scorer cost / latency on every turn.** Running a full evaluator per turn is
  expensive. *Mitigation:* post-turn scoring is opt-in via settings (default off), runs as a
  concurrent read-only session (doc 04), and repo-readiness runs only for repo-level jobs.
- **R-8 — Self-review incentive.** A scorer that shares the builder's context can rationalize.
  *Mitigation:* the scorer runs as an *independent* child session (`session.rs:215`) that reads
  only the change + trajectory + report and never authors the fix (C-INPUT-1).

---

## 9. Commit Sequence

Each commit is independently green (build + relevant tests) and maps to milestones in §5.
Commits are authored and committed as `echobt` (per the commit-identity skill).

1. **`engine: intent-scoring rubric engine with gates + threshold (M1)`** — `intent_scorer.rs`,
   restate-ask + D1–D5 + gate-then-threshold verdict; worked-example and boundary tests. No I/O.
2. **`db,protocol: add 0010 intent_scores table + IntentScoreRepo + part variant (M2)`** —
   migration `0010`, `intent_score.rs`, `MessagePartBody::IntentScore`; roundtrip test.
3. **`engine: repo-readiness signal catalog (44+38) as data (M4a)`** —
   `repo_readiness_signals.rs`; `catalog_is_82` + unique-id tests.
4. **`engine: harness-first repo-readiness report builder (M4b)`** — `repo_readiness.rs`,
   evaluate → group → rank; `na_excluded` + `gaps_ranked_harness_first` tests.
5. **`engine: register intent-scorer evaluator persona + score event (M3a)`** — `agent.rs`
   persona, `event.rs` score variant, `catalog.rs`/`custom_agents.rs` registration.
6. **`engine: scorer reads doc-02 trajectory as independent evaluator (M3b)`** — post-`Done`
   hook in `runner.rs`, child-session spawn, read-only-over-change proof; consumes doc 02.
7. **`engine: effectiveness report aggregation (verdict-first) (M5a)`** —
   `effectiveness_report.rs`; verdict-first + not-run-explicit + evidence-index tests.
8. **`tui,events: effectiveness-report view + score-ready lifecycle event (M5b)`** — report
   view matching `assets/mockups/intent-scorer-report.txt`, slash entry, bus event.
9. **`settings,engine: config + live/concurrency evidence for scoring (M6)`** — settings
   schema, key-gated live report capture, doc-04 concurrency proof; config docs.

> Each commit must leave `cargo build`, `cargo clippy -- -D warnings`, `cargo fmt --check`, and
> the touched-crate `cargo test` suites green (Verification levels 0–1) before the next begins.
