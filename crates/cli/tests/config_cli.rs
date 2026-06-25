//! Black-box checks for the `cortex config` reporting path and the layered-config precedence
//! engine (VAL-SKEL-018..024, 065, 066).
//!
//! Each test builds an isolated scratch tree with its own HOME, project root, and nested CWD,
//! runs the real binary with a cleared environment, and asserts on the strict JSON dump.

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;

const RUN_TIMEOUT: Duration = Duration::from_secs(10);

/// A self-cleaning scratch tree: `<root>/home` is HOME, `<root>/home/proj` the project root,
/// and `<root>/home/proj/sub` the nested invocation directory.
struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = format!(
            "cortex-config-cli-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let root = std::env::temp_dir().join(unique);
        fs::create_dir_all(root.join("home/proj/sub")).expect("create scratch tree");
        Self { root }
    }

    fn home(&self) -> PathBuf {
        self.root.join("home")
    }

    fn nested(&self) -> PathBuf {
        self.root.join("home/proj/sub")
    }

    fn write(&self, rel: &str, contents: &str) {
        let path = self.root.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent dir");
        }
        fs::write(path, contents).expect("write config file");
    }

    fn write_global(&self, contents: &str) {
        self.write("home/.config/cortex/config.toml", contents);
    }

    fn write_project(&self, contents: &str) {
        self.write("home/proj/cortex.toml", contents);
    }

    fn write_nested(&self, contents: &str) {
        self.write("home/proj/sub/cortex.toml", contents);
    }

    /// Run `cortex config --format json` from the nested CWD with a cleared environment plus the
    /// given `CORTEX_*` vars and `--model` CLI override, returning the parsed JSON.
    fn report(&self, env: &[(&str, &str)], model: Option<&str>) -> Value {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command.env_clear();
        command.env("HOME", self.home());
        for (key, value) in env {
            command.env(key, value);
        }
        command.current_dir(self.nested());
        command.arg("config").arg("--format").arg("json");
        if let Some(model) = model {
            command.arg("--model").arg(model);
        }

        let output = run(command);
        assert_eq!(
            output.status.code(),
            Some(0),
            "`cortex config` must exit 0; stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap_or_else(|err| {
            panic!(
                "config report must be a single strict JSON document: {err}; stdout: {}",
                String::from_utf8_lossy(&output.stdout)
            )
        })
    }

    /// The effective `model` value reported from the nested CWD.
    fn model(&self, env: &[(&str, &str)], model: Option<&str>) -> String {
        self.report(env, model)["model"]
            .as_str()
            .expect("model is a string")
            .to_owned()
    }

    /// Run `cortex config --format json` from the nested CWD with a cleared environment plus the
    /// given `CORTEX_*`/substitution vars, returning the raw process output (no exit assertion,
    /// so error paths can be inspected).
    fn raw(&self, env: &[(&str, &str)]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command.env_clear();
        command.env("HOME", self.home());
        for (key, value) in env {
            command.env(key, value);
        }
        command.current_dir(self.nested());
        command.args(["config", "--format", "json"]);
        run(command)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// Run a command with a hard timeout, draining stdout/stderr on dedicated threads so an
/// undrained pipe can never deadlock the child.
fn run(mut command: Command) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().expect("spawn cortex binary");
    let mut out = child.stdout.take().expect("stdout handle");
    let mut err = child.stderr.take().expect("stderr handle");
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out.read_to_end(&mut buf);
        buf
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err.read_to_end(&mut buf);
        buf
    });

    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            break status;
        }
        if start.elapsed() > RUN_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("cortex config did not terminate within {RUN_TIMEOUT:?} (possible hang)");
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: out_thread.join().expect("join stdout reader"),
        stderr: err_thread.join().expect("join stderr reader"),
    }
}

// VAL-SKEL-018: built-in defaults are observable with no config files present.
#[test]
fn defaults_observable_without_config_files() {
    let scratch = Scratch::new();
    assert_eq!(scratch.model(&[], None), "google/gemini-2.5-flash");
}

// VAL-SKEL-019: a value set only in global overrides the default.
#[test]
fn global_overrides_default() {
    let scratch = Scratch::new();
    scratch.write_global(r#"model = "from-global""#);
    assert_eq!(scratch.model(&[], None), "from-global");
}

// VAL-SKEL-020: a project-root value overrides global.
#[test]
fn project_overrides_global() {
    let scratch = Scratch::new();
    scratch.write_global(r#"model = "from-global""#);
    scratch.write_project(r#"model = "from-project""#);
    assert_eq!(scratch.model(&[], None), "from-project");
}

// VAL-SKEL-021: a nested-dir value (closer to the CWD) overrides the project root.
#[test]
fn nested_overrides_project() {
    let scratch = Scratch::new();
    scratch.write_project(r#"model = "from-project""#);
    scratch.write_nested(r#"model = "from-nested""#);
    assert_eq!(scratch.model(&[], None), "from-nested");
}

// VAL-SKEL-022: an env var (CORTEX_MODEL) overrides all file layers.
#[test]
fn env_overrides_files() {
    let scratch = Scratch::new();
    scratch.write_project(r#"model = "from-project""#);
    scratch.write_nested(r#"model = "from-nested""#);
    assert_eq!(
        scratch.model(&[("CORTEX_MODEL", "from-env")], None),
        "from-env"
    );
}

// VAL-SKEL-023: a CLI flag (--model) overrides the env var and all file layers.
#[test]
fn cli_overrides_env_and_files() {
    let scratch = Scratch::new();
    scratch.write_nested(r#"model = "from-nested""#);
    assert_eq!(
        scratch.model(&[("CORTEX_MODEL", "from-env")], Some("from-cli")),
        "from-cli"
    );
}

// VAL-SKEL-024: the full chain is observable end-to-end as each layer is toggled on.
#[test]
fn full_precedence_chain_each_layer_wins_in_turn() {
    let scratch = Scratch::new();
    assert_eq!(scratch.model(&[], None), "google/gemini-2.5-flash");

    scratch.write_global(r#"model = "L-global""#);
    assert_eq!(scratch.model(&[], None), "L-global");

    scratch.write_project(r#"model = "L-project""#);
    assert_eq!(scratch.model(&[], None), "L-project");

    scratch.write_nested(r#"model = "L-nested""#);
    assert_eq!(scratch.model(&[], None), "L-nested");

    assert_eq!(scratch.model(&[("CORTEX_MODEL", "L-env")], None), "L-env");

    assert_eq!(
        scratch.model(&[("CORTEX_MODEL", "L-env")], Some("L-cli")),
        "L-cli"
    );
}

// VAL-SKEL-065: layers deep-merge; key A only in global and key B only in project both survive.
#[test]
fn deep_merge_exposes_keys_from_multiple_layers() {
    let scratch = Scratch::new();
    scratch.write_global(r#"alpha = "from-global""#);
    scratch.write_project(r#"beta = "from-project""#);

    let report = scratch.report(&[], None);
    assert_eq!(report["alpha"], "from-global");
    assert_eq!(report["beta"], "from-project");
    assert_eq!(report["model"], "google/gemini-2.5-flash");
}

// VAL-SKEL-066: an empty / whitespace / comment-only config file is a clean no-op (exit 0).
#[test]
fn empty_config_file_is_a_clean_no_op() {
    for body in ["", "   \n\t\n", "# only comments\n\n  # more\n"] {
        let scratch = Scratch::new();
        scratch.write_project(body);
        assert_eq!(
            scratch.model(&[], None),
            "google/gemini-2.5-flash",
            "defaults must survive an empty config body {body:?}"
        );
    }
}

// The JSON report is strict and machine-parseable with no chatter on stdout.
#[test]
fn json_report_is_strict_and_clean() {
    let scratch = Scratch::new();
    scratch.write_global(r#"model = "x""#);

    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.env_clear();
    command.env("HOME", scratch.home());
    command.current_dir(scratch.nested());
    command.args(["config", "--format", "json"]);
    let output = run(command);

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    assert_eq!(
        stdout.trim_end_matches('\n').lines().count(),
        1,
        "exactly one JSON document, got: {stdout:?}"
    );
    let _: Value = serde_json::from_str(stdout.trim_end()).expect("stdout is exclusively JSON");
    assert!(
        !stdout.contains('\u{1b}'),
        "machine output must carry no ANSI escapes"
    );
    assert!(
        output.stderr.is_empty(),
        "no chatter expected on a clean config run, got: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// Invalid TOML in a present config file fails with a clear, located diagnostic (Exit::Config = 5),
// naming the file on stderr with stdout empty and no panic.
#[test]
fn invalid_toml_yields_a_clear_config_error() {
    let scratch = Scratch::new();
    scratch.write_project("model = \n");

    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.env_clear();
    command.env("HOME", scratch.home());
    command.current_dir(scratch.nested());
    command.args(["config", "--format", "json"]);
    let output = run(command);

    assert_eq!(
        output.status.code(),
        Some(5),
        "invalid config exits with the stable Config code"
    );
    assert!(output.stdout.is_empty(), "stdout must stay empty on error");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cortex.toml"),
        "stderr must name the offending file, got: {stderr}"
    );
    assert!(
        !stderr.contains("panicked"),
        "no panic backtrace allowed, got: {stderr}"
    );
}

// VAL-SKEL-025: `{env:SOME_VAR}` resolves to the variable's value in the reported config.
#[test]
fn env_substitution_resolves_in_report() {
    let scratch = Scratch::new();
    scratch.write_project(r#"secret = "{env:SOME_VAR}""#);
    let report = scratch.report(&[("SOME_VAR", "resolved")], None);
    assert_eq!(report["secret"], "resolved");
}

// VAL-SKEL-026: an unset `{env:MISSING_VAR}` yields a clear stderr diagnostic naming the
// variable, exits non-zero (Config = 5), keeps stdout empty, and never leaks the literal token.
#[test]
fn missing_env_var_yields_a_clear_diagnostic() {
    let scratch = Scratch::new();
    scratch.write_project(r#"secret = "{env:MISSING_VAR}""#);
    let output = scratch.raw(&[]);

    assert_eq!(
        output.status.code(),
        Some(5),
        "missing env exits Config code"
    );
    assert!(output.stdout.is_empty(), "stdout must stay empty on error");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("MISSING_VAR"),
        "stderr must name the variable, got: {stderr}"
    );
    assert!(
        !stderr.contains("{env:MISSING_VAR}"),
        "the literal token must never leak, got: {stderr}"
    );
    assert!(
        !stderr.contains("panicked"),
        "no panic backtrace, got: {stderr}"
    );
}

// VAL-SKEL-027: `{file:./secret.txt}` resolves to the file's trimmed contents.
#[test]
fn file_substitution_resolves_in_report() {
    let scratch = Scratch::new();
    scratch.write("home/proj/secret.txt", "  file-secret\n\n");
    scratch.write_project(r#"secret = "{file:./secret.txt}""#);
    let report = scratch.report(&[], None);
    assert_eq!(report["secret"], "file-secret");
}

// VAL-SKEL-028: a missing `{file:./nope.txt}` yields a clear stderr diagnostic naming the path,
// exits non-zero, keeps stdout empty, and never leaks the literal token.
#[test]
fn missing_file_yields_a_clear_diagnostic() {
    let scratch = Scratch::new();
    scratch.write_project(r#"secret = "{file:./nope.txt}""#);
    let output = scratch.raw(&[]);

    assert_eq!(
        output.status.code(),
        Some(5),
        "missing file exits Config code"
    );
    assert!(output.stdout.is_empty(), "stdout must stay empty on error");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("./nope.txt"),
        "stderr must name the missing path, got: {stderr}"
    );
    assert!(
        !stderr.contains("{file:./nope.txt}"),
        "the literal token must never leak, got: {stderr}"
    );
    assert!(
        !stderr.contains("panicked"),
        "no panic backtrace, got: {stderr}"
    );
}

// VAL-SKEL-030: a wrongly-typed value yields a clear stderr diagnostic naming the offending key
// and the expected type, exits non-zero, and keeps stdout empty.
#[test]
fn type_mismatch_yields_a_clear_diagnostic() {
    let scratch = Scratch::new();
    scratch.write_project("model = 123");
    let output = scratch.raw(&[]);

    assert_eq!(
        output.status.code(),
        Some(5),
        "type mismatch exits Config code"
    );
    assert!(output.stdout.is_empty(), "stdout must stay empty on error");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("model"),
        "stderr must name the offending key, got: {stderr}"
    );
    assert!(
        stderr.contains("string"),
        "stderr must name the expected type, got: {stderr}"
    );
    assert!(
        !stderr.contains("panicked"),
        "no panic backtrace, got: {stderr}"
    );
}

// VAL-SKEL-029: invalid TOML names the file and locates the error (line/column).
#[test]
fn invalid_toml_diagnostic_locates_the_error() {
    let scratch = Scratch::new();
    scratch.write_project("model = \n");
    let output = scratch.raw(&[]);

    assert_eq!(output.status.code(), Some(5));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cortex.toml"),
        "names the file, got: {stderr}"
    );
    assert!(stderr.contains("line"), "locates the error, got: {stderr}");
    assert!(
        !stderr.contains("panicked"),
        "no panic backtrace, got: {stderr}"
    );
}
