//! Integration coverage for config value substitution and parse/validation diagnostics,
//! exercising the public `load` API against real files on disk.
//!
//! Mirrors the M0 contract VAL-SKEL-025..030:
//! - `{env:VAR}` resolves; an unset variable is a clear, non-leaking error.
//! - `{file:PATH}` resolves to trimmed contents; a missing file is a clear, non-leaking error.
//! - invalid TOML is a clear, located parse error naming the file.
//! - a wrongly-typed value names the offending key and expected type.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use settings::{ConfigError, ConfigInputs, load};

/// A self-cleaning scratch tree with an isolated HOME, project root, and nested CWD.
struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = format!(
            "cortex-subst-it-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let root = std::env::temp_dir().join(unique);
        fs::create_dir_all(root.join("home/proj/sub")).expect("create scratch tree");
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
        fs::write(path, contents).expect("write file");
    }

    fn write_project(&self, contents: &str) {
        self.write("home/proj/cortex.toml", contents);
    }

    fn inputs(&self, env: &[(&str, &str)]) -> ConfigInputs {
        ConfigInputs {
            home: Some(self.home()),
            xdg_config_home: None,
            cwd: self.nested(),
            env: env
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
            cli_overrides: BTreeMap::new(),
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

// VAL-SKEL-025: `{env:SOME_VAR}` resolves to the variable's value in the effective config.
#[test]
fn env_substitution_resolves() {
    let scratch = Scratch::new();
    scratch.write_project(r#"secret = "{env:SOME_VAR}""#);
    let config = load(scratch.inputs(&[("SOME_VAR", "resolved")])).expect("loads");
    assert_eq!(
        config.get("secret").and_then(|v| v.as_str()),
        Some("resolved")
    );
}

// VAL-SKEL-026: an unset `{env:MISSING_VAR}` is a hard error naming the variable, never a leak.
#[test]
fn missing_env_var_is_a_clear_non_leaking_error() {
    let scratch = Scratch::new();
    scratch.write_project(r#"secret = "{env:MISSING_VAR}""#);
    let err = load(scratch.inputs(&[])).unwrap_err();
    assert!(matches!(err, ConfigError::EnvMissing { .. }));
    let message = err.to_string();
    assert!(
        message.contains("MISSING_VAR"),
        "names the variable: {message}"
    );
    assert!(
        !message.contains("{env:MISSING_VAR}"),
        "must not leak the literal token: {message}"
    );
}

// VAL-SKEL-027: `{file:PATH}` resolves to the file's trimmed contents.
#[test]
fn file_substitution_resolves_trimmed_contents() {
    let scratch = Scratch::new();
    // The file lives next to the config file; the relative token is anchored to that config file.
    scratch.write("home/proj/secret.txt", "  file-secret\n\n");
    scratch.write_project(r#"secret = "{file:./secret.txt}""#);
    let config = load(scratch.inputs(&[])).expect("loads");
    assert_eq!(
        config.get("secret").and_then(|v| v.as_str()),
        Some("file-secret")
    );
}

// VAL-SKEL-028: a missing `{file:PATH}` is a hard error naming the path, never a leak.
#[test]
fn missing_file_is_a_clear_non_leaking_error() {
    let scratch = Scratch::new();
    scratch.write_project(r#"secret = "{file:./nope.txt}""#);
    let err = load(scratch.inputs(&[])).unwrap_err();
    assert!(matches!(err, ConfigError::FileRead { .. }));
    let message = err.to_string();
    assert!(message.contains("./nope.txt"), "names the path: {message}");
    assert!(
        !message.contains("{file:./nope.txt}"),
        "must not leak the literal token: {message}"
    );
}

// VAL-SKEL-029: invalid TOML is a located parse error naming the offending file.
#[test]
fn invalid_toml_is_a_located_parse_error() {
    let scratch = Scratch::new();
    scratch.write_project("model = \n");
    let err = load(scratch.inputs(&[])).unwrap_err();
    assert!(matches!(err, ConfigError::Parse { .. }));
    let message = err.to_string();
    assert!(message.contains("cortex.toml"), "names the file: {message}");
    // The `toml` crate's error carries the offending line/column.
    assert!(message.contains("line"), "locates the error: {message}");
}

// VAL-SKEL-030: a wrongly-typed value names the offending key and the expected type.
#[test]
fn type_mismatch_names_key_and_expected_type() {
    // A number where the string-typed `model` key is expected.
    let scratch = Scratch::new();
    scratch.write_project("model = 123");
    let err = load(scratch.inputs(&[])).unwrap_err();
    assert!(matches!(err, ConfigError::TypeMismatch { .. }));
    let message = err.to_string();
    assert!(message.contains("model"), "names the key: {message}");
    assert!(
        message.contains("string"),
        "names the expected type: {message}"
    );
}

// VAL-SKEL-030 (reverse direction): a string where the integer-typed `max_steps` is expected.
#[test]
fn type_mismatch_on_numeric_key_names_key_and_expected_type() {
    let scratch = Scratch::new();
    scratch.write_project(r#"max_steps = "lots""#);
    let err = load(scratch.inputs(&[])).unwrap_err();
    assert!(matches!(err, ConfigError::TypeMismatch { .. }));
    let message = err.to_string();
    assert!(message.contains("max_steps"), "names the key: {message}");
    assert!(
        message.contains("integer"),
        "names the expected type: {message}"
    );
}

// The default step budget is observable and typed as an integer.
#[test]
fn max_steps_default_is_an_integer() {
    let scratch = Scratch::new();
    let config = load(scratch.inputs(&[])).expect("loads");
    assert_eq!(config.max_steps(), 25);
    assert!(
        config
            .get("max_steps")
            .and_then(|v| v.as_integer())
            .is_some()
    );
}

// A numeric `max_steps` supplied as an environment string coerces to an integer.
#[test]
fn env_numeric_string_coerces_for_integer_key() {
    let scratch = Scratch::new();
    let config = load(scratch.inputs(&[("CORTEX_MAX_STEPS", "40")])).expect("loads");
    assert_eq!(config.max_steps(), 40);
}

#[test]
fn embedded_tokens_resolve_inside_larger_strings() {
    let scratch = Scratch::new();
    scratch.write("home/proj/chunk.txt", "segment");
    scratch.write_project(
        r#"
message = "prefix-{env:EMBED_VAR}-{file:./chunk.txt}-suffix"
"#,
    );
    let config = load(scratch.inputs(&[("EMBED_VAR", "value")])).expect("loads");
    assert_eq!(
        config.get("message").and_then(|v| v.as_str()),
        Some("prefix-value-segment-suffix")
    );
}

#[test]
fn managed_locked_env_substitution_resolves_and_overrides_env_and_cli() {
    let scratch = Scratch::new();
    let managed_path = scratch.root.join("managed/managed.toml");
    scratch.write("managed/managed.toml", r#"model = "{env:MODEL_ENV}""#);
    scratch.write_project(&format!(
        r#"
model = "project-model"
[config_sources]
managed_file = "{}"
managed_locked_keys = ["model"]
"#,
        managed_path.display()
    ));

    let mut inputs = scratch.inputs(&[
        ("MODEL_ENV", "managed-model"),
        ("CORTEX_MODEL", "from-env-override"),
    ]);
    inputs
        .cli_overrides
        .insert("model".to_owned(), "from-cli-override".to_owned());
    let config = load(inputs).expect("loads");
    assert_eq!(config.model(), "managed-model");
}

#[test]
fn managed_locked_file_substitution_is_resolved_relative_to_managed_file() {
    let scratch = Scratch::new();
    let managed_path = scratch.root.join("managed/managed.toml");
    scratch.write("managed/model.txt", "  managed-from-file  \n");
    scratch.write("managed/managed.toml", r#"model = "{file:./model.txt}""#);
    scratch.write_project(&format!(
        r#"
model = "project-model"
[config_sources]
managed_file = "{}"
managed_locked_keys = ["model"]
"#,
        managed_path.display()
    ));

    let config = load(scratch.inputs(&[])).expect("loads");
    assert_eq!(config.model(), "managed-from-file");
}

#[test]
fn tilde_expands_to_home_for_path_values() {
    let scratch = Scratch::new();
    scratch.write_project(r#"cache_path = "~/cache/data.txt""#);
    let config = load(scratch.inputs(&[])).expect("loads");
    let expected = scratch.home().join("cache/data.txt");
    assert_eq!(
        config.get("cache_path").and_then(|v| v.as_str()),
        Some(expected.to_string_lossy().as_ref())
    );
}
