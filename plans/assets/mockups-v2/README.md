# Cortex Code mockup gallery v2

This directory holds deterministic 120x40 terminal mockups. Each screen has a text
source of truth and a faithful PNG rasterization.

## Render and verify

From the standalone harness:

```bash
cd /projects/cortex-code/tools-throwaway/mockup-harness
cargo run --quiet --features png -- --list
cargo run --quiet --features png -- --all
cargo run --quiet --features png -- <screen>
cargo run --quiet --features png -- <category>
./check.sh --all
./sweep_checks.sh
./sweep_determinism.sh
```

`--all` writes every `<category>/<screen>.txt` and `.png` pair. `check.sh` is the
per-screen conformance gate, while the two sweep scripts check gallery consistency and
byte-stable full renders.

## Session (21)

| Screen | Description |
| --- | --- |
| `home` | Welcome screen with the Code banner, project metadata, and first exchange. |
| `onboarding` | First-run setup steps with a current step and continue controls. |
| `chat-turn` | Completed user and agent turn with readable inline code styling. |
| `chat-long-scroll` | Multi-turn transcript with an explicit earlier-history affordance. |
| `streaming` | Partial agent reply with a single live composing band. |
| `reasoning` | Expanded thinking trace with a collapse affordance. |
| `reasoning-collapsed` | One-line thinking summary with an expand affordance. |
| `rich-response` | Agent response containing a lightweight table and code block. |
| `tool-calls` | Canonical successful, failed, and running tool-call anatomy. |
| `tool-running` | In-flight repository search with progress and one working band. |
| `tool-success` | Successful search result with actionable aligned file references. |
| `tool-success-expanded` | Expanded successful search output with surrounding context. |
| `tool-failed` | Failed build result with evidence and adjacent retry actions. |
| `tool-task` | Completed and running subagent tasks in a single transcript. |
| `tool-task-expanded` | Expanded subagent findings with severity and file references. |
| `diff-edit` | Pending inline diff with a decision dock. |
| `shell` | Completed and running shell commands using tool-line anatomy. |
| `blocked-command` | Plan-mode command denial with a clear recovery route. |
| `turn-error` | Inline provider-turn failure preserving the partial response. |
| `todo-list` | Plan progress list with a single active task and working band. |
| `compact` | Conversation compaction recap with retained-transcript affordance. |

## Input (20)

| Screen | Description |
| --- | --- |
| `slash-menu` | Bottom-anchored slash command menu with grouped built-ins. |
| `agent-picker` | Built-in agent selection dock with role and permission metadata. |
| `command-palette` | Root command palette with filter, shortcut, and scope cues. |
| `command-palette-submenu` | Scoped queued-prompts palette with return affordance. |
| `model-picker` | Provider-grouped model selection dock with cost and context metadata. |
| `attachment-picker` | File attachment picker showing supported and unavailable candidates. |
| `model-picker-filtered` | Narrowed model results for a typed Claude query. |
| `file-mention` | Fuzzy file mention picker with highlighted matching path segments. |
| `export-picker` | Export-format dock with a visible default destination. |
| `fork-picker` | Timeline-backed picker for selecting an earlier turn to fork. |
| `queue-manager` | Ordered pending-prompt manager with movement controls. |
| `composer-multiline` | Multi-line composer with an attached send and newline guide. |
| `skills-picker` | Skill toggle picker with enabled count and apply action. |
| `session-picker` | Session picker with aligned age and message-count columns. |
| `subagent-picker` | Subagent task picker with live child-task status metadata. |
| `variant-picker` | Four-option reasoning-variant selector. |
| `theme-picker` | Theme selector populated from the shared five-theme preview table. |
| `permission-shell` | Shell-command permission dock with scoped allow choices. |
| `permission-file-edit` | File-edit permission dock with an inline mini-diff. |
| `permission-deny-reason` | Denied permission dock with a one-line reason input. |

## Views (16)

| Screen | Description |
| --- | --- |
| `status` | Full status overview for agent, context, model, and connected services. |
| `sidebar-context` | Context-files sidebar tab beside a narrowed real transcript. |
| `sidebar-files` | File-tree sidebar tab with disclosure and repository states. |
| `sidebar-lsp` | Language-server sidebar tab with diagnostic summaries. |
| `sidebar-mcp` | MCP sidebar tab with server connection information. |
| `sidebar-queue` | Prompt-queue sidebar tab with attached reordering controls. |
| `sidebar-todo` | Plan-progress sidebar tab with current and pending steps. |
| `sidebar-subagents` | Subagent sidebar tab with aligned live task states. |
| `timeline` | Turn timeline with checkpoints and inspectable events. |
| `help` | Sparse command and keyboard reference view. |
| `mcp` | MCP connection management view with readable server outcomes. |
| `review` | Review findings list with severity, location, and selected row. |
| `diff-viewer` | Read-only full diff viewer with semantic added and removed rows. |
| `diff-viewer-staging` | Diff staging view with hunk-level actions. |
| `theme-gallery-dark` | Dark-canvas gallery previewing each supported theme. |
| `theme-gallery-light` | Light-canvas gallery previewing each supported theme. |

## Auth (7)

| Screen | Description |
| --- | --- |
| `login` | Entry screen for choosing a provider sign-in route. |
| `connect-provider` | Provider catalogue with connected and active states. |
| `connect-method` | Provider connection-method choice dock. |
| `connect-apikey` | API-key entry dock with format validation and save guidance. |
| `connect-connected` | Successful provider connection confirmation. |
| `connect-disconnect` | Confirm-before-remove dock for a connected provider. |
| `connect-oauth` | Device-code authorization flow with live expiry and copy actions. |

## States (8)

| Screen | Description |
| --- | --- |
| `empty` | Concise empty session state with a clear first action. |
| `error-network` | Network failure block with retry backoff and recovery choices. |
| `error-context` | Context-capacity warning with reclaimable-token guidance. |
| `rate-limit` | Rate-limit view with quota bars and a live retry countdown. |
| `interrupt` | User-interrupted response with recovery path. |
| `toasts` | Borderless feedback toasts with inline dismissal and retry actions. |
| `checkpoint-undo` | Undo confirmation naming the restored checkpoint and redo path. |
| `share` | Share-link confirmation with a concise connected verdict. |

## Orchestration (14)

| Screen | Description |
| --- | --- |
| `mode-switch` | Mode selection with a permission-grant delta matrix. |
| `mode-switch-refused` | Plain-English mode-switch refusal with next step. |
| `workflow-launch` | Ready workflow preflight with waves and editable controls. |
| `workflow-launch-blocked` | Launch preflight pinned on a missing item and fix affordance. |
| `workflow-monitor` | Live workflow monitor with a single authoritative progress table. |
| `workflow-monitor-failure` | Workflow monitor with the failed feature escalated first. |
| `team-board` | Pipeline-stage board with blocked work escalated above routine rows. |
| `reviewer-output` | Changes-requested review with one actionable finding checklist. |
| `reviewer-approved` | Approved review state with a clean all-pass verdict. |
| `verification-ladder` | L0-L4 validation ladder with continuation cues. |
| `intent-scorer-pass` | Passing intent score with dimension evidence and confidence bars. |
| `intent-scorer-fail` | Failing intent score that names the dimension to repair. |
| `concurrency-metrics` | Wave, worker, conflict, and quarantine coordination metrics. |
| `tool-envelope` | Tool-envelope contract view for statuses, payloads, and truncation. |

**Total: 86 screens across 6 categories.**
