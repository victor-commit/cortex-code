//! `cortex auth`: login/list/logout for the local provider credential store.

use std::io::{self, IsTerminal, Read, Write};

use errors::Exit;
use providers::{
    AuthPathInputs, list_stored_providers, remove_stored_credential, store_credential,
};
use redaction::PLACEHOLDER;
use serde::Serialize;

use crate::cli::{
    AuthArgs, AuthCommand, AuthListArgs, AuthLoginArgs, AuthLogoutArgs, OutputFormat,
};

pub fn run(args: AuthArgs, _pure_plugins: bool) -> Exit {
    match args.command {
        AuthCommand::Login(args) => login(args),
        AuthCommand::List(args) => list(args),
        AuthCommand::Logout(args) => logout(args),
    }
}

fn login(args: AuthLoginArgs) -> Exit {
    let Some(provider) = normalized_provider(&args.provider) else {
        eprintln!("cortex auth login: provider must be non-empty");
        return Exit::Usage;
    };
    let Some(secret) = resolve_login_secret(&args, &provider) else {
        eprintln!(
            "cortex auth login: no API key provided (use --api-key, --api-key-env, {default_env}, or pipe on stdin)",
            default_env = provider_default_env(&provider).unwrap_or("<env>")
        );
        return Exit::MissingInput;
    };
    let Some(path) = auth_store_path() else {
        eprintln!(
            "cortex auth login: cannot determine auth-store path; set HOME, XDG_DATA_HOME, CORTEX_DATA_DIR, or CORTEX_AUTH_FILE"
        );
        return Exit::Failure;
    };

    let source = credential_source(&args, &provider);
    tracing::debug!(
        target: "cortex::auth",
        provider = %provider,
        source = %source,
        api_key = PLACEHOLDER,
        "received credential for auth login"
    );
    if let Err(error) = store_credential(&path, &provider, &secret) {
        eprintln!(
            "cortex auth login: failed to persist credential for provider `{provider}`: {error}"
        );
        return Exit::Failure;
    }

    let document = AuthMutationDocument {
        provider: &provider,
        stored: true,
        removed: false,
        source: &source,
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

fn list(args: AuthListArgs) -> Exit {
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

    match args.format.format {
        OutputFormat::Text => {
            let mut out = String::new();
            for provider in &providers {
                out.push_str(&provider.id);
                out.push('\n');
            }
            emit_text(&out)
        }
        OutputFormat::Json | OutputFormat::Jsonl => emit_json(&AuthListDocument { providers }),
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

fn provider_default_env(provider: &str) -> Option<&'static str> {
    (provider == providers::OPENROUTER_ID).then_some(providers::OPENROUTER_ENV)
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
    {
        if std::env::var(&env_name)
            .ok()
            .and_then(|value| non_empty(Some(&value)))
            .is_some()
        {
            return format!("env:{env_name}");
        }
    }
    if let Some(default_env) = provider_default_env(provider) {
        if std::env::var(default_env)
            .ok()
            .and_then(|value| non_empty(Some(&value)))
            .is_some()
        {
            return format!("env:{default_env}");
        }
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
    {
        if let Ok(value) = std::env::var(env_name) {
            if let Some(secret) = non_empty(Some(&value)) {
                return Some(secret);
            }
        }
    }
    if let Some(default_env) = provider_default_env(provider) {
        if let Ok(value) = std::env::var(default_env) {
            if let Some(secret) = non_empty(Some(&value)) {
                return Some(secret);
            }
        }
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
struct AuthListDocument {
    providers: Vec<StoredProviderDocument>,
}

#[derive(Serialize)]
struct AuthMutationDocument<'a> {
    provider: &'a str,
    stored: bool,
    removed: bool,
    source: &'a str,
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
