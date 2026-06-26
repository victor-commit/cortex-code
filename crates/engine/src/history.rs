//! Projection of stored conversation history into canonical [`llm::Message`]s.
//!
//! The runner loads a session's prior turns from persistence as role/content records and projects
//! them into the canonical [`llm::Message`] vocabulary the provider request is built from
//! (architecture §4, "history projection → canonical LLM messages"). The projection is decoupled
//! from the persistence layer: callers pass simple [`HistoryMessage`] records (e.g. mapped from
//! `db::Message`), keeping `engine` independent of the storage schema.

use std::collections::BTreeSet;
use std::path::Path;

use llm::{Message, MessageContentPart, Role};
use protocol::{MessagePart, MessagePartBody, ToolResultPart};

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

/// Extract provider-facing message content parts from persisted rich message parts.
///
/// This maps persisted reasoning metadata/signatures and file/media attachments back to
/// [`MessageContentPart`] so resumed turns can replay provider-native continuation context.
#[must_use]
pub fn extract_message_content_parts(parts: &[MessagePart]) -> Vec<MessageContentPart> {
    let mut seen = BTreeSet::new();
    let mut media = Vec::new();
    for part in parts {
        match &part.body {
            MessagePartBody::File {
                path, media_type, ..
            } => {
                if let Some(media_part) = message_media_part_for_file(path, media_type.as_deref())
                    .or_else(|| {
                        media_type_for_path(Path::new(path))
                            .and_then(|kind| message_media_part_for_file(path, Some(kind)))
                    })
                {
                    push_unique_media_part(&mut media, &mut seen, media_part);
                }
            }
            MessagePartBody::Reasoning { text, metadata } => {
                if let Some(reasoning_part) = reasoning_content_part(text, metadata.as_ref()) {
                    push_unique_media_part(&mut media, &mut seen, reasoning_part);
                }
            }
            MessagePartBody::Tool { attachments, .. } => {
                for attachment in attachments {
                    let media_part = match attachment {
                        ToolResultPart::Image { media_type, path } => {
                            Some(MessageContentPart::Image {
                                media_type: media_type.clone(),
                                path: path.clone(),
                            })
                        }
                        ToolResultPart::Document { media_type, path } => {
                            Some(MessageContentPart::Document {
                                media_type: media_type.clone(),
                                path: path.clone(),
                            })
                        }
                        ToolResultPart::Text { .. } => None,
                    };
                    if let Some(media_part) = media_part {
                        push_unique_media_part(&mut media, &mut seen, media_part);
                    }
                }
            }
            _ => {}
        }
    }
    media
}

fn media_type_for_path(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?.trim().to_ascii_lowercase();
    match extension.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "bmp" => Some("image/bmp"),
        "svg" => Some("image/svg+xml"),
        "pdf" => Some("application/pdf"),
        _ => None,
    }
}

fn reasoning_content_part(
    text: &str,
    metadata: Option<&serde_json::Value>,
) -> Option<MessageContentPart> {
    let metadata = metadata?;
    let object = metadata.as_object()?;
    let provider = object.get("provider").and_then(serde_json::Value::as_str)?;
    let signature = object
        .get("signature")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned);
    Some(MessageContentPart::Reasoning {
        provider: provider.to_owned(),
        text: text.to_owned(),
        signature,
    })
}

fn message_media_part_for_file(path: &str, media_type: Option<&str>) -> Option<MessageContentPart> {
    let kind = media_type?;
    if kind.starts_with("image/") {
        return Some(MessageContentPart::Image {
            media_type: kind.to_owned(),
            path: path.to_owned(),
        });
    }
    if kind == "application/pdf" {
        return Some(MessageContentPart::Document {
            media_type: kind.to_owned(),
            path: path.to_owned(),
        });
    }
    Some(MessageContentPart::File {
        path: path.to_owned(),
        media_type: Some(kind.to_owned()),
    })
}

fn push_unique_media_part(
    media: &mut Vec<MessageContentPart>,
    seen: &mut BTreeSet<String>,
    part: MessageContentPart,
) {
    let key = match &part {
        MessageContentPart::Image { media_type, path } => {
            format!("image|{media_type}|{path}")
        }
        MessageContentPart::Document { media_type, path } => {
            format!("document|{media_type}|{path}")
        }
        MessageContentPart::File { path, media_type } => {
            format!("file|{}|{path}", media_type.as_deref().unwrap_or(""))
        }
        MessageContentPart::Reasoning {
            provider,
            text,
            signature,
        } => {
            format!(
                "reasoning|{provider}|{text}|{}",
                signature.as_deref().unwrap_or_default()
            )
        }
    };
    if seen.insert(key) {
        media.push(part);
    }
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

    #[test]
    fn extracts_reasoning_parts_with_signature() {
        let parts = vec![MessagePart {
            id: "part_1".to_owned(),
            created_at: None,
            body: MessagePartBody::Reasoning {
                text: "hidden thinking".to_owned(),
                metadata: Some(serde_json::json!({
                    "provider": "anthropic",
                    "signature": "sig-123"
                })),
            },
        }];
        let extracted = extract_message_content_parts(&parts);
        assert_eq!(
            extracted,
            vec![MessageContentPart::Reasoning {
                provider: "anthropic".to_owned(),
                text: "hidden thinking".to_owned(),
                signature: Some("sig-123".to_owned()),
            }]
        );
    }
}
