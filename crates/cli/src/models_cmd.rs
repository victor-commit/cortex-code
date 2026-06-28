//! `cortex models`: list built-in and plugin-registered models.
//!
//! The plain listing is fully offline: it reads the on-disk cache written by a prior `--refresh`
//! when present, otherwise the binary's embedded models.dev snapshot — never the network. With
//! `--refresh` it performs a real models.dev fetch behind the `CORTEX_NO_NET` outbound guard,
//! curates the result to the recognizable model set, caches it, and lists it; a fetch failure
//! degrades cleanly (clear diagnostic, non-zero exit) without corrupting the cached catalogue.

use errors::Exit;
use serde::Serialize;
use settings::ConfigInputs;
use std::collections::BTreeMap;
use std::io::{self, Write};

use crate::cli::{ModelsArgs, OutputFormat};
use crate::plugin_bridge::{ModelCostDoc, ModelDoc, PluginState, ProviderDoc};

pub fn run(args: ModelsArgs, pure_plugins: bool) -> Exit {
    if args.refresh && !crate::feature_gate::network_enabled() {
        return crate::feature_gate::network_unavailable("models --refresh");
    }

    let listing = match collect_providers(pure_plugins, args.refresh, "models") {
        Ok(listing) => listing,
        Err(exit) => return exit,
    };
    emit(
        args.format.format,
        &listing.providers,
        &listing.default_model,
        &listing.provider_report,
    )
}

/// The provider catalogue projected for `cortex models` / `cortex provider list`.
///
/// Carries the per-provider docs (after provider-config enable/disable and policy-deny filtering),
/// the resolved default model id, and the redacted provider-config report.
pub(crate) struct ProviderListing {
    pub(crate) providers: Vec<ProviderDoc>,
    pub(crate) default_model: String,
    pub(crate) provider_report: providers::ProviderConfigReport,
}

/// Build the provider listing shared by `cortex models` and `cortex provider list`.
///
/// `refresh` triggers a real models.dev fetch (the caller is responsible for the network feature
/// gate); when false the catalogue is read entirely offline. `command` is the subcommand label used
/// in diagnostics so errors read as `cortex <command>: ...`.
pub(crate) fn collect_providers(
    pure_plugins: bool,
    refresh: bool,
    command: &str,
) -> Result<ProviderListing, Exit> {
    let state = match PluginState::from_process_config(pure_plugins) {
        Ok(state) => state,
        Err(err) => {
            eprintln!("cortex {command}: {err}");
            return Err(Exit::Config);
        }
    };

    let config_inputs = match ConfigInputs::from_process(BTreeMap::new()) {
        Ok(inputs) => inputs,
        Err(err) => {
            eprintln!("cortex {command}: cannot determine configuration context: {err}");
            return Err(Exit::Failure);
        }
    };
    let effective = match settings::load(config_inputs.clone()) {
        Ok(effective) => effective,
        Err(err) => {
            eprintln!("cortex {command}: {err}");
            return Err(Exit::Config);
        }
    };
    let provider_config = match providers::ProviderConfigSet::from_table(effective.table()) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("cortex {command}: {err}");
            return Err(Exit::Config);
        }
    };
    let provider_report = provider_config.redacted_report();

    let (builtin_catalogue, builtin_source) = if refresh {
        refresh_builtin_catalogue()?
    } else {
        offline_builtin_catalogue()
    };
    let merged_catalogue = provider_config.merged_catalogue(&builtin_catalogue);
    let default_model =
        match provider_config.resolve_model(&merged_catalogue, Some(effective.model())) {
            Ok(resolved) => resolved.model_id,
            Err(err) => {
                eprintln!("cortex {command}: {err}");
                return Err(Exit::Config);
            }
        };

    let mut providers = docs_from_catalogue(
        &merged_catalogue,
        &builtin_catalogue,
        builtin_source,
        &default_model,
        &provider_report.providers,
    );
    let mut plugin_providers = state.providers();
    for provider in &mut plugin_providers {
        provider.config = provider_report.providers.get(&provider.id).cloned();
    }
    providers.extend(plugin_providers);
    let providers = providers
        .into_iter()
        .filter(|provider| provider_config.provider_enabled(&provider.id))
        .collect::<Vec<_>>();
    let providers = match filter_denied_providers(&config_inputs, &providers) {
        Ok(providers) => providers,
        Err(err) => {
            eprintln!("cortex {command}: {err}");
            return Err(if err.is_config() {
                Exit::Config
            } else {
                Exit::Failure
            });
        }
    };

    Ok(ProviderListing {
        providers,
        default_model,
        provider_report,
    })
}

/// The offline builtin catalogue: the cache written by a prior `--refresh` when present and
/// valid, otherwise the embedded snapshot. Either source is read from disk with no network access.
fn offline_builtin_catalogue() -> (providers::Catalogue, &'static str) {
    match providers::load_cached_catalogue() {
        Some(cached) => (cached, "cached"),
        None => (providers::Catalogue::embedded().clone(), "builtin"),
    }
}

/// Refresh the builtin providers from models.dev, fail-closed under `CORTEX_NO_NET`.
///
/// The outbound guard is consulted FIRST, so under the kill-switch this returns the net-disabled
/// exit code with no socket opened. On a successful fetch the curated snapshot is cached (a write
/// failure is non-fatal) and listed; on a fetch failure it returns the mapped failure exit with a
/// clear diagnostic on stderr, leaving any existing cache (and the embedded snapshot) intact.
fn refresh_builtin_catalogue() -> Result<(providers::Catalogue, &'static str), Exit> {
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
            Ok((catalogue, "models.dev"))
        }
        Err(diag) => {
            eprintln!("cortex models --refresh: {diag}");
            Err(diag.exit())
        }
    }
}

/// Project a catalogue into the `cortex models` document shape, tagging each entry's `source`.
fn docs_from_catalogue(
    catalogue: &providers::Catalogue,
    builtin_catalogue: &providers::Catalogue,
    builtin_source: &'static str,
    default_model: &str,
    provider_reports: &BTreeMap<String, providers::ProviderReport>,
) -> Vec<ProviderDoc> {
    catalogue
        .providers()
        .map(|provider| ProviderDoc {
            id: provider.id.clone(),
            name: provider.name.clone(),
            source: if builtin_catalogue.provider(&provider.id).is_some() {
                builtin_source
            } else {
                "config"
            },
            config: provider_reports.get(&provider.id).cloned(),
            models: provider
                .models
                .values()
                .map(|model| ModelDoc {
                    id: model.id.clone(),
                    name: model.name.clone(),
                    provider: provider.id.clone(),
                    default: model.id == default_model,
                    attachment: model.attachment,
                    tool_call: model.tool_call,
                    reasoning: model.reasoning,
                    cost: ModelCostDoc {
                        input: model.cost.input,
                        output: model.cost.output,
                    },
                    context_length: model.limit.context,
                    max_output_tokens: model.limit.output,
                    source: if builtin_catalogue.model(&provider.id, &model.id).is_some() {
                        builtin_source
                    } else {
                        "config"
                    },
                })
                .collect(),
        })
        .collect()
}

fn filter_denied_providers(
    inputs: &settings::ConfigInputs,
    providers: &[ProviderDoc],
) -> Result<Vec<ProviderDoc>, policy::ProviderPolicyError> {
    providers.iter().try_fold(Vec::new(), |mut acc, provider| {
        if policy::provider_use_action(inputs, &provider.id)? != permissions::Action::Deny {
            acc.push(provider.clone());
        }
        Ok(acc)
    })
}

#[derive(Serialize)]
struct ModelsDocument<'a> {
    default_model: &'a str,
    providers: &'a [ProviderDoc],
    provider_config: &'a providers::ProviderConfigReport,
}

fn emit(
    format: OutputFormat,
    providers: &[ProviderDoc],
    default_model: &str,
    provider_config: &providers::ProviderConfigReport,
) -> Exit {
    match format {
        OutputFormat::Text => {
            let stdout = io::stdout();
            let mut handle = stdout.lock();
            for provider in providers {
                if let Err(err) = writeln!(handle, "{} ({})", provider.name, provider.id) {
                    return map_stdout_error(err);
                }
                for model in &provider.models {
                    let marker = if model.default { " [default]" } else { "" };
                    if let Err(err) = writeln!(
                        handle,
                        "  {}\t{}{}\tctx={}\tin=${:.2}/Mtok\tout=${:.2}/Mtok",
                        model.id,
                        model.name,
                        marker,
                        model.context_length,
                        model.cost.input,
                        model.cost.output
                    ) {
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
            let document = ModelsDocument {
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
                    eprintln!("cortex models: failed to serialize models: {err}");
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

    eprintln!("cortex models: failed to write output: {err}");
    Exit::Failure
}
