//! Deterministic named interaction scenarios for typed audit evidence.

use std::collections::BTreeMap;

use crate::audit::{
    action_json, audit_screen, field, quote, sha256_hex, transition_json, ActionEvidence,
    AuditDocument, TransitionTrace,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScenarioState {
    pub screen_id: String,
    pub origin_stack: Vec<String>,
    pub draft: String,
    pub transcript: String,
    pub queued_follow_up: String,
    pub scroll_anchor: String,
    pub selection: String,
    pub focus: String,
    pub pending_action: Option<String>,
    pub return_policy: String,
    pub virtual_tick: u32,
    pub state_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreservedFieldHash {
    pub before: String,
    pub after: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttemptEvidence {
    pub attempt_id: String,
    pub owner_id: String,
    pub count: u32,
    pub outcome: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SideEffectEvidence {
    pub effect_id: String,
    pub kind: String,
    pub status: String,
    pub simulated: bool,
    pub effect_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaseReplay {
    pub case_id: String,
    pub requested_tick: u32,
    pub virtual_tick: u32,
    pub before: ScenarioState,
    pub after: ScenarioState,
    pub actions: Vec<ActionEvidence>,
    pub transitions: Vec<TransitionTrace>,
    pub preserved_field_hashes: BTreeMap<String, PreservedFieldHash>,
    pub attempts: Vec<AttemptEvidence>,
    pub side_effects: Vec<SideEffectEvidence>,
    pub audit: AuditDocument,
}

impl CaseReplay {
    pub fn validate(&self) -> Result<(), String> {
        if self.case_id.trim().is_empty()
            || self.requested_tick != self.virtual_tick
            || self.virtual_tick == 0
        {
            return Err("case replay must name one nonzero virtual tick".to_owned());
        }
        validate_state(&self.before)?;
        validate_state(&self.after)?;
        if self.before.virtual_tick + 1 != self.after.virtual_tick
            || self.after.virtual_tick != self.virtual_tick
        {
            return Err(
                "case replay must move through exactly one deterministic virtual tick".to_owned(),
            );
        }
        if self.before.origin_stack != self.after.origin_stack {
            return Err("scenario transition did not preserve its origin stack".to_owned());
        }
        if self.actions.len() != 1 || self.transitions.len() != 1 {
            return Err(
                "each foundation scenario tick must emit one action and one transition".to_owned(),
            );
        }
        let action = &self.actions[0];
        if action.action_id.trim().is_empty() || !action.enabled {
            return Err("case replay emitted an inaccessible action".to_owned());
        }
        let transition = &self.transitions[0];
        if transition.action_id != action.action_id
            || transition.from_screen_id != self.before.screen_id
            || transition.to_screen_id != self.after.screen_id
        {
            return Err(
                "case replay transition does not bind the exact action and states".to_owned(),
            );
        }
        if self.preserved_field_hashes.is_empty()
            || self
                .preserved_field_hashes
                .values()
                .any(|hash| hash.before.len() != 64 || hash.after.len() != 64)
        {
            return Err("case replay needs complete SHA-256 preservation evidence".to_owned());
        }
        if self.attempts.len() != 1
            || self.attempts[0].count != 1
            || self.attempts[0].attempt_id.trim().is_empty()
            || self.attempts[0].owner_id.trim().is_empty()
        {
            return Err("case replay requires exactly one bounded attempt".to_owned());
        }
        if self.side_effects.len() != 1
            || !self.side_effects[0].simulated
            || self.side_effects[0].effect_hash.len() != 64
        {
            return Err("case replay requires one hash-bound simulated side effect".to_owned());
        }
        self.audit.validate()?;
        if self.audit.screen_id != self.after.screen_id {
            return Err("case audit must describe the resulting rendered state".to_owned());
        }
        Ok(())
    }

    pub fn to_json(&self) -> String {
        let mut output = String::new();
        output.push_str("{\"document_type\":\"case-replay\",\"schema_version\":1");
        field(&mut output, "case_id", &self.case_id);
        output.push_str(",\"requested_tick\":");
        output.push_str(&self.requested_tick.to_string());
        output.push_str(",\"virtual_tick\":");
        output.push_str(&self.virtual_tick.to_string());
        output.push_str(",\"before\":");
        state_json(&mut output, &self.before);
        output.push_str(",\"after\":");
        state_json(&mut output, &self.after);
        output.push_str(",\"actions\":[");
        for (index, action) in self.actions.iter().enumerate() {
            if index > 0 {
                output.push(',');
            }
            action_json(&mut output, action);
        }
        output.push_str("],\"transitions\":[");
        for (index, transition) in self.transitions.iter().enumerate() {
            if index > 0 {
                output.push(',');
            }
            transition_json(&mut output, transition);
        }
        output.push_str("],\"preserved_field_hashes\":{");
        for (index, (field_name, hash)) in self.preserved_field_hashes.iter().enumerate() {
            if index > 0 {
                output.push(',');
            }
            quote(&mut output, field_name);
            output.push_str(":{\"before\":");
            quote(&mut output, &hash.before);
            output.push_str(",\"after\":");
            quote(&mut output, &hash.after);
            output.push('}');
        }
        output.push_str("},\"attempts\":[");
        for (index, attempt) in self.attempts.iter().enumerate() {
            if index > 0 {
                output.push(',');
            }
            output.push('{');
            raw_field(&mut output, "attempt_id", &attempt.attempt_id);
            field(&mut output, "owner_id", &attempt.owner_id);
            output.push_str(",\"count\":");
            output.push_str(&attempt.count.to_string());
            field(&mut output, "outcome", &attempt.outcome);
            output.push('}');
        }
        output.push_str("],\"side_effects\":[");
        for (index, effect) in self.side_effects.iter().enumerate() {
            if index > 0 {
                output.push(',');
            }
            output.push('{');
            raw_field(&mut output, "effect_id", &effect.effect_id);
            field(&mut output, "kind", &effect.kind);
            field(&mut output, "status", &effect.status);
            output.push_str(",\"simulated\":");
            output.push_str(if effect.simulated { "true" } else { "false" });
            field(&mut output, "effect_hash", &effect.effect_hash);
            output.push('}');
        }
        output.push_str("],\"audit\":");
        output.push_str(&self.audit.to_json());
        output.push('}');
        output
    }
}

/// Replays a fixed named case through one virtual transition without any I/O.
pub fn replay_case(case_id: &str, tick: u32) -> Result<CaseReplay, String> {
    if tick != 1 {
        return Err(format!("unknown virtual tick {tick} for case {case_id}"));
    }
    let (before, after, action, transition, attempt, effect) = match case_id {
        "permission-allow-once" => permission_allow_once(),
        "permission-cancel" => permission_cancel(),
        "automatic-retry" => automatic_retry(),
        "streaming-interrupt" => streaming_interrupt(),
        "tool-running-interrupt" => tool_running_interrupt(),
        "tool-running-complete" => tool_running_complete(),
        _ => return Err(format!("unknown named case: {case_id}")),
    };
    let preserved_field_hashes = preserved_hashes(&before, &after);
    let audit = audit_screen(&after.screen_id, &format!("case.{case_id}.tick.{tick}"))?;
    let replay = CaseReplay {
        case_id: case_id.to_owned(),
        requested_tick: tick,
        virtual_tick: tick,
        before,
        after,
        actions: vec![action],
        transitions: vec![transition],
        preserved_field_hashes,
        attempts: vec![attempt],
        side_effects: vec![effect],
        audit,
    };
    replay.validate()?;
    Ok(replay)
}

fn permission_allow_once() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let before = state(
        "permission-shell",
        vec!["chat-turn", "permission-shell"],
        "Check the project files\nthen summarize the result.",
        "turn-18",
        "allow-once",
        "permission-choice",
        Some("pending.shell.project-check"),
        "return-to-paused-turn",
        0,
    );
    let after = state(
        "diff-edit",
        vec!["chat-turn", "permission-shell"],
        "Check the project files\nthen summarize the result.",
        "turn-18",
        "allow-once",
        "pending-edit",
        None,
        "return-to-paused-turn",
        1,
    );
    let action = ActionEvidence {
        action_id: "action.permission.allow-once".to_owned(),
        label: "Allow once".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "approval opens a read-only pending edit".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.permission.allow-once.1".to_owned(),
        owner_id: "pending.shell.project-check".to_owned(),
        count: 1,
        outcome: "approved".to_owned(),
    };
    let effect = simulated_effect(
        "effect.shell.pending",
        "permission-decision",
        "approved-without-execution",
        "permission allow-once opened the pending edit",
    );
    (before, after, action, transition, attempt, effect)
}

fn permission_cancel() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let before = state(
        "permission-shell",
        vec!["chat-turn", "permission-shell"],
        "Check the project files\nthen summarize the result.",
        "turn-18",
        "cancel",
        "permission-choice",
        Some("pending.shell.project-check"),
        "return-to-paused-turn",
        0,
    );
    let after = state(
        "chat-turn",
        vec!["chat-turn", "permission-shell"],
        "Check the project files\nthen summarize the result.",
        "turn-18",
        "composer",
        "composer",
        None,
        "return-to-paused-turn",
        1,
    );
    let action = ActionEvidence {
        action_id: "action.permission.cancel".to_owned(),
        label: "Cancel".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "cancel".to_owned(),
        path: "cancellation restores the exact paused conversation".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.permission.cancel.1".to_owned(),
        owner_id: "pending.shell.project-check".to_owned(),
        count: 1,
        outcome: "cancelled".to_owned(),
    };
    let effect = simulated_effect(
        "effect.shell.none",
        "permission-decision",
        "cancelled-without-execution",
        "permission cancellation performed no execution",
    );
    (before, after, action, transition, attempt, effect)
}

fn automatic_retry() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let before = state(
        "rate-limit",
        vec!["chat-turn", "error-network", "rate-limit"],
        "Finish the release notes.",
        "turn-22",
        "automatic-retry",
        "recovery-wait",
        Some("pending.provider.retry"),
        "return-to-paused-turn",
        0,
    );
    let after = state(
        "chat-turn",
        vec!["chat-turn", "error-network", "rate-limit"],
        "Finish the release notes.",
        "turn-22",
        "automatic-retry",
        "composer",
        None,
        "return-to-paused-turn",
        1,
    );
    let action = ActionEvidence {
        action_id: "action.recovery.retry-window".to_owned(),
        label: "Retry window completed".to_owned(),
        enabled: true,
        automatic: true,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "automatic".to_owned(),
        path: "one bounded retry restored the paused conversation".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.provider.retry.1".to_owned(),
        owner_id: "pending.provider.retry".to_owned(),
        count: 1,
        outcome: "restored".to_owned(),
    };
    let effect = simulated_effect(
        "effect.provider.retry",
        "provider-attempt",
        "simulated-success",
        "one bounded provider retry restored the conversation",
    );
    (before, after, action, transition, attempt, effect)
}

fn streaming_interrupt() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "streaming",
        vec!["home", "chat-turn", "streaming"],
        "Also list the tests that should cover the change.",
        "assistant-partial-auth-refactor",
        "stop-reply",
        "streaming-interrupt",
        Some("active.reply.auth-refactor"),
        "return-to-paused-turn",
        0,
    );
    before.transcript = "The refactor moves token checks into one shared session guard.".to_owned();
    before.queued_follow_up =
        "Also list the tests that should cover the change. [attachment: auth-notes.md]"
            .to_owned();
    before.state_hash = state_hash(&before);

    let mut after = state(
        "chat-turn",
        vec!["home", "chat-turn", "streaming"],
        "Also list the tests that should cover the change.",
        "assistant-partial-auth-refactor",
        "composer",
        "composer",
        None,
        "return-to-paused-turn",
        1,
    );
    after.transcript = before.transcript.clone();
    after.queued_follow_up = before.queued_follow_up.clone();
    after.state_hash = state_hash(&after);

    let action = ActionEvidence {
        action_id: "action.streaming.interrupt".to_owned(),
        label: "Stop reply".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "local interruption saves the partial reply and restores the queued follow-up"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.streaming.interrupt.1".to_owned(),
        owner_id: "active.reply.auth-refactor".to_owned(),
        count: 1,
        outcome: "interrupted".to_owned(),
    };
    let effect = simulated_effect(
        "effect.streaming.reply",
        "generation",
        "generation-stopped",
        "the active reply stopped once without altering the saved transcript or queued follow-up",
    );
    (before, after, action, transition, attempt, effect)
}

fn tool_running_interrupt() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "tool-running",
        vec!["home", "chat-turn", "streaming", "tool-running"],
        "Also list the tests that should cover the search.",
        "tool-search-rate-limit",
        "stop-search",
        "tool-running-interrupt",
        Some("activity.tool.search"),
        "return-to-same-turn",
        0,
    );
    before.transcript =
        "Checked session rules completed. Search project code is running.".to_owned();
    before.queued_follow_up =
        "Also list the tests that should cover the search. [attachment: rate-limit-notes.md]"
            .to_owned();
    before.state_hash = state_hash(&before);

    let mut after = state(
        "chat-turn",
        vec!["home", "chat-turn", "streaming", "tool-running"],
        "Also list the tests that should cover the search.",
        "tool-search-rate-limit",
        "composer",
        "composer",
        None,
        "return-to-same-turn",
        1,
    );
    after.transcript = before.transcript.clone();
    after.queued_follow_up = before.queued_follow_up.clone();
    after.state_hash = state_hash(&after);

    let action = ActionEvidence {
        action_id: "action.tool-running.interrupt".to_owned(),
        label: "Stop search".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "only the active search stops; the completed check, queued follow-up, and same turn remain"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.tool.search.interrupt.1".to_owned(),
        owner_id: "activity.tool.search".to_owned(),
        count: 1,
        outcome: "interrupted".to_owned(),
    };
    let effect = simulated_effect(
        "effect.tool.search",
        "tool-execution",
        "search-stopped",
        "the selected search stopped once without changing the completed session check",
    );
    (before, after, action, transition, attempt, effect)
}

fn tool_running_complete() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "tool-running",
        vec!["home", "chat-turn", "streaming", "tool-running"],
        "Also list the tests that should cover the search.",
        "tool-search-rate-limit",
        "search-completion",
        "tool-running",
        Some("activity.tool.search"),
        "return-to-same-turn",
        0,
    );
    before.transcript =
        "Checked session rules completed. Search project code is running.".to_owned();
    before.queued_follow_up =
        "Also list the tests that should cover the search. [attachment: rate-limit-notes.md]"
            .to_owned();
    before.state_hash = state_hash(&before);

    let mut after = state(
        "tool-success",
        vec!["home", "chat-turn", "streaming", "tool-running"],
        "Also list the tests that should cover the search.",
        "tool-search-rate-limit",
        "search-completion",
        "composer",
        None,
        "return-to-same-turn",
        1,
    );
    after.transcript = "Checked session rules completed. Search project code completed with 3 matches in 400 files."
        .to_owned();
    after.queued_follow_up = before.queued_follow_up.clone();
    after.state_hash = state_hash(&after);

    let action = ActionEvidence {
        action_id: "action.tool-running.complete".to_owned(),
        label: "Search completed".to_owned(),
        enabled: true,
        automatic: true,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "automatic".to_owned(),
        path: "the active search resolves once by replacing its live row with one compact terminal result"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.tool.search.complete.1".to_owned(),
        owner_id: "activity.tool.search".to_owned(),
        count: 1,
        outcome: "succeeded".to_owned(),
    };
    let effect = simulated_effect(
        "effect.tool.search",
        "tool-execution",
        "search-completed",
        "one search completion produced one terminal summary with three matches",
    );
    (before, after, action, transition, attempt, effect)
}

#[allow(clippy::too_many_arguments)]
fn state(
    screen_id: &str,
    origin_stack: Vec<&str>,
    draft: &str,
    scroll_anchor: &str,
    selection: &str,
    focus: &str,
    pending_action: Option<&str>,
    return_policy: &str,
    virtual_tick: u32,
) -> ScenarioState {
    let mut state = ScenarioState {
        screen_id: screen_id.to_owned(),
        origin_stack: origin_stack.into_iter().map(str::to_owned).collect(),
        draft: draft.to_owned(),
        transcript: "The current conversation remains available.".to_owned(),
        queued_follow_up: String::new(),
        scroll_anchor: scroll_anchor.to_owned(),
        selection: selection.to_owned(),
        focus: focus.to_owned(),
        pending_action: pending_action.map(str::to_owned),
        return_policy: return_policy.to_owned(),
        virtual_tick,
        state_hash: String::new(),
    };
    state.state_hash = state_hash(&state);
    state
}

fn validate_state(state: &ScenarioState) -> Result<(), String> {
    if state.screen_id.trim().is_empty()
        || state.origin_stack.is_empty()
        || state.draft.is_empty()
        || state.scroll_anchor.is_empty()
        || state.selection.is_empty()
        || state.focus.is_empty()
        || state.return_policy.is_empty()
        || state.state_hash.len() != 64
    {
        return Err("scenario state requires origin and preserved field evidence".to_owned());
    }
    if state_hash(state) != state.state_hash {
        return Err("scenario state hash does not match its stable fields".to_owned());
    }
    Ok(())
}

fn state_hash(state: &ScenarioState) -> String {
    sha256_hex(
        format!(
            "screen={}\norigins={}\ndraft={}\ntranscript={}\nqueued_follow_up={}\nscroll={}\nselection={}\nfocus={}\npending={}\nreturn={}\ntick={}",
            state.screen_id,
            state.origin_stack.join(">"),
            state.draft,
            state.transcript,
            state.queued_follow_up,
            state.scroll_anchor,
            state.selection,
            state.focus,
            state.pending_action.as_deref().unwrap_or(""),
            state.return_policy,
            state.virtual_tick,
        )
        .as_bytes(),
    )
}

fn preserved_hashes(
    before: &ScenarioState,
    after: &ScenarioState,
) -> BTreeMap<String, PreservedFieldHash> {
    let mut hashes = BTreeMap::new();
    for (name, first, second) in [
        ("draft", before.draft.as_str(), after.draft.as_str()),
        (
            "transcript",
            before.transcript.as_str(),
            after.transcript.as_str(),
        ),
        (
            "queued_follow_up",
            before.queued_follow_up.as_str(),
            after.queued_follow_up.as_str(),
        ),
        (
            "scroll_anchor",
            before.scroll_anchor.as_str(),
            after.scroll_anchor.as_str(),
        ),
        (
            "origin_stack",
            &before.origin_stack.join(">"),
            &after.origin_stack.join(">"),
        ),
        (
            "return_policy",
            before.return_policy.as_str(),
            after.return_policy.as_str(),
        ),
    ] {
        hashes.insert(
            name.to_owned(),
            PreservedFieldHash {
                before: sha256_hex(first.as_bytes()),
                after: sha256_hex(second.as_bytes()),
            },
        );
    }
    hashes
}

fn simulated_effect(effect_id: &str, kind: &str, status: &str, detail: &str) -> SideEffectEvidence {
    SideEffectEvidence {
        effect_id: effect_id.to_owned(),
        kind: kind.to_owned(),
        status: status.to_owned(),
        simulated: true,
        effect_hash: sha256_hex(detail.as_bytes()),
    }
}

fn state_json(output: &mut String, state: &ScenarioState) {
    output.push('{');
    raw_field(output, "screen_id", &state.screen_id);
    output.push_str(",\"origin_stack\":[");
    for (index, origin) in state.origin_stack.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        quote(output, origin);
    }
    output.push(']');
    field(output, "draft", &state.draft);
    field(output, "transcript", &state.transcript);
    field(output, "queued_follow_up", &state.queued_follow_up);
    field(output, "scroll_anchor", &state.scroll_anchor);
    field(output, "selection", &state.selection);
    field(output, "focus", &state.focus);
    output.push_str(",\"pending_action\":");
    match &state.pending_action {
        Some(action) => quote(output, action),
        None => output.push_str("null"),
    }
    field(output, "return_policy", &state.return_policy);
    output.push_str(",\"virtual_tick\":");
    output.push_str(&state.virtual_tick.to_string());
    field(output, "state_hash", &state.state_hash);
    output.push('}');
}

fn raw_field(output: &mut String, name: &str, value: &str) {
    quote(output, name);
    output.push(':');
    quote(output, value);
}

#[cfg(feature = "png")]
pub fn write_scenario_png(
    destination: &std::path::Path,
    replay: &CaseReplay,
) -> Result<String, String> {
    use std::fs::OpenOptions;
    use std::io::Write as _;

    validate_png_destination(destination)?;
    let bytes = render_audit_png(&replay.audit)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|error| format!("could not create temporary scenario PNG: {error}"))?;
    output
        .write_all(&bytes)
        .map_err(|error| format!("could not write temporary scenario PNG: {error}"))?;
    output
        .sync_all()
        .map_err(|error| format!("could not finalize temporary scenario PNG: {error}"))?;
    Ok(sha256_hex(&bytes))
}

/// Rasterizes any validated typed audit with the same deterministic renderer
/// used by public scenario and gallery artifacts.
#[cfg(feature = "png")]
pub fn render_audit_png(audit: &AuditDocument) -> Result<Vec<u8>, String> {
    use image::{DynamicImage, ImageFormat, Rgb as Pixel, RgbImage};

    audit.validate()?;
    let canvas = &audit.styled_cells;
    let palette = crate::chrome::tokens::palette(crate::chrome::tokens::Theme::Dark);
    let canvas_color = palette.color(crate::chrome::tokens::ColorToken::Canvas);
    let mut image = RgbImage::from_pixel(
        crate::chrome::tokens::PNG_WIDTH,
        crate::chrome::tokens::PNG_HEIGHT,
        Pixel([canvas_color.red, canvas_color.green, canvas_color.blue]),
    );
    let (regular, bold) = crate::chrome::tokens::embedded_fonts()?;
    for cell in canvas {
        if cell.glyph == ' ' {
            continue;
        }
        let foreground = palette.color(cell.style.foreground);
        let cell_x = cell.column as u32 * 10;
        let cell_y = cell.row as u32 * 21;
        if crate::chrome::tokens::glyph_rendering(cell.glyph)
            == Some(crate::chrome::tokens::GlyphRendering::SyntheticElbow)
        {
            draw_synthetic_elbow(
                &mut image,
                cell_x,
                cell_y,
                Pixel([foreground.red, foreground.green, foreground.blue]),
            );
            continue;
        }
        let font = if cell.style.bold { &bold } else { &regular };
        let (metrics, bitmap) = font.rasterize(cell.glyph, 16.0);
        let base_x = cell.column as i32 * 10 + (10 - metrics.width as i32) / 2;
        let base_y = cell.row as i32 * 21 + (21 - metrics.height as i32) / 2;
        for y in 0..metrics.height {
            for x in 0..metrics.width {
                let alpha = bitmap[y * metrics.width + x] as u16;
                if alpha == 0 {
                    continue;
                }
                let px = base_x + x as i32;
                let py = base_y + y as i32;
                if px < 0 || py < 0 || px >= image.width() as i32 || py >= image.height() as i32 {
                    continue;
                }
                let background = image.get_pixel(px as u32, py as u32).0;
                let blend = |foreground: u8, background: u8| {
                    ((foreground as u16 * alpha + background as u16 * (255 - alpha)) / 255) as u8
                };
                image.put_pixel(
                    px as u32,
                    py as u32,
                    Pixel([
                        blend(foreground.red, background[0]),
                        blend(foreground.green, background[1]),
                        blend(foreground.blue, background[2]),
                    ]),
                );
            }
        }
    }
    let mut bytes = Vec::new();
    DynamicImage::ImageRgb8(image)
        .write_to(&mut std::io::Cursor::new(&mut bytes), ImageFormat::Png)
        .map_err(|error| format!("could not encode scenario PNG: {error}"))?;
    Ok(bytes)
}

/// Draws the approved U+23BF result elbow without consulting a system font.
/// The fixed two-pixel strokes match the 10-by-21 terminal cell geometry.
#[cfg(feature = "png")]
fn draw_synthetic_elbow(
    image: &mut image::RgbImage,
    cell_x: u32,
    cell_y: u32,
    foreground: image::Rgb<u8>,
) {
    let put = |image: &mut image::RgbImage, x: u32, y: u32| {
        if x < image.width() && y < image.height() {
            image.put_pixel(x, y, foreground);
        }
    };
    let corner_y = cell_y + 13;
    for y in cell_y + 1..=corner_y {
        put(image, cell_x + 4, y);
        put(image, cell_x + 5, y);
    }
    for x in cell_x + 4..cell_x + 10 {
        put(image, x, corner_y);
        put(image, x, corner_y + 1);
    }
}

#[cfg(feature = "png")]
pub fn validate_png_destination(destination: &std::path::Path) -> Result<(), String> {
    if !destination.is_absolute() {
        return Err("scenario PNG needs an explicit absolute temporary path".to_owned());
    }
    if destination
        .extension()
        .and_then(|extension| extension.to_str())
        != Some("png")
    {
        return Err("scenario PNG path must end in .png".to_owned());
    }
    if destination.exists() {
        return Err("scenario PNG destination must be a new temporary path".to_owned());
    }
    let parent = destination
        .parent()
        .ok_or_else(|| "scenario PNG destination needs a parent directory".to_owned())?
        .canonicalize()
        .map_err(|error| format!("scenario PNG parent is unavailable: {error}"))?;
    let resolved = parent.join(
        destination
            .file_name()
            .ok_or_else(|| "scenario PNG destination needs a file name".to_owned())?,
    );
    for root in [
        "/projects/cortex-code/plans/assets/mockups-v3",
        "/projects/cortex-code/plans/assets/mockups-v3-gates",
    ] {
        let root = std::path::Path::new(root)
            .canonicalize()
            .map_err(|error| format!("could not resolve protected scenario root: {error}"))?;
        if resolved.starts_with(root) {
            return Err(
                "scenario PNG destination must be a temporary path outside canonical and gate roots"
                    .to_owned(),
            );
        }
    }
    Ok(())
}
