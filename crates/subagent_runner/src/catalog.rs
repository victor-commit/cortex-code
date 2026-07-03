//! Build the permission-filtered subagent catalogue attached to one parent turn, and construct its
//! runner.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use permissions::{Action, PermissionEngine};
use tools::TaskTarget;

use crate::custom_agents::{self, ActiveAgent};
use crate::runner::{CliTaskRunner, CliTaskRunnerConfig};

/// The filtered subagent catalogue attached to one parent turn.
pub struct SubagentCatalog {
    /// Model-facing targets, already filtered by `task:<agent>` permission.
    pub targets: Vec<TaskTarget>,
    pub(crate) agents: BTreeMap<String, ActiveAgent>,
}

impl SubagentCatalog {
    /// Build the task runner for this catalogue.
    ///
    /// Returns the concrete runner so the surface can both attach it to a dispatcher (where it
    /// coerces to `Arc<dyn TaskRunner>`) and later
    /// [`drain`](CliTaskRunner::drain_background) any background subagent tasks before the process
    /// exits.
    #[must_use]
    pub fn runner(&self, config: CliTaskRunnerConfig) -> Arc<CliTaskRunner> {
        Arc::new(CliTaskRunner {
            config,
            agents: self.agents.clone(),
            background_tasks: Arc::new(Mutex::new(BTreeMap::new())),
        })
    }
}

/// Build the subagent catalogue for the current parent.
///
/// Built-in `build` is excluded because it is the primary/default agent. Custom agents are offered
/// only when their markdown mode is `subagent` or `all`. The final list is filtered through the
/// parent permission engine: with no engine every subagent is allowed; with an engine only
/// `task:<agent>` actions classified as `allow` are advertised.
#[must_use]
pub fn permitted_catalog(
    discovery: &custom_agents::Discovery,
    permission_engine: Option<&Arc<PermissionEngine>>,
) -> SubagentCatalog {
    let mut targets = Vec::new();
    let mut agents = BTreeMap::new();

    for agent in engine::Agent::SUBAGENTS {
        if task_allowed(permission_engine, agent.name()) {
            targets.push(TaskTarget::new(agent.name(), agent.description()));
            agents.insert(agent.name().to_owned(), ActiveAgent::Builtin(agent));
        }
    }

    for agent in discovery
        .agents
        .values()
        .filter(|agent| agent.availability.spawnable())
    {
        if task_allowed(permission_engine, &agent.name) {
            targets.push(TaskTarget::new(&agent.name, &agent.description));
            agents.insert(agent.name.clone(), ActiveAgent::Custom(agent.clone()));
        }
    }

    SubagentCatalog { targets, agents }
}

fn task_allowed(engine: Option<&Arc<PermissionEngine>>, agent_name: &str) -> bool {
    engine.is_none_or(|engine| engine.classify("task", agent_name) == Action::Allow)
}
