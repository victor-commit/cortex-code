//! Built-in provider profiles used for first-class runtime defaults and models.dev refresh
//! selection.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ProviderProfile {
    pub(crate) provider_id: &'static str,
    pub(crate) models_dev_id: Option<&'static str>,
    pub(crate) display_name: &'static str,
    pub(crate) default_api_key_env: Option<&'static str>,
    pub(crate) default_base_url: Option<&'static str>,
    pub(crate) default_protocol: &'static str,
    /// Whether this provider advertises an OAuth sign-in method (in addition to the always-present
    /// API-key method). Reflects the built-in OAuth capability the connect flow may offer.
    pub(crate) supports_oauth: bool,
}

const PROFILES: &[ProviderProfile] = &[
    ProviderProfile {
        provider_id: "openrouter",
        models_dev_id: Some("openrouter"),
        display_name: "OpenRouter",
        default_api_key_env: Some("OPENROUTER_API_KEY"),
        default_base_url: Some("https://openrouter.ai/api/v1"),
        default_protocol: "openai-chat",
        supports_oauth: false,
    },
    ProviderProfile {
        provider_id: "bedrock",
        models_dev_id: Some("amazon-bedrock"),
        display_name: "Amazon Bedrock",
        default_api_key_env: Some("AWS_BEARER_TOKEN_BEDROCK"),
        default_base_url: None,
        default_protocol: "anthropic-messages",
        supports_oauth: false,
    },
    ProviderProfile {
        provider_id: "azure",
        models_dev_id: Some("azure"),
        display_name: "Azure OpenAI",
        default_api_key_env: Some("AZURE_OPENAI_API_KEY"),
        default_base_url: None,
        default_protocol: "openai-chat",
        supports_oauth: false,
    },
    ProviderProfile {
        provider_id: "vertex",
        models_dev_id: Some("google-vertex"),
        display_name: "Google Vertex AI",
        default_api_key_env: Some("GOOGLE_API_KEY"),
        default_base_url: None,
        default_protocol: "gemini",
        supports_oauth: false,
    },
    ProviderProfile {
        provider_id: "copilot",
        models_dev_id: Some("github-copilot"),
        display_name: "GitHub Copilot",
        default_api_key_env: Some("GITHUB_COPILOT_TOKEN"),
        default_base_url: None,
        default_protocol: "openai-chat",
        supports_oauth: true,
    },
    ProviderProfile {
        provider_id: "xai",
        models_dev_id: Some("xai"),
        display_name: "xAI",
        default_api_key_env: Some("XAI_API_KEY"),
        default_base_url: None,
        default_protocol: "openai-chat",
        supports_oauth: false,
    },
    ProviderProfile {
        provider_id: "groq",
        models_dev_id: Some("groq"),
        display_name: "Groq",
        default_api_key_env: Some("GROQ_API_KEY"),
        default_base_url: None,
        default_protocol: "openai-chat",
        supports_oauth: false,
    },
    ProviderProfile {
        provider_id: "mistral",
        models_dev_id: Some("mistral"),
        display_name: "Mistral",
        default_api_key_env: Some("MISTRAL_API_KEY"),
        default_base_url: None,
        default_protocol: "openai-chat",
        supports_oauth: false,
    },
    ProviderProfile {
        provider_id: "cohere",
        models_dev_id: Some("cohere"),
        display_name: "Cohere",
        default_api_key_env: Some("COHERE_API_KEY"),
        default_base_url: None,
        default_protocol: "openai-chat",
        supports_oauth: false,
    },
    ProviderProfile {
        provider_id: "deepinfra",
        models_dev_id: Some("deepinfra"),
        display_name: "DeepInfra",
        default_api_key_env: Some("DEEPINFRA_API_KEY"),
        default_base_url: None,
        default_protocol: "openai-chat",
        supports_oauth: false,
    },
    ProviderProfile {
        provider_id: "cerebras",
        models_dev_id: Some("cerebras"),
        display_name: "Cerebras",
        default_api_key_env: Some("CEREBRAS_API_KEY"),
        default_base_url: None,
        default_protocol: "openai-chat",
        supports_oauth: false,
    },
    ProviderProfile {
        provider_id: "perplexity",
        models_dev_id: Some("perplexity"),
        display_name: "Perplexity",
        default_api_key_env: Some("PERPLEXITY_API_KEY"),
        default_base_url: None,
        default_protocol: "openai-chat",
        supports_oauth: false,
    },
    ProviderProfile {
        provider_id: "together",
        models_dev_id: Some("togetherai"),
        display_name: "Together",
        default_api_key_env: Some("TOGETHER_API_KEY"),
        default_base_url: None,
        default_protocol: "openai-chat",
        supports_oauth: false,
    },
    ProviderProfile {
        provider_id: "vercel",
        models_dev_id: Some("vercel"),
        display_name: "Vercel",
        default_api_key_env: Some("VERCEL_API_KEY"),
        default_base_url: None,
        default_protocol: "openai-chat",
        supports_oauth: false,
    },
    ProviderProfile {
        provider_id: "alibaba",
        models_dev_id: Some("alibaba"),
        display_name: "Alibaba",
        default_api_key_env: Some("ALIBABA_API_KEY"),
        default_base_url: None,
        default_protocol: "openai-chat",
        supports_oauth: false,
    },
    ProviderProfile {
        provider_id: "cloudflare",
        models_dev_id: Some("cloudflare-workers-ai"),
        display_name: "Cloudflare",
        default_api_key_env: Some("CLOUDFLARE_API_TOKEN"),
        default_base_url: None,
        default_protocol: "openai-chat",
        supports_oauth: false,
    },
    ProviderProfile {
        provider_id: "openai-compatible",
        models_dev_id: None,
        display_name: "OpenAI-Compatible",
        default_api_key_env: Some("OPENAI_API_KEY"),
        default_base_url: None,
        default_protocol: "openai-chat",
        supports_oauth: false,
    },
    ProviderProfile {
        provider_id: "ollama",
        models_dev_id: Some("ollama-cloud"),
        display_name: "Ollama",
        default_api_key_env: None,
        default_base_url: Some("http://127.0.0.1:11434/v1"),
        default_protocol: "openai-chat",
        supports_oauth: false,
    },
];

/// Built-in OAuth endpoint defaults for an OAuth-capable provider. These are the real
/// authorization/token endpoints and public (non-secret) client id a provider advertises; the
/// `/connect` OAuth flow starts from these and lets `CORTEX_OAUTH_*` env vars override every field
/// (used to point the flow at a controlled loopback endpoint for mechanics testing).
#[derive(Clone, Copy, Debug)]
struct OAuthDefaults {
    provider_id: &'static str,
    authorize_endpoint: &'static str,
    token_endpoint: &'static str,
    /// A public (non-secret) OAuth client id. Never a secret; the redirect_uri is loopback-only.
    client_id: &'static str,
    scopes: &'static [&'static str],
}

/// The built-in OAuth defaults, one per OAuth-capable provider profile. Only providers with
/// `supports_oauth = true` appear here; the flow refuses OAuth for a provider with neither a
/// built-in default nor a `CORTEX_OAUTH_*` override.
const OAUTH_DEFAULTS: &[OAuthDefaults] = &[OAuthDefaults {
    provider_id: "copilot",
    authorize_endpoint: "https://github.com/login/oauth/authorize",
    token_endpoint: "https://github.com/login/oauth/access_token",
    client_id: "Iv1.b507a08c87ecfe98",
    scopes: &["read:user"],
}];

pub(crate) fn profile(provider_id: &str) -> Option<&'static ProviderProfile> {
    PROFILES
        .iter()
        .find(|profile| profile.provider_id == provider_id)
}

/// A resolved OAuth endpoint set for a provider's `/connect` OAuth method: the authorization and
/// token endpoints, the client id, the requested scopes, and an optional pinned loopback callback
/// port. Built from the provider's built-in defaults, with every field overridable via
/// `CORTEX_OAUTH_*` environment variables so a controlled loopback OAuth endpoint can be pointed at
/// for mechanics testing. Carries no secret (the redirect is loopback-only and no client secret is
/// required for the loopback authorization-code flow).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OAuthEndpoints {
    /// The authorization endpoint the authorization URL is built against.
    pub authorize_endpoint: String,
    /// The token endpoint the authorization code is exchanged at.
    pub token_endpoint: String,
    /// The OAuth client id presented to the authorization server (public, non-secret).
    pub client_id: String,
    /// The requested scopes (may be empty).
    pub scopes: Vec<String>,
    /// An explicit loopback callback port (must be in 3200-3299); `None` scans the reserved range.
    pub callback_port: Option<u16>,
}

/// Resolve the OAuth endpoint set for `provider_id`'s `/connect` OAuth method from the process
/// environment, or `None` when the provider does not advertise OAuth or no endpoints resolve.
///
/// Every field is overridable via `CORTEX_OAUTH_<PROVIDER>_<FIELD>` (e.g.
/// `CORTEX_OAUTH_COPILOT_AUTHORIZE_ENDPOINT`), falling back to the un-namespaced
/// `CORTEX_OAUTH_<FIELD>`, then the built-in default. This is the seam a controlled loopback OAuth
/// endpoint uses to exercise the real engine without a third-party provider.
#[must_use]
pub fn oauth_endpoints(provider_id: &str) -> Option<OAuthEndpoints> {
    oauth_endpoints_with(provider_id, |name| std::env::var(name).ok())
}

/// Resolve OAuth endpoints for `provider_id` using an explicit `env` lookup (deterministic and
/// unit-testable). Provider-specific overrides win over generic overrides, which win over built-in
/// defaults.
fn oauth_endpoints_with(
    provider_id: &str,
    env: impl Fn(&str) -> Option<String>,
) -> Option<OAuthEndpoints> {
    let profile = profile(provider_id)?;
    if !profile.supports_oauth {
        return None;
    }
    let defaults = OAUTH_DEFAULTS
        .iter()
        .find(|entry| entry.provider_id == provider_id);

    let provider_key = env_provider_key(provider_id);
    let lookup = |suffix: &str| {
        oauth_non_empty(env(&format!("CORTEX_OAUTH_{provider_key}_{suffix}")).as_deref())
            .or_else(|| oauth_non_empty(env(&format!("CORTEX_OAUTH_{suffix}")).as_deref()))
    };

    let authorize_endpoint = lookup("AUTHORIZE_ENDPOINT")
        .or_else(|| defaults.map(|entry| entry.authorize_endpoint.to_owned()))?;
    let token_endpoint = lookup("TOKEN_ENDPOINT")
        .or_else(|| defaults.map(|entry| entry.token_endpoint.to_owned()))?;
    let client_id =
        lookup("CLIENT_ID").or_else(|| defaults.map(|entry| entry.client_id.to_owned()))?;
    let scopes = lookup("SCOPES").map(|raw| split_oauth_scopes(&raw)).or_else(|| {
        defaults.map(|entry| entry.scopes.iter().map(|scope| (*scope).to_owned()).collect())
    });
    let callback_port = lookup("CALLBACK_PORT").and_then(|raw| raw.parse::<u16>().ok());

    Some(OAuthEndpoints {
        authorize_endpoint,
        token_endpoint,
        client_id,
        scopes: scopes.unwrap_or_default(),
        callback_port,
    })
}

/// Uppercase a provider id into its `CORTEX_OAUTH_<KEY>_*` environment-variable segment,
/// mapping any non-alphanumeric character to `_` (so `openai-compatible` -> `OPENAI_COMPATIBLE`).
fn env_provider_key(provider_id: &str) -> String {
    provider_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

/// Split a comma/space-separated scopes string into non-empty scope tokens.
fn split_oauth_scopes(raw: &str) -> Vec<String> {
    raw.split([',', ' '])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Trim a candidate env value and keep it only if non-empty after trimming.
fn oauth_non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

/// Find the built-in profile whose `models_dev_id` matches `models_dev_id`, when one exists.
///
/// This maps a top-level models.dev provider id (for example `google-vertex` or `amazon-bedrock`)
/// onto the cortex provider profile that supplies its runtime defaults (env/base_url/protocol).
pub(crate) fn profile_by_models_dev_id(models_dev_id: &str) -> Option<&'static ProviderProfile> {
    PROFILES
        .iter()
        .find(|profile| profile.models_dev_id == Some(models_dev_id))
}

/// A provider the mid-session `/connect` flow can connect to: its stable id (the credential-store
/// key), a human display name, the primary API-key environment variable (a real credential source),
/// and whether it advertises an OAuth sign-in method in addition to the always-present API key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConnectableProvider {
    /// The provider id used as the credential-store key and connected-state lookup.
    pub id: &'static str,
    /// The provider's human display name.
    pub display_name: &'static str,
    /// The provider's primary API-key environment variable, when it has one.
    pub api_key_env: Option<&'static str>,
    /// Whether the provider offers an OAuth method (API key is always offered regardless).
    pub supports_oauth: bool,
}

/// The providers offered in the `/connect` flow: every built-in profile EXCEPT the excluded Zen
/// provider, in profile (display) order. The Zen provider is never a member of the profile table,
/// so this filter is a defensive guarantee that the exclusion holds everywhere.
#[must_use]
pub fn connectable_providers() -> Vec<ConnectableProvider> {
    PROFILES
        .iter()
        .filter(|profile| !crate::catalogue::is_excluded_zen(profile.provider_id))
        .map(|profile| ConnectableProvider {
            id: profile.provider_id,
            display_name: profile.display_name,
            api_key_env: profile.default_api_key_env,
            supports_oauth: profile.supports_oauth,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connectable_providers_lists_profiles_without_zen() {
        let providers = connectable_providers();
        assert_eq!(
            providers.len(),
            PROFILES.len(),
            "every profile is connectable (Zen is not a profile, so nothing is dropped)"
        );
        assert!(
            providers.iter().any(|p| p.id == "openrouter"),
            "openrouter is connectable"
        );
        // The excluded Zen provider is absent under either of its ids.
        for provider in &providers {
            assert!(
                !crate::catalogue::is_excluded_zen(provider.id),
                "no connectable provider is the excluded Zen provider: {}",
                provider.id
            );
        }
    }

    #[test]
    fn connectable_providers_reflect_oauth_capability() {
        let providers = connectable_providers();
        let copilot = providers
            .iter()
            .find(|p| p.id == "copilot")
            .expect("copilot is connectable");
        assert!(copilot.supports_oauth, "copilot advertises OAuth");
        let openrouter = providers
            .iter()
            .find(|p| p.id == "openrouter")
            .expect("openrouter is connectable");
        assert!(
            !openrouter.supports_oauth,
            "openrouter is API-key only (no OAuth method)"
        );
        assert_eq!(openrouter.api_key_env, Some("OPENROUTER_API_KEY"));
    }

    #[test]
    fn oauth_endpoints_use_builtin_defaults_when_unoverridden() {
        // An OAuth-capable provider resolves its built-in authorization/token endpoints + client id
        // when no env override is present.
        let resolved = oauth_endpoints_with("copilot", |_| None).expect("copilot has OAuth");
        assert_eq!(
            resolved.authorize_endpoint,
            "https://github.com/login/oauth/authorize"
        );
        assert_eq!(
            resolved.token_endpoint,
            "https://github.com/login/oauth/access_token"
        );
        assert!(!resolved.client_id.is_empty(), "a client id is present");
        assert_eq!(resolved.callback_port, None, "port scans the range by default");
    }

    #[test]
    fn oauth_endpoints_reject_providers_without_oauth() {
        // An api-key-only provider has no OAuth method, so nothing resolves.
        assert!(
            oauth_endpoints_with("openrouter", |_| Some("x".to_owned())).is_none(),
            "openrouter is api-key only"
        );
    }

    #[test]
    fn oauth_endpoints_are_env_overridable_to_a_loopback_endpoint() {
        // A controlled loopback OAuth endpoint overrides every field via CORTEX_OAUTH_<PROVIDER>_*.
        let env = |name: &str| match name {
            "CORTEX_OAUTH_COPILOT_AUTHORIZE_ENDPOINT" => {
                Some("http://127.0.0.1:3271/authorize".to_owned())
            }
            "CORTEX_OAUTH_COPILOT_TOKEN_ENDPOINT" => Some("http://127.0.0.1:3271/token".to_owned()),
            "CORTEX_OAUTH_COPILOT_CLIENT_ID" => Some("fixture-client".to_owned()),
            "CORTEX_OAUTH_COPILOT_SCOPES" => Some("read:user, models".to_owned()),
            "CORTEX_OAUTH_COPILOT_CALLBACK_PORT" => Some("3242".to_owned()),
            _ => None,
        };
        let resolved = oauth_endpoints_with("copilot", env).expect("override resolves");
        assert_eq!(resolved.authorize_endpoint, "http://127.0.0.1:3271/authorize");
        assert_eq!(resolved.token_endpoint, "http://127.0.0.1:3271/token");
        assert_eq!(resolved.client_id, "fixture-client");
        assert_eq!(
            resolved.scopes,
            vec!["read:user".to_owned(), "models".to_owned()]
        );
        assert_eq!(resolved.callback_port, Some(3242));
    }

    #[test]
    fn oauth_endpoints_accept_the_generic_override_fallback() {
        // The un-namespaced CORTEX_OAUTH_<FIELD> override applies when no provider-specific key set.
        let env = |name: &str| match name {
            "CORTEX_OAUTH_TOKEN_ENDPOINT" => Some("http://127.0.0.1:3272/token".to_owned()),
            _ => None,
        };
        let resolved = oauth_endpoints_with("copilot", env).expect("resolves");
        assert_eq!(resolved.token_endpoint, "http://127.0.0.1:3272/token");
        // The un-overridden fields still fall back to the built-in defaults.
        assert_eq!(
            resolved.authorize_endpoint,
            "https://github.com/login/oauth/authorize"
        );
    }

    #[test]
    fn env_provider_key_uppercases_and_sanitizes() {
        assert_eq!(env_provider_key("copilot"), "COPILOT");
        assert_eq!(env_provider_key("openai-compatible"), "OPENAI_COMPATIBLE");
    }
}
