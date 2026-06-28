use clap::{Args, Parser, Subcommand, ValueEnum};

/// Cortex Code — a terminal coding agent with interactive, headless, and server surfaces.
#[derive(Debug, Parser)]
#[command(
    name = "cortex",
    version,
    about = "Cortex Code — terminal coding agent (TUI, headless run, local server)",
    long_version = env!("CARGO_PKG_VERSION"),
    long_about = "Cortex Code is a single-binary terminal coding agent.\n\n\
        Run with no arguments to launch the interactive TUI. Use `cortex run` for\n\
        scriptable headless turns, or `cortex serve` to expose the local HTTP/SSE server.",
    propagate_version = true
)]
pub struct Cli {
    /// Increase logging verbosity (repeatable: `-v` for debug, `-vv` for trace).
    ///
    /// Routes debug/event logs (including event-bus delivery) to stderr; `RUST_LOG` is also
    /// honored. Never affects stdout, so machine output stays clean.
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// Disable every configured plugin and every plugin-registered capability for this process.
    ///
    /// This is a hard kill switch for plugin code: no plugin files or package specs are resolved,
    /// no runtime is detected, no hooks fire, and plugin tools/auth/providers are not advertised.
    /// The environment equivalent is `CORTEX_PURE=1`.
    #[arg(long, global = true)]
    pub pure: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run a single headless turn (scriptable; reads the prompt from an argument, stdin, or both).
    /// Use global `--pure` to disable configured plugins and plugin tools.
    ///
    /// Input sources are combined deterministically. When both a positional prompt argument and
    /// piped stdin are present they are joined in a stable, documented order (piped stdin first,
    /// then the argument, separated by a blank line), so piped context reaches the model alongside
    /// the instruction. With only one source present that source is the prompt; with neither
    /// (empty stdin and no argument) the run fails fast with a clear no-input diagnostic.
    ///
    /// `CORTEX_PURE=1` is the environment equivalent for the plugin kill switch.
    Run(RunArgs),

    /// Launch the interactive terminal UI (same as running with no arguments).
    Tui(TuiArgs),

    /// Start the local loopback HTTP/SSE server.
    Serve(ServeArgs),

    /// Run the Agent Client Protocol (ACP) stdio server for editor integrations.
    Acp(AcpArgs),

    /// Report the effective configuration after merging every layer.
    ///
    /// Layers are merged lowest-precedence first:
    /// defaults < global < project < nested < env < CLI. Environment overrides use the
    /// `CORTEX_<KEY>` mapping (e.g. `CORTEX_MODEL`); `--model` is the highest-precedence
    /// CLI override. Use `--format json` for a machine-parseable dump of the merged config.
    Config(ConfigArgs),

    /// Create, list, and inspect persisted sessions and their messages.
    ///
    /// Sessions and messages live in a SQLite database created on first use under the data
    /// directory (`${XDG_DATA_HOME:-$HOME/.local/share}/cortex/cortex.db` by default). Set
    /// `CORTEX_DB_PATH` to point at an explicit database file, or `CORTEX_DATA_DIR` to relocate
    /// the data directory; the default location is left untouched when an override is used.
    Session(SessionArgs),

    /// Export one persisted session as a portable JSON artifact.
    Export(ExportArgs),

    /// Import a previously exported JSON artifact into persistence.
    Import(ImportArgs),

    /// Summarize persisted token usage and cost.
    Stats(StatsArgs),

    /// Search the working tree for text matches (ripgrep-backed, local-only).
    Search(SearchArgs),

    /// Reconstruct and print a persisted session transcript.
    Replay(ReplayArgs),

    /// Emit shell-completion scripts for bash, zsh, or fish.
    Completion(CompletionArgs),

    /// Generate a roff man page for `cortex`.
    Man(ManArgs),

    /// List the built-in agent personas selectable with `cortex run --agent`.
    Agent(AgentArgs),

    /// Manage provider credentials in the local 0600 auth store.
    Auth(AuthArgs),

    /// List selectable models, including models registered by plugins.
    Models(ModelsArgs),

    /// Generate a custom markdown agent definition with a real model turn.
    Generate(GenerateArgs),

    /// Install project plugins from package specs.
    #[command(alias = "plug")]
    Plugin(PluginArgs),

    /// Inspect provider health/models and start authentication for a selected provider.
    ///
    /// This is the provider-centric view over the model catalogue and the local credential store:
    /// `cortex provider list` reports each configured provider (enabled state, stored-auth status,
    /// and its models), and `cortex provider auth <id>` runs the same credential flow as
    /// `cortex auth login`. The plain listing is fully offline.
    #[command(alias = "providers")]
    Provider(ProviderArgs),

    /// Inspect and manage the multi-directory project model.
    ///
    /// A project groups one or more directories under one stable id, so sessions, search, and
    /// config can span every directory of a single logical project instead of keying identity to a
    /// single working-directory hash. `cortex project current` reports (creating on first use) the
    /// project for the working directory; `cortex project add <dir>` widens it to a second
    /// directory; `cortex project list` shows every project with its directories. All paths are
    /// resolved locally; no network is contacted.
    Project(ProjectArgs),

    /// Create, list, and remove git worktrees, emitting machine events for each operation.
    ///
    /// Worktree operations run real `git worktree` commands in the working directory's repository:
    /// `cortex worktree create <path>` adds a worktree (optionally on a new branch), `cortex
    /// worktree list` reports the attached worktrees (matching `git worktree list`), and `cortex
    /// worktree remove <path>` detaches one. Each create/remove broadcasts a typed `worktree.ready`
    /// / `worktree.removed` / `worktree.failed` event on the bus (observable with `-v`). All paths
    /// are resolved locally; no network is contacted.
    Worktree(WorktreeArgs),

    /// Control-plane account/console login (intentionally unavailable in this build).
    ///
    /// Cortex Code is a local terminal mono-binary with no hosted control plane, so there is no
    /// account or console to sign in to. These subcommands exist so the surface is explicit (no
    /// unknown-command confusion): `login`/`logout` report a clear out-of-scope diagnostic and
    /// exit non-zero, while `status` prints the unsupported status. Provider API keys are managed
    /// with `cortex auth` / `cortex provider auth` instead.
    #[command(alias = "console")]
    Account(AccountArgs),

    /// Manage a project-scoped environment store that feeds the commands the agent runs.
    ///
    /// Values live in a per-project store OUTSIDE the project tree (owner-only on Unix), never in
    /// the agent's own process environment. `set`/`get`/`remove` manage individual values and `all`
    /// lists them with secret-looking values redacted. A stored value is merged into the
    /// environment of commands the agent runs (the `bash` tool and the server `shell` route), so
    /// setting a key makes later runs see it and removing it makes them no longer see it.
    #[command(alias = "environment")]
    Env(EnvArgs),

    /// Outbound-network diagnostics that honor the `CORTEX_NO_NET` kill-switch.
    ///
    /// These are the network-touching command paths: with `CORTEX_NO_NET=1` set they fail closed
    /// before any socket is opened, emitting a `net-disabled` diagnostic on stderr with a non-zero
    /// exit. With the switch unset they proceed to open a real outbound connection.
    Net(NetArgs),

    /// Undo the most recent agent turn's file changes (shadow-git checkpoint restore).
    ///
    /// Reverts every file the last mutating turn changed as one unit, restoring each to its exact
    /// pre-turn bytes (and removing files the turn created). The shadow checkpoint store lives
    /// outside the project tree and never touches the project's own `.git`. A file changed outside
    /// the agent since the snapshot is detected as a conflict and left untouched rather than
    /// clobbered. Repeat to walk further back through history.
    Undo(CheckpointArgs),

    /// Re-apply the most recently undone turn's file changes (shadow-git checkpoint restore).
    ///
    /// Restores the post-turn state an `undo` reverted. A fresh edit made after an undo advances
    /// history along a new branch, so the abandoned change is no longer redoable.
    Redo(CheckpointArgs),

    /// Inspect and maintain the shadow-git snapshot store at finer granularity than whole-turn
    /// undo/redo.
    ///
    /// `cortex snapshot diff` reports the net file changes the agent's snapshots captured (a summary
    /// by default, or the full unified diff with `--full`). `cortex snapshot revert --path <p>`
    /// rolls back a single file (a "patch"), or one hunk of it with `--hunk <n>`, WITHOUT reverting
    /// the other files changed in the same turn; the revert is itself recorded as an undoable
    /// snapshot. `cortex snapshot prune` drops old snapshots and garbage-collects the store while
    /// keeping the most recent undo turns and the redo branch byte-exact. All paths are resolved
    /// locally; no network is contacted.
    Snapshot(SnapshotArgs),

    /// Manage and inspect Model Context Protocol (MCP) servers.
    ///
    /// MCP servers are declared in the layered config under `mcp.<name>` (a local stdio server has
    /// `type = "local"` and a `command` array; a remote server has `type = "remote"` and a `url`).
    /// `cortex mcp list` connects to each enabled server, completes the MCP handshake, and reports
    /// its status (connected/disabled/failed/needs_auth), live PID, and advertised tools under
    /// their server-qualified ids.
    Mcp(McpArgs),

    /// Inspect Language Server Protocol (LSP) server availability.
    Lsp(LspArgs),

    /// Inspect formatter availability and config state.
    #[command(alias = "format")]
    Formatter(FormatterArgs),

    /// Attach to a running or most-recent session and stream its messages until you detach.
    ///
    /// Attach is a read-only observer: it prints the session's existing messages and then follows
    /// new ones as they are persisted by a concurrent `cortex run` (or a `cortex serve` turn) on
    /// the same database. Detaching — Ctrl-C, the end of `--timeout`, or `--once` — never aborts
    /// the underlying turn, which keeps running in its own process. With `--server <URL>` the
    /// existing transcript is fetched from a loopback `cortex serve` first; the live tail still
    /// reads the shared database so it works whether the turn runs headless or through the server.
    Attach(AttachArgs),

    /// Report this build's upgrade status (self-update is described, never silently performed).
    ///
    /// A from-source mono-binary build is upgraded by its installer (cargo/git/package manager),
    /// so `cortex upgrade` reports the current version and the unsupported-self-update status
    /// rather than mutating the installed binary. It changes no files. Use `--help` for the
    /// documented safeguards.
    Upgrade(UpgradeArgs),

    /// Preview or remove Cortex's own data and configuration artifacts (never unrelated files).
    ///
    /// By default this is a dry run that only LISTS the Cortex-owned paths (the data directory
    /// holding the SQLite database, auth/MCP credential stores, and the `cortex` config
    /// directory). Pass `--yes` to actually delete them. Resolution honors `CORTEX_DB_PATH`,
    /// `CORTEX_DATA_DIR`, `XDG_DATA_HOME`, `XDG_CONFIG_HOME`, and `HOME`, so an install prefix can
    /// be uninstalled in isolation. Paths outside the Cortex-owned directories are never touched.
    Uninstall(UninstallArgs),

    /// Read-only diagnostics: inspect the database, effective config, LSP, audit log, and runtime.
    ///
    /// Every `debug` subcommand is read-only and redaction-safe by default: it never mutates the
    /// database or writes secrets in cleartext. `--format json` emits a single strict JSON
    /// document for scripting.
    Debug(DebugArgs),
}

#[derive(Debug, Args)]
pub struct LspArgs {
    #[command(subcommand)]
    pub command: LspCommand,
}

#[derive(Debug, Args)]
pub struct FormatterArgs {
    #[command(subcommand)]
    pub command: FormatterCommand,
}

#[derive(Debug, Subcommand)]
pub enum FormatterCommand {
    /// List formatter states, including configured overrides and marker gating.
    Status(FormatterStatusArgs),
}

#[derive(Debug, Args)]
pub struct FormatterStatusArgs {
    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct AttachArgs {
    /// Session id to attach to. When omitted, the most recently updated session is used.
    #[arg(value_name = "SESSION_ID")]
    pub session: Option<String>,

    /// Print the session's current messages and exit without following new ones.
    #[arg(long, default_value_t = false)]
    pub once: bool,

    /// Stop following after this many seconds. `0` (the default) follows until you interrupt
    /// with Ctrl-C; detaching never aborts the underlying turn.
    #[arg(long, value_name = "SECS", default_value_t = 0)]
    pub timeout: u64,

    /// Poll interval, in milliseconds, for detecting newly persisted messages while following.
    #[arg(long = "poll-ms", value_name = "MS", default_value_t = 250)]
    pub poll_ms: u64,

    /// Fetch the existing transcript from a loopback `cortex serve` origin before tailing. The
    /// live tail still reads the shared database. Honors the `CORTEX_NO_NET` kill-switch.
    #[arg(long, value_name = "URL")]
    pub server: Option<String>,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct UpgradeArgs {
    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct UninstallArgs {
    /// Actually delete the Cortex-owned artifacts. Without this flag the command is a dry run
    /// that only lists what it would remove and changes nothing.
    #[arg(long, default_value_t = false)]
    pub yes: bool,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct DebugArgs {
    #[command(subcommand)]
    pub command: DebugCommand,
}

#[derive(Debug, Subcommand)]
pub enum DebugCommand {
    /// Inspect the SQLite database read-only: path, schema version, row counts, and an optional
    /// integrity check. Never mutates the database.
    Db(DebugDbArgs),

    /// Dump the effective merged configuration with secrets redacted (same as `cortex config`).
    Config(DebugConfigArgs),

    /// Report configured language-server availability (same as `cortex lsp status`).
    Lsp(DebugLspArgs),

    /// Print recent durable audit-log records with secrets redacted.
    Audit(DebugAuditArgs),

    /// Report runtime status: version, resolved paths, build features, and the network kill-switch.
    Info(DebugInfoArgs),
}

#[derive(Debug, Args)]
pub struct DebugDbArgs {
    /// Also run SQLite's read-only `PRAGMA integrity_check` and include its result.
    #[arg(long, default_value_t = false)]
    pub integrity: bool,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct DebugConfigArgs {
    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct DebugLspArgs {
    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct DebugAuditArgs {
    /// Maximum number of most-recent records to print (0 prints all available records).
    #[arg(long, value_name = "N", default_value_t = 50)]
    pub limit: usize,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct DebugInfoArgs {
    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Subcommand)]
pub enum LspCommand {
    /// Run local diagnostics for one file through the matching language server.
    ///
    /// This is a local, user-facing LSP proof path that does not start a model turn. With
    /// `CORTEX_NO_NET=1`, Cortex never attempts language-server auto-install; a missing server is
    /// reported as `no_server`, while a pre-installed server can still return real diagnostics
    /// offline.
    Diagnostics(LspDiagnosticsArgs),
    /// List configured language servers and whether they are disabled, available, installing,
    /// running, unavailable, or crashed.
    Status(LspStatusArgs),
}

#[derive(Debug, Args)]
pub struct LspDiagnosticsArgs {
    /// File to open in the matching language server and collect diagnostics for.
    #[arg(value_name = "FILE")]
    pub file: String,

    /// Request workspace-level pull diagnostics (where supported) instead of only the target file.
    #[arg(long, default_value_t = false)]
    pub workspace: bool,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct LspStatusArgs {
    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct AgentArgs {
    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct AuthArgs {
    #[command(subcommand)]
    pub command: AuthCommand,
}

#[derive(Debug, Subcommand)]
pub enum AuthCommand {
    /// Store a provider API key non-interactively (flag/env/stdin) in the local 0600 auth store.
    Login(AuthLoginArgs),
    /// List providers that currently have a stored credential.
    List(AuthListArgs),
    /// Remove a stored credential for one provider.
    Logout(AuthLogoutArgs),
}

#[derive(Debug, Args)]
pub struct AuthLoginArgs {
    /// Provider id to store a credential for (for example: `openrouter`).
    #[arg(value_name = "PROVIDER")]
    pub provider: String,

    /// API key value to store.
    ///
    /// When omitted, `auth login` falls back to a provider env var (for `openrouter`:
    /// `OPENROUTER_API_KEY`) and then to piped stdin.
    #[arg(long = "api-key", value_name = "KEY")]
    pub api_key: Option<String>,

    /// Explicit environment variable name to read the API key from.
    ///
    /// Checked only when `--api-key` is not provided.
    #[arg(long = "api-key-env", value_name = "ENV")]
    pub api_key_env: Option<String>,

    /// Explicit auth method id/type when a provider exposes multiple methods.
    ///
    /// Examples: `api`, `oauth`, `device`.
    #[arg(long = "auth-method", value_name = "METHOD")]
    pub auth_method: Option<String>,

    /// Structured auth input values (repeatable), forwarded to plugin/built-in auth methods.
    ///
    /// Example: `--auth-input tenant=example --auth-input region=us`.
    #[arg(long = "auth-input", value_name = "KEY=VALUE")]
    pub auth_inputs: Vec<String>,

    /// OAuth authorization code for `oauth` auth methods that require a manual callback code.
    #[arg(long = "oauth-code", value_name = "CODE")]
    pub oauth_code: Option<String>,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct AuthListArgs {
    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct AuthLogoutArgs {
    /// Provider id to remove from the local auth store.
    #[arg(value_name = "PROVIDER")]
    pub provider: String,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct ModelsArgs {
    /// Refresh the catalog from models.dev over the network before listing. Honors the
    /// `CORTEX_NO_NET` kill-switch (fails closed); without this flag the listing is fully offline.
    #[arg(long)]
    pub refresh: bool,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct GenerateArgs {
    /// Description of the custom agent to author.
    #[arg(value_name = "DESCRIPTION")]
    pub description: String,

    /// Override the generated identifier.
    #[arg(long, value_name = "NAME")]
    pub name: Option<String>,

    /// Directory to write the generated markdown agent into.
    ///
    /// Defaults to `.cortex/agents` under the current project, which is searched automatically by
    /// `cortex agent` and `cortex run --agent`.
    #[arg(long = "agents-dir", value_name = "DIR")]
    pub agents_dir: Option<String>,

    /// Model to use for the generator turn (overrides config and defaults).
    #[arg(long, value_name = "MODEL")]
    pub model: Option<String>,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct PluginArgs {
    #[command(subcommand)]
    pub command: PluginCommand,
}

#[derive(Debug, Subcommand)]
pub enum PluginCommand {
    /// Install one package plugin and record it in project config.
    Install(PluginInstallArgs),
    /// List the plugins configured in project `cortex.toml`, with their lockfile pin status.
    List(PluginListArgs),
    /// Re-resolve an installed package plugin and refresh its lockfile pin.
    Update(PluginUpdateArgs),
    /// Remove a plugin from project config and drop its lockfile entry.
    #[command(visible_alias = "uninstall")]
    Remove(PluginRemoveArgs),
}

#[derive(Debug, Args)]
pub struct PluginListArgs {
    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct PluginUpdateArgs {
    /// Package spec to re-resolve (must already be installed in project config).
    #[arg(value_name = "SPEC")]
    pub spec: String,

    /// Optional plugin surface type (`tui` loads only in the TUI surface).
    #[arg(long = "type", value_enum, value_name = "TYPE")]
    pub plugin_type: Option<PluginSurfaceType>,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct PluginRemoveArgs {
    /// Plugin spec to remove from project config (and drop from the lockfile).
    #[arg(value_name = "SPEC")]
    pub spec: String,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct PluginInstallArgs {
    /// Package spec to install (for example `@scope/package` or `file:/path/to/plugin`).
    #[arg(value_name = "SPEC")]
    pub spec: String,

    /// Optional plugin surface type (`tui` loads only in the TUI surface).
    #[arg(long = "type", value_enum, value_name = "TYPE")]
    pub plugin_type: Option<PluginSurfaceType>,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum PluginSurfaceType {
    Run,
    Tui,
}

#[derive(Debug, Args)]
pub struct ProviderArgs {
    #[command(subcommand)]
    pub command: ProviderCommand,
}

#[derive(Debug, Subcommand)]
pub enum ProviderCommand {
    /// List providers with their health (enabled, stored-auth) and models.
    List(ProviderListArgs),
    /// Start authentication for one provider (same flow as `cortex auth login`).
    Auth(AuthLoginArgs),
}

#[derive(Debug, Args)]
pub struct ProviderListArgs {
    /// Filter the listing to a single provider id (case-insensitive exact match).
    #[arg(long = "id", value_name = "PROVIDER")]
    pub id: Option<String>,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct AccountArgs {
    #[command(subcommand)]
    pub command: Option<AccountCommand>,
}

#[derive(Debug, Subcommand)]
pub enum AccountCommand {
    /// Attempt a control-plane account/console login (reports out-of-scope and exits non-zero).
    Login(AccountActionArgs),
    /// Attempt a control-plane account/console logout (reports out-of-scope and exits non-zero).
    Logout(AccountActionArgs),
    /// Report the account/console control-plane support status (unsupported).
    Status(AccountActionArgs),
}

#[derive(Debug, Args)]
pub struct AccountActionArgs {
    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct EnvArgs {
    #[command(subcommand)]
    pub command: Option<EnvCommand>,
}

#[derive(Debug, Subcommand)]
pub enum EnvCommand {
    /// Print one scoped environment value by key.
    Get(EnvGetArgs),

    /// List every scoped environment value (secret-looking values are redacted).
    #[command(alias = "list")]
    All(EnvListArgs),

    /// Set (or overwrite) one scoped environment value.
    Set(EnvSetArgs),

    /// Remove one scoped environment value.
    #[command(alias = "unset")]
    Remove(EnvRemoveArgs),
}

#[derive(Debug, Args)]
pub struct EnvGetArgs {
    /// The variable name to read.
    #[arg(value_name = "KEY")]
    pub key: String,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct EnvListArgs {
    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct EnvSetArgs {
    /// The variable name to set.
    #[arg(value_name = "KEY")]
    pub key: String,

    /// The value to store. When omitted, the value is read from stdin.
    #[arg(value_name = "VALUE")]
    pub value: Option<String>,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct EnvRemoveArgs {
    /// The variable name to remove.
    #[arg(value_name = "KEY")]
    pub key: String,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct McpArgs {
    #[command(subcommand)]
    pub command: McpCommand,
}

#[derive(Debug, Subcommand)]
pub enum McpCommand {
    /// List configured MCP servers with their connection status, PID, and tools.
    List(McpListArgs),
    /// Register a new MCP server in the global config file.
    ///
    /// The definition is written under `[mcp.<name>]` and is picked up by `cortex mcp list` and
    /// `cortex run` on the next invocation. Re-adding the same definition is idempotent; adding a
    /// different one under an existing name errors (unless `--force`) without corrupting the config.
    Add(McpAddArgs),
    /// Authorize an OAuth-protected remote MCP server and persist its token in the 0600 store.
    Auth(McpAuthArgs),
}

#[derive(Debug, Args)]
pub struct McpListArgs {
    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct McpAddArgs {
    /// The name to register the server under (its `mcp.<name>` config key and tool-id prefix).
    #[arg(value_name = "NAME")]
    pub name: String,

    /// The command to launch a local stdio server: the program followed by its arguments.
    ///
    /// Put Cortex-side options such as `--cwd`, `--env`, `--disabled`, and `--force` before this
    /// flag. The command vector is trailing and preserves hyphen-prefixed server arguments, e.g.
    /// `--command npx -y @scope/server --stdio`.
    #[arg(
        long = "command",
        num_args = 1..,
        allow_hyphen_values = true,
        conflicts_with = "url",
        value_name = "CMD"
    )]
    pub command: Vec<String>,

    /// The URL of a remote HTTP/SSE MCP server.
    #[arg(long = "url", conflicts_with = "command", value_name = "URL")]
    pub url: Option<String>,

    /// An HTTP header to send to a remote server, as `KEY=VALUE` (repeatable).
    #[arg(long = "header", value_name = "KEY=VALUE")]
    pub headers: Vec<String>,

    /// The working directory the server child is spawned in.
    #[arg(long = "cwd", value_name = "DIR")]
    pub cwd: Option<String>,

    /// An environment variable to set on the server child, as `KEY=VALUE` (repeatable).
    #[arg(long = "env", value_name = "KEY=VALUE")]
    pub env: Vec<String>,

    /// Per-operation timeout in seconds.
    #[arg(long = "timeout", value_name = "SECS")]
    pub timeout_secs: Option<u64>,

    /// Register the server but leave it disabled (it is recorded but never spawned).
    #[arg(long = "disabled")]
    pub disabled: bool,

    /// Overwrite an existing server of the same name instead of erroring on a conflict.
    #[arg(long = "force")]
    pub force: bool,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct McpAuthArgs {
    /// The configured remote MCP server to authorize.
    #[arg(value_name = "NAME")]
    pub name: String,

    /// Pin the loopback OAuth callback port. Must be in 3200-3299.
    #[arg(long = "callback-port", value_name = "PORT")]
    pub callback_port: Option<u16>,

    /// Timeout in seconds while waiting for the loopback callback.
    #[arg(long = "timeout", default_value_t = 120, value_name = "SECS")]
    pub timeout_secs: u64,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct CheckpointArgs {
    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct SnapshotArgs {
    #[command(subcommand)]
    pub command: SnapshotCommand,
}

#[derive(Debug, Subcommand)]
pub enum SnapshotCommand {
    /// Show the net file changes captured by the agent's snapshots.
    ///
    /// Defaults to a per-file summary (status plus added/removed line counts); pass `--full` for the
    /// complete `git diff`-style unified diff of every changed file.
    Diff(SnapshotDiffArgs),

    /// Revert a single file (a patch), or one hunk of it, without reverting the rest of the turn.
    ///
    /// Restores the named file to its pre-change bytes and records the revert as a fresh, undoable
    /// snapshot. A file changed outside the agent since the snapshot is reported as a conflict and
    /// left untouched rather than clobbered.
    Revert(SnapshotRevertArgs),

    /// Prune old snapshots and garbage-collect the store, keeping recent undo/redo intact.
    Prune(SnapshotPruneArgs),
}

#[derive(Debug, Args)]
pub struct SnapshotDiffArgs {
    /// Restrict the diff to a single session id (default: every applied snapshot).
    #[arg(long, value_name = "ID")]
    pub session: Option<String>,

    /// Include the full unified diff text for each file (default: a summary with line counts).
    #[arg(long)]
    pub full: bool,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct SnapshotRevertArgs {
    /// The workspace-relative path of the file to revert.
    #[arg(long, value_name = "PATH")]
    pub path: String,

    /// Restrict the revert to a single session id (default: every applied snapshot).
    #[arg(long, value_name = "ID")]
    pub session: Option<String>,

    /// Revert only the given 0-based hunk of the file instead of the whole file.
    #[arg(long, value_name = "N")]
    pub hunk: Option<usize>,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct SnapshotPruneArgs {
    /// Keep this many of the most recent undo turns; older applied snapshots are pruned.
    #[arg(long, default_value_t = 10, value_name = "N")]
    pub keep: usize,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct NetArgs {
    #[command(subcommand)]
    pub command: NetCommand,
}

#[derive(Debug, Subcommand)]
pub enum NetCommand {
    /// Probe outbound connectivity to a host:port (honors the `CORTEX_NO_NET` kill-switch).
    ///
    /// The kill-switch is consulted before any address resolution or socket is created, so under
    /// `CORTEX_NO_NET=1` this refuses with a `net-disabled` diagnostic and makes zero connection
    /// attempts. Otherwise it opens a real TCP connection and reports the outcome.
    Check(NetCheckArgs),
}

#[derive(Debug, Args)]
pub struct NetCheckArgs {
    /// Host to probe (defaults to the reference provider's API host, `openrouter.ai`).
    #[arg(long, value_name = "HOST")]
    pub host: Option<String>,

    /// Port to probe (defaults to 443).
    #[arg(long, value_name = "PORT")]
    pub port: Option<u16>,

    /// Connection timeout in seconds.
    #[arg(long = "timeout", default_value_t = 5, value_name = "SECS")]
    pub timeout_secs: u64,

    #[command(flatten)]
    pub format: FormatArgs,
}

/// Output format shared by the `session` subcommands.
#[derive(Debug, Args)]
pub struct FormatArgs {
    /// Output format: text (default) or json (a single strict JSON document).
    #[arg(long, value_enum, default_value_t = OutputFormat::Text, value_name = "FORMAT")]
    pub format: OutputFormat,
}

#[derive(Debug, Args)]
pub struct SessionArgs {
    #[command(subcommand)]
    pub command: SessionCommand,
}

#[derive(Debug, Subcommand)]
pub enum SessionCommand {
    /// Create a new session and print its id.
    New(SessionNewArgs),

    /// List existing sessions, newest first.
    List(SessionListArgs),

    /// Show a session and the messages stored under it.
    Show(SessionShowArgs),

    /// Delete a session and its persisted messages.
    Delete(SessionDeleteArgs),

    /// Append a message to a session (content from an argument or stdin).
    Message(SessionMessageArgs),
}

#[derive(Debug, Args)]
pub struct ProjectArgs {
    #[command(subcommand)]
    pub command: Option<ProjectCommand>,
}

#[derive(Debug, Subcommand)]
pub enum ProjectCommand {
    /// Show (creating on first use) the project for the working directory and its directories.
    Current(ProjectActionArgs),

    /// List every known project with its directories.
    List(ProjectActionArgs),

    /// Add a directory to the working directory's project (widening it to multiple directories).
    Add(ProjectDirectoryArgs),

    /// Remove a (non-primary) directory from the working directory's project.
    Remove(ProjectDirectoryArgs),

    /// List the directories registered to the working directory's project.
    Directories(ProjectActionArgs),

    /// List the sessions that belong to the working directory's project.
    Sessions(ProjectActionArgs),
}

#[derive(Debug, Args)]
pub struct ProjectActionArgs {
    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct ProjectDirectoryArgs {
    /// The directory to add to (or remove from) the project. Resolved relative to the working
    /// directory; it must exist.
    #[arg(value_name = "DIR")]
    pub directory: String,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct WorktreeArgs {
    #[command(subcommand)]
    pub command: Option<WorktreeCommand>,
}

#[derive(Debug, Subcommand)]
pub enum WorktreeCommand {
    /// List the git worktrees attached to the working directory's repository.
    List(WorktreeListArgs),

    /// Create a new git worktree (optionally on a new branch), emitting a worktree event.
    Create(WorktreeCreateArgs),

    /// Remove a git worktree, emitting a worktree event.
    #[command(alias = "rm")]
    Remove(WorktreeRemoveArgs),
}

#[derive(Debug, Args)]
pub struct WorktreeListArgs {
    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct WorktreeCreateArgs {
    /// Path for the new worktree checkout (resolved relative to the working directory).
    #[arg(value_name = "PATH")]
    pub path: String,

    /// Check out (and, with `--new-branch`, create) this branch in the new worktree.
    #[arg(long, value_name = "BRANCH")]
    pub branch: Option<String>,

    /// Create `--branch` as a new branch instead of checking out an existing one.
    #[arg(long, default_value_t = false)]
    pub new_branch: bool,

    /// The commit-ish to base the worktree on (defaults to HEAD).
    #[arg(long = "ref", value_name = "REF")]
    pub reference: Option<String>,

    /// Force creation even when the path is non-empty or the branch is already checked out.
    #[arg(long, default_value_t = false)]
    pub force: bool,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct WorktreeRemoveArgs {
    /// Path of the worktree to remove (resolved relative to the working directory).
    #[arg(value_name = "PATH")]
    pub path: String,

    /// Force removal even when the worktree has uncommitted changes.
    #[arg(long, default_value_t = false)]
    pub force: bool,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct SessionNewArgs {
    /// Optional human-facing title for the session.
    #[arg(long, value_name = "TITLE")]
    pub title: Option<String>,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct SessionListArgs {
    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct SessionShowArgs {
    /// The id of the session to show.
    #[arg(value_name = "ID")]
    pub id: String,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct SessionDeleteArgs {
    /// The id of the session to delete.
    #[arg(value_name = "ID")]
    pub id: String,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct SessionMessageArgs {
    /// The id of the session to append to.
    #[arg(value_name = "ID")]
    pub id: String,

    /// The message content. Takes precedence over stdin; when omitted, stdin is read.
    #[arg(value_name = "CONTENT")]
    pub content: Option<String>,

    /// The role to record for the message.
    #[arg(long, default_value = "user", value_name = "ROLE")]
    pub role: String,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct ExportArgs {
    /// The session id to export.
    #[arg(value_name = "SESSION_ID")]
    pub session_id: String,
}

#[derive(Debug, Args)]
pub struct ImportArgs {
    /// A JSON artifact to import. Accepts a local file path, `-` for stdin, or an `http(s)://`
    /// URL to a shared session (for example a `cortex serve` share link). A URL is fetched over
    /// the network behind the `CORTEX_NO_NET` kill-switch and never has its query string logged.
    #[arg(value_name = "ARTIFACT")]
    pub artifact: String,
}

#[derive(Debug, Args)]
pub struct StatsArgs {
    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct SearchArgs {
    /// Query to search for (ripgrep regex syntax).
    #[arg(value_name = "QUERY")]
    pub query: String,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct ReplayArgs {
    /// The id of the session to replay.
    #[arg(value_name = "SESSION_ID")]
    pub id: String,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct CompletionArgs {
    /// Shell to generate completions for.
    #[arg(value_enum, value_name = "SHELL")]
    pub shell: CompletionShell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum CompletionShell {
    Bash,
    Zsh,
    Fish,
}

#[derive(Debug, Args)]
pub struct ManArgs {}

/// Machine/human output format for headless runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum OutputFormat {
    /// Human-readable text (default).
    #[default]
    Text,
    /// A single strict JSON document on stdout.
    Json,
    /// Newline-delimited JSON events, one object per line.
    Jsonl,
}

#[derive(Debug, Args)]
pub struct RunArgs {
    /// The instruction prompt. When piped stdin is also present, the two are combined in a stable
    /// order (piped stdin first, then this argument, joined by a blank line), so piped context is
    /// sent with the instruction. When omitted, the prompt is read from stdin alone.
    #[arg(value_name = "PROMPT")]
    pub prompt: Option<String>,

    /// Output format: text (default), json, or jsonl.
    #[arg(long, value_enum, default_value_t = OutputFormat::Text, value_name = "FORMAT")]
    pub format: OutputFormat,

    /// Shorthand for `--format jsonl` (newline-delimited JSON events).
    #[arg(long, conflicts_with = "format")]
    pub jsonl: bool,

    /// Validate the final assistant answer against this JSON schema and expose parsed output.
    ///
    /// Supported with `--format json`; on success the parsed value is emitted as
    /// `structured_output`, and on failure the run exits with a typed `schema-validation`
    /// diagnostic.
    #[arg(long, value_name = "SCHEMA")]
    pub schema: Option<String>,

    /// Model to use for this turn (overrides config and defaults).
    #[arg(long, value_name = "MODEL")]
    pub model: Option<String>,

    /// Agent persona to use for this turn.
    #[arg(long, value_name = "NAME")]
    pub agent: Option<String>,

    /// Mode overlay to compose with the selected agent for this turn.
    #[arg(long, value_name = "NAME")]
    pub mode: Option<String>,

    /// Resume an explicit session by id.
    #[arg(long, value_name = "ID")]
    pub session: Option<String>,

    /// Resume the most recent session.
    #[arg(long, short = 'c', conflicts_with = "session")]
    pub r#continue: bool,

    /// Execute a local command path instead of starting an LLM turn.
    ///
    /// This is the headless command surface used by plugin `command.execute.before` hooks. Built-in
    /// proof commands include `touch <path>`, `write-marker <path> [content...]`, and `echo [text...]`.
    #[arg(long, value_name = "NAME")]
    pub command: Option<String>,

    /// Argument passed to `--command` (repeatable, preserves order).
    #[arg(long = "command-arg", value_name = "ARG", requires = "command")]
    pub command_args: Vec<String>,

    /// Allow a gated tool action, in `TOOL:GLOB` form (repeatable).
    ///
    /// `TOOL` selects the gated tool
    /// (`read`/`write`/`edit`/`apply_patch`/`glob`/`grep`/`bash`/`task`) or `*` for any tool;
    /// `GLOB` is matched against the action's subject (a file path for file tools, the parsed
    /// command for `bash`, the subagent name for `task`, or the JSON argument object for dynamic
    /// plugin/MCP tools). Example: `--permission-allow '*:**'` allows everything,
    /// `--permission-allow 'write:src/**'` allows writes under `src/`, and
    /// `--permission-allow 'task:explore'` allows spawning the explore subagent. Allow rules are
    /// applied before deny rules, so an explicit `--permission-deny` always overrides a broader
    /// allow. With no permission flags the run is ungated (every tool action proceeds).
    #[arg(long = "permission-allow", value_name = "TOOL:GLOB")]
    pub permission_allow: Vec<String>,

    /// Deny a gated tool action, in `TOOL:GLOB` form (repeatable).
    ///
    /// A denied action produces no side effect: a denied `write`/`edit`/`apply_patch` leaves the
    /// target file byte-for-byte unchanged, a denied `read`/`glob`/`grep` returns no protected
    /// content, and a denied `bash` command does not run at all. A compound `bash` command with any
    /// denied segment is refused atomically before anything runs. Example:
    /// `--permission-deny 'bash:rm *'`, `--permission-deny 'write:secrets/**'`.
    #[arg(long = "permission-deny", value_name = "TOOL:GLOB")]
    pub permission_deny: Vec<String>,

    /// Allow AND remember a gated tool action, in `TOOL:GLOB` form (repeatable).
    ///
    /// The headless analog of the TUI's "approve and remember": a matching action is surfaced as an
    /// ask (recording a `permission.asked` event), auto-approved, and the "always allow" choice is
    /// saved to the per-project store under the data dir. A LATER, fully separate `cortex run` on
    /// the same project then reuses the saved decision WITHOUT re-asking. Example:
    /// `--permission-allow-save 'bash:echo *'`.
    #[arg(long = "permission-allow-save", value_name = "TOOL:GLOB")]
    pub permission_allow_save: Vec<String>,
}

#[derive(Debug, Args)]
pub struct ConfigArgs {
    /// Output format: text (default, a TOML dump) or json (a single strict JSON document).
    #[arg(long, value_enum, default_value_t = OutputFormat::Text, value_name = "FORMAT")]
    pub format: OutputFormat,

    /// Override the effective model (highest precedence; beats env and every config file).
    #[arg(long, value_name = "MODEL")]
    pub model: Option<String>,
}

#[derive(Debug, Args)]
pub struct TuiArgs {
    /// Attach the TUI to an existing persisted session at startup.
    #[arg(long, value_name = "ID")]
    pub session: Option<String>,

    /// Attach to a loopback `cortex serve` origin for remote control integration.
    #[arg(long, value_name = "URL")]
    pub server: Option<String>,
}

#[derive(Debug, Args)]
pub struct ServeArgs {
    /// Address to bind the loopback server to.
    #[arg(long, default_value = "127.0.0.1", value_name = "HOST")]
    pub hostname: String,

    /// Port to bind the loopback server to.
    #[arg(long, default_value_t = 3200, value_name = "PORT")]
    pub port: u16,

    /// Enable browser-facing loopback CORS for token-gated protected routes.
    ///
    /// This does not relax the loopback-only bind invariant. Use it with `--token` when a local
    /// browser client on localhost needs to call the server with an Authorization header.
    #[arg(long, requires = "token")]
    pub public: bool,

    /// Require `Authorization: Bearer <TOKEN>` on protected server routes.
    ///
    /// The token is never printed, logged, or embedded in the OpenAPI document. `/health`,
    /// `/openapi.json`, and clean `OPTIONS` preflights remain available without the credential.
    #[arg(long, value_name = "TOKEN")]
    pub token: Option<String>,
}

#[derive(Debug, Args)]
pub struct AcpArgs {}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use clap::error::ErrorKind;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn no_args_yields_no_subcommand() {
        let cli = Cli::try_parse_from(["cortex"]).expect("parse");
        assert!(cli.command.is_none());
    }

    #[test]
    fn version_flag_is_recognized() {
        let err = Cli::try_parse_from(["cortex", "--version"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayVersion);
    }

    #[test]
    fn short_version_flag_is_recognized() {
        let err = Cli::try_parse_from(["cortex", "-V"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayVersion);
    }

    #[test]
    fn help_flag_is_recognized() {
        let err = Cli::try_parse_from(["cortex", "--help"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayHelp);
    }

    #[test]
    fn short_help_flag_is_recognized() {
        let err = Cli::try_parse_from(["cortex", "-h"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayHelp);
    }

    #[test]
    fn help_lists_core_subcommands() {
        let mut cmd = Cli::command();
        let help = cmd.render_help().to_string();
        for sub in [
            "run",
            "tui",
            "serve",
            "acp",
            "session",
            "export",
            "import",
            "stats",
            "agent",
            "auth",
            "models",
            "plugin",
            "generate",
            "search",
            "replay",
            "completion",
            "man",
            "lsp",
            "formatter",
        ] {
            assert!(help.contains(sub), "help should list `{sub}`:\n{help}");
        }
    }

    #[test]
    fn agent_command_parses_format() {
        let cli = Cli::try_parse_from(["cortex", "agent", "--format", "json"]).expect("parse");
        match cli.command {
            Some(Command::Agent(args)) => assert_eq!(args.format.format, OutputFormat::Json),
            other => panic!("expected agent, got {other:?}"),
        }
    }

    #[test]
    fn auth_list_parses_format() {
        let cli =
            Cli::try_parse_from(["cortex", "auth", "list", "--format", "json"]).expect("parse");
        match cli.command {
            Some(Command::Auth(AuthArgs {
                command: AuthCommand::List(args),
            })) => {
                assert_eq!(args.format.format, OutputFormat::Json);
            }
            other => panic!("expected auth list, got {other:?}"),
        }
    }

    #[test]
    fn auth_login_parses_provider_key_and_env() {
        let cli = Cli::try_parse_from([
            "cortex",
            "auth",
            "login",
            "openrouter",
            "--api-key",
            "abc",
            "--api-key-env",
            "OPENROUTER_API_KEY",
            "--auth-method",
            "api",
            "--auth-input",
            "region=us",
            "--oauth-code",
            "code-123",
            "--format",
            "json",
        ])
        .expect("parse");
        match cli.command {
            Some(Command::Auth(AuthArgs {
                command: AuthCommand::Login(args),
            })) => {
                assert_eq!(args.provider, "openrouter");
                assert_eq!(args.api_key.as_deref(), Some("abc"));
                assert_eq!(args.api_key_env.as_deref(), Some("OPENROUTER_API_KEY"));
                assert_eq!(args.auth_method.as_deref(), Some("api"));
                assert_eq!(args.auth_inputs, vec!["region=us".to_owned()]);
                assert_eq!(args.oauth_code.as_deref(), Some("code-123"));
                assert_eq!(args.format.format, OutputFormat::Json);
            }
            other => panic!("expected auth login, got {other:?}"),
        }
    }

    #[test]
    fn auth_logout_parses_provider_and_format() {
        let cli =
            Cli::try_parse_from(["cortex", "auth", "logout", "openrouter", "--format", "json"])
                .expect("parse");
        match cli.command {
            Some(Command::Auth(AuthArgs {
                command: AuthCommand::Logout(args),
            })) => {
                assert_eq!(args.provider, "openrouter");
                assert_eq!(args.format.format, OutputFormat::Json);
            }
            other => panic!("expected auth logout, got {other:?}"),
        }
    }

    #[test]
    fn models_parses_format() {
        let cli = Cli::try_parse_from(["cortex", "models", "--format", "json"]).expect("parse");
        match cli.command {
            Some(Command::Models(args)) => assert_eq!(args.format.format, OutputFormat::Json),
            other => panic!("expected models, got {other:?}"),
        }
    }

    #[test]
    fn plugin_install_parses_spec_type_and_format() {
        let cli = Cli::try_parse_from([
            "cortex",
            "plugin",
            "install",
            "@scope/demo",
            "--type",
            "tui",
            "--format",
            "json",
        ])
        .expect("parse");
        match cli.command {
            Some(Command::Plugin(PluginArgs {
                command: PluginCommand::Install(args),
            })) => {
                assert_eq!(args.spec, "@scope/demo");
                assert_eq!(args.plugin_type, Some(PluginSurfaceType::Tui));
                assert_eq!(args.format.format, OutputFormat::Json);
            }
            other => panic!("expected plugin install, got {other:?}"),
        }
    }

    #[test]
    fn plug_alias_parses_plugin_install() {
        let cli = Cli::try_parse_from(["cortex", "plug", "install", "demo"]).expect("parse");
        match cli.command {
            Some(Command::Plugin(PluginArgs {
                command: PluginCommand::Install(args),
            })) => assert_eq!(args.spec, "demo"),
            other => panic!("expected plug install alias, got {other:?}"),
        }
    }

    #[test]
    fn lsp_diagnostics_parses_file_and_format() {
        let cli = Cli::try_parse_from([
            "cortex",
            "lsp",
            "diagnostics",
            "src/main.rs",
            "--format",
            "json",
        ])
        .expect("parse");
        match cli.command {
            Some(Command::Lsp(LspArgs {
                command: LspCommand::Diagnostics(args),
            })) => {
                assert_eq!(args.file, "src/main.rs");
                assert!(!args.workspace);
                assert_eq!(args.format.format, OutputFormat::Json);
            }
            other => panic!("expected lsp diagnostics, got {other:?}"),
        }
    }

    #[test]
    fn lsp_workspace_diagnostics_flag_parses() {
        let cli = Cli::try_parse_from([
            "cortex",
            "lsp",
            "diagnostics",
            "src/main.rs",
            "--workspace",
            "--format",
            "json",
        ])
        .expect("parse");
        match cli.command {
            Some(Command::Lsp(LspArgs {
                command: LspCommand::Diagnostics(args),
            })) => {
                assert_eq!(args.file, "src/main.rs");
                assert!(args.workspace);
                assert_eq!(args.format.format, OutputFormat::Json);
            }
            other => panic!("expected lsp diagnostics, got {other:?}"),
        }
    }

    #[test]
    fn formatter_status_parses_format() {
        let cli = Cli::try_parse_from(["cortex", "formatter", "status", "--format", "json"])
            .expect("parse");
        match cli.command {
            Some(Command::Formatter(FormatterArgs {
                command: FormatterCommand::Status(args),
            })) => {
                assert_eq!(args.format.format, OutputFormat::Json);
            }
            other => panic!("expected formatter status, got {other:?}"),
        }
    }

    #[test]
    fn format_alias_parses_formatter_status() {
        let cli = Cli::try_parse_from(["cortex", "format", "status"]).expect("parse");
        match cli.command {
            Some(Command::Formatter(FormatterArgs {
                command: FormatterCommand::Status(args),
            })) => {
                assert_eq!(args.format.format, OutputFormat::Text);
            }
            other => panic!("expected formatter status via alias, got {other:?}"),
        }
    }

    #[test]
    fn lsp_help_documents_no_net_local_diagnostics_surface() {
        let mut cmd = Cli::command();
        let lsp = cmd
            .get_subcommands_mut()
            .find(|sub| sub.get_name() == "lsp")
            .expect("the `lsp` subcommand exists");
        let diagnostics = lsp
            .get_subcommands_mut()
            .find(|sub| sub.get_name() == "diagnostics")
            .expect("the `lsp diagnostics` subcommand exists");
        let help = diagnostics.render_long_help().to_string();
        for phrase in ["CORTEX_NO_NET=1", "no_server", "pre-installed"] {
            assert!(
                help.contains(phrase),
                "`lsp diagnostics --help` should document `{phrase}`:\n{help}"
            );
        }
    }

    #[test]
    fn generate_command_parses_description_and_options() {
        let cli = Cli::try_parse_from([
            "cortex",
            "generate",
            "an agent that writes tests",
            "--name",
            "unit-tester",
            "--agents-dir",
            ".cortex/agents",
            "--model",
            "google/gemini-2.5-flash",
            "--format",
            "json",
        ])
        .expect("parse");
        match cli.command {
            Some(Command::Generate(args)) => {
                assert_eq!(args.description, "an agent that writes tests");
                assert_eq!(args.name.as_deref(), Some("unit-tester"));
                assert_eq!(args.agents_dir.as_deref(), Some(".cortex/agents"));
                assert_eq!(args.model.as_deref(), Some("google/gemini-2.5-flash"));
                assert_eq!(args.format.format, OutputFormat::Json);
            }
            other => panic!("expected generate, got {other:?}"),
        }
    }

    #[test]
    fn run_help_enumerates_its_flags_for_discovery() {
        // VAL-CROSS-035: a capability available as a flag must be discoverable through
        // per-subcommand `--help`, not only reachable as a hidden/undocumented flag. Model
        // switching (`--model`) in particular is the cross-surface discoverability example, so it
        // (and the other principal run flags) must surface in `cortex run --help`.
        let mut cmd = Cli::command();
        let run = cmd
            .get_subcommands_mut()
            .find(|sub| sub.get_name() == "run")
            .expect("the `run` subcommand exists");
        let help = run.render_help().to_string();
        for flag in [
            "--model",
            "--agent",
            "--mode",
            "--session",
            "--format",
            "--schema",
            "--pure",
        ] {
            assert!(
                help.contains(flag),
                "`run --help` should document `{flag}`:\n{help}"
            );
        }
    }

    #[test]
    fn session_new_parses_title_and_format() {
        let cli = Cli::try_parse_from([
            "cortex", "session", "new", "--title", "demo", "--format", "json",
        ])
        .expect("parse");
        match cli.command {
            Some(Command::Session(SessionArgs {
                command: SessionCommand::New(args),
            })) => {
                assert_eq!(args.title.as_deref(), Some("demo"));
                assert_eq!(args.format.format, OutputFormat::Json);
            }
            other => panic!("expected session new, got {other:?}"),
        }
    }

    #[test]
    fn session_message_parses_id_content_and_role() {
        let cli = Cli::try_parse_from([
            "cortex",
            "session",
            "message",
            "ses_1",
            "hello there",
            "--role",
            "assistant",
        ])
        .expect("parse");
        match cli.command {
            Some(Command::Session(SessionArgs {
                command: SessionCommand::Message(args),
            })) => {
                assert_eq!(args.id, "ses_1");
                assert_eq!(args.content.as_deref(), Some("hello there"));
                assert_eq!(args.role, "assistant");
            }
            other => panic!("expected session message, got {other:?}"),
        }
    }

    #[test]
    fn session_delete_parses_id_and_format() {
        let cli = Cli::try_parse_from(["cortex", "session", "delete", "ses_1", "--format", "json"])
            .expect("parse");
        match cli.command {
            Some(Command::Session(SessionArgs {
                command: SessionCommand::Delete(args),
            })) => {
                assert_eq!(args.id, "ses_1");
                assert_eq!(args.format.format, OutputFormat::Json);
            }
            other => panic!("expected session delete, got {other:?}"),
        }
    }

    #[test]
    fn session_requires_a_subcommand() {
        let err = Cli::try_parse_from(["cortex", "session"]).unwrap_err();
        assert_eq!(
            err.kind(),
            ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
        );
    }

    #[test]
    fn export_parses_session_id() {
        let cli = Cli::try_parse_from(["cortex", "export", "ses_123"]).expect("parse");
        match cli.command {
            Some(Command::Export(args)) => assert_eq!(args.session_id, "ses_123"),
            other => panic!("expected export, got {other:?}"),
        }
    }

    #[test]
    fn import_parses_artifact_path() {
        let cli = Cli::try_parse_from(["cortex", "import", "artifact.json"]).expect("parse");
        match cli.command {
            Some(Command::Import(args)) => assert_eq!(args.artifact, "artifact.json"),
            other => panic!("expected import, got {other:?}"),
        }
    }

    #[test]
    fn replay_parses_session_id_and_format() {
        let cli = Cli::try_parse_from(["cortex", "replay", "ses_123", "--format", "json"])
            .expect("parse");
        match cli.command {
            Some(Command::Replay(args)) => {
                assert_eq!(args.id, "ses_123");
                assert_eq!(args.format.format, OutputFormat::Json);
            }
            other => panic!("expected replay, got {other:?}"),
        }
    }

    #[test]
    fn completion_parses_supported_shells() {
        for shell in ["bash", "zsh", "fish"] {
            let cli = Cli::try_parse_from(["cortex", "completion", shell]).expect("parse");
            match cli.command {
                Some(Command::Completion(args)) => match (shell, args.shell) {
                    ("bash", CompletionShell::Bash)
                    | ("zsh", CompletionShell::Zsh)
                    | ("fish", CompletionShell::Fish) => {}
                    _ => panic!("shell `{shell}` mapped to wrong enum: {:?}", args.shell),
                },
                other => panic!("expected completion, got {other:?}"),
            }
        }
    }

    #[test]
    fn completion_rejects_unknown_shell() {
        let err = Cli::try_parse_from(["cortex", "completion", "powershell-typo"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidValue);
    }

    #[test]
    fn man_command_parses() {
        let cli = Cli::try_parse_from(["cortex", "man"]).expect("parse");
        assert!(matches!(cli.command, Some(Command::Man(_))));
    }

    #[test]
    fn search_parses_query_and_format() {
        let cli = Cli::try_parse_from(["cortex", "search", "UNIQUE-NEEDLE", "--format", "json"])
            .expect("parse");
        match cli.command {
            Some(Command::Search(args)) => {
                assert_eq!(args.query, "UNIQUE-NEEDLE");
                assert_eq!(args.format.format, OutputFormat::Json);
            }
            other => panic!("expected search, got {other:?}"),
        }
    }

    #[test]
    fn run_parses_prompt_and_flags() {
        let cli = Cli::try_parse_from([
            "cortex",
            "run",
            "hello world",
            "--model",
            "google/gemini-2.5-flash",
            "--session",
            "abc",
            "--format",
            "json",
        ])
        .expect("parse");
        match cli.command {
            Some(Command::Run(args)) => {
                assert_eq!(args.prompt.as_deref(), Some("hello world"));
                assert_eq!(args.model.as_deref(), Some("google/gemini-2.5-flash"));
                assert_eq!(args.session.as_deref(), Some("abc"));
                assert_eq!(args.format, OutputFormat::Json);
                assert!(!args.jsonl);
                assert_eq!(args.schema, None);
                assert!(!args.r#continue);
            }
            other => panic!("expected run, got {other:?}"),
        }
    }

    #[test]
    fn pure_is_a_global_flag() {
        let cli = Cli::try_parse_from(["cortex", "run", "--pure", "hi"]).expect("parse");
        assert!(cli.pure);
        assert!(matches!(cli.command, Some(Command::Run(_))));
    }

    #[test]
    fn run_parses_repeatable_permission_flags() {
        let cli = Cli::try_parse_from([
            "cortex",
            "run",
            "hi",
            "--permission-allow",
            "*:**",
            "--permission-deny",
            "bash:rm *",
            "--permission-deny",
            "write:secrets/**",
        ])
        .expect("parse");
        match cli.command {
            Some(Command::Run(args)) => {
                assert_eq!(args.permission_allow, vec!["*:**".to_owned()]);
                assert_eq!(
                    args.permission_deny,
                    vec!["bash:rm *".to_owned(), "write:secrets/**".to_owned()]
                );
            }
            other => panic!("expected run, got {other:?}"),
        }
    }

    #[test]
    fn run_permission_flags_default_empty() {
        let cli = Cli::try_parse_from(["cortex", "run", "hi"]).expect("parse");
        match cli.command {
            Some(Command::Run(args)) => {
                assert!(args.permission_allow.is_empty());
                assert!(args.permission_deny.is_empty());
                assert!(args.permission_allow_save.is_empty());
                assert_eq!(args.schema, None);
            }
            other => panic!("expected run, got {other:?}"),
        }
    }

    #[test]
    fn run_parses_structured_schema_flag() {
        let cli = Cli::try_parse_from([
            "cortex",
            "run",
            "--format",
            "json",
            "--schema",
            "{\"type\":\"object\"}",
            "hi",
        ])
        .expect("parse");
        match cli.command {
            Some(Command::Run(args)) => {
                assert_eq!(args.format, OutputFormat::Json);
                assert_eq!(args.schema.as_deref(), Some("{\"type\":\"object\"}"));
            }
            other => panic!("expected run, got {other:?}"),
        }
    }

    #[test]
    fn run_parses_repeatable_permission_allow_save_flags() {
        let cli = Cli::try_parse_from([
            "cortex",
            "run",
            "hi",
            "--permission-allow-save",
            "bash:echo *",
            "--permission-allow-save",
            "write:src/**",
        ])
        .expect("parse");
        match cli.command {
            Some(Command::Run(args)) => {
                assert_eq!(
                    args.permission_allow_save,
                    vec!["bash:echo *".to_owned(), "write:src/**".to_owned()]
                );
            }
            other => panic!("expected run, got {other:?}"),
        }
    }

    #[test]
    fn run_jsonl_shorthand_is_parsed() {
        let cli = Cli::try_parse_from(["cortex", "run", "hi", "--jsonl"]).expect("parse");
        match cli.command {
            Some(Command::Run(args)) => {
                assert!(args.jsonl);
                assert_eq!(args.format, OutputFormat::Text);
            }
            other => panic!("expected run, got {other:?}"),
        }
    }

    #[test]
    fn run_continue_shorthand() {
        let cli = Cli::try_parse_from(["cortex", "run", "-c", "hi"]).expect("parse");
        match cli.command {
            Some(Command::Run(args)) => assert!(args.r#continue),
            other => panic!("expected run, got {other:?}"),
        }
    }

    #[test]
    fn run_format_and_jsonl_conflict() {
        let err = Cli::try_parse_from(["cortex", "run", "hi", "--format", "json", "--jsonl"])
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ArgumentConflict);
    }

    #[test]
    fn serve_defaults() {
        let cli = Cli::try_parse_from(["cortex", "serve"]).expect("parse");
        match cli.command {
            Some(Command::Serve(args)) => {
                assert_eq!(args.hostname, "127.0.0.1");
                assert_eq!(args.port, 3200);
                assert!(!args.public);
                assert_eq!(args.token, None);
            }
            other => panic!("expected serve, got {other:?}"),
        }
    }

    #[test]
    fn acp_command_parses() {
        let cli = Cli::try_parse_from(["cortex", "acp"]).expect("parse");
        assert!(matches!(cli.command, Some(Command::Acp(_))));
    }

    #[test]
    fn serve_parses_public_token_auth() {
        let cli = Cli::try_parse_from([
            "cortex",
            "serve",
            "--hostname",
            "127.0.0.1",
            "--port",
            "3201",
            "--public",
            "--token",
            "server-token",
        ])
        .expect("parse");
        match cli.command {
            Some(Command::Serve(args)) => {
                assert_eq!(args.hostname, "127.0.0.1");
                assert_eq!(args.port, 3201);
                assert!(args.public);
                assert_eq!(args.token.as_deref(), Some("server-token"));
            }
            other => panic!("expected serve, got {other:?}"),
        }
    }

    #[test]
    fn serve_public_requires_token() {
        let err = Cli::try_parse_from(["cortex", "serve", "--public"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn mcp_add_parses_remote_url_headers_and_timeout() {
        let cli = Cli::try_parse_from([
            "cortex",
            "mcp",
            "add",
            "api",
            "--url",
            "http://127.0.0.1:3210/mcp",
            "--header",
            "Authorization=Bearer static-token",
            "--timeout",
            "5",
            "--format",
            "json",
        ])
        .expect("parse");
        match cli.command {
            Some(Command::Mcp(McpArgs {
                command: McpCommand::Add(args),
            })) => {
                assert_eq!(args.name, "api");
                assert_eq!(args.url.as_deref(), Some("http://127.0.0.1:3210/mcp"));
                assert_eq!(args.headers, vec!["Authorization=Bearer static-token"]);
                assert_eq!(args.timeout_secs, Some(5));
                assert_eq!(args.format.format, OutputFormat::Json);
            }
            other => panic!("expected mcp add, got {other:?}"),
        }
    }

    #[test]
    fn mcp_add_preserves_trailing_hyphen_prefixed_command_args() {
        let cli = Cli::try_parse_from([
            "cortex",
            "mcp",
            "add",
            "demo",
            "--cwd",
            "/tmp/server-work",
            "--env",
            "FOO=bar",
            "--disabled",
            "--force",
            "--format",
            "json",
            "--command",
            "npx",
            "-y",
            "@scope/server",
            "--stdio",
        ])
        .expect("parse");
        match cli.command {
            Some(Command::Mcp(McpArgs {
                command: McpCommand::Add(args),
            })) => {
                assert_eq!(args.name, "demo");
                assert_eq!(
                    args.command,
                    vec![
                        "npx".to_owned(),
                        "-y".to_owned(),
                        "@scope/server".to_owned(),
                        "--stdio".to_owned()
                    ]
                );
                assert_eq!(args.cwd.as_deref(), Some("/tmp/server-work"));
                assert_eq!(args.env, vec!["FOO=bar"]);
                assert!(args.disabled);
                assert!(args.force);
                assert_eq!(args.format.format, OutputFormat::Json);
            }
            other => panic!("expected mcp add, got {other:?}"),
        }
    }

    #[test]
    fn mcp_auth_parses_callback_port_and_timeout() {
        let cli = Cli::try_parse_from([
            "cortex",
            "mcp",
            "auth",
            "api",
            "--callback-port",
            "3214",
            "--timeout",
            "10",
            "--format",
            "json",
        ])
        .expect("parse");
        match cli.command {
            Some(Command::Mcp(McpArgs {
                command: McpCommand::Auth(args),
            })) => {
                assert_eq!(args.name, "api");
                assert_eq!(args.callback_port, Some(3214));
                assert_eq!(args.timeout_secs, 10);
                assert_eq!(args.format.format, OutputFormat::Json);
            }
            other => panic!("expected mcp auth, got {other:?}"),
        }
    }

    #[test]
    fn unknown_subcommand_is_rejected() {
        let err = Cli::try_parse_from(["cortex", "bogus"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
    }

    #[test]
    fn unknown_flag_is_rejected() {
        let err = Cli::try_parse_from(["cortex", "run", "--not-a-flag"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::UnknownArgument);
    }
}
