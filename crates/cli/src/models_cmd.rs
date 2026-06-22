//! `cortex models`: list built-in and plugin-registered models.

use errors::Exit;
use serde::Serialize;

use crate::cli::{ModelsArgs, OutputFormat};
use crate::plugin_bridge::{ModelDoc, PluginState, ProviderDoc};

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
                    tool_call: model.tool_call,
                    reasoning: model.reasoning,
                    source: "builtin",
                })
                .collect(),
        })
        .collect()
}

#[derive(Serialize)]
struct ModelsDocument<'a> {
    providers: &'a [ProviderDoc],
}

fn emit(format: OutputFormat, providers: &[ProviderDoc]) -> Exit {
    match format {
        OutputFormat::Text => {
            for provider in providers {
                println!("{} ({})", provider.name, provider.id);
                for model in &provider.models {
                    println!("  {}\t{}\t{}", model.id, model.name, model.source);
                }
            }
            Exit::Success
        }
        OutputFormat::Json | OutputFormat::Jsonl => {
            match serde_json::to_string(&ModelsDocument { providers }) {
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
