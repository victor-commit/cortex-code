//! OpenRouter wiring on top of the `llm` OpenAI-compatible adapter.
//!
//! OpenRouter is the reference provider (architecture §7): an OpenAI-compatible Chat-completions
//! endpoint reached over the `llm` crate's rustls-only transport. This module resolves OpenRouter's
//! base URL from the catalogue and turns a resolved [`Credential`] into a ready [`OpenAiCompatible`]
//! provider, carrying the optional attribution headers OpenRouter recognizes.
//!
//! The credential is exposed to the transport at exactly one point (building the bearer header);
//! the key is never logged or otherwise rendered here.

use errors::Diagnostic;
use llm::{OPENROUTER_BASE_URL, OpenAiCompatible, OpenAiConfig};

use crate::catalogue::{Catalogue, DEFAULT_MODEL};
use crate::credentials::{Credential, CredentialInputs, resolve_credential};

/// The OpenRouter provider id, matching the catalogue and the credential store key.
pub const OPENROUTER_ID: &str = "openrouter";

/// The default OpenRouter model (architecture §7).
pub const OPENROUTER_DEFAULT_MODEL: &str = DEFAULT_MODEL;

/// Attribution headers OpenRouter optionally records for a request.
const REFERER: &str = "https://cortex.code";
const TITLE: &str = "Cortex Code";

/// Resolve the OpenRouter API base URL from the embedded catalogue, falling back to the canonical
/// constant if the snapshot somehow omits it.
#[must_use]
pub fn openrouter_base_url() -> String {
    Catalogue::embedded()
        .provider(OPENROUTER_ID)
        .and_then(|provider| provider.api.clone())
        .unwrap_or_else(|| OPENROUTER_BASE_URL.to_owned())
}

/// Build a ready OpenRouter [`OpenAiCompatible`] provider from a resolved credential.
///
/// # Errors
/// Returns the [`Diagnostic`] from the transport build (e.g. a rustls client failure) normalized
/// through the LLM error layer.
pub fn openrouter_provider(credential: &Credential) -> Result<OpenAiCompatible, Diagnostic> {
    let mut config = OpenAiConfig::openrouter(credential.expose());
    config.base_url = openrouter_base_url();
    let config = config
        .header("HTTP-Referer", REFERER)
        .header("X-Title", TITLE);
    OpenAiCompatible::new(config).map_err(|error| error.to_diagnostic())
}

/// Resolve the OpenRouter credential across the documented precedence layers and build a provider.
///
/// # Errors
/// Returns [`Diagnostic::NoCredential`] when no layer yields a key, or a transport-build diagnostic
/// if the client cannot be constructed.
pub fn resolve_openrouter(inputs: &CredentialInputs) -> Result<OpenAiCompatible, Diagnostic> {
    let credential = resolve_credential(OPENROUTER_ID, inputs)?;
    openrouter_provider(&credential)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::CredentialSource;

    #[test]
    fn base_url_resolves_from_the_catalogue() {
        assert_eq!(openrouter_base_url(), "https://openrouter.ai/api/v1");
    }

    #[test]
    fn provider_builds_from_a_resolved_credential() {
        let inputs = CredentialInputs {
            env_value: Some("env-key".to_owned()),
            ..Default::default()
        };
        let credential = resolve_credential(OPENROUTER_ID, &inputs).expect("resolves");
        assert_eq!(credential.source(), CredentialSource::Env);
        openrouter_provider(&credential).expect("provider builds");
    }

    #[test]
    fn resolve_openrouter_yields_no_credential_when_nothing_is_set() {
        let err = resolve_openrouter(&CredentialInputs::default()).expect_err("must fail");
        assert_eq!(err.kind(), "no-credential");
        assert_eq!(err, Diagnostic::no_credential(OPENROUTER_ID));
    }
}
