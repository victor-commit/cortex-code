//! `cortex agent`: discover the built-in agent personas available to headless turns and the TUI.

use engine::Agent;
use errors::Exit;
use serde::Serialize;

use crate::cli::{AgentArgs, OutputFormat};

/// Dispatch the `cortex agent` discovery command.
pub fn run(args: AgentArgs) -> Exit {
    match args.format.format {
        OutputFormat::Text => emit_text(),
        OutputFormat::Json | OutputFormat::Jsonl => emit_json(),
    }
}

fn emit_text() -> Exit {
    for agent in Agent::ALL {
        let default = if agent == Agent::DEFAULT {
            " (default)"
        } else {
            ""
        };
        println!("{:<8}{}  {}", agent.name(), default, agent.description());
    }
    Exit::Success
}

fn emit_json() -> Exit {
    let doc = AgentList {
        default: Agent::DEFAULT.name(),
        agents: Agent::ALL
            .iter()
            .map(|agent| AgentInfo {
                name: agent.name(),
                description: agent.description(),
            })
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
struct AgentList<'a> {
    default: &'a str,
    agents: Vec<AgentInfo<'a>>,
}

#[derive(Serialize)]
struct AgentInfo<'a> {
    name: &'a str,
    description: &'a str,
}
