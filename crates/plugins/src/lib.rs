//! External JavaScript plugin host foundation.
//!
//! Plugins are real local JavaScript modules loaded through an external runtime selected in a
//! stable order: Bun, then Node, then Deno. A configured plugin is never silently skipped: if no
//! runtime is available, the plugin file is missing, or the module throws while loading, the host
//! returns an actionable error before the run proceeds.
//!
//! # Request-shaping hooks (plugin author contract)
//!
//! Request-shaping hooks rewrite the outbound LLM request and advertised tool schemas before they
//! leave a surface (`cortex run`, TUI, server turn, ACP). Composition is **deterministic
//! configuration order**: plugins are visited in the order they appear in the `plugin = [...]`
//! array of the effective config, and each hook's return value becomes the next plugin's input for
//! the same phase. The order is stable across runs and does not depend on load timing, plugin
//! package name, or filesystem sort.
//!
//! ## Phases applied to each LLM request
//!
//! Surfaces that shape a request apply these phases in this order:
//!
//! 1. **`chat.message`** — per message. For each configured plugin that registers this hook, the
//!    host walks `messages` in array order and replaces each message with the hook return value.
//! 2. **`experimental.chat.messages.transform`** (alias: **`chat.messages`**) — whole-array
//!    transform. Plugins run in configuration order; each return value replaces the messages array.
//! 3. **`experimental.chat.system.transform`** (alias: **`chat.system`**) — system-prompt text
//!    transform. Plugins run in configuration order. If a system message is missing and a plugin
//!    returns non-empty text, one is inserted at the front of `messages`.
//! 4. **`chat.params`** — request parameter object (e.g. temperature, extra provider params).
//!    Plugins run in configuration order; each return value replaces the parameter map.
//! 5. **`chat.headers`** — extra HTTP headers on the provider request. Plugins run in configuration
//!    order; each return value replaces the header map.
//!
//! Advertised tool definitions are shaped separately (before tools are shown to the model) with:
//!
//! 6. **`tool.definition`** — per tool definition. For each configured plugin that registers this
//!    hook, the host walks each tool definition and replaces it with the hook return value, again
//!    in configuration order across plugins.
//!
//! Empty return values are treated as “no change” for the values hooks; a missing hook on a plugin
//! simply skips that plugin for the phase.
//!
//! ## Composition guarantee
//!
//! When two plugins `a` then `b` are listed as `plugin = ["./a.mjs", "./b.mjs"]` and both register
//! `chat.message`, every run observes callbacks in the order `a`, then `b`. Later plugins always
//! see earlier plugins' transformed messages, params, headers, system text, or tool definitions.
//! That documented order is the contract authors must match when ordering cooperating plugins; it
//! is pinned by black-box tests in `crates/cli/tests/plugin_request_shaping_hooks.rs`.

mod config;
mod distribution;
mod runtime;

pub use config::{PluginConfig, PluginSpec};
pub use distribution::{
    LockedPlugin, locked_plugins, lockfile_relative_path, remove_locked_plugin,
};
pub use runtime::{
    JsRuntime, LoadedPlugin, PluginAgentRegistration, PluginAuthAuthorizeResult,
    PluginAuthCallbackResult, PluginAuthMethodRegistration, PluginAuthMethodType, PluginAuthPrompt,
    PluginAuthPromptOption, PluginAuthRegistration, PluginCatalogRegistration,
    PluginCommandRegistration, PluginConfigRule, PluginEventHooks, PluginHookOutput,
    PluginHookReport, PluginLoadReport, PluginLoadSkip, PluginModelRegistration,
    PluginProviderCompletion, PluginProviderRegistration, PluginRuntimeError,
    PluginSkillRegistration, PluginThemeRegistration, PluginToolExecution, PluginToolRegistration,
    PluginWorkspaceAdapterEntry, PluginWorkspaceRegistration, RuntimeHost, RuntimeSelection,
    detect_runtime, detect_runtime_in_path, merge_compaction_value,
};
