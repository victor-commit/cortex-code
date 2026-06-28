//! Catalog routes that expose the same discoverable entities as the CLI and TUI surfaces.
//!
//! `/agent`, `/model`, `/command`, and `/skill` list the agents, models, commands, and skills a
//! server client can select. Each catalog merges the built-in/static entries with project-local
//! custom definitions and any plugin-registered entries, so a server client sees the same names
//! and metadata as `cortex run`, `cortex models`, and the TUI pickers. None of these routes open a
//! socket; plugin discovery reuses the shared loopback plugin loader.

use std::collections::BTreeMap;

use axum::{Json, extract::State};
use engine::Agent;
use serde::Serialize;

use super::{ApiError, ServerState, config_ext, custom_agents, load_plugin_hooks};

/// One agent catalog entry.
#[derive(Debug, Serialize)]
pub(super) struct AgentEntry {
    name: String,
    description: String,
    /// Availability axis (`primary`/`subagent`/`all`), intentionally different from the CLI
    /// `cortex agent --format json` `mode` (which encodes `built-in` provenance). The per-surface
    /// difference is deliberate and documented in `library/server.md` (VAL-GAP-SERVER-011).
    mode: &'static str,
    source: &'static str,
}

/// One command catalog entry.
#[derive(Debug, Serialize)]
pub(super) struct CommandEntry {
    name: String,
    description: String,
    source: &'static str,
}

/// One skill catalog entry.
#[derive(Debug, Serialize)]
pub(super) struct SkillEntry {
    name: String,
    description: String,
    source: &'static str,
}

/// `GET /agent` — list built-in, custom, and plugin agents.
pub(super) async fn list_agents(
    State(state): State<ServerState>,
) -> Result<Json<Vec<AgentEntry>>, ApiError> {
    let mut entries: Vec<AgentEntry> = Agent::ALL
        .iter()
        .map(|agent| AgentEntry {
            name: agent.name().to_owned(),
            description: agent.description().to_owned(),
            mode: if agent.is_primary() {
                "primary"
            } else {
                "subagent"
            },
            source: "builtin",
        })
        .collect();

    let inputs = state.config.inputs(&state.cwd, BTreeMap::new());
    let discovery = custom_agents::discover(&inputs);
    for agent in discovery.agents.values() {
        entries.push(AgentEntry {
            name: redaction::redact(&agent.name),
            description: redaction::redact(&agent.description),
            mode: availability_mode(agent.availability),
            source: "custom",
        });
    }

    let known: std::collections::HashSet<String> =
        entries.iter().map(|entry| entry.name.clone()).collect();
    for plugin_agent in load_plugin_hooks(&state)?.agent_entries() {
        let name = redaction::redact(&plugin_agent.id);
        if known.contains(&name) {
            continue;
        }
        entries.push(AgentEntry {
            description: redaction::redact(&plugin_agent.description),
            name,
            mode: "primary",
            source: "plugin",
        });
    }

    Ok(Json(entries))
}

/// `GET /model` — list every catalog model, including plugin-registered models.
pub(super) async fn list_models(
    State(state): State<ServerState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let context = config_ext::provider_context(&state)?;
    let models = context.flat_models()?;
    Ok(Json(serde_json::json!({
        "default_model": context.default_model(),
        "models": models,
    })))
}

/// `GET /command` — list config-defined custom commands and plugin commands.
pub(super) async fn list_commands(
    State(state): State<ServerState>,
) -> Result<Json<Vec<CommandEntry>>, ApiError> {
    let inputs = state.config.inputs(&state.cwd, BTreeMap::new());
    let effective = settings::load(inputs).map_err(super::config_api_error)?;
    let mut entries = config_commands(effective.get("command"));

    let known: std::collections::HashSet<String> =
        entries.iter().map(|entry| entry.name.clone()).collect();
    for command in load_plugin_hooks(&state)?.command_entries() {
        let name = redaction::redact(&command.id);
        if known.contains(&name) {
            continue;
        }
        entries.push(CommandEntry {
            description: redaction::redact(&command.description),
            name,
            source: "plugin",
        });
    }
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(Json(entries))
}

/// `GET /skill` — list discovered skills and plugin skills.
pub(super) async fn list_skills(
    State(state): State<ServerState>,
) -> Result<Json<Vec<SkillEntry>>, ApiError> {
    let inputs = state.config.inputs(&state.cwd, BTreeMap::new());
    let discovery = skills::discover(&inputs);
    let mut entries: Vec<SkillEntry> = discovery
        .skills
        .values()
        .map(|skill| SkillEntry {
            name: redaction::redact(&skill.name),
            description: redaction::redact(&skill.description),
            source: "config",
        })
        .collect();

    let known: std::collections::HashSet<String> =
        entries.iter().map(|entry| entry.name.clone()).collect();
    for skill in load_plugin_hooks(&state)?.skill_entries() {
        let name = redaction::redact(&skill.id);
        if known.contains(&name) {
            continue;
        }
        entries.push(SkillEntry {
            description: redaction::redact(&skill.description),
            name,
            source: "plugin",
        });
    }
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(Json(entries))
}

fn availability_mode(availability: custom_agents::AgentAvailability) -> &'static str {
    match availability {
        custom_agents::AgentAvailability::Primary => "primary",
        custom_agents::AgentAvailability::Subagent => "subagent",
        custom_agents::AgentAvailability::All => "all",
    }
}

/// Parse the project config `[command]` table into catalog entries, matching the TUI's custom
/// slash-command discovery so the server and TUI advertise the same commands.
fn config_commands(value: Option<&toml::Value>) -> Vec<CommandEntry> {
    let Some(toml::Value::Table(table)) = value else {
        return Vec::new();
    };
    let mut commands = Vec::new();
    for (raw_name, entry) in table {
        let name = raw_name.trim().trim_start_matches('/');
        if name.is_empty() || name.chars().any(char::is_whitespace) {
            continue;
        }
        let Some((description, enabled)) = command_metadata(entry) else {
            continue;
        };
        if !enabled {
            continue;
        }
        commands.push(CommandEntry {
            name: redaction::redact(&name.to_ascii_lowercase()),
            description: redaction::redact(&description),
            source: "config",
        });
    }
    commands
}

/// Extract `(description, enabled)` from a custom command entry, returning `None` when the entry
/// carries no usable prompt/template.
fn command_metadata(entry: &toml::Value) -> Option<(String, bool)> {
    match entry {
        toml::Value::String(prompt) => {
            (!prompt.trim().is_empty()).then(|| ("Run custom command".to_owned(), true))
        }
        toml::Value::Table(table) => {
            let enabled = table.get("disabled").and_then(toml::Value::as_bool) != Some(true);
            let has_prompt = table
                .get("prompt")
                .and_then(toml::Value::as_str)
                .or_else(|| table.get("template").and_then(toml::Value::as_str))
                .map(str::trim)
                .is_some_and(|prompt| !prompt.is_empty());
            if !has_prompt {
                return None;
            }
            let description = table
                .get("description")
                .and_then(toml::Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .unwrap_or("Run custom command")
                .to_owned();
            Some((description, enabled))
        }
        _ => None,
    }
}
