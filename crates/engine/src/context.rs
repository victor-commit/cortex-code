//! The system-context epoch: the base system prompt joined with the dynamic environment block and
//! the selected agent persona.
//!
//! Architecture §4 establishes a "system-context epoch" once per turn: resolve the agent, then the
//! base system prompt for the model's family (see [`ModelFamily`]) followed by the dynamic
//! environment block (see [`EnvironmentBlock`]) and the agent's persona fragment (§8 "mode
//! fragments"). [`SystemContext::render`] produces the single system message string the runner
//! places at the head of the conversation.

use crate::agent::Agent;
use crate::environment::EnvironmentBlock;
use crate::family::ModelFamily;

/// The assembled system context for a turn: a model-family base prompt, the environment block, and
/// the selected agent persona fragment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemContext {
    /// The model family whose base prompt opens the context.
    pub family: ModelFamily,
    /// The dynamic environment snapshot appended after the base prompt.
    pub environment: EnvironmentBlock,
    /// The agent persona whose fragment is appended after the environment block.
    pub agent: Agent,
}

impl SystemContext {
    /// Build a context for an explicit family and environment, using the default agent persona.
    #[must_use]
    pub fn new(family: ModelFamily, environment: EnvironmentBlock) -> Self {
        Self {
            family,
            environment,
            agent: Agent::DEFAULT,
        }
    }

    /// Build a context for `model_id`, inferring its family, using the default agent persona.
    #[must_use]
    pub fn for_model(model_id: &str, environment: EnvironmentBlock) -> Self {
        Self::new(ModelFamily::from_model_id(model_id), environment)
    }

    /// Select the agent persona for this context (builder style).
    #[must_use]
    pub fn with_agent(mut self, agent: Agent) -> Self {
        self.agent = agent;
        self
    }

    /// Render the full system message: the family base prompt, a blank line, the environment block,
    /// then the agent persona fragment when the selected agent contributes one.
    ///
    /// The default agent (`build`) contributes no fragment, so a default context renders exactly
    /// the base prompt and environment block.
    #[must_use]
    pub fn render(&self) -> String {
        let mut rendered = format!(
            "{base}\n\n{environment}",
            base = self.family.base_prompt(),
            environment = self.environment.render(),
        );
        if let Some(persona) = self.agent.persona_prompt() {
            rendered.push_str("\n\n");
            rendered.push_str(persona);
        }
        rendered
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::environment::GitStatus;

    fn environment() -> EnvironmentBlock {
        EnvironmentBlock {
            cwd: "/tmp/cortex-env-abcd".to_owned(),
            platform: "Linux".to_owned(),
            os: "linux".to_owned(),
            arch: "x86_64".to_owned(),
            date: "2026-06-19".to_owned(),
            git: GitStatus::NotARepo,
        }
    }

    #[test]
    fn render_includes_base_prompt_and_environment() {
        let context = SystemContext::for_model("google/gemini-2.5-flash", environment());
        assert_eq!(context.family, ModelFamily::Gemini);
        let rendered = context.render();
        assert!(rendered.contains("Cortex Code"), "base prompt present");
        assert!(rendered.contains("/tmp/cortex-env-abcd"), "cwd present");
        assert!(rendered.contains("Linux"), "platform present");
        assert!(rendered.contains("<environment>"), "env block present");
    }

    #[test]
    fn base_prompt_precedes_environment_block() {
        let rendered = SystemContext::new(ModelFamily::Default, environment()).render();
        let base_pos = rendered.find("Cortex Code").expect("base prompt");
        let env_pos = rendered.find("<environment>").expect("env block");
        assert!(base_pos < env_pos, "base prompt must precede the env block");
    }

    #[test]
    fn default_agent_render_omits_a_persona_fragment() {
        let with_default =
            SystemContext::new(ModelFamily::Default, environment()).with_agent(Agent::Build);
        let without = SystemContext::new(ModelFamily::Default, environment());
        assert_eq!(
            with_default.render(),
            without.render(),
            "the default build agent must not change the rendered context"
        );
        assert_eq!(without.agent, Agent::DEFAULT);
    }

    #[test]
    fn plan_agent_render_appends_the_persona_after_the_environment_block() {
        let rendered = SystemContext::new(ModelFamily::Gemini, environment())
            .with_agent(Agent::Plan)
            .render();
        let env_pos = rendered.find("<environment>").expect("env block");
        let persona_pos = rendered
            .find("plan persona")
            .expect("plan persona fragment present");
        assert!(
            env_pos < persona_pos,
            "the persona fragment must follow the environment block"
        );
        assert!(
            rendered.contains("MUST NOT"),
            "the plan persona must steer away from edits"
        );
    }

    #[test]
    fn general_agent_render_appends_its_persona() {
        let rendered = SystemContext::new(ModelFamily::Default, environment())
            .with_agent(Agent::General)
            .render();
        assert!(
            rendered.contains("general persona"),
            "the general persona fragment must be present"
        );
    }
}
