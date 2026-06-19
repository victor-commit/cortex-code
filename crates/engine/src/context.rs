//! The system-context epoch: the base system prompt joined with the dynamic environment block.
//!
//! Architecture §4 establishes a "system-context epoch" once per turn: the base system prompt for
//! the model's family (see [`ModelFamily`]) followed by the dynamic environment block (see
//! [`EnvironmentBlock`]). [`SystemContext::render`] produces the single system message string the
//! runner places at the head of the conversation.

use crate::environment::EnvironmentBlock;
use crate::family::ModelFamily;

/// The assembled system context for a turn: a model-family base prompt plus an environment block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemContext {
    /// The model family whose base prompt opens the context.
    pub family: ModelFamily,
    /// The dynamic environment snapshot appended after the base prompt.
    pub environment: EnvironmentBlock,
}

impl SystemContext {
    /// Build a context for an explicit family and environment.
    #[must_use]
    pub fn new(family: ModelFamily, environment: EnvironmentBlock) -> Self {
        Self {
            family,
            environment,
        }
    }

    /// Build a context for `model_id`, inferring its family.
    #[must_use]
    pub fn for_model(model_id: &str, environment: EnvironmentBlock) -> Self {
        Self::new(ModelFamily::from_model_id(model_id), environment)
    }

    /// Render the full system message: the family base prompt, a blank line, then the environment
    /// block.
    #[must_use]
    pub fn render(&self) -> String {
        format!(
            "{base}\n\n{environment}",
            base = self.family.base_prompt(),
            environment = self.environment.render(),
        )
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
}
