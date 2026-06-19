# opencode (sst/opencode) — Architecture & Feature Inventory

Blueprint for the Rust greenfield agent **Cortex Code**. Everything below cites real
paths under `/tmp/opencode-sst`. Version analyzed: `packages/opencode/package.json` → `1.17.4`.

> **Key structural note.** The monorepo contains **two coexisting implementations**:
> - **V1 (legacy, still the default runtime):** `packages/opencode/src/**` — a large Effect-TS
>   app that uses the **Vercel AI SDK** (`ai`, `@ai-sdk/*`) for model streaming. The agentic
>   loop lives in `packages/opencode/src/session/prompt.ts` (1722 lines).
> - **V2 (durable rewrite, in progress):** `packages/core/src/**` + `packages/llm/src/**` +
>   `packages/server/src/**` + `packages/sdk/js/src/v2`. V2 replaces the AI SDK with a
>   **native protocol layer** (`@opencode-ai/llm`) and a durable, SQLite-backed session runner
>   (`packages/core/src/session/runner/llm.ts`). Tracked in `specs/v2/session.md` and
>   `packages/opencode/AGENTS.md` (“V2 Session Core”).
>
> For Cortex Code, **build the V2 model** (native protocols + durable runner + HTTP/SSE server +
> generated SDK), because it is cleaner and protocol-explicit, while pulling the full feature
> surface (tools, TUI, MCP, LSP, formatters) from V1 which is feature-complete.

---

## 1. Overall Architecture (client/server split, packages, comms, event bus)

### Package roles (`/tmp/opencode-sst/packages`)
| Package | Role |
|---|---|
| `opencode` | The CLI binary + V1 app: agentic loop, tools, providers, MCP, LSP, formatters, plugins, server wiring, ACP, GitHub. Entry: `opencode/src/index.ts` (yargs CLI). |
| `core` | V2 durable engine: session store/runner/history/compaction, tool registry, provider/model resolution, config, permission, skills, references, system-context, database (Drizzle/SQLite). |
| `llm` | **Native LLM protocol layer** (no AI SDK): Anthropic Messages, OpenAI Chat + Responses, OpenAI-compatible, Gemini, Bedrock Converse. Streaming over HTTP/WebSocket, error normalization. |
| `server` | V2 HTTP API: `groups/*` (route defs) + `handlers/*` (impls), built on Effect `httpapi` (OpenAPI-capable). |
| `sdk` | Generated TypeScript client (`sdk/js/src/gen` + `sdk/js/src/v2`) from the server’s OpenAPI. |
| `plugin` | Public plugin/hook type surface (`plugin/src/index.ts`, `plugin/src/tui.ts`, `plugin/src/tool.ts`). |
| `tui` | SolidJS terminal UI built on **OpenTUI** (`@opentui/core`, `@opentui/solid`, `@opentui/keymap`). |
| `app`, `web`, `desktop` | Web/desktop share viewers & app shell. |
| Others | `console`, `enterprise`, `containers`, `slack`, `stats`, `http-recorder`, `script`, `storybook`, `function`, `identity`, `effect-*` helpers. |

### Communication
- **Client ↔ server is HTTP + SSE.** The TUI / `run` / SDK talk to a local (or remote) HTTP server.
  Server entry: `opencode/src/server/server.ts` and `server/src/api.ts` / `server/src/routes.ts`.
  It’s an Effect `httpapi` app (`NodeHttpServer`, `HttpRouter`, `OpenApi.fromApi(PublicApi)`).
- **Events stream over SSE** at `/event` (group `server/src/groups/event.ts`,
  handler `server/src/handlers/event.ts`; V1 also `opencode/src/server/event.ts`).
- **mDNS/Bonjour** advertises the local server (`opencode/src/server/mdns.ts`, dep `bonjour-service`).
- `run --attach http://host:port` connects to a remote server (basic auth user/pass) — see `cli/cmd/run.ts`.
- **ACP** (Agent Client Protocol) bridge over stdio for editors: `opencode/src/acp/`, `cli/cmd/acp.ts`,
  dep `@agentclientprotocol/sdk`.

### Event bus
- V2 canonical bus: `EventV2` in `core/src/event.ts`; events are declared via `EventV2.define({type, schema})`
  (e.g. MCP `mcp.tools.changed`, `permission.asked/replied`, `lsp.updated`, `command.executed`).
- V1 global bus: `opencode/src/bus/global.ts`; bridged via `opencode/src/event-v2-bridge.ts` (`EventV2Bridge`).
- TUI-specific events: `opencode/src/server/tui-event.ts` (`TuiEvent.ToastShow`, etc.).

---

## 2. Agentic Loop

### V2 loop — `packages/core/src/session/runner/llm.ts` (the blueprint to replicate)
Orchestration (`SessionRunner.run`):
1. `run({sessionID, force})` checks durable pending inputs (`SessionInput.hasPending` for `"steer"`/`"queue"`).
   Inputs are admitted durably first (`SessionV2.prompt`) then an advisory `SessionExecution.wake` schedules a drain.
2. Outer `while (openActivity)` loop, inner `for step < MAX_STEPS (25)` loop. Each step calls `runTurn`.
3. `runTurnAttempt` per provider turn:
   - Resolves agent (`AgentV2.select`), initializes **system-context epoch** (`SessionContextEpoch.initialize`,
     loads system prompt + skill guidance + reference guidance, combined `SystemContext.combine`).
   - Promotes steer/queue inputs into visible user messages at safe boundaries (`SessionInput.promoteSteers/promoteNextQueued`).
   - Loads projected history (`SessionHistory.entriesForRunner`), translates to canonical LLM messages
     (`runner/to-llm-message.ts`), materializes tools (`tools.materialize(agent.permissions)`).
   - Builds `LLM.request({model, system, messages, tools, providerOptions:{openai:{promptCacheKey}}})`.
   - **Compaction check** before sending (`compaction.compactIfNeeded`) — see §2 compaction.
   - **Single streamed turn:** `llm.stream(request)` → `Stream.runForEach` over events.
4. **Streaming event handling** (`LLMEvent` union): text deltas, reasoning, usage, provider errors,
   and `tool-call`. On a non-provider-executed `tool-call`, it durably records the call, then runs
   `toolMaterialization.settle(...)` (authorize + execute via core-owned registry hook) **in a `FiberSet`
   so tool calls run concurrently**; results published as `LLMEvent.toolResult`. `needsContinuation=true`.
5. After stream completes: awaits all tool fibers, handles interrupts / question-rejection / provider errors,
   `failUnsettledTools` as needed. Returns whether another turn is needed.
6. Continuation: loops again if `needsContinuation` or pending steer inputs; `StepLimitExceededError` at 25.
   Overflow handling: a context-overflow provider error before any assistant text triggers
   `compaction.compactAfterOverflow` then a rebuild (`TurnTransition` defects: `RebuildPreparedTurn`,
   `ContinueAfterOverflowCompaction`).

### V1 loop — `packages/opencode/src/session/prompt.ts`
`SessionPrompt.loop`/`prompt` is the monolithic version: resolves prompt parts, builds tools (`ai` SDK
`tool()`+`jsonSchema()`), system prompt (`session/system.ts`), instructions (`session/instruction.ts`),
streams via `session/llm/ai-sdk.ts` (Vercel AI SDK `streamText`) OR `session/llm/native-request.ts`
(native path), processes parts (`session/processor.ts`), handles compaction (`session/compaction.ts`),
revert/snapshots (`session/revert.ts`), retries (`session/retry.ts`), reminders (`session/reminders.ts`),
summaries/titles (`session/summary.ts`). Max steps prompt: `session/prompt/max-steps.txt`.

### Streaming / tool-call parsing
- V2: native protocol parsers in `packages/llm/src/protocols/*` produce a normalized `LLMEvent`
  stream (`llm/src/schema/events.ts`). Tool-call args are streamed/assembled in
  `llm/src/protocols/utils/tool-stream.ts`.
- V1: AI SDK does the SSE parsing; `partial-json` handles partial tool args.

### Context/message management & compaction
- V2: `core/src/session/history.ts`, `context-epoch.ts`, `compaction.ts`; durable messages in
  `core/src/session/sql.ts` (Drizzle). Compaction policy from config `compaction.{auto,prune,tail_turns,
  preserve_recent_tokens,reserved}` (see §12).
- V1: `session/compaction.ts` (620 lines) + `session/overflow.ts`; summary prompt
  `agent/prompt/compaction.txt`, `agent/prompt/summary.txt`.
- Truncation of large tool outputs to disk: `opencode/src/tool/truncate.ts` (config `tool_output.max_lines/max_bytes`).

---

## 3. Providers / Models / Auth

### Provider config & resolution — `packages/opencode/src/provider/provider.ts` (1963 lines)
- Lazy-loaded **bundled AI-SDK providers** (`BUNDLED_PROVIDERS` map): amazon-bedrock(+mantle), anthropic,
  azure, google, google-vertex(+anthropic), openai, **openai-compatible**, **openrouter**, xai, mistral,
  and more (package deps list ~25 `@ai-sdk/*`, plus `@openrouter/ai-sdk-provider`, `ai-gateway-provider`,
  `gitlab-ai-provider`, `venice-ai-sdk-provider`).
- SSE hardening: `wrapSSE()` adds a stream read-timeout; `timeoutController()` for header timeout.
- Custom providers via config `provider.<id>` (`core/src/v1/config/provider.ts`), `disabled_providers`,
  `enabled_providers`, model selection `model`/`small_model` (`provider/model/provider`).
- V2 native protocol routing: `packages/llm/src/providers/*` (anthropic, openai, openai-compatible(+profile),
  google, amazon-bedrock, azure, xai, cloudflare, github-copilot, **openrouter**) +
  `llm/src/route/*` (executor, endpoint, transport http/websocket, auth, framing).

### models.dev — `packages/core/src/models-dev.ts`
- Fetches the models.dev catalog (cost tiers, context window, capabilities, status alpha/beta/deprecated)
  over HTTP with caching (`Flock`/`Hash`), keyed by a custom User-Agent. Powers model listing & pricing.
- `cli/cmd/models.ts` and `cli/cmd/providers.ts` surface the catalog.

### Auth (`auth login`) — `packages/opencode/src/auth/index.ts`
- Auth record union: `oauth` | `api` | `wellknown` (`Auth.Info`). OAuth dummy key constant
  `OAUTH_DUMMY_KEY`. Stored credentials via `core/src/credential` + `FSUtil`.
- Auth methods are pluggable via the `AuthHook` in `plugin/src/index.ts` (oauth/api with `prompts`
  text/select, `authorize()` returning OAuth url + `callback` for `auto`/`code` flows).
- Bundled provider auth plugins: `opencode/src/plugin/github-copilot/`, `opencode/src/plugin/openai/`,
  plus `@gitlab/opencode-gitlab-auth`, `opencode-poe-auth`, `google-auth-library`, `@aws-sdk/credential-providers`.
- CLI: `opencode auth login/list/logout` is under `cli/cmd/account.ts` (exported as `ConsoleCommand`),
  provider list `cli/cmd/providers.ts`.

### OpenRouter
- V1: `@openrouter/ai-sdk-provider` in `BUNDLED_PROVIDERS["@openrouter/ai-sdk-provider"]`.
- V2 native: `packages/llm/src/providers/openrouter.ts`. Treated as an OpenAI-compatible provider with
  models from models.dev; API-key auth.

---

## 4. Built-in Tools & Permission Model

### Tool definition shape — `opencode/src/tool/tool.ts`
`Tool.Def = { id, description, parameters (Effect Schema), jsonSchema?, execute(args, ctx) }`.
`Tool.Context` carries `sessionID, messageID, agent, abort, callID, messages, metadata(), ask()` (permission).
Invalid args raise `InvalidArgumentsError` (model-facing “rewrite the input”). Registry:
`opencode/src/tool/registry.ts` assembles builtins + custom (file-glob `tool[s]/*.{js,ts}` + plugin tools).

### Built-in tools (V1 `opencode/src/tool/*`, V2 mirror `core/src/tool/*`)
| Tool | File | What it does |
|---|---|---|
| `read` | `read.ts`/`read.txt` | Read file/dir, line-numbered (`<n>: line`), offset/limit (2000 default), images/PDF as attachments. |
| `write` | `write.ts`/`write.txt` | Write/overwrite file; must Read existing first; runs formatters + LSP after. |
| `edit` | `edit.ts`/`edit.txt` | Exact string replace (`oldString`→`newString`, `replaceAll`); requires prior Read; many fuzzy match strategies. |
| `apply_patch` | `apply_patch.ts`/`.txt` | Codex-style multi-file patch envelope (`*** Begin Patch`). Used for GPT models instead of edit/write (registry switches by modelID). |
| `glob` | `glob.ts` | Fast filename pattern matching (ripgrep/`glob`). |
| `grep` | `grep.ts` | Regex content search (ripgrep) with include filter. |
| `shell`/`bash` | `tool/shell.ts`, `tool/shell/`, V2 `bash.ts` | Run shell commands; tree-sitter bash/powershell parsing; permission-gated; background jobs (`background/job.ts`). |
| `task` | `task.ts`/`.txt` | Spawn a sub-agent (see §9); description lists available agent types filtered by permission. |
| `todowrite`/`todo` | `todo.ts`/`todowrite.txt` | Maintain structured todo list per session (`session/todo.ts`). |
| `webfetch` | `webfetch.ts`/`.txt` | Fetch URL → markdown (`turndown`, `htmlparser2`). |
| `websearch` | `websearch.ts` + `mcp-websearch.ts` | Web search; enabled for `opencode` provider or via Exa/Parallel flags (`webSearchEnabled`). |
| `skill` | `skill.ts`/`.txt` | Invoke a Skill (see §9/§12). |
| `question` | `question.ts`/`.txt` | Ask the user multiple-choice/free-text questions mid-run (client must support it). |
| `lsp` | `lsp.ts`/`.txt` | LSP queries: hover/definition/references/workspaceSymbol etc. (experimental flag). |
| `plan`/plan-enter/plan-exit | `plan.ts` + `plan-*.txt` | Plan-mode enter/exit suggestions (experimental, CLI client). |
| `invalid` | `invalid.ts` | Placeholder for unknown tool calls. |
| (custom) | registry | Project `tool[s]/*.ts` files + plugin-provided tools, converted from Zod or JSON Schema. |

Model-aware filtering (`registry.ts → tools()`): GPT (non-oss, non-gpt-4) models get `apply_patch` and lose
`edit`/`write`; others keep `edit`/`write`. Web search gated by provider/flags.

### Permission model — `opencode/src/permission/index.ts` + `evaluate.ts`
- Three actions: **`allow` | `ask` | `deny`** (`PermissionV1`). Rulesets are arrays of
  `{permission, pattern, action}`; `evaluate(permission, pattern, ...rulesets)` returns the **last
  matching rule** (`Wildcard.match` glob), defaulting to `ask`.
- Per-agent rulesets (`agent.permission`) + global config `permission` (`core/src/v1/config/permission.ts`).
  Sub-agent permission narrowing: `agent/subagent-permissions.ts`.
- Runtime asks surface via events `permission.asked`/`permission.replied`; plugin hook `permission.ask`
  can override the decision; saved decisions in `core/src/permission/saved.ts`.
- `experimental.continue_loop_on_deny`, `experimental.policies` add policy controls.

### Checkpointing mutations
- **Snapshots** via git: `opencode/src/snapshot/` + `opencode/src/session/revert.ts` create restore points
  before mutating tools; config `snapshot: true`. `worktree/`, `git/` support. TUI `messages_undo`/`redo`
  revert to snapshots. Patches tracked per message.

---

## 5. TUI / UX

### Library
- **OpenTUI + SolidJS**: `@opentui/core`, `@opentui/solid`, `@opentui/keymap`, `opentui-spinner`,
  reactive `solid-js`. Entry `tui/src/index.tsx`, root `tui/src/app.tsx`, runtime `tui/src/runtime.tsx`.

### Layout & rendering
- Routes: `tui/src/routes/home.tsx`, `tui/src/routes/session/` (chat). Sidebar/home/system feature-plugins
  in `tui/src/feature-plugins/`. Prompt component `tui/src/component/prompt/index.tsx`.
- Streaming render of assistant text; **markdown + syntax highlight** via tree-sitter
  (`web-tree-sitter`, `tree-sitter-bash`, `tree-sitter-powershell`, `tui/src/parsers-config.ts`).
- **Diff rendering**: `@pierre/diffs` + `tui/src/feature-plugins/system/diff-viewer.tsx`, `diff` package.
- **Tool-call cards**, `todo-item.tsx`, spinner, startup loading, error component.
- **Toasts** via `TuiEvent.ToastShow` (`opencode/src/server/tui-event.ts`).

### Interaction modes (`tui/src/component/prompt/index.tsx`, `autocomplete.tsx`)
- **Slash commands** (`/`): autocomplete list of commands; built-in slashes include
  `help, exit, new, sessions, models, agents, themes, variants, diff, status, skills, mcps,
  workspaces, connect, org, editor, warp, move` (`grep slashName`), plus user/MCP/skill commands.
- **@ file mentions**: `insertFileMention()` inserts `@path` with optional line ranges; fuzzy file search
  (`fuzzysort`, frecency `prompt/frecency.tsx`).
- **Shell mode (`!`)**: typing `!` enters shell mode (key `"!"`, “Exit shell mode” on esc/backspace).
- **Command palette**: `tui/src/component/command-palette.tsx` (default `ctrl+p`, keybind `command_list`).
- **Themes**: `tui/src/theme/` (assets), dialog `dialog-theme-list.tsx`, keybind `<leader>t`.
- **Session picker / list / timeline / rename / delete / move / stash**: `dialog-session-*.tsx`,
  `dialog-move-session.tsx`, `dialog-stash.tsx`.
- **Model/provider/agent/variant/skill/mcp/status dialogs**: corresponding `dialog-*.tsx`.
- **Find-in-chat / message navigation**: `messages_*` keybinds, `messages_copy/undo/redo`,
  conceal toggle.
- **which-key panel**: `which_key_toggle` (`ctrl+alt+k`).

### Keybinds — `tui/src/config/keybind.ts` (466 lines)
Leader default `ctrl+x`. Highlights: `app_exit` (`ctrl+c,ctrl+d,<leader>q`), `command_list` (`ctrl+p`),
`editor_open` (`<leader>e`), `theme_list` (`<leader>t`), `sidebar_toggle` (`<leader>b`),
`session_new` (`<leader>n`), `session_list` (`<leader>l`), `session_compact` (`<leader>c`),
`model_list` (`<leader>m`), `agent_list` (`<leader>a`), `variant_cycle` (`ctrl+t`),
`session_quick_switch_1..9` (`<leader>1..9`), plus a full readline-style input editing map and
dialog/autocomplete bindings. Fully user-overridable (config `keybinds`).

---

## 6. MCP — `packages/opencode/src/mcp/`

- SDK: `@modelcontextprotocol/sdk`. Transports: **stdio** (`StdioClientTransport`) for local servers,
  **Streamable HTTP** + **SSE** (tries StreamableHTTP first, then SSE) for remote
  (`connectRemote`/`connectLocal` in `mcp/index.ts`).
- Config: `mcp: { name: {type:"local", command:[...], cwd, environment, timeout, enabled} | {type:"remote",
  url, headers, oauth, timeout, enabled} }` (`core/src/v1/config/mcp.ts`).
- Handshake: `Client({name:"opencode", version}).connect(transport)` with timeout; capabilities checked
  (`getServerCapabilities().tools`), then **tools/list** via `McpCatalog.defs` and **tools/call** via
  `McpCatalog.convertTool` (name `sanitize(server)_sanitize(tool)`). Also lists **prompts** and **resources**.
- Live updates: `ToolListChangedNotificationSchema` re-lists tools and emits `mcp.tools.changed`;
  `LoggingMessageNotificationSchema` forwards server logs.
- **OAuth**: `mcp/oauth-provider.ts` (`McpOAuthProvider`), `mcp/oauth-callback.ts` (local callback server),
  `mcp/auth.ts` (token storage, expiry). Flow: `startAuth` → open browser → `waitForCallback` → CSRF
  state check → `finishAuth(code)`. Statuses: connected/disabled/failed/needs_auth/needs_client_registration.
  CLI `opencode mcp auth <name>` (`cli/cmd/mcp.ts`).
- Cleanup kills stdio descendant PIDs on shutdown.

---

## 7. LSP — `packages/opencode/src/lsp/`

- `lsp.ts` (service, 512 lines), `client.ts` (JSON-RPC over stdio via `vscode-jsonrpc`),
  `server.ts` (server definitions + auto-install), `launch.ts` (spawn), `language.ts`
  (extension→languageId map, ~80 languages), `diagnostic.ts`.
- **Auto-install**: server definitions know how to download/launch their language server (npm/binary).
- **Diagnostics → agent**: after edit/write, file is opened in the matching LSP server, diagnostics are
  collected and fed back into the tool result so the model sees errors; events `lsp.updated`.
- **`lsp` tool** (`tool/lsp.ts`, experimental flag `experimentalLspTool`): hover, definition, references,
  document/workspace symbols — uses `filePath` + 1-based `line`/`character`.

---

## 8. Formatters — `packages/opencode/src/format/formatter.ts` + `index.ts`

- Run after `write`/`edit` mutations, matched by extension; each formatter has an `enabled(context)` probe.
- Built-ins: **gofmt, mix (Elixir), prettier, oxfmt (exp), biome, zig, clang-format, ktlint, ruff,
  air (R), uv (Python), rubocop, standardrb, htmlbeautifier (erb), dart, ocamlformat, terraform,
  latexindent, gleam, shfmt, nixfmt, rustfmt, pint (Laravel), ormolu (Haskell), cljfmt, dfmt**.
- Detection mixes `which(bin)` + project-config presence (e.g. prettier in package.json, biome.json,
  ruff in pyproject `[tool.ruff]`). Custom/override via config `formatter` (`core/src/v1/config/formatter.ts`).

---

## 9. Agents / Sub-agents / Skills

### Agent model — `opencode/src/agent/agent.ts`
- `Agent.Info`: `{name, description, mode: "subagent"|"primary"|"all", native?, hidden?, topP?, temperature?,
  color?, permission (ruleset), model?, variant?, prompt?, options, steps?}`.
- Built-in agents (config `agent.*`): **plan, build, general, explore, title, summary, compaction**
  (`core/src/v1/config/config.ts`). System prompts in `agent/prompt/*.txt` and `session/prompt/*.txt`
  (anthropic, beast, codex, gpt, gemini, kimi, default, plan-mode, etc., selected per model).
- **Custom markdown agents** with YAML frontmatter: parsed via `gray-matter` (`config/markdown.ts`,
  `config/agent.ts`), discovered in agent dirs. Frontmatter carries the `Agent.Info` fields; body is the prompt.
- **Modes**: legacy `mode.{build,plan}`; “build/plan” switching prompts (`session/prompt/build-switch.txt`,
  `plan-mode.txt`). Primary vs subagent governs whether `task` can call it.
- **`task` mechanism** (`tool/task.ts`): primary agent spawns a subagent session; the registry’s
  `describeTask()` advertises non-primary agents filtered by `Permission.evaluate("task", name)`.
  V2 model: subagent runs as its own durable session.
- **Agent generation**: `cli/cmd/generate.ts` + `agent/generate.txt` + `agent.generate()` (LLM authors a
  new agent definition: identifier/whenToUse/systemPrompt).

### Skills — `opencode/src/skill/` (`discovery.ts`, `index.ts`) + `core/src/skill/`
- Skills are markdown “capabilities” discovered from skill dirs (config `skills`); invoked by the `skill`
  tool. Guidance injected into system context (`core/src/skill/guidance.ts`). TUI `dialog-skill.tsx`,
  `/skills`. References (`core/src/reference/`) are a parallel concept injected as guidance.

---

## 10. Plugins / Hooks — `packages/plugin/` + `packages/opencode/src/plugin/`

- **Runtime**: a plugin is `Plugin = (input: PluginInput, options?) => Promise<Hooks>`. `PluginInput` gives
  `{client (SDK), project, directory, worktree, serverUrl, $ (Bun shell), experimental_workspace.register}`.
  Loader: `opencode/src/plugin/loader.ts`, install: `plugin/install.ts`, registry: `plugin/index.ts`.
  Disable all with `--pure` (`OPENCODE_PURE`).
- **Distribution**: npm packages or local files; config `plugin: Array<string | [string, options]>`
  (`core/src/v1/config/plugin.ts`, `npm-package-arg`).
- **Hook lifecycle events** (`plugin/src/index.ts → interface Hooks`):
  `dispose`, `event`, `config`, `tool` (register tools), `auth` (AuthHook), `provider` (ProviderHook.models),
  `chat.message`, `chat.params`, `chat.headers`, `permission.ask`, `command.execute.before`,
  `tool.execute.before`, `shell.env`, `tool.execute.after`, `tool.definition`,
  `experimental.chat.messages.transform`, `experimental.chat.system.transform`,
  `experimental.provider.small_model`, `experimental.session.compacting`,
  `experimental.compaction.autocontinue`, `experimental.text.complete`.
- **TUI plugins**: `plugin/src/tui.ts` (custom routes, event bus `TuiEventBus`, `on(type, handler)`);
  feature-plugins in `tui/src/feature-plugins/`, plugin shim `tui/src/plugin/command-shim.ts`.
- Bundled provider plugins: azure, cloudflare, digitalocean, github-copilot, openai, xai
  (`opencode/src/plugin/*`). `cli/cmd/plug.ts` manages plugins.

---

## 11. Server / API / SDK

### HTTP API — `packages/server/src` (V2) + `opencode/src/server` (wiring)
- Route **groups** (definitions) and **handlers** (impls), merged in `server/src/handlers.ts`:
  `health, location, agent, session, message, model, provider, integration, credential, permission,
  fs, command, skill, event, question, reference`.
- Built on Effect `httpapi`; **OpenAPI** via `OpenApi.fromApi(PublicApi)` (`server.ts → openapi()`).
- **SSE `/event`** streams the event bus to clients (group/handler `event`).
- **Session CRUD** + messages: `groups/session.ts`, `groups/message.ts` (create/list/get/delete, prompt,
  abort, share/unshare, revert/redo, summarize, etc.).
- Middleware: `middleware/authorization.ts` (basic auth), `session-location.ts`, `schema-error.ts`;
  CORS `opencode/src/server/cors.ts`.
- `opencode serve` (`cli/cmd/serve.ts`) starts it; `--port/--hostname`, mDNS optional.

### SDK — `packages/sdk/js`
- Generated client (`sdk/js/src/gen` + `sdk/js/src/v2`), build script `sdk/js/script/build.ts`
  (regenerate from OpenAPI). `createOpencodeClient` is what plugins receive as `input.client`.

### Session share
- `opencode/src/share/session.ts` + `share-next.ts`: `share(sessionID) → {url}`, `unshare`,
  upload session to a share service; config `share: "manual"|"auto"|"disabled"` (legacy `autoshare`).
  Viewed in `packages/web` / `packages/app`. CLI export/import: `cli/cmd/export.ts`, `import.ts`.

---

## 12. Config — `packages/core/src/v1/config/config.ts` (+ `opencode/src/config/`)

- **Format**: JSON / JSONC (`jsonc-parser`) `opencode.json[c]`, plus markdown `AGENTS.md`/rules for instructions.
  `$schema` supported. Parsing/layering: `opencode/src/config/{config,parse,paths,markdown,variable}.ts`.
- **Layering/precedence** (low→high): global (`~/.config/opencode` via `xdg-basedir`) → project root →
  nested dirs → env vars (`config/variable.ts` does `{env:...}`/`{file:...}` substitution) → CLI flags.
  Remote/well-known config supported (`WellKnown.config`/`remote_config`).
- **Key fields**: `shell, logLevel, server, command, skills, references, watcher, snapshot, plugin,
  share, autoupdate, disabled_providers, enabled_providers, model, small_model, default_agent, username,
  mode, agent, provider, mcp, formatter, lsp, instructions, permission, tools (enable/disable map),
  attachment, enterprise.url, tool_output.{max_lines,max_bytes}, compaction.{auto,prune,tail_turns,
  preserve_recent_tokens,reserved}, experimental.{disable_paste_summary, batch_tool, openTelemetry,
  primary_tools, continue_loop_on_deny, mcp_timeout, policies, ...}`.
- **AGENTS.md / rules / instructions**: `instructions: string[]` globs plus auto-loaded `AGENTS.md`
  (and legacy rules) are concatenated into the system prompt (`session/instruction.ts`,
  `session/system.ts`). Per-directory instructions supported.

---

## 13. CLI Surface — `packages/opencode/src/index.ts` (yargs) + `cli/cmd/*`

Global flags: `--print-logs`, `--log-level`, `--pure`, `--help/-h`, `--version/-v`, `completion`.
Default (no subcommand) launches the **TUI**.

| Command | File | Purpose |
|---|---|---|
| `run [message]` | `cli/cmd/run.ts` | Headless one-shot/continue. Flags: `--command, --continue, --session, --fork, --share, --model, --agent, --format(default|json), --file, --title, --attach, --username/--password, --dir, --port, --variant, --thinking, --replay, --replay-limit`. |
| `tui` | `cli/cmd/tui.ts` | Explicit TUI (TuiThreadCommand). |
| `serve` | `cli/cmd/serve.ts` | Start HTTP/SSE server. |
| `acp` | `cli/cmd/acp.ts` | Agent Client Protocol over stdio (editors). |
| `attach` | `cli/cmd/attach.ts` | Attach to a running server. |
| `auth` (account/console) | `cli/cmd/account.ts` | `login/list/logout` providers. |
| `providers` | `cli/cmd/providers.ts` | List providers. |
| `models` | `cli/cmd/models.ts` | List models (models.dev). |
| `agent` | `cli/cmd/agent.ts` | Manage/create agents. |
| `generate` | `cli/cmd/generate.ts` | Generate agent/config artifacts. |
| `mcp` | `cli/cmd/mcp.ts` | MCP add/list/auth/connect. |
| `github` | `cli/cmd/github.ts` (+`.handler/.shared`) | GitHub app/action integration. |
| `pr` | `cli/cmd/pr.ts` | PR workflow. |
| `session` | `cli/cmd/session.ts` | Session management. |
| `export` / `import` | `cli/cmd/export.ts` / `import.ts` | Session export/import. |
| `stats` | `cli/cmd/stats.ts` | Usage stats. |
| `web` | `cli/cmd/web.ts` | Web share/app. |
| `upgrade` / `uninstall` | `cli/cmd/upgrade.ts` / `uninstall.ts` | Self-update / remove. |
| `plug` | `cli/cmd/plug.ts` | Plugin management. |
| `db` | `cli/cmd/db.ts` | DB maintenance (V2 SQLite). |
| `debug` | `cli/cmd/debug/` | Debug subcommands. |
| `completion` | (yargs) | Shell completion. |

In-TUI **slash commands** (not CLI): `/help, /new, /sessions, /models, /agents, /themes, /variants,
/diff, /status, /skills, /mcps, /workspaces, /connect, /org, /editor, /move, /warp, /exit, /init, /review`
(the last two from `command/template/{initialize,review}.txt`), plus user/MCP/skill-defined commands.

---

## Capability Checklist

| Capability | Where it lives in opencode | Notes for Cortex Code |
|---|---|---|
| Client/server split (HTTP+SSE) | `server/src/*`, `opencode/src/server/server.ts` | Effect httpapi + OpenAPI; replicate as Rust HTTP server (axum) + SSE `/event`. |
| Event bus | `core/src/event.ts` (`EventV2`), `opencode/src/bus/global.ts` | Typed pub/sub; back SSE + in-proc subscribers. |
| Durable agentic loop | `core/src/session/runner/llm.ts` | MAX_STEPS=25, steer/queue inputs, concurrent tool fibers, overflow→compaction rebuild. **Primary blueprint.** |
| Legacy loop (reference) | `opencode/src/session/prompt.ts` | Feature-complete; uses AI SDK. |
| Native LLM protocols | `packages/llm/src/protocols/*`, `providers/*`, `route/*` | Anthropic/OpenAI Chat+Responses/OpenAI-compat/Gemini/Bedrock; HTTP+WS; normalized `LLMEvent`. Replicate natively in Rust (reqwest+SSE). |
| AI-SDK provider bundle (V1) | `opencode/src/provider/provider.ts` | ~25 `@ai-sdk/*`; in Rust replace with native protocols. |
| models.dev catalog | `core/src/models-dev.ts` | Cached HTTP fetch; cost/context/capabilities. |
| Auth (oauth/api/wellknown) | `opencode/src/auth/index.ts`, plugin `AuthHook` | OAuth+API key, github-copilot/gitlab/poe; credential store. |
| OpenRouter | `llm/src/providers/openrouter.ts`, `@openrouter/ai-sdk-provider` | OpenAI-compatible + models.dev. |
| Tool framework | `opencode/src/tool/tool.ts`, `registry.ts` | Schema-validated params, JSON Schema export, truncation, plugin/custom tools. |
| Built-in tools | `opencode/src/tool/*` (mirror `core/src/tool/*`) | read/write/edit/apply_patch/glob/grep/shell/task/todo/webfetch/websearch/skill/question/lsp/plan. |
| Model-aware tool swap | `registry.ts → tools()` | GPT→apply_patch, others→edit/write. |
| Permission model | `opencode/src/permission/index.ts`+`evaluate.ts` | allow/ask/deny, glob rules, last-match-wins, per-agent, saved decisions, plugin override. |
| Checkpoint/revert | `opencode/src/snapshot/`, `session/revert.ts`, `worktree/`, `git/` | Git snapshots before mutations; undo/redo. |
| TUI | `packages/tui` (OpenTUI + SolidJS) | Streaming md+tree-sitter highlight, diffs (`@pierre/diffs`), tool cards, dialogs, toasts. Rust → ratatui equivalent. |
| Slash commands | `tui/.../prompt/autocomplete.tsx`, `command/index.ts` | `/help /new /models /agents /themes /init /review …` + MCP/skill/user. |
| @ mentions / ! shell / palette | `tui/src/component/prompt/index.tsx`, `command-palette.tsx` | fuzzysort + frecency; shell mode `!`; `ctrl+p` palette. |
| Themes / keybinds | `tui/src/theme/`, `tui/src/config/keybind.ts` | Leader `ctrl+x`; fully overridable. |
| MCP client | `opencode/src/mcp/*` | stdio + StreamableHTTP/SSE; tools/prompts/resources; OAuth; live tool-list updates. |
| LSP | `opencode/src/lsp/*` | JSON-RPC stdio, auto-install, diagnostics→agent, `lsp` tool, ~80 langs. |
| Formatters | `opencode/src/format/formatter.ts` | ~25 formatters, extension+config detection, run post-edit. |
| Agents/sub-agents | `opencode/src/agent/agent.ts`, `tool/task.ts` | built-ins + markdown frontmatter agents; modes; `task` spawns subagents. |
| Skills/references | `opencode/src/skill/*`, `core/src/skill,reference` | markdown capabilities, system-context guidance injection. |
| Plugins/hooks | `packages/plugin/*`, `opencode/src/plugin/*` | ~20 lifecycle hooks, tool/auth/provider registration, npm dist, TUI plugins. |
| HTTP API + OpenAPI | `server/src/groups,handlers`, `server.ts openapi()` | 16 handler groups; generate OpenAPI. |
| Generated SDK | `packages/sdk/js` | regenerate from OpenAPI. |
| Session share | `opencode/src/share/*`, `web`/`app` | share/unshare URL; export/import. |
| Config layering | `core/src/v1/config/config.ts`, `opencode/src/config/*` | JSON(C)+markdown, global→project→nested→env→flags, `{env}/{file}` substitution. |
| AGENTS.md / instructions | `session/instruction.ts`, `system.ts` | auto-loaded + `instructions` globs into system prompt. |
| Compaction | `core/src/session/compaction.ts`, `opencode/src/session/compaction.ts`,`overflow.ts` | auto + overflow recovery; tunable. |
| Tool-output truncation | `opencode/src/tool/truncate.ts` | spill to disk, `tool_output.max_lines/bytes`. |
| Background jobs | `opencode/src/background/job.ts` | long-running shell. |
| ACP / GitHub / PR | `opencode/src/acp`, `cli/cmd/{github,pr}.ts` | editor + CI integrations (optional for MVP). |
| CLI | `opencode/src/index.ts` + `cli/cmd/*` | run/tui/serve/acp/auth/mcp/agent/models/... |
| Persistence | `core/src/database/*` (Drizzle/SQLite), `opencode/src/storage/db.*` | durable sessions/messages; Rust → sqlx/SQLite. |
