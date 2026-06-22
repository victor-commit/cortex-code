//! Lightweight type validation for the small set of known, typed config keys.
//!
//! Config is intentionally schemaless for forward-compatibility: unknown keys pass through
//! untouched (so new options need no code change to be reported). A handful of keys, however,
//! have a fixed expected type and are validated so a wrongly-typed value produces a clear,
//! located diagnostic instead of silently mis-driving the tool.
//!
//! Because the environment and CLI layers can only ever produce strings, an integer-typed key
//! accepts a string that parses as an integer (and is normalized to one); a string that is not
//! a valid integer, or any other non-integer type, is a type error. String-typed keys, by
//! contrast, only ever legitimately receive strings, so any non-string value is a type error.

use toml::Value;
use toml::value::Table;

use crate::ConfigError;

/// Validate (and where applicable normalize) the known typed keys in `table`.
///
/// # Errors
/// Returns [`ConfigError::TypeMismatch`] for a key whose value cannot be the expected type.
pub fn validate(table: &mut Table) -> Result<(), ConfigError> {
    require_string(table, "model")?;
    require_agent(table)?;
    require_integer(table, "max_steps")?;
    Ok(())
}

fn require_agent(table: &Table) -> Result<(), ConfigError> {
    match table.get("agent") {
        None | Some(Value::String(_)) | Some(Value::Table(_)) => Ok(()),
        Some(other) => Err(ConfigError::type_mismatch(
            "agent",
            "a string or table",
            other,
        )),
    }
}

/// A string-typed key must hold a TOML string. Absent is fine (defaults/lower layers cover it).
fn require_string(table: &Table, key: &str) -> Result<(), ConfigError> {
    match table.get(key) {
        None | Some(Value::String(_)) => Ok(()),
        Some(other) => Err(ConfigError::type_mismatch(key, "a string", other)),
    }
}

/// An integer-typed key holds a TOML integer, or a string that parses as one (normalized to an
/// integer in place so the reported config shows a number). Anything else is a type error.
fn require_integer(table: &mut Table, key: &str) -> Result<(), ConfigError> {
    let Some(value) = table.get(key) else {
        return Ok(());
    };
    match value {
        Value::Integer(_) => Ok(()),
        Value::String(text) => match text.trim().parse::<i64>() {
            Ok(parsed) => {
                table.insert(key.to_owned(), Value::Integer(parsed));
                Ok(())
            }
            Err(_) => Err(ConfigError::type_mismatch_found(
                key,
                "an integer",
                "a string",
            )),
        },
        other => Err(ConfigError::type_mismatch(key, "an integer", other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(toml: &str) -> Table {
        toml::from_str(toml).expect("valid toml fixture")
    }

    #[test]
    fn string_key_accepts_a_string() {
        let mut t = table(r#"model = "google/gemini-2.5-flash""#);
        validate(&mut t).expect("string model is valid");
    }

    #[test]
    fn string_key_rejects_a_number_naming_key_and_type() {
        let mut t = table("model = 123");
        let err = validate(&mut t).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("model"), "names the key: {message}");
        assert!(
            message.contains("string"),
            "names the expected type: {message}"
        );
        assert!(
            message.contains("integer"),
            "names what was found: {message}"
        );
    }

    #[test]
    fn integer_key_accepts_an_integer() {
        let mut t = table("max_steps = 25");
        validate(&mut t).expect("integer max_steps is valid");
        assert_eq!(t["max_steps"].as_integer(), Some(25));
    }

    #[test]
    fn integer_key_coerces_a_numeric_string() {
        // The env/CLI layers always produce strings; a numeric string normalizes to an integer.
        let mut t = table(r#"max_steps = "30""#);
        validate(&mut t).expect("numeric string coerces");
        assert_eq!(t["max_steps"].as_integer(), Some(30));
    }

    #[test]
    fn integer_key_rejects_a_non_numeric_string_naming_key_and_type() {
        let mut t = table(r#"max_steps = "lots""#);
        let err = validate(&mut t).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("max_steps"), "names the key: {message}");
        assert!(
            message.contains("integer"),
            "names the expected type: {message}"
        );
    }

    #[test]
    fn integer_key_rejects_a_float() {
        let mut t = table("max_steps = 1.5");
        let err = validate(&mut t).unwrap_err();
        assert!(err.to_string().contains("max_steps"));
    }

    #[test]
    fn absent_keys_are_fine() {
        let mut t = table(r#"unrelated = "value""#);
        validate(&mut t).expect("absent typed keys are not required");
    }
}
