//! `{env:VAR}` and `{file:PATH}` value substitution for the merged config.
//!
//! Substitution applies to a string value that is *exactly* one token (the whole value is
//! `{env:NAME}` or `{file:PATH}`); tokens embedded inside surrounding text are left verbatim.
//! Resolution walks every string in the merged table, including those nested in sub-tables and
//! arrays, so secrets referenced anywhere (e.g. `[auth] key = "{env:OPENROUTER_API_KEY}"`) are
//! resolved.
//!
//! - `{env:NAME}` resolves to the value of environment variable `NAME`. An unset variable is a
//!   hard error (a clear diagnostic naming the variable), never a silent leak of the literal
//!   token into an effective value.
//! - `{file:PATH}` resolves to the contents of `PATH`, **trimmed** of leading/trailing
//!   whitespace (so a trailing newline in a secret file does not corrupt the value). A relative
//!   path is resolved against the invocation directory. A missing/unreadable file is a hard
//!   error naming the path.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use toml::Value;
use toml::value::Table;

use crate::ConfigError;

const ENV_PREFIX: &str = "{env:";
const FILE_PREFIX: &str = "{file:";
const SUFFIX: char = '}';

/// Resolve every `{env:}`/`{file:}` token in `table` in place.
///
/// `env` is the full environment to resolve `{env:}` tokens against; `cwd` anchors relative
/// `{file:}` paths.
///
/// # Errors
/// Returns [`ConfigError::EnvMissing`] for an unset variable and [`ConfigError::FileRead`] for
/// an unreadable substitution file.
pub fn substitute(
    table: &mut Table,
    env: &BTreeMap<String, String>,
    cwd: &Path,
) -> Result<(), ConfigError> {
    substitute_table(table, "", env, cwd)
}

fn substitute_table(
    table: &mut Table,
    prefix: &str,
    env: &BTreeMap<String, String>,
    cwd: &Path,
) -> Result<(), ConfigError> {
    for (key, value) in table.iter_mut() {
        substitute_value(value, &join(prefix, key), env, cwd)?;
    }
    Ok(())
}

fn substitute_value(
    value: &mut Value,
    path: &str,
    env: &BTreeMap<String, String>,
    cwd: &Path,
) -> Result<(), ConfigError> {
    match value {
        Value::String(raw) => {
            if let Some(resolved) = resolve(raw, path, env, cwd)? {
                *value = Value::String(resolved);
            }
        }
        Value::Table(table) => substitute_table(table, path, env, cwd)?,
        Value::Array(items) => {
            for (index, item) in items.iter_mut().enumerate() {
                substitute_value(item, &format!("{path}.{index}"), env, cwd)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Resolve `raw` if the whole string is a single token. Returns `Ok(None)` when `raw` is not a
/// substitution token (it is left unchanged).
fn resolve(
    raw: &str,
    path: &str,
    env: &BTreeMap<String, String>,
    cwd: &Path,
) -> Result<Option<String>, ConfigError> {
    if let Some(name) = token_body(raw, ENV_PREFIX) {
        return resolve_env(name, path, env).map(Some);
    }
    if let Some(file) = token_body(raw, FILE_PREFIX) {
        return resolve_file(file, path, cwd).map(Some);
    }
    Ok(None)
}

/// Extract the inner body of a `{prefix...}` token when `raw` is exactly that token, i.e. it
/// starts with `prefix` and ends with `}`. Returns `None` otherwise.
fn token_body<'a>(raw: &'a str, prefix: &str) -> Option<&'a str> {
    raw.strip_prefix(prefix)
        .and_then(|rest| rest.strip_suffix(SUFFIX))
}

fn resolve_env(
    name: &str,
    path: &str,
    env: &BTreeMap<String, String>,
) -> Result<String, ConfigError> {
    env.get(name)
        .cloned()
        .ok_or_else(|| ConfigError::env_missing(path, name))
}

fn resolve_file(rel: &str, path: &str, cwd: &Path) -> Result<String, ConfigError> {
    let full = resolve_path(rel, cwd);
    match std::fs::read_to_string(&full) {
        Ok(contents) => Ok(contents.trim().to_owned()),
        Err(source) => Err(ConfigError::file_read(path, rel, source)),
    }
}

fn resolve_path(rel: &str, cwd: &Path) -> PathBuf {
    let candidate = Path::new(rel);
    if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        cwd.join(candidate)
    }
}

/// Build a dotted diagnostic path (`prefix.key`), or just `key` at the top level.
fn join(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_owned()
    } else {
        format!("{prefix}.{key}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    fn table(toml: &str) -> Table {
        toml::from_str(toml).expect("valid toml fixture")
    }

    #[test]
    fn env_token_resolves_to_variable_value() {
        let mut t = table(r#"key = "{env:SOME_VAR}""#);
        substitute(&mut t, &env(&[("SOME_VAR", "resolved")]), Path::new("/tmp")).expect("resolve");
        assert_eq!(t["key"].as_str(), Some("resolved"));
    }

    #[test]
    fn missing_env_var_errors_and_names_the_variable() {
        let mut t = table(r#"key = "{env:MISSING_VAR}""#);
        let err = substitute(&mut t, &env(&[]), Path::new("/tmp")).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("MISSING_VAR"), "got: {message}");
        // The literal token must never survive into the effective value.
        assert!(
            !message.contains("{env:"),
            "diagnostic must not echo the raw token verbatim"
        );
    }

    #[test]
    fn non_token_strings_are_left_verbatim() {
        let mut t = table(
            r#"
            plain = "just text"
            braces = "{not a token}"
            embedded = "prefix {env:X} suffix"
            "#,
        );
        substitute(&mut t, &env(&[("X", "v")]), Path::new("/tmp")).expect("resolve");
        assert_eq!(t["plain"].as_str(), Some("just text"));
        assert_eq!(t["braces"].as_str(), Some("{not a token}"));
        assert_eq!(t["embedded"].as_str(), Some("prefix {env:X} suffix"));
    }

    #[test]
    fn nested_tables_and_arrays_are_resolved() {
        let mut t = table(
            r#"
            [auth]
            key = "{env:TOKEN}"
            extras = ["{env:A}", "literal"]
            "#,
        );
        substitute(
            &mut t,
            &env(&[("TOKEN", "secret"), ("A", "first")]),
            Path::new("/tmp"),
        )
        .expect("resolve");
        let auth = t["auth"].as_table().expect("auth table");
        assert_eq!(auth["key"].as_str(), Some("secret"));
        let extras = auth["extras"].as_array().expect("extras array");
        assert_eq!(extras[0].as_str(), Some("first"));
        assert_eq!(extras[1].as_str(), Some("literal"));
    }

    #[test]
    fn file_token_resolves_to_trimmed_contents() {
        let dir = std::env::temp_dir().join(format!("cortex-subst-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let file = dir.join("secret.txt");
        std::fs::write(&file, "  s3cr3t-value\n\n").expect("write");

        let mut t = table(r#"key = "{file:secret.txt}""#);
        substitute(&mut t, &env(&[]), &dir).expect("resolve");
        assert_eq!(t["key"].as_str(), Some("s3cr3t-value"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_errors_and_names_the_path() {
        let mut t = table(r#"key = "{file:./nope.txt}""#);
        let err = substitute(&mut t, &env(&[]), Path::new("/tmp")).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("./nope.txt"), "got: {message}");
        assert!(
            !message.contains("{file:"),
            "diagnostic must not echo the raw token verbatim"
        );
    }
}
