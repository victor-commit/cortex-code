//! The `todo` tool: a structured, per-session task list the model maintains across tool calls.
//!
//! Behavior (architecture §3.4, validation contract VAL-TOOLS-038/039):
//!
//! - A call carrying a `todos` array commits the complete list (each item is `content` + a
//!   `pending`/`in_progress`/`completed`/`cancelled` status + optional priority), replacing the
//!   session's prior list wholesale, and returns the stored list. Re-committing the full list with
//!   one item's fields changed is how the model updates a single task while the others keep their
//!   prior values.
//! - A call with no `todos` reads the current list (useful for confirming state).
//! - The list is scoped to the active session through the [`TodoStore`](crate::tool::TodoStore)
//!   seam, so one session never sees another's list and a fresh session starts empty.

use futures_util::future::BoxFuture;
use protocol::{TodoItem, TodoPriority, TodoStatus};
use serde_json::{Value, json};

use crate::tool::{Tool, ToolContext, ToolOutput};

const DESCRIPTION: &str = "Maintain a structured task list for the current session. Pass `todos` \
as the complete array of items you want the list to hold; each item has `content` (the task text) \
and `status` (one of `pending`, `in_progress`, `completed`, `cancelled`). You can also set \
`priority` (`high`, `medium`, `low`), defaulting to `medium` when omitted. Each call replaces the \
whole list, so to update a single task, resend the full list with that task's fields changed. Call \
with no `todos` to read the current list. The list is private to this session.";

/// The `todo` tool.
pub struct TodoTool;

impl Tool for TodoTool {
    fn name(&self) -> &str {
        "todo"
    }

    fn description(&self) -> &str {
        DESCRIPTION
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "todos": {
                    "type": "array",
                    "description": "The complete todo list to store for this session (omit to read the current list).",
                    "items": {
                        "type": "object",
                        "properties": {
                            "content": {
                                "type": "string",
                                "description": "The task text."
                            },
                            "status": {
                                "type": "string",
                                "enum": ["pending", "in_progress", "completed", "cancelled"],
                                "description": "The task's lifecycle status."
                            },
                            "priority": {
                                "type": "string",
                                "enum": ["high", "medium", "low"],
                                "description": "Optional task priority. Defaults to medium when omitted."
                            }
                        },
                        "required": ["content", "status"]
                    }
                }
            }
        })
    }

    fn execute<'a>(&'a self, arguments: Value, ctx: &'a ToolContext) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move { run(&arguments, ctx) })
    }
}

/// Execute a todo read or replace against the resolved arguments.
fn run(arguments: &Value, ctx: &ToolContext) -> ToolOutput {
    let Some(session_id) = ctx.session_id() else {
        return ToolOutput::error("todo: no active session; the todo list requires a session");
    };
    if let Err(message) = ctx.ensure_permitted("todo", session_id) {
        return ToolOutput::error(format!("todo: {message}"));
    }
    let Some(store) = ctx.todo_store() else {
        return ToolOutput::error("todo: no todo store is configured for this session");
    };

    // A present `todos` argument is a write (replace the whole list); its absence is a read.
    match arguments.get("todos") {
        Some(Value::Null) | None => render_list(store.list(session_id)),
        Some(Value::Array(raw)) => {
            let items = match parse_items(raw) {
                Ok(items) => items,
                Err(message) => return ToolOutput::error(format!("todo: {message}")),
            };
            if let Err(message) = store.replace(session_id, &items) {
                return ToolOutput::error(format!("todo: {message}"));
            }
            render_list(store.list(session_id))
        }
        Some(_) => ToolOutput::error("todo: 'todos' must be an array of items"),
    }
}

/// Parse the raw `todos` array into typed items: each needs a non-empty `content` (or `text`) and a
/// recognized `status` (defaulting to `pending` when absent). `priority` is optional and defaults to
/// `medium`.
fn parse_items(raw: &[Value]) -> Result<Vec<TodoItem>, String> {
    let mut items = Vec::with_capacity(raw.len());
    for (index, value) in raw.iter().enumerate() {
        let content = value
            .get("content")
            .or_else(|| value.get("text"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|content| !content.is_empty())
            .ok_or_else(|| format!("item {} is missing a non-empty 'content'", index + 1))?;

        let status = match value.get("status").and_then(Value::as_str) {
            Some(token) => TodoStatus::parse(token).ok_or_else(|| {
                format!(
                    "item {} has an invalid status '{token}' (use pending, in_progress, completed, or cancelled)",
                    index + 1
                )
            })?,
            None => TodoStatus::Pending,
        };
        let priority = match value.get("priority").and_then(Value::as_str) {
            Some(token) => TodoPriority::parse(token).ok_or_else(|| {
                format!(
                    "item {} has an invalid priority '{token}' (use high, medium, or low)",
                    index + 1
                )
            })?,
            None => TodoPriority::Medium,
        };
        items.push(TodoItem::new_with_priority(content, status, priority));
    }
    Ok(items)
}

/// Render the stored list (or a read/store error) as the tool result.
fn render_list(result: Result<Vec<TodoItem>, String>) -> ToolOutput {
    match result {
        Ok(items) => ToolOutput::text(render_items(&items)),
        Err(message) => ToolOutput::error(format!("todo: {message}")),
    }
}

/// Render the items as a stable, numbered, model-readable list (or a clear empty indication).
fn render_items(items: &[TodoItem]) -> String {
    if items.is_empty() {
        return "(no todos)".to_owned();
    }
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            format!(
                "{}. [{}][{}] {}",
                index + 1,
                item.status.as_str(),
                item.priority.as_str(),
                item.content
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use permissions::{Action, PermissionEngine, Rule, Ruleset};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use crate::tool::TodoStore;

    /// An in-memory [`TodoStore`] double keyed by session id, isolating the tool logic from SQLite.
    #[derive(Default)]
    struct MemoryTodoStore {
        sessions: Mutex<HashMap<String, Vec<TodoItem>>>,
    }

    impl TodoStore for MemoryTodoStore {
        fn list(&self, session_id: &str) -> Result<Vec<TodoItem>, String> {
            Ok(self
                .sessions
                .lock()
                .unwrap()
                .get(session_id)
                .cloned()
                .unwrap_or_default())
        }

        fn replace(&self, session_id: &str, items: &[TodoItem]) -> Result<(), String> {
            self.sessions
                .lock()
                .unwrap()
                .insert(session_id.to_owned(), items.to_vec());
            Ok(())
        }
    }

    fn ctx_with_store(session: &str, store: Arc<MemoryTodoStore>) -> ToolContext {
        ToolContext::new(std::env::temp_dir())
            .with_session(session)
            .with_todo_store(store)
    }

    fn write_args(items: &[(&str, &str)]) -> Value {
        let todos: Vec<Value> = items
            .iter()
            .map(|(content, status)| json!({ "content": content, "status": status }))
            .collect();
        json!({ "todos": todos })
    }

    fn write_args_with_priority(items: &[(&str, &str, &str)]) -> Value {
        let todos: Vec<Value> = items
            .iter()
            .map(|(content, status, priority)| {
                json!({ "content": content, "status": status, "priority": priority })
            })
            .collect();
        json!({ "todos": todos })
    }

    // VAL-TOOLS-038: writing 3 items then updating item 1 to completed leaves item 1 completed and
    // items 2-3 with their prior statuses, persisted for the session.
    #[test]
    fn write_then_update_marks_item_one_completed_and_keeps_the_rest() {
        let store = Arc::new(MemoryTodoStore::default());
        let ctx = ctx_with_store("ses_a", store.clone());

        let first = run(
            &write_args(&[
                ("design the schema", "pending"),
                ("write the parser", "pending"),
                ("add tests", "pending"),
            ]),
            &ctx,
        );
        assert!(!first.is_error);

        let updated = run(
            &write_args(&[
                ("design the schema", "completed"),
                ("write the parser", "pending"),
                ("add tests", "pending"),
            ]),
            &ctx,
        );
        assert!(!updated.is_error);

        let stored = store.list("ses_a").unwrap();
        assert_eq!(stored.len(), 3);
        assert_eq!(stored[0].status, TodoStatus::Completed);
        assert_eq!(stored[1].status, TodoStatus::Pending);
        assert_eq!(stored[2].status, TodoStatus::Pending);
        assert_eq!(stored[0].priority, TodoPriority::Medium);
        let text = updated.model_content();
        assert!(
            text.contains("1. [completed][medium] design the schema"),
            "got: {text}"
        );
        assert!(
            text.contains("2. [pending][medium] write the parser"),
            "got: {text}"
        );
    }

    // VAL-TOOLS-039: a fresh session sees an empty list even when another session has items.
    #[test]
    fn todos_are_isolated_per_session() {
        let store = Arc::new(MemoryTodoStore::default());
        let ctx_a = ctx_with_store("ses_a", store.clone());
        run(&write_args(&[("only in a", "pending")]), &ctx_a);

        let ctx_b = ctx_with_store("ses_b", store.clone());
        let read_b = run(&json!({}), &ctx_b);
        assert!(!read_b.is_error);
        assert_eq!(read_b.model_content(), "(no todos)");
    }

    #[test]
    fn reading_with_no_argument_returns_the_current_list() {
        let store = Arc::new(MemoryTodoStore::default());
        let ctx = ctx_with_store("ses_a", store.clone());
        run(&write_args(&[("task one", "in_progress")]), &ctx);

        let read = run(&json!({}), &ctx);
        assert!(!read.is_error);
        assert_eq!(read.model_content(), "1. [in_progress][medium] task one");
    }

    #[test]
    fn an_invalid_status_is_a_clear_error() {
        let store = Arc::new(MemoryTodoStore::default());
        let ctx = ctx_with_store("ses_a", store);
        let output = run(&write_args(&[("task", "banana")]), &ctx);
        assert!(output.is_error);
        assert!(output.model_content().contains("invalid status"));
    }

    #[test]
    fn cancelled_status_and_priority_round_trip() {
        let store = Arc::new(MemoryTodoStore::default());
        let ctx = ctx_with_store("ses_a", store.clone());
        let output = run(
            &write_args_with_priority(&[("defer this", "cancelled", "high")]),
            &ctx,
        );
        assert!(!output.is_error, "got: {}", output.model_content());
        assert_eq!(output.model_content(), "1. [cancelled][high] defer this");
        let stored = store.list("ses_a").expect("list");
        assert_eq!(stored[0].status, TodoStatus::Cancelled);
        assert_eq!(stored[0].priority, TodoPriority::High);
    }

    #[test]
    fn an_invalid_priority_is_a_clear_error() {
        let store = Arc::new(MemoryTodoStore::default());
        let ctx = ctx_with_store("ses_a", store);
        let output = run(
            &json!({ "todos": [{ "content": "task", "status": "pending", "priority": "urgent"}]}),
            &ctx,
        );
        assert!(output.is_error);
        assert!(output.model_content().contains("invalid priority"));
    }

    #[test]
    fn an_item_without_content_is_a_clear_error() {
        let store = Arc::new(MemoryTodoStore::default());
        let ctx = ctx_with_store("ses_a", store);
        let output = run(&json!({ "todos": [{ "status": "pending" }] }), &ctx);
        assert!(output.is_error);
        assert!(output.model_content().contains("content"));
    }

    #[test]
    fn no_store_configured_is_a_clear_error() {
        let ctx = ToolContext::new(std::env::temp_dir()).with_session("ses_a");
        let output = run(&json!({}), &ctx);
        assert!(output.is_error);
        assert!(output.model_content().contains("todo store"));
    }

    #[test]
    fn permission_deny_blocks_todo_reads_and_writes() {
        let store = Arc::new(MemoryTodoStore::default());
        let engine = Arc::new(PermissionEngine::new(Ruleset::new(vec![Rule::new(
            "todo",
            "*",
            Action::Deny,
        )])));
        let ctx = ToolContext::new(std::env::temp_dir())
            .with_session("ses_a")
            .with_todo_store(store.clone())
            .with_permissions(engine);
        let write = run(&write_args(&[("task one", "pending")]), &ctx);
        assert!(write.is_error);
        assert_eq!(store.list("ses_a").expect("list").len(), 0);
        let read = run(&json!({}), &ctx);
        assert!(read.is_error);
        assert!(
            read.model_content()
                .to_ascii_lowercase()
                .contains("permission")
        );
    }

    #[test]
    fn no_session_is_a_clear_error() {
        let store = Arc::new(MemoryTodoStore::default());
        let ctx = ToolContext::new(std::env::temp_dir()).with_todo_store(store);
        let output = run(&json!({}), &ctx);
        assert!(output.is_error);
        assert!(output.model_content().contains("session"));
    }
}
