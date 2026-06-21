//! The tool registry: the set of tools advertised to the model and the dispatch of a tool call to
//! its implementation.
//!
//! The registry owns the boxed [`Tool`] implementations, projects them into the provider-agnostic
//! [`llm::ToolDef`] catalogue the runner advertises, and routes an assembled tool call (a name plus
//! a JSON arguments string) to the matching tool. An unknown tool or unparseable arguments resolve
//! to an error [`ToolOutput`] rather than aborting the turn.

use llm::ToolDef;

use crate::apply_patch::ApplyPatchTool;
use crate::bash::BashTool;
use crate::edit::EditTool;
use crate::glob::GlobTool;
use crate::grep::GrepTool;
use crate::lsp::LspTool;
use crate::read::ReadTool;
use crate::skill::SkillTool;
use crate::task::TaskTool;
use crate::todo::TodoTool;
use crate::tool::{TaskTarget, Tool, ToolContext, ToolOutput};
use crate::write::WriteTool;

/// An ordered set of callable tools.
#[derive(Default)]
pub struct ToolRegistry {
    tools: Vec<Box<dyn Tool>>,
}

impl ToolRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self { tools: Vec::new() }
    }

    /// A registry pre-populated with the built-in tool set for non-GPT models (the default,
    /// e.g. `google/gemini-2.5-flash`): the mutating `read`/`write`/`edit` plus the utility tools
    /// `bash`/`glob`/`grep`/`lsp`/`todo`. This set has no `apply_patch`.
    #[must_use]
    pub fn builtin() -> Self {
        let mut registry = Self::new();
        registry.register(Box::new(ReadTool));
        registry.register(Box::new(WriteTool));
        registry.register(Box::new(EditTool));
        registry.register_utility_tools();
        registry
    }

    /// A registry pre-populated with the built-in tool set for GPT-family models: `read` and
    /// `apply_patch` plus the utility tools `bash`/`glob`/`grep`/`lsp`/`todo`. GPT-family models
    /// edit files through the `apply_patch` envelope, so `edit` and `write` are deliberately swapped
    /// out (architecture §3.4, model-aware tool materialization); the shell/search/LSP/todo
    /// utilities are model-agnostic and present for every family.
    #[must_use]
    pub fn gpt() -> Self {
        let mut registry = Self::new();
        registry.register(Box::new(ReadTool));
        registry.register(Box::new(ApplyPatchTool));
        registry.register_utility_tools();
        registry
    }

    /// Register the utility tools shared by every model family (`bash`, `glob`, `grep`, `lsp`,
    /// `todo`).
    fn register_utility_tools(&mut self) {
        self.register(Box::new(BashTool));
        self.register(Box::new(GlobTool));
        self.register(Box::new(GrepTool));
        self.register(Box::new(LspTool));
        self.register(Box::new(TodoTool));
    }

    /// Add the `task` tool with the already-filtered subagent targets.
    ///
    /// The registry omits `task` when `targets` is empty, so the model is not offered a delegation
    /// tool that could never spawn anything.
    pub fn register_task(&mut self, targets: Vec<TaskTarget>) {
        if !targets.is_empty() {
            self.register(Box::new(TaskTool::new(targets)));
        }
    }

    /// Add the `skill` tool with the already-discovered valid markdown skills.
    ///
    /// The registry omits `skill` when no skills are present, so an unrelated project exposes no
    /// inert skill selector and no skill guidance can leak into a turn.
    pub fn register_skills(&mut self, skills: Vec<skills::Skill>) {
        if !skills.is_empty() {
            self.register(Box::new(SkillTool::new(skills)));
        }
    }

    /// Add a tool to the registry.
    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.push(tool);
    }

    /// The number of registered tools.
    #[must_use]
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Whether the registry has no tools.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// The tool named `name`, if registered.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools
            .iter()
            .find(|tool| tool.name() == name)
            .map(AsRef::as_ref)
    }

    /// The provider-agnostic tool catalogue advertised to the model.
    #[must_use]
    pub fn definitions(&self) -> Vec<ToolDef> {
        self.tools
            .iter()
            .map(|tool| ToolDef {
                name: tool.name().to_owned(),
                description: tool.description().to_owned(),
                parameters: tool.parameters(),
            })
            .collect()
    }

    /// Execute the tool named `name` with the assembled JSON `arguments` string against `ctx`.
    ///
    /// An unknown tool or an unparseable arguments string resolves to an error [`ToolOutput`] so the
    /// turn continues and the model can recover. A blank arguments string is treated as an empty
    /// object so a no-argument tool call still dispatches.
    pub async fn execute(&self, name: &str, arguments: &str, ctx: &ToolContext) -> ToolOutput {
        let Some(tool) = self.get(name) else {
            return ToolOutput::error(format!("unknown tool: {name}"));
        };
        let parsed = if arguments.trim().is_empty() {
            serde_json::Value::Object(serde_json::Map::new())
        } else {
            match serde_json::from_str(arguments) {
                Ok(value) => value,
                Err(err) => {
                    return ToolOutput::error(format!("{name}: invalid arguments JSON: {err}"));
                }
            }
        };
        tool.execute(parsed, ctx).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_registry_advertises_read_write_and_edit() {
        let registry = ToolRegistry::builtin();
        let defs = registry.definitions();
        assert!(defs.iter().any(|def| def.name == "read"));
        assert!(registry.get("read").is_some());
        assert!(defs.iter().any(|def| def.name == "write"));
        assert!(registry.get("write").is_some());
        assert!(defs.iter().any(|def| def.name == "edit"));
        assert!(registry.get("edit").is_some());
    }

    #[test]
    fn both_registries_advertise_the_utility_tools() {
        for registry in [ToolRegistry::builtin(), ToolRegistry::gpt()] {
            for name in ["bash", "glob", "grep", "lsp", "todo"] {
                assert!(
                    registry.get(name).is_some(),
                    "expected the `{name}` utility tool to be registered"
                );
                assert!(
                    registry.definitions().iter().any(|def| def.name == name),
                    "expected `{name}` in the advertised catalogue"
                );
            }
        }
    }

    #[test]
    fn task_tool_is_registered_only_when_targets_exist() {
        let mut registry = ToolRegistry::builtin();
        registry.register_task(Vec::new());
        assert!(registry.get("task").is_none());

        registry.register_task(vec![TaskTarget::new("explore", "Read-only exploration")]);
        assert!(registry.get("task").is_some());
        let task = registry
            .definitions()
            .into_iter()
            .find(|def| def.name == "task")
            .expect("task definition");
        let enum_values: Vec<&str> = task.parameters["properties"]["agent"]["enum"]
            .as_array()
            .expect("enum")
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect();
        assert_eq!(enum_values, vec!["explore"]);
    }

    #[test]
    fn skill_tool_is_registered_only_when_skills_exist() {
        let mut registry = ToolRegistry::builtin();
        registry.register_skills(Vec::new());
        assert!(registry.get("skill").is_none());

        registry.register_skills(vec![skills::Skill {
            name: "date-helper".to_owned(),
            description: "Use when formatting dates".to_owned(),
            guidance: "Append SKILL-DATE-OK.".to_owned(),
            source: std::path::PathBuf::from("SKILL.md"),
        }]);
        assert!(registry.get("skill").is_some());
        let skill = registry
            .definitions()
            .into_iter()
            .find(|def| def.name == "skill")
            .expect("skill definition");
        let enum_values: Vec<&str> = skill.parameters["properties"]["name"]["enum"]
            .as_array()
            .expect("enum")
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect();
        assert_eq!(enum_values, vec!["date-helper"]);
        assert!(
            !skill.description.contains("SKILL-DATE-OK"),
            "skill guidance must not leak through the tool definition"
        );
    }

    #[test]
    fn builtin_registry_has_no_apply_patch() {
        let registry = ToolRegistry::builtin();
        assert!(registry.get("apply_patch").is_none());
        assert!(
            !registry
                .definitions()
                .iter()
                .any(|def| def.name == "apply_patch")
        );
    }

    #[test]
    fn gpt_registry_advertises_apply_patch_and_omits_edit_and_write() {
        let registry = ToolRegistry::gpt();
        let names: Vec<String> = registry
            .definitions()
            .into_iter()
            .map(|def| def.name)
            .collect();
        assert!(names.contains(&"apply_patch".to_owned()), "got: {names:?}");
        assert!(names.contains(&"read".to_owned()), "got: {names:?}");
        assert!(!names.contains(&"edit".to_owned()), "got: {names:?}");
        assert!(!names.contains(&"write".to_owned()), "got: {names:?}");
        assert!(registry.get("apply_patch").is_some());
        assert!(registry.get("edit").is_none());
        assert!(registry.get("write").is_none());
    }

    #[tokio::test]
    async fn unknown_tool_resolves_to_an_error_output() {
        let registry = ToolRegistry::builtin();
        let ctx = ToolContext::new(std::env::temp_dir());
        let output = registry.execute("nope", "{}", &ctx).await;
        assert!(output.is_error);
        assert!(output.model_content().contains("unknown tool"));
    }

    #[tokio::test]
    async fn invalid_arguments_resolve_to_an_error_output() {
        let registry = ToolRegistry::builtin();
        let ctx = ToolContext::new(std::env::temp_dir());
        let output = registry.execute("read", "{not json", &ctx).await;
        assert!(output.is_error);
        assert!(output.model_content().contains("invalid arguments"));
    }
}
