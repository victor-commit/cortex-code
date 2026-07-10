//! The tool registry: the set of tools advertised to the model and the dispatch of a tool call to
//! its implementation.
//!
//! The registry owns the boxed [`Tool`] implementations, projects them into the provider-agnostic
//! [`llm::ToolDef`] catalogue the runner advertises, and routes an assembled tool call (a name plus
//! a JSON arguments string) to the matching tool. An unknown tool or unparseable arguments resolve
//! to an error [`ToolOutput`] rather than aborting the turn.

use std::collections::BTreeSet;

use llm::ToolDef;

use crate::apply_patch::ApplyPatchTool;
use crate::bash::BashTool;
use crate::edit::EditTool;
use crate::glob::GlobTool;
use crate::grep::GrepTool;
use crate::invalid::{InvalidTool, invalid_arguments_output, unknown_tool_output};
use crate::loop_safeguard::LoopSafeguardTool;
use crate::lsp::LspTool;
use crate::plan::PlanTool;
use crate::plan_enter::PlanEnterTool;
use crate::plan_exit::PlanExitTool;
use crate::plan_exit_compat::PlanExitCompatTool;
use crate::question::QuestionTool;
use crate::read::ReadTool;
use crate::skill::SkillTool;
use crate::task::TaskTool;
use crate::todo::TodoTool;
use crate::tool::{TaskTarget, Tool, ToolContext, ToolOutput};
use crate::webfetch::WebFetchTool;
use crate::websearch::{WebSearchConfig, WebSearchTool};
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
    /// e.g. `google/gemini-2.5-flash`): the mutating `read`/`write`/`edit`, shared utility tools,
    /// and advisory capability hooks. This set has no `apply_patch`.
    /// `websearch` is deliberately omitted until a surface supplies an explicit provider config.
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
    /// `apply_patch` plus the utility tools `bash`/`glob`/`grep`/`lsp`/`todo`/`webfetch` and
    /// advisory tools `question`/`plan`/`plan-enter`/`plan-exit`/`plan_exit`/`invalid`/
    /// `loop-safeguard`. GPT-family
    /// models edit files through the `apply_patch` envelope, so `edit` and `write` are deliberately
    /// swapped out (architecture §3.4, model-aware tool materialization); the shell/code-search/LSP/
    /// todo utilities are model-agnostic and present for every family. `websearch` is deliberately
    /// omitted until a surface supplies an explicit provider config.
    #[must_use]
    pub fn gpt() -> Self {
        let mut registry = Self::new();
        registry.register(Box::new(ReadTool));
        registry.register(Box::new(ApplyPatchTool));
        registry.register_utility_tools();
        registry
    }

    /// The stable union of every model-family built-in id.
    ///
    /// MCP managers reserve this complete set before connecting so their model-visible ids are
    /// surface-independent and no MCP capability can shadow either the default or GPT-family
    /// built-in materialization.
    #[must_use]
    pub fn reserved_builtin_ids() -> Vec<String> {
        Self::builtin()
            .definitions()
            .into_iter()
            .chain(Self::gpt().definitions())
            .map(|definition| definition.name)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    /// Register the utility tools shared by every model family (`bash`, `glob`, `grep`, `lsp`,
    /// `todo`, `webfetch`) plus advisory capability hooks (`question`, `plan`, `plan-enter`,
    /// `plan-exit`, `plan_exit`, `invalid`, `loop-safeguard`).
    fn register_utility_tools(&mut self) {
        self.register(Box::new(BashTool));
        self.register(Box::new(GlobTool));
        self.register(Box::new(GrepTool));
        self.register(Box::new(LspTool));
        self.register(Box::new(TodoTool));
        self.register(Box::new(WebFetchTool));
        self.register(Box::new(QuestionTool));
        self.register(Box::new(PlanTool));
        self.register(Box::new(PlanEnterTool));
        self.register(Box::new(PlanExitTool));
        self.register(Box::new(PlanExitCompatTool));
        self.register(Box::new(InvalidTool));
        self.register(Box::new(LoopSafeguardTool));
    }

    /// Add the real `websearch` tool with an explicit provider configuration.
    ///
    /// The default registries do not include `websearch`, so a run with no provider configured is
    /// unmistakably default-off. Calling this method is the single materialization seam used by
    /// surfaces after they have resolved `[websearch] enabled = true`.
    pub fn register_websearch(&mut self, config: WebSearchConfig) {
        self.register(Box::new(WebSearchTool::new(config)));
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

    /// Replace any already-registered tool sharing `tool`'s name with `tool`, appending it when no
    /// tool by that name exists.
    ///
    /// [`get`](Self::get) resolves the first tool matching a name, so a surface that materializes a
    /// capability-backed variant (for example an ACP client filesystem `read`/`write`) must
    /// override the built-in rather than merely register alongside it, or the built-in would keep
    /// winning dispatch.
    pub fn override_tool(&mut self, tool: Box<dyn Tool>) {
        let name = tool.name().to_owned();
        self.tools.retain(|existing| existing.name() != name);
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
            return unknown_tool_output(name);
        };
        let parsed = if arguments.trim().is_empty() {
            serde_json::Value::Object(serde_json::Map::new())
        } else {
            match serde_json::from_str(arguments) {
                Ok(value) => value,
                Err(err) => {
                    return invalid_arguments_output(name, arguments, &err);
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
            for name in [
                "bash",
                "glob",
                "grep",
                "lsp",
                "todo",
                "webfetch",
                "question",
                "plan",
                "plan-enter",
                "plan-exit",
                "plan_exit",
                "invalid",
                "loop-safeguard",
            ] {
                assert!(
                    registry.get(name).is_some(),
                    "expected the `{name}` utility tool to be registered"
                );
                assert!(
                    registry.definitions().iter().any(|def| def.name == name),
                    "expected `{name}` in the advertised catalogue"
                );
            }
            assert!(
                registry.get("websearch").is_none(),
                "websearch is default-off until configured"
            );
        }
    }

    #[test]
    fn websearch_registers_only_with_explicit_config() {
        let mut registry = ToolRegistry::builtin();
        assert!(registry.get("websearch").is_none());
        registry.register_websearch(WebSearchConfig::duckduckgo());
        assert!(registry.get("websearch").is_some());
        assert!(
            registry
                .definitions()
                .iter()
                .any(|def| def.name == "websearch"),
            "configured websearch must be advertised"
        );
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
        assert_eq!(
            task.parameters["properties"]["subagent_type"]["type"],
            serde_json::json!("string")
        );
        assert!(
            task.parameters["properties"]["subagent_type"]["description"]
                .as_str()
                .unwrap_or_default()
                .contains("explore")
        );
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
        let name_description = skill.parameters["properties"]["name"]["description"]
            .as_str()
            .expect("name description");
        assert!(name_description.contains("date-helper"));
        assert!(skill.parameters["properties"]["name"]["enum"].is_null());
        assert_eq!(
            skill.parameters["anyOf"][0]["required"][0],
            serde_json::Value::String("name".to_owned())
        );
        assert_eq!(
            skill.parameters["anyOf"][1]["required"][0],
            serde_json::Value::String("skill".to_owned())
        );
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

    #[tokio::test]
    async fn override_tool_replaces_the_builtin_and_wins_dispatch() {
        struct StubRead;
        impl Tool for StubRead {
            fn name(&self) -> &str {
                "read"
            }
            fn description(&self) -> &str {
                "stub read"
            }
            fn parameters(&self) -> serde_json::Value {
                serde_json::json!({ "type": "object" })
            }
            fn execute<'a>(
                &'a self,
                _arguments: serde_json::Value,
                _ctx: &'a ToolContext,
            ) -> futures_util::future::BoxFuture<'a, ToolOutput> {
                Box::pin(async { ToolOutput::text("OVERRIDDEN-READ") })
            }
        }

        let mut registry = ToolRegistry::builtin();
        let before = registry.len();
        registry.override_tool(Box::new(StubRead));

        assert_eq!(
            registry.len(),
            before,
            "override must replace, not grow the registry"
        );
        assert_eq!(
            registry
                .definitions()
                .iter()
                .filter(|def| def.name == "read")
                .count(),
            1,
            "exactly one `read` tool may be advertised after override"
        );

        let ctx = ToolContext::new(std::env::temp_dir());
        let output = registry.execute("read", "{}", &ctx).await;
        assert!(!output.is_error);
        assert_eq!(output.model_content(), "OVERRIDDEN-READ");
    }
}
