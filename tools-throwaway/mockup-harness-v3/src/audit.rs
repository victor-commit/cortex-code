//! Typed, deterministic render evidence for the isolated v3 harness.
//!
//! This module deliberately records semantic render facts instead of source
//! locations. Every candidate has exactly one event, every event has visible
//! cell references, and interactive evidence uses stable action identities.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use crate::registry::canonical_inventory;
use crate::{
    chrome::{
        components::DecisionDock,
        layout::{ScreenComposition, StatusLine, SurfaceLayout},
        tokens::{validate_glyphs, ColorToken, TextStyle, CANVAS_COLUMNS, CANVAS_ROWS},
    },
    Surface,
};

pub const CANVAS_CELL_COUNT: usize = CANVAS_COLUMNS as usize * CANVAS_ROWS as usize;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderRef {
    pub row: u16,
    pub start: u16,
    pub end: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StyledCell {
    pub row: u16,
    pub column: u16,
    pub glyph: char,
    pub style: TextStyle,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Identity {
    pub screen_id: String,
    pub fixture_id: String,
    pub candidate_id: String,
    pub event_id: String,
    pub family: String,
    pub subject_id: String,
    pub render_refs: Vec<RenderRef>,
    pub detail_level: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionEvidence {
    pub action_id: String,
    pub label: String,
    pub enabled: bool,
    pub automatic: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticCandidate {
    pub identity: Identity,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticEvent {
    pub identity: Identity,
    pub action: Option<ActionEvidence>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransitionTrace {
    pub action_id: String,
    pub from_screen_id: String,
    pub to_screen_id: String,
    pub kind: String,
    pub path: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuditDocument {
    pub screen_id: String,
    pub fixture_id: String,
    pub styled_cells: Vec<StyledCell>,
    pub candidates: Vec<SemanticCandidate>,
    pub events: Vec<SemanticEvent>,
    pub transitions: Vec<TransitionTrace>,
}

impl AuditDocument {
    pub fn validate(&self) -> Result<(), String> {
        if self.screen_id.trim().is_empty() || self.fixture_id.trim().is_empty() {
            return Err("audit screen and fixture identities must be present".to_owned());
        }
        validate_styled_cells(&self.styled_cells)?;

        let mut candidates_by_id = BTreeMap::new();
        for candidate in &self.candidates {
            validate_identity(&candidate.identity, &self.screen_id, &self.fixture_id)?;
            if candidates_by_id
                .insert(candidate.identity.candidate_id.clone(), &candidate.identity)
                .is_some()
            {
                return Err(format!(
                    "duplicate semantic candidate identity: {}",
                    candidate.identity.candidate_id
                ));
            }
        }
        let candidate_ids = candidates_by_id.keys().cloned().collect::<BTreeSet<_>>();

        let mut event_ids = BTreeSet::new();
        let mut event_candidates = BTreeSet::new();
        let mut action_ids = BTreeSet::new();
        let mut activity_owners = BTreeSet::new();
        for event in &self.events {
            validate_identity(&event.identity, &self.screen_id, &self.fixture_id)?;
            let Some(candidate) = candidates_by_id.get(&event.identity.candidate_id) else {
                return Err(format!(
                    "orphan semantic event: {} references unknown candidate: {}",
                    event.identity.event_id, event.identity.candidate_id
                ));
            };
            if event.identity.event_id != candidate.event_id {
                return Err(format!(
                    "unknown semantic event: {}",
                    event.identity.event_id
                ));
            }
            if !event_ids.insert(event.identity.event_id.clone()) {
                return Err(format!(
                    "duplicate semantic event identity: {}",
                    event.identity.event_id
                ));
            }
            if !event_candidates.insert(event.identity.candidate_id.clone()) {
                return Err(format!(
                    "multiple semantic events consume candidate: {}",
                    event.identity.candidate_id
                ));
            }
            if event.identity.family == "activity"
                && !activity_owners.insert(event.identity.subject_id.clone())
            {
                return Err(format!(
                    "duplicate activity owner: {}",
                    event.identity.subject_id
                ));
            }
            if let Some(action) = &event.action {
                validate_action(action)?;
                if !action_ids.insert(action.action_id.clone()) {
                    return Err(format!("duplicate action identity: {}", action.action_id));
                }
            }
        }
        for candidate_id in candidate_ids {
            if !event_candidates.contains(&candidate_id) {
                return Err(format!(
                    "missing semantic event for candidate: {candidate_id}"
                ));
            }
        }

        let mut transition_actions = BTreeSet::new();
        for transition in &self.transitions {
            if transition.action_id.trim().is_empty()
                || transition.from_screen_id.trim().is_empty()
                || transition.to_screen_id.trim().is_empty()
                || transition.kind.trim().is_empty()
                || transition.path.trim().is_empty()
            {
                return Err("transition traces require complete stable evidence".to_owned());
            }
            if !action_ids.contains(&transition.action_id) {
                return Err(format!(
                    "transition references an unknown action: {}",
                    transition.action_id
                ));
            }
            if transition.from_screen_id != self.screen_id
                || !canonical_inventory()
                    .iter()
                    .any(|screen| screen.id == transition.from_screen_id)
                || !canonical_inventory()
                    .iter()
                    .any(|screen| screen.id == transition.to_screen_id)
            {
                return Err(format!(
                    "unknown transition: {} -> {}",
                    transition.from_screen_id, transition.to_screen_id
                ));
            }
            if transition.kind == "dead-end" {
                return Err(format!(
                    "transition reaches a dead end: {}",
                    transition.action_id
                ));
            }
            if !transition_actions.insert(transition.action_id.clone()) {
                return Err(format!(
                    "multiple transition traces reference action: {}",
                    transition.action_id
                ));
            }
        }
        Ok(())
    }

    pub fn render_txt(&self) -> String {
        let mut text = String::with_capacity(CANVAS_CELL_COUNT + CANVAS_ROWS as usize);
        for row in 0..CANVAS_ROWS {
            for column in 0..CANVAS_COLUMNS {
                let cell =
                    &self.styled_cells[row as usize * CANVAS_COLUMNS as usize + column as usize];
                text.push(cell.glyph);
            }
            text.push('\n');
        }
        text
    }

    pub fn to_json(&self) -> String {
        let mut output = String::new();
        output.push_str("{\"document_type\":\"typed-render-audit\",\"schema_version\":1");
        field(&mut output, "screen_id", &self.screen_id);
        if let Some(screen) = canonical_inventory()
            .iter()
            .find(|screen| screen.id == self.screen_id)
        {
            field(&mut output, "category", screen.category.as_str());
            field(&mut output, "flow", screen.flow.as_str());
        }
        field(&mut output, "fixture_id", &self.fixture_id);
        output.push_str(",\"styled_cells\":[");
        for (index, cell) in self.styled_cells.iter().enumerate() {
            if index > 0 {
                output.push(',');
            }
            output.push_str("{\"row\":");
            output.push_str(&cell.row.to_string());
            output.push_str(",\"column\":");
            output.push_str(&cell.column.to_string());
            field(&mut output, "glyph", &cell.glyph.to_string());
            output.push_str(",\"style\":{");
            raw_field(&mut output, "foreground", color_name(cell.style.foreground));
            output.push_str(",\"background\":");
            match cell.style.background {
                Some(background) => quote(&mut output, color_name(background)),
                None => output.push_str("null"),
            }
            output.push_str(",\"bold\":");
            output.push_str(if cell.style.bold { "true" } else { "false" });
            output.push_str("}}");
        }
        output.push_str("],\"candidates\":[");
        for (index, candidate) in self.candidates.iter().enumerate() {
            if index > 0 {
                output.push(',');
            }
            identity_json(&mut output, &candidate.identity);
        }
        output.push_str("],\"events\":[");
        for (index, event) in self.events.iter().enumerate() {
            if index > 0 {
                output.push(',');
            }
            output.push('{');
            identity_fields_json(&mut output, &event.identity);
            output.push_str(",\"action\":");
            match &event.action {
                Some(action) => action_json(&mut output, action),
                None => output.push_str("null"),
            }
            output.push('}');
        }
        output.push_str("],\"transitions\":[");
        for (index, transition) in self.transitions.iter().enumerate() {
            if index > 0 {
                output.push(',');
            }
            transition_json(&mut output, transition);
        }
        output.push_str("]}");
        output
    }
}

/// Creates stable evidence for a canonical state without writing an artifact.
pub fn audit_screen(screen_id: &str, fixture_id: &str) -> Result<AuditDocument, String> {
    let screen = canonical_inventory()
        .iter()
        .find(|screen| screen.id == screen_id)
        .ok_or_else(|| format!("unknown canonical screen: {screen_id}"))?;
    if fixture_id.trim().is_empty() {
        return Err("fixture identity must not be empty".to_owned());
    }

    let (action, transition) = screen_action(screen_id);
    let statusline = StatusLine::new("~/project", "main", "Claude Opus 4.8")?;
    ScreenComposition::new(screen_id, screen.surface, statusline.clone()).validate()?;
    let action_row = if screen.surface == Surface::ActionDock {
        SurfaceLayout::for_surface(Surface::ActionDock)
            .action_dock()
            .expect("the action dock surface has one dock region")
            .y
    } else {
        8
    };
    let mut canvas = Canvas::new();
    canvas.write(1, 2, "Cortex Code", TextStyle::bright())?;
    canvas.write(3, 2, title_for_screen(screen_id), TextStyle::bright())?;
    canvas.write(5, 2, description_for_screen(screen_id), TextStyle::text())?;
    if screen.surface == Surface::ActionDock {
        let dock = DecisionDock::new(
            Surface::ActionDock,
            title_for_screen(screen_id),
            description_for_screen(screen_id),
            &action.label,
        )?;
        for (offset, line) in dock.render()?.into_iter().enumerate() {
            canvas.write(action_row + offset as u16, 2, &line.text, line.style)?;
        }
    } else {
        canvas.write(
            action_row,
            2,
            &format!("[enter] {}", action.label),
            TextStyle::accent(),
        )?;
    }
    canvas.write(
        10,
        2,
        "Your draft and conversation stay available.",
        TextStyle::muted(),
    )?;
    canvas.write(CANVAS_ROWS - 1, 2, statusline.text(), statusline.style())?;

    let candidates = vec![
        candidate(
            screen_id,
            fixture_id,
            "surface",
            screen_id,
            RenderRef {
                row: 3,
                start: 2,
                end: 2 + title_for_screen(screen_id).chars().count() as u16,
            },
        ),
        candidate(
            screen_id,
            fixture_id,
            "origin",
            "chat-turn",
            RenderRef {
                row: 10,
                start: 2,
                end: 45,
            },
        ),
        candidate(
            screen_id,
            fixture_id,
            "primary-action",
            &action.action_id,
            RenderRef {
                row: action_row
                    + if screen.surface == Surface::ActionDock {
                        2
                    } else {
                        0
                    },
                start: 2,
                end: 2 + 8 + action.label.chars().count() as u16,
            },
        ),
        candidate(
            screen_id,
            fixture_id,
            "statusline",
            "main",
            RenderRef {
                row: CANVAS_ROWS - 1,
                start: 2,
                end: 38,
            },
        ),
    ];
    let events = candidates
        .iter()
        .map(|candidate| SemanticEvent {
            identity: candidate.identity.clone(),
            action: (candidate.identity.family == "primary-action").then(|| action.clone()),
        })
        .collect::<Vec<_>>();

    let document = AuditDocument {
        screen_id: screen_id.to_owned(),
        fixture_id: fixture_id.to_owned(),
        styled_cells: canvas.cells,
        candidates,
        events,
        transitions: vec![transition],
    };
    document.validate()?;
    Ok(document)
}

fn candidate(
    screen_id: &str,
    fixture_id: &str,
    family: &str,
    subject_id: &str,
    render_ref: RenderRef,
) -> SemanticCandidate {
    let candidate_id = format!("candidate.{family}.{subject_id}");
    SemanticCandidate {
        identity: Identity {
            screen_id: screen_id.to_owned(),
            fixture_id: fixture_id.to_owned(),
            event_id: format!("event.{family}.{subject_id}"),
            candidate_id,
            family: family.to_owned(),
            subject_id: subject_id.to_owned(),
            render_refs: vec![render_ref],
            detail_level: "summary".to_owned(),
        },
    }
}

fn screen_action(screen_id: &str) -> (ActionEvidence, TransitionTrace) {
    let (action_id, label, automatic, target, kind, path) = match screen_id {
        "permission-shell" => (
            "action.permission.allow-once",
            "Allow once",
            false,
            "diff-edit",
            "direct",
            "permission approval opens the pending edit",
        ),
        "rate-limit" => (
            "action.recovery.retry-window",
            "Wait for retry",
            true,
            "chat-turn",
            "automatic",
            "the bounded retry restores the paused conversation",
        ),
        "diff-edit" => (
            "action.diff.open-review",
            "Inspect changes",
            false,
            "diff-viewer-staging",
            "direct",
            "the pending edit opens focused review",
        ),
        _ => (
            "action.screen.continue",
            "Continue",
            false,
            screen_id,
            "direct",
            "the current state remains available",
        ),
    };
    let action = ActionEvidence {
        action_id: action_id.to_owned(),
        label: label.to_owned(),
        enabled: true,
        automatic,
    };
    let transition = TransitionTrace {
        action_id: action_id.to_owned(),
        from_screen_id: screen_id.to_owned(),
        to_screen_id: target.to_owned(),
        kind: kind.to_owned(),
        path: path.to_owned(),
    };
    (action, transition)
}

fn title_for_screen(screen_id: &str) -> &'static str {
    match screen_id {
        "permission-shell" => "Allow this project check?",
        "diff-edit" => "Review the pending change",
        "rate-limit" => "Connection is waiting to retry",
        "chat-turn" => "Continue the conversation",
        _ => "Continue your work",
    }
}

fn description_for_screen(screen_id: &str) -> &'static str {
    match screen_id {
        "permission-shell" => "Code wants to check the project files before continuing.",
        "diff-edit" => "The change is waiting for review and has not been applied.",
        "rate-limit" => "Code will retry once when the safe waiting window ends.",
        _ => "One clear next action keeps this work moving.",
    }
}

fn validate_identity(identity: &Identity, screen_id: &str, fixture_id: &str) -> Result<(), String> {
    if identity.screen_id != screen_id || identity.fixture_id != fixture_id {
        return Err("semantic identity is not bound to its rendered screen and fixture".to_owned());
    }
    for (name, value) in [
        ("candidate_id", &identity.candidate_id),
        ("event_id", &identity.event_id),
        ("family", &identity.family),
        ("subject_id", &identity.subject_id),
        ("detail_level", &identity.detail_level),
    ] {
        if value.trim().is_empty() {
            return Err(format!("semantic identity is missing {name}"));
        }
    }
    if identity.render_refs.is_empty() {
        return Err("semantic identity needs rendered cell references".to_owned());
    }
    for reference in &identity.render_refs {
        if reference.row >= CANVAS_ROWS
            || reference.start >= reference.end
            || reference.end > CANVAS_COLUMNS
        {
            return Err("semantic identity contains an out-of-bounds render reference".to_owned());
        }
    }
    Ok(())
}

fn validate_action(action: &ActionEvidence) -> Result<(), String> {
    if action.action_id.trim().is_empty() || action.label.trim().is_empty() {
        return Err("interactive evidence needs a stable action identity and label".to_owned());
    }
    if !action.enabled {
        return Err(format!(
            "audit contains inaccessible action: {}",
            action.action_id
        ));
    }
    Ok(())
}

fn validate_styled_cells(cells: &[StyledCell]) -> Result<(), String> {
    if cells.len() != CANVAS_CELL_COUNT {
        return Err(format!(
            "styled cell audit must contain exactly {CANVAS_CELL_COUNT} cells"
        ));
    }
    for (index, cell) in cells.iter().enumerate() {
        let expected_row = index / CANVAS_COLUMNS as usize;
        let expected_column = index % CANVAS_COLUMNS as usize;
        if cell.row as usize != expected_row || cell.column as usize != expected_column {
            return Err("styled cells must be ordered in canonical row-major order".to_owned());
        }
        validate_glyphs(&cell.glyph.to_string())?;
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct Canvas {
    cells: Vec<StyledCell>,
}

impl Canvas {
    fn new() -> Self {
        let style = TextStyle {
            foreground: ColorToken::Text,
            background: Some(ColorToken::Canvas),
            bold: false,
        };
        let mut cells = Vec::with_capacity(CANVAS_CELL_COUNT);
        for row in 0..CANVAS_ROWS {
            for column in 0..CANVAS_COLUMNS {
                cells.push(StyledCell {
                    row,
                    column,
                    glyph: ' ',
                    style,
                });
            }
        }
        Self { cells }
    }

    fn write(&mut self, row: u16, column: u16, text: &str, style: TextStyle) -> Result<(), String> {
        validate_glyphs(text)?;
        let width = text.chars().count() as u16;
        if row >= CANVAS_ROWS || column + width > CANVAS_COLUMNS {
            return Err("audit text exceeds the fixed 120x40 canvas".to_owned());
        }
        for (offset, glyph) in text.chars().enumerate() {
            self.cells[row as usize * CANVAS_COLUMNS as usize + column as usize + offset] =
                StyledCell {
                    row,
                    column: column + offset as u16,
                    glyph,
                    style,
                };
        }
        Ok(())
    }
}

fn color_name(color: ColorToken) -> &'static str {
    match color {
        ColorToken::Canvas => "canvas",
        ColorToken::Surface => "surface",
        ColorToken::Text => "text",
        ColorToken::Bright => "bright",
        ColorToken::Muted => "muted",
        ColorToken::Faint => "faint",
        ColorToken::Border => "border",
        ColorToken::Accent => "accent",
        ColorToken::Info => "info",
        ColorToken::Ok => "ok",
        ColorToken::Warn => "warn",
        ColorToken::Bad => "bad",
        ColorToken::AddBackground => "add-background",
        ColorToken::DeleteBackground => "delete-background",
    }
}

fn identity_json(output: &mut String, identity: &Identity) {
    output.push('{');
    identity_fields_json(output, identity);
    output.push('}');
}

fn identity_fields_json(output: &mut String, identity: &Identity) {
    raw_field(output, "screen_id", &identity.screen_id);
    field(output, "fixture_id", &identity.fixture_id);
    field(output, "candidate_id", &identity.candidate_id);
    field(output, "event_id", &identity.event_id);
    field(output, "family", &identity.family);
    field(output, "subject_id", &identity.subject_id);
    field(output, "detail_level", &identity.detail_level);
    output.push_str(",\"render_refs\":[");
    for (index, reference) in identity.render_refs.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        let _ = write!(
            output,
            "{{\"row\":{},\"start\":{},\"end\":{}}}",
            reference.row, reference.start, reference.end
        );
    }
    output.push(']');
}

pub(crate) fn action_json(output: &mut String, action: &ActionEvidence) {
    output.push('{');
    raw_field(output, "action_id", &action.action_id);
    field(output, "label", &action.label);
    output.push_str(",\"enabled\":");
    output.push_str(if action.enabled { "true" } else { "false" });
    output.push_str(",\"automatic\":");
    output.push_str(if action.automatic { "true" } else { "false" });
    output.push('}');
}

pub(crate) fn transition_json(output: &mut String, transition: &TransitionTrace) {
    output.push('{');
    raw_field(output, "action_id", &transition.action_id);
    field(output, "from_screen_id", &transition.from_screen_id);
    field(output, "to_screen_id", &transition.to_screen_id);
    field(output, "kind", &transition.kind);
    field(output, "path", &transition.path);
    output.push('}');
}

pub(crate) fn field(output: &mut String, name: &str, value: &str) {
    output.push(',');
    raw_field(output, name, value);
}

fn raw_field(output: &mut String, name: &str, value: &str) {
    quote(output, name);
    output.push(':');
    quote(output, value);
}

pub(crate) fn quote(output: &mut String, value: &str) {
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character.is_control() => {
                let _ = write!(output, "\\u{:04x}", character as u32);
            }
            character => output.push(character),
        }
    }
    output.push('"');
}

/// Small, dependency-free SHA-256 implementation for deterministic evidence.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut state = [
        0x6a09e667u32,
        0xbb67ae85,
        0x3c6ef372,
        0xa54ff53a,
        0x510e527f,
        0x9b05688c,
        0x1f83d9ab,
        0x5be0cd19,
    ];
    let bit_length = (bytes.len() as u64).wrapping_mul(8);
    let mut padded = bytes.to_vec();
    padded.push(0x80);
    while !(padded.len() + 8).is_multiple_of(64) {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_length.to_be_bytes());

    for chunk in padded.chunks_exact(64) {
        let mut words = [0u32; 64];
        for (index, word) in words[..16].iter_mut().enumerate() {
            *word = u32::from_be_bytes(chunk[index * 4..index * 4 + 4].try_into().unwrap());
        }
        for index in 16..64 {
            let s0 = words[index - 15].rotate_right(7)
                ^ words[index - 15].rotate_right(18)
                ^ (words[index - 15] >> 3);
            let s1 = words[index - 2].rotate_right(17)
                ^ words[index - 2].rotate_right(19)
                ^ (words[index - 2] >> 10);
            words[index] = words[index - 16]
                .wrapping_add(s0)
                .wrapping_add(words[index - 7])
                .wrapping_add(s1);
        }
        let mut working = state;
        for (index, constant) in SHA256_CONSTANTS.iter().enumerate() {
            let s1 = working[4].rotate_right(6)
                ^ working[4].rotate_right(11)
                ^ working[4].rotate_right(25);
            let choice = (working[4] & working[5]) ^ (!working[4] & working[6]);
            let temporary_one = working[7]
                .wrapping_add(s1)
                .wrapping_add(choice)
                .wrapping_add(*constant)
                .wrapping_add(words[index]);
            let s0 = working[0].rotate_right(2)
                ^ working[0].rotate_right(13)
                ^ working[0].rotate_right(22);
            let majority =
                (working[0] & working[1]) ^ (working[0] & working[2]) ^ (working[1] & working[2]);
            let temporary_two = s0.wrapping_add(majority);
            working[7] = working[6];
            working[6] = working[5];
            working[5] = working[4];
            working[4] = working[3].wrapping_add(temporary_one);
            working[3] = working[2];
            working[2] = working[1];
            working[1] = working[0];
            working[0] = temporary_one.wrapping_add(temporary_two);
        }
        for (target, value) in state.iter_mut().zip(working) {
            *target = target.wrapping_add(value);
        }
    }
    let mut output = String::with_capacity(64);
    for word in state {
        let _ = write!(output, "{word:08x}");
    }
    output
}

const SHA256_CONSTANTS: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];
