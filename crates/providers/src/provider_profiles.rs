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

pub(crate) fn profile(provider_id: &str) -> Option<&'static ProviderProfile> {
    PROFILES
        .iter()
        .find(|profile| profile.provider_id == provider_id)
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
}
