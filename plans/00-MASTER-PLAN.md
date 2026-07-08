# 00. MASTER: Plan-of-Plans for the Cortex Code Rebuild

> **What this document is.** The top-level index and execution strategy over the eight
> subsystem design docs in this directory. It sequences them, it does not re-specify them.
> Each `## N.` section below coordinates; the depth lives in the linked docs. No product
> Rust is written or implied here, and nothing under `crates/` is touched. This is the
> last design deliverable before implementation begins.
>
> **Scope discipline.** This file INDEXES and ORDERS the eight docs. Where a detail is
> owned by a subsystem doc, this doc points at it (`doc NN §K`) rather than restating it.

---

## 0. Mission & Locked Contracts

Grounded in [`../MISSION-BRIEF.md`](../MISSION-BRIEF.md) §0, §5, §8, §9, §15.

- **Greenfield rebuild.** Cortex Code is rebuilt from zero in `/projects/cortex-code` (a
  Rust workspace): one binary, three surfaces (interactive TUI, headless `cortex run`,
  local `cortex serve`). No incremental patching of prior art (brief §0.1, §1).
- **"SANS AUCUN MOCK" on every user-facing surface.** Everything a user can reach must
  work for real: real LLM calls, real tools, real MCP / LSP / plugins, a real server. The
  live path is the default whenever a key is present (`CORTEX_NO_NET` remains the
  kill-switch). Deterministic fixtures and cassettes are permitted **only inside tests**,
  never as shipped default behavior (brief §0.2, §3, §15).
- **OpenRouter is the reference LLM for live tests.** The architecture stays
  multi-provider, but mission validation exercises real agent turns against OpenRouter
  (key in `.env.local`, sourced per shell, never committed or logged; redaction active).
  Suggested test models: `google/gemini-2.5-flash` (fast/cheap) and
  `anthropic/claude-3.5-sonnet` (quality). Do not use `gemini-3.5-flash` (does not exist)
  (brief §0.5, §7, §9).
- **Naming is frozen.** All surface names are PICKED from `GLOSSARY.md` and
  `docs/rename/naming-map.md`. Build agents never coin domain terms; a missing term is
  flagged, not invented. The banned prior-art brand string appears nowhere (code, prompts,
  commits, docs), and the frozen nautical vocabulary is never used as a live name (brief
  §0.4, §8, §15). See §8 below.
- **Process and format hygiene.** Kill by exact PID only (no `pkill`/name kill); test
  ports live in 3100-3199; format on-touch only (`rustfmt --edition 2024 --skip-children`),
  never a global `cargo fmt` (brief §9.3, §9.4, §15).

---

## 1. Document Index

The eight subsystem docs, in build order. Each is DONE and verified; this master doc is
the coordinating layer over them.

| # | Doc | One-line purpose |
|---|-----|------------------|
| 01 | [Prompt + Mode System](./01-prompt-mode-system.md) | Program-owned ordered PromptBlock assembler, the five modes/surfaces as data, and the schema-level MODE GATE + refusal envelope. |
| 02 | [Engine: 7-step loop](./02-engine-7step-loop.md) | The program-owned `sense→decide→gate→act→check→remind→record` control loop plus the trajectory + Session Log record substrate. |
| 03 | [Workflow, Team & Task Board](./03-workflow-team-board.md) | Bounded multi-agent Workflow lifecycle, the persistent Team + Task Board, and deterministic core-owned runtime handoffs. |
| 04 | [Concurrency & parallelism](./04-concurrency.md) | Bounded parallel `act`, depth-limited nested Subagent fan-out, deterministic tree cancellation, backpressure, and fair scheduling. |
| 05 | [Reviewers + Verification ladder](./05-reviewers-verification.md) | The held-out Verification Levels 2-4 (held-out checks, intent audit, process check) and the Code Reviewer / QA Validator subagents + gate. |
| 06 | [Subagents & Skills](./06-subagents-skills.md) | Nested delegation (Agent Mode only), scoped capability inheritance, and Skill gating with most-restrictive composition. |
| 07 | [Intent Scorer](./07-intent-scorer.md) | Intent-completion scoring rubric, the effectiveness report, and the Repo Readiness signals + report (harness-first). |
| 08 | [CLI / TUI UI/UX](./08-cli-uiux-mockups.md) | The operator surface over docs 01-07: mode switch, workflow launch/monitor, team board, reviewer output, ladder, scorer report, metrics. |

---

## 2. Architecture Overview

The rebuild is organized around four load-bearing decisions. These are the substantive
differentiators (`ORIGINALITY-GUARD.md` §4); originality is carried by them, not by
invented words (§8).

**A. The program-owned 7-step loop** (owned by [doc 02](./02-engine-7step-loop.md) §2.1).
Each turn runs seven phases in order:

```
sense → decide → gate → act → check → remind → record
```

The model owns **only** `decide`: one provider round-trip that proposes exactly one action
plus a reason. The program owns everything else. `sense` assembles state (touched files,
last result, queued reminders, live-objective slice, compaction). `gate` validates the one
action against mode grant + plan scope + safety + budgets (doc 01). `act` executes the
single allowed action (its internal read fan-out is a doc-04 concurrency detail, not a
model-controlled batch). `check` inspects the real exit/output/diff and runs the
agent-visible ladder rungs. `remind` fires catalog reminder slices for the next `sense`.
`record` appends one trajectory entry + a Session Log delta. The model never drives the
outer loop and never declares its own "done."

**B. Tool-access-defined modes** (owned by [doc 01](./01-prompt-mode-system.md)). Modes
differ by the tool set the harness hands them, enforced at the schema level, not by prose:

- `00-core-identity` is always present (identity + honesty contract).
- Exactly one mode body per turn: **Plan Mode** (read-only) XOR **Edit Mode**
  (read + write + `run`) XOR **Agent Mode** (Edit grant + `spawn_subagent`).
- **Chat Mode**, **Headless**, and the **Orchestration mindset** (`06-orchestration-mindset`)
  are composing surfaces: they append to the active mode, they do not add a fourth mode.
- The **block assembly order is fixed and program-owned**:

  ```
  [00-core-identity][mode-body][instruction-blocks][runtime-context][task-reminder][tool-cards]
  ```

- The MODE GATE filters the exposed tool schema per mode; a disallowed call returns the
  three-field refusal envelope `{ status: "refused", payload: { reason }, truncated: false }`.
  Mode promotion is a **core-granted** event the model requests, never a tool it calls.

**C. The held-out Verification ladder** (Levels 0-4). Levels 0 (Deterministic) and 1
(Agent-visible tests) are worker-facing and run inside the loop's `check` step
([doc 02](./02-engine-7step-loop.md) §2.3). Levels **2-4 are held out from the model**
([doc 05](./05-reviewers-verification.md) §2.1): L2 held-out checks, L3 intent audit
(Code Reviewer), L4 process check (QA Validator + trajectory reconciliation). The worker
never sees L2-L4 cases or verdicts before submit, so scores are earned, not gamed.

**D. The reliability-vector success metric.** Success is a vector across repeated,
perturbed attempts (does the change hold, is it reversible, was it reported honestly), not
a single lucky green run. The [Intent Scorer](./07-intent-scorer.md) and the held-out
ladder together make "build to the test" detectable rather than rewarded.

Modes, loop, and ladder compose: the loop drives the assembler and calls the gate (doc 02
consumes doc 01); the gate classifies nesting and fan-out (doc 04 rides doc 01); the
record substrate feeds the held-out ladder and the scorer (docs 05/07 consume doc 02).

---

## 3. Global Dependency Graph

Waves group the docs by how far they sit from the substrate. Within a wave, intra-wave
edges still order the work. Inbound edges are taken from each doc's §7.

| Wave | Docs | Depends on (inbound, from §7) | Unblocks |
|------|------|-------------------------------|----------|
| **W1** | 01, 02, 06 | 01: none (foundational substrate). 02: 01 (gate, for its M2). 06: 01 (Agent-Mode signal) + 02 (bounded loop). | everything |
| **W2** | 04, 05, 07 | 04: 02 (`act`/abort/doom/record) + 01 (gate). 05: 02 (`check` + trajectory) + 06 (subagent dispatch). 07: 02 (trajectory + Session Log) + 04 (concurrent evaluator envelope). | 03, 08 |
| **W3** | 03 | 02 (worker turns are engine turns) + 04 (serial before parallel workers) + 05 (milestone-acceptance evidence) + 06 (worker = subagent). | 08 |
| **W4** | 08 | 01, 02, 03, 05, 06, 07 (renders all their surfaces; adds no core semantics). | none (closes the set) |

ASCII DAG (edges point from dependency to dependent):

```
                 01 (mode gate) ─────────────┬───────────────┬───────────────┐
                     │                        │               │               │
                     ▼                        ▼               ▼               ▼
 W1                 02 (7-step loop) ───────► 04 ───────►     05              06
                     │        │                │             ▲ │              │
                     │        └──────────────► 06 ──────────┘ │              │
                     │                          │             │              │
 W2                  ├───────────────────────► 05 ◄───────────┘              │
                     ├───────────────────────► 07 ◄──── 04                   │
                     │                                                        │
 W3                  └──► 03 ◄──── 04 ◄──── 05 ◄──── 06 ◄─────────────────────┘
                          │
 W4                       └──► 08 ◄──── {01,02,03,05,06,07}
```

Reading the intra-wave order: in **W1**, 01 lands first, 02 scaffolds in parallel and
consumes the gate at its M2, 06 follows once 01+02 are green. In **W2**, 04/05/07 build on
the W1 substrate and can proceed largely in parallel (04 and the internal legs of 05/07
start first; the trajectory-consuming legs wait on doc 02 M5). **W3** (03) needs 04's
cancellable worker primitive and 05's evidence model. **W4** (08) is the UI over all of
it.

---

## 4. Global Milestone Sequence

The per-doc §5 milestones merged into one cross-subsystem roadmap, ordered by wave and by
the dependency edges above. Each milestone keeps its home-doc id (e.g. `02.M5`) and its
own machine-checkable acceptance gate lives in that doc.

**Phase I - Substrate (W1).**

1. `01.M1` Block registry + fixed six-block order (assembler; golden regression lock).
2. `02.M1` Phase scaffolding around existing tool execution (parity, in parallel with 01.M1).
3. `01.M2` Mode bodies + surfaces as data (selection table + instruction-block matrix).
4. `01.M3` MODE GATE enforcement + three-field refusal envelope.
5. `02.M2` Gate integration (consumes `01.M3`) + step/wall-clock/edited-lines budgets.
6. `02.M3` Check phase (real result + L0-L1) + `detect → diagnose → fix`.
7. `02.M4` Reminder scheduler (11 catalog keys, three trigger classes).
8. `02.M5` Record → trajectory + Session Log. **Land early: shared contract for docs 05/07.**
9. `01.M4` CLI / TUI / slash mode selection + switching.
10. `06.M1-M2` Depth plumbing, then Agent-Mode-gated `task` advertisement.
11. `06.M3-M4` Bounded nested spawn + typed ceiling refusal; `specialized` is a leaf.
12. `06.M5-M7` Scoped MCP inheritance, Skill gating, most-restrictive composition.
13. `06.M8` CLI/TUI parity + live (gated) nested-delegation evidence.

**Phase II - Verification, Concurrency, Scoring (W2).**

14. `04.M1` Cancellation-aware `act` fan-out; `04.M5` doom-loop diagnostic (coordinated with `02.M3`).
15. `04.M2-M4` Depth-limited nested spawn, per-agent fan-out + global ceiling, deterministic tree cancellation + drain-on-cancel.
16. `04.M6` Concurrency event surface + dashboard feed.
17. `05.M1-M2` Verdict substrate (schema/repo/part), reviewers registered off by default.
18. `05.M3` Level 2 held-out checks (short-circuit before any reviewer).
19. `05.M4-M5` Level 3 Code Reviewer + fan-out; Level 4 QA Validator + fan-out; verdict persistence.
20. `05.M6` Ladder + reviewer-output surfaces.
21. `07.M1-M2-M4` Rubric engine, verdict persistence, Repo Readiness signals (44+38 = 82) + report (internal, parallel).
22. `07.M3` Scorer as independent read-only evaluator over the doc-02 trajectory.
23. `07.M5` Effectiveness report (verdict-first) + render.
24. `07.M6` Live end-to-end + concurrency proof (doc-04 envelope, key-gated).

**Phase III - Orchestration (W3).**

25. `03.M1-M2` Workflow spine types + append-only log; deterministic board reducer.
26. `03.M3` Orchestrator/Coordinator/Worker personas + typed briefing/result dispatch.
27. `03.M4` Workflow Preflight (Step-0 artifact verification) + branch/log/intents seeding.
28. `03.M5` Serial `run_workflow` runtime loop (parallelism deferred to doc 04's primitive).
29. `03.M6` Milestone acceptance validator (evidence hooks to doc 05).
30. `03.M7` Persistent Coordinator Team front-end over the board.

**Phase IV - Operator Surface (W4).**

31. `08.M1` Shared chrome + tool-envelope conformance.
32. `08.M2` Mode switch (over doc 01 grant).
33. `08.M3` Workflow launch + monitor (over doc 03).
34. `08.M4` Team board (inspection-only).
35. `08.M5` Reviewer output + verification ladder (masked L2-L4).
36. `08.M6` Intent-scorer report + concurrency metrics.

Each milestone's real-surface artifact (CLI transcript, `--jsonl` capture, TUI frame,
`sqlite3` query, live OpenRouter run) is captured per its home doc's §6.

---

## 5. Global Commit & Branch Strategy

Each subsystem doc ships its own atomic, independently-green commit sequence (its §9). The
cross-subsystem plan is **one branch per subsystem, PR'd in wave order**, so shared
contracts land before consumers.

**Branch/PR order** (each PR = one doc's §9 commit run; all commits authored and committed
as `echobt` per the commit-identity policy):

| Order | Branch | Doc §9 landed | Merge gate before next |
|-------|--------|---------------|------------------------|
| 1 | `feat/01-prompt-mode` | [01 §9](./01-prompt-mode-system.md) (12 commits: assembler → bodies → gate → CLI/TUI → live schema test) | W1 |
| 2 | `feat/02-engine-loop` | [02 §9](./02-engine-7step-loop.md) (9 commits: scaffold → gate wire → check → reminders → trajectory/Session Log → full loop) | W1 |
| 3 | `feat/06-subagents-skills` | [06 §9](./06-subagents-skills.md) (9 commits: depth fields → task gate → nested spawn → leaf → MCP/skill inheritance → composition → live parity) | W1 |
| 4 | `feat/04-concurrency` | [04 §9](./04-concurrency.md) (7 commits: cancellable fan-out → doom diagnostic → policy → nested grant → semaphore/ceiling → tree cancel → event surface) | W2 |
| 5 | `feat/05-reviewers-verification` | [05 §9](./05-reviewers-verification.md) (6 commits: verdict substrate → reviewers off-by-default → L2 → L3 → L4 → surfaces) | W2 |
| 6 | `feat/07-intent-scorer` | [07 §9](./07-intent-scorer.md) (9 commits: rubric → persistence → readiness catalog → report → persona → trajectory read → effectiveness → view → live) | W2 |
| 7 | `feat/03-workflow-team` | [03 §9](./03-workflow-team-board.md) (11 commits: scaffold → log migration → reducer → personas → briefing → preflight → serial loop → acceptance → Coordinator → CLI wire) | W3 |
| 8 | `feat/08-cli-uiux` | [08 §9](./08-cli-uiux-mockups.md) (commits: chrome/tool-envelope → mode switch → workflow launch/monitor → team board → reviewer/ladder → scorer/metrics) | W4 |

**Migration-number coordination (critical, cross-doc).** Docs 02, 05, and 07 each add a DB
migration and independently name `0009`/`0010`. Migration numbers are assigned once, at
integration, from the single ordered `migrations.rs` set: doc 02's trajectory table takes
the first free slot, then doc 05's verdict table, then doc 07's `intent_scores` table.
Renumber any colliding claim before merge (see §6). Every migration is idempotent on a
fresh and an existing db.

**Per-commit gate.** Each commit leaves `cargo build`, `cargo clippy -- -D warnings`,
`cargo fmt --check`, and its home doc's scoped `cargo test -p <crate>` set green
(Verification Levels 0-1) before the next begins.

---

## 6. Cross-Cutting Risks

Risks that span more than one subsystem and therefore cannot be closed inside a single doc.

| Risk | Spans | Mitigation (owner) |
|------|-------|--------------------|
| **Naming drift.** A build agent coins a term or reintroduces a banned string. | all | Names are PICKED from `GLOSSARY.md` + `docs/rename/naming-map.md`; a CI gate fails on the banned prior-art brand string and on frozen nautical live-terms (§8). Dead terms appear only in migration-traceability notes, never as live names. |
| **No-mock enforcement.** A "finished" feature secretly ships a fixture/cassette on the user path. | all user-facing surfaces | Live is the default when a key is present; fixtures are test-only. Every feature needs an end-to-end live OpenRouter proof (§7). doc 06/07 live legs skip-if-absent but the mission gate runs them with the real key. |
| **Nested-concurrency explosion / leaks.** Enabling nesting (doc 06) risks depth×fan-out blowup, deadlock, starvation, or orphaned children. | 04 ↔ 06 ↔ 03 | doc 04 hardens the primitive: three independent bounds (wave width, per-agent fan-out, global ceiling), depth-ordered admission, drain-on-cancel + exact-PID reaping. doc 06 gates `task` on Agent Mode + depth. doc 03 builds workflows only on this hardened primitive. |
| **Verification hold-out integrity.** L2-L4 cases or verdicts leak into model-facing context, breaking "earned not gamed." | 02 ↔ 05 ↔ 08 | doc 02 keeps L0-L1 in `check`; doc 05 keeps the L2-L4 driver strictly core-side and asserts held-out fixtures never render; doc 08 masks L2-L4 in the ladder view (`███ withheld ███` is the only value the render layer receives). |
| **Migration-number collision.** Docs 02/05/07 independently claim `0009`/`0010`. | 02 ↔ 05 ↔ 07 | Assign numbers once at integration from the ordered `migrations.rs` set; renumber before merge (§5). |
| **Doom-loop double-ownership.** `detect → diagnose → fix` is described by both doc 02 (`02.M3`) and doc 04 (`04.M5`). | 02 ↔ 04 | One rewrite of `DoomLoopGuard`, coordinated across the two milestones; the gate requires the diagnostic before re-admission (single enforcement point). |
| **Shared read-model drift.** doc 08 views read artifacts authored by docs 02/03/05/07; an upstream schema change silently breaks a panel. | 08 ↔ {02,03,05,07} | Parse through shared typed readers with fixture-backed tests; fail visibly with an in-panel error state, never render stale/partial data. |

---

## 7. Verification & Acceptance

The whole rebuild is proven done by the same five-level ladder each subsystem uses, plus
live mission validation.

**Programmatic gate (offline, per `services.yaml`).**

- **build:** `cargo build -p cortex-cli` (default features).
- **typecheck:** `cargo check --workspace`.
- **lint:** `cargo clippy --workspace -- -D warnings` (scoped `cargo clippy -p <crate>` per worker).
- **test:** the workspace offline suites (fixtures/cassettes allowed here only); scoped `cargo test -p <crate>`.
- **format:** on-touch only (`rustfmt --edition 2024 --skip-children <files>`), never global.
- **naming gate:** the banned-brand scan returns nothing across `crates/**`, prompts, and commit messages; the frozen-nautical live-term scan is empty (§8).

**Verification ladder (the acceptance floor and ceiling).**

- **L0 Deterministic** and **L1 Agent-visible tests** run in the loop's `check`
  ([doc 02](./02-engine-7step-loop.md)); they are the floor.
- **L2 held-out**, **L3 Code Reviewer intent audit**, **L4 QA Validator process check**
  run core-side, held out from the model ([doc 05](./05-reviewers-verification.md)). Any
  FAIL blocks merge and re-posts the change; "looks good but" encodes as FAIL.
- The [Intent Scorer](./07-intent-scorer.md) issues an evidence-backed effectiveness
  verdict per finished turn (verdict-first, gating dims force overall FAIL, not-run is
  explicit).

**Live acceptance (the no-mock proof).** With `OPENROUTER_API_KEY` set, real agent turns
run end-to-end and prove real effects: files written, commands executed, LSP diagnostics
reinjected, real MCP `tools/call`, plugin hooks fired. The **headless `cortex run`
(JSON/JSONL)** surface is the primary validation surface (real effects, scriptable,
cheap); **`cortex serve`** is validated via `curl` (HTTP + SSE `/event`) on a
3100-3199 port; the **TUI** is validated via captured sessions/frames. Live legs
skip-if-absent, but mission validation MUST run them with the real key. Every terminal and
board transition is core-reducer-owned, so the model can never fabricate a "done."

**Process hygiene at acceptance.** No orphan processes: anything that launches the
binary/server/LSP/MCP stops it by exact PID; test ports stay in 3100-3199; no
watch/interactive mode in tests.

---

## 8. Naming & Originality Compliance

- **Authority.** `GLOSSARY.md` (frozen, one-term-one-surface) and
  `docs/rename/naming-map.md` (nautical → professional-generic) are the only sources of
  surface names. Build agents PICK names from these; a missing term is flagged, never
  coined.
- **Register.** The vocabulary is the shared professional register of terminal coding
  agents (Plan Mode, Edit Mode, Agent Mode, Chat Mode, Headless, Subagents, Skills,
  Reviewers, Orchestrator, Workflow, Team, Coordinator, Task Board, Intent Scorer). It is
  field vocabulary, not any one vendor's property and not a bespoke thesaurus.
- **Zero prior-art brand string.** The banned prior-art TypeScript agent name never
  appears in code, prompts, commit messages, or docs; where prior art must be referenced
  it is called "the reference TypeScript agent." A CI banned-brand gate enforces this
  (brief §8).
- **Zero frozen nautical live-terms.** The pre-rename nautical vocabulary is dead; the
  canonical dead-to-live mapping is `docs/rename/naming-map.md`. A pre-rename term may
  appear only as a one-time migration-traceability note in a subsystem doc, never as a
  live surface name; the professional-generic right-hand name is always the live one.
- **Originality is architectural, not lexical.** It is carried by the four differentiators
  in §2 (program-owned control flow, tool-access-defined modes, the held-out Verification
  ladder, the reliability-vector success metric), enforced alongside the mechanical
  banned-term gates in `ORIGINALITY-GUARD.md` §5.

---

## 9. How to Execute

The concrete order an implementer follows these eight docs. Read each doc's §0 (contracts)
before touching its code; obey its §6 (TDD, RED → GREEN → real-surface) and §9 (commits).

1. **Prime.** Read this master doc, then [`../MISSION-BRIEF.md`](../MISSION-BRIEF.md) §0/§5/§9/§15,
   then `GLOSSARY.md` + `docs/rename/naming-map.md`. Confirm the OpenRouter key with one
   live call; confirm ports 3100-3199 are free.
2. **Wave 1 (substrate).** Land [01](./01-prompt-mode-system.md) (assembler → bodies →
   gate → CLI/TUI), then [02](./02-engine-7step-loop.md) (scaffold → gate wire → check →
   reminders → **trajectory/Session Log early**), then [06](./06-subagents-skills.md)
   (nested delegation once 01+02 are green). This delivers a real headless agent turn over
   OpenRouter with tool-access-defined modes.
3. **Wave 2 (verification, concurrency, scoring).** Land [04](./04-concurrency.md),
   [05](./05-reviewers-verification.md), and [07](./07-intent-scorer.md) on the W1
   substrate. Assign migration numbers once here (§5, §6). This delivers the held-out
   ladder, the hardened concurrent worker primitive, and the intent scorer.
4. **Wave 3 (orchestration).** Land [03](./03-workflow-team-board.md): serial Workflow
   first, then the Team front-end. Parallel workers ride doc 04's primitive; milestone
   acceptance rides doc 05's evidence model.
5. **Wave 4 (operator surface).** Land [08](./08-cli-uiux-mockups.md): render every
   subsystem surface against the captured frames in
   [`assets/mockups/`](./assets/mockups/), pass the resize matrix, keep L2-L4 masked.
6. **Prove.** Run the §7 gate: build, typecheck, clippy, offline suites, the naming gate,
   the five-level ladder, and the live OpenRouter acceptance on all three surfaces. A
   feature is done only with its RED→GREEN proof and its real-surface artifact captured.

> Throughout: one action per model step, program-owned everything else; live by default,
> fixtures test-only; PID-exact cleanup; format on-touch; names picked, never coined.
