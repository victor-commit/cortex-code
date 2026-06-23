//! `cortex models`: list built-in and plugin-registered models.

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
    let mut providers = builtin_providers();
    providers.extend(state.providers());
    emit(args.format.format, &providers)
}

fn builtin_providers() -> Vec<ProviderDoc> {
    providers::Catalogue::embedded()
        .providers()
        .map(|provider| ProviderDoc {
            id: provider.id.clone(),
            name: provider.name.clone(),
            source: "builtin",
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
                    source: "builtin",
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
