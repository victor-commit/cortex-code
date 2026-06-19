//! Agent personas: named behavioral modes that shape a turn via a system-prompt fragment.
//!
//! Architecture §4 has the runner "resolve agent" as the first step of the system-context epoch,
//! and §8 lists per-persona "mode fragments" as part of prompt assembly. M1 ships the headless
//! subset: a small set of built-in personas selectable with `cortex run --agent <name>`. Each
//! persona contributes an optional system-prompt fragment appended after the dynamic environment
//! block (see [`crate::SystemContext`]), so the same model behaves differently depending on the
//! selected agent. The richer custom/markdown agents and subagent spawning described in §3.4 land
//! in M4; this module is the M1 seam they extend.

/// A built-in agent persona selectable with `cortex run --agent <name>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agent {
    /// The default coding agent: implements and answers directly (baseline behavior).
    Build,
    /// The planning persona: investigates and produces a plan/outline, never making or claiming
    /// edits.
    Plan,
    /// A general-purpose assistant persona.
    General,
}

impl Agent {
    /// The agent used when `--agent` is not given.
    pub const DEFAULT: Agent = Agent::Build;

    /// Every built-in agent, in the stable order used for availability hints.
    pub const ALL: [Agent; 3] = [Agent::Build, Agent::Plan, Agent::General];

    /// The canonical, lowercase name of this agent (the value accepted by `--agent` and surfaced
    /// in machine output).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Agent::Build => "build",
            Agent::Plan => "plan",
            Agent::General => "general",
        }
    }

    /// Resolve an agent by case-insensitive name, trimming surrounding whitespace.
    ///
    /// # Errors
    /// Returns [`UnknownAgent`] (naming the offending input and listing the available agents) when
    /// no built-in persona matches.
    pub fn from_name(name: &str) -> Result<Self, UnknownAgent> {
        let normalized = name.trim().to_ascii_lowercase();
        Self::ALL
            .into_iter()
            .find(|agent| agent.name() == normalized)
            .ok_or_else(|| UnknownAgent::new(name))
    }

    /// The persona system-prompt fragment appended to the system context, or `None` for the
    /// baseline coding agent (which adds nothing beyond the family base prompt, so a default run is
    /// byte-identical to one with no persona).
    #[must_use]
    pub const fn persona_prompt(self) -> Option<&'static str> {
        match self {
            Agent::Build => None,
            Agent::Plan => Some(PLAN_PERSONA),
            Agent::General => Some(GENERAL_PERSONA),
        }
    }

    /// A comma-separated list of every available agent name, for help text and diagnostics.
    #[must_use]
    pub fn available_names() -> String {
        Self::ALL
            .iter()
            .map(|agent| agent.name())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

impl Default for Agent {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// An error returned when a requested `--agent` name matches no built-in persona.
///
/// Carries both the offending input and the list of available agents so the CLI can render an
/// actionable diagnostic that names the unknown agent and hints what is selectable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownAgent {
    requested: String,
    available: String,
}

impl UnknownAgent {
    fn new(requested: &str) -> Self {
        Self {
            requested: requested.to_owned(),
            available: Agent::available_names(),
        }
    }

    /// The unknown name the caller requested.
    #[must_use]
    pub fn requested(&self) -> &str {
        &self.requested
    }

    /// The comma-separated list of available agent names.
    #[must_use]
    pub fn available(&self) -> &str {
        &self.available
    }
}

impl std::fmt::Display for UnknownAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "unknown agent '{}': available agents are {}",
            self.requested, self.available
        )
    }
}

impl std::error::Error for UnknownAgent {}

/// The plan persona fragment: investigate and design, never implement or claim to have implemented.
const PLAN_PERSONA: &str = "You are operating in the plan persona. Your role is to investigate and \
design, not to implement. You MUST NOT make any changes to the system, and you MUST NOT claim to \
have made, applied, written, or committed any edits, files, or commands. Never state that you have \
created or modified anything. Instead, produce a clear, structured plan: present your response as a \
numbered outline of the steps required to accomplish the request, noting the files or components \
involved and how the result would be verified. Describe what should be done; do not do it.";

/// The general persona fragment: a broad, direct general-purpose assistant.
const GENERAL_PERSONA: &str = "You are operating in the general persona: a helpful, knowledgeable \
general-purpose assistant. Answer the request directly and concisely, drawing on broad knowledge \
across software engineering and general topics, without assuming a specialized planning or \
implementation workflow.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_agent_is_build() {
        assert_eq!(Agent::default(), Agent::Build);
        assert_eq!(Agent::DEFAULT, Agent::Build);
        assert_eq!(Agent::DEFAULT.name(), "build");
    }

    #[test]
    fn from_name_resolves_every_built_in_agent() {
        assert_eq!(Agent::from_name("build"), Ok(Agent::Build));
        assert_eq!(Agent::from_name("plan"), Ok(Agent::Plan));
        assert_eq!(Agent::from_name("general"), Ok(Agent::General));
    }

    #[test]
    fn from_name_is_case_insensitive_and_trims() {
        assert_eq!(Agent::from_name("  PLAN "), Ok(Agent::Plan));
        assert_eq!(Agent::from_name("Build"), Ok(Agent::Build));
        assert_eq!(Agent::from_name("GENERAL"), Ok(Agent::General));
    }

    #[test]
    fn from_name_rejects_unknown_with_an_actionable_error() {
        let err = Agent::from_name("no-such-agent").expect_err("unknown name must be rejected");
        assert_eq!(err.requested(), "no-such-agent");
        let message = err.to_string();
        assert!(
            message.contains("no-such-agent"),
            "the error must name the unknown agent, got: {message}"
        );
        assert!(
            message.contains("plan") && message.contains("build") && message.contains("general"),
            "the error must hint the available agents, got: {message}"
        );
    }

    #[test]
    fn available_names_lists_every_agent() {
        let names = Agent::available_names();
        for agent in Agent::ALL {
            assert!(
                names.contains(agent.name()),
                "available_names must include {}, got: {names}",
                agent.name()
            );
        }
    }

    #[test]
    fn build_persona_is_neutral_others_have_fragments() {
        assert!(
            Agent::Build.persona_prompt().is_none(),
            "the default build agent adds no extra fragment"
        );
        let plan = Agent::Plan.persona_prompt().expect("plan has a fragment");
        assert!(
            plan.to_lowercase().contains("plan") && plan.contains("MUST NOT"),
            "the plan persona must steer toward planning and away from edits"
        );
        assert!(
            !Agent::General
                .persona_prompt()
                .expect("general has a fragment")
                .trim()
                .is_empty(),
            "the general persona has a non-empty fragment"
        );
    }
}
