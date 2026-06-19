//! Integration coverage for per-agent rulesets and subagent narrowing (VAL-PERM-009, VAL-PERM-010).
//!
//! Per-agent rulesets are loaded from real `cortex.toml` files via `load_agent_rulesets`: a base
//! `[[permission]]` applies to every agent and `[[agent.<name>.permission]]` overrides it per
//! agent, so the same action is decided differently under two agents. Subagent narrowing is
//! exercised through the engine's parent cap: an action denied for the parent stays denied for a
//! spawned subagent regardless of the subagent's own ruleset.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use permissions::{
    Action, Outcome, PermissionEngine, Rule, Ruleset, load_agent_rulesets, request_for,
};
use settings::ConfigInputs;

/// A self-cleaning scratch tree with an isolated HOME and project-root config.
struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = format!(
            "cortex-perm-agents-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let root = std::env::temp_dir().join(unique);
        fs::create_dir_all(root.join("home/proj")).expect("create project tree");
        Self { root }
    }

    fn write_project(&self, contents: &str) {
        let path = self.root.join("home/proj/cortex.toml");
        fs::write(path, contents).expect("write project config");
    }

    fn inputs(&self) -> ConfigInputs {
        ConfigInputs {
            home: Some(self.root.join("home")),
            xdg_config_home: None,
            cwd: self.root.join("home/proj"),
            env: Vec::new(),
            cli_overrides: BTreeMap::new(),
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// VAL-PERM-009: per-agent rulesets apply independently — the same action is decided differently
/// under two agents whose rulesets differ.
#[test]
fn per_agent_rulesets_decide_the_same_action_differently() {
    let scratch = Scratch::new();
    // base asks for writes; plan denies all writes; build allows all writes.
    scratch.write_project(
        r#"
        [[permission]]
        tool = "write"
        pattern = "**"
        action = "ask"

        [[agent.plan.permission]]
        tool = "write"
        pattern = "**"
        action = "deny"

        [[agent.build.permission]]
        tool = "write"
        pattern = "**"
        action = "allow"
        "#,
    );

    let rulesets = load_agent_rulesets(&scratch.inputs()).expect("load per-agent rulesets");
    assert_eq!(rulesets.agent_names(), vec!["build", "plan"]);

    // The identical write target is denied under `plan` but allowed under `build`.
    let plan = PermissionEngine::new(rulesets.for_agent("plan"));
    let build = PermissionEngine::new(rulesets.for_agent("build"));
    assert_eq!(
        plan.evaluate(&request_for("write", "src/a.txt")),
        Outcome::Denied,
        "agent plan denies the write"
    );
    assert_eq!(
        build.evaluate(&request_for("write", "src/a.txt")),
        Outcome::Allowed,
        "agent build allows the same write"
    );

    // An agent with no overrides falls back to the shared base (ask → headless deny).
    let general = PermissionEngine::new(rulesets.for_agent("general"));
    assert_eq!(
        general.evaluate(&request_for("write", "src/a.txt")),
        Outcome::Denied,
        "an agent with no overrides uses the base ruleset (ask fails closed headless)"
    );
}

/// An agent's override wins over a conflicting base rule for the same pattern (last-match-wins
/// across the base + agent layering).
#[test]
fn agent_override_wins_over_the_base() {
    let scratch = Scratch::new();
    scratch.write_project(
        r#"
        [[permission]]
        tool = "bash"
        pattern = "*"
        action = "allow"

        [[agent.plan.permission]]
        tool = "bash"
        pattern = "*"
        action = "deny"
        "#,
    );
    let rulesets = load_agent_rulesets(&scratch.inputs()).expect("load");
    // base allows all bash; plan denies all bash.
    assert_eq!(rulesets.base().classify("bash", "ls"), Action::Allow);
    assert_eq!(
        rulesets.for_agent("plan").classify("bash", "ls"),
        Action::Deny
    );
    // build (no overrides) inherits the base allow.
    assert_eq!(
        rulesets.for_agent("build").classify("bash", "ls"),
        Action::Allow
    );
}

/// VAL-PERM-010: a subagent's permissions cannot exceed the spawning agent's — an action denied for
/// the parent remains denied when attempted by the subagent, regardless of the subagent's ruleset.
#[test]
fn subagent_cannot_exceed_the_parent() {
    // Parent allows bash generally but denies `rm *`.
    let parent = Ruleset::new(vec![
        Rule::new("bash", "*", Action::Allow),
        Rule::new("bash", "rm *", Action::Deny),
    ]);
    // The subagent's own ruleset would allow everything, including rm.
    let child = Ruleset::new(vec![Rule::new("bash", "*", Action::Allow)]);

    let subagent = PermissionEngine::new(child).with_parent_cap(parent);

    // The parent denies `rm *`, so the subagent's attempt to rm is still blocked.
    assert_eq!(
        subagent.evaluate(&request_for("bash", "rm victim.txt")),
        Outcome::Denied,
        "the subagent cannot run an action the parent denies"
    );
    // A command the parent allows is governed by the subagent's allow.
    assert_eq!(
        subagent.evaluate(&request_for("bash", "echo hi")),
        Outcome::Allowed,
        "the subagent may run what the parent permits"
    );
}

/// The subagent cap composes with per-agent rulesets: spawning a `build` subagent under a `plan`
/// parent still cannot write where the parent denies.
#[test]
fn subagent_cap_composes_with_per_agent_rulesets() {
    let scratch = Scratch::new();
    scratch.write_project(
        r#"
        [[agent.plan.permission]]
        tool = "write"
        pattern = "**"
        action = "deny"

        [[agent.build.permission]]
        tool = "write"
        pattern = "**"
        action = "allow"
        "#,
    );
    let rulesets = load_agent_rulesets(&scratch.inputs()).expect("load");

    // A `build` subagent (allow write **) spawned under a `plan` parent (deny write **) is capped.
    let subagent = PermissionEngine::new(rulesets.for_agent("build"))
        .with_parent_cap(rulesets.for_agent("plan"));
    assert_eq!(
        subagent.evaluate(&request_for("write", "src/a.txt")),
        Outcome::Denied,
        "a permissive subagent under a denying parent is still denied"
    );
}
