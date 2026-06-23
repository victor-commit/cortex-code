//! External JavaScript plugin host foundation.
//!
//! Plugins are real local JavaScript modules loaded through an external runtime selected in a
//! stable order: Bun, then Node, then Deno. A configured plugin is never silently skipped: if no
//! runtime is available, the plugin file is missing, or the module throws while loading, the host
//! returns an actionable error before the run proceeds.

mod config;
mod distribution;
mod runtime;

pub use config::{PluginConfig, PluginSpec};
pub use runtime::{
    JsRuntime, LoadedPlugin, PluginAuthRegistration, PluginConfigRule, PluginEventHooks,
    PluginHookOutput, PluginHookReport, PluginLoadReport, PluginModelRegistration,
    PluginProviderRegistration, PluginRuntimeError, PluginToolExecution, PluginToolRegistration,
    RuntimeHost, RuntimeSelection, detect_runtime, detect_runtime_in_path,
};
