# Cortex Code

Cortex Code is an agentic coding tool for the terminal, shipped as a single Rust
binary named `cortex`. It runs real model turns, edits and runs code through a
gated tool layer, and persists every session so runs can be resumed, replayed,
exported, and inspected.

Live model turns go through [OpenRouter](https://openrouter.ai); the default
model is `google/gemini-2.5-flash` and the provider layer is multi-protocol
(OpenAI Chat, Anthropic Messages, Gemini, and OpenAI-compatible endpoints).
Sessions, messages, usage, and saved decisions are stored locally in SQLite.

## Surfaces

One binary exposes several surfaces over the same session engine:

- **TUI** (`cortex`) — interactive terminal UI with streamed markdown +
  tree-sitter syntax highlighting, readable diffs, expand/collapse tool cards,
  slash commands, `@file` mentions, a `!` shell mode, a command palette, themes,
  keybindings, session pickers, in-chat search, toasts, and undo/redo.
- **Headless CLI** (`cortex run`) — scriptable one-shot/continue turns that read
  the prompt from an argument, stdin, or both, with strict `--format json` and
  `--jsonl` output for scripts and CI.
- **Local server** (`cortex serve`) — an axum loopback HTTP server with an SSE
  `/event` stream, session CRUD, an OpenAPI document, and remote TUI control.
- **ACP** (`cortex acp`) — an Agent Client Protocol stdio server (a working
  subset) for editor integrations.
- **TypeScript SDK** (`sdk/`) — a typed Node/TypeScript client, event stream,
  and process launcher for the local server, generated from its OpenAPI schema.

## Features

- **Providers & models** — OpenRouter by default, a multi-protocol provider
  layer, and a dynamic multi-provider model catalogue in the `/models`
  picker. The picker auto-refreshes the live models.dev catalogue in the
  background on startup and each time it opens (no manual `cortex models
  --refresh` needed): the offline list shows instantly with a subtle
  refreshing indicator, then the fetched catalogue swaps in place and a
  successful fetch warms the on-disk cache next to `auth.json`. The catalogue
  is curated from every supported models.dev provider (endpoint overridable
  via `CORTEX_MODELS_DEV_URL`) and grouped by provider with the latest models
  first; the primary provider is curated dynamically, newest-first and capped
  per vendor, so current models beyond the embedded snapshot appear. Entries
  carry honest metadata (context length, price, capability descriptors), and
  providers you hold a credential for are marked as connected. Offline
  (`CORTEX_NO_NET` or the outbound guards) it resolves from the cache, then
  the embedded snapshot, with no network attempt, hang, or error. `cortex
  models --refresh` still updates it from the CLI.
- **Mid-session connect** — the `/connect` slash command opens a
  provider-connect flow in the mono-orange auth style: pick a provider, choose
  an auth method (API key always, OAuth only where the provider supports it),
  and enter a masked API key (never echoed) that is stored `0600` in
  `auth.json`. The provider is usable immediately and shows as connected in
  `/models` with no restart. OAuth-capable providers use a real loopback OAuth
  flow (callback on the reserved 3200-3299 port range, CSRF-protected), and a
  connected provider can be disconnected from the same flow (confirm before
  remove), which removes the stored credential. Secrets live only in the `0600`
  `auth.json` and are never printed, logged, or echoed.
- **Tools** — read, write, edit, apply_patch, glob, grep (ripgrep), bash (with
  background jobs), todo, webfetch, websearch, the `task` subagent tool, an `lsp`
  tool, `skill`, and `question`, with large-output truncation.
- **Permissions, policy & trust** — allow/ask/deny rules over glob and bash
  patterns, last-match-wins, per-agent scoping, and saved decisions.
- **Checkpoints** — shadow-git snapshots before mutations, with `undo`/`redo`.
- **MCP** — client support for stdio servers and remote HTTP/SSE servers,
  including OAuth-protected remotes, with live tool-list updates.
- **LSP** — diagnostics fed back into the agent after edits, plus navigation.
- **Formatters** — run after writes/edits for supported languages.
- **Agents** — built-in personas plus custom markdown agents (frontmatter),
  agent generation, and real `task` subagents.
- **Skills & memory** — guidance injection from skills, plus layered project
  memory (`AGENTS.md`, `CLAUDE.md`, `CONTEXT.md`, global and dynamic sources).
- **Context compaction** — automatic compaction and overflow recovery.
- **Audit & redaction** — wired audit logging and secret redaction across
  surfaces, with a `CORTEX_NO_NET` offline kill switch.
- **Plugins** — project plugins and lifecycle hooks (see [Plugins](#plugins)).
- **Persistence & inspection** — SQLite-backed sessions plus `stats`, `search`,
  `export`/`import`, and `replay`.

## Plugins

Project plugins are local JavaScript modules listed in config (for example
`plugin = ["./my-plugin.mjs"]` or `[spec, options]` pairs). They execute in an
external runtime with fixed discovery order: **Bun**, then **Node**, then
**Deno**. A configured plugin is never silently skipped; missing runtime,
missing file, or a load error fail closed with a clear diagnostic.

### Request-shaping hook composition

Request-shaping hooks rewrite the outbound LLM request and advertised tool
schemas before a turn leaves the surface. Composition is **configuration order**:
plugins are visited in the order they appear in the effective `plugin = [...]`
list. Each hook's return value becomes the next plugin's input for that phase,
so the order is deterministic and stable across runs.

Phases applied to each shaped LLM request:

1. `chat.message` — per message, plugins in config order
2. `experimental.chat.messages.transform` (alias `chat.messages`) — whole array
3. `experimental.chat.system.transform` (alias `chat.system`) — system text
4. `chat.params` — provider parameter map
5. `chat.headers` — extra HTTP headers

Tool schemas are shaped separately when definitions are advertised:

6. `tool.definition` — per tool definition, plugins in config order

If plugins `a` and `b` are listed as `plugin = ["./a.mjs", "./b.mjs"]` and both
register the same hook, every run observes `a` then `b`. Pure mode
(`--pure` / `CORTEX_PURE`) and pure management probes disable plugin loading
entirely. The crate docs in `plugins` (`crates/plugins/src/lib.rs`) are the
authoritative API-facing statement of this contract; black-box tests in
`crates/cli/tests/plugin_request_shaping_hooks.rs` pin the reordering behavior.

## Design & UX

The interactive TUI and the headless CLI share a mono-orange design system: an
`ORANGE`/`ORANGE_D`/`AMBER` palette on a near-black `#0d0d0d` base, with an
explicit background painted on every cell so nothing falls through to the
terminal default. The visual language is safe-glyph only (no braille), using the
`✦` author mark, `▶`/`▼` disclosure toggles, and rounded card, panel, and modal
corners (`╭ ╮ ╰ ╯`); selection uses an inverse selection-bar highlight.

- **Themes** — five chrome themes are reachable from the `/themes` gallery:
  `dark` (default), `light`, `monokai`, `ocean`, and `vscode`. In-chat and diff
  code use a VS Code Dark+ syntax palette, while the gallery previews render each
  theme in its own palette.
- **Animations** — a safe spinner, a token-streaming reveal with a blinking
  cursor, toast slide/fade, selection highlight, and eighths-based progress. All
  animations honor `--no-anim` (or `CORTEX_NO_ANIM=1`) and reduced motion, which
  disable the animated reveal without changing the underlying streaming or
  network delivery.
- **First-run onboarding** — a welcome screen appears on first run, and
  `cortex auth login` (with no provider argument, on a terminal) opens an
  interactive provider picker with masked API-key input (the key is never echoed
  or logged) written to the `0600` auth store.
- **Styled CLI** — the headless CLI uses truecolor ANSI styling that is
  TTY-gated: it auto-disables when output is piped or redirected and honors
  `NO_COLOR`. Machine output stays byte-stable, so `--format json` and `--jsonl`
  are byte-identical whether piped or on a TTY and carry zero ANSI when piped.
  Styled surfaces include `--help`, the `serve` banner and `-v` access log, and
  `run`, `stats`, `search`, `models`, and `replay`.

## Workspace layout

The repository is a Cargo workspace (`crates/`) whose `cli` crate builds the
`cortex` binary. The crates group into a few layers:

- **Foundation** — `protocol`, `errors`, `events`, `redaction`, `settings`,
  `db`.
- **Model & engine** — `llm`, `providers`, `engine` (the durable session
  runner), plus `format` and `memory`.
- **Tools & policy** — `tools`, `permissions`, `policy`, `checkpoints`, `mcp`,
  `lsp`, `plugins`, `skills`, `audit`.
- **TUI framework** — `tui-core`, `tui-buffer`, `tui-text`, `tui-widgets`,
  `tui-syntax`, `frame`, `tui-components`, and the `tui` application crate.
- **Surfaces** — `server` (HTTP/SSE/OpenAPI), `acp`, and `cli` (the binary).

The `sdk/` directory holds the TypeScript SDK for the local server.

## Prerequisites

- A Rust toolchain on **edition 2024** (Rust 1.85+).
- An OpenRouter API key for live LLM runs.

## Setup

Build the binary:

```bash
cargo build -p cli
./target/debug/cortex --version
```

### API key

Place your key in a local, gitignored `.env.local` at the repo root and source
it into the shell:

```bash
# .env.local
OPENROUTER_API_KEY=your-key
```

```bash
set -a; . ./.env.local; set +a
```

Alternatively, export it directly:

```bash
export OPENROUTER_API_KEY="your-key"
```

or store it in the local `0600` auth store:

```bash
./target/debug/cortex auth login openrouter --api-key "$OPENROUTER_API_KEY"
./target/debug/cortex auth list
```

You can also connect providers from inside the TUI with the `/connect` slash
command, which stores the key in the same `0600` `auth.json` and marks the
provider as connected in `/models` without a restart.

`.env.local` is gitignored; never commit your key.

## Run

Launch the interactive TUI:

```bash
./target/debug/cortex
```

Disable animations (reduced motion) by launching the TUI subcommand with
`--no-anim`, or set `CORTEX_NO_ANIM=1` for the bare invocation:

```bash
./target/debug/cortex tui --no-anim
CORTEX_NO_ANIM=1 ./target/debug/cortex
```

Run a headless live turn:

```bash
./target/debug/cortex run "Say hello from Cortex Code in one short sentence."
./target/debug/cortex run --format json "Reply with a short greeting." | jq .
./target/debug/cortex run --jsonl "Stream a short greeting."
```

Start the local server:

```bash
./target/debug/cortex serve --hostname 127.0.0.1 --port 3200
```

Run the ACP stdio server (for editor integrations):

```bash
./target/debug/cortex acp
```

## Useful CLI surfaces

Most inspection commands support `--format json`.

```bash
./target/debug/cortex config --format json
./target/debug/cortex models
./target/debug/cortex models --refresh
./target/debug/cortex search "TODO"

./target/debug/cortex session new --title "work item"
./target/debug/cortex session list
./target/debug/cortex session show <session-id>
./target/debug/cortex replay <session-id>
./target/debug/cortex stats

./target/debug/cortex mcp list
./target/debug/cortex lsp status
```

Headless runs can gate built-in, plugin, and MCP tools with permission rules:

```bash
./target/debug/cortex run --permission-deny 'bash:rm *' "Inspect the project safely."
```

## TypeScript SDK

The SDK under `sdk/` targets the local server. From `sdk/`:

```bash
npm install
npm run typecheck
npm test
```

Regenerate the typed client from the server's OpenAPI document with
`npm run generate`.

## Persistence

Sessions, messages, usage data, and saved local decisions are stored in SQLite.
The default database is:

```text
${XDG_DATA_HOME:-$HOME/.local/share}/cortex/cortex.db
```

Set `CORTEX_DB_PATH` to use a specific database file, or `CORTEX_DATA_DIR` to
move the Cortex data directory.

## Network controls

Set `CORTEX_NO_NET=1` to fail closed before any outbound network call:

```bash
CORTEX_NO_NET=1 ./target/debug/cortex run "Say hello"
CORTEX_NO_NET=1 ./target/debug/cortex net check
```

Default features include network and the local server surface; network is
**additive**. A minimal offline-only binary compiles without those defaults:

```bash
cargo build -p cli --no-default-features
./target/debug/cortex --version
```

That build still runs local commands (`--help`, `config`, `search`, `session`,
`stats`, `replay`, `export`/`import`, completions). Network-backed paths fail
closed until you rebuild with the default features (or `--features network`).

## Test and validate

Build, typecheck, and lint:

```bash
cargo build -p cli
cargo check --workspace
cargo clippy --workspace --no-deps
```

Run the offline workspace suite. Several integration tests read the Cortex data
directory, model cache, and temp state, and a few share ports or process groups, so
isolate `HOME`/`TMPDIR` and run the suite single-threaded. Live OpenRouter tests skip
automatically when `OPENROUTER_API_KEY` is not set rather than failing:

```bash
old_home="$HOME"
test_home="$(mktemp -d)"
test_tmp="$test_home/tmp"
mkdir -p "$test_tmp"
HOME="$test_home" TMPDIR="$test_tmp" \
  CARGO_HOME="${CARGO_HOME:-$old_home/.cargo}" \
  RUSTUP_HOME="${RUSTUP_HOME:-$old_home/.rustup}" \
  env -u OPENROUTER_API_KEY -u CORTEX_DATA_DIR -u CORTEX_DB_PATH -u CORTEX_AUTH_FILE \
    -u XDG_DATA_HOME -u XDG_CONFIG_HOME \
  cargo test --workspace -- --test-threads=1
```

Leaving `OPENROUTER_API_KEY` exported runs the live-gated tests against OpenRouter
as well; the offline path above is what you want for a clean local CI-style pass.

## License & attribution

Cortex Code is distributed under Apache-2.0 (see `LICENSE`). The project is
clean-room original except for one owner-authorized source port of the TUI
framework crates, which retains its upstream MIT license. Each ported file
carries a per-file provenance header, and the full provenance ledger is recorded
in `NOTICE`.
