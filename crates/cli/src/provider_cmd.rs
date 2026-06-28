//! `cortex provider`: provider-centric health/model listing and authentication.
//!
//! This is the provider view over two existing surfaces — the model catalogue (`cortex models`)
//! and the local credential store (`cortex auth`). `provider list` reports each configured provider
//! with its health (enabled state, stored-auth status, model count) and models; `provider auth`
//! runs the exact same credential flow as `cortex auth login`. The listing is fully offline: it
//! reads the embedded/cached catalogue plus the on-disk auth store, never the network.

use std::collections::BTreeSet;
use std::io::{self, Write};

use errors::Exit;
use serde::Serialize;

use crate::cli::{
    AuthArgs, AuthCommand, OutputFormat, ProviderArgs, ProviderCommand, ProviderListArgs,
};
use crate::models_cmd::collect_providers;
use crate::plugin_bridge::ModelDoc;

pub fn run(args: ProviderArgs, pure_plugins: bool) -> Exit {
    match args.command {
        ProviderCommand::List(args) => list(args, pure_plugins),
        ProviderCommand::Auth(args) => crate::auth_cmd::run(
            AuthArgs {
                command: AuthCommand::Login(args),
            },
            pure_plugins,
        ),
    }
}

fn list(args: ProviderListArgs, pure_plugins: bool) -> Exit {
    let listing = match collect_providers(pure_plugins, false, "provider") {
        Ok(listing) => listing,
        Err(exit) => return exit,
    };

    let authenticated = stored_providers();

    let filter = args
        .id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase);

    let mut entries = listing
        .providers
        .into_iter()
        .filter(|provider| {
            filter
                .as_deref()
                .is_none_or(|id| provider.id.to_ascii_lowercase() == id)
        })
        .map(|provider| ProviderEntry {
            authenticated: authenticated.contains(&provider.id),
            enabled: true,
            model_count: provider.models.len(),
            id: provider.id,
            name: provider.name,
            source: provider.source,
            config: provider.config,
            models: provider.models,
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.id.cmp(&right.id));

    emit(
        args.format.format,
        &entries,
        &listing.default_model,
        &listing.provider_report,
    )
}

/// The set of provider ids that currently hold a credential in the local 0600 auth store.
///
/// Mirrors `cortex auth list`: only file-stored credentials count as authenticated, so the status
/// flips deterministically after `cortex provider auth` / `cortex auth login` persists a key.
fn stored_providers() -> BTreeSet<String> {
    match providers::AuthPathInputs::from_process().resolve() {
        Some(path) => providers::list_stored_providers(&path)
            .unwrap_or_default()
            .into_iter()
            .collect(),
        None => BTreeSet::new(),
    }
}

#[derive(Serialize)]
struct ProviderEntry {
    id: String,
    name: String,
    source: &'static str,
    enabled: bool,
    authenticated: bool,
    model_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    config: Option<providers::ProviderReport>,
    models: Vec<ModelDoc>,
}

#[derive(Serialize)]
struct ProviderListDocument<'a> {
    default_model: &'a str,
    providers: &'a [ProviderEntry],
    provider_config: &'a providers::ProviderConfigReport,
}

fn emit(
    format: OutputFormat,
    providers: &[ProviderEntry],
    default_model: &str,
    provider_config: &providers::ProviderConfigReport,
) -> Exit {
    match format {
        OutputFormat::Text => {
            let stdout = io::stdout();
            let mut handle = stdout.lock();
            for provider in providers {
                let auth = if provider.authenticated {
                    "authenticated"
                } else {
                    "no-credential"
                };
                if let Err(err) = writeln!(
                    handle,
                    "{} ({})\t{auth}\tmodels={}",
                    provider.name, provider.id, provider.model_count
                ) {
                    return map_stdout_error(err);
                }
                for model in &provider.models {
                    let marker = if model.default { " [default]" } else { "" };
                    if let Err(err) = writeln!(handle, "  {}{}", model.id, marker) {
                        return map_stdout_error(err);
                    }
                }
            }
            if let Err(err) = handle.flush() {
                return map_stdout_error(err);
            }
            Exit::Success
        }
        OutputFormat::Json | OutputFormat::Jsonl => {
            let document = ProviderListDocument {
                default_model,
                providers,
                provider_config,
            };
            match serde_json::to_string(&document) {
                Ok(line) => {
                    println!("{line}");
                    Exit::Success
                }
                Err(err) => {
                    eprintln!("cortex provider: failed to serialize providers: {err}");
                    Exit::Failure
                }
            }
        }
    }
}

fn map_stdout_error(err: io::Error) -> Exit {
    if err.kind() == io::ErrorKind::BrokenPipe {
        return Exit::Success;
    }
    eprintln!("cortex provider: failed to write output: {err}");
    Exit::Failure
}
