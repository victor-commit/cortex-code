//! `{env:VAR}` and `{file:PATH}` value substitution for the merged config.
//!
//! Substitution applies to a string value that is *exactly* one token (the whole value is
//! `{env:NAME}` or `{file:PATH}`), and also when those tokens are embedded inside surrounding
//! text.
//! Resolution walks every string in the merged table, including those nested in sub-tables and
//! arrays, so secrets referenced anywhere (e.g. `[auth] key = "{env:OPENROUTER_API_KEY}"`) are
//! resolved.
//!
//! - `{env:NAME}` resolves to the value of environment variable `NAME`. An unset variable is a
//!   hard error (a clear diagnostic naming the variable), never a silent leak of the literal
//!   token into an effective value.
//! - `{file:PATH}` resolves to the contents of `PATH`, **trimmed** of leading/trailing
//!   whitespace (so a trailing newline in a secret file does not corrupt the value). A relative
//!   path is resolved against the config file directory for file-backed layers (or against the
//!   invocation directory for non-file layers). A missing/unreadable file is a hard
//!   error naming the path.
//! - A string beginning with `~` (for example `~/secrets.txt`) expands to the isolated HOME when
//!   HOME is available, so no literal `~` reaches operational path values.

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
    base_dir: &Path,
    home: Option<&Path>,
) -> Result<(), ConfigError> {
    substitute_table(table, "", env, base_dir, home)
}

fn substitute_table(
    table: &mut Table,
    prefix: &str,
    env: &BTreeMap<String, String>,
    base_dir: &Path,
    home: Option<&Path>,
) -> Result<(), ConfigError> {
    for (key, value) in table.iter_mut() {
        substitute_value(value, &join(prefix, key), env, base_dir, home)?;
    }
    Ok(())
}

fn substitute_value(
    value: &mut Value,
    path: &str,
    env: &BTreeMap<String, String>,
    base_dir: &Path,
    home: Option<&Path>,
) -> Result<(), ConfigError> {
    match value {
        Value::String(raw) => {
            let resolved = resolve(raw, path, env, base_dir, home)?;
            if resolved != *raw {
                *value = Value::String(resolved);
            }
        }
        Value::Table(table) => substitute_table(table, path, env, base_dir, home)?,
        Value::Array(items) => {
            for (index, item) in items.iter_mut().enumerate() {
                substitute_value(item, &format!("{path}.{index}"), env, base_dir, home)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Resolve every substitution token in `raw`, including embedded tokens.
fn resolve(
    raw: &str,
    path: &str,
    env: &BTreeMap<String, String>,
    base_dir: &Path,
    home: Option<&Path>,
) -> Result<String, ConfigError> {
    let mut out = String::with_capacity(raw.len());
    let mut cursor = 0;
    while cursor < raw.len() {
        let Some(open_rel) = raw[cursor..].find('{') else {
            out.push_str(&raw[cursor..]);
            break;
        };
        let open = cursor + open_rel;
        out.push_str(&raw[cursor..open]);

        if let Some(body) = raw[open..].strip_prefix(ENV_PREFIX)
            && let Some(close) = body.find(SUFFIX)
        {
            let name = &body[..close];
            out.push_str(&resolve_env(name, path, env)?);
            cursor = open + ENV_PREFIX.len() + close + 1;
            continue;
        }
        if let Some(body) = raw[open..].strip_prefix(FILE_PREFIX)
            && let Some(close) = body.find(SUFFIX)
        {
            let file = &body[..close];
            out.push_str(&resolve_file(file, path, base_dir, home)?);
            cursor = open + FILE_PREFIX.len() + close + 1;
            continue;
        }

        out.push('{');
        cursor = open + 1;
    }

    Ok(expand_home_in_string(&out, home))
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

fn resolve_file(
    rel: &str,
    path: &str,
    base_dir: &Path,
    home: Option<&Path>,
) -> Result<String, ConfigError> {
    let full = resolve_path(rel, base_dir, home);
    match std::fs::read_to_string(&full) {
        Ok(contents) => Ok(contents.trim().to_owned()),
        Err(source) => Err(ConfigError::file_read(path, rel, source)),
    }
}

fn resolve_path(raw: &str, base_dir: &Path, home: Option<&Path>) -> PathBuf {
    let expanded = expand_home_in_string(raw, home);
    let candidate = Path::new(&expanded);
    if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        base_dir.join(candidate)
    }
}

fn expand_home_in_string(raw: &str, home: Option<&Path>) -> String {
    let Some(home) = home else {
        return raw.to_owned();
    };
    if raw == "~" {
        return home.display().to_string();
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return home.join(rest).display().to_string();
    }
    raw.to_owned()
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
        substitute(
            &mut t,
            &env(&[("SOME_VAR", "resolved")]),
            Path::new("/tmp"),
            None,
        )
        .expect("resolve");
        assert_eq!(t["key"].as_str(), Some("resolved"));
    }

    #[test]
    fn missing_env_var_errors_and_names_the_variable() {
        let mut t = table(r#"key = "{env:MISSING_VAR}""#);
        let err = substitute(&mut t, &env(&[]), Path::new("/tmp"), None).unwrap_err();
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
        substitute(&mut t, &env(&[("X", "v")]), Path::new("/tmp"), None).expect("resolve");
        assert_eq!(t["plain"].as_str(), Some("just text"));
        assert_eq!(t["braces"].as_str(), Some("{not a token}"));
        assert_eq!(t["embedded"].as_str(), Some("prefix v suffix"));
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
            None,
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
        substitute(&mut t, &env(&[]), &dir, None).expect("resolve");
        assert_eq!(t["key"].as_str(), Some("s3cr3t-value"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_errors_and_names_the_path() {
        let mut t = table(r#"key = "{file:./nope.txt}""#);
        let err = substitute(&mut t, &env(&[]), Path::new("/tmp"), None).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("./nope.txt"), "got: {message}");
        assert!(
            !message.contains("{file:"),
            "diagnostic must not echo the raw token verbatim"
        );
    }

    #[test]
    fn file_tokens_can_be_embedded_inside_larger_strings() {
        let dir = std::env::temp_dir().join(format!("cortex-subst-embed-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(dir.join("segment.txt"), "VALUE").expect("write");
        let mut t = table(r#"key = "prefix:{file:segment.txt}:suffix""#);
        substitute(&mut t, &env(&[]), &dir, None).expect("resolve");
        assert_eq!(t["key"].as_str(), Some("prefix:VALUE:suffix"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tilde_expands_to_home_for_path_like_values() {
        let mut t = table(r#"path = "~/cache/file.txt""#);
        substitute(
            &mut t,
            &env(&[]),
            Path::new("/tmp"),
            Some(Path::new("/home/tester")),
        )
        .expect("resolve");
        assert_eq!(t["path"].as_str(), Some("/home/tester/cache/file.txt"));
    }
}
