//! Coverage for the shared compaction-hook seam (`PluginEventHooks::apply_compaction_hooks`) that
//! the server turn path reuses so a server-driven compaction is transformed identically to the
//! headless `run` path. The pure merge helper is asserted offline; the real hook invocation is
//! gated on a JS runtime.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use audit::AuditSink;
use plugins::{PluginConfig, PluginEventHooks, detect_runtime, merge_compaction_value};
use serde_json::{Value, json};
use toml::Value as TomlValue;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "cortex-plugin-compaction-{tag}-{}-{}",
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
        eprintln!("skipping plugin compaction-hook proof: no Bun, Node, or Deno on PATH");
        true
    } else {
        false
    }
}

#[test]
fn merge_compaction_value_folds_string_bool_and_object() {
    let base = json!({
        "summary": "original",
        "tail_start_id": "tail-1",
        "continued_request": true,
        "model": "m",
    });

    // A bare string sets the summary.
    let merged = merge_compaction_value(base.clone(), json!("HOOKED"));
    assert_eq!(merged["summary"], "HOOKED");
    assert_eq!(merged["tail_start_id"], "tail-1");

    // A bare bool sets the continuation flag.
    let merged = merge_compaction_value(base.clone(), json!(false));
    assert_eq!(merged["continued_request"], false);
    assert_eq!(merged["summary"], "original");

    // An object is shallow-merged over the current fields.
    let merged = merge_compaction_value(base.clone(), json!({ "summary": "X", "model": "small" }));
    assert_eq!(merged["summary"], "X");
    assert_eq!(merged["model"], "small");
    assert_eq!(merged["tail_start_id"], "tail-1");

    // Null leaves the value untouched.
    assert_eq!(merge_compaction_value(base.clone(), Value::Null), base);
}

/// A real plugin's `experimental.session.compacting` and `experimental.compaction.autocontinue`
/// hooks transform the compaction record JSON through the external runtime, and each invocation is
/// recorded to the attached audit sink with a `transform_compaction` action.
#[test]
fn apply_compaction_hooks_transforms_records_and_audits() {
    if skip_without_runtime() {
        return;
    }

    let project = scratch("apply");
    let plugin = project.join("plugin.mjs");
    let marker = project.join("hooks.jsonl");
    std::fs::write(
        &plugin,
        r#"
import { appendFile } from "node:fs/promises";

export default async function plugin(input, options) {
  async function mark(payload) {
    await appendFile(options.marker, JSON.stringify(payload) + "\n");
  }
  return {
    experimental: {
      session: {
        async compacting(record) {
          await mark({ hook: "experimental.session.compacting", tail: record.tail_start_id });
          return { ...record, summary: `HOOKED:${record.summary}` };
        }
      },
      compaction: {
        async autocontinue(record) {
          await mark({ hook: "experimental.compaction.autocontinue", tail: record.tail_start_id });
          return { ...record, continued_request: false };
        }
      }
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
    let sink = Arc::new(AuditSink::new(&audit_path, "ses_compaction").with_surface("serve"));
    let hooks = PluginEventHooks::load(&config, "serve")
        .expect("plugin loads through the external runtime")
        .with_audit(sink);

    let mut records = vec![json!({
        "summary": "kept BLUEBIRD",
        "tail_start_id": "tail-42",
        "continued_request": true,
        "model": "google/gemini-2.5-flash",
    })];
    hooks.apply_compaction_hooks(&mut records);

    // The plugin transformed both the summary and the continuation flag in place.
    assert_eq!(records[0]["summary"], "HOOKED:kept BLUEBIRD");
    assert_eq!(records[0]["continued_request"], false);
    assert_eq!(records[0]["tail_start_id"], "tail-42");

    // Both hook invocations fired their side effect.
    let log = std::fs::read_to_string(&marker).expect("hook marker exists");
    assert!(
        log.contains("experimental.session.compacting"),
        "session.compacting hook fired: {log}"
    );
    assert!(
        log.contains("experimental.compaction.autocontinue"),
        "compaction.autocontinue hook fired: {log}"
    );

    // Each invocation is a typed, surface-attributed audit record tagged transform_compaction.
    let audit_records = audit::read_jsonl(&audit_path).expect("audit log parseable");
    for hook in [
        "experimental.session.compacting",
        "experimental.compaction.autocontinue",
    ] {
        let record = audit_records
            .iter()
            .find(|record| record["type"] == "plugin_hook" && record["hook"] == hook)
            .unwrap_or_else(|| panic!("missing audit record for {hook}: {audit_records:#?}"));
        assert_eq!(record["action"], "transform_compaction");
        assert_eq!(record["outcome"], "success");
        assert_eq!(record["surface"], "serve");
        assert_eq!(record["detail"], "tail-42");
    }
}

/// With no loaded plugins the seam is a no-op: the record JSON is returned untouched.
#[test]
fn apply_compaction_hooks_is_a_noop_without_plugins() {
    let hooks = PluginEventHooks::disabled("serve");
    let original = json!({
        "summary": "kept BLUEBIRD",
        "tail_start_id": "tail-1",
        "continued_request": true,
        "model": "google/gemini-2.5-flash",
    });
    let mut records = vec![original.clone()];
    hooks.apply_compaction_hooks(&mut records);
    assert_eq!(records[0], original);
}
