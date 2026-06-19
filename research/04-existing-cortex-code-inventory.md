# Existing `/droid/cortex-code` Reference Inventory

> READ-ONLY reference study of the existing Rust workspace at `/droid/cortex-code`
> ("Cortex Code", a local-first terminal coding agent built to mirror opencode).
> Goal: learn what is genuinely REAL vs MOCK / PARTIAL / ORPHAN / STUB / ABSENT so
> the from-scratch rebuild can make everything real. Nothing in `/droid/cortex-code`
> was modified.
>
> Method: read the authoritative docs (`README.md`, `docs/rapport-parite-opencode.md`,
> `docs/parity-matrix.md`, `docs/tui-destub-ledger.md`, port-authorization decision)
> and cross-check against the actual source (crate LOC, reverse-dependency graph,
> feature flags, key files). Verdicts are grounded in code, not self-declared docs.

---

## 0. TL;DR

The workspace is large and **mostly real**: ~340k LOC across 41 crates. The parity
docs claim near-complete structural parity with opencode, and code inspection
largely confirms it for CLI, TUI, providers, tools, MCP, LSP, sessions/permissions,
checkpoints, and the local server. The honest caveats:

- The **default transport is offline replay/cassette**; live provider/web calls are
  real code but opt-in (`CORTEX_TRANSPORT=live`, credential-gated, `CORTEX_NO_NET=1`
  kill-switch).
- **Plugins** run via an *external* detected runtime (Bun/Node/Deno delegation), not
  an embedded JS engine.
- Several crates are **orphans / stubs not wired into the `cortex` binary**:
  `cortex-runtime` + `cortex-runtime-types` (explicit "module skeleton" rebuild),
  `cortex-replay` (6.5k LOC, zero reverse deps; the `cortex replay` CLI command is a
  display-only stub), `cortex-audit` (no consumers), `cortex-browser` (excluded by
  design), `cortex-qa-render` (QA/test tooling only).
- A few provider auth modes are deliberately reduced (Azure AD, Vertex JWT, Bedrock
  SSO all absent — API-key/env only), and provider OAuth login, hosted share, and
  Windows are excluded by design.

---

## 1. Crate map (`/droid/cortex-code/crates/`)

41 member crates (root `Cargo.toml`, `resolver = "3"`, edition 2024). LOC measured
from `src/*.rs`.

| Crate | LOC | One-line responsibility |
|---|---:|---|
| cortex-protocol | 5.5k | Wire/DTO types and protocol vocabulary shared across crates |
| cortex-events | 0.6k | Event type definitions / event bus primitives |
| cortex-redaction | 1.2k | Secret scrubbing / canonical redaction placeholders (security) |
| cortex-config | 4.1k | Layered TOML config (defaults<user<project<env<CLI), validation, diagnostics |
| cortex-errors | 0.3k | Shared error/severity types |
| cortex-persistence | 4.7k | SQLite session/message/todo/usage repositories + migrations |
| cortex-core | 18.1k | Agent engine: session pipeline, prompts, compaction, LSP-after-edit, turn loop |
| cortex-runtime | 7.9k | **STUB**: "agent-runtime-rebuild" skeleton; mostly empty modules (orphan) |
| cortex-runtime-types | 0.2k | Leaf DTOs for the runtime rebuild (only consumed by cortex-runtime) |
| cortex-permissions | 2.3k | allow/ask/deny engine, bash command patterns, most-specific resolution |
| cortex-tools | 22.3k | Agent tools: read/write/ls/edit/multiedit/patch/bash/grep/glob/todo/webfetch/websearch/research/github/format |
| cortex-providers | 16.5k | Provider adapters + transport (live reqwest + cassette replay), SigV4, catalog |
| cortex-mcp | 5.9k | MCP client: stdio + HTTP/SSE, handshake, tools/list, tools/call, OAuth, invoke |
| cortex-plugins | 12.9k | Plugin system: manifest/lockfile/registry/hooks + external TS runtime delegation |
| cortex-replay | 6.6k | **ORPHAN**: replay/cassette tooling crate; no reverse deps into the binary |
| cortex-cli | 21.8k | The `cortex` binary: arg parsing, all subcommands, headless run, wiring |
| cortex-test-support | 1.4k | Offline/no-net test harness helpers (dev) |
| cortex-sandbox | 1.5k | Sandbox policy surface + `cortex sandbox` command (no kernel isolation claim) |
| cortex-checkpoints | 1.7k | Shadow-git snapshot store powering /undo /redo (never project .git) |
| cortex-hooks | 1.7k | Lifecycle hook definitions/dispatch |
| cortex-agents | 3.3k | Agent abstraction, subagents, task tool, batch, routing, custom markdown bridge |
| cortex-review | 0.4k | `cortex review` surface (thin) |
| cortex-policy | 3.2k | Trust/policy engine (trust levels, decisions) |
| cortex-audit | 0.5k | **ORPHAN**: audit category/event types; no consumers wire it |
| cortex-memory | 3.2k | Project/session memory (AGENTS.md/CORTEX.md surfaces) |
| cortex-acp | 4.6k | Agent Client Protocol server (initialize/session.new/prompt/cancel) |
| cortex-browser | 1.4k | **ORPHAN/EXCLUDED**: computer-use browser surface, not wired into binary |
| cortex-concurrency | 0.8k | Resource lock manager / concurrency primitives (used by core) |
| cortex-search | 3.6k | Code/text search engine backing `cortex search` |
| cortex-tui-text | 3.4k | TUI text layout/measurement (ported framework) |
| cortex-tui-syntax | 2.0k | Tree-sitter syntax highlighting for code blocks |
| cortex-tui-core | 4.1k | Ported TUI framework core (color/palette/primitives) |
| cortex-tui-components | 8.5k | Ported widget library (forms, modals, lists, selectors, etc.) |
| cortex-tui-buffer | 2.3k | TUI cell/buffer model |
| cortex-tui-widgets | 10.2k | Higher-level widgets (input, etc.) |
| cortex-frame | 16.4k | **Ported** frame engine (markdown/diff/animation/chat widgets) — renamed from upstream src/cortex-core |
| cortex-tui-capture | 7.2k | Deterministic TUI capture for golden snapshot tests |
| cortex-tui-app | 89.3k | The full TUI application: event loop, views, slash commands, keybinds, bridge |
| cortex-lsp | 5.5k | LSP client: diagnostics, navigation, hierarchy, assisted edit + `cortex lsp status` |
| cortex-server | 8.6k | `cortex serve` axum loopback server: session CRUD, SSE, TUI remote, OpenAPI, local share |
| cortex-qa-render | 0.3k | **DEV-ONLY**: deterministic PNG renderer for QA goldens (not in product graph) |

Reverse-dependency check (`grep` of `Cargo.toml` dependency edges) confirms the
orphans: `cortex-audit`, `cortex-replay`, `cortex-runtime`, `cortex-browser`,
`cortex-qa-render` have **no** in-workspace consumers other than (for
`cortex-runtime`) the `cortex-runtime-types` leaf it owns.

---

## 2. Subsystem status (REAL / PARTIAL / MOCK / ORPHAN / STUB / ABSENT)

| Subsystem | Status | Evidence (file / doc) | Work to make it real in the rebuild |
|---|---|---|---|
| **Providers — adapters** (openai, anthropic, gemini, openai-compat: mistral/groq/xai/deepseek, ollama, lmstudio, custom) | REAL | `crates/cortex-providers/src/{openai,anthropic,gemini,openai_compat,ollama,lmstudio,custom_providers}.rs`; parity PM-063..073 | Keep adapters; default the transport to live when a credential exists. |
| **Providers — HTTP transport** | REAL but OFFLINE-DEFAULT | `cortex-providers/src/transport*.rs`; reqwest rustls; default = cassette replay, live opt-in via `CORTEX_TRANSPORT=live`; `transport_switch.rs:276` notes some live base URLs "not wired … (needs region/project)" | Make live the default path (still credential-gated); finish region/project base-URL wiring for vertex/bedrock. |
| **Providers — Azure / Vertex / Bedrock** | PARTIAL (auth reduced) | `azure.rs` (api-key only, no Entra), `vertex.rs` (external token only, no service-account JWT), `bedrock.rs` (env creds + SigV4 only, no SSO/profiles); parity PM-074/075/076 `adapted` | Add Azure AD/Entra, Vertex service-account JWT signing, Bedrock profile/SSO if real parity wanted. |
| **Model catalog / models.dev** | REAL (embedded) | Embedded snapshot ≥80 models; `cortex models --refresh` is only network path; PM-062 | Keep embedded snapshot; optionally refresh from models.dev online. |
| **Auth / credential store** | REAL | `cortex auth login/list/logout`, env > 0600 file store > config indirection; PM-003/004/005/077 | Keep. Provider OAuth login is ABSENT by design (PM-157). |
| **Headless run (`cortex run`)** | REAL | `cortex-cli/src/main.rs`; `--model/--agent/--continue/--session/--share`, stdin, `--jsonl`; PM-002 (offline + live e2e evidence) | Keep design. |
| **TUI rendering (markdown/syntax/diff/code blocks)** | REAL (ported) | `cortex-frame` (markdown/table/code_block/renderer), `cortex-tui-syntax` (tree-sitter); PM-101 | Keep; framework is a sanctioned direct port (see §6). |
| **TUI @-mentions / fuzzy picker** | REAL | PM-105 (`tests/golden/tui` T78); `cortex-tui-app` views | Keep. |
| **TUI slash commands + autocomplete** | REAL | PM-021..038 (16 commands + custom markdown); `cortex-tui-app` slash registry | Keep. `/connect`,`/details`,`/thinking` absent (minor). |
| **TUI compat de-stub** | REAL (fully de-stubbed) | `docs/tui-destub-ledger.md`: `compat.rs` deleted (0 lines), real persistence/LSP/MCP probes wired | Confirms TUI is bound to real backends, not placeholders. |
| **MCP call execution (stdio)** | REAL | `cortex-mcp/src/invoke.rs` (~700 LOC), `client.rs`, `lifecycle.rs`; spawn/handshake/tools.call/restart; `fake.rs` (46 LOC) is test-only; PM-053 | Keep. |
| **MCP HTTP/SSE** | PARTIAL (static-header auth) | `cortex-mcp/src/http_lifecycle.rs`, `oauth.rs` (MCP-remote OAuth, not provider OAuth); PM-054 `adapted` | Header auth real; broaden if full MCP OAuth flows desired. |
| **Plugins — code execution** | PARTIAL (external runtime delegation) | `cortex-plugins/src/runtime/{detection,invoke}.rs`, `host.rs`, `hooks.rs`; delegates to detected Bun/Node/Deno; no embedded JS engine; rapport-parite §3.3 `adapted` | For "fully real & self-contained", embed a JS runtime (e.g. deno_core/boa) or formally adopt the external-runtime contract. |
| **Plugins — manifest/registry/lockfile/hooks** | REAL | `manifest_install.rs`, `registry.rs`, `lockfile.rs`, `hooks.rs`; PM-135/136 | Keep. |
| **LSP (diagnostics + navigation + assisted edit)** | REAL | `cortex-lsp/src/{edit,*}.rs`, `cortex lsp status`, diagnostics appended to edit results; PM-055 | Keep (exceeds opencode on CLI surface). |
| **Formatters** | REAL | `cortex-tools/src/format/{mod,runner}.rs`; permission-gated + checkpoint before run | Keep. |
| **Sub-agents / task tool** | REAL | `cortex-agents/src/{task_tool,subagent,batch,routing,custom_bridge}.rs`; PM-051/080 | Keep (synchronous in-process). |
| **Web tool — webfetch** | REAL (live default-on) | `cortex-tools/src/web/fetch.rs` + `fetch_http.rs` (reqwest rustls); compiled by default via cli `live` → `cortex-tools/web-live`; `CORTEX_NO_NET` guard; PM-050 | Keep; honors offline kill-switch. |
| **Web tool — websearch / research** | PARTIAL (default-off policy, live deferred) | `cortex-tools/src/web/{search,research}.rs`; real parse logic, production reqwest behind `web-live`; `WebResearchPolicy::default` = disabled; research.rs comments "live web adapter intentionally DEFERRED" | Wire a real search provider/endpoint and flip from default-off policy; finish live search transport. |
| **GitHub tool** | PARTIAL | `cortex-tools/src/github/{ops,http,mod}.rs`; injectable seam, reqwest behind `web-live`; hosted GitHub App excluded (PM-020/153) | Local git/GitHub via bash is the real path; hosted App is ABSENT by design. |
| **Session share** | PARTIAL (local-only by design) | `cortex-server/src/routes/shares.rs`: "no upload code path exists", loopback `127.0.0.1:{port}` only; PM-033/133, excluded PM-154 | Local static export is real; hosted share is intentionally ABSENT. |
| **Server / API / SSE / OpenAPI** | REAL | `cortex-server/src/{state,routes/*,openapi.rs (52k),ws.rs}`; loopback bind, `--public`+`--token`; PM-130/131/134/137 | Keep. |
| **TUI remote control** | REAL | `cortex-server/src/control.rs`, `routes/tui.rs`; append-prompt/submit/clear/execute; PM-132 | Keep. |
| **SDK (TypeScript)** | PARTIAL (minimal, excluded full) | `sdk/` is intentionally NOT a workspace member/dep; PM-158 excluded | Minimal generated SDK only; full SDK out of scope. |
| **ACP (Agent Client Protocol)** | REAL (subset) | `cortex-acp` (4.6k), wired in cli; initialize/session.new/prompt/cancel; PM-017 | Keep; subset of public spec. |
| **Themes / keybinds** | REAL | 6 builtin + adaptive `system` + custom JSON; keybind registry + leader key; PM-120..125; `docs/themes.md` | Keep. |
| **Permissions / trust / policy** | REAL | `cortex-permissions` (allow/ask/deny), `cortex-policy` (trust); PM-092/093 | Keep — a strength. |
| **Checkpoints (undo/redo)** | REAL | `cortex-checkpoints` shadow-git, consumed by core/lsp/tools; PM-094 | Keep — a strength. |
| **Persistence (sessions/usage/todos)** | REAL | `cortex-persistence` SQLite repos + migrations; PM-008/049/090/091 | Keep. |
| **Memory (AGENTS.md/CORTEX.md)** | REAL | `cortex-memory` wired into core+cli; PM-144 | Keep. |
| **Search (`cortex search`)** | REAL | `cortex-search` wired into core+cli | Keep. |
| **Redaction / offline guarantees** | REAL | `cortex-redaction`; `CORTEX_NO_NET=1` kill-switch; offline conformance suite; `docs/offline.md` | Keep — a strength. |
| **Replay (`cortex replay` CLI)** | STUB (command) + ORPHAN (crate) | `cortex-cli/src/main.rs:248` `run_replay()` emits a display-only record telling the user to open the TUI; the 6.6k-LOC `cortex-replay` crate has **zero** reverse deps | Either wire `cortex-replay` into the binary or rebuild a real headless replay path. |
| **Runtime rebuild crate** | STUB / ORPHAN | `cortex-runtime/src/lib.rs`: "Mostly module skeleton … intentionally empty stubs"; only `sources` (clock/RNG/id seam) implemented; no reverse deps | Don't ship as-is; the real turn loop lives in `cortex-core::session_pipeline`. Decide one runtime for the rebuild. |
| **Audit logging** | ORPHAN | `cortex-audit/src/lib.rs` defines categories/events but no consumer wires it | Build a real audit sink wired into permissions/shell/network/MCP/plugin events. |
| **Browser / computer-use** | ORPHAN / EXCLUDED | `cortex-browser` (1.4k) not depended on by anything; PM-150 excluded | Out of scope unless rebuild wants it; would need real CDP/automation. |
| **OS sandbox (landlock/seccomp)** | ABSENT (by design) | `cortex-sandbox` is policy surface only; PM-151 excluded; README "No OS-level sandbox claim" | Add real kernel isolation only if the rebuild commits to testing it. |
| **Provider OAuth login / hosted share / GitHub App / telemetry / Windows** | ABSENT (by design) | PM-153/154/155/156/157; README "Scope boundaries" | Intentional exclusions; revisit per rebuild scope. |
| **QA render (PNG goldens)** | DEV-ONLY | `cortex-qa-render` used by tests, not product | Keep as test tooling. |

---

## 3. Consolidated gaps to close for "full real"

Ordered by how much they matter for a make-everything-real rebuild:

1. **Live transport as a first-class default (not just opt-in replay).** All provider
   adapters and webfetch are real, but the shipped default is offline cassette/replay.
   The rebuild should make live the default when a credential is present, keeping the
   `CORTEX_NO_NET` kill-switch and redaction. Also finish the vertex/bedrock live
   base-URL wiring flagged at `transport_switch.rs:276`.
2. **Web search/research → real.** `WebResearchPolicy::default` is disabled and the
   live search transport is "intentionally DEFERRED" behind `web-live`. Wire a real
   search endpoint/provider and enable by config rather than off-by-default-with-no-backend.
3. **Plugin execution → genuinely self-contained.** Today it delegates to an external
   detected JS runtime (Bun/Node/Deno). For "real without external deps", embed a JS
   engine; otherwise formalize the external-runtime contract and detect/fail clearly.
4. **Replay path → real and wired.** `cortex replay` is a display-only stub and the
   6.6k-LOC `cortex-replay` crate is orphaned. Decide: wire it in or rebuild a real
   headless replay/inspect.
5. **Audit logging → wire it.** `cortex-audit` types exist but nothing emits to them.
   Hook permission/shell/file/network/MCP/plugin/hook events into a real audit sink.
6. **Resolve the dual runtime.** `cortex-runtime`/`cortex-runtime-types` is an empty
   skeleton ("agent-runtime-rebuild") competing with the real `cortex-core` pipeline.
   The rebuild should pick ONE runtime and delete the other to avoid orphan drift.
7. **Provider auth breadth (optional).** Azure AD/Entra, Vertex service-account JWT,
   Bedrock SSO/profiles, and provider OAuth logins are absent/reduced. Add if true
   parity is required; otherwise keep documented as scope boundaries.
8. **MCP remote OAuth (optional).** HTTP/SSE MCP uses static-header auth; full MCP
   OAuth flows would close PM-054's `adapted` gap.
9. **Hosted share / GitHub App / telemetry / Windows / OS sandbox / browser** are all
   intentionally excluded; only revisit if the rebuild's scope explicitly wants them
   (and OS sandbox only if you will actually test the isolation claim).

---

## 4. What this project does well (keep in the new design)

- **Clean crate boundaries** with leaf-safety invariants (e.g. `cortex-runtime` and
  `cortex-runtime-types` deliberately declare no `cortex-*` edges to avoid the
  `core → plugins` cycle). Cargo `[bans] multiple-versions = "deny"` keeps the lock
  tree tight, and `deny.toml` gates licenses.
- **Security/redaction first**: a dedicated `cortex-redaction` crate with shared
  canonical placeholders, cassettes scrubbed of secrets, child env stripped of
  secrets for bash, `0600` credential store, and a hard `CORTEX_NO_NET=1` kill-switch
  enforced *before* any socket opens (`fetch_http.rs::guard_outbound`).
- **Offline-first, fail-closed**: canonical diagnostic enum for no-credential /
  network-disabled / transport-unavailable / cassette-missing, with an offline
  conformance suite that classifies every parity-matrix row under an empty net
  namespace (PM-141/142/143).
- **Permissions/trust engine** with allow/ask/deny, bash command patterns, and
  most-specific-rule resolution — stronger than opencode per the parity report.
- **Checkpoints via shadow git** (never the project `.git`) powering /undo /redo at
  message granularity with conflict detection.
- **SQLite persistence** with explicit migrations for sessions/messages/todos/usage.
- **Deterministic test discipline**: golden TUI snapshots, cassette replay, a
  determinism seam (clock/RNG/id injection), and an n-gram clean-room similarity scan.
- **Single binary, three surfaces** (TUI / `run` / `serve`) with strict stdout/stderr
  hygiene and tty/pipe/NO_COLOR detection.
- **Feature-gated network**: `--no-default-features` compiles zero network/server code;
  network is additive, not load-bearing.

---

## 5. Toolchain & build

- **`rust-toolchain.toml`**: `channel = "stable"`, components `rustfmt`,`clippy`,
  profile `minimal`. Root `Cargo.toml`: `edition = "2024"`, `rust-version = "1.85"`
  (MSRV 1.85), `resolver = "3"`, `publish = false`.
- **Workspace lints**: `unsafe_code = "forbid"`, clippy `all = "warn"`.
- **Key root deps**: clap 4.6 (+clap_complete/clap_mangen), crossterm 0.28, ratatui
  0.29, tui-input/tui-textarea, rusqlite 0.39 (bundled), tree-sitter 0.26
  (+tree-sitter-bash), tokio 1, reqwest 0.12 (rustls-only, no openssl/native-tls),
  rustls 0.23, axum 0.8, sha2 0.10 + hmac 0.12 (SigV4), pulldown-cmark 0.13, serde/
  serde_json, regex 1.12, png+font8x8 (QA render), notify 8 (config watch). Clipboard
  is OSC52 by decision (no `arboard` dep).
- **Build**: `cargo build --release -p cortex-cli` → `target/release/cortex`. Default
  features `new_tui`, `live`, `server`. `cargo build --no-default-features -p cortex-cli`
  = minimal, no network/server. (`target` is a symlink to `/data/cargo-targets/...`.)
- **`justfile`** delegates to `./xtask/dev.sh`: `fmt`, `check`, `test`, `clippy`,
  `boundaries`; `just verify` runs all five. `xtask/` holds the dev scripts; the
  crate-boundary guard enforces the leaf-safety invariants above.
- **Tests**: deterministic offline suites (`tests/offline/conformance_suite.sh` with
  A/B/C contracts, `tests/offline/tui_suite.sh`, golden snapshots under
  `tests/golden/tui/`) + optional live suites (`tests/live/*`) that SKIP cleanly
  without a credential. Evidence committed under `.omo/evidence/`.

---

## 6. Port authorization & clean-room constraints

- The product is **clean-room by default**: original product text, commands, UI labels
  and workflows; competitor products inform high-level capability awareness only. No
  copying of competitor prompts, source, UI copy, hidden flags, visual identity, or
  distinctive workflows (`docs/clean-room-policy.md`, README "Clean room").
- **One owner-authorized source-port exception** (`docs/decisions/port-authorization-cortex-code.md`,
  status Accepted, decision `direct-port-allowed`):
  - Source repo `CortexLM/cortex-code`, pinned commit
    `7954d0218faf6f26aa84df0f6d8b09ea9f2e5ac9`.
  - Authorized roots ONLY: `src/cortex-core`, `src/cortex-tui`,
    `src/cortex-tui-components`, `src/cortex-tui-capture`, `cortex-tui-framework`.
  - Mode: **direct port** (copy then adapt — crate renames, dep-version alignment, API
    fixes) preserving upstream structure/behavior. The upstream `src/cortex-core`
    frame engine is ported under the crate name **`cortex-frame`** to avoid colliding
    with the agent `cortex-core` (see `docs/decisions/crate-name-map.md`).
  - License: product stays Apache-2.0; ported code keeps upstream license, tracked
    root-by-root in `NOTICE`. Public license metadata alone does NOT authorize a port —
    only the owner agreement + pinned boundary do.
  - **Provenance header required on every ported file** (Source-Repo, Source-Commit,
    Source-Path, Target-Path mandatory; Upstream-License + `See:` for NOTICE/audit).
    Visible in `crates/cortex-frame/src/*` and `crates/cortex-tui-components/src/*`.
  - Anything outside that repo/commit/roots stays under normal clean-room rules; any
    expansion needs a new owner decision.
- For the rebuild: the same TUI framework port is reusable under the same authorization
  and provenance/NOTICE discipline; everything else must remain clean-room original.

---

## Appendix: evidence trail

- Docs read: `README.md`, `docs/rapport-parite-opencode.md` (2026-06-15, code-verified
  gap analysis), `docs/parity-matrix.md` (frozen contract, PM-001..158),
  `docs/tui-destub-ledger.md`, `docs/decisions/port-authorization-cortex-code.md`,
  `Cargo.toml`, `rust-toolchain.toml`, `justfile`, `crates/cortex-cli/Cargo.toml`.
- Code inspected: `cortex-providers/src/transport_switch.rs:276`,
  `cortex-tools/src/web/{research.rs,fetch_http.rs}`, `cortex-tools/Cargo.toml` (web-live),
  `cortex-mcp/src/fake.rs` (test-only), `cortex-server/src/routes/shares.rs`
  (no-upload/loopback), `cortex-runtime/src/lib.rs` (skeleton),
  `cortex-audit/src/lib.rs`, `cortex-cli/src/main.rs:248` (`run_replay` stub).
- Reverse-dependency scan over all `crates/*/Cargo.toml` confirmed orphans:
  `cortex-audit`, `cortex-replay`, `cortex-runtime`, `cortex-browser`,
  `cortex-qa-render`.
