# Cortex Code

Cortex Code is a Rust terminal coding agent packaged as the `cortex` binary. It provides:

- an interactive terminal UI (`cortex`)
- a headless, scriptable CLI (`cortex run`)
- a local loopback HTTP/SSE server (`cortex serve`)
- local config, auth, models, search, session, MCP/LSP, and tool-permission surfaces

Live model turns use OpenRouter. Sessions and local state are persisted with SQLite so runs can be resumed, replayed, exported, and inspected.

## Prerequisites

- Rust 1.85+ (edition 2024 compatible)
- An OpenRouter API key for live LLM runs

## Setup

```bash
cargo build -p cli
./target/debug/cortex --version
```

## Authenticate

For live runs, either export an OpenRouter key for the current shell:

```bash
export OPENROUTER_API_KEY="your-key"
```

or store it in the local 0600 auth store:

```bash
./target/debug/cortex auth login openrouter --api-key "$OPENROUTER_API_KEY"
./target/debug/cortex auth list
```

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

./target/debug/cortex mcp list
./target/debug/cortex lsp status
```

Headless runs can gate built-in, plugin, and MCP tools with permission rules:

```bash
./target/debug/cortex run --permission-deny 'bash:rm *' "Inspect the project safely."
```

## Persistence

Sessions, messages, usage data, and saved local decisions are stored in SQLite. The default database is:

```text
${XDG_DATA_HOME:-$HOME/.local/share}/cortex/cortex.db
```

Set `CORTEX_DB_PATH` to use a specific database file, or `CORTEX_DATA_DIR` to move the Cortex data directory.

## Network controls

Set `CORTEX_NO_NET=1` to fail closed before any outbound network call:

```bash
CORTEX_NO_NET=1 ./target/debug/cortex run "Say hello"
CORTEX_NO_NET=1 ./target/debug/cortex net check
```

Default features include network and server support. For a minimal no-network build:

```bash
cargo build -p cli --no-default-features
./target/debug/cortex --version
```

## Test and validate

```bash
cargo test --workspace
cargo check --workspace
cargo clippy --workspace --no-deps
cargo build -p cli
```

Live OpenRouter tests skip automatically when `OPENROUTER_API_KEY` is not set.
