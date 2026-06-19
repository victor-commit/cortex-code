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
}

impl TodoStatus {
    /// The canonical wire token for this status (`pending` / `in_progress` / `completed`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            TodoStatus::Pending => "pending",
            TodoStatus::InProgress => "in_progress",
            TodoStatus::Completed => "completed",
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
}

impl TodoItem {
    /// Build a todo item with `content` and `status`.
    #[must_use]
    pub fn new(content: impl Into<String>, status: TodoStatus) -> Self {
        Self {
            content: content.into(),
            status,
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
        assert_eq!(TodoStatus::parse("bogus"), None);
    }

    #[test]
    fn status_serializes_to_snake_case() {
        let value = serde_json::to_value(TodoStatus::InProgress).expect("serialize");
        assert_eq!(value, serde_json::json!("in_progress"));
    }

    #[test]
    fn item_round_trips_through_json() {
        let item = TodoItem::new("write the parser", TodoStatus::Completed);
        let json = serde_json::to_string(&item).expect("serialize");
        let back: TodoItem = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(item, back);
    }
}
