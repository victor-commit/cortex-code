# Codex (openai/codex) Architecture — Rust Technical Analysis

Source analyzed locally at `/tmp/codex/codex-rs` (Rust workspace) and `/tmp/codex/codex-cli` (npm wrapper). All paths below are relative to `/tmp/codex/codex-rs` unless noted.

> Context: research for **Cortex Code**, a greenfield Rust terminal coding agent (primary reference: opencode). Codex is the most mature production Rust coding agent in the open, so the value here is concrete Rust crate choices and architectural patterns, not feature parity.

---

## 0. High-level shape & a caution

Codex is a **very large, mature, enterprise-grade Cargo workspace**: ~110 member crates (`Cargo.toml` `[workspace].members`). It has clearly grown well past a "terminal coding agent" into a platform (cloud tasks, app-server/daemon, realtime voice/WebRTC, plugins, connectors, multi-agent, OAuth, OTel, marketplace, V8 "code mode", etc.).

For Cortex Code the lesson is **selective extraction**: the workspace decomposition, the streaming/agent-loop, the provider/transport stack, the tool/sandbox model, and the TUI stack are all worth studying. The bulk of the crates (cloud, realtime, connectors, plugins, app-server, V8) are **out of scope** and should NOT be replicated.

The `codex-cli` directory is just a thin npm package (`package.json`, `bin/`, `scripts/`) that downloads/wraps the platform-specific Rust binary. All real logic is in `codex-rs`.

---

## 1. Workspace / crate structure

Workspace root: `Cargo.toml` (edition 2024, resolver 2, shared `[workspace.dependencies]` and very strict `[workspace.lints.clippy]` — `unwrap_used`/`expect_used` = deny, plus dozens of manual_* lints denied).

### Core agent crates (the ones that matter for Cortex)

| Crate | Path | Responsibility |
|---|---|---|
| `codex-core` | `core/` | The brain. Agentic loop, session/turn/thread management, tool dispatch, sandbox decisions, history/compaction, rollout persistence, MCP integration. ~200 source files. Intended (per README) to become a reusable library. |
| `codex-protocol` | `protocol/` | Wire/domain types shared across crates: `ResponseItem`, `EventMsg`, `AskForApproval`, `TokenUsage`, exec output types, permissions. The serde contract layer. |
| `codex-client` | `codex-client/` | **Generic** HTTP transport: `HttpTransport` trait + `ReqwestTransport`, `Request`/`Response`, retry policy primitives, raw SSE byte→frame helper. Zero OpenAI awareness. |
| `codex-api` | `codex-api/` | **Typed** OpenAI Responses API layer on top of `codex-client`: request builders, SSE→`ResponseEvent` parsing, rate-limit parsing, error mapping, WebSocket Responses transport, auth header injection. |
| `model-provider-info` | `model-provider-info/` | `ModelProviderInfo` config struct + built-in provider catalog (OpenAI, Bedrock, Ollama, LM Studio) and merge logic. |
| `codex-tools` | `tools/` | Tool spec/name primitives (`ToolSpec`, `ToolName`, `ToolExecutor` trait, `ToolCall`). |
| `codex-apply-patch` | `apply-patch/` | The `apply_patch` envelope: lark grammar, parser, streaming parser, fuzzy hunk matching (`seek_sequence`), file application. Can build as standalone executable. |
| `codex-execpolicy` | `execpolicy/` | Starlark-based policy engine deciding whether a command is safe/auto-approvable. |
| `codex-sandboxing` | `sandboxing/` | Cross-platform sandbox abstraction: macOS Seatbelt (`.sbpl` policies), Linux bubblewrap/landlock policy transforms, `SandboxType`, `get_platform_sandbox()`. |
| `codex-linux-sandbox` | `linux-sandbox/` | Linux sandbox runner (bwrap + landlock + seccomp), self-invoked subprocess. |
| `codex-exec-server` | `exec-server/` | Filesystem/exec execution server abstraction (`ExecutorFileSystem`) used by apply-patch and shell. |
| `codex-mcp` | `codex-mcp/` | MCP **client-side integration**: `McpConnectionManager`, tool listing/aggregation, elicitation, OAuth, "codex apps" connectors. |
| `codex-rmcp-client` | `rmcp-client/` | Low-level MCP client built on the `rmcp` crate: stdio/streamable-HTTP/in-process transports, OAuth login flows. |
| `codex-mcp-server` | `mcp-server/` | Codex-**as-an-MCP-server** (`codex mcp-server`) so other agents can call Codex. |
| `codex-config` | `config/` | TOML config loading, profiles, permissions, MCP edit, keymap, requirements/diagnostics. |
| `codex-tui` | `tui/` | Ratatui full-screen TUI (the default UX). ~150 files. |
| `codex-exec` | `exec/` | Headless/non-interactive runner (`codex exec PROMPT`). |
| `codex-cli` | `cli/` | clap multitool dispatching to all subcommands. |
| `codex-arg0` | `arg0/` | argv[0] dispatch + Tokio runtime bootstrap (one binary, many personalities via busybox-style dispatch). |
| `codex-login` | `login/` | ChatGPT/OpenAI OAuth + API key auth, `auth.json`. |
| `codex-rollout` | `rollout/` | Session persistence (JSONL "rollout" files) for resume/fork. |
| `codex-otel` | `otel/` | OpenTelemetry/tracing wiring. |

Plus ~30 small `utils/*` leaf crates (e.g. `utils/stream-parser`, `utils/output-truncation`, `utils/fuzzy-match`, `utils/pty`, `utils/rustls-provider`, `utils/absolute-path`). The **micro-crate utils pattern** keeps compile units small and dependencies explicit.

**Pattern worth noting:** clean dependency layering `codex-client` (generic transport) → `codex-api` (typed Responses) → `codex-core` (business logic) → `tui`/`exec`/`cli` (frontends). This separation is the single best structural idea to copy.

---

## 2. The agentic loop in Rust

### Runtime
- **tokio multi-thread runtime**, bootstrapped in `arg0/src/lib.rs` (`build_runtime()` → `Builder::new_multi_thread()`), then `runtime.block_on(main_fn)`. Apply-patch self-invocation uses a `new_current_thread` runtime.
- Heavy use of `Arc<Session>`, `Arc<TurnContext>`, `tokio::sync::{mpsc, Mutex, oneshot}`, `tokio_util::sync::CancellationToken` for structured cancellation, and `async-channel`/`crossbeam-channel` for some fan-out.

### Loop structure (`core/src/session/turn.rs`, ~2200 lines)
The conversation is modeled as **Thread → Turn → Sampling request → Tool calls**:

1. `run_turn(...)` (line 131) — top-level per-user-submission. Sets up `TurnDiffTracker`, drains pending input, runs hooks, then enters a `loop`.
2. Inside the loop it calls `run_sampling_request(...)` (line ~890) which itself loops for **stream retries** (provider `stream_max_retries`, exponential `backoff(retries)`, and a WebSocket→HTTPS fallback transport switch).
3. `try_run_sampling_request(...)` does one model call: builds a `Prompt` (`build_prompt`, line 862 — input items + `router.model_visible_specs()` + parallel-tool-call flag + base instructions + optional output JSON schema), streams the response, and dispatches tool calls.
4. Tool calls flow through `ToolCallRuntime` → `ToolRouter` (`core/src/tools/router.rs`) → `ToolRegistry.dispatch_any(...)`. `ToolRouter::build_tool_call` maps a `ResponseItem::FunctionCall`/`CustomToolCall`/`ToolSearchCall` into a typed `ToolCall { tool_name, call_id, payload }`.
5. Tool results are appended to history as `ResponseItem`s; the loop continues while `needs_follow_up` (model wants to continue OR pending user input exists).
6. **Auto-compaction**: when token limit reached and follow-up needed, `run_auto_compact(...)` summarizes history mid-turn and continues. Also a pre-sampling compact pass.
7. Turn ends when the model affirmatively ends its turn (`end_turn`) and no pending input; stop-hooks can force continuation.

### Streaming
- The model response is an async `Stream` of `ResponseEvent` (see §3). The loop consumes deltas (`OutputTextDelta`, `ReasoningSummaryDelta`, `ToolCallInputDelta`) and terminal items (`OutputItemDone`, `Completed`).
- `client_common::ResponseStream` wraps an `mpsc::Receiver<Result<ResponseEvent>>` and, on `Drop`, fires a `CancellationToken` to tell the upstream mapper task the consumer left — clean backpressure/teardown.

**Key idea to copy:** the explicit nested-loop design — outer turn loop, inner retry loop, single-shot request fn — with `CancellationToken` threaded through everything, and tool dispatch decoupled behind a registry/router.

---

## 3. Provider / model integration

### Transport stack (two layers)
- **`codex-client`** (`transport.rs`): `HttpTransport` trait with `execute()` (unary) and `stream()` (returns `StreamResponse { status, headers, bytes: ByteStream }`). `ReqwestTransport` wraps `reqwest::Client` and uses `resp.bytes_stream()` for streaming. `sse.rs` turns a `ByteStream` into SSE `data:` frames via `eventsource_stream::Eventsource` with an **idle timeout** (`tokio::time::timeout`).
- **`codex-api`** (`sse/responses.rs`): `spawn_response_stream()` parses response headers (rate limits, `openai-model`, `x-reasoning-included`, `x-request-id`, turn-state), spawns a tokio task running `process_sse()`, and returns a `ResponseStream` of `ResponseEvent`. `process_sse` loops over SSE frames with idle-timeout, deserializes each into `ResponsesStreamEvent`, and maps via `process_responses_event()` to `ResponseEvent` variants (text delta, reasoning delta, tool-call input delta, output item done/added, completed-with-usage, rate limits, etc.). Errors like `context_length_exceeded`, `insufficient_quota`, `rate_limit_exceeded` (with retry-after regex parsing), `server_is_overloaded` are mapped to typed `ApiError`.

### Wire API
- **Responses API only.** `WireApi` enum in `model-provider-info/src/lib.rs` now has a single `Responses` variant; `wire_api = "chat"` is **explicitly removed** with an error message. This is a notable divergence from most agents (which use Chat Completions). Codex bet fully on `/v1/responses`.
- Request body: `ResponsesApiRequest` (`codex-api/src/common.rs`) — `model`, `instructions`, `input: Vec<ResponseItem>`, `tools: Vec<Value>`, `tool_choice`, `parallel_tool_calls`, `reasoning {effort, summary}`, `store`, `stream`, `include`, `text` (verbosity + JSON-schema output format), `prompt_cache_key`, `service_tier`, `client_metadata`.
- Also supports **Responses-over-WebSocket** (`ResponseCreateWsRequest`, `supports_websockets` flag) for lower latency, with HTTPS fallback.

### Provider config (`ModelProviderInfo`)
Fields: `name`, `base_url`, `env_key` (env var name for API key), `experimental_bearer_token`, `auth` (command-backed bearer), `aws` (SigV4), `wire_api`, `query_params`, `http_headers`, `env_http_headers` (header→env-var mapping), retry/timeout knobs, `requires_openai_auth`, `supports_websockets`.

- `api_key()` reads the key from the configured env var.
- `to_api_provider()` builds the `codex-api::Provider` (base URL, header map, `RetryConfig`, stream idle timeout). Default base URL is `https://api.openai.com/v1` (or `https://chatgpt.com/backend-api/codex` for ChatGPT auth).
- Built-in providers: OpenAI, Amazon Bedrock, Ollama, LM Studio. Users add more via `[model_providers.*]` in `config.toml`.

### OpenAI-compatible / OpenRouter feasibility
**Feasible but with a caveat.** Because base URL, headers, env-key, and query params are all configurable, pointing at any OpenAI-compatible endpoint is straightforward — **as long as it speaks the Responses API**. OpenRouter / most third-party gateways speak **Chat Completions**, which Codex has now removed. So Codex's own provider layer is *not* directly reusable for OpenRouter unless you re-introduce a Chat-Completions wire path.

> **For Cortex:** keep Codex's clean `Provider`/transport split, but design the wire layer as a trait with **both** a Responses adapter and a Chat-Completions adapter so OpenRouter/OpenAI-compatible/Anthropic backends work. This is exactly where opencode's multi-provider model is the better reference.

---

## 4. Tooling

### Tools exposed (handlers in `core/src/tools/handlers/`)
- **`shell` / unified_exec** (`shell.rs`, `unified_exec.rs`) — run commands, with PTY support (`portable-pty`), sandboxed, with approval flow and per-call additional-permission requests.
- **`apply_patch`** (`apply_patch.rs`) — the structured edit tool (see below).
- **`view_image`** — feed an image into context.
- **`request_user_input`, `request_permissions`, `request_plugin_install`** — interaction/approval tools.
- **`plan`, goal tools** (`CreateGoal/UpdateGoal/GetGoal`) — planning/goal tracking.
- **`tool_search`** — dynamic tool discovery (lazy-exposed MCP tools).
- **`mcp` / `mcp_resource`** — call MCP server tools and read MCP resources.
- **`multi_agents`** (incl. v2), `test_sync`, `list_available_plugins_to_install` — platform extras.

Tools are registered into a `ToolRegistry`; `ToolRouter::model_visible_specs()` produces the spec list sent to the model. Each handler implements a `ToolExecutor`-style trait and a "spec" describing its JSON schema. Output is truncated for the model via `utils/output-truncation` (`format_exec_output_for_model` adds exit code + wall time + truncation notice).

### apply_patch (`apply-patch/`)
- Custom patch envelope (`*** Begin Patch` / `*** Update File:` / `@@` hunks), defined by `apply_patch.lark` grammar, parsed by `parser.rs`, with a **streaming parser** (`streaming_parser.rs`) so edits render live.
- **Fuzzy matching** via `seek_sequence.rs` (tolerates whitespace/line drift) — applies hunks even when context lines don't match exactly. Uses `similar` for diffs and `diffy`.
- Applies via the `ExecutorFileSystem` abstraction (so it can run inside the sandbox / over the exec-server).
- Can be invoked as a standalone executable (`--codex-run-as-apply-patch`) so it runs inside the sandbox as a subprocess.

### Sandbox / approval model
`AskForApproval` policy: `UnlessTrusted`, `OnFailure`, `OnRequest`, `Never`, `Granular(...)`. Safety decisions live in `core/src/safety.rs`:
- `assess_patch_safety()` / command safety return `SafetyCheck::{AutoApprove{sandbox_type, user_explicitly_approved}, AskUser, Reject{reason}}`.
- Auto-approve only when a real sandbox can be enforced (`get_platform_sandbox()` returns `Some`); otherwise ask or reject.
- Writes are checked against **writable roots** (`FileSystemSandboxPolicy::can_write_path_with_cwd`), with hard-link escape protection (still sandboxes even if paths look constrained).
- `--sandbox` CLI flag: `read-only` | `workspace-write` | `danger-full-access`.

---

## 5. TUI

- **Ratatui** (`ratatui = "0.29"`, patched to a `nornagon/ratatui` fork) + **crossterm 0.28** (also a fork). `ratatui-macros`, `ansi-to-tui`, `syntect` (syntax highlighting), `textwrap`, `unicode-width`/`unicode-segmentation` for layout, `image` for inline images.
- Entry: `tui/src/main.rs` → `codex_tui::run_main`. `tui/src/tui.rs` owns terminal lifecycle: `enable_raw_mode`, `EnterAlternateScreen`, bracketed paste, focus change, keyboard-enhancement detection, a **custom `Terminal`** wrapper (`custom_terminal.rs`) and a `TuiEventStream` (async crossterm `EventStream`).
- Architecture is **event-driven**: an `AppEvent` enum (`app_event.rs`, ~33KB) flows over a channel (`app_event_sender.rs`); `app.rs` (~56KB) is the top-level state machine; `chatwidget.rs` (~79KB) renders the conversation. Streaming model deltas are turned into history cells (`history_cell/`, `insert_history.rs`, `markdown_stream.rs`).
- Rich rendering subsystems: `markdown_render.rs` (~88KB) + `markdown_stream.rs` for live markdown, `diff_render.rs` (~95KB) for patch/diff display, `pager_overlay.rs`, `transcript_reflow.rs`, `wrapping.rs`, theme/palette detection (`terminal_palette.rs`, `theme_picker.rs`), a `bottom_pane/` input area, slash commands (`slash_command.rs`), and even pets/ascii animation.
- Snapshot testing with `insta` (`snapshots/` dirs everywhere) is used heavily to lock TUI rendering.

**Caution:** the TUI is enormous and over-featured. The *patterns* (event-channel + state machine + history-cell rendering + insta snapshots + custom terminal wrapper to inline scrollback history) are great; the *volume* is not something to copy wholesale.

---

## 6. MCP support

Codex is both an MCP **client** and an MCP **server**.

- **Client** (`codex-mcp/` + `rmcp-client/`): built on the official **`rmcp` crate** (`rmcp = "0.15"`). `McpConnectionManager` (`codex-mcp/src/connection_manager.rs`) manages multiple server connections, aggregates `list_all_tools()`, namespaces tool names, and handles elicitation + OAuth. Transports (`rmcp-client/src/rmcp_client.rs`): **stdio** (child process via `StdioServerLauncher`), **streamable HTTP** (`StreamableHttpClientTransport`, with optional OAuth `AuthClient`), and **in-process** (`InProcessTransportFactory`). Full **OAuth login** flows for HTTP MCP servers (`oauth.rs`, `perform_oauth_login.rs`).
- **Server** (`mcp-server/`): `codex mcp-server` exposes Codex itself as an MCP tool to other agents.
- **Management**: `codex mcp add/list/get/remove` (CLI `mcp_cmd.rs`) edits `config.toml`; `config/src/mcp_edit.rs`, `mcp_types.rs`.
- Tools can be **lazily exposed** (`deferred_mcp_tools` + `tool_search`) to keep the model's tool list small when many MCP tools exist.

**For Cortex:** use the `rmcp` crate directly. Codex's `McpConnectionManager` + transport-recipe pattern (stdio / streamable-http / in-process behind one enum) is the reference design for MCP client support.

---

## 7. Config format & CLI surface

### Config
- **TOML** at `$CODEX_HOME/config.toml` (default `~/.codex/`). Loaded by `codex-config` (`config/src/loader/`, `config_toml.rs`, `types.rs` ~35KB). Supports **profiles**, layered overrides, `-c key=value` CLI overrides (`utils/json-to-toml` converts CLI values), strict unknown-key detection (`strict_config.rs`, `serde_ignored`).
- Key sections: model + provider selection, `[model_providers.*]`, `[mcp_servers.*]`, permissions/sandbox, hooks, keymap (`tui_keymap.rs`), skills, plugins, `notify`.
- Auth stored separately in `auth.json` (managed by `codex-login`).
- Session state persisted as **JSONL rollout files** (`codex-rollout`) enabling `codex resume` / fork.

### CLI (`cli/src/main.rs`, clap derive)
Default (no subcommand) → interactive TUI. Subcommands: `exec` (headless), `login`/`logout`, `mcp` (manage) + `mcp-server` (run), `sandbox` (test sandbox), `execpolicy`, `resume`, `cloud`, `apply` (apply patch), `app-server`/`exec-server`, `debug`, `features`, `plugin`, `marketplace`, `doctor`. One binary, busybox-style argv[0] dispatch via `codex-arg0`.

---

## 8. Safety / sandboxing details

- **macOS: Seatbelt** (`sandboxing/src/seatbelt.rs` + `.sbpl` policy files: `seatbelt_base_policy.sbpl`, `seatbelt_network_policy.sbpl`, `restricted_read_only_platform_defaults.sbpl`). Spawns the command under `sandbox-exec` with a generated profile.
- **Linux: bubblewrap + Landlock + seccomp** (`sandboxing/src/landlock.rs`, `policy_transforms.rs`; runner in `linux-sandbox/`). Uses crates `landlock = "0.4"`, `seccompiler = "0.5"`, bundled `bwrap`. Self-invokes (`CODEX_LINUX_SANDBOX_ARG0`) to set up the sandbox in a child.
- **Windows**: restricted token / `windows-sandbox-rs`.
- `SandboxType` enum: `None`, `MacosSeatbelt`, (Linux variants). `get_platform_sandbox()` returns the platform default.
- **Network policy**: separate network-proxy crate + `seatbelt_network_policy.sbpl`; workspace-write sandbox blocks network by default.
- **execpolicy** (`execpolicy/`): a **Starlark**-based (`starlark = "0.13"`) rules engine deciding command auto-approval, with extensive tests. Distinct from the OS sandbox: this is "should we even let the model run this without asking".
- Defense in depth: model proposes → execpolicy/safety classifies → OS sandbox enforces → approval prompt for anything not auto-approvable.

---

## 9. Rust dependencies worth reusing (actual versions from root `Cargo.toml`)

| Concern | Crate(s) Codex uses |
|---|---|
| Async runtime | `tokio = "1"` (multi-thread), `tokio-stream`, `tokio-util` (CancellationToken), `async-trait`, `async-stream`, `futures`, `async-channel`, `crossbeam-channel` |
| HTTP client | `reqwest = "0.12"` (with cookies), `http = "1.3"` |
| TLS | `rustls = "0.23"` (ring), `rustls-native-certs`, `rustls-pki-types`, custom `utils/rustls-provider` |
| SSE parsing | **`eventsource-stream = "0.2"`** (the key one) |
| WebSocket | `tokio-tungstenite = "0.28"`, `tungstenite` (forks for proxy/deflate) |
| JSON / serde | `serde`, `serde_json`, `serde_with`, `serde_path_to_error`, `serde_ignored`, `schemars` (JSON schema), `jsonptr` |
| TOML config | `toml = "0.9"`, `toml_edit`, `serde_yaml` |
| CLI | `clap = "4"` (derive), `clap_complete` |
| TUI | `ratatui = "0.29"` (forked), `crossterm = "0.28"` (forked), `ratatui-macros`, `ansi-to-tui`, `syntect`, `textwrap`, `unicode-width`, `unicode-segmentation`, `owo-colors`, `arboard` (clipboard) |
| MCP | **`rmcp = "0.15"`** |
| Diffing / patch | `similar = "2.7"`, `diffy = "0.4"` |
| Fuzzy / search | `nucleo` (helix fuzzy matcher), `bm25`, `ignore`, `globset`, `glob`, `wildmatch` |
| Process / PTY | `portable-pty = "0.9"`, `shlex`, `which` |
| Tree-sitter | `tree-sitter = "0.25"`, `tree-sitter-bash` (command parsing) |
| Sandbox | `landlock`, `seccompiler` (Linux); `.sbpl` for macOS Seatbelt |
| Policy engine | `starlark = "0.13"` |
| Errors / logging | `anyhow`, `thiserror = "2"`, `tracing` + `tracing-subscriber` + `tracing-appender`, `color-eyre` |
| Telemetry | `opentelemetry` 0.31 stack, `sentry` |
| Storage | `sqlx = "0.8"` (sqlite, for state DB) |
| Auth/crypto | `jsonwebtoken`, `keyring`, `ed25519-dalek`, `base64`, `sha2` |
| Testing | `insta` (snapshots), `wiremock`, `assert_cmd`, `predicates`, `pretty_assertions`, `test-log`, `tokio-test` |
| Misc | `uuid`, `chrono`/`time`, `dirs`, `url`, `bytes`, `indexmap`, `strum` |

---

## What to borrow for Cortex Code (Rust)

### Adopt (high confidence)
1. **Layered transport architecture.** Mirror `codex-client` (generic `HttpTransport` trait + `ReqwestTransport` + raw SSE helper) → `codex-api` (typed wire layer) → `core`. This trait-based seam makes providers, retries, and testing (wiremock) clean. *Single best idea here.*
2. **Streaming as `Stream<Item = Result<ResponseEvent>>`** backed by `mpsc` + a tokio task, with a `Drop`-fires-`CancellationToken` teardown. Use `eventsource-stream` for SSE with an **idle timeout** (`tokio::time::timeout` per frame).
3. **Event-typed model output.** A `ResponseEvent` enum (text delta, reasoning delta, tool-call input delta, item done, completed-with-token-usage, rate limits) decouples the agent loop from wire format and from the renderer.
4. **Agent loop shape:** outer turn loop → inner stream-retry loop (exp backoff + transport fallback) → single-shot request fn; `CancellationToken` threaded everywhere; tool dispatch behind a **`ToolRegistry` + `ToolRouter`** so tools are pluggable and parallel-capable.
5. **Tool trait + JSON-schema spec per tool**, with model-visible spec list generated from the registry, and **output truncation** before sending results back (exit code + wall time + truncation notice).
6. **apply_patch design:** envelope + grammar + **streaming parser** + **fuzzy hunk matching** (`seek_sequence`, `similar`). This is more robust than raw unified diffs for LLM-generated edits. Strongly consider porting the matching strategy.
7. **MCP via `rmcp`** with a connection-manager that aggregates tools across servers and a transport-recipe enum (stdio / streamable-http / in-process). Support lazy tool exposure + `tool_search` if many tools.
8. **Sandbox abstraction** (`SandboxType` + `get_platform_sandbox()`) with a `SafetyCheck { AutoApprove / AskUser / Reject }` decision type and writable-root checks. Use `landlock`+`seccompiler` on Linux, Seatbelt `.sbpl` on macOS. Auto-approve only when a sandbox can actually be enforced.
9. **TOML config** with profiles, `-c key=value` overrides, strict unknown-key detection (`serde_ignored`), `schemars` for schema; separate `auth.json`; **JSONL rollout files** for resume/fork.
10. **TUI patterns:** Ratatui + crossterm `EventStream`, an `AppEvent` channel + central state machine, history-cell rendering for streamed markdown, and **`insta` snapshot tests** to lock rendering. Use `syntect` for highlighting.
11. **Strict workspace hygiene:** workspace-level `[workspace.dependencies]`, edition 2024, and the **deny-list clippy lints** (`unwrap_used`, `expect_used`, redundant_clone, etc.). Cheap to adopt early, expensive to retrofit.
12. **One binary, multiple frontends** (interactive TUI default + `exec` headless + `mcp-server`) sharing the `core` library. Keep `core` frontend-agnostic from day one.

### Adapt / diverge
- **Do NOT go Responses-API-only.** Codex removed Chat Completions, which breaks OpenRouter and most OpenAI-compatible/third-party gateways. Cortex should make the **wire protocol a trait** with both a Responses adapter and a Chat-Completions adapter (and room for Anthropic). This is where **opencode is the better reference**.
- Provider config struct (`ModelProviderInfo`: base_url / env_key / headers / env_http_headers / retry knobs) is an excellent template — keep it, just add a `wire_api` that actually has multiple variants.

### Avoid (scope/complexity traps)
- The sheer **crate sprawl** (~110 crates: cloud-tasks, realtime/WebRTC, connectors, plugins/marketplace, app-server + daemon, V8 "code mode", multi-agent v2, goals/guardian/memories). Start with ~8-12 focused crates; resist platformization.
- **Forked ratatui/crossterm/tungstenite** — Codex maintains private forks (`nornagon/ratatui`, etc.). Prefer upstream crates; only fork if you hit a hard wall.
- Enterprise extras: OTel/Sentry, AWS Bedrock SigV4, keyring, attestation, OAuth-for-MCP, Starlark execpolicy — valuable later, but **not** for an MVP. (Note `starlark` execpolicy is powerful but heavy; a simpler allow/deny rule set is fine initially.)
- Giant single files (e.g. `tui/src/lib.rs` ~100KB, `core/src/session/mod.rs` ~129KB, `turn.rs` ~85KB) — learn the patterns, but keep Cortex's modules smaller.
