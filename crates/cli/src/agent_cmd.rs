//! `cortex agent`: discover built-in and custom markdown agents available to headless turns.

use engine::Agent;
use errors::Exit;
use serde::Serialize;
use settings::ConfigInputs;

use crate::cli::{AgentArgs, OutputFormat};
use crate::custom_agents;

/// Dispatch the `cortex agent` discovery command.
pub fn run(args: AgentArgs) -> Exit {
    let discovery = match ConfigInputs::from_process(std::collections::BTreeMap::new()) {
        Ok(inputs) => {
            let discovery = custom_agents::discover(&inputs);
            for warning in &discovery.warnings {
                eprintln!("cortex agent: {warning}");
            }
            discovery
        }
        Err(err) => {
            eprintln!("cortex agent: cannot determine the working directory: {err}");
            return Exit::Failure;
        }
    };

    match args.format.format {
        OutputFormat::Text => emit_text(&discovery),
        OutputFormat::Json | OutputFormat::Jsonl => emit_json(&discovery),
    }
}

fn emit_text(discovery: &custom_agents::Discovery) -> Exit {
    for agent in Agent::ALL {
        let default = if agent == Agent::DEFAULT {
            " (default)"
        } else {
            ""
        };
        println!("{:<8}{}  {}", agent.name(), default, agent.description());
    }
    for agent in discovery.agents.values() {
        let model = agent
            .model
            .as_deref()
            .map(|model| format!(" [model: {model}]"))
            .unwrap_or_default();
        println!(
            "{:<8}  {}{} ({})",
            agent.name,
            agent.description,
            model,
            agent.source.display()
        );
    }
    Exit::Success
}

fn emit_json(discovery: &custom_agents::Discovery) -> Exit {
    let doc = AgentList {
        default: Agent::DEFAULT.name(),
        agents: Agent::ALL
            .iter()
            .map(|agent| AgentInfo {
                name: agent.name().to_owned(),
                description: agent.description().to_owned(),
                mode: "built-in".to_owned(),
                model: None,
                custom: false,
                source: None,
            })
            .chain(discovery.agents.values().map(|agent| AgentInfo {
                name: agent.name.clone(),
                description: agent.description.clone(),
                mode: agent.mode.clone(),
                model: agent.model.clone(),
                custom: true,
                source: Some(agent.source.display().to_string()),
            }))
            .collect(),
    };
    match serde_json::to_string(&doc) {
        Ok(line) => {
            println!("{line}");
            Exit::Success
        }
        Err(err) => {
            eprintln!("cortex agent: failed to serialize output: {err}");
            Exit::Failure
        }
    }
}

#[derive(Serialize)]
struct AgentList {
    default: &'static str,
    agents: Vec<AgentInfo>,
}

#[derive(Serialize)]
struct AgentInfo {
    name: String,
    description: String,
    mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    custom: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<String>,
}
