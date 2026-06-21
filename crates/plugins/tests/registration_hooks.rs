use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use plugins::{PluginConfig, RuntimeHost, detect_runtime};
use serde_json::json;
use toml::Value as TomlValue;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "cortex-plugin-registration-{tag}-{}-{}",
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
        eprintln!("skipping plugin registration proof: no Bun, Node, or Deno on PATH");
        true
    } else {
        false
    }
}

#[test]
fn registration_hooks_return_config_tool_auth_and_provider_entries() {
    if skip_without_runtime() {
        return;
    }

    let project = scratch("load");
    let plugin = project.join("plugin.mjs");
    let marker = project.join("events.jsonl");
    std::fs::write(
        &plugin,
        r#"
import { appendFile } from "node:fs/promises";

export default async function plugin(input, options) {
  return {
    async event(event) {
      await appendFile(options.marker, JSON.stringify({
        kind: event.kind,
        type: event.type,
        phase: event.phase,
        surface: event.surface
      }) + "\n");
    },
    async config(config) {
      return {
        permission: [
          { tool: "bash", pattern: "rm *", action: "deny" }
        ]
      };
    },
    async tool(registry) {
      registry.register({
        name: "cortex_echo",
        description: "Echo text from the plugin runtime.",
        parameters: {
          type: "object",
          properties: { text: { type: "string" } },
          required: ["text"],
          additionalProperties: false
        },
        async execute(args) {
          return `ECHO:${args.text}`;
        }
      });
    },
    async auth() {
      return [{ id: "plugin-auth", name: "Plugin Auth", env: ["PLUGIN_AUTH_TOKEN"] }];
    },
    async provider() {
      return {
        id: "plugin-provider",
        name: "Plugin Provider",
        models: [
          { id: "plugin/example-model", name: "Plugin Example", tool_call: false }
        ],
        async complete(request) {
          return `PLUGIN_MODEL:${request.model}`;
        }
      };
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

    let loaded = RuntimeHost::new()
        .with_timeout(Duration::from_secs(15))
        .load_plugins(&config)
        .expect("plugin loads");
    assert_eq!(loaded.len(), 1);

    let report = &loaded[0].report;
    for hook in ["event", "config", "tool", "auth", "provider"] {
        assert!(
            report.hooks.iter().any(|candidate| candidate == hook),
            "hook `{hook}` should be detected: {:?}",
            report.hooks
        );
        assert!(
            report
                .invoked_hooks
                .iter()
                .any(|candidate| candidate == hook),
            "hook `{hook}` should be invoked during registration: {:?}",
            report.invoked_hooks
        );
    }
    assert_eq!(report.config_rules[0].tool, "bash");
    assert_eq!(report.config_rules[0].action, "deny");
    assert_eq!(report.tools[0].name, "cortex_echo");
    assert_eq!(report.auth_methods[0].id, "plugin-auth");
    assert_eq!(report.providers[0].models[0].id, "plugin/example-model");

    let execution = RuntimeHost::new()
        .with_timeout(Duration::from_secs(15))
        .execute_tool(&loaded[0].spec, "cortex_echo", json!({ "text": "real-js" }))
        .expect("tool executes");
    assert_eq!(execution.content, "ECHO:real-js");

    let completion = RuntimeHost::new()
        .with_timeout(Duration::from_secs(15))
        .complete_with_provider(&loaded[0].spec, "plugin/example-model", json!([]))
        .expect("provider completes");
    assert_eq!(completion, "PLUGIN_MODEL:plugin/example-model");

    RuntimeHost::new()
        .with_timeout(Duration::from_secs(15))
        .invoke_event(
            &loaded[0].spec,
            json!({
                "kind": "tool.call",
                "type": "tool.call",
                "id": "call_123",
                "name": "cortex_echo"
            }),
        )
        .expect("event hook runs");
    let log = std::fs::read_to_string(marker).expect("event marker");
    assert!(
        log.contains("\"kind\":\"lifecycle\"") && log.contains("\"kind\":\"tool.call\""),
        "event hook should see load and later real events: {log}"
    );
}
