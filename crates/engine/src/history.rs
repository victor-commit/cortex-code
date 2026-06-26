//! Projection of stored conversation history into canonical [`llm::Message`]s.
//!
//! The runner loads a session's prior turns from persistence as role/content records and projects
//! them into the canonical [`llm::Message`] vocabulary the provider request is built from
//! (architecture §4, "history projection → canonical LLM messages"). The projection is decoupled
//! from the persistence layer: callers pass simple [`HistoryMessage`] records (e.g. mapped from
//! `db::Message`), keeping `engine` independent of the storage schema.

use llm::{Message, MessageContentPart, Role};

/// A stored conversation entry to be projected into a canonical message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryMessage {
    /// The stored role string (e.g. `user`, `assistant`, `system`, `tool`).
    pub role: String,
    /// The message content.
    pub content: String,
    /// Optional non-text media attachments associated with the message.
    pub parts: Vec<MessageContentPart>,
}

impl HistoryMessage {
    /// Construct a history entry from owned parts.
    #[must_use]
    pub fn new(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
            parts: Vec::new(),
        }
    }

    /// Attach typed media parts to this history entry.
    #[must_use]
    pub fn with_parts(mut self, parts: Vec<MessageContentPart>) -> Self {
        self.parts = parts;
        self
    }
}

/// Map a stored role string to a canonical [`Role`].
///
/// Matching is case-insensitive; an unrecognized role projects to [`Role::User`] so unexpected
/// stored data is still presented as conversational input rather than dropped or mis-attributed to
/// the assistant.
#[must_use]
pub fn project_role(role: &str) -> Role {
    match role.trim().to_ascii_lowercase().as_str() {
        "system" => Role::System,
        "assistant" => Role::Assistant,
        "tool" => Role::Tool,
        _ => Role::User,
    }
}

/// Project a slice of stored history entries into canonical [`Message`]s in order.
#[must_use]
pub fn project_history(history: &[HistoryMessage]) -> Vec<Message> {
    history
        .iter()
        .map(|entry| Message {
            role: project_role(&entry.role),
            content: entry.content.clone(),
            parts: entry.parts.clone(),
            tool_call_id: None,
            tool_calls: Vec::new(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_known_roles() {
        assert_eq!(project_role("system"), Role::System);
        assert_eq!(project_role("user"), Role::User);
        assert_eq!(project_role("assistant"), Role::Assistant);
        assert_eq!(project_role("tool"), Role::Tool);
    }

    #[test]
    fn role_matching_is_case_insensitive_and_trimmed() {
        assert_eq!(project_role("  Assistant "), Role::Assistant);
        assert_eq!(project_role("SYSTEM"), Role::System);
    }

    #[test]
    fn unknown_role_projects_to_user() {
        assert_eq!(project_role("moderator"), Role::User);
        assert_eq!(project_role(""), Role::User);
    }

    #[test]
    fn projects_history_in_order_preserving_content() {
        let history = [
            HistoryMessage::new("user", "remember marmalade"),
            HistoryMessage::new("assistant", "noted"),
        ];
        let projected = project_history(&history);
        assert_eq!(projected.len(), 2);
        assert_eq!(projected[0].role, Role::User);
        assert_eq!(projected[0].content, "remember marmalade");
        assert_eq!(projected[1].role, Role::Assistant);
        assert_eq!(projected[1].content, "noted");
    }
}
