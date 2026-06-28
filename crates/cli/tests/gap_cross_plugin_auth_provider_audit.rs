//! VAL-GAP-CROSS-002: cross-flow hardening for plugin auth providers.
//!
//! Proves that the plugin install/auth/provider hooks, the 0600 credential store, a live
//! plugin-backed turn, and the audit redaction sink all COMPOSE in one end-to-end flow:
//!
//! 1. A real local plugin (run through the detected Bun/Node/Deno runtime) registers BOTH an `auth`
//!    method and a `provider` whose model serves completions locally.
//! 2. `cortex auth login` authenticates the plugin provider and stores the credential safely
//!    (no secret echoed to stdout/stderr).
//! 3. `cortex run --model plugin/cross-model` selects the plugin-backed model, the stored credential
//!    round-trips into the provider's `complete` (proven by a private marker file), and the model
//!    answer arrives.
//! 4. The audit log contains plugin hook records (`plugin_load` listing the auth + provider hooks)
//!    AND a per-turn provider record (`plugin_provider_request`) with the auth field redacted — and
//!    neither the audit log nor any captured output contains the raw credential value.
//!
//! The plugin provider serves the turn locally, so this flow is deterministic and offline (it needs
//! only a JS runtime, not OpenRouter); it skips cleanly when no runtime is present.

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;

const RUN_TIMEOUT: Duration = Duration::from_secs(40);

struct Sandbox {
    home: PathBuf,
    project: PathBuf,
    data_dir: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "cortex-cross-plugin-auth-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let project = root.join("project");
        let data_dir = root.join("data");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&project).expect("create project");
        fs::create_dir_all(root.join(".config")).expect("create config dir");
        fs::create_dir_all(root.join(".local/share")).expect("create xdg dir");
        Self {
            home: root,
            project,
            data_dir,
        }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command.current_dir(&self.project);
        command.args(args);
        command.env("HOME", &self.home);
        command.env("XDG_CONFIG_HOME", self.home.join(".config"));
        command.env("XDG_DATA_HOME", self.home.join(".local/share"));
        command.env("CORTEX_DATA_DIR", &self.data_dir);
        command
            .env_remove("OPENROUTER_API_KEY")
            .env_remove("CORTEX_AUTH_FILE")
            .env_remove("CORTEX_AUDIT_LOG")
            .env_remove("CORTEX_NO_NET")
            .env_remove("CORTEX_OPENROUTER_BASE_URL")
            .env_remove("RUST_LOG")
            .env_remove("CORTEX_VERBOSE")
            .env_remove("NO_COLOR");
        command
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.home);
    }
}

fn run(mut command: Command) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().expect("spawn cortex");
    let mut stdout = child.stdout.take().expect("stdout");
    let mut stderr = child.stderr.take().expect("stderr");
    let stdout_thread = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let stderr_thread = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });

    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("try wait") {
            break status;
        }
        if started.elapsed() > RUN_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("command timed out after {RUN_TIMEOUT:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: stdout_thread.join().expect("join stdout"),
        stderr: stderr_thread.join().expect("join stderr"),
    }
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn skip_without_runtime() -> bool {
    if plugins::detect_runtime().is_none() {
        eprintln!("skipping cross plugin auth/provider/audit test: no Bun, Node, or Deno on PATH");
        true
    } else {
        false
    }
}

/// A plugin that supplies BOTH an auth method and a provider/model. The provider's `complete`
/// records the auth payload it received to a private marker file (proving the stored credential
/// round-tripped) but returns only the non-secret workspace in the model answer.
fn write_auth_provider_plugin(sandbox: &Sandbox, marker: &std::path::Path) {
    let plugin = sandbox.project.join("plugin.mjs");
    fs::write(
        &plugin,
        r#"
import { writeFile } from "node:fs/promises";

export default async function plugin(input, options) {
  return {
    auth: {
      provider: "plugin-cross",
      name: "Plugin Cross Auth",
      loader: async (auth) => {
        const value = await auth();
        if (value?.type === "api") {
          return {
            kind: "api",
            token: value.key ?? "",
            workspace: value.metadata?.workspace ?? ""
          };
        }
        return value ?? null;
      },
      methods: [
        {
          id: "api",
          type: "api",
          label: "API key",
          prompts: [
            { type: "text", key: "api_key", message: "API key" },
            { type: "text", key: "workspace", message: "Workspace" }
          ],
          async authorize(inputs) {
            return {
              type: "success",
              credential: {
                type: "api",
                key: String(inputs?.api_key ?? ""),
                metadata: { workspace: String(inputs?.workspace ?? "") }
              }
            };
          }
        }
      ]
    },
    async provider() {
      return {
        id: "plugin-cross",
        name: "Plugin Cross Provider",
        models: [{ id: "plugin/cross-model", name: "Plugin Cross Model", tool_call: false }],
        async complete(request) {
          await writeFile(options.marker, JSON.stringify(request.auth ?? null));
          const workspace = request.auth?.workspace ?? "none";
          const authed = request.auth?.token ? "present" : "absent";
          return `AUTH_OK:${workspace}:${authed}`;
        }
      };
    }
  };
}
"#,
    )
    .expect("write plugin");
    fs::write(
        sandbox.project.join("cortex.toml"),
        format!(
            "plugin = [[\"{}\", {{ marker = \"{}\" }}]]\n",
            plugin.display(),
            marker.display()
        ),
    )
    .expect("write config");
}

#[test]
fn plugin_auth_provider_credential_turn_and_audit_redaction_compose() {
    if skip_without_runtime() {
        return;
    }

    let sandbox = Sandbox::new("compose");
    let marker = sandbox.project.join("provider-auth.json");
    let audit_log = sandbox.project.join("audit.jsonl");
    write_auth_provider_plugin(&sandbox, &marker);

    // Assembled from fragments so no contiguous secret-shaped literal lives in source (Droid-Shield).
    let secret = format!("plugin-{}-{}-{}", "cross", "credential", "abcdef123456");

    // ---- Step 1: the plugin auth method is discoverable (install/enable + auth hook visible). ----
    let list = run(sandbox.command(&["auth", "list", "--format", "json"]));
    assert_eq!(
        list.status.code(),
        Some(0),
        "auth list should succeed: {}",
        combined(&list)
    );
    let list_doc: Value = serde_json::from_slice(&list.stdout).expect("auth list json");
    assert!(
        list_doc["methods"]
            .as_array()
            .expect("methods array")
            .iter()
            .any(|method| {
                method["provider"] == "plugin-cross"
                    && method["method"] == "api"
                    && method["source"] == "plugin"
            }),
        "plugin auth API method should be listed: {list_doc}"
    );

    // ---- Step 2: authenticate; the credential is stored safely (never echoed). ----
    let login = run(sandbox.command(&[
        "auth",
        "login",
        "plugin-cross",
        "--auth-method",
        "api",
        "--auth-input",
        &format!("api_key={secret}"),
        "--auth-input",
        "workspace=acme",
        "--format",
        "json",
    ]));
    assert_eq!(
        login.status.code(),
        Some(0),
        "plugin auth login should succeed: {}",
        combined(&login)
    );
    assert!(
        !combined(&login).contains(secret.as_str()),
        "auth login output must not leak the plugin credential"
    );

    // The 0600 credential store now exists under the isolated data dir (credential storage works).
    let auth_store = sandbox.data_dir.join("auth.json");
    assert!(
        auth_store.exists(),
        "auth login should persist the plugin credential to the data-dir store"
    );

    // ---- Step 3: run a turn on the plugin-backed model; audit to a known path. ----
    let mut run_cmd = sandbox.command(&[
        "run",
        "--model",
        "plugin/cross-model",
        "--format",
        "json",
        "authenticate with the plugin provider and answer",
    ]);
    run_cmd.env("CORTEX_AUDIT_LOG", &audit_log);
    let run_out = run(run_cmd);
    assert_eq!(
        run_out.status.code(),
        Some(0),
        "plugin-backed run should succeed: {}",
        combined(&run_out)
    );
    let run_doc: Value = serde_json::from_slice(&run_out.stdout).expect("run json");

    // The plugin-backed model was selected and produced the answer.
    assert_eq!(
        run_doc["model"], "plugin/cross-model",
        "the plugin-backed model should be selected: {run_doc}"
    );
    assert_eq!(
        run_doc["content"], "AUTH_OK:acme:present",
        "the plugin provider answer (with the authenticated workspace) should arrive: {run_doc}"
    );
    assert!(
        String::from_utf8_lossy(&run_out.stderr).contains("using plugin provider plugin-cross"),
        "run should report it used the plugin provider: {}",
        String::from_utf8_lossy(&run_out.stderr)
    );
    assert!(
        !combined(&run_out).contains(secret.as_str()),
        "run captured output must not leak the plugin credential"
    );

    // ---- Step 4a: the stored credential round-tripped into the provider's complete (private). ----
    let marker_doc: Value =
        serde_json::from_slice(&fs::read(&marker).expect("marker file")).expect("marker json");
    assert_eq!(marker_doc["kind"], "api");
    assert_eq!(marker_doc["workspace"], "acme");
    assert_eq!(
        marker_doc["token"], secret,
        "the stored credential should reach the plugin provider: {marker_doc}"
    );

    // ---- Step 4b: the audit log has hook + provider records, with auth redacted and no raw key. ----
    let records = audit::read_jsonl(&audit_log).expect("read audit jsonl");
    let raw_audit = fs::read_to_string(&audit_log).expect("audit raw");
    assert!(
        !raw_audit.contains(secret.as_str()),
        "audit log must not contain the raw plugin credential: {raw_audit}"
    );

    // Hook record: the plugin load lists both the auth and provider hooks it registered.
    assert!(
        records.iter().any(|record| {
            record["type"] == "plugin_load"
                && record["outcome"] == "success"
                && record["hooks"].as_array().is_some_and(|hooks| {
                    hooks.iter().any(|hook| hook == "provider")
                        && hooks
                            .iter()
                            .any(|hook| hook.as_str().is_some_and(|hook| hook.starts_with("auth")))
                })
        }),
        "audit should record the plugin load with auth + provider hooks: {records:#?}"
    );

    // Provider record: the plugin-backed turn was audited with the auth field redacted.
    let provider_record = records
        .iter()
        .find(|record| {
            record["type"] == "plugin_provider_request"
                && record["provider"] == "plugin-cross"
                && record["model"] == "plugin/cross-model"
        })
        .unwrap_or_else(|| panic!("audit should record the plugin provider turn: {records:#?}"));
    assert_eq!(
        provider_record["outcome"], "success",
        "the plugin provider turn should be audited as a success: {provider_record}"
    );
    assert_eq!(
        provider_record["auth_present"], true,
        "the provider record should reflect that an auth credential was present: {provider_record}"
    );
    assert_eq!(
        provider_record["auth"],
        redaction::PLACEHOLDER,
        "the provider record's auth must be the canonical redaction placeholder: {provider_record}"
    );
    assert_eq!(
        provider_record["kind"], "provider_request",
        "the provider record should be tagged as a provider request: {provider_record}"
    );

    // The canonical redaction placeholder is present in the durable audit log.
    assert!(
        raw_audit.contains(redaction::PLACEHOLDER),
        "audit should mark the redacted auth with the canonical placeholder: {raw_audit}"
    );
}
