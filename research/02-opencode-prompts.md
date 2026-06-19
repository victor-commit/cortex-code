# opencode Prompt System — Inventory & Rebranding Notes

> Reference for Cortex Code. Raw prompt files were copied verbatim to
> `/projects/cortex-code/research/opencode-prompts-raw/` (subdirs: `session-prompt/`,
> `agent-prompt/`, `tool/`, `command/`). This document summarizes how they are selected and
> assembled, and which identity strings MUST be replaced when adapting them for Cortex Code.

## 1. Inventory

| Raw file | Category | When used | Selected by |
|---|---|---|---|
| `session-prompt/default.txt` | system | Fallback/base agent system prompt | default when no model-specific match |
| `session-prompt/anthropic.txt` | system | Claude models | model id contains `anthropic`/`claude` |
| `session-prompt/gpt.txt` | system | OpenAI GPT (chat) | model id `gpt-*` |
| `session-prompt/codex.txt` | system | OpenAI Codex/Responses models | codex/responses models |
| `session-prompt/copilot-gpt-5.txt` | system | GitHub Copilot GPT-5 | copilot provider + gpt-5 |
| `session-prompt/gemini.txt` | system | Google Gemini | model id `gemini` |
| `session-prompt/kimi.txt` | system | Moonshot Kimi | model id `kimi` |
| `session-prompt/beast.txt` | system | "Beast mode" high-agency variant | configured/agent opt-in |
| `session-prompt/trinity.txt` | system | Trinity variant | model/agent opt-in |
| `session-prompt/build-switch.txt` | system fragment | Injected when switching into build mode | mode transition |
| `session-prompt/plan.txt` | system fragment | Plan agent base | plan agent |
| `session-prompt/plan-mode.txt` | system fragment | Plan-mode constraints (no mutations) | plan mode active |
| `session-prompt/plan-reminder-anthropic.txt` | reminder | Re-injected plan reminder for Claude | plan mode + anthropic |
| `session-prompt/max-steps.txt` | reminder | Injected near step budget limit | step-count threshold |
| `agent-prompt/explore.txt` | agent | `explore` sub-agent | explore agent |
| `agent-prompt/generate.txt` | meta | Generates a new custom agent definition | `agent create` flow |
| `agent-prompt/compaction.txt` | aux | Context compaction/summarization | overflow → compaction |
| `agent-prompt/summary.txt` | aux | Session summary | summary flow |
| `agent-prompt/title.txt` | aux | Auto session title from first message | new session |
| `tool/read.txt` … `tool/websearch.txt` | tool desc | Tool description strings shown to the model | per registered tool |
| `tool/task.txt` | tool desc | `task` sub-agent tool description | task tool |
| `tool/plan-enter.txt`, `tool/plan-exit.txt` | tool desc | Plan mode enter/exit tools | plan tools |
| `command/review.txt` | command | `/review` slash command prompt | review command |

Tool description files captured: `apply_patch, edit, glob, grep, lsp, plan-enter, plan-exit, question, read, skill, task, todowrite, webfetch, websearch, write`.

## 2. System prompt selection & assembly

Assembled in `packages/opencode/src/session/prompt.ts` + `session/system.ts` +
`session/instruction.ts`:

1. **Base system prompt** chosen by model family (anthropic / gpt / codex / gemini / kimi /
   copilot-gpt-5 / default). "beast"/"trinity" are opt-in high-agency variants.
2. **Dynamic context appended** (the "environment block"): cwd, platform/OS, date, whether the
   dir is a git repo, and a project file-tree snippet.
3. **Rules / instructions injection**: `AGENTS.md` (and legacy `CLAUDE.md`) from project + global
   config, plus any `instructions[]` globs/URLs (`session/instruction.ts`,
   `config/markdown.ts`). Built-in system context comes from
   `packages/core/src/system-context/builtins.ts`.
4. **Mode fragments**: plan-mode / build-switch / plan-reminder are appended when relevant.
5. **Step-budget reminder** (`max-steps.txt`) injected as the step count approaches `MAX_STEPS`
   (25 in V2 runner).
6. **Tool descriptions** (`tool/*.txt`) are attached to each tool definition in the registry
   (`tool/registry.ts`), so the model sees per-tool usage guidance.

## 3. Tool descriptions (purpose)

- `read` — read a file (line ranges, truncation rules). `write` — create/overwrite. `edit` —
  exact string replace. `apply_patch` — GPT/Codex-style multi-file patch envelope.
- `glob` — file name matching. `grep` — ripgrep content search.
- `bash`/shell (see architecture report) — run commands with permission gating.
- `todowrite` — task list management. `webfetch` — fetch URL → markdown. `websearch` — web
  search. `lsp` — language intelligence. `skill` — load a SKILL.md capability. `question` — ask
  the user a structured question. `task` — spawn a sub-agent. `plan-enter`/`plan-exit` — toggle
  plan mode.

## 4. Agent / auxiliary prompts

- **Agents**: `explore` (read-only investigation), plan (mutations blocked), build (full tools);
  `generate.txt` produces a new custom agent spec from a description.
- **Compaction** (`compaction.txt`): summarizes the conversation when the context window
  overflows, preserving key state so the loop can continue.
- **Summary** (`summary.txt`) and **Title** (`title.txt`): short auxiliary generations.

## 5. Rebranding notes (MUST replace for Cortex Code)

Grep of the raw prompts shows product identity strings that must be swapped — never ship the
name "opencode" or its URLs:

- `default.txt` contains:
  - `/help: Get help with using opencode` → `/help: Get help with using Cortex Code`
  - `report the issue at https://github.com/anomalyco/opencode/issues` → Cortex Code repo/URL
  - `When the user directly asks about opencode ... opencode docs at https://opencode.ai` →
    replace product name + docs URL with Cortex Code equivalents (or remove the web-docs
    self-reference entirely).
- Any other `opencode` / `opencode.ai` / `anomalyco/opencode` occurrences in the prompt set must
  be replaced with the Cortex Code brand. **Action for workers:** after porting, run
  `rg -i "opencode" crates/**/prompts` and assert zero hits.
- Tone/verbosity rules (concise, <4 lines, no preamble, no emojis) are generic and can be kept.
- Decision: store Cortex Code prompts as adapted `.txt`/embedded strings under a dedicated
  prompts module; keep the same selection-by-model-family structure.

## 6. Recommendation for Cortex Code

Replicate the structure exactly: per-model-family base prompt + dynamic environment block +
AGENTS.md/rules injection + mode fragments + per-tool description strings + compaction/title/
summary auxiliaries. Adapt the captured raw text, rebrand all identity strings, and add a CI
guard that fails if the competitor name appears anywhere in source or commits.
