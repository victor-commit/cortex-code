# Factory Droid missions — reverse engineering distill (read-only)

Source: binary `/root/.local/bin/droid` 0.179.0 (not stripped), strings + live missionDir behavior observed in mission `7060a79b-…`.

## Runtime shape

1. **Orchestrator session** (Droid in mission mode) owns planning tools: `propose_mission`, `start_mission_run`, handoff dismissal, feature list mutation.
2. **missionDir** (under `~/.factory/missions/<uuid>/`) is the sole mission state root. Writing mission artifacts elsewhere bricks the mission.
3. **factoryd / droid daemon** spawns **worker sessions** that execute one feature at a time. Stock Factory runner is **sequential** (`start_mission_run` blocks; workers run one-by-one).
4. Workers load skills from `{missionDir}/skills/`, follow `mission-worker-base` + feature `skillName`, implement in **repo workingDirectory**, commit via harness conventions, then **EndFeatureRun** handoff JSON.

## missionDir schema (authoritative files)

| File | Role |
| --- | --- |
| `mission.md` | Accepted proposal / mission-level decisions |
| `architecture.md` | Authoritative system design for workers |
| `validation-contract.md` | Behavioral assertions with stable IDs (`VAL-…`) |
| `validation-state.json` | per-assertion `pending|passed|failed|blocked` |
| `features.json` | Ordered feature queue: id, description, skillName, milestone, preconditions, expectedBehavior, fulfills[], status |
| `AGENTS.md` | Boundaries, directives, conventions |
| `services.yaml` | commands (test/typecheck/lint/…) + long-running services |
| `init.sh` | Idempotent env setup |
| `skills/<name>/SKILL.md` | Worker procedures |
| `library/` | Living knowledge (incl. `user-testing.md`) |
| `handoffs/` | Worker EndFeatureRun JSON |
| `validation/<milestone>/{scrutiny,user-testing}/` | Validator synthesis |
| `progress_log.jsonl`, `state.json`, `working_directory.txt` | Runtime progress |

## Feature execution rules (from embedded orchestrator skill text)

- Features execute in **array order**; first pending runs next.
- `fulfills` = assertion IDs this leaf feature **completes** (unique across features).
- Never manually create `scrutiny-validator` / `user-testing-validator` skillNames — **auto-injected** when a milestone’s implementation features complete.
- On worker `successState: failure|partial`, feature resets to `pending`.
- Handoff fields: salientSummary, whatWasImplemented, whatWasLeftUndone, verification.commandsRun, tests.added, discoveredIssues, returnToOrchestrator, successState.
- Preemption: insert pending feature at top + `start_mission_run`.
- Sealed milestones: do not add features after validators pass.

## Parallelization gap (our product differentiator)

Stock Factory missions runner observed: **one worker at a time**.  
Cortex product requirement: **parallel implementer workers** (pool N/max, deps-aware scheduling) with Mission Control UI (product chrome + MC V2 premium single-page), while validators may remain serial per milestone.

## MCP / tools observed in skill text

Orchestrator tools include propose/start mission, dismiss handoff items, Task subagents, Skill loader. Workers use standard coding tools + EndFeatureRun.

## Non-goals of RE

- No bypass of Factory auth/licensing.
- No copying proprietary JS bundles into Cortex.
- Behavioral similarity of **mission orchestration semantics** only; clean-room Rust implementation under Apache-2.0.
