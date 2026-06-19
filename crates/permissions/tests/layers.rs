//! Integration coverage for combining permission rules across config layers (VAL-PERM-017).
//!
//! Exercises the public `load_layered` API against real `cortex.toml` files on disk: a project
//! layer deterministically overrides a conflicting global layer, and removing the project rule
//! restores the global outcome. The layering order mirrors the `settings` engine
//! (global < project < nested).

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use permissions::{Action, Outcome, PermissionEngine, PermissionRequest, load_layered};
use settings::ConfigInputs;

/// A self-cleaning scratch tree with an isolated HOME, project root, and nested CWD.
struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = format!(
            "cortex-perm-layers-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let root = std::env::temp_dir().join(unique);
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

    fn inputs(&self) -> ConfigInputs {
        ConfigInputs {
            home: Some(self.home()),
            xdg_config_home: None,
            cwd: self.nested(),
            env: Vec::new(),
            cli_overrides: std::collections::BTreeMap::new(),
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

const ALLOW_RM: &str = r#"
[[permission]]
tool = "bash"
pattern = "rm *"
action = "allow"
"#;

const DENY_RM: &str = r#"
[[permission]]
tool = "bash"
pattern = "rm *"
action = "deny"
"#;

#[test]
fn project_layer_overrides_a_conflicting_global_layer() {
    // Global allows `rm *`; project denies it. The project rule is concatenated AFTER the global
    // rule, so under last-matching-rule-wins it governs: the action is denied.
    let scratch = Scratch::new();
    scratch.write_global(ALLOW_RM);
    scratch.write_project(DENY_RM);

    let ruleset = load_layered(&scratch.inputs()).expect("load layered rules");
    assert_eq!(ruleset.classify("bash", "rm x"), Action::Deny);

    let engine = PermissionEngine::new(ruleset);
    assert_eq!(
        engine.evaluate(&PermissionRequest::new("bash", "rm x")),
        Outcome::Denied,
        "project deny wins over global allow"
    );
}

#[test]
fn removing_the_project_rule_restores_the_global_outcome() {
    // The same global config, but without the project override, falls back to the global allow —
    // proving the cross-layer combination is deterministic and a layer is not silently dropped.
    let scratch = Scratch::new();
    scratch.write_global(ALLOW_RM);

    let ruleset = load_layered(&scratch.inputs()).expect("load layered rules");
    assert_eq!(ruleset.classify("bash", "rm x"), Action::Allow);

    let engine = PermissionEngine::new(ruleset);
    assert_eq!(
        engine.evaluate(&PermissionRequest::new("bash", "rm x")),
        Outcome::Allowed,
        "with no project override the global allow governs"
    );
}

#[test]
fn outcome_is_stable_across_repeated_loads() {
    // The combination is deterministic: loading the same fixtures repeatedly yields the same
    // ordered ruleset and the same decision every time.
    let scratch = Scratch::new();
    scratch.write_global(ALLOW_RM);
    scratch.write_project(DENY_RM);

    let first = load_layered(&scratch.inputs()).expect("load");
    let second = load_layered(&scratch.inputs()).expect("reload");
    assert_eq!(first, second, "ruleset is identical across loads");
    assert_eq!(first.classify("bash", "rm x"), Action::Deny);
}

#[test]
fn no_config_layers_default_to_ask() {
    // With no config files at all the ruleset is empty and every action defaults to ask.
    let scratch = Scratch::new();
    let ruleset = load_layered(&scratch.inputs()).expect("load");
    assert!(ruleset.is_empty());
    assert_eq!(ruleset.classify("bash", "rm x"), Action::Ask);
}
