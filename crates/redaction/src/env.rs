//! The secret environment list stripped from spawned child processes.
//!
//! The agent's `bash` tool spawns shell commands. A spawned command must not be able to read the
//! agent's credentials out of its environment, so the child's environment is rebuilt with the
//! secrets removed. This module is the single source of truth for *which* variables are secret:
//!
//! - [`SECRET_ENV_VARS`] / [`is_secret_env_var`]: the credential variables stripped by **name**
//!   (e.g. `OPENROUTER_API_KEY`), regardless of their value.
//! - [`value_is_secret`]: a value-shape catch-all — any variable whose value contains an
//!   API-key-shaped secret (detected by [`redact`](crate::redact)) is stripped too, so a secret
//!   copied into an unexpected variable name still cannot leak into a child.

use crate::secret::redact;

/// The credential environment variables stripped from any child process the agent spawns.
///
/// Matched case-insensitively by [`is_secret_env_var`]. The list centralizes the known credential
/// variables; the value-shape catch-all [`value_is_secret`] covers any secret that lands in a name
/// not listed here.
pub const SECRET_ENV_VARS: &[&str] = &[
    "OPENROUTER_API_KEY",
    "ANTHROPIC_API_KEY",
    "OPENAI_API_KEY",
    "GEMINI_API_KEY",
    "GROQ_API_KEY",
    "MISTRAL_API_KEY",
    "DEEPSEEK_API_KEY",
    "XAI_API_KEY",
    "GOOGLE_API_KEY",
    "GOOGLE_GENERATIVE_AI_API_KEY",
    "AZURE_API_KEY",
    "PERPLEXITY_API_KEY",
    "TOGETHER_API_KEY",
    "FIREWORKS_API_KEY",
    "CEREBRAS_API_KEY",
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "GITHUB_TOKEN",
    "GH_TOKEN",
    "GITLAB_TOKEN",
    "NPM_TOKEN",
    "CARGO_REGISTRY_TOKEN",
    "HF_TOKEN",
];

/// Name fragments that indicate a credential-bearing variable even when the exact provider name
/// is not in [`SECRET_ENV_VARS`]. This catches synthetic validator secrets and copied credentials
/// under ad-hoc names while preserving ordinary runtime variables like `PATH` and `HOME`.
const SECRET_ENV_NAME_MARKERS: &[&str] = &[
    "API_KEY",
    "ACCESS_KEY",
    "ACCESS_TOKEN",
    "AUTH_TOKEN",
    "PRIVATE_KEY",
    "SECRET",
    "TOKEN",
    "PASSWORD",
    "PASSWD",
    "CREDENTIAL",
    "CREDENTIALS",
];

/// Whether `name` is a known credential environment variable that must be stripped from a child
/// process (case-insensitive match against [`SECRET_ENV_VARS`]).
#[must_use]
pub fn is_secret_env_var(name: &str) -> bool {
    let known = SECRET_ENV_VARS
        .iter()
        .any(|known| known.eq_ignore_ascii_case(name));
    known || name_contains_secret_marker(name)
}

fn name_contains_secret_marker(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    SECRET_ENV_NAME_MARKERS
        .iter()
        .any(|marker| upper.contains(marker))
}

/// Whether `value` contains an API-key-shaped secret (i.e. [`redact`](crate::redact) would change
/// it). Used as the value-shape catch-all so a secret stored under an unlisted variable name is
/// still stripped from a spawned child.
#[must_use]
pub fn value_is_secret(value: &str) -> bool {
    redact(value) != value
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake-but-realistically-shaped OpenRouter key, assembled from fragments so no contiguous
    /// key-shaped literal lives in source (the commit secret scanner flags those).
    fn openrouter_key() -> String {
        let body = "deadbeef".repeat(8);
        format!("sk-{}-{}", "or-v1", body)
    }

    #[test]
    fn the_openrouter_key_is_a_secret_env_var() {
        assert!(is_secret_env_var("OPENROUTER_API_KEY"));
        // Case-insensitive.
        assert!(is_secret_env_var("openrouter_api_key"));
    }

    #[test]
    fn ordinary_variables_are_not_secret_by_name() {
        for name in ["PATH", "HOME", "PWD", "TERM", "LANG", "CORTEX_NO_NET"] {
            assert!(!is_secret_env_var(name), "{name} must not be secret");
        }
    }

    #[test]
    fn synthetic_and_common_secret_names_are_secret_by_name() {
        for name in [
            "CORTEX_MCP_PARENT_SECRET",
            "GITHUB_TOKEN",
            "DATABASE_PASSWORD",
            "AWS_ACCESS_KEY_ID",
            "SOME_API_KEY",
            "custom_private_key",
        ] {
            assert!(is_secret_env_var(name), "{name} must be secret");
        }
    }

    #[test]
    fn a_key_shaped_value_is_a_secret() {
        assert!(value_is_secret(&openrouter_key()));
        assert!(value_is_secret(&format!("Bearer {}", openrouter_key())));
    }

    #[test]
    fn ordinary_values_are_not_secret_by_shape() {
        for value in [
            "/usr/bin:/bin",
            "google/gemini-2.5-flash",
            "1",
            "",
            "task-runner",
        ] {
            assert!(!value_is_secret(value), "{value:?} must not be secret");
        }
    }
}
