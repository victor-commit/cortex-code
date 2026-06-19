//! Model-family detection and the base system prompt selected per family.
//!
//! The system-context epoch (architecture §8) opens with a base system prompt chosen by the
//! active model's family, because different model families respond best to differently-shaped
//! instructions. The family is inferred from the resolved model id (e.g. `google/gemini-2.5-flash`
//! is the Gemini family); an id that matches no known family falls back to a neutral default
//! prompt so every model still receives a coherent system context.

/// The family a model belongs to, used to pick the base system prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelFamily {
    /// Anthropic Claude models.
    Anthropic,
    /// OpenAI GPT chat models.
    Gpt,
    /// OpenAI Codex / code-specialized models.
    Codex,
    /// Google Gemini models.
    Gemini,
    /// Any model that matches no specific family.
    Default,
}

impl ModelFamily {
    /// Infer the family from a (possibly provider-prefixed) model id.
    ///
    /// Matching is case-insensitive and substring-based so both bare ids (`gpt-4o`) and
    /// provider-qualified ids (`openai/gpt-4o`, `google/gemini-2.5-flash`) resolve correctly. The
    /// more specific Codex check precedes the general GPT check.
    #[must_use]
    pub fn from_model_id(model_id: &str) -> Self {
        let id = model_id.to_ascii_lowercase();
        if id.contains("claude") || id.contains("anthropic") {
            ModelFamily::Anthropic
        } else if id.contains("codex") {
            ModelFamily::Codex
        } else if id.contains("gpt")
            || id.starts_with("openai/")
            || id.starts_with("o1")
            || id.starts_with("o3")
        {
            ModelFamily::Gpt
        } else if id.contains("gemini") || id.starts_with("google/") {
            ModelFamily::Gemini
        } else {
            ModelFamily::Default
        }
    }

    /// The base system prompt for this family. Each is clean-room original Cortex Code text and
    /// shares a common grounding instruction so the dynamic environment block that follows is used
    /// for cwd/platform/date/repository questions.
    #[must_use]
    pub fn base_prompt(self) -> &'static str {
        match self {
            ModelFamily::Anthropic => ANTHROPIC_PROMPT,
            ModelFamily::Gpt => GPT_PROMPT,
            ModelFamily::Codex => CODEX_PROMPT,
            ModelFamily::Gemini => GEMINI_PROMPT,
            ModelFamily::Default => DEFAULT_PROMPT,
        }
    }
}

const DEFAULT_PROMPT: &str = "You are Cortex Code, a precise terminal coding agent that helps \
engineers work in their project from the command line. Use the environment context provided below \
to answer questions about the current working directory, platform, date, and repository state \
accurately. Be direct and concise; when asked to think step by step, reason first and then state \
your final answer.";

const GEMINI_PROMPT: &str = "You are Cortex Code, a precise terminal coding agent. Favor clear, \
structured answers. Use the environment context provided below to answer questions about the \
current working directory, platform, date, and repository state accurately. Be direct and \
concise; when asked to think step by step, reason first and then state your final answer.";

const ANTHROPIC_PROMPT: &str = "You are Cortex Code, a precise and careful terminal coding agent. \
Use the environment context provided below to answer questions about the current working \
directory, platform, date, and repository state accurately. Be direct and concise; when asked to \
think step by step, reason first and then state your final answer.";

const GPT_PROMPT: &str = "You are Cortex Code, a precise terminal coding agent. Prefer concise, \
actionable replies. Use the environment context provided below to answer questions about the \
current working directory, platform, date, and repository state accurately. Be direct and \
concise; when asked to think step by step, reason first and then state your final answer.";

const CODEX_PROMPT: &str = "You are Cortex Code, a code-focused terminal agent. Keep answers tight \
and code-accurate. Use the environment context provided below to answer questions about the \
current working directory, platform, date, and repository state accurately. Be direct and \
concise; when asked to think step by step, reason first and then state your final answer.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_each_known_family() {
        assert_eq!(
            ModelFamily::from_model_id("google/gemini-2.5-flash"),
            ModelFamily::Gemini
        );
        assert_eq!(
            ModelFamily::from_model_id("anthropic/claude-3.5-sonnet"),
            ModelFamily::Anthropic
        );
        assert_eq!(
            ModelFamily::from_model_id("openai/gpt-4o"),
            ModelFamily::Gpt
        );
        assert_eq!(
            ModelFamily::from_model_id("openai/codex-mini"),
            ModelFamily::Codex
        );
    }

    #[test]
    fn unknown_model_falls_back_to_default() {
        assert_eq!(
            ModelFamily::from_model_id("some/unlisted-model-v9"),
            ModelFamily::Default
        );
    }

    #[test]
    fn detection_is_case_insensitive() {
        assert_eq!(
            ModelFamily::from_model_id("GOOGLE/Gemini-2.5-Flash"),
            ModelFamily::Gemini
        );
    }

    #[test]
    fn every_family_has_a_non_empty_base_prompt() {
        for family in [
            ModelFamily::Anthropic,
            ModelFamily::Gpt,
            ModelFamily::Codex,
            ModelFamily::Gemini,
            ModelFamily::Default,
        ] {
            let prompt = family.base_prompt();
            assert!(
                !prompt.trim().is_empty(),
                "family {family:?} prompt is empty"
            );
            assert!(prompt.contains("Cortex Code"));
        }
    }
}
