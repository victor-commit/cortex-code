//! Black-box tests for the CLI plugin / provider / account surfaces:
//! - `cortex account` / `cortex console` control-plane login (VAL-GAP-CLI-002),
//! - `cortex provider list` / `cortex provider auth` (VAL-GAP-CLI-003),
//! - `cortex plugin install/list/update/remove` lifecycle (VAL-GAP-CLI-005).
//!
//! Every check is offline, deterministic, and uses isolated scratch dirs with the ambient Cortex
//! env scrubbed so each test fully controls resolution. The plugin lifecycle test resolves a real
//! local package plugin through the external JS runtime (skipped cleanly when none is on PATH).

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

struct Sandbox {
    home: PathBuf,
    project: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        let id = SCRATCH_COUNTER.fetch_add(1, Ordering::SeqCst);
        let home =
            std::env::temp_dir().join(format!("cortex-cli-ppa-{tag}-{}-{id}", std::process::id()));
        let project = home.join("project");
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&project).expect("create project");
        std::fs::create_dir_all(home.join(".local/share/cortex")).expect("create data dir");
        Self { home, project }
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

/// A base command rooted in the sandbox project with the ambient Cortex env scrubbed.
fn cortex(sandbox: &Sandbox, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.current_dir(&sandbox.project);
    command.args(args);
    command.env("HOME", &sandbox.home);
    command.env("XDG_CONFIG_HOME", sandbox.home.join(".config"));
    command.env("CORTEX_DATA_DIR", sandbox.home.join(".local/share/cortex"));
    command
        .env_remove("OPENROUTER_API_KEY")
        .env_remove("CORTEX_OPENROUTER_BASE_URL")
        .env_remove("CORTEX_NO_NET")
        .env_remove("CORTEX_AUTH_FILE")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE");
    command
}

fn run(mut command: Command, timeout: Duration) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().expect("spawn cortex");
    let mut out = child.stdout.take().expect("stdout");
    let mut err = child.stderr.take().expect("stderr");
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        use std::io::Read;
        let _ = out.read_to_end(&mut buf);
        buf
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        use std::io::Read;
        let _ = err.read_to_end(&mut buf);
        buf
    });

    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            break status;
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            panic!("command did not terminate within {timeout:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: out_thread.join().expect("join stdout"),
        stderr: err_thread.join().expect("join stderr"),
    }
}

fn quick(sandbox: &Sandbox, args: &[&str]) -> Output {
    run(cortex(sandbox, args), Duration::from_secs(20))
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn json(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|err| panic!("stdout is not JSON ({err}): {}", stdout(output)))
}

fn skip_without_runtime() -> bool {
    if plugins::detect_runtime().is_none() {
        eprintln!("skipping plugin lifecycle proof: no Bun, Node, or Deno on PATH");
        true
    } else {
        false
    }
}

fn write_package_plugin(root: &Path, name: &str, version: &str, marker: &Path) {
    std::fs::create_dir_all(root).expect("create package root");
    std::fs::write(
        root.join("package.json"),
        format!(
            r#"{{
  "name": "{name}",
  "version": "{version}",
  "type": "module",
  "main": "index.mjs",
  "engines": {{ "cortex": ">=0.1.0 <1.0.0" }}
}}
"#
        ),
    )
    .expect("write package json");
    std::fs::write(
        root.join("index.mjs"),
        format!(
            r#"
import {{ appendFile }} from "node:fs/promises";
export default async function plugin() {{
  return {{
    async event() {{
      await appendFile("{}", "loaded\n");
    }}
  }};
}}
"#,
            marker.display()
        ),
    )
    .expect("write package entry");
}

fn marker_lines(marker: &Path) -> usize {
    std::fs::read_to_string(marker)
        .map(|text| text.lines().count())
        .unwrap_or(0)
}

// --------------------------------------------------------------------------
// VAL-GAP-CLI-002: account/console login surfaces are explicit or unavailable
// --------------------------------------------------------------------------

#[test]
fn account_login_reports_clear_out_of_scope_diagnostic() {
    let sandbox = Sandbox::new("account-login");
    let output = quick(&sandbox, &["account", "login"]);
    assert_ne!(
        output.status.code(),
        Some(0),
        "account login is out of scope and must exit non-zero"
    );
    // stdout stays clean for machine consumers on the failure path.
    assert!(
        stdout(&output).is_empty(),
        "account login must not emit anything on stdout: {}",
        stdout(&output)
    );
    let err = stderr(&output);
    assert!(
        err.contains("not available") && err.to_lowercase().contains("control-plane"),
        "diagnostic should explain the control-plane surface is unavailable: {err}"
    );
    assert!(
        err.contains("cortex auth"),
        "diagnostic should point to the supported alternative: {err}"
    );
}

#[test]
fn console_logout_alias_is_explicit_and_non_zero() {
    let sandbox = Sandbox::new("console-logout");
    // `console` is the documented alias for `account`.
    let output = quick(&sandbox, &["console", "logout"]);
    assert_ne!(output.status.code(), Some(0));
    assert!(stdout(&output).is_empty());
    assert!(
        stderr(&output).to_lowercase().contains("not available"),
        "logout diagnostic should be explicit: {}",
        stderr(&output)
    );
}

#[test]
fn account_status_reports_unsupported_document() {
    let sandbox = Sandbox::new("account-status");
    let output = quick(&sandbox, &["account", "status", "--format", "json"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "status query should succeed: {}",
        stderr(&output)
    );
    let doc = json(&output);
    assert_eq!(doc["type"], "account");
    assert_eq!(doc["supported"], false);
    assert_eq!(doc["status"], "unavailable");
    assert_eq!(doc["control_plane"], false);
    assert_eq!(doc["alternative"], "cortex auth");
}

#[test]
fn account_is_a_known_command_not_an_unknown_subcommand_error() {
    let sandbox = Sandbox::new("account-known");
    // A clap "unrecognized subcommand" error would name the offending token; a known command
    // renders help instead. We assert the help path (exit 0) to prove no unknown-command confusion.
    let help = quick(&sandbox, &["account", "--help"]);
    assert_eq!(help.status.code(), Some(0));
    let text = stdout(&help).to_lowercase();
    assert!(
        text.contains("login") && text.contains("status"),
        "account --help should document its subcommands and status: {text}"
    );
    assert!(
        text.contains("unavailable") || text.contains("not available"),
        "account --help should document the unsupported status: {text}"
    );
}

// --------------------------------------------------------------------------
// VAL-GAP-CLI-003: providers command lists, filters, and authenticates
// --------------------------------------------------------------------------

#[test]
fn provider_list_matches_config_and_filters_by_id() {
    let sandbox = Sandbox::new("provider-list");
    let output = quick(&sandbox, &["provider", "list", "--format", "json"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "provider list should succeed: {}",
        stderr(&output)
    );
    let doc = json(&output);
    assert_eq!(doc["default_model"], "google/gemini-2.5-flash");
    let providers = doc["providers"].as_array().expect("providers array");
    let openrouter = providers
        .iter()
        .find(|p| p["id"] == "openrouter")
        .expect("openrouter present in default catalogue");
    assert_eq!(openrouter["enabled"], true);
    assert_eq!(
        openrouter["authenticated"], false,
        "no credential is stored yet"
    );
    assert!(
        openrouter["model_count"].as_u64().unwrap_or(0) >= 1,
        "provider health should report a model count: {openrouter}"
    );

    // Filtering narrows the list to the requested id.
    let filtered = quick(
        &sandbox,
        &["provider", "list", "--id", "openrouter", "--format", "json"],
    );
    let fdoc = json(&filtered);
    let fproviders = fdoc["providers"].as_array().expect("filtered providers");
    assert_eq!(
        fproviders.len(),
        1,
        "filter should keep exactly one provider"
    );
    assert_eq!(fproviders[0]["id"], "openrouter");

    // A non-matching filter yields an empty (but valid) list.
    let none = quick(
        &sandbox,
        &[
            "provider",
            "list",
            "--id",
            "no-such-provider",
            "--format",
            "json",
        ],
    );
    assert_eq!(none.status.code(), Some(0));
    assert!(
        json(&none)["providers"]
            .as_array()
            .expect("array")
            .is_empty(),
        "unknown filter id should produce an empty list"
    );
}

#[test]
fn provider_list_reflects_disabled_provider_config() {
    let sandbox = Sandbox::new("provider-disabled");
    let synthetic_key = format!("sk-{}-{}", "local", "configfixturebody000000");
    std::fs::write(
        sandbox.project.join("cortex.toml"),
        format!(
            r#"
[provider]
enabled = ["local"]
disabled = ["openrouter"]
[provider.model_overrides]
"google/gemini-2.5-flash" = "local/test-model"
[provider.local]
base_url = "http://127.0.0.1:3299"
api_key = "{synthetic_key}"
models = [{{ id = "local/test-model", name = "Local Test Model", tool_call = true, context_length = 8192, max_output_tokens = 1024 }}]
"#
        ),
    )
    .expect("write config");
    let output = quick(&sandbox, &["provider", "list", "--format", "json"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "provider list should succeed with configured providers: {}",
        stderr(&output)
    );
    let doc = json(&output);
    let providers = doc["providers"].as_array().expect("providers array");
    assert!(
        !providers.iter().any(|p| p["id"] == "openrouter"),
        "a disabled provider must not appear in the provider listing: {doc}"
    );
    assert!(
        providers.iter().any(|p| p["id"] == "local"),
        "a configured (enabled) provider must appear in the listing: {doc}"
    );
    // The configured api key must never be echoed in the machine output.
    assert!(
        !stdout(&output).contains(&synthetic_key),
        "provider list must redact configured secrets"
    );
}

#[test]
fn provider_auth_changes_authentication_status() {
    let sandbox = Sandbox::new("provider-auth");

    let before = quick(
        &sandbox,
        &["provider", "list", "--id", "openrouter", "--format", "json"],
    );
    assert_eq!(
        json(&before)["providers"][0]["authenticated"],
        false,
        "openrouter starts unauthenticated"
    );

    // `provider auth` runs the same credential flow as `auth login`.
    let synthetic_key = format!("sk-{}-{}", "or-v1", "provideridfixturebody00");
    let auth = quick(
        &sandbox,
        &[
            "provider",
            "auth",
            "openrouter",
            "--api-key",
            &synthetic_key,
            "--format",
            "json",
        ],
    );
    assert_eq!(
        auth.status.code(),
        Some(0),
        "provider auth should store the credential: {}",
        stderr(&auth)
    );
    assert_eq!(json(&auth)["provider"], "openrouter");
    assert_eq!(json(&auth)["stored"], true);

    let after = quick(
        &sandbox,
        &["provider", "list", "--id", "openrouter", "--format", "json"],
    );
    assert_eq!(
        json(&after)["providers"][0]["authenticated"],
        true,
        "auth status must flip to true after provider auth"
    );

    // The credential is consistent with `cortex auth list`.
    let auth_list = quick(&sandbox, &["auth", "list", "--format", "json"]);
    let stored = json(&auth_list)["providers"]
        .as_array()
        .expect("auth providers")
        .iter()
        .any(|p| p["id"] == "openrouter");
    assert!(stored, "auth list should also show the stored provider");

    // The synthetic key must never be echoed in machine output.
    assert!(
        !stdout(&after).contains(&synthetic_key) && !stdout(&auth).contains(&synthetic_key),
        "provider output must never leak the API key"
    );
}

// --------------------------------------------------------------------------
// VAL-GAP-CLI-005: plugin install command resolves and removes plugins
// --------------------------------------------------------------------------

#[test]
fn plugin_lifecycle_install_list_update_remove_is_deterministic() {
    if skip_without_runtime() {
        return;
    }

    let sandbox = Sandbox::new("plugin-lifecycle");
    let package = sandbox.home.join("pkg");
    let marker = sandbox.home.join("plugin-marker.txt");
    write_package_plugin(&package, "lifecycle-proof", "3.1.4", &marker);
    let spec = format!("file:{}", package.display());

    // install -> records spec in config + pins the lockfile.
    let install = run(
        cortex(&sandbox, &["plugin", "install", &spec, "--format", "json"]),
        Duration::from_secs(30),
    );
    assert_eq!(
        install.status.code(),
        Some(0),
        "plugin install should succeed: {}",
        stderr(&install)
    );
    let config_text =
        std::fs::read_to_string(sandbox.project.join("cortex.toml")).expect("project config");
    assert!(
        config_text.contains(&spec),
        "install should record the plugin spec in project config: {config_text}"
    );
    let lock_path = sandbox.project.join(".cortex/plugin-lock.json");
    let lock_after_install = std::fs::read_to_string(&lock_path).expect("plugin lockfile");
    assert!(
        lock_after_install.contains("lifecycle-proof")
            && lock_after_install.contains("3.1.4")
            && lock_after_install.contains("sha256:"),
        "lockfile should pin package, version, and integrity: {lock_after_install}"
    );

    // list -> shows the configured plugin with its lockfile pin.
    let list = quick(&sandbox, &["plugin", "list", "--format", "json"]);
    assert_eq!(list.status.code(), Some(0));
    let listed = json(&list);
    let entries = listed["plugins"].as_array().expect("plugins array");
    assert_eq!(entries.len(), 1, "exactly one plugin should be configured");
    assert_eq!(entries[0]["spec"], spec);
    assert_eq!(entries[0]["source"], "package");
    assert_eq!(entries[0]["locked"]["version"], "3.1.4");

    // The installed plugin actually loads on the run surface.
    let run_loaded = quick(
        &sandbox,
        &["run", "--command", "echo", "--command-arg", "installed"],
    );
    assert_eq!(
        run_loaded.status.code(),
        Some(0),
        "installed plugin should not break a run: {}",
        stderr(&run_loaded)
    );
    assert!(
        marker_lines(&marker) >= 1,
        "installed plugin event hook should have fired"
    );

    // update -> re-resolves and keeps the lockfile deterministic (same package/version).
    let update = run(
        cortex(&sandbox, &["plugin", "update", &spec, "--format", "json"]),
        Duration::from_secs(30),
    );
    assert_eq!(
        update.status.code(),
        Some(0),
        "plugin update should succeed: {}",
        stderr(&update)
    );
    assert_eq!(json(&update)["version"], "3.1.4");
    let lock_after_update = std::fs::read_to_string(&lock_path).expect("plugin lockfile");
    assert_eq!(
        lock_after_install, lock_after_update,
        "update must keep the lockfile pin deterministic"
    );

    // remove (via the `uninstall` alias) -> drops config + lock entries.
    let marker_before_remove = marker_lines(&marker);
    let remove = quick(
        &sandbox,
        &["plugin", "uninstall", &spec, "--format", "json"],
    );
    assert_eq!(
        remove.status.code(),
        Some(0),
        "plugin remove should succeed: {}",
        stderr(&remove)
    );
    assert_eq!(json(&remove)["config_changed"], true);
    assert_eq!(json(&remove)["lock_removed"], true);

    let list_after = quick(&sandbox, &["plugin", "list", "--format", "json"]);
    assert!(
        json(&list_after)["plugins"]
            .as_array()
            .expect("array")
            .is_empty(),
        "removed plugin must no longer be listed"
    );
    let lock_final = std::fs::read_to_string(&lock_path).expect("plugin lockfile");
    assert!(
        !lock_final.contains("lifecycle-proof"),
        "removed plugin must be dropped from the lockfile: {lock_final}"
    );

    // The removed plugin no longer loads: a fresh run does not advance the marker.
    let run_removed = quick(
        &sandbox,
        &["run", "--command", "echo", "--command-arg", "removed"],
    );
    assert_eq!(run_removed.status.code(), Some(0));
    assert_eq!(
        marker_lines(&marker),
        marker_before_remove,
        "removed plugin hook must not fire after removal"
    );
}

#[test]
fn plugin_remove_of_unknown_spec_is_a_clear_usage_error() {
    let sandbox = Sandbox::new("plugin-remove-unknown");
    let output = quick(&sandbox, &["plugin", "remove", "file:/nope/missing"]);
    assert_ne!(output.status.code(), Some(0));
    assert!(
        stderr(&output).contains("not found"),
        "removing a non-installed plugin should report it was not found: {}",
        stderr(&output)
    );
}

#[test]
fn plugin_update_of_unconfigured_spec_is_a_clear_usage_error() {
    let sandbox = Sandbox::new("plugin-update-unknown");
    let output = quick(&sandbox, &["plugin", "update", "@scope/never-installed"]);
    assert_ne!(output.status.code(), Some(0));
    assert!(
        stderr(&output).contains("not installed"),
        "updating an unconfigured plugin should report it is not installed: {}",
        stderr(&output)
    );
}
