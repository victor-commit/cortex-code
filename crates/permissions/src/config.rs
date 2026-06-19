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

use std::fmt;
use std::path::{Path, PathBuf};

use settings::{ConfigError, ConfigInputs};
use toml::Value;
use toml::value::Table;

use crate::action::Action;
use crate::rule::Rule;
use crate::ruleset::Ruleset;

/// The config key holding the ordered array of permission rule tables.
const PERMISSION_KEY: &str = "permission";

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
    Ok(Ruleset::from_layers(layers))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(toml: &str) -> Table {
        toml::from_str(toml).expect("valid toml fixture")
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
}
