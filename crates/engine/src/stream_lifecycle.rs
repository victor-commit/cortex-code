//! Streaming lifecycle events and stable part-id helpers for turn reconstruction.

use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::RunnerEvent;

const PART_LIFECYCLE_TYPE: &str = "part_lifecycle";

/// Streaming lifecycle phase for a message part segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PartLifecyclePhase {
    Start,
    Delta,
    End,
}

/// Streaming lifecycle kind for a message part segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PartLifecycleKind {
    Text,
    Reasoning,
    ToolInput,
    ToolProgress,
    Step,
}

/// One lifecycle event emitted while a turn streams.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PartLifecycleEvent {
    /// Event discriminator for JSONL consumers.
    #[serde(rename = "type")]
    pub event_type: String,
    /// Start, delta, or end.
    pub phase: PartLifecyclePhase,
    /// The part category.
    pub part_kind: PartLifecycleKind,
    /// Stable part id that correlates lifecycle segments and persisted parts.
    pub part_id: String,
    /// UTC timestamp (`YYYY-MM-DD HH:MM:SS`) when this lifecycle segment was emitted.
    pub timestamp: String,
    /// Optional text payload (for text/reasoning/tool deltas).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Tool call id for tool lifecycle events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    /// Tool name for tool lifecycle events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Step index for step lifecycle events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<usize>,
    /// Additional structured metadata.
    #[serde(default, skip_serializing_if = "json_is_empty_or_null")]
    pub metadata: JsonValue,
}

impl PartLifecycleEvent {
    fn new(
        phase: PartLifecyclePhase,
        part_kind: PartLifecycleKind,
        part_id: String,
        timestamp: String,
    ) -> Self {
        Self {
            event_type: PART_LIFECYCLE_TYPE.to_owned(),
            phase,
            part_kind,
            part_id,
            timestamp,
            text: None,
            call_id: None,
            name: None,
            step: None,
            metadata: JsonValue::Null,
        }
    }
}

fn json_is_empty_or_null(value: &JsonValue) -> bool {
    value.is_null() || matches!(value, JsonValue::Object(map) if map.is_empty())
}

/// Deterministic lifecycle tracker that turns runner events into start/delta/end part events.
#[derive(Debug, Clone)]
pub struct PartLifecycleTracker {
    turn_id: String,
    model_id: String,
    step_started: bool,
    text_started: bool,
    reasoning_started: bool,
    running_tools: HashSet<String>,
    part_timestamps: HashMap<String, String>,
}

impl PartLifecycleTracker {
    /// Create a tracker for one turn (keyed by the originating user message id).
    #[must_use]
    pub fn new(turn_id: impl Into<String>, model_id: impl Into<String>) -> Self {
        Self {
            turn_id: turn_id.into(),
            model_id: model_id.into(),
            step_started: false,
            text_started: false,
            reasoning_started: false,
            running_tools: HashSet::new(),
            part_timestamps: HashMap::new(),
        }
    }

    /// Observe one runner event and emit any derived lifecycle events.
    pub fn observe(&mut self, event: &RunnerEvent) -> Vec<PartLifecycleEvent> {
        let mut emitted = Vec::new();
        self.ensure_step_started(&mut emitted);

        match event {
            RunnerEvent::TextDelta { text } if !text.is_empty() => {
                let part_id = text_part_id(&self.turn_id);
                if !self.text_started {
                    emitted.push(self.lifecycle_start(PartLifecycleKind::Text, &part_id));
                    self.text_started = true;
                }
                let mut delta = self.lifecycle_delta(PartLifecycleKind::Text, &part_id);
                delta.text = Some(text.clone());
                emitted.push(delta);
            }
            RunnerEvent::Reasoning { text } if !text.is_empty() => {
                let part_id = reasoning_part_id(&self.turn_id);
                if !self.reasoning_started {
                    emitted.push(self.lifecycle_start(PartLifecycleKind::Reasoning, &part_id));
                    self.reasoning_started = true;
                }
                let mut delta = self.lifecycle_delta(PartLifecycleKind::Reasoning, &part_id);
                delta.text = Some(text.clone());
                emitted.push(delta);
            }
            RunnerEvent::ToolCall {
                id,
                name,
                arguments,
            } => {
                let input_id = tool_pending_part_id(&self.turn_id, id);
                let running_id = tool_running_part_id(&self.turn_id, id);

                let mut input_start = self.lifecycle_start(PartLifecycleKind::ToolInput, &input_id);
                input_start.call_id = Some(id.clone());
                input_start.name = Some(name.clone());
                emitted.push(input_start);

                let mut input_delta = self.lifecycle_delta(PartLifecycleKind::ToolInput, &input_id);
                input_delta.call_id = Some(id.clone());
                input_delta.name = Some(name.clone());
                input_delta.text = Some(arguments.clone());
                emitted.push(input_delta);

                let mut input_end = self.lifecycle_end(PartLifecycleKind::ToolInput, &input_id);
                input_end.call_id = Some(id.clone());
                input_end.name = Some(name.clone());
                emitted.push(input_end);

                if self.running_tools.insert(id.clone()) {
                    let mut running_start =
                        self.lifecycle_start(PartLifecycleKind::ToolProgress, &running_id);
                    running_start.call_id = Some(id.clone());
                    running_start.name = Some(name.clone());
                    emitted.push(running_start);
                }
            }
            RunnerEvent::ToolResult {
                id,
                name,
                parts,
                is_error,
            } => {
                let running_id = tool_running_part_id(&self.turn_id, id);
                if self.running_tools.insert(id.clone()) {
                    let mut running_start =
                        self.lifecycle_start(PartLifecycleKind::ToolProgress, &running_id);
                    running_start.call_id = Some(id.clone());
                    running_start.name = Some(name.clone());
                    emitted.push(running_start);
                }

                let text = parts
                    .iter()
                    .filter_map(protocol::ToolResultPart::as_text)
                    .collect::<Vec<_>>()
                    .join("\n");
                let mut delta = self.lifecycle_delta(PartLifecycleKind::ToolProgress, &running_id);
                delta.call_id = Some(id.clone());
                delta.name = Some(name.clone());
                if !text.is_empty() {
                    delta.text = Some(text);
                }
                delta.metadata = serde_json::json!({ "is_error": is_error });
                emitted.push(delta);

                let mut end = self.lifecycle_end(PartLifecycleKind::ToolProgress, &running_id);
                end.call_id = Some(id.clone());
                end.name = Some(name.clone());
                end.metadata = serde_json::json!({ "is_error": is_error });
                emitted.push(end);
                self.running_tools.remove(id);
            }
            RunnerEvent::Done { reason, steps } => {
                self.close_open_parts(&mut emitted, Some((reason, *steps)));
            }
            RunnerEvent::Error { kind, message } => {
                self.close_open_parts(&mut emitted, None);
                let step_id = step_part_id(&self.turn_id);
                let mut delta = self.lifecycle_delta(PartLifecycleKind::Step, &step_id);
                delta.metadata = serde_json::json!({
                    "error_kind": kind,
                    "message": message,
                });
                emitted.push(delta);
            }
            RunnerEvent::Usage(_)
            | RunnerEvent::Retry { .. }
            | RunnerEvent::ToolSetChanged { .. }
            | RunnerEvent::TextDelta { .. }
            | RunnerEvent::Reasoning { .. } => {}
        }

        emitted
    }

    /// A copy of first-seen timestamps keyed by part id.
    #[must_use]
    pub fn part_timestamps(&self) -> HashMap<String, String> {
        self.part_timestamps.clone()
    }

    fn ensure_step_started(&mut self, emitted: &mut Vec<PartLifecycleEvent>) {
        if self.step_started {
            return;
        }
        self.step_started = true;
        let step_id = step_part_id(&self.turn_id);
        let mut start = self.lifecycle_start(PartLifecycleKind::Step, &step_id);
        start.step = Some(1);
        emitted.push(start);

        let mut delta = self.lifecycle_delta(PartLifecycleKind::Step, &step_id);
        delta.step = Some(1);
        delta.metadata = serde_json::json!({ "model": self.model_id });
        emitted.push(delta);
    }

    fn close_open_parts(
        &mut self,
        emitted: &mut Vec<PartLifecycleEvent>,
        terminal_step: Option<(&llm::FinishReason, usize)>,
    ) {
        if self.reasoning_started {
            let reasoning_id = reasoning_part_id(&self.turn_id);
            emitted.push(self.lifecycle_end(PartLifecycleKind::Reasoning, &reasoning_id));
            self.reasoning_started = false;
        }
        if self.text_started {
            let text_id = text_part_id(&self.turn_id);
            emitted.push(self.lifecycle_end(PartLifecycleKind::Text, &text_id));
            self.text_started = false;
        }
        if !self.running_tools.is_empty() {
            let call_ids = self.running_tools.iter().cloned().collect::<Vec<_>>();
            for call_id in call_ids {
                let running_id = tool_running_part_id(&self.turn_id, &call_id);
                let mut end = self.lifecycle_end(PartLifecycleKind::ToolProgress, &running_id);
                end.call_id = Some(call_id.clone());
                end.metadata = serde_json::json!({ "implicit_end": true });
                emitted.push(end);
                self.running_tools.remove(&call_id);
            }
        }
        let step_id = step_part_id(&self.turn_id);
        let mut end = self.lifecycle_end(PartLifecycleKind::Step, &step_id);
        if let Some((reason, steps)) = terminal_step {
            end.step = Some(steps);
            end.metadata = serde_json::json!({
                "finish_reason": serde_json::to_value(reason).ok().and_then(|value| value.as_str().map(ToOwned::to_owned)).unwrap_or_else(|| format!("{reason:?}")),
                "steps": steps,
            });
        }
        emitted.push(end);
    }

    fn lifecycle_start(&mut self, kind: PartLifecycleKind, part_id: &str) -> PartLifecycleEvent {
        self.lifecycle_event(PartLifecyclePhase::Start, kind, part_id)
    }

    fn lifecycle_delta(&mut self, kind: PartLifecycleKind, part_id: &str) -> PartLifecycleEvent {
        self.lifecycle_event(PartLifecyclePhase::Delta, kind, part_id)
    }

    fn lifecycle_end(&mut self, kind: PartLifecycleKind, part_id: &str) -> PartLifecycleEvent {
        self.lifecycle_event(PartLifecyclePhase::End, kind, part_id)
    }

    fn lifecycle_event(
        &mut self,
        phase: PartLifecyclePhase,
        kind: PartLifecycleKind,
        part_id: &str,
    ) -> PartLifecycleEvent {
        let timestamp = utc_timestamp_string(SystemTime::now());
        self.part_timestamps
            .entry(part_id.to_owned())
            .or_insert_with(|| timestamp.clone());
        PartLifecycleEvent::new(phase, kind, part_id.to_owned(), timestamp)
    }
}

/// Deterministic id for the persisted agent part in one turn.
#[must_use]
pub fn agent_part_id(turn_id: &str) -> String {
    format!("part_{}_agent", encode_fragment(turn_id))
}

/// Deterministic id for the text part in one turn.
#[must_use]
pub fn text_part_id(turn_id: &str) -> String {
    format!("part_{}_text", encode_fragment(turn_id))
}

/// Deterministic id for the reasoning part in one turn.
#[must_use]
pub fn reasoning_part_id(turn_id: &str) -> String {
    format!("part_{}_reasoning", encode_fragment(turn_id))
}

/// Deterministic id for the step boundary lifecycle in one turn.
#[must_use]
pub fn step_part_id(turn_id: &str) -> String {
    format!("part_{}_step", encode_fragment(turn_id))
}

/// Deterministic id for the step start part in one turn.
#[must_use]
pub fn step_start_part_id(turn_id: &str) -> String {
    format!("part_{}_step_start", encode_fragment(turn_id))
}

/// Deterministic id for the step finish part in one turn.
#[must_use]
pub fn step_finish_part_id(turn_id: &str, steps: usize) -> String {
    format!("part_{}_step_finish_{steps}", encode_fragment(turn_id))
}

/// Deterministic id for a pending tool-input part.
#[must_use]
pub fn tool_pending_part_id(turn_id: &str, call_id: &str) -> String {
    format!(
        "part_{}_tool_{}_pending",
        encode_fragment(turn_id),
        encode_fragment(call_id)
    )
}

/// Deterministic id for a running tool-progress part.
#[must_use]
pub fn tool_running_part_id(turn_id: &str, call_id: &str) -> String {
    format!(
        "part_{}_tool_{}_running",
        encode_fragment(turn_id),
        encode_fragment(call_id)
    )
}

/// Deterministic id for the terminal tool part (completed or error).
#[must_use]
pub fn tool_terminal_part_id(turn_id: &str, call_id: &str, is_error: bool) -> String {
    let suffix = if is_error { "error" } else { "completed" };
    format!(
        "part_{}_tool_{}_{}",
        encode_fragment(turn_id),
        encode_fragment(call_id),
        suffix
    )
}

/// Deterministic id for a usage part.
#[must_use]
pub fn usage_part_id(turn_id: &str, ordinal: usize) -> String {
    format!("part_{}_usage_{ordinal}", encode_fragment(turn_id))
}

/// Deterministic id for a retry part.
#[must_use]
pub fn retry_part_id(turn_id: &str, ordinal: usize) -> String {
    format!("part_{}_retry_{ordinal}", encode_fragment(turn_id))
}

/// Deterministic id for a compaction part.
#[must_use]
pub fn compaction_part_id(turn_id: &str, ordinal: usize) -> String {
    format!("part_{}_compaction_{ordinal}", encode_fragment(turn_id))
}

/// Deterministic id for an attachment-backed file part.
#[must_use]
pub fn file_part_id(turn_id: &str, call_id: &str, ordinal: usize) -> String {
    format!(
        "part_{}_tool_{}_file_{ordinal}",
        encode_fragment(turn_id),
        encode_fragment(call_id)
    )
}

/// Deterministic id for a synthesized patch-summary part.
#[must_use]
pub fn patch_part_id(turn_id: &str, call_id: &str, ordinal: usize) -> String {
    format!(
        "part_{}_tool_{}_patch_{ordinal}",
        encode_fragment(turn_id),
        encode_fragment(call_id)
    )
}

fn encode_fragment(raw: &str) -> String {
    let mut encoded = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        let ch = byte as char;
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
            encoded.push(ch);
        } else {
            encoded.push('_');
            encoded.push_str(&format!("{byte:02x}"));
        }
    }
    encoded
}

fn utc_timestamp_string(time: SystemTime) -> String {
    let secs = time
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (year, month, day) = civil_from_days(secs.div_euclid(86_400));
    let secs_of_day = secs.rem_euclid(86_400);
    let hour = secs_of_day / 3_600;
    let minute = (secs_of_day % 3_600) / 60;
    let second = secs_of_day % 60;
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}")
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    (year, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm::FinishReason;

    #[test]
    fn tracker_emits_text_reasoning_tool_and_step_lifecycle() {
        let mut tracker = PartLifecycleTracker::new("msg_turn_1", "google/gemini-2.5-flash");
        let mut out = Vec::new();
        out.extend(tracker.observe(&RunnerEvent::Reasoning {
            text: "thinking".to_owned(),
        }));
        out.extend(tracker.observe(&RunnerEvent::ToolCall {
            id: "call_a".to_owned(),
            name: "todo".to_owned(),
            arguments: "{\"todos\":\"1. [pending] x\"}".to_owned(),
        }));
        out.extend(tracker.observe(&RunnerEvent::ToolResult {
            id: "call_a".to_owned(),
            name: "todo".to_owned(),
            parts: vec![protocol::ToolResultPart::text("ok")],
            is_error: false,
        }));
        out.extend(tracker.observe(&RunnerEvent::TextDelta {
            text: "done".to_owned(),
        }));
        out.extend(tracker.observe(&RunnerEvent::Done {
            reason: FinishReason::Stop,
            steps: 2,
        }));

        let kinds: Vec<(PartLifecycleKind, PartLifecyclePhase)> = out
            .iter()
            .map(|event| (event.part_kind, event.phase))
            .collect();
        for required in [
            PartLifecycleKind::Step,
            PartLifecycleKind::Reasoning,
            PartLifecycleKind::Text,
            PartLifecycleKind::ToolInput,
            PartLifecycleKind::ToolProgress,
        ] {
            assert!(
                kinds
                    .iter()
                    .any(|(kind, phase)| *kind == required && *phase == PartLifecyclePhase::Start),
                "missing start for {required:?}: {kinds:?}"
            );
            assert!(
                kinds
                    .iter()
                    .any(|(kind, phase)| *kind == required && *phase == PartLifecyclePhase::Delta),
                "missing delta for {required:?}: {kinds:?}"
            );
            assert!(
                kinds
                    .iter()
                    .any(|(kind, phase)| *kind == required && *phase == PartLifecyclePhase::End),
                "missing end for {required:?}: {kinds:?}"
            );
        }

        assert!(
            out.iter()
                .all(|event| !event.part_id.is_empty() && !event.timestamp.is_empty()),
            "all lifecycle events carry part ids and timestamps"
        );
    }

    #[test]
    fn id_helpers_are_stable_per_turn() {
        let a = tool_pending_part_id("msg_1", "call 1");
        let b = tool_pending_part_id("msg_1", "call 1");
        assert_eq!(a, b);
        assert_ne!(a, tool_pending_part_id("msg_2", "call 1"));
    }
}
