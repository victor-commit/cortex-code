//! OpenRouter wiring on top of the `llm` OpenAI-compatible adapter.
//!
//! OpenRouter is the reference provider (architecture §7): an OpenAI-compatible Chat-completions
//! endpoint reached over the `llm` crate's rustls-only transport. This module resolves OpenRouter's
//! base URL from the catalogue and turns a resolved [`Credential`] into a ready [`OpenAiCompatible`]
//! provider, carrying the optional attribution headers OpenRouter recognizes.
//!
//! The credential is exposed to the transport at exactly one point (building the bearer header);
//! the key is never logged or otherwise rendered here.

use std::time::Duration;

use errors::Diagnostic;
use llm::{OPENROUTER_BASE_URL, OpenAiCompatible, OpenAiConfig};

use crate::catalogue::{Catalogue, DEFAULT_MODEL};
use crate::credentials::{Credential, CredentialInputs, resolve_credential};

/// The OpenRouter provider id, matching the catalogue and the credential store key.
pub const OPENROUTER_ID: &str = "openrouter";
/// Canonical OpenRouter API base URL.
pub const OPENROUTER_API_BASE: &str = OPENROUTER_BASE_URL;

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
        .unwrap_or_else(|| OPENROUTER_API_BASE.to_owned())
}

/// Build a ready OpenRouter [`OpenAiCompatible`] provider from a resolved credential, targeting
/// the catalogue/default base URL.
///
/// # Errors
/// Returns the [`Diagnostic`] from the transport build (e.g. a rustls client failure) normalized
/// through the LLM error layer.
pub fn openrouter_provider(credential: &Credential) -> Result<OpenAiCompatible, Diagnostic> {
    openrouter_provider_with_base_url(credential, &openrouter_base_url())
}

/// Build a ready OpenRouter [`OpenAiCompatible`] provider from a resolved credential against an
/// explicit `base_url`.
///
/// This is the seam for pointing the provider at an alternate endpoint (e.g. a self-hosted
/// gateway, or a deliberately unreachable loopback port in tests), without disturbing the
/// catalogue default for normal runs.
///
/// # Errors
/// Returns the [`Diagnostic`] from the transport build (e.g. a rustls client failure) normalized
/// through the LLM error layer.
pub fn openrouter_provider_with_base_url(
    credential: &Credential,
    base_url: &str,
) -> Result<OpenAiCompatible, Diagnostic> {
    openrouter_provider_with_options(credential, base_url, None)
}

/// Build a ready OpenRouter [`OpenAiCompatible`] provider from a resolved credential against an
/// explicit `base_url`, optionally overriding the transport's per-frame idle/stall timeout.
///
/// A `Some(idle_timeout)` retargets how long the transport waits for each successive SSE frame
/// before treating the stream as stalled and failing closed with a transport diagnostic. A `None`
/// keeps the transport default, so normal runs are unaffected. This is the seam a forced-stall test
/// uses to set a tiny idle budget and prove the run aborts within a bounded window rather than
/// hanging (VAL-RUN-004).
///
/// # Errors
/// Returns the [`Diagnostic`] from the transport build (e.g. a rustls client failure) normalized
/// through the LLM error layer.
pub fn openrouter_provider_with_options(
    credential: &Credential,
    base_url: &str,
    idle_timeout: Option<Duration>,
) -> Result<OpenAiCompatible, Diagnostic> {
    openai_compatible_provider_with_options(
        OPENROUTER_ID,
        credential,
        base_url,
        Some(
            [
                (String::from("HTTP-Referer"), String::from(REFERER)),
                (String::from("X-Title"), String::from(TITLE)),
            ]
            .as_slice(),
        ),
        idle_timeout,
    )
}

/// Build a ready OpenAI-compatible provider for `provider_id` from a resolved credential.
///
/// `extra_headers` are merged into the request transport for every call.
///
/// # Errors
/// Returns the normalized transport diagnostic if the client cannot be constructed.
pub fn openai_compatible_provider_with_options(
    provider_id: &str,
    credential: &Credential,
    base_url: &str,
    extra_headers: Option<&[(String, String)]>,
    idle_timeout: Option<Duration>,
) -> Result<OpenAiCompatible, Diagnostic> {
    openai_compatible_provider_with_api_key(
        provider_id,
        credential.expose(),
        base_url,
        extra_headers,
        idle_timeout,
    )
}

pub(crate) fn openai_compatible_provider_with_api_key(
    provider_id: &str,
    api_key: &str,
    base_url: &str,
    extra_headers: Option<&[(String, String)]>,
    idle_timeout: Option<Duration>,
) -> Result<OpenAiCompatible, Diagnostic> {
    let mut config = OpenAiConfig {
        base_url: base_url.to_owned(),
        api_key: api_key.to_owned(),
        provider_name: provider_id.to_owned(),
        extra_headers: extra_headers
            .map(|headers| headers.to_vec())
            .unwrap_or_default(),
        include_usage: true,
        transport: llm::TransportConfig::default(),
    };
    if let Some(idle_timeout) = idle_timeout {
        config.transport.idle_timeout = idle_timeout;
    }
    OpenAiCompatible::new(config).map_err(|error| error.to_diagnostic())
}

/// Resolve `provider_id`'s credential and build a configured OpenAI-compatible provider.
///
/// # Errors
/// Returns [`Diagnostic::NoCredential`] when no key is available, or a normalized transport
/// diagnostic if the client cannot be constructed.
pub fn resolve_openai_compatible_with_options(
    provider_id: &str,
    inputs: &CredentialInputs,
    base_url: &str,
    extra_headers: Option<&[(String, String)]>,
    idle_timeout: Option<Duration>,
) -> Result<OpenAiCompatible, Diagnostic> {
    let credential = resolve_credential(provider_id, inputs)?;
    openai_compatible_provider_with_options(
        provider_id,
        &credential,
        base_url,
        extra_headers,
        idle_timeout,
    )
}

/// Resolve the OpenRouter credential across the documented precedence layers and build a provider
/// against the catalogue/default base URL.
///
/// # Errors
/// Returns [`Diagnostic::NoCredential`] when no layer yields a key, or a transport-build diagnostic
/// if the client cannot be constructed.
pub fn resolve_openrouter(inputs: &CredentialInputs) -> Result<OpenAiCompatible, Diagnostic> {
    resolve_openrouter_with(inputs, None)
}

/// Resolve the OpenRouter credential across the documented precedence layers and build a provider,
/// honoring an optional `base_url` override.
///
/// A `None` (or empty) override falls back to the catalogue/default base URL, so normal runs are
/// unaffected; a non-empty override targets the supplied endpoint. The credential resolution and
/// redaction guarantees are identical to [`resolve_openrouter`].
///
/// # Errors
/// Returns [`Diagnostic::NoCredential`] when no layer yields a key, or a transport-build diagnostic
/// if the client cannot be constructed.
pub fn resolve_openrouter_with(
    inputs: &CredentialInputs,
    base_url_override: Option<&str>,
) -> Result<OpenAiCompatible, Diagnostic> {
    resolve_openrouter_with_options(inputs, base_url_override, None)
}

/// Resolve the OpenRouter credential across the documented precedence layers and build a provider,
/// honoring an optional `base_url` override and an optional per-frame idle/stall timeout.
///
/// The `idle_timeout` is the transport's fail-closed stall guard: when set, a stalled upstream
/// stream is aborted within that bounded window with a transport diagnostic instead of hanging
/// (VAL-RUN-004). A `None` keeps the transport default. Credential resolution and redaction
/// guarantees are identical to [`resolve_openrouter`].
///
/// # Errors
/// Returns [`Diagnostic::NoCredential`] when no layer yields a key, or a transport-build diagnostic
/// if the client cannot be constructed.
pub fn resolve_openrouter_with_options(
    inputs: &CredentialInputs,
    base_url_override: Option<&str>,
    idle_timeout: Option<Duration>,
) -> Result<OpenAiCompatible, Diagnostic> {
    let credential = resolve_credential(OPENROUTER_ID, inputs)?;
    let base_url = match base_url_override.map(str::trim) {
        Some(override_url) if !override_url.is_empty() => override_url.to_owned(),
        _ => openrouter_base_url(),
    };
    openrouter_provider_with_options(&credential, &base_url, idle_timeout)
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

    #[test]
    fn provider_builds_against_an_explicit_base_url() {
        let inputs = CredentialInputs {
            env_value: Some("env-key".to_owned()),
            ..Default::default()
        };
        let credential = resolve_credential(OPENROUTER_ID, &inputs).expect("resolves");
        openrouter_provider_with_base_url(&credential, "http://127.0.0.1:3250")
            .expect("provider builds against an override base URL");
    }

    #[test]
    fn resolve_with_an_override_still_requires_a_credential() {
        let err =
            resolve_openrouter_with(&CredentialInputs::default(), Some("http://127.0.0.1:3250"))
                .expect_err("an endpoint override does not substitute for a credential");
        assert_eq!(err.kind(), "no-credential");
    }

    #[test]
    fn provider_builds_with_an_idle_timeout_override() {
        let inputs = CredentialInputs {
            env_value: Some("env-key".to_owned()),
            ..Default::default()
        };
        let credential = resolve_credential(OPENROUTER_ID, &inputs).expect("resolves");
        openrouter_provider_with_options(
            &credential,
            "http://127.0.0.1:3250",
            Some(Duration::from_millis(5)),
        )
        .expect("provider builds with a tiny idle budget");
        // Resolving through the full precedence path with a tiny idle budget also builds.
        resolve_openrouter_with_options(&inputs, None, Some(Duration::from_millis(5)))
            .expect("resolve with an idle-timeout override builds");
    }

    #[test]
    fn resolve_with_a_blank_override_falls_back_to_the_default_and_builds() {
        let inputs = CredentialInputs {
            env_value: Some("env-key".to_owned()),
            ..Default::default()
        };
        // A blank/whitespace override is treated as "no override": the provider still builds
        // (against the catalogue default) rather than failing on an empty URL.
        resolve_openrouter_with(&inputs, Some("   ")).expect("blank override falls back");
        resolve_openrouter_with(&inputs, None).expect("absent override uses the default");
    }
}
