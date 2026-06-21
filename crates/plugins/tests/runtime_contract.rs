use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use plugins::{PluginConfig, PluginRuntimeError, RuntimeHost, detect_runtime};
use serde_json::Value;
use toml::Value as TomlValue;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "cortex-plugin-runtime-{tag}-{}-{}",
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

fn write_plugin(path: &Path, body: &str) {
    std::fs::write(path, body).expect("write plugin");
}

fn skip_without_runtime() -> bool {
    if detect_runtime().is_none() {
        eprintln!("skipping plugin runtime proof: no Bun, Node, or Deno on PATH");
        true
    } else {
        false
    }
}

#[test]
fn local_plugin_executes_for_real_and_receives_context() {
    if skip_without_runtime() {
        return;
    }

    let project = scratch("execute");
    let plugin = project.join("plugin.mjs");
    let marker = project.join("marker.json");
    write_plugin(
        &plugin,
        r#"
import { writeFile } from "node:fs/promises";

export default async function plugin(input, options) {
  if (!input.client || !input.server) {
    throw new Error("missing client/server handle");
  }
  const shell = await input.shell("printf shell-ok");
  return {
    async event(event) {
      const runtime = globalThis.Bun
        ? `bun:${Bun.version}`
        : globalThis.Deno
          ? `deno:${Deno.version.deno}`
          : `node:${process.versions.node}`;
      await writeFile(options.marker, JSON.stringify({
        eventKind: event.kind,
        directory: input.directory,
        projectDirectory: input.project.directory,
        handles: Boolean(input.client && input.server),
        shell: shell.stdout,
        runtime,
        pid: process.pid
      }));
    }
  };
}
"#,
    );
    let config = config_from(
        &project,
        &format!(
            "plugin = [[\"./plugin.mjs\", {{ marker = \"{}\" }}]]",
            marker.display()
        ),
    );

    let reports = RuntimeHost::new()
        .with_timeout(Duration::from_secs(15))
        .load_config(&config)
        .expect("plugin loads");

    assert_eq!(reports.len(), 1);
    assert!(
        reports[0].hooks.iter().any(|hook| hook == "event"),
        "the plugin returned an event hook"
    );
    assert!(
        reports[0].invoked_hooks.iter().any(|hook| hook == "event"),
        "the host invoked the event hook"
    );

    let marker: Value =
        serde_json::from_str(&std::fs::read_to_string(marker).expect("marker exists")).unwrap();
    assert_eq!(marker["eventKind"], "lifecycle");
    assert_eq!(marker["directory"], project.display().to_string());
    assert_eq!(marker["projectDirectory"], project.display().to_string());
    assert_eq!(marker["handles"], true);
    assert_eq!(marker["shell"], "shell-ok");
    assert!(
        marker["runtime"]
            .as_str()
            .is_some_and(|runtime| runtime.starts_with("bun:")
                || runtime.starts_with("node:")
                || runtime.starts_with("deno:")),
        "runtime marker proves JS execution: {marker}"
    );
    assert!(
        marker["pid"].as_u64().is_some_and(|pid| pid > 0),
        "runtime pid recorded: {marker}"
    );
}

#[test]
fn configured_plugin_fails_closed_when_no_runtime_is_on_path() {
    let project = scratch("no-runtime");
    let empty_path = project.join("empty-path");
    std::fs::create_dir_all(&empty_path).unwrap();
    let marker = project.join("should-not-exist");
    let plugin = project.join("plugin.mjs");
    write_plugin(
        &plugin,
        &format!(
            r#"
import {{ writeFile }} from "node:fs/promises";
export default async function plugin() {{
  await writeFile("{}", "executed");
}}
"#,
            marker.display()
        ),
    );
    let config = config_from(&project, r#"plugin = ["./plugin.mjs"]"#);

    let err = RuntimeHost::new()
        .with_path(empty_path.into_os_string())
        .with_timeout(Duration::from_secs(2))
        .load_config(&config)
        .unwrap_err();

    assert!(matches!(err, PluginRuntimeError::NoRuntime));
    assert!(
        !marker.exists(),
        "the plugin body must not run without a runtime"
    );
    assert!(
        err.to_string().contains("Bun, Node, or Deno"),
        "missing-runtime error is actionable: {err}"
    );
}

#[test]
fn plugin_load_throw_names_the_plugin_and_runtime_message() {
    if skip_without_runtime() {
        return;
    }

    let project = scratch("throw");
    let plugin = project.join("boom.mjs");
    write_plugin(
        &plugin,
        r#"
throw new Error("boom-load");
export default function plugin() {
  return {};
}
"#,
    );
    let config = config_from(&project, r#"plugin = ["./boom.mjs"]"#);

    let err = RuntimeHost::new()
        .with_timeout(Duration::from_secs(15))
        .load_config(&config)
        .unwrap_err();
    let message = err.to_string();

    assert!(
        message.contains("boom.mjs"),
        "error names plugin path: {message}"
    );
    assert!(
        message.contains("boom-load"),
        "error includes runtime error message: {message}"
    );
    assert!(
        matches!(err, PluginRuntimeError::PluginThrew { .. }),
        "load throw is attributed as a plugin load failure"
    );
}
