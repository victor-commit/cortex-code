//! Loading permission rules from the layered TOML configuration.
//!
//! Rules are declared as an array of tables under the `permission` key, in evaluation order:
//!
//! ```toml
//! [[permission]]
//! tool = "bash"      # the gated tool, or "*" for any tool (default "*")
//! pattern = "rm *"   # glob matched against the action's subject (default "*")
//! action = "deny"    # one of allow | ask | deny (required)
//! ```
//!
//! Within a single config file the rules keep their declared order. Across config layers the
//! per-layer rule lists are concatenated in ascending precedence order (global, project, nested)
//! by [`load_layered`], so under last-matching-rule-wins a project-layer rule overrides a
//! conflicting global-layer rule deterministically (VAL-PERM-017).

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde_json::Value as JsonValue;
use settings::{ConfigError, ConfigInputs};
use toml::Value;
use toml::value::Table;

use crate::action::Action;
use crate::agent::AgentRulesets;
use crate::rule::Rule;
use crate::ruleset::Ruleset;

/// The config key holding the ordered array of permission rule tables.
const PERMISSION_KEY: &str = "permission";

/// The config key holding the per-agent permission override tables.
const AGENT_KEY: &str = "agent";
const LEGACY_ENV_PERMISSION: &str = concat!("OPEN", "CODE_PERMISSION");
const CORTEX_ENV_PERMISSION: &str = "CORTEX_PERMISSION";
const PERMISSION_ENV_PRECEDENCE: &[&str] = &[LEGACY_ENV_PERMISSION, CORTEX_ENV_PERMISSION];

/// A failure loading or parsing permission rules from configuration.
#[derive(Debug)]
pub enum RuleError {
    /// A config layer could not be read or parsed (propagated from `settings`).
    Config(ConfigError),
    /// The `permission` config value was malformed (wrong shape, missing/invalid fields).
    Malformed {
        /// The config file the malformed rule came from, when known.
        source: Option<PathBuf>,
        /// A human-readable description of what was wrong.
        message: String,
    },
}

impl RuleError {
    fn malformed(source: Option<&Path>, message: impl Into<String>) -> Self {
        Self::Malformed {
            source: source.map(Path::to_path_buf),
            message: message.into(),
        }
    }
}

impl From<ConfigError> for RuleError {
    fn from(error: ConfigError) -> Self {
        Self::Config(error)
    }
}

impl fmt::Display for RuleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(error) => write!(formatter, "{error}"),
            Self::Malformed {
                source: Some(path),
                message,
            } => write!(
                formatter,
                "invalid permission rule in {}: {message}",
                path.display()
            ),
            Self::Malformed {
                source: None,
                message,
            } => write!(formatter, "invalid permission rule: {message}"),
        }
    }
}

impl std::error::Error for RuleError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Config(error) => Some(error),
            Self::Malformed { .. } => None,
        }
    }
}

/// Parse the ordered permission rules declared in a single config `table`.
///
/// An absent `permission` key yields an empty list. `source` (if known) is woven into any
/// diagnostic so a malformed rule names its file.
///
/// # Errors
/// Returns [`RuleError::Malformed`] when `permission` is not an array of tables, or an entry is
/// missing a required field or carries a wrongly-typed/invalid value.
pub fn rules_from_table(table: &Table, source: Option<&Path>) -> Result<Vec<Rule>, RuleError> {
    let Some(value) = table.get(PERMISSION_KEY) else {
        return Ok(Vec::new());
    };
    let entries = value.as_array().ok_or_else(|| {
        RuleError::malformed(
            source,
            format!("`{PERMISSION_KEY}` must be an array of rule tables"),
        )
    })?;

    let mut rules = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        rules.push(rule_from_entry(entry, index, source)?);
    }
    Ok(rules)
}

/// Parse one `[[permission]]` entry into a [`Rule`].
fn rule_from_entry(entry: &Value, index: usize, source: Option<&Path>) -> Result<Rule, RuleError> {
    let table = entry.as_table().ok_or_else(|| {
        RuleError::malformed(source, format!("rule #{} must be a table", index + 1))
    })?;

    let tool = optional_string(table, "tool", index, source)?.unwrap_or_else(|| "*".to_owned());
    let pattern =
        optional_string(table, "pattern", index, source)?.unwrap_or_else(|| "*".to_owned());

    let action_str = optional_string(table, "action", index, source)?.ok_or_else(|| {
        RuleError::malformed(
            source,
            format!("rule #{} is missing the required `action` field", index + 1),
        )
    })?;
    let action = Action::parse(&action_str).ok_or_else(|| {
        RuleError::malformed(
            source,
            format!(
                "rule #{} has invalid action `{action_str}` (expected allow, ask, or deny)",
                index + 1
            ),
        )
    })?;

    Ok(Rule::new(tool, pattern, action))
}

/// Read an optional string field, erroring clearly when present but not a string.
fn optional_string(
    table: &Table,
    key: &str,
    index: usize,
    source: Option<&Path>,
) -> Result<Option<String>, RuleError> {
    match table.get(key) {
        None => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(RuleError::malformed(
            source,
            format!("rule #{}: `{key}` must be a string", index + 1),
        )),
    }
}

/// Load and combine the permission ruleset from every config layer for the given inputs.
///
/// The layers are read in ascending precedence order (global, project root, nested) via
/// [`settings::layer_tables`], each layer's rules are parsed in declared order, and all are
/// concatenated into one [`Ruleset`]. Under last-matching-rule-wins this gives higher layers
/// precedence over conflicting lower ones (VAL-PERM-017). With no config present the ruleset is
/// empty (every action defaults to `ask`).
///
/// # Errors
/// Returns [`RuleError`] when a config layer cannot be read/parsed or a rule is malformed.
pub fn load_layered(inputs: &ConfigInputs) -> Result<Ruleset, RuleError> {
    let mut layers = Vec::new();
    for layer in settings::layer_tables(inputs)? {
        let rules = rules_from_table(&layer.table, Some(&layer.source))?;
        layers.push(Ruleset::new(rules));
    }
    let env_rules = rules_from_permission_env(inputs)?;
    if !env_rules.is_empty() {
        layers.push(Ruleset::new(env_rules));
    }
    Ok(Ruleset::from_layers(layers))
}

/// Load the per-agent permission rulesets from every config layer.
///
/// Each layer contributes its top-level `[[permission]]` rules to the shared **base** ruleset (the
/// rules applied to every agent) and any `[[agent.<name>.permission]]` rules to that agent's
/// override ruleset. Across layers the per-layer rule lists are concatenated in ascending
/// precedence order (global, project root, nested), so under last-matching-rule-wins a higher layer
/// overrides a conflicting lower one. The effective ruleset for an agent is its base plus its
/// overrides ([`AgentRulesets::for_agent`]); two agents whose rules differ decide the same action
/// differently (VAL-PERM-009). With no config present the result is empty (every agent defaults to
/// ask).
///
/// # Errors
/// Returns [`RuleError`] when a config layer cannot be read/parsed, the `agent` value is not a
/// table of agent tables, or a rule is malformed.
pub fn load_agent_rulesets(inputs: &ConfigInputs) -> Result<AgentRulesets, RuleError> {
    let mut base_layers: Vec<Ruleset> = Vec::new();
    let mut agent_layers: BTreeMap<String, Vec<Ruleset>> = BTreeMap::new();

    for layer in settings::layer_tables(inputs)? {
        let source = Some(layer.source.as_path());

        let base_rules = rules_from_table(&layer.table, source)?;
        base_layers.push(Ruleset::new(base_rules));

        if let Some(agent_value) = layer.table.get(AGENT_KEY) {
            let Some(agent_table) = agent_value.as_table() else {
                if agent_value.as_str().is_some() {
                    continue;
                }
                return Err(RuleError::malformed(
                    source,
                    format!("`{AGENT_KEY}` must be a table of agents or a default-agent string"),
                ));
            };
            for (name, value) in agent_table {
                let table = value.as_table().ok_or_else(|| {
                    RuleError::malformed(source, format!("`{AGENT_KEY}.{name}` must be a table"))
                })?;
                let rules = rules_from_table(table, source)?;
                agent_layers
                    .entry(name.clone())
                    .or_default()
                    .push(Ruleset::new(rules));
            }
        }
    }
    let env_rules = rules_from_permission_env(inputs)?;
    if !env_rules.is_empty() {
        base_layers.push(Ruleset::new(env_rules));
    }

    let base = Ruleset::from_layers(base_layers);
    let per_agent = agent_layers
        .into_iter()
        .map(|(name, layers)| (name, Ruleset::from_layers(layers)))
        .collect();
    Ok(AgentRulesets::new(base, per_agent))
}

fn rules_from_permission_env(inputs: &ConfigInputs) -> Result<Vec<Rule>, RuleError> {
    let mut rules = Vec::new();
    for env_name in PERMISSION_ENV_PRECEDENCE {
        let Some(value) = env_value(inputs, env_name) else {
            continue;
        };
        if value.trim().is_empty() {
            continue;
        }
        rules.extend(parse_permission_env_value(env_name, value)?);
    }
    Ok(rules)
}

fn env_value<'a>(inputs: &'a ConfigInputs, env_name: &str) -> Option<&'a str> {
    inputs.env.iter().rev().find_map(|(name, value)| {
        if name == env_name {
            Some(value.as_str())
        } else {
            None
        }
    })
}

fn parse_permission_env_value(env_name: &str, raw: &str) -> Result<Vec<Rule>, RuleError> {
    if let Ok(parsed) = serde_json::from_str::<JsonValue>(raw) {
        return parse_legacy_permission_json(env_name, &parsed);
    }
    if let Ok(table) = toml::from_str::<Table>(raw) {
        if table.contains_key(PERMISSION_KEY) {
            return rules_from_table(&table, None);
        }
    }
    Err(RuleError::malformed(
        None,
        format!(
            "{env_name} must be JSON permission object/map or TOML containing `[[permission]]`"
        ),
    ))
}

fn parse_legacy_permission_json(env_name: &str, value: &JsonValue) -> Result<Vec<Rule>, RuleError> {
    let map = value.as_object().ok_or_else(|| {
        RuleError::malformed(
            None,
            format!("{env_name} must decode to a JSON object at the top level"),
        )
    })?;
    let mut rules = Vec::new();
    for (key, entry) in map {
        if key == "tools" {
            let tool_map = entry.as_object().ok_or_else(|| {
                RuleError::malformed(
                    None,
                    format!("{env_name}.tools must be a JSON object of bool values"),
                )
            })?;
            for (tool, enabled) in tool_map {
                let enabled = enabled.as_bool().ok_or_else(|| {
                    RuleError::malformed(None, format!("{env_name}.tools.{tool} must be a boolean"))
                })?;
                rules.push(Rule::new(
                    tool.clone(),
                    "*",
                    if enabled { Action::Allow } else { Action::Deny },
                ));
            }
            continue;
        }
        parse_legacy_capability_entry(env_name, key, entry, &mut rules)?;
    }
    Ok(rules)
}

fn parse_legacy_capability_entry(
    env_name: &str,
    capability: &str,
    entry: &JsonValue,
    rules: &mut Vec<Rule>,
) -> Result<(), RuleError> {
    if let Some(action) = parse_action_json(entry)? {
        rules.push(Rule::new(capability, "*", action));
        return Ok(());
    }
    let Some(map) = entry.as_object() else {
        return Err(RuleError::malformed(
            None,
            format!(
                "{env_name}.{capability} must be an action (allow/ask/deny), bool, or object map"
            ),
        ));
    };
    for (pattern, value) in map {
        let action = parse_action_json(value)?.ok_or_else(|| {
            RuleError::malformed(
                None,
                format!("{env_name}.{capability}.{pattern} must be allow/ask/deny or bool"),
            )
        })?;
        rules.push(Rule::new(capability, pattern, action));
    }
    Ok(())
}

fn parse_action_json(value: &JsonValue) -> Result<Option<Action>, RuleError> {
    if let Some(enabled) = value.as_bool() {
        return Ok(Some(if enabled { Action::Allow } else { Action::Deny }));
    }
    if let Some(text) = value.as_str() {
        let action = Action::parse(text).ok_or_else(|| {
            RuleError::malformed(
                None,
                format!("invalid action `{text}` (expected allow, ask, deny, or bool)"),
            )
        })?;
        return Ok(Some(action));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::fs;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn table(toml: &str) -> Table {
        toml::from_str(toml).expect("valid toml fixture")
    }

    fn scratch_dir(tag: &str) -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "cortex-perm-config-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    fn inputs_with_env(env: Vec<(String, String)>) -> ConfigInputs {
        let cwd = scratch_dir("env");
        ConfigInputs {
            home: Some(cwd.clone()),
            xdg_config_home: None,
            cwd,
            env,
            cli_overrides: BTreeMap::new(),
        }
    }

    #[test]
    fn absent_permission_key_yields_no_rules() {
        let rules = rules_from_table(&table(r#"model = "x""#), None).expect("parse");
        assert!(rules.is_empty());
    }

    #[test]
    fn parses_ordered_rules_with_defaults() {
        let rules = rules_from_table(
            &table(
                r#"
                [[permission]]
                tool = "bash"
                pattern = "*"
                action = "allow"

                [[permission]]
                tool = "bash"
                pattern = "rm *"
                action = "deny"

                [[permission]]
                action = "ask"
                "#,
            ),
            None,
        )
        .expect("parse");

        assert_eq!(rules.len(), 3);
        assert_eq!(rules[0], Rule::new("bash", "*", Action::Allow));
        assert_eq!(rules[1], Rule::new("bash", "rm *", Action::Deny));
        // Omitted tool/pattern default to the `*` wildcard.
        assert_eq!(rules[2], Rule::new("*", "*", Action::Ask));
    }

    #[test]
    fn rejects_non_array_permission() {
        let err = rules_from_table(&table(r#"permission = "allow""#), None).unwrap_err();
        assert!(err.to_string().contains("array of rule tables"), "{err}");
    }

    #[test]
    fn rejects_missing_action() {
        let err = rules_from_table(
            &table(
                r#"
                [[permission]]
                tool = "bash"
                "#,
            ),
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("action"), "{err}");
    }

    #[test]
    fn rejects_invalid_action_value() {
        let err = rules_from_table(
            &table(
                r#"
                [[permission]]
                action = "permit"
                "#,
            ),
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("invalid action"), "{err}");
    }

    #[test]
    fn rejects_wrongly_typed_field() {
        let err = rules_from_table(
            &table(
                r#"
                [[permission]]
                tool = 7
                action = "allow"
                "#,
            ),
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("must be a string"), "{err}");
    }

    #[test]
    fn permission_env_json_supports_capabilities_and_tools_bool_map() {
        let inputs = inputs_with_env(vec![(
            LEGACY_ENV_PERMISSION.to_owned(),
            r#"{
              "edit": {"src/**":"allow","secrets/**":"deny"},
              "bash": "deny",
              "tools": {"question": false, "skill": true}
            }"#
            .to_owned(),
        )]);
        let mut rules = rules_from_permission_env(&inputs).expect("env parse");
        rules.sort_by(|left, right| {
            (
                left.tool.as_str(),
                left.pattern.as_str(),
                left.action.as_str(),
            )
                .cmp(&(
                    right.tool.as_str(),
                    right.pattern.as_str(),
                    right.action.as_str(),
                ))
        });
        let mut expected = vec![
            Rule::new("bash", "*", Action::Deny),
            Rule::new("edit", "secrets/**", Action::Deny),
            Rule::new("edit", "src/**", Action::Allow),
            Rule::new("question", "*", Action::Deny),
            Rule::new("skill", "*", Action::Allow),
        ];
        expected.sort_by(|left, right| {
            (
                left.tool.as_str(),
                left.pattern.as_str(),
                left.action.as_str(),
            )
                .cmp(&(
                    right.tool.as_str(),
                    right.pattern.as_str(),
                    right.action.as_str(),
                ))
        });
        assert_eq!(rules, expected);
    }

    #[test]
    fn permission_env_toml_supports_permission_array() {
        let inputs = inputs_with_env(vec![(
            "CORTEX_PERMISSION".to_owned(),
            r#"
[[permission]]
tool = "bash"
pattern = "git push*"
action = "deny"

[[permission]]
tool = "edit"
pattern = "src/**"
action = "allow"
"#
            .to_owned(),
        )]);
        let rules = rules_from_permission_env(&inputs).expect("env parse");
        assert_eq!(
            rules,
            vec![
                Rule::new("bash", "git push*", Action::Deny),
                Rule::new("edit", "src/**", Action::Allow),
            ]
        );
    }

    #[test]
    fn invalid_permission_env_errors_without_echoing_secret_payload() {
        let secret = "dont-leak-this-token";
        let inputs = inputs_with_env(vec![(
            "CORTEX_PERMISSION".to_owned(),
            format!(r#"not-valid-json-{secret}"#),
        )]);
        let err = rules_from_permission_env(&inputs).expect_err("invalid env should fail");
        let message = err.to_string();
        assert!(
            !message.contains(secret),
            "error should not echo secret-bearing payload: {message}"
        );
        assert!(message.contains("CORTEX_PERMISSION"), "{message}");
    }

    #[test]
    fn env_rules_are_loaded_as_highest_precedence_layer() {
        let root = scratch_dir("precedence");
        let home = root.join("home");
        let project = home.join("project");
        fs::create_dir_all(project.join(".config/cortex")).expect("dirs");
        fs::create_dir_all(&project).expect("project dir");
        fs::write(
            project.join("cortex.toml"),
            r#"
[[permission]]
tool = "bash"
pattern = "git push*"
action = "allow"
"#,
        )
        .expect("project config");
        let inputs = ConfigInputs {
            home: Some(home.clone()),
            xdg_config_home: None,
            cwd: project,
            env: vec![(
                "CORTEX_PERMISSION".to_owned(),
                r#"{"bash":"deny"}"#.to_owned(),
            )],
            cli_overrides: BTreeMap::new(),
        };
        let ruleset = load_layered(&inputs).expect("layered rules");
        assert_eq!(ruleset.classify("bash", "git push origin"), Action::Deny);
    }

    #[test]
    fn dual_permission_env_precedence_is_deterministic() {
        let legacy = (
            LEGACY_ENV_PERMISSION.to_owned(),
            r#"{"bash":"allow","read":"deny"}"#.to_owned(),
        );
        let cortex = (
            CORTEX_ENV_PERMISSION.to_owned(),
            r#"{"bash":"deny"}"#.to_owned(),
        );

        let ordered =
            rules_from_permission_env(&inputs_with_env(vec![legacy.clone(), cortex.clone()]))
                .expect("env parse");
        let reversed =
            rules_from_permission_env(&inputs_with_env(vec![cortex, legacy])).expect("env parse");
        assert_eq!(
            ordered, reversed,
            "env ordering should not affect effective legacy/new permission precedence"
        );

        let ruleset = Ruleset::new(ordered);
        assert_eq!(
            ruleset.classify("bash", "echo hi"),
            Action::Deny,
            "new permission env should override legacy on conflict"
        );
        assert_eq!(
            ruleset.classify("read", ".env"),
            Action::Deny,
            "legacy-only capabilities remain in effect when not overridden"
        );
    }
}
