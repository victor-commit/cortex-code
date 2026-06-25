//! Environment-variable config overrides.
//!
//! A config key `<key>` is overridable by the environment variable `CORTEX_<KEY>` (the key
//! uppercased), e.g. `model` <- `CORTEX_MODEL`. This layer sits above every file layer and
//! below CLI flags.
//!
//! A small set of `CORTEX_*` variables are operational switches (the network kill-switch,
//! path overrides) rather than config keys, so they are never mapped into the config.

use std::collections::BTreeMap;

/// `CORTEX_*` suffixes that are operational controls, not config keys.
const RESERVED_SUFFIXES: &[&str] = &[
    "NO_NET",
    // Permission env payloads are parsed by the permissions loader, not as raw settings strings.
    "PERMISSION",
    "CONFIG_FILE",
    "CONFIG_DIR",
    "CONFIG_CONTENT",
    "CONFIG_HOME",
    "HOME",
    "DATA_DIR",
    "PURE",
    "STREAM_IDLE_TIMEOUT_MS",
];

/// The environment variable name that overrides config `key` (e.g. `model` -> `CORTEX_MODEL`).
#[must_use]
pub fn env_var_name(key: &str) -> String {
    format!("CORTEX_{}", key.to_ascii_uppercase())
}

/// Extract config overrides from an environment iterator: every `CORTEX_<KEY>` pair (minus the
/// reserved operational variables) becomes a `<key>` -> value override.
pub fn overrides_from<I>(vars: I) -> BTreeMap<String, String>
where
    I: IntoIterator<Item = (String, String)>,
{
    let mut overrides = BTreeMap::new();
    for (name, value) in vars {
        let Some(suffix) = name.strip_prefix("CORTEX_") else {
            continue;
        };
        if suffix.is_empty() || RESERVED_SUFFIXES.contains(&suffix) {
            continue;
        }
        overrides.insert(suffix.to_ascii_lowercase(), value);
    }
    overrides
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn maps_cortex_prefixed_vars_to_lowercase_keys() {
        let overrides = overrides_from(vars(&[
            ("CORTEX_MODEL", "from-env"),
            ("CORTEX_SMALL_MODEL", "mini"),
        ]));
        assert_eq!(overrides.get("model").map(String::as_str), Some("from-env"));
        assert_eq!(
            overrides.get("small_model").map(String::as_str),
            Some("mini")
        );
    }

    #[test]
    fn ignores_non_cortex_and_reserved_vars() {
        let overrides = overrides_from(vars(&[
            ("PATH", "/usr/bin"),
            ("CORTEX_NO_NET", "1"),
            ("CORTEX_PERMISSION", r#"{"bash":"deny"}"#),
            ("CORTEX_PURE", "1"),
            ("CORTEX_CONFIG_FILE", "/x"),
            ("CORTEX_CONFIG_CONTENT", "model = \"x\""),
            ("CORTEX_STREAM_IDLE_TIMEOUT_MS", "50"),
            ("CORTEX_", "empty-suffix"),
        ]));
        assert!(overrides.is_empty(), "got: {overrides:?}");
    }

    #[test]
    fn env_var_name_uppercases_the_key() {
        assert_eq!(env_var_name("model"), "CORTEX_MODEL");
        assert_eq!(env_var_name("small_model"), "CORTEX_SMALL_MODEL");
    }
}
