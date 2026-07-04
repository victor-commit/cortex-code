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
}

const PROFILES: &[ProviderProfile] = &[
    ProviderProfile {
        provider_id: "openrouter",
        models_dev_id: Some("openrouter"),
        display_name: "OpenRouter",
        default_api_key_env: Some("OPENROUTER_API_KEY"),
        default_base_url: Some("https://openrouter.ai/api/v1"),
        default_protocol: "openai-chat",
    },
    ProviderProfile {
        provider_id: "bedrock",
        models_dev_id: Some("amazon-bedrock"),
        display_name: "Amazon Bedrock",
        default_api_key_env: Some("AWS_BEARER_TOKEN_BEDROCK"),
        default_base_url: None,
        default_protocol: "anthropic-messages",
    },
    ProviderProfile {
        provider_id: "azure",
        models_dev_id: Some("azure"),
        display_name: "Azure OpenAI",
        default_api_key_env: Some("AZURE_OPENAI_API_KEY"),
        default_base_url: None,
        default_protocol: "openai-chat",
    },
    ProviderProfile {
        provider_id: "vertex",
        models_dev_id: Some("google-vertex"),
        display_name: "Google Vertex AI",
        default_api_key_env: Some("GOOGLE_API_KEY"),
        default_base_url: None,
        default_protocol: "gemini",
    },
    ProviderProfile {
        provider_id: "copilot",
        models_dev_id: Some("github-copilot"),
        display_name: "GitHub Copilot",
        default_api_key_env: Some("GITHUB_COPILOT_TOKEN"),
        default_base_url: None,
        default_protocol: "openai-chat",
    },
    ProviderProfile {
        provider_id: "xai",
        models_dev_id: Some("xai"),
        display_name: "xAI",
        default_api_key_env: Some("XAI_API_KEY"),
        default_base_url: None,
        default_protocol: "openai-chat",
    },
    ProviderProfile {
        provider_id: "groq",
        models_dev_id: Some("groq"),
        display_name: "Groq",
        default_api_key_env: Some("GROQ_API_KEY"),
        default_base_url: None,
        default_protocol: "openai-chat",
    },
    ProviderProfile {
        provider_id: "mistral",
        models_dev_id: Some("mistral"),
        display_name: "Mistral",
        default_api_key_env: Some("MISTRAL_API_KEY"),
        default_base_url: None,
        default_protocol: "openai-chat",
    },
    ProviderProfile {
        provider_id: "cohere",
        models_dev_id: Some("cohere"),
        display_name: "Cohere",
        default_api_key_env: Some("COHERE_API_KEY"),
        default_base_url: None,
        default_protocol: "openai-chat",
    },
    ProviderProfile {
        provider_id: "deepinfra",
        models_dev_id: Some("deepinfra"),
        display_name: "DeepInfra",
        default_api_key_env: Some("DEEPINFRA_API_KEY"),
        default_base_url: None,
        default_protocol: "openai-chat",
    },
    ProviderProfile {
        provider_id: "cerebras",
        models_dev_id: Some("cerebras"),
        display_name: "Cerebras",
        default_api_key_env: Some("CEREBRAS_API_KEY"),
        default_base_url: None,
        default_protocol: "openai-chat",
    },
    ProviderProfile {
        provider_id: "perplexity",
        models_dev_id: Some("perplexity"),
        display_name: "Perplexity",
        default_api_key_env: Some("PERPLEXITY_API_KEY"),
        default_base_url: None,
        default_protocol: "openai-chat",
    },
    ProviderProfile {
        provider_id: "together",
        models_dev_id: Some("togetherai"),
        display_name: "Together",
        default_api_key_env: Some("TOGETHER_API_KEY"),
        default_base_url: None,
        default_protocol: "openai-chat",
    },
    ProviderProfile {
        provider_id: "vercel",
        models_dev_id: Some("vercel"),
        display_name: "Vercel",
        default_api_key_env: Some("VERCEL_API_KEY"),
        default_base_url: None,
        default_protocol: "openai-chat",
    },
    ProviderProfile {
        provider_id: "alibaba",
        models_dev_id: Some("alibaba"),
        display_name: "Alibaba",
        default_api_key_env: Some("ALIBABA_API_KEY"),
        default_base_url: None,
        default_protocol: "openai-chat",
    },
    ProviderProfile {
        provider_id: "cloudflare",
        models_dev_id: Some("cloudflare-workers-ai"),
        display_name: "Cloudflare",
        default_api_key_env: Some("CLOUDFLARE_API_TOKEN"),
        default_base_url: None,
        default_protocol: "openai-chat",
    },
    ProviderProfile {
        provider_id: "openai-compatible",
        models_dev_id: None,
        display_name: "OpenAI-Compatible",
        default_api_key_env: Some("OPENAI_API_KEY"),
        default_base_url: None,
        default_protocol: "openai-chat",
    },
    ProviderProfile {
        provider_id: "ollama",
        models_dev_id: Some("ollama-cloud"),
        display_name: "Ollama",
        default_api_key_env: None,
        default_base_url: Some("http://127.0.0.1:11434/v1"),
        default_protocol: "openai-chat",
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
