//! Persisted rich message-part model shared across storage and surfaces.
//!
//! Message parts preserve structured turn details (text, reasoning, tool lifecycle, retries,
//! compaction metadata, usage, and related artifacts) without flattening everything into one text
//! blob.

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::ToolResultPart;

/// One persisted message part with a stable id and optional persistence timestamp.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessagePart {
    /// Stable part identifier. Persistence fills this when omitted.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    /// Persistence timestamp (UTC `YYYY-MM-DD HH:MM:SS`) when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    /// The discriminated payload.
    #[serde(flatten)]
    pub body: MessagePartBody,
}

impl MessagePart {
    /// Build a text part.
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            id: String::new(),
            created_at: None,
            body: MessagePartBody::Text { text: text.into() },
        }
    }

    /// Build a reasoning part.
    #[must_use]
    pub fn reasoning(text: impl Into<String>) -> Self {
        Self {
            id: String::new(),
            created_at: None,
            body: MessagePartBody::Reasoning { text: text.into() },
        }
    }
}

/// Tool lifecycle states preserved in rich parts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolPartState {
    Pending,
    Running,
    Completed,
    Error,
}

/// Step lifecycle phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepPartPhase {
    Start,
    Finish,
}

/// Discriminated rich message-part payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessagePartBody {
    Text {
        text: String,
    },
    Reasoning {
        text: String,
    },
    Tool {
        call_id: String,
        name: String,
        state: ToolPartState,
        #[serde(skip_serializing_if = "Option::is_none")]
        input: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        output: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        attachments: Vec<ToolResultPart>,
        #[serde(default, skip_serializing_if = "message_part_json_is_empty_or_null")]
        metadata: JsonValue,
    },
    Step {
        phase: StepPartPhase,
        step: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        finish_reason: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        input_tokens: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        output_tokens: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        total_tokens: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cost: Option<f64>,
    },
    Patch {
        #[serde(skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        diff: String,
    },
    File {
        path: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        media_type: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        source: Option<String>,
    },
    Agent {
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        mode: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
    },
    Subtask {
        title: String,
        state: String,
    },
    Compaction {
        summary: String,
        tail_start_id: String,
        continued_request: bool,
    },
    Snapshot {
        label: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        created: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        updated: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        deleted: Vec<String>,
    },
    Retry {
        attempt: usize,
        max_attempts: usize,
        delay_ms: u64,
        error_kind: String,
        classification: String,
        message: String,
    },
    Usage {
        input_tokens: u32,
        output_tokens: u32,
        total_tokens: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        cost: Option<f64>,
    },
}

fn message_part_json_is_empty_or_null(value: &JsonValue) -> bool {
    value.is_null() || matches!(value, JsonValue::Object(map) if map.is_empty())
}
