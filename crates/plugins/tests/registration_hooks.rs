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
  input.experimental_workspace.register("folder", {
    name: "Folder adapter",
    description: "Workspace adapter from plugin test"
  });
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
        auth: "plugin-auth",
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
    assert_eq!(
        report.tools[0]
            .auth
            .as_ref()
            .map(|auth| auth.provider.as_str()),
        Some("plugin-auth")
    );
    assert_eq!(
        report.tools[0].auth.as_ref().map(|auth| auth.required),
        Some(true)
    );
    assert_eq!(report.auth_methods[0].id, "plugin-auth");
    assert_eq!(report.providers[0].models[0].id, "plugin/example-model");
    assert!(
        report.workspace_entries.iter().any(|workspace| {
            workspace.workspace_type == "folder"
                && workspace.name == "Folder adapter"
                && workspace.description == "Workspace adapter from plugin test"
        }),
        "workspace registrations should be separate from tool/auth/provider registrations: {:?}",
        report.workspace_entries
    );

    let execution = RuntimeHost::new()
        .with_timeout(Duration::from_secs(15))
        .execute_tool(
            &loaded[0].spec,
            "cortex_echo",
            json!({ "text": "real-js" }),
            None,
        )
        .expect("tool executes");
    assert_eq!(execution.content, "ECHO:real-js");

    let completion = RuntimeHost::new()
        .with_timeout(Duration::from_secs(15))
        .complete_with_provider(&loaded[0].spec, "plugin/example-model", json!([]), None)
        .expect("provider completes");
    assert_eq!(completion.content, "PLUGIN_MODEL:plugin/example-model");

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

#[test]
fn load_with_resolved_config_passes_config_into_plugin_hook() {
    if skip_without_runtime() {
        return;
    }

    let project = scratch("resolved-config");
    let plugin = project.join("plugin.mjs");
    let marker = project.join("config.jsonl");
    std::fs::write(
        &plugin,
        r#"
import { appendFile } from "node:fs/promises";

export default async function plugin(input, options) {
  return {
    async config(config) {
      await appendFile(options.marker, JSON.stringify({
        model: config.model ?? null,
        agent: config.agent ?? null,
        max_steps: config.max_steps ?? null
      }) + "\n");
      return {
        ...config,
        permission: [
          { tool: "bash", pattern: "echo blocked-by-config-hook", action: "deny" }
        ]
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
    let resolved = json!({
        "model": "anthropic/claude-sonnet-4",
        "agent": "general",
        "max_steps": 25
    });

    let loaded = RuntimeHost::new()
        .with_timeout(Duration::from_secs(15))
        .load_plugins_for_surface_with_config(&config, "run", resolved.clone())
        .expect("plugin loads with resolved config");
    assert_eq!(loaded.len(), 1);
    assert_eq!(
        loaded[0]
            .report
            .config
            .as_ref()
            .and_then(|cfg| cfg.get("model")),
        Some(&json!("anthropic/claude-sonnet-4"))
    );
    assert_eq!(
        loaded[0]
            .report
            .config
            .as_ref()
            .and_then(|cfg| cfg.get("agent")),
        Some(&json!("general"))
    );
    assert_eq!(
        loaded[0]
            .report
            .config
            .as_ref()
            .and_then(|cfg| cfg.get("max_steps")),
        Some(&json!(25))
    );
    assert_eq!(loaded[0].report.config_rules[0].action, "deny");

    let line = std::fs::read_to_string(marker)
        .expect("config marker")
        .lines()
        .find(|entry| !entry.trim().is_empty())
        .expect("one marker line")
        .to_owned();
    let observed: serde_json::Value = serde_json::from_str(&line).expect("valid config json");
    assert_eq!(observed["model"], resolved["model"]);
    assert_eq!(observed["agent"], resolved["agent"]);
    assert_eq!(observed["max_steps"], resolved["max_steps"]);
}
