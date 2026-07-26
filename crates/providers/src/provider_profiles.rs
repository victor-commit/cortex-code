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
        provider_id: "anthropic",
        models_dev_id: Some("anthropic"),
        display_name: "Anthropic",
        default_api_key_env: Some("ANTHROPIC_API_KEY"),
        default_base_url: None,
        default_protocol: "anthropic-messages",
        supports_oauth: false,
    },
    ProviderProfile {
        provider_id: "openai",
        models_dev_id: Some("openai"),
        display_name: "OpenAI",
        default_api_key_env: Some("OPENAI_API_KEY"),
        default_base_url: None,
        default_protocol: "openai-responses",
        supports_oauth: false,
    },
    ProviderProfile {
        provider_id: "google",
        models_dev_id: Some("google"),
        display_name: "Google",
        default_api_key_env: Some("GEMINI_API_KEY"),
        default_base_url: None,
        default_protocol: "gemini",
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
        provider_id: "chatgpt",
        models_dev_id: None,
        display_name: "ChatGPT",
        default_api_key_env: None,
        // Codex subscription backend (not api.openai.com). Stream path appends /responses.
        default_base_url: Some("https://chatgpt.com/backend-api/codex"),
        default_protocol: "openai-responses",
        supports_oauth: true,
    },
    ProviderProfile {
        provider_id: "xai",
        models_dev_id: Some("xai"),
        display_name: "xAI",
        default_api_key_env: Some("XAI_API_KEY"),
        default_base_url: None,
        default_protocol: "openai-chat",
        supports_oauth: true,
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
    ProviderProfile {
        provider_id: "lmstudio",
        models_dev_id: None,
        display_name: "LM Studio",
        default_api_key_env: None,
        default_base_url: Some("http://127.0.0.1:1234/v1"),
        default_protocol: "openai-chat",
        supports_oauth: false,
    },
    ProviderProfile {
        provider_id: "llamacpp",
        models_dev_id: None,
        display_name: "llama.cpp",
        default_api_key_env: None,
        default_base_url: Some("http://127.0.0.1:8080/v1"),
        default_protocol: "openai-chat",
        supports_oauth: false,
    },
    ProviderProfile {
        provider_id: "deepseek",
        models_dev_id: Some("deepseek"),
        display_name: "DeepSeek",
        default_api_key_env: Some("DEEPSEEK_API_KEY"),
        default_base_url: None,
        default_protocol: "openai-chat",
        supports_oauth: false,
    },
    ProviderProfile {
        provider_id: "moonshotai",
        models_dev_id: Some("moonshotai"),
        display_name: "Moonshot AI",
        default_api_key_env: Some("MOONSHOT_API_KEY"),
        default_base_url: None,
        default_protocol: "openai-chat",
        supports_oauth: false,
    },
    ProviderProfile {
        provider_id: "zai",
        models_dev_id: Some("zai"),
        display_name: "Z.AI",
        default_api_key_env: Some("ZAI_API_KEY"),
        default_base_url: None,
        default_protocol: "openai-chat",
        supports_oauth: false,
    },
    ProviderProfile {
        provider_id: "fireworks-ai",
        models_dev_id: Some("fireworks-ai"),
        display_name: "Fireworks AI",
        default_api_key_env: Some("FIREWORKS_API_KEY"),
        default_base_url: None,
        default_protocol: "openai-chat",
        supports_oauth: false,
    },
    ProviderProfile {
        provider_id: "huggingface",
        models_dev_id: Some("huggingface"),
        display_name: "Hugging Face",
        default_api_key_env: Some("HF_TOKEN"),
        default_base_url: None,
        default_protocol: "openai-chat",
        supports_oauth: false,
    },
    ProviderProfile {
        provider_id: "nvidia",
        models_dev_id: Some("nvidia"),
        display_name: "NVIDIA",
        default_api_key_env: Some("NVIDIA_API_KEY"),
        default_base_url: None,
        default_protocol: "openai-chat",
        supports_oauth: false,
    },
    ProviderProfile {
        provider_id: "baseten",
        models_dev_id: Some("baseten"),
        display_name: "Baseten",
        default_api_key_env: Some("BASETEN_API_KEY"),
        default_base_url: None,
        default_protocol: "openai-chat",
        supports_oauth: false,
    },
    ProviderProfile {
        provider_id: "minimax",
        models_dev_id: Some("minimax"),
        display_name: "MiniMax",
        default_api_key_env: Some("MINIMAX_API_KEY"),
        default_base_url: None,
        default_protocol: "openai-chat",
        supports_oauth: false,
    },
    ProviderProfile {
        provider_id: "nebius",
        models_dev_id: Some("nebius"),
        display_name: "Nebius",
        default_api_key_env: Some("NEBIUS_API_KEY"),
        default_base_url: None,
        default_protocol: "openai-chat",
        supports_oauth: false,
    },
    ProviderProfile {
        provider_id: "venice",
        models_dev_id: Some("venice"),
        display_name: "Venice",
        default_api_key_env: Some("VENICE_API_KEY"),
        default_base_url: None,
        default_protocol: "openai-chat",
        supports_oauth: false,
    },
];

/// Built-in OAuth endpoint defaults for an OAuth-capable provider. These are the real
/// authorization/token endpoints and public (non-secret) client id a provider advertises; the
/// `/connect` OAuth flow starts from these and lets `CORTEX_OAUTH_*` env vars override every field
/// (used to point the flow at a controlled loopback endpoint for mechanics testing).
///
/// Per-method extras (authorize query params, token body params) and extra request headers are
/// declarable here so subscription providers (e.g. ChatGPT) can pin non-standard OAuth knobs
/// without hard-coding them in the engine. Every extras field is overridable via
/// `CORTEX_OAUTH_<PROVIDER>_<FIELD>` (see [`oauth_endpoints`]).
#[derive(Clone, Copy, Debug)]
struct OAuthDefaults {
    provider_id: &'static str,
    authorize_endpoint: &'static str,
    token_endpoint: &'static str,
    /// RFC 8628 device authorization endpoint when the provider supports device-code login.
    /// `None` when only the browser authorization-code flow is offered.
    device_authorization_endpoint: Option<&'static str>,
    /// A public (non-secret) OAuth client id. Never a secret; the redirect_uri is loopback-only.
    client_id: &'static str,
    scopes: &'static [&'static str],
    /// Extra `key=value` query parameters appended to the authorization URL (e.g. ChatGPT
    /// `id_token_add_organizations` / `codex_cli_simplified_flow`).
    authorize_extra_params: &'static [(&'static str, &'static str)],
    /// Extra `key=value` form fields merged into the token-endpoint request body.
    token_extra_params: &'static [(&'static str, &'static str)],
    /// Extra HTTP headers sent on OAuth HTTP requests (authorize is browser-navigated; headers
    /// apply to token / device / refresh POSTs once the engine wires them).
    extra_request_headers: &'static [(&'static str, &'static str)],
    /// Optional product-pinned loopback callback port (e.g. ChatGPT Codex `1455`).
    callback_port: Option<u16>,
    /// Loopback callback path (`/callback` or product pin `/auth/callback`).
    callback_path: &'static str,
}

/// The built-in OAuth defaults, one per OAuth-capable provider profile. Only providers with
/// `supports_oauth = true` appear here; the flow refuses OAuth for a provider with neither a
/// built-in default nor a `CORTEX_OAUTH_*` override.
const OAUTH_DEFAULTS: &[OAuthDefaults] = &[
    OAuthDefaults {
        provider_id: "copilot",
        authorize_endpoint: "https://github.com/login/oauth/authorize",
        token_endpoint: "https://github.com/login/oauth/access_token",
        device_authorization_endpoint: None,
        client_id: "Iv1.b507a08c87ecfe98",
        scopes: &["read:user"],
        authorize_extra_params: &[],
        token_extra_params: &[],
        extra_request_headers: &[],
        callback_port: None,
        callback_path: "/callback",
    },
    OAuthDefaults {
        provider_id: "chatgpt",
        authorize_endpoint: "https://auth.openai.com/oauth/authorize",
        token_endpoint: "https://auth.openai.com/oauth/token",
        device_authorization_endpoint: None,
        client_id: "app_EMoamEEZ73f0CkXaXp7hrann",
        scopes: &["openid", "profile", "email", "offline_access"],
        authorize_extra_params: &[
            ("id_token_add_organizations", "true"),
            ("codex_cli_simplified_flow", "true"),
        ],
        token_extra_params: &[],
        extra_request_headers: &[],
        callback_port: Some(1455),
        callback_path: "/auth/callback",
    },
    OAuthDefaults {
        provider_id: "xai",
        authorize_endpoint: "https://auth.x.ai/oauth2/authorize",
        token_endpoint: "https://auth.x.ai/oauth2/token",
        device_authorization_endpoint: Some("https://auth.x.ai/oauth2/device/code"),
        client_id: "b1a00492-073a-47ea-816f-4c329264a828",
        scopes: &[
            "openid",
            "profile",
            "email",
            "offline_access",
            "grok-cli:access",
            "api:access",
        ],
        authorize_extra_params: &[],
        token_extra_params: &[],
        extra_request_headers: &[],
        callback_port: Some(56121),
        callback_path: "/callback",
    },
];

pub(crate) fn profile(provider_id: &str) -> Option<&'static ProviderProfile> {
    PROFILES
        .iter()
        .find(|profile| profile.provider_id == provider_id)
}

/// A resolved OAuth endpoint set for a provider's `/connect` OAuth method: the authorization and
/// token endpoints, the client id, the requested scopes, an optional pinned loopback callback
/// port, plus per-method extra auth params and extra request headers. Built from the provider's
/// built-in defaults, with every field overridable via `CORTEX_OAUTH_*` environment variables so a
/// controlled loopback OAuth endpoint can be pointed at for mechanics testing. Carries no secret
/// (the redirect is loopback-only and no client secret is required for the loopback
/// authorization-code flow).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OAuthEndpoints {
    /// The authorization endpoint the authorization URL is built against.
    pub authorize_endpoint: String,
    /// The token endpoint the authorization code is exchanged at.
    pub token_endpoint: String,
    /// RFC 8628 device authorization endpoint when device-code login is supported.
    pub device_authorization_endpoint: Option<String>,
    /// The OAuth client id presented to the authorization server (public, non-secret).
    pub client_id: String,
    /// The requested scopes (may be empty).
    pub scopes: Vec<String>,
    /// An explicit loopback callback port (3200-3299 or product pins 1455/56121); `None` scans the range.
    pub callback_port: Option<u16>,
    /// Loopback callback path (`/callback` or `/auth/callback`).
    pub callback_path: String,
    /// Extra query parameters for the authorization URL (`key`, `value`), in declaration order.
    pub authorize_extra_params: Vec<(String, String)>,
    /// Extra form body fields for the token request (`key`, `value`), in declaration order.
    pub token_extra_params: Vec<(String, String)>,
    /// Extra HTTP headers for OAuth token/device/refresh requests (`name`, `value`).
    pub extra_request_headers: Vec<(String, String)>,
}

/// Resolve the OAuth endpoint set for `provider_id`'s `/connect` OAuth method from the process
/// environment, or `None` when the provider does not advertise OAuth or no endpoints resolve.
///
/// Every field is overridable via `CORTEX_OAUTH_<PROVIDER>_<FIELD>` (e.g.
/// `CORTEX_OAUTH_COPILOT_AUTHORIZE_ENDPOINT`), falling back to the un-namespaced
/// `CORTEX_OAUTH_<FIELD>`, then the built-in default. This is the seam a controlled loopback OAuth
/// endpoint uses to exercise the real engine without a third-party provider.
///
/// # `CORTEX_OAUTH_*` field names
///
/// | Field suffix | Meaning | Value shape |
/// |---|---|---|
/// | `AUTHORIZE_ENDPOINT` | Authorization URL | absolute URL |
/// | `TOKEN_ENDPOINT` | Token URL | absolute URL |
/// | `DEVICE_AUTHORIZATION_ENDPOINT` | RFC 8628 device authorization URL | absolute URL |
/// | `CLIENT_ID` | Public OAuth client id | non-empty string |
/// | `SCOPES` | Space/comma-separated scopes | `openid,profile` |
/// | `CALLBACK_PORT` | Pinned loopback port | `3200`–`3299` or product pins `1455`/`56121` |
/// | `CALLBACK_PATH` | Loopback callback path | `/callback` or `/auth/callback` |
/// | `AUTHORIZE_EXTRA_PARAMS` | Extra authorize query params | `key=value,key2=value2` |
/// | `TOKEN_EXTRA_PARAMS` | Extra token body params | `key=value,key2=value2` |
/// | `EXTRA_HEADERS` | Extra OAuth request headers | `Header-Name=value,Other=value` |
///
/// Provider-specific keys win (`CORTEX_OAUTH_CHATGPT_AUTHORIZE_EXTRA_PARAMS`); generic keys
/// (`CORTEX_OAUTH_AUTHORIZE_EXTRA_PARAMS`) are the fallback. A set override **replaces** the
/// built-in extras list for that field (it does not merge).
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
    resolve_oauth_endpoints(provider_id, defaults, env)
}

/// Core resolution: merge `defaults` with `CORTEX_OAUTH_*` lookups. Used by
/// [`oauth_endpoints_with`] and by unit tests that inject synthetic [`OAuthDefaults`] (e.g. the
/// ChatGPT extras shape) without registering a full profile row yet.
fn resolve_oauth_endpoints(
    provider_id: &str,
    defaults: Option<&OAuthDefaults>,
    env: impl Fn(&str) -> Option<String>,
) -> Option<OAuthEndpoints> {
    let provider_key = env_provider_key(provider_id);
    let lookup = |suffix: &str| {
        oauth_non_empty(env(&format!("CORTEX_OAUTH_{provider_key}_{suffix}")).as_deref())
            .or_else(|| oauth_non_empty(env(&format!("CORTEX_OAUTH_{suffix}")).as_deref()))
    };

    let authorize_endpoint = lookup("AUTHORIZE_ENDPOINT")
        .or_else(|| defaults.map(|entry| entry.authorize_endpoint.to_owned()))?;
    let token_endpoint = lookup("TOKEN_ENDPOINT")
        .or_else(|| defaults.map(|entry| entry.token_endpoint.to_owned()))?;
    let device_authorization_endpoint = lookup("DEVICE_AUTHORIZATION_ENDPOINT").or_else(|| {
        defaults.and_then(|entry| {
            entry
                .device_authorization_endpoint
                .map(|endpoint| endpoint.to_owned())
        })
    });
    let client_id =
        lookup("CLIENT_ID").or_else(|| defaults.map(|entry| entry.client_id.to_owned()))?;
    let scopes = lookup("SCOPES").map(|raw| split_oauth_scopes(&raw)).or_else(|| {
        defaults.map(|entry| entry.scopes.iter().map(|scope| (*scope).to_owned()).collect())
    });
    let callback_port = lookup("CALLBACK_PORT")
        .and_then(|raw| raw.parse::<u16>().ok())
        .or_else(|| defaults.and_then(|entry| entry.callback_port));
    let callback_path = lookup("CALLBACK_PATH").unwrap_or_else(|| {
        defaults
            .map(|entry| entry.callback_path.to_owned())
            .unwrap_or_else(|| "/callback".to_owned())
    });

    let authorize_extra_params = lookup("AUTHORIZE_EXTRA_PARAMS")
        .map(|raw| parse_oauth_kv_list(&raw))
        .unwrap_or_else(|| {
            defaults
                .map(|entry| owned_oauth_pairs(entry.authorize_extra_params))
                .unwrap_or_default()
        });
    let token_extra_params = lookup("TOKEN_EXTRA_PARAMS")
        .map(|raw| parse_oauth_kv_list(&raw))
        .unwrap_or_else(|| {
            defaults
                .map(|entry| owned_oauth_pairs(entry.token_extra_params))
                .unwrap_or_default()
        });
    let extra_request_headers = lookup("EXTRA_HEADERS")
        .map(|raw| parse_oauth_kv_list(&raw))
        .unwrap_or_else(|| {
            defaults
                .map(|entry| owned_oauth_pairs(entry.extra_request_headers))
                .unwrap_or_default()
        });

    Some(OAuthEndpoints {
        authorize_endpoint,
        token_endpoint,
        device_authorization_endpoint,
        client_id,
        scopes: scopes.unwrap_or_default(),
        callback_port,
        callback_path,
        authorize_extra_params,
        token_extra_params,
        extra_request_headers,
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

/// Parse a comma-separated `key=value` list used by `CORTEX_OAUTH_*_EXTRA_*` / `EXTRA_HEADERS`.
/// Each pair splits on the first `=`; empty keys are dropped. Values may contain `=`.
fn parse_oauth_kv_list(raw: &str) -> Vec<(String, String)> {
    raw.split(',')
        .map(str::trim)
        .filter(|pair| !pair.is_empty())
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            let key = key.trim();
            if key.is_empty() {
                return None;
            }
            Some((key.to_owned(), value.trim().to_owned()))
        })
        .collect()
}

/// Copy a static `[(k, v)]` extras table into owned pairs.
fn owned_oauth_pairs(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
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
        let chatgpt = providers
            .iter()
            .find(|p| p.id == "chatgpt")
            .expect("chatgpt is connectable");
        assert!(chatgpt.supports_oauth, "chatgpt advertises OAuth");
        let xai = providers
            .iter()
            .find(|p| p.id == "xai")
            .expect("xai is connectable");
        assert!(xai.supports_oauth, "xai advertises xAI OAuth");
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

    #[test]
    fn anthropic_profile_pins_messages_protocol_and_env() {
        let p = profile("anthropic").expect("anthropic is a first-class profile");
        assert_eq!(p.default_protocol, "anthropic-messages");
        assert_eq!(p.default_api_key_env, Some("ANTHROPIC_API_KEY"));
    }

    #[test]
    fn openai_profile_pins_responses_protocol_and_env() {
        let p = profile("openai").expect("openai is a first-class profile");
        assert_eq!(p.default_protocol, "openai-responses");
        assert_eq!(p.default_api_key_env, Some("OPENAI_API_KEY"));
    }

    #[test]
    fn google_profile_pins_gemini_protocol_and_env() {
        let p = profile("google").expect("google is a first-class profile");
        assert_eq!(p.default_protocol, "gemini");
        assert_eq!(p.default_api_key_env, Some("GEMINI_API_KEY"));
    }

    #[test]
    fn openai_chat_provider_profiles_resolve_protocol_and_env() {
        // First-class OpenAI-Chat catalogue providers must pin protocol + a non-empty API-key env.
        let cases = [
            "deepseek",
            "moonshotai",
            "zai",
            "fireworks-ai",
            "huggingface",
            "nvidia",
            "baseten",
            "minimax",
            "nebius",
            "venice",
        ];
        for id in cases {
            let p = profile(id).unwrap_or_else(|| panic!("{id} is a first-class profile"));
            assert_eq!(
                p.default_protocol, "openai-chat",
                "{id} default_protocol"
            );
            let env = p
                .default_api_key_env
                .unwrap_or_else(|| panic!("{id} default_api_key_env is Some"));
            assert!(
                !env.is_empty(),
                "{id} default_api_key_env must be non-empty"
            );
        }
    }

    #[test]
    fn local_keyless_profiles_pin_loopback_base_url_without_api_key_env() {
        // Local OpenAI-compatible servers (like ollama) are keyless and need an explicit
        // loopback default_base_url — D11 exception for locals that models.dev does not host.
        let cases = [
            ("lmstudio", "http://127.0.0.1:1234/v1"),
            ("llamacpp", "http://127.0.0.1:8080/v1"),
        ];
        for (id, expected_base) in cases {
            let p = profile(id).unwrap_or_else(|| panic!("{id} is a first-class local profile"));
            assert_eq!(
                p.default_protocol, "openai-chat",
                "{id} default_protocol"
            );
            assert_eq!(
                p.default_api_key_env, None,
                "{id} is keyless (no required API-key env)"
            );
            assert_eq!(
                p.default_base_url,
                Some(expected_base),
                "{id} pins an explicit loopback base URL"
            );
            assert!(
                !p.supports_oauth,
                "{id} does not advertise OAuth"
            );
        }
    }

    #[test]
    fn oauth_defaults_declare_per_method_extras_and_headers() {
        // W4 ChatGPT shape: authorize extras + optional token extras + request headers are
        // declarable on OAuthDefaults without registering a full profile row yet.
        let defaults = OAuthDefaults {
            provider_id: "fixture-chatgpt",
            authorize_endpoint: "https://auth.openai.com/oauth/authorize",
            token_endpoint: "https://auth.openai.com/oauth/token",
            device_authorization_endpoint: None,
            client_id: "app_EMoamEEZ73f0CkXaXp7hrann",
            scopes: &["openid", "profile", "email", "offline_access"],
            authorize_extra_params: &[
                ("id_token_add_organizations", "true"),
                ("codex_cli_simplified_flow", "true"),
            ],
            token_extra_params: &[("audience", "https://api.openai.com/v1")],
            extra_request_headers: &[("OpenAI-Beta", "responses=experimental")],
            callback_port: Some(1455),
            callback_path: "/auth/callback",
        };
        let resolved = resolve_oauth_endpoints("fixture-chatgpt", Some(&defaults), |_| None)
            .expect("synthetic defaults resolve");
        assert_eq!(
            resolved.authorize_extra_params,
            vec![
                ("id_token_add_organizations".to_owned(), "true".to_owned()),
                ("codex_cli_simplified_flow".to_owned(), "true".to_owned()),
            ]
        );
        assert_eq!(
            resolved.token_extra_params,
            vec![("audience".to_owned(), "https://api.openai.com/v1".to_owned())]
        );
        assert_eq!(
            resolved.extra_request_headers,
            vec![("OpenAI-Beta".to_owned(), "responses=experimental".to_owned())]
        );
        assert_eq!(resolved.client_id, "app_EMoamEEZ73f0CkXaXp7hrann");
    }

    #[test]
    fn oauth_endpoints_extras_are_env_overridable_via_cortex_oauth_field() {
        // CORTEX_OAUTH_<PROVIDER>_<FIELD> replaces built-in extras (copilot defaults are empty).
        let env = |name: &str| match name {
            "CORTEX_OAUTH_COPILOT_AUTHORIZE_EXTRA_PARAMS" => Some(
                "id_token_add_organizations=true,codex_cli_simplified_flow=true".to_owned(),
            ),
            "CORTEX_OAUTH_COPILOT_TOKEN_EXTRA_PARAMS" => {
                Some("audience=https://api.example/v1".to_owned())
            }
            "CORTEX_OAUTH_COPILOT_EXTRA_HEADERS" => {
                Some("X-Test-Header=fixture,Accept=application/json".to_owned())
            }
            _ => None,
        };
        let resolved = oauth_endpoints_with("copilot", env).expect("override resolves");
        assert_eq!(
            resolved.authorize_extra_params,
            vec![
                ("id_token_add_organizations".to_owned(), "true".to_owned()),
                ("codex_cli_simplified_flow".to_owned(), "true".to_owned()),
            ]
        );
        assert_eq!(
            resolved.token_extra_params,
            vec![("audience".to_owned(), "https://api.example/v1".to_owned())]
        );
        assert_eq!(
            resolved.extra_request_headers,
            vec![
                ("X-Test-Header".to_owned(), "fixture".to_owned()),
                ("Accept".to_owned(), "application/json".to_owned()),
            ]
        );
        // Core endpoints still fall back to built-ins when un-overridden.
        assert_eq!(
            resolved.authorize_endpoint,
            "https://github.com/login/oauth/authorize"
        );
    }

    #[test]
    fn oauth_endpoints_extras_accept_generic_override_fallback() {
        let env = |name: &str| match name {
            "CORTEX_OAUTH_AUTHORIZE_EXTRA_PARAMS" => {
                Some("originator=cortex".to_owned())
            }
            _ => None,
        };
        let resolved = oauth_endpoints_with("copilot", env).expect("resolves");
        assert_eq!(
            resolved.authorize_extra_params,
            vec![("originator".to_owned(), "cortex".to_owned())]
        );
    }

    #[test]
    fn oauth_endpoints_extras_default_empty_when_unset() {
        // Copilot has no built-in extras; unset env yields empty vectors (backward compatible).
        let resolved = oauth_endpoints_with("copilot", |_| None).expect("copilot has OAuth");
        assert!(
            resolved.authorize_extra_params.is_empty(),
            "no authorize extras by default"
        );
        assert!(
            resolved.token_extra_params.is_empty(),
            "no token extras by default"
        );
        assert!(
            resolved.extra_request_headers.is_empty(),
            "no extra headers by default"
        );
    }

    #[test]
    fn oauth_endpoints_extras_provider_specific_wins_over_generic() {
        // Provider-namespaced CORTEX_OAUTH_<PROVIDER>_* beats un-namespaced CORTEX_OAUTH_*.
        let env = |name: &str| match name {
            "CORTEX_OAUTH_COPILOT_AUTHORIZE_EXTRA_PARAMS" => {
                Some("from=provider".to_owned())
            }
            "CORTEX_OAUTH_AUTHORIZE_EXTRA_PARAMS" => Some("from=generic".to_owned()),
            "CORTEX_OAUTH_COPILOT_EXTRA_HEADERS" => Some("X-From=provider".to_owned()),
            "CORTEX_OAUTH_EXTRA_HEADERS" => Some("X-From=generic".to_owned()),
            "CORTEX_OAUTH_COPILOT_TOKEN_EXTRA_PARAMS" => Some("tok=provider".to_owned()),
            "CORTEX_OAUTH_TOKEN_EXTRA_PARAMS" => Some("tok=generic".to_owned()),
            _ => None,
        };
        let resolved = oauth_endpoints_with("copilot", env).expect("resolves");
        assert_eq!(
            resolved.authorize_extra_params,
            vec![("from".to_owned(), "provider".to_owned())]
        );
        assert_eq!(
            resolved.token_extra_params,
            vec![("tok".to_owned(), "provider".to_owned())]
        );
        assert_eq!(
            resolved.extra_request_headers,
            vec![("X-From".to_owned(), "provider".to_owned())]
        );
    }

    #[test]
    fn parse_oauth_kv_list_splits_on_first_eq_and_drops_empty_keys() {
        assert_eq!(
            parse_oauth_kv_list("a=1,b=two=parts, =skip,c=,d"),
            vec![
                ("a".to_owned(), "1".to_owned()),
                ("b".to_owned(), "two=parts".to_owned()),
                ("c".to_owned(), String::new()),
            ]
        );
    }

    #[test]
    fn chatgpt_oauth_defaults_pin_codex_client_and_callback() {
        let resolved = oauth_endpoints_with("chatgpt", |_| None).expect("chatgpt has OAuth");
        assert_eq!(
            resolved.authorize_endpoint,
            "https://auth.openai.com/oauth/authorize"
        );
        assert_eq!(
            resolved.token_endpoint,
            "https://auth.openai.com/oauth/token"
        );
        assert_eq!(resolved.client_id, "app_EMoamEEZ73f0CkXaXp7hrann");
        assert_eq!(
            resolved.scopes,
            vec![
                "openid".to_owned(),
                "profile".to_owned(),
                "email".to_owned(),
                "offline_access".to_owned(),
            ]
        );
        assert_eq!(
            resolved.authorize_extra_params,
            vec![
                ("id_token_add_organizations".to_owned(), "true".to_owned()),
                ("codex_cli_simplified_flow".to_owned(), "true".to_owned()),
            ]
        );
        assert_eq!(resolved.callback_port, Some(1455));
        assert_eq!(resolved.callback_path, "/auth/callback");
    }

    #[test]
    fn xai_oauth_defaults_pin_xai_client_and_callback() {
        let resolved = oauth_endpoints_with("xai", |_| None).expect("xai has OAuth");
        assert_eq!(
            resolved.authorize_endpoint,
            "https://auth.x.ai/oauth2/authorize"
        );
        assert_eq!(resolved.token_endpoint, "https://auth.x.ai/oauth2/token");
        assert_eq!(
            resolved.device_authorization_endpoint.as_deref(),
            Some("https://auth.x.ai/oauth2/device/code")
        );
        assert_eq!(
            resolved.client_id,
            "b1a00492-073a-47ea-816f-4c329264a828"
        );
        assert_eq!(
            resolved.scopes,
            vec![
                "openid".to_owned(),
                "profile".to_owned(),
                "email".to_owned(),
                "offline_access".to_owned(),
                "grok-cli:access".to_owned(),
                "api:access".to_owned(),
            ]
        );
        assert_eq!(resolved.callback_port, Some(56121));
        assert_eq!(resolved.callback_path, "/callback");
        assert!(resolved.authorize_extra_params.is_empty());
    }
}
