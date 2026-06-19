//! Integration coverage for the layered-config precedence engine, exercising the public
//! `load` API against real config files on disk.
//!
//! Mirrors the M0 contract: defaults < global < project < nested < env < CLI (VAL-SKEL-018..024),
//! per-key deep-merge (VAL-SKEL-065), and the empty-file clean no-op (VAL-SKEL-066).

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use cortex_config::{ConfigInputs, DEFAULT_MODEL, EffectiveConfig, load};

/// A self-cleaning scratch tree with an isolated HOME, project root, and nested CWD.
struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = format!(
            "cortex-config-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let root = std::env::temp_dir().join(unique);
        fs::create_dir_all(root.join("home")).expect("create home");
        fs::create_dir_all(root.join("home/proj/sub")).expect("create project tree");
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

    /// Inputs that run "from" the nested CWD, with the given env and CLI overrides.
    fn inputs(&self, env: &[(&str, &str)], cli: &[(&str, &str)]) -> ConfigInputs {
        ConfigInputs {
            home: Some(self.home()),
            xdg_config_home: None,
            cwd: self.nested(),
            env: env
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
            cli_overrides: cli
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        }
    }

    fn load(&self, env: &[(&str, &str)], cli: &[(&str, &str)]) -> EffectiveConfig {
        load(self.inputs(env, cli)).expect("config loads")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

// VAL-SKEL-018: built-in defaults are observable with no config files present.
#[test]
fn defaults_when_no_files_present() {
    let scratch = Scratch::new();
    let config = scratch.load(&[], &[]);
    assert_eq!(config.model(), DEFAULT_MODEL);
    assert_eq!(config.model(), "google/gemini-2.5-flash");
}

// VAL-SKEL-019: a value set only in global overrides the default.
#[test]
fn global_overrides_default() {
    let scratch = Scratch::new();
    scratch.write_global(r#"model = "from-global""#);
    assert_eq!(scratch.load(&[], &[]).model(), "from-global");
}

// VAL-SKEL-020: a project-root value overrides global.
#[test]
fn project_overrides_global() {
    let scratch = Scratch::new();
    scratch.write_global(r#"model = "from-global""#);
    scratch.write_project(r#"model = "from-project""#);
    assert_eq!(scratch.load(&[], &[]).model(), "from-project");
}

// VAL-SKEL-021: a nested-dir value (closer to the CWD) overrides the project root.
#[test]
fn nested_overrides_project() {
    let scratch = Scratch::new();
    scratch.write_project(r#"model = "from-project""#);
    scratch.write_nested(r#"model = "from-nested""#);
    assert_eq!(scratch.load(&[], &[]).model(), "from-nested");
}

// VAL-SKEL-022: an env var overrides all file layers.
#[test]
fn env_overrides_files() {
    let scratch = Scratch::new();
    scratch.write_project(r#"model = "from-project""#);
    scratch.write_nested(r#"model = "from-nested""#);
    let config = scratch.load(&[("CORTEX_MODEL", "from-env")], &[]);
    assert_eq!(config.model(), "from-env");
}

// VAL-SKEL-023: a CLI flag overrides the env var and all file layers.
#[test]
fn cli_overrides_env_and_files() {
    let scratch = Scratch::new();
    scratch.write_nested(r#"model = "from-nested""#);
    let config = scratch.load(&[("CORTEX_MODEL", "from-env")], &[("model", "from-cli")]);
    assert_eq!(config.model(), "from-cli");
}

// VAL-SKEL-024: the full precedence chain is observable as each layer is toggled on.
#[test]
fn full_precedence_chain_each_layer_wins_in_turn() {
    let scratch = Scratch::new();

    // defaults
    assert_eq!(scratch.load(&[], &[]).model(), DEFAULT_MODEL);

    // + global
    scratch.write_global(r#"model = "L-global""#);
    assert_eq!(scratch.load(&[], &[]).model(), "L-global");

    // + project
    scratch.write_project(r#"model = "L-project""#);
    assert_eq!(scratch.load(&[], &[]).model(), "L-project");

    // + nested
    scratch.write_nested(r#"model = "L-nested""#);
    assert_eq!(scratch.load(&[], &[]).model(), "L-nested");

    // + env
    assert_eq!(
        scratch.load(&[("CORTEX_MODEL", "L-env")], &[]).model(),
        "L-env"
    );

    // + CLI
    assert_eq!(
        scratch
            .load(&[("CORTEX_MODEL", "L-env")], &[("model", "L-cli")])
            .model(),
        "L-cli"
    );
}

// VAL-SKEL-065: layers deep-merge; non-overridden keys from lower layers survive.
#[test]
fn deep_merge_keeps_unrelated_keys_from_lower_layers() {
    let scratch = Scratch::new();
    scratch.write_global(r#"alpha = "from-global""#);
    scratch.write_project(r#"beta = "from-project""#);

    let config = scratch.load(&[], &[]);
    assert_eq!(
        config.get("alpha").and_then(|v| v.as_str()),
        Some("from-global")
    );
    assert_eq!(
        config.get("beta").and_then(|v| v.as_str()),
        Some("from-project")
    );
    // The default also survives alongside both file-only keys.
    assert_eq!(config.model(), DEFAULT_MODEL);
}

// VAL-SKEL-066: an empty / whitespace / comment-only config file is a clean no-op.
#[test]
fn empty_or_comment_only_file_is_a_no_op() {
    for body in ["", "   \n\t\n", "# only a comment\n\n   # another\n"] {
        let scratch = Scratch::new();
        scratch.write_project(body);
        let result = load(scratch.inputs(&[], &[]));
        let config = result.expect("empty/comment-only config must load without error");
        assert_eq!(
            config.model(),
            DEFAULT_MODEL,
            "defaults intact for body {body:?}"
        );
    }
}

// A config in a nested `.cortex/config.toml` directory is also discovered and wins over the root.
#[test]
fn dot_cortex_dir_config_is_discovered() {
    let scratch = Scratch::new();
    scratch.write_project(r#"model = "from-project""#);
    scratch.write(
        "home/proj/sub/.cortex/config.toml",
        r#"model = "from-dotcortex""#,
    );
    assert_eq!(scratch.load(&[], &[]).model(), "from-dotcortex");
}

// The JSON report is a single strict object exposing the effective values.
#[test]
fn json_report_exposes_effective_values() {
    let scratch = Scratch::new();
    scratch.write_global(r#"alpha = "a""#);
    let config = scratch.load(&[], &[("model", "cli-model")]);

    let json: serde_json::Value =
        serde_json::from_str(&config.to_json()).expect("report is valid JSON");
    assert_eq!(json["model"], "cli-model");
    assert_eq!(json["alpha"], "a");
}
