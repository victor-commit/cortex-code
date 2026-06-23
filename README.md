# Cortex Code

Cortex Code is a Rust terminal coding agent with three surfaces:

- **TUI**: `cortex` (interactive terminal UI)
- **Headless CLI**: `cortex run` (scriptable one-shot turns)
- **Local server**: `cortex serve` (loopback HTTP/SSE API)

## Prerequisites

- Rust toolchain (edition 2024 compatible)
- An OpenRouter API key for live LLM turns

## Build

```bash
cargo build -p cli
./target/debug/cortex --version
```

## Authenticate for live runs

You can provide your OpenRouter key in any of these ways:

1. Environment variable:

```bash
export OPENROUTER_API_KEY="your-key"
```

2. Local env file (`.env.local`) and source it in your shell:

```bash
set -a; . /projects/cortex-code/.env.local; set +a
```

3. Store it in the local 0600 auth store:

```bash
./target/debug/cortex auth login openrouter --api-key "$OPENROUTER_API_KEY"
```

## Run

Live text output:

```bash
set -a; . /projects/cortex-code/.env.local; set +a
./target/debug/cortex run "Say hello from Cortex Code in one short sentence."
```

Machine-readable JSON output:

```bash
./target/debug/cortex run --format json "Reply with a short greeting." | jq .
```

## Other surfaces

Launch the interactive TUI:

```bash
./target/debug/cortex
```

Start the local server:

```bash
./target/debug/cortex serve --hostname 127.0.0.1 --port 3200
```

## Test

```bash
cargo test --workspace
```

This command runs offline suites end-to-end. Live-only tests skip cleanly when `OPENROUTER_API_KEY` is not set.

## Network kill-switch

Set `CORTEX_NO_NET=1` to fail closed before any outbound network call:

```bash
CORTEX_NO_NET=1 ./target/debug/cortex run "Say hello"
```

## Minimal no-network build

Default features include network and server support. To build a minimal no-network binary, disable default features:

```bash
cargo build -p cli --no-default-features
./target/debug/cortex --version
```
