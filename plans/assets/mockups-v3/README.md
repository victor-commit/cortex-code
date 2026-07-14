# Cortex Code mockup gallery v3

Deterministic 120x40 terminal mockups for the isolated v3 harness. Each canonical
screen is one TXT source of truth plus a faithful 1200x840 PNG rasterization.
Users experience **six coherent journeys**; the gallery still retains exactly
**86** behavioral states as regression artifacts under six physical categories.

This file covers only the v3 gallery, public harness CLI, gate provenance, and
output roots. It does not replace repository-root product documentation.

## Output locations

| Role | Path | Contents |
| --- | --- | --- |
| Canonical gallery | `plans/assets/mockups-v3/<category>/<screen>.{txt,png}` | All 86 IDs, 172 artifacts |
| Gate storyboards | `plans/assets/mockups-v3-gates/<flow>/<NN>-<screen>.{txt,png}` | 35 ordered frames (70 artifacts) |
| Isolated harness | `tools-throwaway/mockup-harness-v3/` | Registry, renderer, scenarios, checks |
| Human gate evidence | mission `evidence/v3-human-gate/<review_bundle_sha256>/` | Immutable request + orchestrator decision |

Canonical renders never write into the gate root, and gate publication never
overwrites the canonical gallery. Scenario case/tick PNG destinations must be
caller-owned temporary paths; canonical and gate roots are rejected. Frozen
v1/v2 galleries and the v2 harness remain comparison-only and must stay byte-stable.

## Render, audit, case, and validation commands

Work from the isolated harness root:

```bash
cd /projects/cortex-code/tools-throwaway/mockup-harness-v3
```

### Inventory and render selectors

Public binary: `mockup-harness-v3` (build with `--features png`).

```bash
# Exact 86-row inventory: category, id, status, flow, surface
cargo run --quiet --bin mockup-harness-v3 --features png -- --list

# Publish every currently implemented screen into the canonical gallery
cargo run --quiet --bin mockup-harness-v3 --features png -- --implemented

# Publish the full registry into the canonical gallery (exact 86 pairs)
cargo run --quiet --bin mockup-harness-v3 --features png -- --all

# One canonical screen by ID (example)
cargo run --quiet --bin mockup-harness-v3 --features png -- home

# One physical category (example)
cargo run --quiet --bin mockup-harness-v3 --features png -- session

# One ordered gate flow storyboard (example)
cargo run --quiet --bin mockup-harness-v3 --features png -- --flow daily-work
```

Valid categories: `session`, `input`, `views`, `auth`, `states`, `orchestration`.
Valid flow names: `daily-work`, `input-and-approval`, `work-configuration`,
`provider-and-mcp-connection`, `recovery`, `mission`.

### Semantic audit

```bash
# Full inventory audit JSON on stdout
cargo run --quiet --bin mockup-harness-v3 --features png -- --audit-json

# One screen or other public selector
cargo run --quiet --bin mockup-harness-v3 --features png -- --audit-json home
cargo run --quiet --bin mockup-harness-v3 --features png -- --audit-json --flow mission
```

Audit JSON is stdout-only; diagnostics go to stderr. Exit 3 means registry,
manifest, or validation failure with no publication.

### Named scenario cases

```bash
# Deterministic interaction snapshot as JSON
cargo run --quiet --bin mockup-harness-v3 --features png -- \
  --case <case-id> --tick <n> --audit-json

# 120x40 TXT to stdout
cargo run --quiet --bin mockup-harness-v3 --features png -- \
  --case <case-id> --tick <n> --txt

# Faithful PNG only to an explicit non-gallery temporary path; prints SHA-256
cargo run --quiet --bin mockup-harness-v3 --features png -- \
  --case <case-id> --tick <n> --png /tmp/v3-case-snapshot.png
```

Unknown case or invalid tick exits 2; impossible actions exit 3. Case PNG
paths inside `mockups-v3/` or `mockups-v3-gates/` are rejected.

### Negative fixtures

```bash
cargo run --quiet --bin mockup-harness-v3 --features png -- \
  negative-fixture <fixture-id>
```

Expected result for a known invalid fixture is fail-closed exit 3.

### Final validation gate and approval provenance

Mission-manifest commands (from the harness root, with
`V3_ISOLATION_SESSION_ID` set for isolation tools):

```bash
# Per-screen and whole-gallery programmatic conformance
./check.sh --implemented
./check.sh --all

# Aggregate gallery / vision / secret / README / approval ledger checks
./sweep_checks.sh

# Two clean full renders compared by SHA-256
./sweep_determinism.sh

# Frozen v1/v2 reference integrity
./check_v2_immutable.sh

# Recompute every gate, review, bridge, user-decision, and pre-bulk-tree digest
./check_gate_approval.sh \
  /root/.factory/missions/7060a79b-ccfb-445c-9e82-ee4504b320dc/evidence/v3-human-gate
```

Rust baseline for the final programmatic gate:

```bash
CARGO_INCREMENTAL=1 CARGO_BUILD_JOBS=16 cargo test --features png
CARGO_INCREMENTAL=1 CARGO_BUILD_JOBS=16 cargo build --features png
CARGO_INCREMENTAL=1 CARGO_BUILD_JOBS=16 cargo clippy --all-targets --features png -- -D warnings
```

**Human gate provenance:** gate workers write immutable
`evidence/v3-human-gate/<review_bundle_sha256>/request.json` with status
`awaiting_user`. Only the orchestrator authors sibling `decision.json` with
status `approved` or `rejected`. Workers never create or mutate decisions.
Missing, rejected, malformed, worker-authored, or hash-stale decisions fail
`check_gate_approval.sh` closed. Milestone bulk work requires an approved
decision that still matches every request, gate artifact, review sheet, bridge,
user-message, and pre-bulk-tree digest.

Public CLI exit codes: `0` success; `1` I/O or raster failure; `2` usage or
unknown selector with no writes; `3` validation failure with no publication;
`75` coordinator busy with no writes. All selectors preflight fully before any
staging directory is created; publication is atomic.

## Category index (physical gallery layout)

Counts must match `cargo run --bin mockup-harness-v3 --features png -- --list`.

### Session (21)

| ID | Description | Flow |
| --- | --- | --- |
| `home` | Welcome shell with restrained Code brand, project context, and first exchange. | daily-work |
| `onboarding` | First-run setup dock that starts How Code should work. | work-configuration |
| `chat-turn` | Completed user and agent turn returned to the same conversation. | daily-work |
| `chat-long-scroll` | Multi-turn transcript with an explicit jump to earlier turns. | daily-work |
| `streaming` | Partial agent reply with one live composing band. | daily-work |
| `reasoning` | Expanded thinking summary with a collapse affordance. | daily-work |
| `reasoning-collapsed` | One-line thinking summary with an expand affordance. | daily-work |
| `rich-response` | Agent reply with light table and code structure, still transcript-first. | daily-work |
| `tool-calls` | Shared successful, failed, and running tool-line anatomy. | daily-work |
| `tool-running` | In-flight repository search with progress and one working band. | daily-work |
| `tool-success` | Successful tool result with actionable aligned file references. | daily-work |
| `tool-success-expanded` | Expanded successful tool output with surrounding context. | daily-work |
| `tool-failed` | Failed tool result with one plain-English cause and recovery dock. | recovery |
| `tool-task` | Completed and running subagent tasks inside the transcript. | daily-work |
| `tool-task-expanded` | Expanded subagent findings with severity and locations. | daily-work |
| `diff-edit` | Pending edit presented in a bottom decision dock. | input-and-approval |
| `shell` | Shell tool lines for completed and running commands. | daily-work |
| `blocked-command` | Plan-mode denial that preserves work and offers one recovery path. | recovery |
| `turn-error` | Inline provider-turn failure that preserves the partial response. | recovery |
| `todo-list` | Plan progress with one active task and compact working band. | daily-work |
| `compact` | Compaction recap with a retained-transcript affordance. | daily-work |

### Input (20)

| ID | Description | Flow |
| --- | --- | --- |
| `slash-menu` | Bottom-anchored slash command menu with grouped built-ins. | input-and-approval |
| `agent-picker` | How Code should work choice dock with readable tradeoffs. | work-configuration |
| `command-palette` | Root command palette with filter, shortcut, and scope cues. | input-and-approval |
| `command-palette-submenu` | Scoped queued-prompts palette with an explicit return path. | input-and-approval |
| `model-picker` | Human-labeled model selection with provider group and tradeoffs. | work-configuration |
| `attachment-picker` | Project-file attachment picker with supported and unavailable rows. | input-and-approval |
| `model-picker-filtered` | Narrowed human model results for a typed filter query. | work-configuration |
| `file-mention` | Fuzzy file mention picker with matching path segments. | input-and-approval |
| `export-picker` | Export-format dock with a visible default destination. | input-and-approval |
| `fork-picker` | Timeline-backed earlier-turn fork selection. | work-configuration |
| `queue-manager` | Ordered pending-prompt manager with local movement controls. | input-and-approval |
| `composer-multiline` | Multi-line composer draft held while temporary actions interrupt. | input-and-approval |
| `skills-picker` | Skill enablement dock with apply action and returned origin. | work-configuration |
| `session-picker` | Readable session picker with age and message-count columns. | work-configuration |
| `subagent-picker` | Subagent task picker with live child status metadata. | work-configuration |
| `variant-picker` | Reasoning-variant selector presented as a plain choice set. | work-configuration |
| `theme-picker` | Theme selection dock backed by shared gallery previews. | work-configuration |
| `permission-shell` | Shell permission dock with scoped allow choices. | input-and-approval |
| `permission-file-edit` | File-edit permission dock with an inline mini-diff. | input-and-approval |
| `permission-deny-reason` | Denial dock collecting a one-line reason before return. | input-and-approval |

### Views (16)

| ID | Description | Flow |
| --- | --- | --- |
| `status` | Temporary status detail for model, work style, and connections. | work-configuration |
| `timeline` | Temporary turn timeline with restore and inspect points. | work-configuration |
| `mcp` | Capability-led MCP list ordered by attention, not transport jargon. | provider-and-mcp-connection |
| `diff-viewer` | Read-only full diff detail with semantic addition and deletion rows. | daily-work |
| `diff-viewer-staging` | Hunk-level staging detail that applies selected inclusions once. | input-and-approval |
| `help` | Sparse local key and command reference without a global hint row. | input-and-approval |
| `review` | Findings list with severity, location, and one selected row. | daily-work |
| `theme-gallery-dark` | Dark-canvas gallery preview of supported themes. | work-configuration |
| `theme-gallery-light` | Light-canvas gallery preview of supported themes. | work-configuration |
| `sidebar-context` | Contextual inspector for related context files. | daily-work |
| `sidebar-files` | Contextual inspector for the project file tree. | daily-work |
| `sidebar-lsp` | Contextual inspector for language diagnostics summary. | daily-work |
| `sidebar-todo` | Contextual inspector for plan progress steps. | daily-work |
| `sidebar-subagents` | Contextual inspector for live subagent tasks. | daily-work |
| `sidebar-mcp` | Contextual inspector for MCP capabilities beside the chat. | provider-and-mcp-connection |
| `sidebar-queue` | Contextual inspector for the prompt queue with local reorder keys. | input-and-approval |

### Auth (7)

| ID | Description | Flow |
| --- | --- | --- |
| `login` | Entry dock that chooses a provider sign-in route. | provider-and-mcp-connection |
| `connect-provider` | Provider catalogue with connected and active states. | provider-and-mcp-connection |
| `connect-method` | Method choice for the selected provider. | provider-and-mcp-connection |
| `connect-apikey` | Masked API-key entry with save guidance and secret hygiene. | provider-and-mcp-connection |
| `connect-oauth` | Device-code authorization with expiry and copy actions. | provider-and-mcp-connection |
| `connect-connected` | Successful provider connection that returns to origin. | provider-and-mcp-connection |
| `connect-disconnect` | Confirm-before-remove dock for a connected provider. | provider-and-mcp-connection |

### States (8)

| ID | Description | Flow |
| --- | --- | --- |
| `empty` | Empty session shell with one clear first action. | work-configuration |
| `error-network` | Network failure with one plain cause and primary retry path. | recovery |
| `error-context` | Context-capacity warning with reclaim guidance and preserved work. | recovery |
| `rate-limit` | Rate-limit wait with readable retry timing and preserved draft. | recovery |
| `interrupt` | User-interrupted generation with one obvious resume path. | recovery |
| `toasts` | Borderless success and failure feedback with local dismiss or retry. | recovery |
| `checkpoint-undo` | Undo confirmation naming the restored checkpoint and redo path. | recovery |
| `share` | Share confirmation dock with a concise connected verdict. | daily-work |

### Orchestration (14)

| ID | Description | Flow |
| --- | --- | --- |
| `mode-switch` | Work-style switch dock showing the permission-grant delta. | work-configuration |
| `mode-switch-refused` | Plain-English work-style refusal with the allowed next step. | work-configuration |
| `workflow-launch` | Ready mission preflight with an explicit launch decision. | mission |
| `workflow-launch-blocked` | Blocked mission preflight pinned on the missing fix action. | mission |
| `workflow-monitor` | Compact mission monitor that hides workers and internals by default. | mission |
| `workflow-monitor-failure` | Exception-first monitor that escalates the failed task first. | mission |
| `team-board` | Goal-led task board with blocked work ordered above routine rows. | mission |
| `reviewer-output` | Changes-requested review with one actionable finding checklist. | mission |
| `reviewer-approved` | Approved review that still requires explicit human completion. | mission |
| `verification-ladder` | Decision-facing validation stages for the current mission run. | mission |
| `intent-scorer-pass` | Advisory pass report that never completes the mission alone. | mission |
| `intent-scorer-fail` | Advisory fail report that routes repairs instead of completion. | mission |
| `concurrency-metrics` | Advanced runtime detail for concurrency, opened on demand. | mission |
| `tool-envelope` | Focused tool evidence with redaction and truncation markers. | mission |

**Category totals: session 21 + input 20 + views 16 + auth 7 + states 8 + orchestration 14 = 86.**

## Flow index (one owner flow per ID)

Each ID below appears under exactly one flow owner, matching the registry
`flow` column from `--list`. Gate storyboard frames are listed first per flow;
remaining owned variants follow in registry order.

### Flow 1 · daily-work (24)

Storyboard gate sequence:

`home` → `chat-turn` → `streaming` → `tool-running` → `tool-success` → `rich-response`

| ID | Category | Surface | Role |
| --- | --- | --- | --- |
| `home` | session | conversation-shell | Storyboard entry |
| `chat-turn` | session | conversation-shell | Storyboard; completed turn return |
| `streaming` | session | conversation-shell | Storyboard live reply |
| `tool-running` | session | conversation-shell | Storyboard in-flight tool |
| `tool-success` | session | conversation-shell | Storyboard successful tool |
| `rich-response` | session | conversation-shell | Storyboard closed reply |
| `chat-long-scroll` | session | conversation-shell | Longer transcript variant |
| `reasoning` | session | conversation-shell | Expanded thinking disclosure |
| `reasoning-collapsed` | session | conversation-shell | Closed thinking default |
| `tool-calls` | session | conversation-shell | Mixed tool-line anatomy |
| `tool-success-expanded` | session | conversation-shell | Expanded successful tool |
| `tool-task` | session | conversation-shell | Subagent task summary |
| `tool-task-expanded` | session | conversation-shell | Expanded task findings |
| `shell` | session | conversation-shell | Shell tool conversation |
| `todo-list` | session | conversation-shell | Plan progress in chat |
| `compact` | session | conversation-shell | Compaction recap |
| `diff-viewer` | views | temporary-detail-view | Read-only code detail |
| `review` | views | temporary-detail-view | Findings detail |
| `sidebar-context` | views | contextual-inspector | Context inspector |
| `sidebar-files` | views | contextual-inspector | Files inspector |
| `sidebar-lsp` | views | contextual-inspector | Diagnostics inspector |
| `sidebar-todo` | views | contextual-inspector | Plan inspector |
| `sidebar-subagents` | views | contextual-inspector | Subagent inspector |
| `share` | states | action-dock | Share confirmation |

### Flow 2 · input-and-approval (15)

Storyboard gate sequence:

`composer-multiline` → `file-mention` → `permission-shell` → `diff-edit` → `diff-viewer-staging` → `chat-turn`

`chat-turn` remains physical category `session` / flow `daily-work`; the gate
frame reuses that fixture as the Flow 2 success return, not a second ownership.

| ID | Category | Surface | Role |
| --- | --- | --- | --- |
| `composer-multiline` | input | action-dock | Storyboard draft composer |
| `file-mention` | input | action-dock | Storyboard mention picker |
| `permission-shell` | input | action-dock | Storyboard shell permission |
| `diff-edit` | session | action-dock | Storyboard pending edit |
| `diff-viewer-staging` | views | temporary-detail-view | Storyboard hunk staging |
| `slash-menu` | input | action-dock | Slash command dock |
| `command-palette` | input | action-dock | Root palette |
| `command-palette-submenu` | input | action-dock | Scoped palette |
| `attachment-picker` | input | action-dock | Attachment selection |
| `export-picker` | input | action-dock | Export format choice |
| `queue-manager` | input | action-dock | Prompt queue manager |
| `permission-file-edit` | input | action-dock | File-edit permission |
| `permission-deny-reason` | input | action-dock | Denial reason capture |
| `help` | views | temporary-detail-view | Local help detail |
| `sidebar-queue` | views | contextual-inspector | Queue inspector |

### Flow 3 · work-configuration (17)

Storyboard gate sequence:

`onboarding` → `model-picker` → `agent-picker` → `mode-switch` → `session-picker` → `timeline`

| ID | Category | Surface | Role |
| --- | --- | --- | --- |
| `onboarding` | session | action-dock | Storyboard first-run setup |
| `model-picker` | input | action-dock | Storyboard model choice |
| `agent-picker` | input | action-dock | Storyboard work-style choice |
| `mode-switch` | orchestration | action-dock | Storyboard work-style switch |
| `session-picker` | input | action-dock | Storyboard session resume |
| `timeline` | views | temporary-detail-view | Storyboard history detail |
| `model-picker-filtered` | input | action-dock | Filtered model list |
| `fork-picker` | input | action-dock | Fork earlier turn |
| `skills-picker` | input | action-dock | Skills apply dock |
| `subagent-picker` | input | action-dock | Subagent selection |
| `variant-picker` | input | action-dock | Reasoning variant |
| `theme-picker` | input | action-dock | Theme apply dock |
| `status` | views | temporary-detail-view | Status detail |
| `theme-gallery-dark` | views | temporary-detail-view | Dark theme gallery |
| `theme-gallery-light` | views | temporary-detail-view | Light theme gallery |
| `empty` | states | conversation-shell | Empty session seed |
| `mode-switch-refused` | orchestration | action-dock | Refused work-style change |

### Flow 4 · provider-and-mcp-connection (9)

Storyboard gate sequence:

`login` → `connect-provider` → `connect-method` → `connect-oauth` → `connect-connected` → `mcp`

Providers and MCP stay separate entry points. The shared storyboard ends on MCP
management after a successful provider connection.

| ID | Category | Surface | Role |
| --- | --- | --- | --- |
| `login` | auth | action-dock | Storyboard sign-in entry |
| `connect-provider` | auth | action-dock | Storyboard provider catalogue |
| `connect-method` | auth | action-dock | Storyboard method choice |
| `connect-oauth` | auth | action-dock | Storyboard OAuth device flow |
| `connect-connected` | auth | action-dock | Storyboard connected confirm |
| `mcp` | views | temporary-detail-view | Storyboard MCP manage view |
| `connect-apikey` | auth | action-dock | API-key connection variant |
| `connect-disconnect` | auth | action-dock | Disconnect confirmation |
| `sidebar-mcp` | views | contextual-inspector | MCP contextual inspector |

### Flow 5 · recovery (9)

Storyboard gate sequence:

`turn-error` → `error-network` → `rate-limit` → `chat-turn` → `checkpoint-undo`

`chat-turn` is owned by daily-work and appears only as the recovery success
return frame in the gate sequence.

| ID | Category | Surface | Role |
| --- | --- | --- | --- |
| `turn-error` | session | action-dock | Storyboard provider-turn failure |
| `error-network` | states | action-dock | Storyboard network recovery |
| `rate-limit` | states | action-dock | Storyboard rate-limit wait |
| `checkpoint-undo` | states | action-dock | Storyboard undo confirmation |
| `tool-failed` | session | action-dock | Failed tool recovery |
| `blocked-command` | session | action-dock | Blocked command recovery |
| `error-context` | states | action-dock | Context capacity recovery |
| `interrupt` | states | action-dock | Interrupted reply recovery |
| `toasts` | states | conversation-shell | Transient feedback recovery |

### Flow 6 · mission (12)

Storyboard gate sequence:

`workflow-launch-blocked` → `workflow-launch` → `workflow-monitor` → `workflow-monitor-failure` → `reviewer-output` → `reviewer-approved`

| ID | Category | Surface | Role |
| --- | --- | --- | --- |
| `workflow-launch-blocked` | orchestration | action-dock | Storyboard blocked preflight |
| `workflow-launch` | orchestration | action-dock | Storyboard ready launch |
| `workflow-monitor` | orchestration | conversation-shell | Storyboard compact monitor |
| `workflow-monitor-failure` | orchestration | action-dock | Storyboard exception-first failure |
| `reviewer-output` | orchestration | temporary-detail-view | Storyboard changes-requested review |
| `reviewer-approved` | orchestration | action-dock | Storyboard approved + human complete |
| `team-board` | orchestration | temporary-detail-view | Goal-led team board detail |
| `verification-ladder` | orchestration | temporary-detail-view | Validation stage detail |
| `intent-scorer-pass` | orchestration | temporary-detail-view | Advisory pass report |
| `intent-scorer-fail` | orchestration | temporary-detail-view | Advisory fail report |
| `concurrency-metrics` | orchestration | temporary-detail-view | Advanced concurrency evidence |
| `tool-envelope` | orchestration | temporary-detail-view | Tool evidence envelope |

**Flow totals: daily-work 24 + input-and-approval 15 + work-configuration 17 + provider-and-mcp-connection 9 + recovery 9 + mission 12 = 86.**

## Surfaces

Every state uses exactly one surface arrangement:

| Surface | Role |
| --- | --- |
| `conversation-shell` | Transcript-first work, including compact mission monitoring |
| `action-dock` | Bottom decision dock for pickers, permissions, recovery, and launch |
| `contextual-inspector` | Narrow side inspector that preserves the conversation |
| `temporary-detail-view` | Full temporary detail that returns to recorded origin |

## Geometry and fidelity

- Terminal canvas: 120 columns x 40 rows UTF-8 TXT
- PNG: 1200x840 RGB faithful rasterization
- Exact inventory: 86 TXT + 86 PNG canonical pairs; 35 TXT + 35 PNG gate pairs
- One statusline form: `~/path · branch · <Human Model Name>`
- Closed glyph set only; no decorative emoji

## Boundary reminders

- Do not modify repository-root `README.md`, frozen v1/v2 galleries, or the real product TUI.
- Do not author or rewrite human-gate `decision.json`.
- Serialize writing to each output root; vision validators are read-only after publication.
- Cleanup may remove only previously owned renderer-manifest files inside the selected v3 root.

**Total: 86 screens across 6 categories and 6 user flows.**
