//! `cortex auth`: login/list/logout for the local provider credential store.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, IsTerminal, Read, Write};
use std::time::Duration;

use errors::Exit;
use plugins::{PluginAuthMethodType, PluginAuthPrompt};
use providers::{
    AuthPathInputs, list_stored_providers, remove_stored_credential, store_credential,
};
use redaction::PLACEHOLDER;
use serde::Serialize;
use serde_json::Value as JsonValue;
use settings::ConfigInputs;

use crate::account_store::{forget_account, list_accounts, record_account};

use crate::cli::{
    AuthArgs, AuthCommand, AuthListArgs, AuthLoginArgs, AuthLogoutArgs, OutputFormat,
};
use crate::plugin_bridge::{PluginAuthMethodDoc, PluginState};

pub fn run(args: AuthArgs, pure_plugins: bool) -> Exit {
    let plugin_state = match PluginState::from_process_config_for_surface(pure_plugins, "auth") {
        Ok(state) => state,
        Err(err) => {
            eprintln!("cortex auth: {err}");
            return Exit::Failure;
        }
    };

    match args.command {
        AuthCommand::Login(args) => login(args, &plugin_state),
        AuthCommand::List(args) => list(args, &plugin_state),
        AuthCommand::Logout(args) => logout(args),
    }
}

fn login(args: AuthLoginArgs, plugin_state: &PluginState) -> Exit {
    let Some(provider) = normalized_provider(&args.provider) else {
        eprintln!("cortex auth login: provider must be non-empty");
        return Exit::Usage;
    };
    let Some(path) = auth_store_path() else {
        eprintln!(
            "cortex auth login: cannot determine auth-store path; set HOME, XDG_DATA_HOME, CORTEX_DATA_DIR, or CORTEX_AUTH_FILE"
        );
        return Exit::Failure;
    };

    let available_methods = available_methods(plugin_state);
    let selected_method =
        match select_auth_method(&provider, args.auth_method.as_deref(), &available_methods) {
            Ok(method) => method,
            Err(message) => {
                eprintln!("cortex auth login: {message}");
                return Exit::Usage;
            }
        };

    let credential = if let Some(method) = selected_method.as_ref() {
        let inputs = match gather_method_inputs(&args, &provider, method) {
            Ok(inputs) => inputs,
            Err(message) => {
                eprintln!("cortex auth login: {message}");
                return Exit::MissingInput;
            }
        };
        match method.method_type {
            AuthMethodType::Api => match authorize_api_method(plugin_state, method, inputs) {
                Ok(credential) => credential,
                Err(message) => {
                    eprintln!("cortex auth login: {message}");
                    return Exit::Failure;
                }
            },
            AuthMethodType::Oauth => {
                match authorize_oauth_method(plugin_state, method, inputs, &args) {
                    Ok(credential) => credential,
                    Err(message) => {
                        eprintln!("cortex auth login: {message}");
                        return Exit::Failure;
                    }
                }
            }
        }
    } else {
        let Some(secret) = resolve_login_secret(&args, &provider) else {
            eprintln!(
                "cortex auth login: no API key provided (use --api-key, --api-key-env, {default_env}, or pipe on stdin)",
                default_env = provider_default_env(&provider).unwrap_or_else(|| "<env>".to_owned())
            );
            return Exit::MissingInput;
        };
        JsonValue::String(secret)
    };

    let source = selected_method.as_ref().map_or_else(
        || credential_source(&args, &provider),
        AuthMethodDescriptor::source_label,
    );

    tracing::debug!(
        target: "cortex::auth",
        provider = %provider,
        source = %source,
        api_key = PLACEHOLDER,
        "received credential for auth login"
    );

    let preserve_structured = selected_method.as_ref().is_some_and(|method| {
        method.source == "plugin" || method.method_type == AuthMethodType::Oauth
    });
    if let Err(error) = store_auth_payload(&path, &provider, &credential, preserve_structured) {
        eprintln!(
            "cortex auth login: failed to persist credential for provider `{provider}`: {error}"
        );
        return Exit::Failure;
    }

    let method_id = selected_method
        .as_ref()
        .map(|method| method.method_id.as_str());
    let label = selected_method
        .as_ref()
        .map(|method| method.provider_name.as_str());
    // Mirror the credential in a durable account record (metadata only; the 0600 store holds the
    // secret). Best-effort: a database hiccup must not fail an otherwise-successful login.
    record_account(&provider, label, method_id, &source);
    let document = AuthMutationDocument {
        provider: &provider,
        stored: true,
        removed: false,
        source: &source,
        method: method_id,
        message: "credential stored",
    };
    emit_mutation(
        args.format.format,
        &document,
        &format!(
            "stored credential for provider `{provider}` in {} (source: {source})\n",
            path.display()
        ),
    )
}

fn list(args: AuthListArgs, plugin_state: &PluginState) -> Exit {
    let providers = match auth_store_path() {
        Some(path) => match list_stored_providers(&path) {
            Ok(stored) => stored
                .into_iter()
                .map(|id| StoredProviderDocument { id, source: "file" })
                .collect::<Vec<_>>(),
            Err(error) => {
                eprintln!("cortex auth list: failed to read auth store: {error}");
                return Exit::Failure;
            }
        },
        None => Vec::new(),
    };
    let logged_in = providers
        .iter()
        .map(|provider| provider.id.clone())
        .collect::<BTreeSet<_>>();
    let methods = available_methods(plugin_state)
        .into_iter()
        .map(|method| AuthMethodListDocument {
            logged_in: logged_in.contains(&method.provider_id),
            provider: method.provider_id,
            provider_name: method.provider_name,
            method: method.method_id,
            method_type: method.method_type,
            label: method.label,
            source: method.source,
            plugin: method.plugin,
        })
        .collect::<Vec<_>>();

    let accounts = list_accounts();

    match args.format.format {
        OutputFormat::Text => {
            let mut out = String::new();
            for provider in &providers {
                out.push_str(&provider.id);
                out.push('\n');
            }
            if !accounts.is_empty() {
                out.push('\n');
                out.push_str("accounts:\n");
                for account in &accounts {
                    let source = account.source.as_deref().unwrap_or("-");
                    let method = account.method.as_deref().unwrap_or("-");
                    out.push_str(&format!("{} [{method}:{source}]\n", account.provider));
                }
            }
            if !methods.is_empty() {
                out.push('\n');
                out.push_str("available auth methods:\n");
                for method in &methods {
                    let logged = if method.logged_in { " (logged in)" } else { "" };
                    out.push_str(&format!(
                        "{}:{} [{}:{}]{}\n",
                        method.provider, method.method, method.source, method.method_type, logged
                    ));
                }
            }
            emit_text(&out)
        }
        OutputFormat::Json | OutputFormat::Jsonl => emit_json(&AuthListDocument {
            providers,
            accounts,
            methods,
        }),
    }
}

fn logout(args: AuthLogoutArgs) -> Exit {
    let Some(provider) = normalized_provider(&args.provider) else {
        eprintln!("cortex auth logout: provider must be non-empty");
        return Exit::Usage;
    };
    let removed = match auth_store_path() {
        Some(path) => match remove_stored_credential(&path, &provider) {
            Ok(removed) => removed,
            Err(error) => {
                eprintln!(
                    "cortex auth logout: failed to update auth store for provider `{provider}`: {error}"
                );
                return Exit::Failure;
            }
        },
        None => false,
    };
    // Mirror credential removal in the durable account records (best-effort).
    forget_account(&provider);

    let (message, source) = if removed {
        ("credential removed", "file")
    } else {
        ("provider not logged in (nothing to remove)", "none")
    };
    let document = AuthMutationDocument {
        provider: &provider,
        stored: false,
        removed,
        source,
        method: None,
        message,
    };

    let text = if removed {
        format!("removed stored credential for provider `{provider}`\n")
    } else {
        format!("provider `{provider}` is not logged in (nothing to remove)\n")
    };
    emit_mutation(args.format.format, &document, &text)
}

fn auth_store_path() -> Option<std::path::PathBuf> {
    AuthPathInputs::from_process().resolve()
}

fn normalized_provider(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_ascii_lowercase())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum AuthMethodType {
    Api,
    Oauth,
}

impl std::fmt::Display for AuthMethodType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Api => f.write_str("api"),
            Self::Oauth => f.write_str("oauth"),
        }
    }
}

#[derive(Clone, Debug)]
enum AuthPromptDescriptor {
    Text {
        key: String,
        message: String,
        placeholder: Option<String>,
    },
    Select {
        key: String,
        message: String,
        options: Vec<AuthPromptOptionDescriptor>,
    },
}

#[derive(Clone, Debug)]
struct AuthPromptOptionDescriptor {
    label: String,
    value: String,
}

#[derive(Clone, Debug)]
struct AuthMethodDescriptor {
    source: &'static str,
    plugin: Option<String>,
    provider_id: String,
    provider_name: String,
    method_id: String,
    method_type: AuthMethodType,
    label: String,
    prompts: Vec<AuthPromptDescriptor>,
    env: Vec<String>,
}

impl AuthMethodDescriptor {
    fn source_label(&self) -> String {
        match &self.plugin {
            Some(plugin) => format!("plugin:{plugin}:{}", self.method_id),
            None => format!("{}:{}", self.source, self.method_id),
        }
    }
}

fn available_methods(plugin_state: &PluginState) -> Vec<AuthMethodDescriptor> {
    let mut methods = plugin_state
        .auth_methods()
        .into_iter()
        .map(AuthMethodDescriptor::from_plugin)
        .collect::<Vec<_>>();

    for method in builtin_auth_methods() {
        if methods.iter().any(|existing| {
            existing.provider_id == method.provider_id && existing.method_id == method.method_id
        }) {
            continue;
        }
        methods.push(method);
    }
    methods
}

fn builtin_auth_methods() -> Vec<AuthMethodDescriptor> {
    vec![
        builtin_api_method("copilot", "GitHub Copilot", &["GITHUB_COPILOT_TOKEN"]),
        builtin_oauth_method("copilot", "GitHub Copilot"),
        builtin_api_method("codex", "OpenAI Codex", &["OPENAI_API_KEY"]),
        builtin_api_method("gitlab", "GitLab", &["GITLAB_TOKEN", "GITLAB_API_TOKEN"]),
        builtin_api_method("azure", "Azure OpenAI", &["AZURE_OPENAI_API_KEY"]),
        builtin_api_method("xai", "xAI", &["XAI_API_KEY"]),
    ]
}

fn builtin_api_method(
    provider_id: &str,
    provider_name: &str,
    env: &[&str],
) -> AuthMethodDescriptor {
    AuthMethodDescriptor {
        source: "builtin",
        plugin: None,
        provider_id: provider_id.to_owned(),
        provider_name: provider_name.to_owned(),
        method_id: "api".to_owned(),
        method_type: AuthMethodType::Api,
        label: "API key".to_owned(),
        prompts: vec![AuthPromptDescriptor::Text {
            key: "api_key".to_owned(),
            message: "API key".to_owned(),
            placeholder: None,
        }],
        env: env.iter().map(|value| (*value).to_owned()).collect(),
    }
}

fn builtin_oauth_method(provider_id: &str, provider_name: &str) -> AuthMethodDescriptor {
    AuthMethodDescriptor {
        source: "builtin",
        plugin: None,
        provider_id: provider_id.to_owned(),
        provider_name: provider_name.to_owned(),
        method_id: "oauth".to_owned(),
        method_type: AuthMethodType::Oauth,
        label: "OAuth authorization code".to_owned(),
        prompts: vec![
            AuthPromptDescriptor::Text {
                key: "client_id".to_owned(),
                message: "OAuth client id".to_owned(),
                placeholder: None,
            },
            AuthPromptDescriptor::Text {
                key: "client_secret".to_owned(),
                message: "OAuth client secret".to_owned(),
                placeholder: Some("optional".to_owned()),
            },
            AuthPromptDescriptor::Text {
                key: "authorize_endpoint".to_owned(),
                message: "OAuth authorize endpoint".to_owned(),
                placeholder: None,
            },
            AuthPromptDescriptor::Text {
                key: "token_endpoint".to_owned(),
                message: "OAuth token endpoint".to_owned(),
                placeholder: None,
            },
        ],
        env: vec![],
    }
}

impl AuthMethodDescriptor {
    fn from_plugin(method: PluginAuthMethodDoc) -> Self {
        Self {
            source: "plugin",
            plugin: Some(method.plugin),
            provider_id: method.provider_id,
            provider_name: method.provider_name,
            method_id: method.method.id,
            method_type: match method.method.method_type {
                PluginAuthMethodType::Api => AuthMethodType::Api,
                PluginAuthMethodType::Oauth => AuthMethodType::Oauth,
            },
            label: method.method.label,
            prompts: method
                .method
                .prompts
                .into_iter()
                .filter_map(prompt_from_plugin)
                .collect(),
            env: method.method.env,
        }
    }
}

fn prompt_from_plugin(prompt: PluginAuthPrompt) -> Option<AuthPromptDescriptor> {
    match prompt {
        PluginAuthPrompt::Text {
            key,
            message,
            placeholder,
        } => Some(AuthPromptDescriptor::Text {
            key,
            message,
            placeholder,
        }),
        PluginAuthPrompt::Select {
            key,
            message,
            options,
        } => {
            let options = options
                .into_iter()
                .map(|option| AuthPromptOptionDescriptor {
                    label: option.label,
                    value: option.value,
                })
                .collect::<Vec<_>>();
            if options.is_empty() {
                None
            } else {
                Some(AuthPromptDescriptor::Select {
                    key,
                    message,
                    options,
                })
            }
        }
    }
}

fn select_auth_method(
    provider: &str,
    requested: Option<&str>,
    methods: &[AuthMethodDescriptor],
) -> Result<Option<AuthMethodDescriptor>, String> {
    let mut candidates = methods
        .iter()
        .filter(|method| method.provider_id == provider)
        .cloned()
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return Ok(None);
    }

    if let Some(requested) = requested.map(str::trim).filter(|value| !value.is_empty()) {
        let requested_lower = requested.to_ascii_lowercase();
        let Some(selected) = candidates.into_iter().find(|method| {
            method.method_id.eq_ignore_ascii_case(&requested_lower)
                || method
                    .method_type
                    .to_string()
                    .eq_ignore_ascii_case(&requested_lower)
        }) else {
            return Err(format!(
                "provider `{provider}` has no auth method `{requested}`"
            ));
        };
        return Ok(Some(selected));
    }

    if let Some(api) = candidates
        .iter()
        .find(|method| method.method_type == AuthMethodType::Api)
    {
        return Ok(Some(api.clone()));
    }
    Ok(candidates.drain(..1).next())
}

fn gather_method_inputs(
    args: &AuthLoginArgs,
    provider: &str,
    method: &AuthMethodDescriptor,
) -> Result<BTreeMap<String, String>, String> {
    let mut inputs = parse_auth_inputs(&args.auth_inputs)?;
    if let Some(api_key) = args
        .api_key
        .as_deref()
        .and_then(|value| non_empty(Some(value)))
    {
        inputs.entry("api_key".to_owned()).or_insert(api_key);
    }
    if let Some(env_name) = args
        .api_key_env
        .as_deref()
        .and_then(|value| non_empty(Some(value)))
        && let Ok(value) = std::env::var(&env_name)
        && let Some(secret) = non_empty(Some(&value))
    {
        inputs.entry("api_key".to_owned()).or_insert(secret);
    }

    for prompt in &method.prompts {
        let key = match prompt {
            AuthPromptDescriptor::Text { key, .. } | AuthPromptDescriptor::Select { key, .. } => {
                key
            }
        };
        if inputs.contains_key(key) {
            continue;
        }
        if method.source == "builtin" && key == "client_secret" {
            continue;
        }
        if key.eq_ignore_ascii_case("api_key") || key.eq_ignore_ascii_case("token") {
            if let Some(secret) =
                resolve_login_secret(args, provider).or_else(|| env_secret_for_method(method))
            {
                inputs.insert(key.to_owned(), secret);
                continue;
            }
        }
        let value = prompt_for_value(prompt)
            .map_err(|err| err.to_string())?
            .ok_or_else(|| {
            format!(
                "missing required auth input `{key}` (pass --auth-input {key}=... or provide interactive stdin)"
            )
        })?;
        inputs.insert(key.to_owned(), value);
    }

    if method.method_type == AuthMethodType::Api
        && !inputs.contains_key("api_key")
        && !inputs.contains_key("token")
    {
        if let Some(secret) =
            resolve_login_secret(args, provider).or_else(|| env_secret_for_method(method))
        {
            inputs.insert("api_key".to_owned(), secret);
        }
    }
    Ok(inputs)
}

fn parse_auth_inputs(values: &[String]) -> Result<BTreeMap<String, String>, String> {
    let mut map = BTreeMap::new();
    for value in values {
        let Some((key, raw)) = value.split_once('=') else {
            return Err(format!(
                "invalid --auth-input `{value}` (expected KEY=VALUE)"
            ));
        };
        let key = key.trim();
        let raw = raw.trim();
        if key.is_empty() || raw.is_empty() {
            return Err(format!(
                "invalid --auth-input `{value}` (expected KEY=VALUE)"
            ));
        }
        map.insert(key.to_owned(), raw.to_owned());
    }
    Ok(map)
}

fn required_auth_input(inputs: &BTreeMap<String, String>, key: &str) -> Result<String, String> {
    non_empty(inputs.get(key).map(String::as_str))
        .ok_or_else(|| format!("missing required auth input `{key}` (pass --auth-input {key}=...)"))
}

fn parse_optional_callback_port(value: Option<&String>) -> Result<Option<u16>, String> {
    let Some(value) = value.and_then(|value| non_empty(Some(value.as_str()))) else {
        return Ok(None);
    };
    let port = value
        .parse::<u16>()
        .map_err(|_| format!("invalid callback_port `{value}` (expected integer 3200-3299)"))?;
    Ok(Some(port))
}

fn parse_optional_timeout_seconds(value: Option<&String>) -> Result<Duration, String> {
    let Some(value) = value.and_then(|value| non_empty(Some(value.as_str()))) else {
        return Ok(Duration::from_secs(120));
    };
    let seconds = value
        .parse::<u64>()
        .map_err(|_| format!("invalid timeout_seconds `{value}` (expected integer >= 1)"))?;
    if seconds == 0 {
        return Err("invalid timeout_seconds `0` (expected integer >= 1)".to_owned());
    }
    Ok(Duration::from_secs(seconds))
}

fn parse_scope_inputs(inputs: &BTreeMap<String, String>) -> Vec<String> {
    inputs
        .get("scopes")
        .or_else(|| inputs.get("scope"))
        .map(|raw| {
            raw.split([',', ' '])
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn prompt_for_value(prompt: &AuthPromptDescriptor) -> io::Result<Option<String>> {
    let stdin = io::stdin();
    if !stdin.is_terminal() {
        return Ok(None);
    }
    match prompt {
        AuthPromptDescriptor::Text {
            message,
            placeholder,
            ..
        } => {
            let mut stdout = io::stdout().lock();
            if let Some(placeholder) = placeholder {
                write!(stdout, "{message} [{placeholder}]: ")?;
            } else {
                write!(stdout, "{message}: ")?;
            }
            stdout.flush()?;
            let mut line = String::new();
            stdin.read_line(&mut line)?;
            Ok(non_empty(Some(&line)))
        }
        AuthPromptDescriptor::Select {
            message, options, ..
        } => {
            let mut stdout = io::stdout().lock();
            writeln!(stdout, "{message}")?;
            for (index, option) in options.iter().enumerate() {
                writeln!(stdout, "  {}. {}", index + 1, option.label)?;
            }
            write!(stdout, "select option: ")?;
            stdout.flush()?;
            let mut line = String::new();
            stdin.read_line(&mut line)?;
            let selected = non_empty(Some(&line));
            let Some(selected) = selected else {
                return Ok(None);
            };
            if let Ok(index) = selected.parse::<usize>()
                && let Some(option) = options.get(index.saturating_sub(1))
            {
                return Ok(Some(option.value.clone()));
            }
            Ok(options
                .iter()
                .find(|option| option.value == selected || option.label == selected)
                .map(|option| option.value.clone()))
        }
    }
}

fn env_secret_for_method(method: &AuthMethodDescriptor) -> Option<String> {
    method.env.iter().find_map(|name| {
        std::env::var(name)
            .ok()
            .and_then(|value| non_empty(Some(&value)))
    })
}

fn authorize_api_method(
    plugin_state: &PluginState,
    method: &AuthMethodDescriptor,
    inputs: BTreeMap<String, String>,
) -> Result<JsonValue, String> {
    let input_json = JsonValue::Object(
        inputs
            .iter()
            .map(|(key, value)| (key.clone(), JsonValue::String(value.clone())))
            .collect(),
    );
    if method.source == "plugin" {
        let response = plugin_state.authorize_auth_method(
            &method.provider_id,
            &method.method_id,
            input_json,
        )?;
        parse_credential_response(response)
    } else {
        let key = inputs
            .get("api_key")
            .or_else(|| inputs.get("token"))
            .or_else(|| inputs.get("key"))
            .cloned()
            .ok_or_else(|| {
                format!(
                    "provider `{}` requires an API key (use --api-key or --auth-input api_key=...)",
                    method.provider_id
                )
            })?;
        Ok(JsonValue::String(key))
    }
}

fn authorize_oauth_method(
    plugin_state: &PluginState,
    method: &AuthMethodDescriptor,
    inputs: BTreeMap<String, String>,
    args: &AuthLoginArgs,
) -> Result<JsonValue, String> {
    if method.source != "plugin" {
        return authorize_builtin_oauth_method(method, inputs, args);
    }

    let input_json = JsonValue::Object(
        inputs
            .iter()
            .map(|(key, value)| (key.clone(), JsonValue::String(value.clone())))
            .collect(),
    );
    let response =
        plugin_state.authorize_auth_method(&method.provider_id, &method.method_id, input_json)?;
    let object = as_object(response);
    let result_type = object
        .get("type")
        .and_then(JsonValue::as_str)
        .unwrap_or("failed");
    if result_type == "success" {
        return parse_credential_response(JsonValue::Object(object));
    }
    if result_type != "oauth_pending" {
        return Err(format!(
            "plugin auth method `{}` returned `{result_type}`",
            method.method_id
        ));
    }

    let callback_id = object
        .get("callback_id")
        .and_then(JsonValue::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            format!(
                "plugin auth method `{}` did not return a callback id",
                method.method_id
            )
        })?;
    let mode = object
        .get("method")
        .and_then(JsonValue::as_str)
        .unwrap_or("code")
        .to_ascii_lowercase();
    if let Some(url) = object.get("url").and_then(JsonValue::as_str) {
        if !url.trim().is_empty() {
            eprintln!("cortex auth login: open this URL to authorize: {url}");
        }
    }
    if let Some(instructions) = object.get("instructions").and_then(JsonValue::as_str) {
        if !instructions.trim().is_empty() {
            eprintln!("cortex auth login: {instructions}");
        }
    }

    let code = if mode == "auto" {
        None
    } else {
        args.oauth_code
            .as_deref()
            .and_then(|value| non_empty(Some(value)))
            .or_else(|| inputs.get("code").cloned())
            .or_else(read_interactive_oauth_code)
    };
    if mode != "auto" && code.is_none() {
        return Err(
            "OAuth code is required (use --oauth-code CODE or --auth-input code=CODE)".to_owned(),
        );
    }

    let callback =
        plugin_state.complete_auth_callback(&method.provider_id, &callback_id, code.as_deref())?;
    parse_credential_response(callback)
}

fn authorize_builtin_oauth_method(
    _method: &AuthMethodDescriptor,
    inputs: BTreeMap<String, String>,
    _args: &AuthLoginArgs,
) -> Result<JsonValue, String> {
    let client_id = required_auth_input(&inputs, "client_id")?;
    let authorize_endpoint = required_auth_input(&inputs, "authorize_endpoint")?;
    let token_endpoint = required_auth_input(&inputs, "token_endpoint")?;
    let client_secret = non_empty(inputs.get("client_secret").map(String::as_str));
    let callback_port = parse_optional_callback_port(inputs.get("callback_port"))?;
    let scopes = parse_scope_inputs(&inputs);
    let timeout = parse_optional_timeout_seconds(inputs.get("timeout_seconds"))?;

    let oauth_config = mcp::oauth::OAuthConfig {
        authorize_endpoint,
        token_endpoint,
        client_id: client_id.clone(),
        scopes,
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| format!("failed to start OAuth runtime: {err}"))?;
    let request = runtime
        .block_on(mcp::oauth::begin(oauth_config, callback_port))
        .map_err(|err| err.to_string())?;
    eprintln!(
        "cortex auth login: open this URL to authorize: {}",
        request.authorize_url
    );
    eprintln!(
        "cortex auth login: waiting for OAuth callback on {}",
        request.redirect_uri
    );
    let token = runtime
        .block_on(request.complete(timeout))
        .map_err(|err| err.to_string())?;

    let mut credential = serde_json::Map::new();
    credential.insert("type".to_owned(), JsonValue::String("oauth".to_owned()));
    credential.insert(
        "access_token".to_owned(),
        JsonValue::String(token.access_token),
    );
    credential.insert("token_type".to_owned(), JsonValue::String(token.token_type));
    if let Some(refresh_token) = token.refresh_token {
        credential.insert("refresh_token".to_owned(), JsonValue::String(refresh_token));
    }
    if let Some(expires_at) = token.expires_at {
        credential.insert(
            "expires_at".to_owned(),
            JsonValue::Number(expires_at.into()),
        );
    }
    credential.insert(
        "token_endpoint".to_owned(),
        JsonValue::String(token.token_endpoint),
    );
    credential.insert("client_id".to_owned(), JsonValue::String(token.client_id));
    if let Some(secret) = client_secret {
        credential.insert("client_secret".to_owned(), JsonValue::String(secret));
    }
    Ok(JsonValue::Object(credential))
}

fn read_interactive_oauth_code() -> Option<String> {
    let stdin = io::stdin();
    if !stdin.is_terminal() {
        return None;
    }
    let mut stdout = io::stdout().lock();
    if write!(stdout, "OAuth code: ")
        .and_then(|()| stdout.flush())
        .is_err()
    {
        return None;
    }
    let mut line = String::new();
    if stdin.read_line(&mut line).is_err() {
        return None;
    }
    non_empty(Some(&line))
}

fn parse_credential_response(value: JsonValue) -> Result<JsonValue, String> {
    let object = as_object(value);
    let result_type = object
        .get("type")
        .and_then(JsonValue::as_str)
        .unwrap_or("failed");
    if result_type == "failed" {
        return Err("authorization failed".to_owned());
    }
    if let Some(credential) = object.get("credential") {
        return Ok(credential.clone());
    }
    if let Some(key) = object.get("key").and_then(JsonValue::as_str) {
        return Ok(JsonValue::String(key.to_owned()));
    }
    if object.contains_key("access") || object.contains_key("refresh") {
        return Ok(JsonValue::Object(object));
    }
    Ok(JsonValue::Object(object))
}

fn as_object(value: JsonValue) -> serde_json::Map<String, JsonValue> {
    match value {
        JsonValue::Object(map) => map,
        JsonValue::Null => serde_json::Map::new(),
        other => {
            let mut map = serde_json::Map::new();
            map.insert("payload".to_owned(), other);
            map
        }
    }
}

fn store_auth_payload(
    path: &std::path::Path,
    provider: &str,
    credential: &JsonValue,
    preserve_structured: bool,
) -> io::Result<()> {
    let secret = match credential {
        JsonValue::String(value) => value.clone(),
        JsonValue::Object(map) if !preserve_structured => map
            .get("key")
            .and_then(JsonValue::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| serde_json::to_string(credential).unwrap_or_default()),
        _ => serde_json::to_string(credential).unwrap_or_default(),
    };
    store_credential(path, provider, &secret)
}

fn provider_default_env(provider: &str) -> Option<String> {
    let inputs = ConfigInputs::from_process(BTreeMap::new()).ok()?;
    let effective = settings::load(inputs).ok()?;
    let provider_config = providers::ProviderConfigSet::from_table(effective.table()).ok()?;
    provider_config
        .provider(provider)
        .and_then(|configured| configured.auth.api_key_env.clone())
        .or_else(|| {
            (provider == providers::OPENROUTER_ID).then(|| providers::OPENROUTER_ENV.to_owned())
        })
}

fn credential_source(args: &AuthLoginArgs, provider: &str) -> String {
    if args
        .api_key
        .as_deref()
        .and_then(|value| non_empty(Some(value)))
        .is_some()
    {
        return "flag".to_owned();
    }
    if let Some(env_name) = args
        .api_key_env
        .as_deref()
        .and_then(|value| non_empty(Some(value)))
        && std::env::var(&env_name)
            .ok()
            .and_then(|value| non_empty(Some(&value)))
            .is_some()
    {
        return format!("env:{env_name}");
    }
    if let Some(default_env) = provider_default_env(provider)
        && std::env::var(&default_env)
            .ok()
            .and_then(|value| non_empty(Some(&value)))
            .is_some()
    {
        return format!("env:{default_env}");
    }
    "stdin".to_owned()
}

fn resolve_login_secret(args: &AuthLoginArgs, provider: &str) -> Option<String> {
    if let Some(flag) = args
        .api_key
        .as_deref()
        .and_then(|value| non_empty(Some(value)))
    {
        return Some(flag);
    }
    if let Some(env_name) = args
        .api_key_env
        .as_deref()
        .and_then(|value| non_empty(Some(value)))
        && let Ok(value) = std::env::var(env_name)
        && let Some(secret) = non_empty(Some(&value))
    {
        return Some(secret);
    }
    if let Some(default_env) = provider_default_env(provider)
        && let Ok(value) = std::env::var(&default_env)
        && let Some(secret) = non_empty(Some(&value))
    {
        return Some(secret);
    }
    read_stdin_secret().ok().flatten()
}

fn read_stdin_secret() -> io::Result<Option<String>> {
    let stdin = io::stdin();
    if stdin.is_terminal() {
        return Ok(None);
    }
    let mut buffer = String::new();
    stdin.lock().read_to_string(&mut buffer)?;
    Ok(non_empty(Some(&buffer)))
}

fn non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

#[derive(Serialize)]
struct StoredProviderDocument {
    id: String,
    source: &'static str,
}

#[derive(Serialize)]
struct AuthMethodListDocument {
    provider: String,
    provider_name: String,
    method: String,
    #[serde(rename = "type")]
    method_type: AuthMethodType,
    label: String,
    source: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    plugin: Option<String>,
    logged_in: bool,
}

#[derive(Serialize)]
struct AuthListDocument {
    providers: Vec<StoredProviderDocument>,
    accounts: Vec<crate::account_store::AccountDocument>,
    methods: Vec<AuthMethodListDocument>,
}

#[derive(Serialize)]
struct AuthMutationDocument<'a> {
    provider: &'a str,
    stored: bool,
    removed: bool,
    source: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    method: Option<&'a str>,
    message: &'a str,
}

fn emit_mutation(format: OutputFormat, doc: &AuthMutationDocument<'_>, text: &str) -> Exit {
    match format {
        OutputFormat::Text => emit_text(text),
        OutputFormat::Json | OutputFormat::Jsonl => emit_json(doc),
    }
}

fn emit_json<T: Serialize>(value: &T) -> Exit {
    match serde_json::to_string(value) {
        Ok(mut body) => {
            body.push('\n');
            emit_text(&body)
        }
        Err(err) => {
            eprintln!("cortex auth: failed to serialize output: {err}");
            Exit::Failure
        }
    }
}

fn emit_text(body: &str) -> Exit {
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    if handle
        .write_all(body.as_bytes())
        .and_then(|()| handle.flush())
        .is_err()
    {
        return Exit::Failure;
    }
    Exit::Success
}
