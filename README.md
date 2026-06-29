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
  layer, an embedded model catalogue, and `cortex models --refresh` to update it.
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
- **Plugins** — project plugins and lifecycle hooks.
- **Persistence & inspection** — SQLite-backed sessions plus `stats`, `search`,
  `export`/`import`, and `replay`.

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

`.env.local` is gitignored; never commit your key.

## Run

Launch the interactive TUI:

```bash
./target/debug/cortex
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

Default features include network and server support. For a minimal no-network
build:

```bash
cargo build -p cli --no-default-features
./target/debug/cortex --version
```

## Test and validate

```bash
cargo build -p cli
cargo check --workspace
cargo clippy --workspace --no-deps
cargo test --workspace
```

Live OpenRouter tests skip automatically when `OPENROUTER_API_KEY` is not set.

## License & attribution

Cortex Code is distributed under Apache-2.0 (see `LICENSE`). The project is
clean-room original except for one owner-authorized source port of the TUI
framework crates, which retains its upstream MIT license. Each ported file
carries a per-file provenance header, and the full provenance ledger is recorded
in `NOTICE`.
