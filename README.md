# Cortex

**Cortex** is an open-source terminal coding agent (`cortex`, Apache-2.0). It
runs real model turns, edits and executes code through a gated tool layer,
persists every session, and ships an interactive TUI plus a parallel missions
engine with Mission Control.

Live model turns go through [OpenRouter](https://openrouter.ai) by default
(multi-protocol provider layer: OpenAI Chat, Anthropic Messages, Gemini, and
OpenAI-compatible endpoints). Sessions, messages, usage, and saved decisions
are stored locally in SQLite. Offline/cassette paths are **test-only** — the
product default is live when credentials are present.

[Building from source](#building-from-source) ·
[Surfaces](#surfaces) ·
[Documentation](#documentation) ·
[Repository layout](#repository-layout) ·
[Development](#development) ·
[Contributing](#contributing) ·
[License](#license)

## Building from source

**Requirements**

- Rust **1.85+** (edition 2024)
- Optional: [tmux](https://github.com/tmux/tmux) **3.4+** for e2e harness
- An OpenRouter API key (or another configured provider) for live turns

```bash
cargo build -p cli --bins
./target/debug/cortex --version
```

Release build:

```bash
cargo build -p cli --bins --release
./target/release/cortex --version
```

### Credentials

**Never commit keys.** Prefer environment variables or the local auth store.

```bash
# Option A — env (gitignored .env.local)
# .env.local
OPENROUTER_API_KEY=your-key

set -a; . ./.env.local; set +a

# Option B — export for the shell
export OPENROUTER_API_KEY="your-key"

# Option C — 0600 auth store
./target/debug/cortex auth login openrouter --api-key "$OPENROUTER_API_KEY"
./target/debug/cortex auth list
```

Inside the TUI, `/connect` stores the same `auth.json` credential and marks the
provider connected in `/models` without a restart.

For gated live integration tests:

```bash
export CORTEX_LIVE=1
# OPENROUTER_API_KEY must already be set (or loaded from auth store)
```

## Surfaces

One binary exposes several surfaces over the same session engine:

| Surface | Command | Role |
| --- | --- | --- |
| **TUI** | `cortex` | Interactive shell: streamed markdown, tool cards, permission docks, slash commands, pickers, Mission Control |
| **Headless CLI** | `cortex run` | Scriptable one-shot/continue turns (`--format json`, `--jsonl`) |
| **Local server** | `cortex serve` | Loopback HTTP + SSE `/event`, session CRUD, OpenAPI (default `127.0.0.1:18765`, range **18765–18864**) |
| **Missions** | `cortex mission` | Parallel missionDir orchestration (default pool 3 / max 4) |
| **ACP** | `cortex acp` | Agent Client Protocol stdio subset for editors |
| **TypeScript SDK** | `sdk/` | Typed client for the local server |

### Quick examples

```bash
# Interactive TUI
./target/debug/cortex
CORTEX_NO_ANIM=1 ./target/debug/cortex   # reduced motion

# Headless turn
./target/debug/cortex run "Say hello from Cortex in one short sentence."
./target/debug/cortex run --format json "Reply with a short greeting." | jq .

# Local server (loopback only)
./target/debug/cortex serve --hostname 127.0.0.1 --port 18765
curl -sf http://127.0.0.1:18765/health

# Missions (isolated root)
ROOT=/tmp/cortex-missions-demo
rm -rf "$ROOT" && mkdir -p "$ROOT"
./target/debug/cortex mission create --fixture --id demo --root "$ROOT"
./target/debug/cortex mission run demo --root "$ROOT" --parallel 3
./target/debug/cortex mission status demo --root "$ROOT" --format json

# ACP
./target/debug/cortex acp
```

Missions live under `~/.cortex/missions/<id>/` (override with `--root` or
`CORTEX_MISSIONS_ROOT`).

## Features

- **Providers & models** — OpenRouter-first multi-protocol layer; dynamic
  models.dev catalogue in `/models` (auto-refresh; offline cache + embedded
  snapshot) plus first-class profiles. See
  [docs/providers-and-oauth.md](docs/providers-and-oauth.md).
- **Auth / connect** — `cortex auth login`, TUI `/connect`: API key (masked,
  never echoed) or OAuth where supported (ChatGPT subscription, xAI, Copilot);
  credentials in mode-`0600` `auth.json`. Overrides: `CORTEX_OAUTH_*`.
- **Tools** — read, write, edit, apply_patch, glob, grep, bash (background),
  todo, webfetch, websearch, `task` subagent, `lsp`, `skill`, `question`.
- **Permissions** — allow/ask/deny over glob and bash patterns; saved decisions.
- **Checkpoints** — shadow-git snapshots; undo/redo.
- **MCP / LSP** — stdio + remote HTTP/SSE MCP (incl. OAuth remotes); LSP
  diagnostics and navigation after edits.
- **Agents, skills, memory** — personas, markdown agents, skill injection,
  layered project memory (`AGENTS.md`, …).
- **Context compaction** — automatic compaction and overflow recovery.
- **Audit & redaction** — secret redaction across logs/config; `CORTEX_NO_NET`
  offline kill switch.
- **Plugins** — local JS modules with ordered request-shaping hooks.
- **Parallel missions** — missionDir store, dep-aware scheduler, handoffs,
  milestone validation gates, Mission Control home binding.

### TUI chrome (defaults)

| Element | Behavior |
| --- | --- |
| Top bar | Muted cwd left · context budget `N / M` right |
| User turn | Bare `❯` prompt |
| Assistant | Thought / markdown hierarchy |
| Jewel | Right-aligned model chip, e.g. `Cortex (high) · always-approve` |
| Footer | Compact packing with `\|` separators; busy turns show `[stop]` |
| Palette | **CortexNight** primary theme (`/themes` to switch) |

## Documentation

| Doc | Contents |
| --- | --- |
| [docs/README.md](docs/README.md) | Docs index |
| [docs/providers-and-oauth.md](docs/providers-and-oauth.md) | Provider catalogue, profiles, subscription OAuth, env matrix |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Build, test, and PR expectations |
| [SECURITY.md](SECURITY.md) | Vulnerability reporting |

## Repository layout

| Path | Contents |
| --- | --- |
| `crates/` | Rust workspace (protocol, engine, providers, TUI, server, CLI, …) |
| `crates/cli` | Composition root; builds the `cortex` binary |
| `sdk/` | TypeScript SDK for the local server |
| `docs/` | Product documentation |
| `tests/` | Shared fixtures and e2e helpers |
| `.cargo/` | Local cargo config (linker flags) |
| `.config/nextest.toml` | cargo-nextest profile for the test suite |

### Crate groups

- **Foundation** — `protocol`, `errors`, `events`, `redaction`, `settings`, `db`
- **Model & engine** — `llm`, `providers`, `engine`, `format`, `memory`
- **Tools & policy** — `tools`, `permissions`, `policy`, `checkpoints`, `mcp`,
  `lsp`, `plugins`, `skills`, `audit`, `subagent_runner`
- **Missions** — `missions`
- **TUI** — `tui-core`, `tui-buffer`, `tui-text`, `tui-widgets`, `tui-syntax`,
  `frame`, `tui-components`, `tui`
- **Surfaces** — `server`, `acp`, `cli`

## Development

TUI design harness (sibling repo, not a workspace member):
[cortex-tui-designer](https://github.com/CortexLM/cortex-tui-designer).

```bash
# Fast validation
cargo check -p cli --bins

# Full workspace build
cargo build --workspace

# Clippy (warnings denied in CI-style gates)
cargo clippy --workspace --all-targets -- -D warnings

# Per-crate tests (prefer targeted crates; full workspace is slower)
cargo test -p providers --lib
cargo test -p missions -- --test-threads=8

# Workspace tests with isolated XDG
export XDG_CONFIG_HOME=/tmp/cortex-xdg/config
export XDG_DATA_HOME=/tmp/cortex-xdg/data
mkdir -p "$XDG_CONFIG_HOME" "$XDG_DATA_HOME"
cargo test --workspace -- --test-threads=8

# Tmux e2e (real binary + pane capture)
cargo test -p cli --test tmux_e2e -- --nocapture
```

### Useful CLI surfaces

Most inspection commands support `--format json`.

```bash
./target/debug/cortex config --format json
./target/debug/cortex models
./target/debug/cortex models --refresh
./target/debug/cortex search "TODO"
./target/debug/cortex session list
./target/debug/cortex mcp list
./target/debug/cortex lsp status
./target/debug/cortex mission --help
```

Headless permission gates:

```bash
./target/debug/cortex run --permission-deny 'bash:rm *' "Inspect the project safely."
```

### Network controls

```bash
CORTEX_NO_NET=1 ./target/debug/cortex run "Say hello"
CORTEX_NO_NET=1 ./target/debug/cortex net check

# Minimal offline-only binary
cargo build -p cli --no-default-features
```

### Persistence

Default database:

```text
${XDG_DATA_HOME:-$HOME/.local/share}/cortex/cortex.db
```

Override with `CORTEX_DB_PATH` or `CORTEX_DATA_DIR`. Auth lives next to the
config/data trees in a mode-`0600` `auth.json`.

### Plugins

Project plugins are local JavaScript modules listed in config (for example
`plugin = ["./my-plugin.mjs"]`). They execute in an external runtime with fixed
discovery order: **Bun**, then **Node**, then **Deno**. A configured plugin is
never silently skipped. Pure mode (`--pure` / `CORTEX_PURE`) disables plugin
loading. See `crates/plugins/src/lib.rs`.

### TypeScript SDK

```bash
cd sdk
npm install
npm run typecheck
npm test
# npm run generate   # regenerate client from OpenAPI
```

### Secrets hygiene

- `.env` / `.env.local` are gitignored.
- Fixtures use placeholders only — no real provider keys in tree.
- The redaction crate scrubs secret-shaped strings from logs and audit sinks.

```bash
# Sanity scan (should be empty of real keys)
rg -n 'sk-or-v1-[A-Za-z0-9]{20,}' --glob '!target/**' --glob '!.git/**' .
```

## Contributing

Contributions are welcome under Apache-2.0. See [CONTRIBUTING.md](CONTRIBUTING.md)
for build/test expectations and PR guidelines. Please report security issues
privately via [SECURITY.md](SECURITY.md).

## License

Cortex is distributed under **Apache-2.0** (see [`LICENSE`](LICENSE)). The
project is clean-room original except for one owner-authorized source port of
the TUI framework crates, which retains its upstream MIT license. Each ported
file carries a per-file provenance header; the full ledger is in
[`NOTICE`](NOTICE).

Do not commit API keys, cloud credentials, or private secrets.
