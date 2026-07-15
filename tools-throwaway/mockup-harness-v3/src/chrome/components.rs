//! Shared screen anatomy for the v3 conversation surfaces.
//!
//! These types deliberately model readable user-facing content, local keys,
//! focus, and a single primary action. Screens compose them instead of
//! hand-rolling detached pickers, recovery cards, or mission dashboards.
//!
//! Shortcut and return copy uses Claude-style prose bylines
//! (`Enter to apply · Esc to cancel`) — never bracket key-chord atoms.

use crate::{
    chrome::tokens::{ColorToken, TextStyle, ACCENT_USE_BUDGET},
    Surface,
};

/// Known journey-breadcrumb prefixes that must never be primary titles (N-12).
const FORBIDDEN_JOURNEY_TITLE_PREFIXES: &[&str] = &[
    "Daily work ·",
    "Mission ·",
    "Input and approval ·",
    "Work configuration ·",
    "Provider and MCP ·",
    "Recovery ·",
];

/// Implementation meta phrases banned from the composer jewel (N-13).
const FORBIDDEN_COMPOSER_DEBUG_META: &[&str] =
    &["cursor at character", "wraps visually"];

/// One Claude-style shortcut clause: `Enter to confirm`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShortcutHint {
    key_words: String,
    verb: String,
}

impl ShortcutHint {
    pub fn new(key_words: impl Into<String>, verb: impl Into<String>) -> Result<Self, String> {
        let key_words = key_words.into().trim().to_owned();
        let verb = noun_clause(&verb.into());
        if key_words.is_empty() || verb.is_empty() {
            return Err("shortcut hint requires key words and a verb".to_owned());
        }
        if contains_bracket_key_atom(&key_words) || contains_bracket_key_atom(&verb) {
            return Err("shortcut hints may not use bracket key atoms".to_owned());
        }
        Ok(Self { key_words, verb })
    }

    pub fn render(&self) -> String {
        format!("{} to {}", self.key_words, self.verb)
    }
}

/// Middot-joined prose byline for local shortcuts (F-8 / VAL-V3-DS-030).
///
/// Example: `Enter to send · Esc to cancel · ↑↓ to choose`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShortcutByline {
    hints: Vec<ShortcutHint>,
}

impl ShortcutByline {
    pub fn new(hints: Vec<ShortcutHint>) -> Result<Self, String> {
        if hints.is_empty() {
            return Err("shortcut byline requires at least one hint".to_owned());
        }
        if hints
            .iter()
            .any(|hint| contains_bracket_key_atom(&hint.render()))
        {
            return Err("shortcut byline may not emit bracket key atoms".to_owned());
        }
        Ok(Self { hints })
    }

    /// Common confirm/cancel pair used by docks and cards.
    pub fn enter_esc(enter_verb: &str, esc_verb: &str) -> Result<Self, String> {
        Self::new(vec![
            ShortcutHint::new("Enter", enter_verb)?,
            ShortcutHint::new("Esc", esc_verb)?,
        ])
    }

    /// Confirm, cancel, and list navigation.
    pub fn enter_esc_choose(enter_verb: &str, esc_verb: &str) -> Result<Self, String> {
        Self::new(vec![
            ShortcutHint::new("Enter", enter_verb)?,
            ShortcutHint::new("Esc", esc_verb)?,
            ShortcutHint::new("↑↓", "choose")?,
        ])
    }

    pub fn render(&self) -> String {
        self.hints
            .iter()
            .map(ShortcutHint::render)
            .collect::<Vec<_>>()
            .join(" · ")
    }

    pub fn as_component_line(&self, style: TextStyle) -> ComponentLine {
        ComponentLine::new(self.render(), style)
    }
}

/// True when `text` contains a bracket key-chord atom such as `[enter]` or `[@]`.
/// Content tokens like `[attachment: path]` are ignored.
pub fn contains_bracket_key_atom(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            if let Some(rel_end) = text[i + 1..].find(']') {
                let inner = &text[i + 1..i + 1 + rel_end];
                if is_key_chord_token(inner) {
                    return true;
                }
                i += 2 + rel_end;
                continue;
            }
        }
        i += 1;
    }
    false
}

/// True when a primary title uses a journey breadcrumb prefix (N-12).
pub fn is_journey_breadcrumb_title(title: &str) -> bool {
    let trimmed = title.trim();
    FORBIDDEN_JOURNEY_TITLE_PREFIXES
        .iter()
        .any(|prefix| trimmed.starts_with(prefix))
}

/// Fail closed when a title is a journey breadcrumb rather than a human subject.
pub fn reject_journey_breadcrumb_title(title: &str) -> Result<(), String> {
    if is_journey_breadcrumb_title(title) {
        return Err(format!(
            "journey breadcrumb titles are banned (use a human subject): {title}"
        ));
    }
    Ok(())
}

/// Map audit journey-origin identity strings to human **painted** subject titles (N-12).
///
/// Audit `visible_origin` may still carry journey tags for continuity gates; primary
/// on-screen titles MUST call this (or paint a human subject directly) so users never
/// see storyboard crumbs like `Daily work · …`.
pub fn subject_title_for_origin(journey_or_subject: &str) -> &'static str {
    let trimmed = journey_or_subject.trim();
    match trimmed {
        "Daily work · current conversation" => "Current conversation",
        "Input and approval · paused conversation" => "Paused conversation",
        "Work configuration · current conversation" => "Work configuration",
        "Work configuration · onboarding" => "First-run setup",
        "Provider and MCP · connect" | "Provider and MCP · connections" => "Connections",
        "Recovery · paused conversation" => "Recovery",
        "Mission · current conversation" => "Mission",
        // Already-human subjects and unknown custom titles pass through via a small set
        // of known fixtures; callers with non-static copy should paint their own subject.
        "Current conversation" => "Current conversation",
        "Paused conversation" => "Paused conversation",
        "Work configuration" => "Work configuration",
        "First-run setup" => "First-run setup",
        "Connections" => "Connections",
        "Recovery" => "Recovery",
        "Mission" => "Mission",
        _ if is_journey_breadcrumb_title(trimmed) => {
            // Last-resort stripping so unknown journey crumbs never paint raw.
            if let Some((_, subject)) = trimmed.split_once('·') {
                let s = subject.trim();
                if !s.is_empty() {
                    // Leak a stable static for the handful of dynamic subjects used
                    // in screens by returning common tails; Fall through nonempty tails.
                    return match s {
                        "current conversation" => "Current conversation",
                        "paused conversation" => "Paused conversation",
                        "onboarding" => "First-run setup",
                        other if other.eq_ignore_ascii_case("connect") => "Connections",
                        _ => "Conversation",
                    };
                }
            }
            "Conversation"
        }
        _ => "Conversation",
    }
}

/// Compact Claude-style user-turn surface text: soft SURFACE wash + bright body (F-9).
///
/// Pads one column on each side when space remains so the wash reads as a true surface
/// rather than a bare label. Callers own vertical spacing; this paints a single row.
pub fn user_turn_surface_line(text: &str) -> Result<ComponentLine, String> {
    let body = text.trim();
    if body.is_empty() {
        return Err("user turn surface requires text".to_owned());
    }
    if contains_bracket_key_atom(body) {
        return Err("user turn surface may not use bracket key atoms".to_owned());
    }
    if is_journey_breadcrumb_title(body) {
        return Err("user turn surface may not be a journey breadcrumb title".to_owned());
    }
    Ok(ComponentLine::new(
        format!(" {body} "),
        TextStyle::user_turn(),
    ))
}

/// Fail closed when composer chrome leaks implementation meta (N-13).
pub fn reject_composer_debug_meta(text: &str) -> Result<(), String> {
    let lower = text.to_ascii_lowercase();
    for phrase in FORBIDDEN_COMPOSER_DEBUG_META {
        if lower.contains(phrase) {
            return Err(format!("composer debug meta is banned: {phrase}"));
        }
    }
    if contains_bracket_key_atom(text) {
        return Err("composer meta may not use bracket key atoms".to_owned());
    }
    Ok(())
}

/// Clean composer header meta: line count only (no wrap/cursor debug).
pub fn composer_header_meta(authored_line_count: usize) -> Result<String, String> {
    let text = if authored_line_count == 1 {
        "Draft · 1 line".to_owned()
    } else {
        format!("Draft · {authored_line_count} lines")
    };
    reject_composer_debug_meta(&text)?;
    Ok(text)
}

fn is_key_chord_token(inner: &str) -> bool {
    let token = inner.trim();
    if token.is_empty() || token.len() > 12 {
        return false;
    }
    // File/content chips such as `[attachment: path]` are not key chords.
    if token.contains(':') || token.contains(' ') || token.contains('/') {
        return false;
    }
    let lower = token.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "enter"
            | "esc"
            | "escape"
            | "tab"
            | "space"
            | "return"
            | "shift"
            | "↑↓"
            | "↑"
            | "↓"
            | "@"
            | "/"
            | "?"
            | "sc"
            | "ctrl+k"
            | "ctrl+g"
            | "ctrl+c"
            | "ctrl+d"
    ) || token
        .chars()
        .all(|c| "↑↓←→".contains(c) && !c.is_whitespace())
        || (token.len() <= 3
            && token
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '^'))
}

fn noun_clause(verb: &str) -> String {
    let trimmed = verb.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let mut chars = trimmed.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn byline_or_empty(result: Result<ShortcutByline, String>) -> String {
    result
        .map(|byline| byline.render())
        .unwrap_or_else(|_| String::new())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComponentLine {
    pub text: String,
    pub style: TextStyle,
}

impl ComponentLine {
    /// Construct a styled chrome line. Public for sibling chrome modules
    /// (mission-control primitives, layout helpers).
    pub fn new(text: impl Into<String>, style: TextStyle) -> Self {
        Self {
            text: text.into(),
            style,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ComponentAnatomy {
    pub primary_action_count: usize,
    pub secondary_action_count: usize,
    pub has_local_keys: bool,
    pub accent_uses: usize,
}

impl ComponentAnatomy {
    /// Shared empty anatomy for passive chrome (rows, headers, logs).
    pub const fn passive() -> Self {
        Self {
            primary_action_count: 0,
            secondary_action_count: 0,
            has_local_keys: false,
            accent_uses: 0,
        }
    }

    const fn actionable(secondary_action_count: usize, accent_uses: usize) -> Self {
        Self {
            primary_action_count: 1,
            secondary_action_count,
            has_local_keys: true,
            accent_uses,
        }
    }
}

/// Every shared component exposes bounded, audit-friendly visual anatomy.
pub trait Component {
    fn lines(&self) -> Vec<ComponentLine>;
    fn anatomy(&self) -> ComponentAnatomy;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActivityState {
    Running,
    Waiting,
    Succeeded,
    Failed,
    Expanded,
}

impl ActivityState {
    const fn marker(self) -> &'static str {
        match self {
            Self::Running => "●",
            Self::Waiting => "○",
            Self::Succeeded => "✓",
            Self::Failed => "✗",
            Self::Expanded => "▾",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Running => "Running",
            Self::Waiting => "Waiting",
            Self::Succeeded => "Complete",
            Self::Failed => "Needs attention",
            Self::Expanded => "Details open",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActivityRow {
    label: String,
    state: ActivityState,
    detail: String,
    elapsed: Option<String>,
    interrupt: Option<String>,
    compact: bool,
}

impl ActivityRow {
    pub fn new(label: &str, state: ActivityState, detail: &str) -> Self {
        Self {
            label: label.to_owned(),
            state,
            detail: detail.to_owned(),
            elapsed: None,
            interrupt: None,
            compact: false,
        }
    }

    pub fn with_elapsed(mut self, elapsed: &str) -> Self {
        self.elapsed = Some(elapsed.to_owned());
        self
    }

    /// The interrupt control is local to the live activity, never a global hint.
    pub fn with_interrupt(mut self, action: &str) -> Self {
        self.interrupt = Some(action.to_owned());
        self
    }

    /// Keeps a tool's status, progress, and scoped control in one inline row.
    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }
}

impl Component for ActivityRow {
    fn lines(&self) -> Vec<ComponentLine> {
        let mut summary = format!(
            "{} {} · {}",
            self.state.marker(),
            self.label,
            self.state.label()
        );
        if let Some(elapsed) = &self.elapsed {
            summary.push_str(&format!(" · {elapsed}"));
        }
        let interrupt_byline = self.interrupt.as_ref().map(|action| {
            byline_or_empty(
                ShortcutHint::new("Esc", action.as_str())
                    .and_then(|hint| ShortcutByline::new(vec![hint])),
            )
        });
        if !self.compact {
            let mut lines = vec![
                ComponentLine::new(summary, TextStyle::text()),
                ComponentLine::new(format!("  {}", self.detail), TextStyle::muted()),
            ];
            if let Some(byline) = interrupt_byline {
                lines.push(ComponentLine::new(format!("  {byline}"), TextStyle::bright()));
            }
            return lines;
        }

        summary.push_str(&format!(" · {}", self.detail));
        if let Some(byline) = interrupt_byline {
            summary.push_str(&format!(" · {byline}"));
        }
        let style = match self.state {
            ActivityState::Running => TextStyle::bright(),
            ActivityState::Succeeded => TextStyle {
                foreground: ColorToken::Ok,
                background: None,
                bold: false,
            },
            ActivityState::Failed => TextStyle {
                foreground: ColorToken::Bad,
                background: None,
                bold: false,
            },
            _ => TextStyle::text(),
        };
        vec![ComponentLine::new(summary, style)]
    }

    fn anatomy(&self) -> ComponentAnatomy {
        ComponentAnatomy {
            has_local_keys: self.interrupt.is_some(),
            ..ComponentAnatomy::passive()
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DisclosureState {
    Collapsed,
    Expanded,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisclosureBlock {
    summary: String,
    state: DisclosureState,
    expanded_lines: Vec<String>,
    expand_label: String,
    collapse_label: String,
}

impl DisclosureBlock {
    pub fn new(summary: &str, state: DisclosureState) -> Self {
        Self {
            summary: summary.to_owned(),
            state,
            expanded_lines: Vec::new(),
            expand_label: "Expand".to_owned(),
            collapse_label: "Collapse".to_owned(),
        }
    }

    pub fn with_expanded_detail(mut self, detail: &str) -> Self {
        self.expanded_lines = vec![detail.to_owned()];
        self
    }

    /// Focused multi-line expansion under the same summary row.
    pub fn with_expanded_lines<I, S>(mut self, lines: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.expanded_lines = lines.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_action_labels(mut self, expand: &str, collapse: &str) -> Self {
        self.expand_label = expand.to_owned();
        self.collapse_label = collapse.to_owned();
        self
    }
}

impl Component for DisclosureBlock {
    fn lines(&self) -> Vec<ComponentLine> {
        match self.state {
            DisclosureState::Collapsed => {
                let byline = byline_or_empty(
                    ShortcutHint::new("Enter", self.expand_label.as_str())
                        .and_then(|hint| ShortcutByline::new(vec![hint])),
                );
                vec![ComponentLine::new(
                    format!("› {} · {byline}", self.summary),
                    TextStyle::text(),
                )]
            }
            DisclosureState::Expanded => {
                let byline = byline_or_empty(
                    ShortcutHint::new("Enter", self.collapse_label.as_str())
                        .and_then(|hint| ShortcutByline::new(vec![hint])),
                );
                let mut lines = vec![ComponentLine::new(
                    format!("▾ {} · {byline}", self.summary),
                    TextStyle::text(),
                )];
                for detail in &self.expanded_lines {
                    lines.push(ComponentLine::new(
                        format!("  {detail}"),
                        TextStyle::muted(),
                    ));
                }
                lines
            }
        }
    }

    fn anatomy(&self) -> ComponentAnatomy {
        ComponentAnatomy {
            has_local_keys: true,
            accent_uses: 1,
            ..ComponentAnatomy::passive()
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectionRow {
    label: String,
    consequence: String,
}

impl SelectionRow {
    pub fn new(label: &str, consequence: &str) -> Self {
        Self {
            label: label.to_owned(),
            consequence: consequence.to_owned(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectionDock {
    title: String,
    rows: Vec<SelectionRow>,
    selected_index: usize,
    consequence: String,
    primary_action: String,
}

impl SelectionDock {
    pub fn new(
        title: &str,
        rows: Vec<SelectionRow>,
        selected_index: usize,
        consequence: &str,
        primary_action: &str,
    ) -> Result<Self, String> {
        if rows.is_empty() || selected_index >= rows.len() {
            return Err("selection dock requires one selected row".to_owned());
        }
        if primary_action.trim().is_empty() {
            return Err("selection dock requires one primary action".to_owned());
        }
        Ok(Self {
            title: title.to_owned(),
            rows,
            selected_index,
            consequence: consequence.to_owned(),
            primary_action: primary_action.to_owned(),
        })
    }
}

impl Component for SelectionDock {
    fn lines(&self) -> Vec<ComponentLine> {
        let mut lines = vec![ComponentLine::new(self.title.clone(), TextStyle::bright())];
        for (index, row) in self.rows.iter().enumerate() {
            let selected = index == self.selected_index;
            let marker = if selected { "›" } else { " " };
            lines.push(ComponentLine::new(
                format!("{marker} {}", row.label),
                if selected {
                    TextStyle::accent()
                } else {
                    TextStyle::text()
                },
            ));
            if selected {
                lines.push(ComponentLine::new(
                    format!("  {}", row.consequence),
                    TextStyle::muted(),
                ));
            }
        }
        let byline = byline_or_empty(ShortcutByline::enter_esc_choose(
            self.primary_action.as_str(),
            "cancel",
        ));
        lines.extend([
            ComponentLine::new(format!("  {}", self.consequence), TextStyle::muted()),
            ComponentLine::new(byline, TextStyle::bright()),
        ]);
        lines
    }

    fn anatomy(&self) -> ComponentAnatomy {
        ComponentAnatomy::actionable(1, 1)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionDock {
    surface: Surface,
    title: String,
    effect: String,
    primary_action: String,
    secondary_actions: Vec<String>,
}

impl DecisionDock {
    pub fn new(
        surface: Surface,
        title: &str,
        effect: &str,
        primary_action: &str,
    ) -> Result<Self, String> {
        if surface != Surface::ActionDock {
            return Err("decision dock must render on the action dock surface".to_owned());
        }
        if primary_action.trim().is_empty() {
            return Err("decision dock requires one primary action".to_owned());
        }
        Ok(Self {
            surface,
            title: title.to_owned(),
            effect: effect.to_owned(),
            primary_action: primary_action.to_owned(),
            secondary_actions: Vec::new(),
        })
    }

    pub fn with_secondary(mut self, action: &str) -> Result<Self, String> {
        if self.secondary_actions.len() == 2 {
            return Err("decision dock allows at most two secondary actions".to_owned());
        }
        self.secondary_actions.push(action.to_owned());
        Ok(self)
    }

    fn validate(&self) -> Result<(), String> {
        if self.surface != Surface::ActionDock {
            return Err("decision dock must render on the action dock surface".to_owned());
        }
        if self.primary_action.trim().is_empty() {
            return Err("decision dock requires one primary action".to_owned());
        }
        if self.secondary_actions.len() > 2 {
            return Err("decision dock allows at most two secondary actions".to_owned());
        }
        Ok(())
    }

    /// Emits dock lines only after enforcing the central action hierarchy and
    /// bottom-dock placement contract.
    pub fn render(&self) -> Result<Vec<ComponentLine>, String> {
        self.validate()?;
        validate_primary_actions(1, false)?;
        validate_key_placement("key.action-dock.enter", true)?;
        validate_status_copy("status.decision", &self.effect)?;
        Ok(self.render_lines())
    }

    fn render_lines(&self) -> Vec<ComponentLine> {
        let mut lines = vec![
            ComponentLine::new(self.title.clone(), TextStyle::bright()),
            ComponentLine::new(self.effect.clone(), TextStyle::text()),
            ComponentLine::new(
                format!("› {}", self.primary_action),
                TextStyle::accent(),
            ),
        ];
        for action in &self.secondary_actions {
            lines.push(ComponentLine::new(
                format!("  {action}"),
                TextStyle::muted(),
            ));
        }
        let byline = byline_or_empty(ShortcutByline::enter_esc(
            self.primary_action.as_str(),
            "cancel",
        ));
        lines.push(ComponentLine::new(byline, TextStyle::muted()));
        lines
    }
}

impl Component for DecisionDock {
    fn lines(&self) -> Vec<ComponentLine> {
        self.render()
            .expect("DecisionDock construction enforces render invariants")
    }

    fn anatomy(&self) -> ComponentAnatomy {
        ComponentAnatomy::actionable(self.secondary_actions.len(), 1)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionState {
    Connected,
    NeedsSignIn,
    Blocked,
    Failed,
}

impl ConnectionState {
    const fn label(self) -> &'static str {
        match self {
            Self::Connected => "Connected",
            Self::NeedsSignIn => "Sign in needed",
            Self::Blocked => "Needs attention",
            Self::Failed => "Could not connect",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectionRow {
    capability: String,
    state: ConnectionState,
    primary_action: String,
}

impl ConnectionRow {
    pub fn new(capability: &str, state: ConnectionState, primary_action: &str) -> Self {
        Self {
            capability: capability.to_owned(),
            state,
            primary_action: primary_action.to_owned(),
        }
    }
}

impl Component for ConnectionRow {
    fn lines(&self) -> Vec<ComponentLine> {
        let byline = byline_or_empty(
            ShortcutHint::new("Enter", self.primary_action.as_str())
                .and_then(|hint| ShortcutByline::new(vec![hint])),
        );
        vec![
            ComponentLine::new(
                format!("{} · {}", self.capability, self.state.label()),
                TextStyle::text(),
            ),
            ComponentLine::new(format!("  {byline}"), TextStyle::bright()),
        ]
    }

    fn anatomy(&self) -> ComponentAnatomy {
        ComponentAnatomy::actionable(0, 0)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectionDetail {
    capability: String,
    origin: String,
    detail: String,
}

impl ConnectionDetail {
    pub fn new(capability: &str, origin: &str, detail: &str) -> Self {
        Self {
            capability: capability.to_owned(),
            origin: origin.to_owned(),
            detail: detail.to_owned(),
        }
    }
}

impl Component for ConnectionDetail {
    fn lines(&self) -> Vec<ComponentLine> {
        let byline = byline_or_empty(
            ShortcutHint::new("Esc", format!("return to {}", self.origin))
                .and_then(|hint| ShortcutByline::new(vec![hint])),
        );
        vec![
            ComponentLine::new(self.capability.clone(), TextStyle::bright()),
            ComponentLine::new(self.detail.clone(), TextStyle::text()),
            ComponentLine::new(byline, TextStyle::muted()),
        ]
    }

    fn anatomy(&self) -> ComponentAnatomy {
        ComponentAnatomy::actionable(0, 0)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryBlock {
    cause: String,
    preserved: String,
    primary_action: String,
    evidence_action: Option<String>,
}

impl RecoveryBlock {
    pub fn new(cause: &str, preserved: &str, primary_action: &str) -> Self {
        Self {
            cause: cause.to_owned(),
            preserved: preserved.to_owned(),
            primary_action: primary_action.to_owned(),
            evidence_action: None,
        }
    }

    pub fn with_evidence(mut self, action: &str) -> Self {
        self.evidence_action = Some(action.to_owned());
        self
    }
}

impl Component for RecoveryBlock {
    fn lines(&self) -> Vec<ComponentLine> {
        let mut lines = vec![
            ComponentLine::new(self.cause.clone(), TextStyle::bright()),
            ComponentLine::new(self.preserved.clone(), TextStyle::text()),
            ComponentLine::new(
                format!("› {}", self.primary_action),
                TextStyle::accent(),
            ),
        ];
        if let Some(action) = &self.evidence_action {
            lines.push(ComponentLine::new(
                format!("  {action}"),
                TextStyle::muted(),
            ));
        }
        let mut hints = Vec::new();
        if let Ok(hint) = ShortcutHint::new("Enter", self.primary_action.as_str()) {
            hints.push(hint);
        }
        if let Some(action) = &self.evidence_action {
            if let Ok(hint) = ShortcutHint::new("d", format!("for {}", noun_clause(action))) {
                hints.push(hint);
            }
        }
        lines.push(ComponentLine::new(
            byline_or_empty(ShortcutByline::new(hints)),
            TextStyle::muted(),
        ));
        lines
    }

    fn anatomy(&self) -> ComponentAnatomy {
        ComponentAnatomy::actionable(self.evidence_action.is_some() as usize, 1)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MissionCard {
    goal: String,
    stage: String,
    completed: usize,
    remaining: usize,
    blocker: Option<String>,
    primary_action: String,
}

impl MissionCard {
    pub fn new(
        goal: &str,
        stage: &str,
        completed: usize,
        remaining: usize,
        blocker: Option<&str>,
        primary_action: &str,
    ) -> Self {
        Self {
            goal: goal.to_owned(),
            stage: stage.to_owned(),
            completed,
            remaining,
            blocker: blocker.map(str::to_owned),
            primary_action: primary_action.to_owned(),
        }
    }
}

impl Component for MissionCard {
    fn lines(&self) -> Vec<ComponentLine> {
        let mut lines = vec![
            ComponentLine::new(self.goal.clone(), TextStyle::bright()),
            ComponentLine::new(
                format!(
                    "{} · {} complete · {} remaining",
                    self.stage, self.completed, self.remaining
                ),
                TextStyle::text(),
            ),
        ];
        if let Some(blocker) = &self.blocker {
            lines.push(ComponentLine::new(
                format!("Needs attention: {blocker}"),
                TextStyle::text(),
            ));
        }
        lines.push(ComponentLine::new(
            format!("› {}", self.primary_action),
            TextStyle::accent(),
        ));
        lines.push(ComponentLine::new(
            byline_or_empty(
                ShortcutHint::new("Enter", self.primary_action.as_str())
                    .and_then(|hint| ShortcutByline::new(vec![hint])),
            ),
            TextStyle::muted(),
        ));
        lines
    }

    fn anatomy(&self) -> ComponentAnatomy {
        ComponentAnatomy::actionable(0, 1)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InspectorTab {
    label: String,
}

impl InspectorTab {
    pub fn new(label: &str) -> Self {
        Self {
            label: label.to_owned(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Inspector {
    title: String,
    origin: String,
    tabs: Vec<InspectorTab>,
    selected_index: usize,
}

impl Inspector {
    pub fn new(
        title: &str,
        origin: &str,
        tabs: Vec<InspectorTab>,
        selected_index: usize,
    ) -> Result<Self, String> {
        if tabs.is_empty() || selected_index >= tabs.len() {
            return Err("inspector requires one selected contextual tab".to_owned());
        }
        Ok(Self {
            title: title.to_owned(),
            origin: origin.to_owned(),
            tabs,
            selected_index,
        })
    }
}

impl Component for Inspector {
    fn lines(&self) -> Vec<ComponentLine> {
        let mut lines = vec![ComponentLine::new(self.title.clone(), TextStyle::bright())];
        for (index, tab) in self.tabs.iter().enumerate() {
            let selected = index == self.selected_index;
            lines.push(ComponentLine::new(
                format!("{} {}", if selected { "›" } else { " " }, tab.label),
                if selected {
                    TextStyle::accent()
                } else {
                    TextStyle::muted()
                },
            ));
        }
        // Keep the return line short enough for the 37-column inspector.
        lines.push(ComponentLine::new(
            byline_or_empty(
                ShortcutHint::new("Esc", "return to conversation")
                    .and_then(|hint| ShortcutByline::new(vec![hint])),
            ),
            TextStyle::muted(),
        ));
        lines
    }

    fn anatomy(&self) -> ComponentAnatomy {
        ComponentAnatomy::actionable(0, 1)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DetailView {
    title: String,
    origin: String,
    content: Vec<String>,
}

impl DetailView {
    pub fn new(title: &str, origin: &str, content: Vec<&str>) -> Self {
        Self {
            title: title.to_owned(),
            origin: origin.to_owned(),
            content: content.into_iter().map(str::to_owned).collect(),
        }
    }
}

impl Component for DetailView {
    fn lines(&self) -> Vec<ComponentLine> {
        let mut lines = vec![ComponentLine::new(self.title.clone(), TextStyle::bright())];
        lines.extend(
            self.content
                .iter()
                .cloned()
                .map(|line| ComponentLine::new(line, TextStyle::text())),
        );
        lines.push(ComponentLine::new(
            byline_or_empty(
                ShortcutHint::new("Esc", format!("return to {}", self.origin))
                    .and_then(|hint| ShortcutByline::new(vec![hint])),
            ),
            TextStyle::accent(),
        ));
        lines
    }

    fn anatomy(&self) -> ComponentAnatomy {
        ComponentAnatomy::actionable(0, 1)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiffKind {
    Added,
    Removed,
    Unchanged,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffBlock {
    path: String,
    lines: Vec<(DiffKind, String)>,
}

impl DiffBlock {
    pub fn new(path: &str, lines: Vec<(DiffKind, &str)>) -> Self {
        Self {
            path: path.to_owned(),
            lines: lines
                .into_iter()
                .map(|(kind, line)| (kind, line.to_owned()))
                .collect(),
        }
    }
}

impl Component for DiffBlock {
    fn lines(&self) -> Vec<ComponentLine> {
        let mut rendered = vec![ComponentLine::new(
            format!("Changes in {}", self.path),
            TextStyle::bright(),
        )];
        rendered.extend(self.lines.iter().map(|(kind, line)| {
            let (prefix, style) = match kind {
                DiffKind::Added => (
                    "+",
                    TextStyle {
                        foreground: ColorToken::Ok,
                        background: Some(ColorToken::AddBackground),
                        bold: false,
                    },
                ),
                DiffKind::Removed => (
                    "-",
                    TextStyle {
                        foreground: ColorToken::Bad,
                        background: Some(ColorToken::DeleteBackground),
                        bold: false,
                    },
                ),
                DiffKind::Unchanged => (" ", TextStyle::text()),
            };
            ComponentLine::new(format!("{prefix}{line}"), style)
        }));
        rendered
    }

    fn anatomy(&self) -> ComponentAnatomy {
        ComponentAnatomy::passive()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewBlock {
    title: String,
    summary: String,
    primary_action: String,
    affected_result: Option<String>,
    secondary_action: Option<String>,
    /// When false, actions live on a peer dock; the block is outcome-only.
    include_actions: bool,
}

impl ReviewBlock {
    pub fn new(title: &str, summary: &str, primary_action: &str) -> Self {
        Self {
            title: title.to_owned(),
            summary: summary.to_owned(),
            primary_action: primary_action.to_owned(),
            affected_result: None,
            secondary_action: None,
            include_actions: true,
        }
    }

    pub fn with_affected_result(mut self, result: &str) -> Self {
        self.affected_result = Some(result.to_owned());
        self
    }

    pub fn with_secondary(mut self, action: &str) -> Self {
        self.secondary_action = Some(action.to_owned());
        self
    }

    /// Outcome only; primary/secondary actions remain on the bottom dock.
    pub fn outcome_only(mut self) -> Self {
        self.include_actions = false;
        self
    }
}

impl Component for ReviewBlock {
    fn lines(&self) -> Vec<ComponentLine> {
        let mut lines = vec![
            ComponentLine::new(self.title.clone(), TextStyle::bright()),
            ComponentLine::new(self.summary.clone(), TextStyle::text()),
        ];
        if let Some(affected) = &self.affected_result {
            lines.push(ComponentLine::new(
                format!("Affected · {affected}"),
                TextStyle::muted(),
            ));
        }
        if self.include_actions {
            lines.push(ComponentLine::new(
                format!("› {}", self.primary_action),
                TextStyle::accent(),
            ));
            if let Some(secondary) = &self.secondary_action {
                lines.push(ComponentLine::new(
                    format!("  {secondary}"),
                    TextStyle::muted(),
                ));
            }
            let mut hints = Vec::new();
            if let Ok(hint) = ShortcutHint::new("Enter", self.primary_action.as_str()) {
                hints.push(hint);
            }
            if let Some(secondary) = &self.secondary_action {
                if let Ok(hint) = ShortcutHint::new("d", format!("for {}", noun_clause(secondary)))
                {
                    hints.push(hint);
                }
            }
            lines.push(ComponentLine::new(
                byline_or_empty(ShortcutByline::new(hints)),
                TextStyle::muted(),
            ));
        }
        lines
    }

    fn anatomy(&self) -> ComponentAnatomy {
        if self.include_actions {
            ComponentAnatomy::actionable(self.secondary_action.is_some() as usize, 1)
        } else {
            ComponentAnatomy::passive()
        }
    }
}

/// Compact decision-dock body used by dense offender rework: title, one effect
/// line optional, one primary, up to two compact secondary/cancel lines.
/// Avoids restating list-row bodies or dual bracket footnotes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompactActionDock {
    title: String,
    summary: Option<String>,
    primary: String,
    secondary_lines: Vec<String>,
}

impl CompactActionDock {
    pub fn new(title: &str, primary: &str) -> Result<Self, String> {
        if title.trim().is_empty() || primary.trim().is_empty() {
            return Err("compact action dock needs a title and primary action".to_owned());
        }
        if contains_bracket_key_atom(primary) {
            return Err("compact action dock primary may not use bracket key atoms".to_owned());
        }
        Ok(Self {
            title: title.to_owned(),
            summary: None,
            primary: primary.to_owned(),
            secondary_lines: Vec::new(),
        })
    }

    pub fn with_summary(mut self, summary: &str) -> Self {
        self.summary = Some(summary.to_owned());
        self
    }

    pub fn with_secondary_line(mut self, line: &str) -> Result<Self, String> {
        if self.secondary_lines.len() >= 3 {
            return Err("compact action dock allows at most three secondary lines".to_owned());
        }
        if contains_bracket_key_atom(line) {
            return Err("compact action dock secondary may not use bracket key atoms".to_owned());
        }
        self.secondary_lines.push(line.to_owned());
        Ok(self)
    }

    pub fn lines(&self) -> Vec<ComponentLine> {
        let mut lines = vec![ComponentLine::new(self.title.clone(), TextStyle::bright())];
        if let Some(summary) = &self.summary {
            lines.push(ComponentLine::new(summary.clone(), TextStyle::muted()));
        }
        lines.push(ComponentLine::new(
            format!("› {}", self.primary),
            TextStyle::accent(),
        ));
        for line in &self.secondary_lines {
            lines.push(ComponentLine::new(line.clone(), TextStyle::muted()));
        }
        lines.push(ComponentLine::new(
            byline_or_empty(ShortcutByline::enter_esc(self.primary.as_str(), "cancel")),
            TextStyle::muted(),
        ));
        lines
    }
}

/// Restrained entry-only "Code" ASCII brand (F-6). Shared so login/onboarding
/// do not re-encode snowflake banner tables.
pub const ENTRY_CODE_BANNER: [&str; 6] = [
    " ██████              ██",
    "██                   ██",
    "██        ████    █████   ████",
    "██       ██  ██  ██  ██  ██████",
    "██       ██  ██  ██  ██  ██",
    " ██████   ████    █████   █████",
];

/// Rejects a component that would compete with the one obvious next action or
/// misuse the restrained accent budget.
pub fn validate_component(component: &impl Component) -> Result<(), String> {
    let anatomy = component.anatomy();
    if anatomy.primary_action_count > 1 {
        return Err("a component may expose only one primary action".to_owned());
    }
    if anatomy.secondary_action_count > 2 {
        return Err("a component may expose at most two secondary actions".to_owned());
    }
    if anatomy.accent_uses > ACCENT_USE_BUDGET {
        return Err("a component exceeds the accent-use budget".to_owned());
    }
    for line in component.lines() {
        if line.text.contains("ctx ") {
            return Err("components may not add statusline context usage".to_owned());
        }
        if contains_bracket_key_atom(&line.text) {
            return Err(format!(
                "components may not emit bracket key atoms: {}",
                line.text
            ));
        }
        if is_journey_breadcrumb_title(&line.text) {
            return Err(format!(
                "components may not paint journey breadcrumb titles: {}",
                line.text
            ));
        }
        if reject_composer_debug_meta(&line.text).is_err()
            && (line.text.to_ascii_lowercase().contains("cursor at character")
                || line.text.to_ascii_lowercase().contains("wraps visually"))
        {
            return Err(format!(
                "components may not emit composer debug meta: {}",
                line.text
            ));
        }
    }
    Ok(())
}

/// Rejects controls that are detached from the object or dock they operate.
pub fn validate_key_placement(key: &str, is_local: bool) -> Result<(), String> {
    if !is_local {
        return Err(format!("misplaced key: {key}"));
    }
    Ok(())
}

/// Requires a textual status alongside color or a functional marker.
pub fn validate_status_copy(status_id: &str, text: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err(format!("color-only status: {status_id}"));
    }
    Ok(())
}

/// Enforces singular decision hierarchy and prevents automatic waits from
/// presenting a manual primary action.
pub fn validate_primary_actions(count: usize, automatic: bool) -> Result<(), String> {
    if count == 0 {
        return Err("hidden primary action: primary-action".to_owned());
    }
    if count > 1 {
        return Err("duplicate primary action: primary-action".to_owned());
    }
    if automatic {
        return Err(
            "automatic wait exposes a primary action: action.recovery.retry-window".to_owned(),
        );
    }
    Ok(())
}
