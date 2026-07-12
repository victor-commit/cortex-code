//! Shared screen anatomy for the v3 conversation surfaces.
//!
//! These types deliberately model readable user-facing content, local keys,
//! focus, and a single primary action. Screens compose them instead of
//! hand-rolling detached pickers, recovery cards, or mission dashboards.

use crate::{
    chrome::tokens::{ColorToken, TextStyle, ACCENT_USE_BUDGET},
    Surface,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComponentLine {
    pub text: String,
    pub style: TextStyle,
}

impl ComponentLine {
    fn new(text: impl Into<String>, style: TextStyle) -> Self {
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
    const fn passive() -> Self {
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
        if !self.compact {
            let mut lines = vec![
                ComponentLine::new(summary, TextStyle::text()),
                ComponentLine::new(format!("  {}", self.detail), TextStyle::muted()),
            ];
            if let Some(action) = &self.interrupt {
                lines.push(ComponentLine::new(
                    format!("  [s] {action}"),
                    TextStyle::bright(),
                ));
            }
            return lines;
        }

        summary.push_str(&format!(" · {}", self.detail));
        if let Some(action) = &self.interrupt {
            summary.push_str(&format!(" · [s] {action}"));
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
    expanded_detail: Option<String>,
}

impl DisclosureBlock {
    pub fn new(summary: &str, state: DisclosureState) -> Self {
        Self {
            summary: summary.to_owned(),
            state,
            expanded_detail: None,
        }
    }

    pub fn with_expanded_detail(mut self, detail: &str) -> Self {
        self.expanded_detail = Some(detail.to_owned());
        self
    }
}

impl Component for DisclosureBlock {
    fn lines(&self) -> Vec<ComponentLine> {
        match self.state {
            DisclosureState::Collapsed => vec![ComponentLine::new(
                format!("› {}  [enter] Expand", self.summary),
                TextStyle::text(),
            )],
            DisclosureState::Expanded => {
                let mut lines = vec![ComponentLine::new(
                    format!("▾ {}  [enter] Collapse", self.summary),
                    TextStyle::text(),
                )];
                if let Some(detail) = &self.expanded_detail {
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
        lines.extend([
            ComponentLine::new(format!("  {}", self.consequence), TextStyle::muted()),
            ComponentLine::new(
                format!("[enter] {}  [esc] Cancel  [↑↓] Choose", self.primary_action),
                TextStyle::bright(),
            ),
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
                format!("[enter] {}", self.primary_action),
                TextStyle::accent(),
            ),
        ];
        for (index, action) in self.secondary_actions.iter().enumerate() {
            let key = if index == 0 { "d" } else { "esc" };
            lines.push(ComponentLine::new(
                format!("[{key}] {action}"),
                TextStyle::muted(),
            ));
        }
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
        vec![
            ComponentLine::new(
                format!("{} · {}", self.capability, self.state.label()),
                TextStyle::text(),
            ),
            ComponentLine::new(
                format!("  [enter] {}", self.primary_action),
                TextStyle::bright(),
            ),
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
        vec![
            ComponentLine::new(self.capability.clone(), TextStyle::bright()),
            ComponentLine::new(self.detail.clone(), TextStyle::text()),
            ComponentLine::new(
                format!("[esc] Return to {}", self.origin),
                TextStyle::muted(),
            ),
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
                format!("[enter] {}", self.primary_action),
                TextStyle::accent(),
            ),
        ];
        if let Some(action) = &self.evidence_action {
            lines.push(ComponentLine::new(
                format!("[d] {action}"),
                TextStyle::muted(),
            ));
        }
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
            format!("[enter] {}", self.primary_action),
            TextStyle::accent(),
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
        lines.push(ComponentLine::new(
            format!("[esc] Return to conversation ({})", self.origin),
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
            format!("[esc] Return to {}", self.origin),
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
                format!("[enter] {}", self.primary_action),
                TextStyle::accent(),
            ));
            if let Some(secondary) = &self.secondary_action {
                lines.push(ComponentLine::new(
                    format!("[d] {secondary}"),
                    TextStyle::muted(),
                ));
            }
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
    if component
        .lines()
        .iter()
        .any(|line| line.text.contains("ctx "))
    {
        return Err("components may not add statusline context usage".to_owned());
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
