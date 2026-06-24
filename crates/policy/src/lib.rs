//! Project trust/policy engine: gate the mutating/exec tool surface by a project's trust level.
//!
//! A project is either **trusted** (the default) or **untrusted**. An untrusted project restricts
//! the effectful tools (`write`/`edit`/`apply_patch`/`bash`) identically across every surface by
//! contributing a highest-precedence DENY layer to the shared permission engine, so the SAME action
//! is blocked in headless `cortex run` and the interactive TUI; marking the project trusted removes
//! the restriction so the action is permitted in both (VAL-CROSS-030).
//!
//! Trust is read from the layered configuration's top-level `trust` key (`"trusted"`/`"untrusted"`),
//! so it is per-project, persistent across runs, and resolved identically by every surface (each
//! surface reads the same project config). The default — no key, or an unrecognized value — is
//! [`TrustLevel::Trusted`], preserving the existing ungated behavior.
//!
//! This crate stays thin and surface-agnostic: it answers "what trust level does this project
//! have?" and "what permission rules does that level contribute?". Surfaces install
//! [`trust_ruleset`] into the permission engine's hard-deny layer, so trust-deny is consulted
//! before saved decisions and ordinary rules (trust-deny > saved-decision > ruleset > default-ask).
//! That keeps untrusted as an absolute safety gate while still letting saved decisions apply when
//! trusted.

use permissions::{Action, Rule, Ruleset};
use settings::{ConfigError, ConfigInputs};

/// The tools an untrusted project restricts: the mutating file tools and shell execution.
pub const RESTRICTED_TOOLS: &[&str] = &["write", "edit", "apply_patch", "bash"];

/// The top-level config key naming a project's trust level.
const TRUST_KEY: &str = "trust";

/// A project's trust level, governing whether the effectful tool surface is permitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TrustLevel {
    /// The effectful tools are permitted (the default; existing behavior is unchanged).
    #[default]
    Trusted,
    /// The effectful tools (`write`/`edit`/`apply_patch`/`bash`) are denied.
    Untrusted,
}

impl TrustLevel {
    /// Parse a configured trust value (case-insensitive, surrounding whitespace ignored).
    ///
    /// `"trusted"`/`"trust"` and `"untrusted"`/`"untrust"` are recognized; any other value yields
    /// `None` (the caller defaults to [`TrustLevel::Trusted`]).
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "trusted" | "trust" => Some(Self::Trusted),
            "untrusted" | "untrust" => Some(Self::Untrusted),
            _ => None,
        }
    }

    /// The canonical lowercase token for this level.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Trusted => "trusted",
            Self::Untrusted => "untrusted",
        }
    }

    /// Whether the effectful tools are permitted under this level.
    #[must_use]
    pub const fn is_trusted(self) -> bool {
        matches!(self, Self::Trusted)
    }
}

/// The permission [`Ruleset`] a trust level contributes.
///
/// An untrusted project denies each [`RESTRICTED_TOOLS`] entry (so the mutating/exec action is
/// blocked, identically in every surface); a trusted project contributes an empty ruleset.
/// Surfaces install these rules as hard denies.
#[must_use]
pub fn trust_ruleset(level: TrustLevel) -> Ruleset {
    match level {
        TrustLevel::Trusted => Ruleset::empty(),
        TrustLevel::Untrusted => Ruleset::new(
            RESTRICTED_TOOLS
                .iter()
                .map(|tool| Rule::new(*tool, "*", Action::Deny))
                .collect(),
        ),
    }
}

/// Resolve a project's trust level from its layered configuration (the top-level `trust` key),
/// defaulting to [`TrustLevel::Trusted`] when unset or unrecognized.
///
/// # Errors
/// Returns the underlying [`ConfigError`] when a configuration layer cannot be loaded (bad TOML,
/// failed `{env:}`/`{file:}` substitution, or a type mismatch) — the same failure the surfaces
/// already surface when resolving config.
pub fn resolve_trust(inputs: &ConfigInputs) -> Result<TrustLevel, ConfigError> {
    let effective = settings::load(inputs.clone())?;
    Ok(effective
        .get(TRUST_KEY)
        .and_then(|value| value.as_str())
        .and_then(TrustLevel::parse)
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use permissions::{
        Decision, Outcome, PermissionEngine, PermissionRequest, SavedDecision, SavedDecisions,
    };
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A self-cleaning scratch tree with an isolated HOME and a project dir inside it.
    struct Scratch {
        root: PathBuf,
    }

    impl Scratch {
        fn new(tag: &str) -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let root = std::env::temp_dir().join(format!(
                "cortex-policy-{tag}-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(root.join("home/proj")).expect("create project tree");
            Self { root }
        }

        fn write_project(&self, contents: &str) {
            fs::write(self.root.join("home/proj/cortex.toml"), contents).expect("write config");
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

    #[test]
    fn parse_is_case_insensitive_and_defaults_otherwise() {
        assert_eq!(TrustLevel::parse("trusted"), Some(TrustLevel::Trusted));
        assert_eq!(
            TrustLevel::parse("  UNTRUSTED "),
            Some(TrustLevel::Untrusted)
        );
        assert_eq!(TrustLevel::parse("Trust"), Some(TrustLevel::Trusted));
        assert_eq!(TrustLevel::parse("maybe"), None);
        assert!(TrustLevel::default().is_trusted());
    }

    #[test]
    fn trusted_contributes_no_rules() {
        assert!(trust_ruleset(TrustLevel::Trusted).is_empty());
    }

    // An untrusted project denies every restricted tool, regardless of subject.
    #[test]
    fn untrusted_denies_every_restricted_tool() {
        let ruleset = trust_ruleset(TrustLevel::Untrusted);
        for tool in RESTRICTED_TOOLS {
            assert_eq!(
                ruleset.classify(tool, "anything"),
                Action::Deny,
                "untrusted must deny {tool}"
            );
        }
        // A non-restricted tool (e.g. read) is not denied by the trust layer alone.
        assert_eq!(ruleset.classify("read", "src/a.txt"), Action::Ask);
    }

    // A trust `deny` wins over a project allow when layered on top (last-matching-rule-wins), so an
    // untrusted project blocks a mutating action even if the project config allowed it.
    #[test]
    fn trust_deny_overrides_a_project_allow_when_layered_on_top() {
        let project = Ruleset::new(vec![Rule::new("bash", "*", Action::Allow)]);
        let combined = Ruleset::from_layers([project, trust_ruleset(TrustLevel::Untrusted)]);
        assert_eq!(combined.classify("bash", "echo hi"), Action::Deny);
    }

    #[test]
    fn untrusted_trust_denies_outrank_saved_allow_when_installed_as_hard_denies() {
        let saved = Arc::new(SavedDecisions::from_decisions(vec![SavedDecision::new(
            "bash",
            "*",
            Decision::Allow,
        )]));
        let engine = PermissionEngine::new(Ruleset::new(vec![Rule::new("bash", "*", Action::Ask)]))
            .with_saved_decisions(saved)
            .with_hard_denies(trust_ruleset(TrustLevel::Untrusted));

        assert_eq!(engine.classify("bash", "echo hi"), Action::Deny);
        assert_eq!(
            engine.evaluate(&PermissionRequest::new("bash", "echo hi")),
            Outcome::Denied
        );
    }

    #[test]
    fn trusted_trust_level_keeps_saved_allow_behavior() {
        let saved = Arc::new(SavedDecisions::from_decisions(vec![SavedDecision::new(
            "bash",
            "*",
            Decision::Allow,
        )]));
        let engine = PermissionEngine::new(Ruleset::new(vec![Rule::new("bash", "*", Action::Ask)]))
            .with_saved_decisions(saved)
            .with_hard_denies(trust_ruleset(TrustLevel::Trusted));

        assert_eq!(engine.classify("bash", "echo hi"), Action::Allow);
        assert_eq!(
            engine.evaluate(&PermissionRequest::new("bash", "echo hi")),
            Outcome::Allowed
        );
    }

    #[test]
    fn resolve_trust_defaults_to_trusted_with_no_config() {
        let scratch = Scratch::new("default");
        assert_eq!(
            resolve_trust(&scratch.inputs()).expect("resolve"),
            TrustLevel::Trusted
        );
    }

    #[test]
    fn resolve_trust_reads_untrusted_from_project_config() {
        let scratch = Scratch::new("untrusted");
        scratch.write_project("trust = \"untrusted\"\n");
        assert_eq!(
            resolve_trust(&scratch.inputs()).expect("resolve"),
            TrustLevel::Untrusted
        );
    }

    #[test]
    fn resolve_trust_reads_trusted_back_from_project_config() {
        let scratch = Scratch::new("trusted");
        scratch.write_project("trust = \"trusted\"\n");
        assert_eq!(
            resolve_trust(&scratch.inputs()).expect("resolve"),
            TrustLevel::Trusted
        );
    }
}
