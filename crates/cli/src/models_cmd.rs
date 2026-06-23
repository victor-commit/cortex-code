//! `cortex models`: list built-in and plugin-registered models.
//!
//! The plain listing is fully offline: it reads the on-disk cache written by a prior `--refresh`
//! when present, otherwise the binary's embedded models.dev snapshot — never the network. With
//! `--refresh` it performs a real models.dev fetch behind the `CORTEX_NO_NET` outbound guard,
//! curates the result to the recognizable model set, caches it, and lists it; a fetch failure
//! degrades cleanly (clear diagnostic, non-zero exit) without corrupting the cached catalogue.

use errors::Exit;
use serde::Serialize;

use crate::cli::{ModelsArgs, OutputFormat};
use crate::plugin_bridge::{ModelCostDoc, ModelDoc, PluginState, ProviderDoc};

pub fn run(args: ModelsArgs, pure_plugins: bool) -> Exit {
    let state = match PluginState::from_process_config(pure_plugins) {
        Ok(state) => state,
        Err(err) => {
            eprintln!("cortex models: {err}");
            return Exit::Config;
        }
    };

    let builtin = if args.refresh {
        match refresh_builtin_providers() {
            Ok(providers) => providers,
            Err(exit) => return exit,
        }
    } else {
        offline_builtin_providers()
    };

    let mut providers = builtin;
    providers.extend(state.providers());
    emit(args.format.format, &providers)
}

/// The offline builtin providers: the cache written by a prior `--refresh` when present and valid,
/// otherwise the embedded snapshot. Either source is read from disk with no network access.
fn offline_builtin_providers() -> Vec<ProviderDoc> {
    match providers::load_cached_catalogue() {
        Some(cached) => docs_from_catalogue(&cached, "cached"),
        None => docs_from_catalogue(providers::Catalogue::embedded(), "builtin"),
    }
}

/// Refresh the builtin providers from models.dev, fail-closed under `CORTEX_NO_NET`.
///
/// The outbound guard is consulted FIRST, so under the kill-switch this returns the net-disabled
/// exit code with no socket opened. On a successful fetch the curated snapshot is cached (a write
/// failure is non-fatal) and listed; on a fetch failure it returns the mapped failure exit with a
/// clear diagnostic on stderr, leaving any existing cache (and the embedded snapshot) intact.
fn refresh_builtin_providers() -> Result<Vec<ProviderDoc>, Exit> {
    if let Err(diag) = redaction::guard_outbound("models.dev catalog refresh") {
        eprintln!("cortex models --refresh: {diag}");
        return Err(diag.exit());
    }

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("cortex models --refresh: failed to start the async runtime: {err}");
            return Err(Exit::Failure);
        }
    };

    let url = providers::models_dev_url();
    match runtime.block_on(providers::refresh_openrouter_catalogue(&url)) {
        Ok(catalogue) => {
            if let Err(err) = providers::write_models_cache(&catalogue) {
                // A cache write failure must not fail the refresh: the listing is already in hand.
                eprintln!(
                    "cortex models --refresh: warning: could not cache the refreshed catalog: {err}"
                );
            }
            Ok(docs_from_catalogue(&catalogue, "models.dev"))
        }
        Err(diag) => {
            eprintln!("cortex models --refresh: {diag}");
            Err(diag.exit())
        }
    }
}

/// Project a catalogue into the `cortex models` document shape, tagging each entry's `source`.
fn docs_from_catalogue(catalogue: &providers::Catalogue, source: &'static str) -> Vec<ProviderDoc> {
    catalogue
        .providers()
        .map(|provider| ProviderDoc {
            id: provider.id.clone(),
            name: provider.name.clone(),
            source,
            models: provider
                .models
                .values()
                .map(|model| ModelDoc {
                    id: model.id.clone(),
                    name: model.name.clone(),
                    provider: provider.id.clone(),
                    default: provider.id == providers::DEFAULT_PROVIDER
                        && model.id == providers::DEFAULT_MODEL,
                    attachment: model.attachment,
                    tool_call: model.tool_call,
                    reasoning: model.reasoning,
                    cost: ModelCostDoc {
                        input: model.cost.input,
                        output: model.cost.output,
                    },
                    context_length: model.limit.context,
                    max_output_tokens: model.limit.output,
                    source,
                })
                .collect(),
        })
        .collect()
}

#[derive(Serialize)]
struct ModelsDocument<'a> {
    default_model: &'a str,
    providers: &'a [ProviderDoc],
}

fn emit(format: OutputFormat, providers: &[ProviderDoc]) -> Exit {
    match format {
        OutputFormat::Text => {
            for provider in providers {
                println!("{} ({})", provider.name, provider.id);
                for model in &provider.models {
                    let marker = if model.default { " [default]" } else { "" };
                    println!(
                        "  {}\t{}{}\tctx={}\tin=${:.2}/Mtok\tout=${:.2}/Mtok",
                        model.id,
                        model.name,
                        marker,
                        model.context_length,
                        model.cost.input,
                        model.cost.output
                    );
                }
            }
            Exit::Success
        }
        OutputFormat::Json | OutputFormat::Jsonl => {
            let document = ModelsDocument {
                default_model: ::providers::DEFAULT_MODEL,
                providers,
            };
            match serde_json::to_string(&document) {
                Ok(line) => {
                    println!("{line}");
                    Exit::Success
                }
                Err(err) => {
                    eprintln!("cortex models: failed to serialize models: {err}");
                    Exit::Failure
                }
            }
        }
    }
}
