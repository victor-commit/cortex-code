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

fn write_package_plugin(root: &Path, name: &str, version: &str, body: &str) {
    std::fs::create_dir_all(root).expect("create package root");
    std::fs::write(
        root.join("package.json"),
        format!(
            r#"{{
  "name": "{name}",
  "version": "{version}",
  "type": "module",
  "main": "index.mjs"
}}
"#
        ),
    )
    .expect("write package json");
    std::fs::write(root.join("index.mjs"), body).expect("write package entry");
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
fn package_plugin_resolves_executes_receives_options_and_writes_lockfile() {
    if skip_without_runtime() {
        return;
    }

    let project = scratch("package");
    let package = project.join("package-plugin");
    let marker = project.join("package-marker.txt");
    write_package_plugin(
        &package,
        "cortex-package-plugin-proof",
        "1.2.3",
        r#"
import { writeFile } from "node:fs/promises";

export default async function plugin(input, options) {
  return {
    async event() {
      await writeFile(options.marker, `label=${options.label};dir=${input.directory}`);
    }
  };
}
"#,
    );
    let config = config_from(
        &project,
        &format!(
            "plugin = [[\"file:{}\", {{ marker = \"{}\", label = \"L1\" }}]]",
            package.display(),
            marker.display()
        ),
    );

    let reports = RuntimeHost::new()
        .with_timeout(Duration::from_secs(30))
        .load_config(&config)
        .expect("package plugin loads through package manager and runtime resolver");

    assert_eq!(reports.len(), 1);
    assert_eq!(
        std::fs::read_to_string(&marker).unwrap(),
        format!("label=L1;dir={}", project.display())
    );
    let lockfile = project.join(".cortex/plugin-lock.json");
    let lock = std::fs::read_to_string(&lockfile).expect("package lockfile exists");
    assert!(
        lock.contains("cortex-package-plugin-proof") && lock.contains("1.2.3"),
        "lockfile pins package version: {lock}"
    );

    let config = config_from(
        &project,
        &format!(
            "plugin = [[\"file:{}\", {{ marker = \"{}\", label = \"L2\" }}]]",
            package.display(),
            marker.display()
        ),
    );
    RuntimeHost::new()
        .with_timeout(Duration::from_secs(30))
        .load_config(&config)
        .expect("second package load uses the pinned module path");
    assert_eq!(
        std::fs::read_to_string(&marker).unwrap(),
        format!("label=L2;dir={}", project.display())
    );
}

#[test]
fn bare_package_spec_resolves_from_project_node_modules() {
    if skip_without_runtime() {
        return;
    }

    let project = scratch("bare-package");
    let name = "cortex-node-modules-plugin-proof";
    let package = project.join("node_modules").join(name);
    let marker = project.join("bare-package-marker.txt");
    write_package_plugin(
        &package,
        name,
        "4.5.6",
        r#"
import { writeFile } from "node:fs/promises";

export default async function plugin(_input, options) {
  return {
    async event() {
      await writeFile(options.marker, `bare=${options.label}`);
    }
  };
}
"#,
    );
    let config = config_from(
        &project,
        &format!(
            "plugin = [[\"{name}\", {{ marker = \"{}\", label = \"NODE_MODULES\" }}]]",
            marker.display()
        ),
    );

    RuntimeHost::new()
        .with_timeout(Duration::from_secs(15))
        .load_config(&config)
        .expect("bare package plugin resolves through runtime resolver");

    assert_eq!(
        std::fs::read_to_string(&marker).unwrap(),
        "bare=NODE_MODULES"
    );
    let lock = std::fs::read_to_string(project.join(".cortex/plugin-lock.json"))
        .expect("bare package lockfile exists");
    assert!(
        lock.contains(name) && lock.contains("4.5.6"),
        "lockfile pins bare package version: {lock}"
    );
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

#[test]
fn lenient_loading_skips_unresolvable_plugins_and_keeps_the_resolvable_subset() {
    if skip_without_runtime() {
        return;
    }

    let project = scratch("lenient");
    let good = project.join("good.mjs");
    write_plugin(
        &good,
        r#"
export default function plugin() {
  return {};
}
"#,
    );
    // One resolvable local plugin and one missing local file. Strict loading would fail closed on
    // the missing file; lenient loading must skip it and still load the resolvable plugin.
    let config = config_from(&project, r#"plugin = ["./good.mjs", "./missing.mjs"]"#);

    let (loaded, skipped) = RuntimeHost::new()
        .with_timeout(Duration::from_secs(15))
        .load_plugins_for_surface_with_config_lenient(
            &config,
            "serve",
            Value::Object(Default::default()),
        );

    assert_eq!(
        loaded.len(),
        1,
        "the resolvable plugin should still load: {loaded:?}"
    );
    assert_eq!(loaded[0].spec.raw_spec(), "./good.mjs");
    assert_eq!(skipped.len(), 1, "the missing plugin should be skipped");
    assert_eq!(skipped[0].spec, "./missing.mjs");
    assert!(
        skipped[0]
            .reason
            .to_ascii_lowercase()
            .contains("does not exist"),
        "skip reason should explain the missing file: {}",
        skipped[0].reason
    );
}
