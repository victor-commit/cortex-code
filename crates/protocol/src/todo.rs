//! The per-session todo DTOs shared between the `todo` tool, the `db` persistence layer, and the
//! surfaces.
//!
//! A todo list is an ordered sequence of [`TodoItem`]s, each carrying its text ([`TodoItem::content`])
//! and a [`TodoStatus`]. This type lives in `protocol` so the `tools` capability (which writes the
//! list), the `db` repository (which persists it per session), and any surface can name it without
//! depending on each other.

use serde::{Deserialize, Serialize};

/// The lifecycle status of a single todo item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    /// Not started yet.
    Pending,
    /// Actively being worked on.
    InProgress,
    /// Finished.
    Completed,
    /// Intentionally canceled and no longer actionable.
    Cancelled,
}

impl TodoStatus {
    /// The canonical wire token for this status (`pending` / `in_progress` / `completed` /
    /// `cancelled`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            TodoStatus::Pending => "pending",
            TodoStatus::InProgress => "in_progress",
            TodoStatus::Completed => "completed",
            TodoStatus::Cancelled => "cancelled",
        }
    }

    /// Parse a status token leniently: case-insensitive, accepting both the `in_progress` and
    /// `in-progress`/`in progress` spellings the model may emit. Returns `None` for an unrecognized
    /// token so the caller can decide a default rather than guessing silently.
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        match token
            .trim()
            .to_ascii_lowercase()
            .replace([' ', '-'], "_")
            .as_str()
        {
            "pending" => Some(TodoStatus::Pending),
            "in_progress" | "inprogress" => Some(TodoStatus::InProgress),
            "completed" | "complete" | "done" => Some(TodoStatus::Completed),
            "cancelled" | "canceled" | "cancel" => Some(TodoStatus::Cancelled),
            _ => None,
        }
    }
}

/// Priority level of a todo item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TodoPriority {
    /// Urgent item.
    High,
    /// Default priority.
    #[default]
    Medium,
    /// Nice-to-have item.
    Low,
}

impl TodoPriority {
    /// The canonical wire token for this priority (`high` / `medium` / `low`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            TodoPriority::High => "high",
            TodoPriority::Medium => "medium",
            TodoPriority::Low => "low",
        }
    }

    /// Parse a priority token leniently, returning `None` for unknown values.
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        match token.trim().to_ascii_lowercase().as_str() {
            "high" => Some(TodoPriority::High),
            "medium" | "med" | "normal" => Some(TodoPriority::Medium),
            "low" => Some(TodoPriority::Low),
            _ => None,
        }
    }
}

/// One item in a session's todo list: its text plus its lifecycle status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoItem {
    /// The task text.
    pub content: String,
    /// The lifecycle status of the task.
    pub status: TodoStatus,
    /// Priority level.
    #[serde(default)]
    pub priority: TodoPriority,
}

impl TodoItem {
    /// Build a todo item with `content` and `status` using the default priority (`medium`).
    #[must_use]
    pub fn new(content: impl Into<String>, status: TodoStatus) -> Self {
        Self {
            content: content.into(),
            status,
            priority: TodoPriority::default(),
        }
    }

    /// Build a todo item with explicit priority.
    #[must_use]
    pub fn new_with_priority(
        content: impl Into<String>,
        status: TodoStatus,
        priority: TodoPriority,
    ) -> Self {
        Self {
            content: content.into(),
            status,
            priority,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_round_trips_through_its_wire_token() {
        for status in [
            TodoStatus::Pending,
            TodoStatus::InProgress,
            TodoStatus::Completed,
            TodoStatus::Cancelled,
        ] {
            assert_eq!(TodoStatus::parse(status.as_str()), Some(status));
        }
    }

    #[test]
    fn status_parse_is_lenient() {
        assert_eq!(TodoStatus::parse("PENDING"), Some(TodoStatus::Pending));
        assert_eq!(
            TodoStatus::parse("in-progress"),
            Some(TodoStatus::InProgress)
        );
        assert_eq!(
            TodoStatus::parse("in progress"),
            Some(TodoStatus::InProgress)
        );
        assert_eq!(
            TodoStatus::parse(" Completed "),
            Some(TodoStatus::Completed)
        );
        assert_eq!(TodoStatus::parse("done"), Some(TodoStatus::Completed));
        assert_eq!(TodoStatus::parse("canceled"), Some(TodoStatus::Cancelled));
        assert_eq!(TodoStatus::parse("bogus"), None);
    }

    #[test]
    fn status_serializes_to_snake_case() {
        let value = serde_json::to_value(TodoStatus::InProgress).expect("serialize");
        assert_eq!(value, serde_json::json!("in_progress"));
    }

    #[test]
    fn priority_round_trips_through_its_wire_token() {
        for priority in [TodoPriority::High, TodoPriority::Medium, TodoPriority::Low] {
            assert_eq!(TodoPriority::parse(priority.as_str()), Some(priority));
        }
    }

    #[test]
    fn priority_serializes_to_snake_case() {
        let value = serde_json::to_value(TodoPriority::Medium).expect("serialize");
        assert_eq!(value, serde_json::json!("medium"));
    }

    #[test]
    fn item_round_trips_through_json() {
        let item = TodoItem::new_with_priority(
            "write the parser",
            TodoStatus::Completed,
            TodoPriority::High,
        );
        let json = serde_json::to_string(&item).expect("serialize");
        let back: TodoItem = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(item, back);
    }

    #[test]
    fn item_without_priority_defaults_to_medium() {
        let value = serde_json::json!({
            "content": "keep compatibility",
            "status": "pending"
        });
        let item: TodoItem = serde_json::from_value(value).expect("deserialize");
        assert_eq!(item.priority, TodoPriority::Medium);
    }
}
