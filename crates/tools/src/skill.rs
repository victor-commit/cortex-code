//! The `skill` tool: engage one discovered markdown skill for the rest of the turn.
//!
//! Skill discovery lives in the `skills` crate. This tool receives the already-discovered valid
//! skills from the surface, advertises only their names/descriptions, and returns the chosen
//! skill's guidance only when the model explicitly engages it. The engine then treats a successful
//! `skill` tool result as system-context guidance for the continuation round.

use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use skills::Skill;

use crate::tool::{Tool, ToolContext, ToolOutput};

/// Model-facing tool used to engage a discovered markdown skill.
pub struct SkillTool {
    skills: Vec<Skill>,
    description: String,
}

impl SkillTool {
    /// Build a skill tool from the valid discovered skills.
    #[must_use]
    pub fn new(mut skills: Vec<Skill>) -> Self {
        skills.sort_by(|a, b| a.name.cmp(&b.name));
        let description = render_description(&skills);
        Self {
            skills,
            description,
        }
    }

    fn find(&self, name: &str) -> Option<&Skill> {
        let normalized = name.trim().to_ascii_lowercase();
        self.skills.iter().find(|skill| skill.name == normalized)
    }
}

impl Tool for SkillTool {
    fn name(&self) -> &str {
        "skill"
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters(&self) -> Value {
        let names = self
            .skills
            .iter()
            .map(|skill| Value::String(skill.name.clone()))
            .collect::<Vec<_>>();
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "The skill name to engage.",
                    "enum": names
                }
            },
            "required": ["name"],
            "additionalProperties": false
        })
    }

    fn execute<'a>(&'a self, arguments: Value, ctx: &'a ToolContext) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let Some(name) = arguments.get("name").and_then(Value::as_str) else {
                return ToolOutput::error("skill: missing required string argument `name`");
            };
            let Some(skill) = self.find(name) else {
                return ToolOutput::error(format!(
                    "skill: unknown skill `{}` (available: {})",
                    name,
                    self.skills
                        .iter()
                        .map(|skill| skill.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            };
            if !ctx.permits("skill", &skill.name) {
                return ToolOutput::error(format!("skill: permission denied for `{}`", skill.name));
            }

            ToolOutput::text(format!(
                "Skill `{}` engaged. Treat the following guidance as system-level instructions \
                 for the rest of this turn:\n\n{}",
                skill.name, skill.guidance
            ))
        })
    }
}

fn render_description(skills: &[Skill]) -> String {
    let mut out = String::from(
        "Engage one discovered skill by name. Call this before answering when the user's task \
         matches a skill description. A skill's guidance is not active until this tool is called.\n\
         Available skills:",
    );
    for skill in skills {
        out.push_str("\n- ");
        out.push_str(&skill.name);
        out.push_str(": ");
        out.push_str(skill.description.trim());
    }
    out
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn skill(name: &str, description: &str, guidance: &str) -> Skill {
        Skill {
            name: name.to_owned(),
            description: description.to_owned(),
            guidance: guidance.to_owned(),
            source: PathBuf::from("SKILL.md"),
        }
    }

    #[test]
    fn advertises_skill_names_in_schema_without_guidance() {
        let tool = SkillTool::new(vec![skill(
            "date-helper",
            "Use when formatting dates",
            "Append SKILL-DATE-OK.",
        )]);

        let parameters = tool.parameters();
        assert_eq!(parameters["properties"]["name"]["enum"][0], "date-helper");
        assert!(tool.description().contains("Use when formatting dates"));
        assert!(
            !tool.description().contains("SKILL-DATE-OK"),
            "the model-facing advertisement must not leak guidance"
        );
    }

    #[tokio::test]
    async fn returns_guidance_only_for_the_engaged_skill() {
        let tool = SkillTool::new(vec![
            skill("alpha", "Use alpha", "Append SKILL-ALPHA."),
            skill("beta", "Use beta", "Append SKILL-BETA."),
        ]);
        let ctx = ToolContext::new(std::env::temp_dir());

        let output = tool.execute(json!({ "name": "alpha" }), &ctx).await;

        assert!(!output.is_error);
        let content = output.model_content();
        assert!(content.contains("SKILL-ALPHA"));
        assert!(
            !content.contains("SKILL-BETA"),
            "engaging one skill must not inject another"
        );
    }
}
