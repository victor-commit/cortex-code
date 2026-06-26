use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct Sandbox {
    root: PathBuf,
    home: PathBuf,
    project: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "cortex-gap-tui-themes-audio-plugin-slots-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        let home = root.join("home");
        let project = root.join("project");
        std::fs::create_dir_all(&home).expect("create home");
        std::fs::create_dir_all(&project).expect("create project");
        Self {
            root,
            home,
            project,
        }
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn write(path: impl AsRef<Path>, body: &str) {
    std::fs::write(path, body).expect("write fixture");
}

fn tuistory_available() -> bool {
    Command::new("tuistory")
        .arg("--version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

fn runtime_available() -> bool {
    plugins::detect_runtime().is_some()
}

struct SessionGuard {
    session: String,
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        let _ = Command::new("tuistory")
            .arg("-s")
            .arg(&self.session)
            .arg("close")
            .output();
    }
}

fn launch_tui_session(sandbox: &Sandbox, session: &str) -> SessionGuard {
    let output = Command::new("tuistory")
        .arg("launch")
        .arg("--session")
        .arg(session)
        .arg("--background")
        .arg("--cols")
        .arg("240")
        .arg("--rows")
        .arg("60")
        .arg("--cwd")
        .arg(sandbox.project.display().to_string())
        .arg("--env")
        .arg(format!("HOME={}", sandbox.home.display()))
        .arg("--env")
        .arg(format!(
            "XDG_CONFIG_HOME={}",
            sandbox.home.join(".config").display()
        ))
        .arg("--env")
        .arg(format!(
            "CORTEX_DATA_DIR={}",
            sandbox.home.join(".local/share/cortex").display()
        ))
        .arg("--env")
        .arg("CORTEX_NO_NET=1")
        .arg("--env")
        .arg("OPENROUTER_API_KEY=")
        .arg(format!("{} tui", env!("CARGO_BIN_EXE_cortex")))
        .output()
        .expect("launch tuistory session");
    assert!(
        output.status.success(),
        "tuistory launch failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    SessionGuard {
        session: session.to_owned(),
    }
}

fn tuistory(session: &str, args: &[&str]) -> std::process::Output {
    Command::new("tuistory")
        .arg("-s")
        .arg(session)
        .args(args)
        .output()
        .expect("run tuistory command")
}

fn snapshot(session: &str, extra: &[&str]) -> String {
    let mut args = vec!["snapshot", "--immediate"];
    args.extend_from_slice(extra);
    let output = tuistory(session, &args);
    assert!(
        output.status.success(),
        "snapshot failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn has_visible_cells(filtered_snapshot: &str) -> bool {
    filtered_snapshot
        .chars()
        .any(|ch| !ch.is_whitespace() && ch != '█' && ch != '▏')
}

fn wait_for_contains(session: &str, needle: &str, timeout: Duration) -> bool {
    let started = Instant::now();
    while started.elapsed() < timeout {
        let screen = snapshot(session, &["--trim"]);
        if screen.contains(needle) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(120));
    }
    false
}

#[test]
fn gap_tui_010_model_picker_fixture_surfaces_provider_grouping_and_metadata_labels() {
    if !tuistory_available() {
        eprintln!("skipping TUI model-picker metadata test: tuistory is unavailable");
        return;
    }

    let sandbox = Sandbox::new("val-gap-tui-010");
    write(
        sandbox.project.join("cortex.toml"),
        r#"
[provider]
disabled = ["legacy"]

[provider.fixture]
models = [{ id = "fixture/free-model", name = "Fixture Free Model", tool_call = true, context_length = 8192, max_output_tokens = 1024, cost = { input = 0, output = 0 } }]

[provider.legacy]
models = [{ id = "legacy/disabled-model", name = "Disabled Fixture Model", tool_call = true, context_length = 8192, max_output_tokens = 1024, cost = { input = 1, output = 1 } }]
"#,
    );

    let session = format!(
        "gap-tui-010-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let _guard = launch_tui_session(&sandbox, &session);

    assert!(tuistory(&session, &["type", "/models"]).status.success());
    assert!(tuistory(&session, &["press", "enter"]).status.success());

    assert!(
        wait_for_contains(&session, "(openrouter)", Duration::from_secs(20)),
        "model picker should include grouped provider headers"
    );
    assert!(
        wait_for_contains(&session, "fixture (fixture)", Duration::from_secs(20)),
        "fixture provider group should be visible in the picker"
    );
    assert!(
        wait_for_contains(&session, "legacy (legacy)", Duration::from_secs(20)),
        "disabled provider group should still be visible in the picker"
    );
    assert!(
        wait_for_contains(&session, "[current]", Duration::from_secs(20)),
        "picker should mark the currently selected model"
    );
    assert!(
        wait_for_contains(
            &session,
            "fixture/free-model [free]",
            Duration::from_secs(20)
        ),
        "fixture should expose a zero-cost model with the free marker"
    );
    assert!(
        wait_for_contains(
            &session,
            "legacy/disabled-model [unavailable]",
            Duration::from_secs(20)
        ),
        "disabled provider model should carry the unavailable label"
    );
}

#[test]
fn gap_tui_011_custom_json_theme_preview_cancel_accept_and_persist() {
    if !tuistory_available() {
        eprintln!("skipping TUI theme gap test: tuistory is unavailable");
        return;
    }

    let sandbox = Sandbox::new("val-gap-tui-011");
    write(
        sandbox.project.join("custom-theme.json"),
        r##"
{
  "name": "dark_plus",
  "extends": "dark",
  "primary": "#ff00ff",
  "surface": ["#121212", "#1a1a1a", "#242424", "#2a2a2a"]
}
"##,
    );
    write(
        sandbox.project.join("cortex.toml"),
        r#"
theme_files = ["./custom-theme.json"]
"#,
    );

    let session = format!("gap-tui-011-{}", std::process::id());
    let _guard = launch_tui_session(&sandbox, &session);

    assert!(tuistory(&session, &["type", "/themes"]).status.success());
    assert!(tuistory(&session, &["press", "enter"]).status.success());
    assert!(
        wait_for_contains(&session, "dark_plus", Duration::from_secs(20)),
        "theme picker should list custom JSON theme"
    );

    // Preview custom theme (dark -> dark_plus) and prove its custom magenta primary color appears.
    assert!(tuistory(&session, &["press", "down"]).status.success());
    let preview_magenta = snapshot(&session, &["--fg", "#ff00ff", "--immediate", "--trim"]);
    assert!(
        has_visible_cells(&preview_magenta),
        "preview should render cells with the custom primary color"
    );

    // Cancel and verify the custom-preview color disappears and is not persisted.
    assert!(tuistory(&session, &["press", "esc"]).status.success());
    let global_config = sandbox.home.join(".config/cortex/config.toml");
    let cancelled_magenta = snapshot(&session, &["--fg", "#ff00ff", "--trim"]);
    assert!(
        !has_visible_cells(&cancelled_magenta),
        "cancel should restore prior theme colors"
    );
    if let Ok(cancelled_config) = std::fs::read_to_string(&global_config) {
        assert!(
            !cancelled_config.contains("theme = \"dark_plus\""),
            "cancel should not persist the previewed theme: {cancelled_config}"
        );
    }

    // Re-open, select custom theme again, and accept to persist.
    assert!(tuistory(&session, &["type", "/themes"]).status.success());
    assert!(tuistory(&session, &["press", "enter"]).status.success());
    assert!(tuistory(&session, &["press", "down"]).status.success());
    assert!(tuistory(&session, &["press", "enter"]).status.success());
    std::thread::sleep(Duration::from_millis(200));
    drop(_guard);

    let persisted = std::fs::read_to_string(&global_config).expect("read global config");
    assert!(
        persisted.contains("theme = \"dark_plus\""),
        "accepting the picker selection should persist the theme globally: {persisted}"
    );

    // Relaunch and prove the accepted custom theme is active after restart.
    let session2 = format!("gap-tui-011-restart-{}", std::process::id());
    let _guard2 = launch_tui_session(&sandbox, &session2);
    let restart_magenta = snapshot(&session2, &["--fg", "#ff00ff", "--trim"]);
    assert!(
        has_visible_cells(&restart_magenta),
        "custom theme should remain active after restart"
    );
}

#[test]
fn gap_tui_017_attention_cues_fire_for_documented_events_and_respect_quiet() {
    if !tuistory_available() {
        eprintln!("skipping TUI attention gap test: tuistory is unavailable");
        return;
    }

    let sandbox = Sandbox::new("val-gap-tui-017");
    write(
        sandbox.project.join("cortex.toml"),
        r#"
[attention]
enabled = true
quiet = false
audio = false
visual = true
events = ["error"]
"#,
    );

    let session = format!("gap-tui-017-{}", std::process::id());
    let _guard = launch_tui_session(&sandbox, &session);
    assert!(
        tuistory(&session, &["type", "trigger an error"])
            .status
            .success()
    );
    assert!(tuistory(&session, &["press", "enter"]).status.success());
    assert!(
        wait_for_contains(&session, "attention error", Duration::from_secs(20)),
        "error turns should trigger documented visual attention cue when enabled"
    );
    drop(_guard);

    // Quiet mode suppresses the same cue.
    write(
        sandbox.project.join("cortex.toml"),
        r#"
[attention]
enabled = true
quiet = true
audio = false
visual = true
events = ["error"]
"#,
    );
    let session_quiet = format!("gap-tui-017-quiet-{}", std::process::id());
    let _guard_quiet = launch_tui_session(&sandbox, &session_quiet);
    assert!(
        tuistory(&session_quiet, &["type", "trigger another error"])
            .status
            .success()
    );
    assert!(
        tuistory(&session_quiet, &["press", "enter"])
            .status
            .success()
    );
    std::thread::sleep(Duration::from_millis(1200));
    let quiet_screen = snapshot(&session_quiet, &["--trim"]);
    assert!(
        !quiet_screen.contains("attention error"),
        "quiet mode should suppress documented attention cues"
    );
}

#[test]
fn tui_plugin_command_slot_is_exposed_for_type_tui_plugins() {
    if !tuistory_available() || !runtime_available() {
        eprintln!("skipping TUI plugin-slot proof: tuistory or JS runtime unavailable");
        return;
    }

    let sandbox = Sandbox::new("tui-plugin-slot");
    write(
        sandbox.project.join("plugin.mjs"),
        r#"
export default async function plugin() {
  return {
    async command({ register }) {
      register({ id: "slotcmd", name: "slotcmd", description: "Plugin command slot proof" });
      return [];
    }
  };
}
"#,
    );
    write(
        sandbox.project.join("cortex.toml"),
        r#"
plugin = [[ "./plugin.mjs", { type = "tui" } ]]
"#,
    );

    let session = format!("gap-tui-slot-{}", std::process::id());
    let _guard = launch_tui_session(&sandbox, &session);
    assert!(tuistory(&session, &["type", "/slot"]).status.success());
    assert!(
        wait_for_contains(&session, "slotcmd", Duration::from_secs(20)),
        "plugin command slot should appear in slash autocomplete"
    );
    assert!(tuistory(&session, &["press", "enter"]).status.success());
    assert!(
        wait_for_contains(
            &session,
            "plugin command slot `slotcmd`",
            Duration::from_secs(10)
        ),
        "dispatching plugin slot command should produce visible slot notice"
    );
}
