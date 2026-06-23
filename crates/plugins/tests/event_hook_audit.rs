use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use audit::AuditSink;
use plugins::{PluginConfig, PluginEventHooks, detect_runtime};
use serde_json::{Value, json};
use toml::Value as TomlValue;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "cortex-plugin-event-audit-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn config_from(project: &Path, body: &str) -> PluginConfig {
    let value: TomlValue = toml::from_str(body).expect("valid toml");
    PluginConfig::from_toml_value(value.get("plugin"), project).expect("plugin config")
}

fn skip_without_runtime() -> bool {
    if detect_runtime().is_none() {
        eprintln!("skipping plugin event-hook audit proof: no Bun, Node, or Deno on PATH");
        true
    } else {
        false
    }
}

/// A non-headless surface (TUI/server) drives plugin `event` hooks through `PluginEventHooks`.
/// When an audit sink is attached, the loaded plugin and each real event-hook invocation must be
/// recorded as typed, surface-attributed audit records in the shared sink, matching the headless
/// behavior. This reproduces VAL-CROSS-031: before the fix the side-effect marker fired but no
/// plugin/hook audit records existed for TUI/server.
#[test]
fn event_hooks_emit_surface_attributed_audit_records() {
    if skip_without_runtime() {
        return;
    }

    let project = scratch("tui-surface");
    let plugin = project.join("plugin.mjs");
    let marker = project.join("hook-marker.jsonl");
    std::fs::write(
        &plugin,
        r#"
import { appendFile } from "node:fs/promises";

export default async function plugin(input, options) {
  return {
    async event(event) {
      await appendFile(options.marker, JSON.stringify({
        kind: event.kind,
        surface: event.surface
      }) + "\n");
    }
  };
}
"#,
    )
    .expect("write plugin");

    let config = config_from(
        &project,
        &format!(
            "plugin = [[\"./plugin.mjs\", {{ marker = \"{}\" }}]]",
            marker.display()
        ),
    );

    let audit_path = project.join("audit.jsonl");
    let sink = Arc::new(AuditSink::new(&audit_path, "ses_tui_audit").with_surface("tui"));

    let hooks = PluginEventHooks::load(&config, "tui")
        .expect("plugin loads through the external runtime")
        .with_audit(sink);

    hooks.invoke_event(&json!({
        "kind": "turn.started",
        "session": "ses_tui_audit",
        "message": "msg_1"
    }));

    // The side effect still fires across the surface (load lifecycle event + the explicit event).
    let log = std::fs::read_to_string(&marker).expect("event marker exists");
    assert!(
        log.contains("\"kind\":\"turn.started\""),
        "the explicit event hook fired its side effect: {log}"
    );

    let records = audit::read_jsonl(&audit_path).expect("audit log is parseable");

    let load = records
        .iter()
        .find(|record| record["type"] == "plugin_load")
        .unwrap_or_else(|| panic!("a plugin_load record must exist: {records:#?}"));
    assert_eq!(load["surface"], "tui", "load record attributed to tui");
    assert_eq!(
        load["source"], "tui",
        "load record source attributed to tui"
    );
    assert!(
        load["plugin"]
            .as_str()
            .is_some_and(|spec| spec.contains("plugin.mjs")),
        "load record names the plugin: {load:#?}"
    );
    assert!(
        load["hooks"]
            .as_array()
            .is_some_and(|hooks| hooks.iter().any(|hook| hook == "event")),
        "load record lists the event hook: {load:#?}"
    );

    let hook = records
        .iter()
        .find(|record| record["type"] == "plugin_hook" && record["hook"] == "event")
        .unwrap_or_else(|| panic!("a plugin_hook event record must exist: {records:#?}"));
    assert_eq!(hook["surface"], "tui", "hook record attributed to tui");
    assert_eq!(
        hook["source"], "tui",
        "hook record source attributed to tui"
    );
    assert!(
        hook["plugin"]
            .as_str()
            .is_some_and(|spec| spec.contains("plugin.mjs")),
        "hook record names the plugin: {hook:#?}"
    );
    assert_eq!(hook["outcome"], "success", "hook invocation succeeded");

    // Distinct surfaces must produce distinct attribution: a "serve" sink labels records "server".
    let serve_audit = project.join("audit-serve.jsonl");
    let serve_sink = Arc::new(
        AuditSink::new(&serve_audit, "ses_serve_audit").with_surface_and_source("serve", "server"),
    );
    let serve_hooks = PluginEventHooks::load(&config, "serve")
        .expect("plugin loads for serve surface")
        .with_audit(serve_sink);
    serve_hooks.invoke_event(&json!({ "kind": "turn.started" }));
    let serve_records = audit::read_jsonl(&serve_audit).expect("serve audit parseable");
    let serve_hook: &Value = serve_records
        .iter()
        .find(|record| record["type"] == "plugin_hook" && record["hook"] == "event")
        .unwrap_or_else(|| {
            panic!("a serve plugin_hook event record must exist: {serve_records:#?}")
        });
    assert_eq!(serve_hook["surface"], "serve");
    assert_eq!(serve_hook["source"], "server");
}
