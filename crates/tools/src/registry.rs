//! The tool registry: the set of tools advertised to the model and the dispatch of a tool call to
//! its implementation.
//!
//! The registry owns the boxed [`Tool`] implementations, projects them into the provider-agnostic
//! [`llm::ToolDef`] catalogue the runner advertises, and routes an assembled tool call (a name plus
//! a JSON arguments string) to the matching tool. An unknown tool or unparseable arguments resolve
//! to an error [`ToolOutput`] rather than aborting the turn.

use llm::ToolDef;

use crate::read::ReadTool;
use crate::tool::{Tool, ToolContext, ToolOutput};
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

    /// A registry pre-populated with the built-in tools available in headless `cortex run`.
    #[must_use]
    pub fn builtin() -> Self {
        let mut registry = Self::new();
        registry.register(Box::new(ReadTool));
        registry.register(Box::new(WriteTool));
        registry
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
    fn builtin_registry_advertises_read_and_write() {
        let registry = ToolRegistry::builtin();
        let defs = registry.definitions();
        assert!(defs.iter().any(|def| def.name == "read"));
        assert!(registry.get("read").is_some());
        assert!(defs.iter().any(|def| def.name == "write"));
        assert!(registry.get("write").is_some());
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
