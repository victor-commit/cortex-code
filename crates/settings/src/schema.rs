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

use crate::{ConfigError, describe};

/// Validate (and where applicable normalize) the known typed keys in `table`.
///
/// # Errors
/// Returns [`ConfigError::TypeMismatch`] for a key whose value cannot be the expected type.
pub fn validate(table: &mut Table) -> Result<(), ConfigError> {
    require_string(table, "model")?;
    require_agent(table)?;
    require_integer(table, "max_steps")?;
    validate_provider(table)?;
    validate_formatter(table)?;
    validate_lsp(table)?;
    validate_mcp(table)?;
    validate_permission_array(table, "permission")?;
    validate_agent_permission_overrides(table)?;
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

fn validate_provider(table: &Table) -> Result<(), ConfigError> {
    let Some(value) = table.get("provider") else {
        return Ok(());
    };
    let provider = expect_table(value, "provider", "a table")?;
    for (key, value) in provider {
        match key.as_str() {
            "enabled" | "disabled" => {
                expect_array_of_strings(value, &format!("provider.{key}"))?;
            }
            "model_overrides" => {
                expect_table_of_strings(value, &format!("provider.{key}"))?;
            }
            provider_id => {
                let entry = expect_table(value, &format!("provider.{provider_id}"), "a table")?;
                validate_provider_entry(provider_id, entry)?;
            }
        }
    }
    Ok(())
}

fn validate_provider_entry(provider_id: &str, table: &Table) -> Result<(), ConfigError> {
    let base = format!("provider.{provider_id}");
    for (key, value) in table {
        match key.as_str() {
            "enabled" => expect_bool(value, &format!("{base}.enabled"))?,
            "base_url" | "api_key" | "api_key_env" | "protocol" => {
                expect_string(value, &format!("{base}.{key}"))?;
            }
            "auth" => {
                let auth = expect_table(value, &format!("{base}.auth"), "a table")?;
                for (auth_key, auth_value) in auth {
                    match auth_key.as_str() {
                        "api_key" | "api_key_env" => {
                            expect_string(auth_value, &format!("{base}.auth.{auth_key}"))?;
                        }
                        _ => {}
                    }
                }
            }
            "model_overrides" => {
                expect_table_of_strings(value, &format!("{base}.model_overrides"))?;
            }
            "models" => {
                expect_model_container(value, &format!("{base}.models"))?;
            }
            "options" => {
                let options = expect_table(value, &format!("{base}.options"), "a table")?;
                if let Some(headers) = options.get("headers") {
                    expect_table_of_strings(headers, &format!("{base}.options.headers"))?;
                }
                if let Some(params) = options.get("params") {
                    expect_table(params, &format!("{base}.options.params"), "a table")?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn expect_model_container(value: &Value, key: &str) -> Result<(), ConfigError> {
    match value {
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                expect_table(item, &format!("{key}[{index}]"), "a table")?;
            }
            Ok(())
        }
        Value::Table(models) => {
            for (model, value) in models {
                expect_table(value, &format!("{key}.{model}"), "a table")?;
            }
            Ok(())
        }
        _ => Err(ConfigError::type_mismatch_found(
            key,
            "an array or table",
            describe(value.type_str()),
        )),
    }
}

fn validate_formatter(table: &Table) -> Result<(), ConfigError> {
    let Some(value) = table.get("formatter") else {
        return Ok(());
    };
    match value {
        Value::Boolean(_) => Ok(()),
        Value::Table(formatter) => {
            if let Some(custom) = formatter.get("custom") {
                validate_formatter_custom(custom, "formatter.custom")?;
            }
            for builtin in [
                "gofmt",
                "biome",
                "mix",
                "clang_format",
                "zig_fmt",
                "ktlint",
                "dart",
                "terraform",
                "shfmt",
                "nixfmt",
                "ocamlformat",
                "rubocop",
                "standardrb",
                "erb",
                "gleam",
                "cljfmt",
                "ormolu",
                "latexindent",
                "air",
                "rustfmt",
                "prettier",
                "ruff",
            ] {
                if let Some(config) = formatter.get(builtin) {
                    let config = expect_table(config, &format!("formatter.{builtin}"), "a table")?;
                    if let Some(disabled) = config.get("disabled") {
                        expect_bool(disabled, &format!("formatter.{builtin}.disabled"))?;
                    }
                    if let Some(command) = config.get("command") {
                        expect_string_or_array_of_strings(
                            command,
                            &format!("formatter.{builtin}.command"),
                        )?;
                    }
                    if let Some(extensions) = config.get("extensions") {
                        expect_string_or_array_of_strings(
                            extensions,
                            &format!("formatter.{builtin}.extensions"),
                        )?;
                    }
                    if let Some(env) = config.get("env") {
                        expect_table_of_strings(env, &format!("formatter.{builtin}.env"))?;
                    }
                }
            }
            Ok(())
        }
        other => Err(ConfigError::type_mismatch_found(
            "formatter",
            "a boolean or table",
            describe(other.type_str()),
        )),
    }
}

fn validate_formatter_custom(value: &Value, key: &str) -> Result<(), ConfigError> {
    match value {
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                let table = expect_table(item, &format!("{key}[{index}]"), "a table")?;
                validate_custom_formatter_entry(table, &format!("{key}[{index}]"))?;
            }
            Ok(())
        }
        Value::Table(table)
            if table.contains_key("command") || table.contains_key("extensions") =>
        {
            validate_custom_formatter_entry(table, key)
        }
        Value::Table(table) => {
            for (name, value) in table {
                let entry = expect_table(value, &format!("{key}.{name}"), "a table")?;
                validate_custom_formatter_entry(entry, &format!("{key}.{name}"))?;
            }
            Ok(())
        }
        other => Err(ConfigError::type_mismatch_found(
            key,
            "a table or array of tables",
            describe(other.type_str()),
        )),
    }
}

fn validate_custom_formatter_entry(table: &Table, key: &str) -> Result<(), ConfigError> {
    let command = table.get("command").ok_or_else(|| {
        ConfigError::type_mismatch_found(
            format!("{key}.command"),
            "a string or array of strings",
            "missing value",
        )
    })?;
    expect_string_or_array_of_strings(command, &format!("{key}.command"))?;

    let extensions = table.get("extensions").ok_or_else(|| {
        ConfigError::type_mismatch_found(
            format!("{key}.extensions"),
            "a string or array of strings",
            "missing value",
        )
    })?;
    expect_string_or_array_of_strings(extensions, &format!("{key}.extensions"))?;
    if let Some(env) = table.get("env") {
        expect_table_of_strings(env, &format!("{key}.env"))?;
    }
    if let Some(disabled) = table.get("disabled") {
        expect_bool(disabled, &format!("{key}.disabled"))?;
    }
    Ok(())
}

fn validate_lsp(table: &Table) -> Result<(), ConfigError> {
    let Some(value) = table.get("lsp") else {
        return Ok(());
    };
    match value {
        Value::Boolean(_) => Ok(()),
        Value::Table(lsp) => {
            if let Some(servers) = lsp.get("servers") {
                let servers = expect_table(servers, "lsp.servers", "a table")?;
                for (name, config) in servers {
                    let config = expect_table(config, &format!("lsp.servers.{name}"), "a table")?;
                    if let Some(program) = config.get("program") {
                        expect_string(program, &format!("lsp.servers.{name}.program"))?;
                    }
                    if let Some(args) = config.get("args") {
                        expect_array_of_strings(args, &format!("lsp.servers.{name}.args"))?;
                    }
                    if let Some(extensions) = config.get("extensions") {
                        expect_array_of_strings(
                            extensions,
                            &format!("lsp.servers.{name}.extensions"),
                        )?;
                    }
                    if let Some(disabled) = config.get("disabled") {
                        expect_bool(disabled, &format!("lsp.servers.{name}.disabled"))?;
                    }
                    if let Some(env) = config.get("env") {
                        expect_table_of_strings(env, &format!("lsp.servers.{name}.env"))?;
                    }
                }
            }
            Ok(())
        }
        other => Err(ConfigError::type_mismatch_found(
            "lsp",
            "a boolean or table",
            describe(other.type_str()),
        )),
    }
}

fn validate_mcp(table: &Table) -> Result<(), ConfigError> {
    let Some(value) = table.get("mcp") else {
        return Ok(());
    };
    let mcp = expect_table(value, "mcp", "a table of named servers")?;
    for (name, value) in mcp {
        let server = expect_table(value, &format!("mcp.{name}"), "a table")?;
        for (key, value) in server {
            let path = format!("mcp.{name}.{key}");
            match key.as_str() {
                "type" | "url" | "cwd" => {
                    expect_string(value, &path)?;
                }
                "enabled" => expect_bool(value, &path)?,
                "command" => expect_array_of_strings(value, &path)?,
                "headers" | "environment" => expect_table_of_strings(value, &path)?,
                "timeout" => expect_integer(value, &path)?,
                "oauth" => {
                    let oauth = expect_table(value, &path, "a table")?;
                    for (oauth_key, oauth_value) in oauth {
                        let oauth_path = format!("{path}.{oauth_key}");
                        match oauth_key.as_str() {
                            "client_id" | "authorize_endpoint" | "token_endpoint" => {
                                expect_string(oauth_value, &oauth_path)?;
                            }
                            "scopes" => {
                                expect_array_of_strings(oauth_value, &oauth_path)?;
                            }
                            "callback_port" => {
                                expect_integer(oauth_value, &oauth_path)?;
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn validate_permission_array(table: &Table, key: &str) -> Result<(), ConfigError> {
    let Some(value) = table.get(key) else {
        return Ok(());
    };
    let rules = expect_array(value, key, "an array of rule tables")?;
    for (index, rule) in rules.iter().enumerate() {
        let rule_key = format!("{key}[{index}]");
        let table = expect_table(rule, &rule_key, "a table")?;
        let Some(action) = table.get("action") else {
            return Err(ConfigError::type_mismatch_found(
                format!("{rule_key}.action"),
                "a string (`allow`, `ask`, or `deny`)",
                "missing value",
            ));
        };
        let action = expect_string(action, &format!("{rule_key}.action"))?;
        if !matches!(action, "allow" | "ask" | "deny") {
            return Err(ConfigError::type_mismatch_found(
                format!("{rule_key}.action"),
                "a string (`allow`, `ask`, or `deny`)",
                format!("the unsupported action `{action}`"),
            ));
        }
        if let Some(tool) = table.get("tool") {
            expect_string(tool, &format!("{rule_key}.tool"))?;
        }
        if let Some(pattern) = table.get("pattern") {
            expect_string(pattern, &format!("{rule_key}.pattern"))?;
        }
    }
    Ok(())
}

fn validate_agent_permission_overrides(table: &Table) -> Result<(), ConfigError> {
    let Some(value) = table.get("agent") else {
        return Ok(());
    };
    let Value::Table(agents) = value else {
        return Ok(());
    };
    for (name, value) in agents {
        let agent = expect_table(value, &format!("agent.{name}"), "a table")?;
        if let Some(permission) = agent.get("permission") {
            let root = format!("agent.{name}.permission");
            let wrapper = Table::from_iter([(root.clone(), permission.clone())]);
            validate_permission_array(&wrapper, &root)?;
        }
    }
    Ok(())
}

fn expect_table<'a>(
    value: &'a Value,
    key: &str,
    expected: &'static str,
) -> Result<&'a Table, ConfigError> {
    value
        .as_table()
        .ok_or_else(|| ConfigError::type_mismatch_found(key, expected, describe(value.type_str())))
}

fn expect_array<'a>(
    value: &'a Value,
    key: &str,
    expected: &'static str,
) -> Result<&'a [Value], ConfigError> {
    value
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| ConfigError::type_mismatch_found(key, expected, describe(value.type_str())))
}

fn expect_string<'a>(value: &'a Value, key: &str) -> Result<&'a str, ConfigError> {
    value.as_str().ok_or_else(|| {
        ConfigError::type_mismatch_found(key, "a string", describe(value.type_str()))
    })
}

fn expect_bool(value: &Value, key: &str) -> Result<(), ConfigError> {
    if value.is_bool() {
        Ok(())
    } else {
        Err(ConfigError::type_mismatch_found(
            key,
            "a boolean",
            describe(value.type_str()),
        ))
    }
}

fn expect_integer(value: &Value, key: &str) -> Result<(), ConfigError> {
    if value.is_integer() {
        Ok(())
    } else {
        Err(ConfigError::type_mismatch_found(
            key,
            "an integer",
            describe(value.type_str()),
        ))
    }
}

fn expect_array_of_strings(value: &Value, key: &str) -> Result<(), ConfigError> {
    let values = expect_array(value, key, "an array of strings")?;
    for (index, value) in values.iter().enumerate() {
        expect_string(value, &format!("{key}[{index}]"))?;
    }
    Ok(())
}

fn expect_table_of_strings(value: &Value, key: &str) -> Result<(), ConfigError> {
    let table = expect_table(value, key, "a table of strings")?;
    for (name, value) in table {
        expect_string(value, &format!("{key}.{name}"))?;
    }
    Ok(())
}

fn expect_string_or_array_of_strings(value: &Value, key: &str) -> Result<(), ConfigError> {
    match value {
        Value::String(_) => Ok(()),
        Value::Array(_) => expect_array_of_strings(value, key),
        _ => Err(ConfigError::type_mismatch_found(
            key,
            "a string or array of strings",
            describe(value.type_str()),
        )),
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
    fn mcp_oauth_fields_accept_expected_types() {
        let mut t = table(
            r#"
[mcp.remote]
type = "remote"
url = "http://127.0.0.1:3226/mcp"
[mcp.remote.oauth]
client_id = "demo-client"
authorize_endpoint = "http://127.0.0.1:3226/authorize"
token_endpoint = "http://127.0.0.1:3226/token"
scopes = ["openid", "profile"]
callback_port = 3227
"#,
        );
        validate(&mut t).expect("mcp oauth config is valid");
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
