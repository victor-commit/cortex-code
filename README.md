# Cortex

**Cortex** is an open-source terminal coding agent (`cortex`, Apache-2.0). It
runs real model turns, edits and executes code through a gated tool layer,
persists every session, and ships a **Cortex**-look interactive TUI plus a
**parallel missions** engine with Mission Control.

Live model turns go through [OpenRouter](https://openrouter.ai) by default
(multi-protocol provider layer: OpenAI Chat, Anthropic Messages, Gemini, and
OpenAI-compatible endpoints). Sessions, messages, usage, and saved decisions
are stored locally in SQLite. Offline/cassette paths are **test-only** — the
product default is live when credentials are present.

## Surfaces

One binary exposes several surfaces over the same session engine:

- **TUI** (`cortex`) — interactive Cortex shell: streamed markdown +
  tree-sitter highlighting, tool cards, permission docks, slash commands,
  `@file` mentions, model/session pickers, help/status, and **Mission Control**
  (MC V2 premium home for parallel workers).
- **Headless CLI** (`cortex run`) — scriptable one-shot/continue turns
  (`--format json`, `--jsonl`).
- **Local server** (`cortex serve`) — axum loopback HTTP + SSE `/event`,
  session CRUD, OpenAPI, remote TUI control. Binds **127.0.0.1:18765** by
  default (reserved range **18765–18864**).
- **Missions** (`cortex mission`) — clean-room Factory-similar missionDir
  orchestration with a **parallel** implementer pool (default 3 / max 4).
- **ACP** (`cortex acp`) — Agent Client Protocol stdio subset for editors.
- **TypeScript SDK** (`sdk/`) — typed client for the local server.

## Product identity (Cortex TUI)

The interactive session shell follows **Cortex** chrome, not a copper
Cortex brand plate:

| Element | Behavior |
| --- | --- |
| Top bar | Muted cwd left · context budget `N / M` right |
| User turn | Bare `❯` wash (no `You` nameplate) |
| Assistant | Thought / markdown hierarchy (no `◆ Cortex` speaker plate) |
| Jewel | Right-aligned model chip, e.g. `Cortex (high) · always-approve` |
| Footer | Putty packing with `|` separators; busy turns show `[stop]` |
| Palette | **CortexNight** primary theme |
| Missions home | **MC V2** single page: multi-worker streams, feature rail, progress |

Themes remain switchable (`/themes`); the product default conversation DNA is
Cortex. Mission Control is copper-free.

## Features

- **Providers & models** — OpenRouter-first multi-protocol layer; dynamic
  models.dev catalogue in `/models` (auto-refresh; offline cache + embedded
  snapshot) plus first-class profiles. `cortex models --refresh` from the CLI.
  See [docs/providers-and-oauth.md](docs/providers-and-oauth.md).
- **Auth / connect** — `cortex auth login`, TUI `/connect`: API key (masked,
  never echoed) or OAuth where supported (ChatGPT subscription, xAI xAI,
  Copilot); credentials in `0600` `auth.json`. OAuth overrides: `CORTEX_OAUTH_*`.
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
- **Plugins** — local JS modules with ordered request-shaping hooks (see
  [Plugins](#plugins)).
- **Docs** — [docs/](docs/README.md) (providers, subscription OAuth, env matrix).
- **Parallel missions** — missionDir store, dep-aware scheduler, handoffs,
  milestone validation gates, MC home binding.

## Quick start

### Prerequisites

- Rust **1.85+** (edition 2024)
- Optional: [tmux](https://github.com/tmux/tmux) **3.4+** for e2e harness
- OpenRouter API key for live turns

### Build

```bash
cargo build -p cli --bins
./target/debug/cortex --version
```

### Live API keys

**Never commit keys.** Prefer environment or the local auth store.

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

For gated live integration tests and probes:

```bash
export CORTEX_LIVE=1
# OPENROUTER_API_KEY must already be set (or loaded from auth store)
```

Workers and tests never print key material; redaction is enforced on log sinks.

### Interactive TUI

```bash
./target/debug/cortex
# reduced motion:
CORTEX_NO_ANIM=1 ./target/debug/cortex
./target/debug/cortex tui --no-anim
```

### Headless live turn

```bash
./target/debug/cortex run "Say hello from Cortex in one short sentence."
./target/debug/cortex run --format json "Reply with a short greeting." | jq .
./target/debug/cortex run --jsonl "Stream a short greeting."
```

### Local server (loopback only)

```bash
./target/debug/cortex serve --hostname 127.0.0.1 --port 18765
curl -sf http://127.0.0.1:18765/health
```

Defaults are **127.0.0.1** and **18765**. Ports outside **18765–18864** are
rejected. Do not bind non-loopback hosts for product serve.

### Parallel missions

Missions live under `~/.cortex/missions/<id>/` (override with `--root` or
`CORTEX_MISSIONS_ROOT`). They never write under Factory `~/.factory/missions`.

```bash
ROOT=/tmp/cortex-missions-demo
rm -rf "$ROOT" && mkdir -p "$ROOT"

./target/debug/cortex mission create --fixture --id demo --root "$ROOT"
./target/debug/cortex mission run demo --root "$ROOT" --parallel 3
./target/debug/cortex mission status demo --root "$ROOT" --format json
./target/debug/cortex mission list --root "$ROOT"
```

- **Pool:** default parallel **3**, hard max **4**; dependency-aware.
- **Artifacts:** `features.json`, handoffs, `progress_log.jsonl`, validation
  seals under the missionDir.
- **Isolation fixture:** `cortex mission create --isolation` seeds a pack with
  one intentional failure so siblings still complete.
- **TUI:** Mission Control home shows multi-worker slots and the feature rail.

### ACP

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
./target/debug/cortex mission --help
```

Headless permission gates:

```bash
./target/debug/cortex run --permission-deny 'bash:rm *' "Inspect the project safely."
```

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
list. Each hook's return value becomes the next plugin's input for that phase.

Phases applied to each shaped LLM request:

1. `chat.message` — per message, plugins in config order
2. `experimental.chat.messages.transform` (alias `chat.messages`) — whole array
3. `experimental.chat.system.transform` (alias `chat.system`) — system text
4. `chat.params` — provider parameter map
5. `chat.headers` — extra HTTP headers

Tool schemas are shaped separately:

6. `tool.definition` — per tool definition, plugins in config order

Pure mode (`--pure` / `CORTEX_PURE`) disables plugin loading entirely. See
`crates/plugins/src/lib.rs` and `crates/cli/tests/plugin_request_shaping_hooks.rs`.

## Workspace layout

Cargo workspace under `crates/`; `cli` builds the `cortex` binary:

- **Foundation** — `protocol`, `errors`, `events`, `redaction`, `settings`, `db`
- **Model & engine** — `llm`, `providers`, `engine`, `format`, `memory`
- **Tools & policy** — `tools`, `permissions`, `policy`, `checkpoints`, `mcp`,
  `lsp`, `plugins`, `skills`, `audit`, `subagent_runner`
- **Missions** — `missions` (store, parallel scheduler, validators, handoffs)
- **TUI** — `tui-core`, `tui-buffer`, `tui-text`, `tui-widgets`, `tui-syntax`,
  `frame`, `tui-components`, `tui`
- **Surfaces** — `server`, `acp`, `cli`

The `sdk/` directory holds the TypeScript SDK for the local server.

## Persistence

Default database:

```text
${XDG_DATA_HOME:-$HOME/.local/share}/cortex/cortex.db
```

Override with `CORTEX_DB_PATH` or `CORTEX_DATA_DIR`. Auth lives next to the
config/data trees in a mode-`0600` `auth.json`.

## Network controls

```bash
CORTEX_NO_NET=1 ./target/debug/cortex run "Say hello"
CORTEX_NO_NET=1 ./target/debug/cortex net check
```

Minimal offline-only binary:

```bash
cargo build -p cli --no-default-features
./target/debug/cortex --version
```

## Test and validate

Gate commands used by product scrutiny (see mission `services.yaml`):

```bash
# Build binary
CARGO_INCREMENTAL=1 cargo build -p cli --bins

# Full workspace typecheck/build
CARGO_INCREMENTAL=1 cargo build --workspace

# Clippy (warnings denied)
CARGO_INCREMENTAL=1 cargo clippy --workspace --all-targets -- -D warnings

# Workspace tests (isolated XDG so host caches do not leak into model tests)
export XDG_CONFIG_HOME=/tmp/cortex-mission-xdg/config
export XDG_DATA_HOME=/tmp/cortex-mission-xdg/data
mkdir -p "$XDG_CONFIG_HOME" "$XDG_DATA_HOME"
CARGO_INCREMENTAL=1 cargo test --workspace -- --test-threads=8
```

### Tmux e2e

The harness spawns a dedicated tmux session (120×40), runs the real `cortex`
binary, captures the pane, and tears down without orphan sessions:

```bash
CARGO_INCREMENTAL=1 cargo test -p cli --test tmux_e2e -- --nocapture
```

Live OpenRouter half of the harness (and other `#[ignore]`/gated live tests)
requires credentials plus:

```bash
export CORTEX_LIVE=1
export OPENROUTER_API_KEY=…   # never commit
```

Absent keys skip the live path rather than failing offline CI.

### Missions package tests

```bash
cargo test -p missions -- --test-threads=8
```

### Secrets hygiene

- `.env` / `.env.local` are gitignored.
- Fixtures use placeholders only — no real `sk-or-v1-…` keys in tree.
- Redaction crate scrubs secret-shaped strings from logs and audit sinks.

```bash
# Sanity scan (should be empty of real keys)
rg -n 'sk-or-v1-[A-Za-z0-9]{20,}' --glob '!target/**' --glob '!.git/**' .
```

## TypeScript SDK

From `sdk/`:

```bash
npm install
npm run typecheck
npm test
```

Regenerate the client with `npm run generate`.

## License & attribution

Cortex is distributed under **Apache-2.0** (see `LICENSE`). The project is
clean-room original except for one owner-authorized source port of the TUI
framework crates, which retains its upstream MIT license. Each ported file
carries a per-file provenance header; the full ledger is in `NOTICE`.

Do not commit API keys, cloud credentials, or private mission secrets.
