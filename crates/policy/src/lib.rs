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
use std::path::PathBuf;

/// The tools an untrusted project restricts: the mutating file tools and shell execution.
pub const RESTRICTED_TOOLS: &[&str] = &["write", "edit", "apply_patch", "bash"];

/// The top-level config key naming a project's trust level.
const TRUST_KEY: &str = "trust";
/// The top-level key holding experimental configuration.
const EXPERIMENTAL_KEY: &str = "experimental";
/// The experimental policies array key.
const EXPERIMENTAL_POLICIES_KEY: &str = "policies";
/// The policy action used to govern provider access.
pub const PROVIDER_USE_ACTION: &str = "provider.use";

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

/// A provider-policy evaluation error: either config could not be read/parsed, the policy payload
/// itself is malformed, or a provider is explicitly denied.
#[derive(Debug)]
pub enum ProviderPolicyError {
    /// The layered config could not be loaded.
    Config(ConfigError),
    /// `experimental.policies` exists but is malformed.
    Invalid {
        /// The file that carried the malformed value, when known.
        source: Option<PathBuf>,
        /// A human-readable policy parse message.
        message: String,
    },
    /// The provider is explicitly denied by policy.
    Denied {
        /// The provider id that was denied.
        provider: String,
    },
}

impl ProviderPolicyError {
    #[must_use]
    pub const fn is_config(&self) -> bool {
        matches!(self, Self::Config(_) | Self::Invalid { .. })
    }
}

impl std::fmt::Display for ProviderPolicyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Config(err) => write!(formatter, "{err}"),
            Self::Invalid { source, message } => {
                if let Some(path) = source {
                    write!(
                        formatter,
                        "invalid `experimental.policies` in {}: {message}",
                        path.display()
                    )
                } else {
                    write!(formatter, "invalid `experimental.policies`: {message}")
                }
            }
            Self::Denied { provider } => write!(
                formatter,
                "provider `{provider}` is denied by `experimental.policies` for `{PROVIDER_USE_ACTION}`"
            ),
        }
    }
}

impl std::error::Error for ProviderPolicyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Config(err) => Some(err),
            Self::Invalid { .. } | Self::Denied { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PolicyStatement {
    action: String,
    resource: String,
    effect: Action,
}

/// Resolve the ordered policy rules from `experimental.policies`.
///
/// Statements keep authored order within each file. Across files, order is reversed from the
/// ordinary settings precedence so user-global policy can override repository policy.
///
/// # Errors
/// Returns [`ProviderPolicyError::Config`] when config layers cannot be loaded and
/// [`ProviderPolicyError::Invalid`] when a policy entry is malformed.
pub fn resolve_provider_policy_rules(
    inputs: &ConfigInputs,
) -> Result<Ruleset, ProviderPolicyError> {
    let mut rules = Vec::new();
    for layer in settings::layer_tables(inputs)
        .map_err(ProviderPolicyError::Config)?
        .into_iter()
        .rev()
    {
        for statement in parse_policy_layer(&layer)? {
            rules.push(Rule::new(
                statement.action,
                statement.resource,
                statement.effect,
            ));
        }
    }
    Ok(Ruleset::new(rules))
}

/// Evaluate whether `provider_id` may be used under `experimental.policies`.
///
/// Fallback behavior is allow when no statement matches.
///
/// # Errors
/// Returns config/policy parse errors from [`resolve_provider_policy_rules`].
pub fn provider_use_action(
    inputs: &ConfigInputs,
    provider_id: &str,
) -> Result<Action, ProviderPolicyError> {
    let rules = resolve_provider_policy_rules(inputs)?;
    Ok(match rules.classify(PROVIDER_USE_ACTION, provider_id) {
        Action::Deny => Action::Deny,
        Action::Allow | Action::Ask => Action::Allow,
    })
}

/// Enforce provider governance, returning a deny error when policy blocks `provider_id`.
///
/// # Errors
/// Returns config/policy parse errors and an explicit deny when the provider is blocked.
pub fn enforce_provider_use(
    inputs: &ConfigInputs,
    provider_id: &str,
) -> Result<(), ProviderPolicyError> {
    if provider_use_action(inputs, provider_id)? == Action::Deny {
        return Err(ProviderPolicyError::Denied {
            provider: provider_id.to_owned(),
        });
    }
    Ok(())
}

fn parse_policy_layer(
    layer: &settings::ConfigLayer,
) -> Result<Vec<PolicyStatement>, ProviderPolicyError> {
    let Some(experimental) = layer.table.get(EXPERIMENTAL_KEY) else {
        return Ok(Vec::new());
    };
    let experimental = experimental
        .as_table()
        .ok_or_else(|| ProviderPolicyError::Invalid {
            source: Some(layer.source.clone()),
            message: format!("`{EXPERIMENTAL_KEY}` must be a table"),
        })?;
    let Some(policies) = experimental.get(EXPERIMENTAL_POLICIES_KEY) else {
        return Ok(Vec::new());
    };
    let policies = policies
        .as_array()
        .ok_or_else(|| ProviderPolicyError::Invalid {
            source: Some(layer.source.clone()),
            message: format!("`{EXPERIMENTAL_KEY}.{EXPERIMENTAL_POLICIES_KEY}` must be an array"),
        })?;
    let mut statements = Vec::with_capacity(policies.len());
    for (index, value) in policies.iter().enumerate() {
        let entry = value
            .as_table()
            .ok_or_else(|| ProviderPolicyError::Invalid {
                source: Some(layer.source.clone()),
                message: format!(
                    "`{EXPERIMENTAL_KEY}.{EXPERIMENTAL_POLICIES_KEY}[{index}]` must be a table"
                ),
            })?;
        let effect = non_empty_string(entry.get("effect")).ok_or_else(|| ProviderPolicyError::Invalid {
            source: Some(layer.source.clone()),
            message: format!(
                "`{EXPERIMENTAL_KEY}.{EXPERIMENTAL_POLICIES_KEY}[{index}].effect` must be a non-empty string"
            ),
        })?;
        let action = non_empty_string(entry.get("action")).ok_or_else(|| ProviderPolicyError::Invalid {
            source: Some(layer.source.clone()),
            message: format!(
                "`{EXPERIMENTAL_KEY}.{EXPERIMENTAL_POLICIES_KEY}[{index}].action` must be a non-empty string"
            ),
        })?;
        let resource = non_empty_string(entry.get("resource")).ok_or_else(|| ProviderPolicyError::Invalid {
            source: Some(layer.source.clone()),
            message: format!(
                "`{EXPERIMENTAL_KEY}.{EXPERIMENTAL_POLICIES_KEY}[{index}].resource` must be a non-empty string"
            ),
        })?;
        let effect = match effect.to_ascii_lowercase().as_str() {
            "allow" => Action::Allow,
            "deny" => Action::Deny,
            other => {
                return Err(ProviderPolicyError::Invalid {
                    source: Some(layer.source.clone()),
                    message: format!(
                        "`{EXPERIMENTAL_KEY}.{EXPERIMENTAL_POLICIES_KEY}[{index}].effect` must be `allow` or `deny`, found `{other}`"
                    ),
                });
            }
        };
        statements.push(PolicyStatement {
            action: action.to_owned(),
            resource: resource.to_owned(),
            effect,
        });
    }
    Ok(statements)
}

fn non_empty_string(value: Option<&toml::Value>) -> Option<&str> {
    let text = value?.as_str()?;
    let trimmed = text.trim();
    (!trimmed.is_empty()).then_some(trimmed)
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

        fn write_global(&self, contents: &str) {
            let dir = self.root.join("home/.config/cortex");
            fs::create_dir_all(&dir).expect("create global dir");
            fs::write(dir.join("config.toml"), contents).expect("write global config");
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

    #[test]
    fn provider_policies_default_to_allow_when_unset() {
        let scratch = Scratch::new("provider-default-allow");
        assert_eq!(
            provider_use_action(&scratch.inputs(), "openrouter").expect("policy resolve"),
            Action::Allow
        );
        assert!(enforce_provider_use(&scratch.inputs(), "openrouter").is_ok());
    }

    #[test]
    fn provider_policies_follow_last_match_wins_within_one_file() {
        let scratch = Scratch::new("provider-last-match");
        scratch.write_project(
            r#"
[experimental]
policies = [
  { effect = "deny", action = "provider.use", resource = "*" },
  { effect = "allow", action = "provider.use", resource = "openrouter" },
  { effect = "deny", action = "provider.use", resource = "openrouter" },
]
"#,
        );
        assert_eq!(
            provider_use_action(&scratch.inputs(), "openrouter").expect("policy resolve"),
            Action::Deny
        );
    }

    #[test]
    fn provider_policies_reverse_layer_order_so_global_overrides_project() {
        let scratch = Scratch::new("provider-reverse-layers");
        scratch.write_project(
            r#"
[experimental]
policies = [{ effect = "allow", action = "provider.use", resource = "openrouter" }]
"#,
        );
        scratch.write_global(
            r#"
[experimental]
policies = [{ effect = "deny", action = "provider.use", resource = "openrouter" }]
"#,
        );
        assert_eq!(
            provider_use_action(&scratch.inputs(), "openrouter").expect("policy resolve"),
            Action::Deny
        );
    }

    #[test]
    fn invalid_provider_policy_shape_is_rejected() {
        let scratch = Scratch::new("provider-invalid-shape");
        scratch.write_project(
            r#"
[experimental]
policies = "not-an-array"
"#,
        );
        let err = provider_use_action(&scratch.inputs(), "openrouter").expect_err("invalid shape");
        assert!(matches!(err, ProviderPolicyError::Invalid { .. }));
        assert!(
            err.to_string().contains("experimental.policies"),
            "message: {err}"
        );
    }

    #[test]
    fn enforce_provider_use_reports_a_denied_provider() {
        let scratch = Scratch::new("provider-denied");
        scratch.write_project(
            r#"
[experimental]
policies = [{ effect = "deny", action = "provider.use", resource = "openrouter" }]
"#,
        );
        let err = enforce_provider_use(&scratch.inputs(), "openrouter").expect_err("denied");
        assert!(matches!(err, ProviderPolicyError::Denied { .. }));
        assert!(err.to_string().contains("openrouter"), "message: {err}");
    }
}
