//! The `skill` tool: engage one discovered markdown skill for the rest of the turn.
//!
//! Skill discovery lives in the `skills` crate. This tool receives the already-discovered valid
//! skills from the surface, advertises only their names/descriptions, and returns the chosen
//! skill's guidance only when the model explicitly engages it. The tool separately relays only the
//! already-parsed body as a typed system-context update for the continuation round.

use std::fs;
use std::path::{Path, PathBuf};

use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use skills::Skill;
use url::Url;

use crate::tool::{Tool, ToolContext, ToolOutput};

const FILE_LIMIT: usize = 24;

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
            .map(|skill| skill.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": format!(
                        "The skill name to engage. Available skills: {names}"
                    )
                },
                "skill": {
                    "type": "string",
                    "description": "Compatibility alias for `name`."
                }
            },
            "anyOf": [
                { "required": ["name"] },
                { "required": ["skill"] }
            ],
            "additionalProperties": false
        })
    }

    fn execute<'a>(&'a self, arguments: Value, ctx: &'a ToolContext) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let Some(name) = arguments
                .get("name")
                .and_then(Value::as_str)
                .or_else(|| arguments.get("skill").and_then(Value::as_str))
            else {
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
            if let Err(message) = ctx.ensure_permitted("skill", &skill.name) {
                return ToolOutput::error(message);
            }

            ToolOutput::text(render_skill_content(skill, ctx))
                .with_trusted_skill_guidance(skill.trusted_guidance())
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
    out.push_str(
        "\n\nWhen engaged, the tool returns skill guidance plus the skill base directory and a \
         file inventory with per-file permission status.",
    );
    out
}

fn render_skill_content(skill: &Skill, ctx: &ToolContext) -> String {
    let guidance = skill.guidance().trim();
    let (base_dir, files) = skill_inventory(skill, ctx);
    let base_uri = Url::from_directory_path(&base_dir)
        .map(|url| url.to_string())
        .unwrap_or_else(|_| base_dir.to_string_lossy().to_string());

    let mut lines = vec![
        format!("<skill_content name=\"{}\">", skill.name),
        format!("# Skill: {}", skill.name),
        String::new(),
        guidance.to_owned(),
        String::new(),
        format!("Base directory for this skill: {base_uri}"),
        "Relative paths in this skill are relative to this base directory.".to_owned(),
        "Note: file list is sampled and includes permission status.".to_owned(),
        String::new(),
        "<skill_files>".to_owned(),
    ];

    if files.is_empty() {
        lines.push("<file status=\"none\">(no supporting files discovered)</file>".to_owned());
    } else {
        for (path, status) in files {
            lines.push(format!("<file status=\"{status}\">{path}</file>"));
        }
    }

    lines.push("</skill_files>".to_owned());
    lines.push("</skill_content>".to_owned());
    lines.join("\n")
}

fn skill_inventory(skill: &Skill, ctx: &ToolContext) -> (PathBuf, Vec<(String, &'static str)>) {
    let base_dir = skill
        .source
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let source_file = skill
        .source
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned);
    let files = discover_skill_files(&base_dir, source_file.as_deref());

    let inventory = files
        .into_iter()
        .take(FILE_LIMIT)
        .map(|file| {
            let relative = file
                .strip_prefix(&base_dir)
                .ok()
                .unwrap_or(file.as_path())
                .to_string_lossy()
                .replace('\\', "/");
            let status = if ctx
                .permission_check("read", &file.to_string_lossy())
                .allowed
            {
                "allowed"
            } else {
                "permission_denied"
            };
            (relative, status)
        })
        .collect::<Vec<_>>();

    (base_dir, inventory)
}

fn discover_skill_files(base_dir: &Path, source_file: Option<&str>) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![base_dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = fs::read_dir(&current) else {
            continue;
        };
        let mut paths = entries
            .flatten()
            .map(|entry| entry.path())
            .collect::<Vec<_>>();
        paths.sort();
        for path in paths {
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.is_file() {
                if source_file
                    .and_then(|name| {
                        path.file_name()
                            .and_then(|file| file.to_str())
                            .map(|file| file == name)
                    })
                    .unwrap_or(false)
                {
                    continue;
                }
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;

    use super::*;
    use permissions::{Action, PermissionEngine, Rule, Ruleset};

    fn skill(name: &str, description: &str, guidance: &str) -> Skill {
        skill_at(name, description, guidance, PathBuf::from("SKILL.md"))
    }

    fn skill_at(name: &str, description: &str, guidance: &str, source: PathBuf) -> Skill {
        skills::parse_skill_markdown(
            &format!("---\nname: {name}\ndescription: {description}\n---\n{guidance}\n"),
            source,
        )
        .expect("valid test skill")
    }

    #[test]
    fn advertises_skill_names_in_schema_without_guidance() {
        let tool = SkillTool::new(vec![skill(
            "date-helper",
            "Use when formatting dates",
            "Append SKILL-DATE-OK.",
        )]);

        let parameters = tool.parameters();
        let description = parameters["properties"]["name"]["description"]
            .as_str()
            .expect("name description");
        assert!(description.contains("date-helper"));
        assert!(parameters["properties"]["name"]["enum"].is_null());
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
        assert_eq!(
            output
                .trusted_skill_guidance()
                .expect("parsed skill produces a trusted update")
                .as_str(),
            "Append SKILL-ALPHA."
        );
        assert!(
            !output
                .trusted_skill_guidance()
                .expect("parsed skill produces a trusted update")
                .as_str()
                .contains("Skill `alpha`"),
            "descriptive presentation text must not be promoted"
        );
        assert!(
            !content.contains("SKILL-BETA"),
            "engaging one skill must not inject another"
        );
    }

    #[tokio::test]
    async fn accepts_skill_alias_argument_name() {
        let tool = SkillTool::new(vec![skill("alpha", "Use alpha", "Append SKILL-ALPHA.")]);
        let ctx = ToolContext::new(std::env::temp_dir());

        let output = tool.execute(json!({ "skill": "alpha" }), &ctx).await;
        assert!(!output.is_error);
        assert!(output.model_content().contains("SKILL-ALPHA"));
    }

    #[tokio::test]
    async fn denied_skill_emits_no_trusted_system_context_update() {
        let tool = SkillTool::new(vec![skill("alpha", "Use alpha", "Append SKILL-ALPHA.")]);
        let permissions = Arc::new(PermissionEngine::new(Ruleset::new(vec![Rule::new(
            "skill",
            "*",
            Action::Deny,
        )])));
        let ctx = ToolContext::new(std::env::temp_dir()).with_permissions(permissions);

        let output = tool.execute(json!({ "name": "alpha" }), &ctx).await;

        assert!(output.is_error, "denied engagement is an error");
        assert!(
            output.trusted_skill_guidance().is_none(),
            "a denied call must not carry trusted guidance"
        );
        assert!(!output.model_content().contains("SKILL-ALPHA"));
    }

    #[tokio::test]
    async fn includes_base_directory_and_file_inventory_in_skill_output() {
        let root = std::env::temp_dir().join(format!(
            "cortex-skill-tool-inventory-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("references")).expect("create references dir");
        fs::write(root.join("references").join("guide.md"), "# guide").expect("write guide");
        fs::write(root.join("helper.sh"), "echo helper").expect("write helper");

        let tool = SkillTool::new(vec![skill_at(
            "alpha",
            "Use alpha",
            "Append SKILL-ALPHA.",
            root.join("SKILL.md"),
        )]);
        let ctx = ToolContext::new(root.clone());

        let output = tool.execute(json!({ "name": "alpha" }), &ctx).await;
        assert!(!output.is_error);
        let content = output.model_content();
        assert!(
            content.contains("Base directory for this skill:"),
            "{content}"
        );
        assert!(
            content.contains("references/guide.md") || content.contains("helper.sh"),
            "{content}"
        );
    }

    #[tokio::test]
    async fn inventory_marks_permission_denied_files() {
        let root = std::env::temp_dir().join(format!(
            "cortex-skill-tool-permissions-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("references")).expect("create references dir");
        fs::create_dir_all(root.join("secrets")).expect("create secrets dir");
        fs::write(root.join("references").join("guide.md"), "# guide").expect("write guide");
        fs::write(root.join("secrets").join("private.md"), "# secret").expect("write secret");

        let tool = SkillTool::new(vec![skill_at(
            "alpha",
            "Use alpha",
            "Append SKILL-ALPHA.",
            root.join("SKILL.md"),
        )]);
        let permissions = Arc::new(PermissionEngine::new(Ruleset::new(vec![
            Rule::new("skill", "*", Action::Allow),
            Rule::new("read", "*", Action::Allow),
            Rule::new("read", "secrets/**", Action::Deny),
        ])));
        let ctx = ToolContext::new(root.clone()).with_permissions(permissions);

        let output = tool.execute(json!({ "name": "alpha" }), &ctx).await;
        assert!(!output.is_error);
        let content = output.model_content();
        assert!(
            content.contains("status=\"allowed\""),
            "allowed file status should be explicit: {content}"
        );
        assert!(
            content.contains("status=\"permission_denied\""),
            "denied file status should be explicit: {content}"
        );
    }
}
