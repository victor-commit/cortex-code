//! Mission Control density primitives for orchestration and chat builders.
//!
//! These helpers encode the dense Factory Mission Control anatomy without
//! violating Cortex rules:
//! - live `●` marker uses phrase/color, never a filled statusline strip (N-1)
//! - panel headers carry counters (`16/22`, `1-12 of n`)
//! - zoned panels split active work | checklist/log
//! - selection uses `›` without nested brackets (N-11 / F-5)
//! - compressed log lines optionally carry relative ages
//! - bylines remain Claude prose (`ShortcutByline`); no global chat keybar (N-9)
//!
//! Builders compose these into screens. This module does **not** rewrite
//! existing screen fixtures; it unlocks Phase-A orchestration and chat work.
//!
//! # Usage (builders)
//!
//! ```ignore
//! use mockup_harness_v3::chrome::mission_control::*;
//!
//! let band = StatusBand::new(LiveState::Running)
//!     .with_stage("Active work")
//!     .with_fraction(16, 22)?;
//!
//! let panel = ZonedPanel::new(vec![
//!     Zone::new(
//!         PanelHeader::new("Active Feature"),
//!         ZoneBody::Lines(vec!["Goal and stage lines".into()]),
//!     ),
//!     Zone::new(
//!         PanelHeader::new("Features").with_counter(PanelCounter::Fraction {
//!             done: 16,
//!             total: 22,
//!         }),
//!         ZoneBody::Selection(SelectionSurface::new(rows, selected)?),
//!     ),
//!     Zone::new(
//!         PanelHeader::new("Progress Log")
//!             .with_counter(PanelCounter::PageRange { start: 1, end: 12, total: 124 }),
//!         ZoneBody::Log(CompressedLog::new(log_lines).with_page(1, 12, 124)),
//!     ),
//! ]);
//! // Optional mission-local strip (temporary detail only):
//! let strip = MissionLocalKeyStrip::new(ShortcutByline::enter_esc("open", "back")?);
//! ```

use crate::chrome::{
    components::{
        contains_bracket_key_atom, Component, ComponentAnatomy, ComponentLine, ShortcutByline,
    },
    tokens::TextStyle,
};

/// `done/total` counter (e.g. `16/22`).
pub fn format_fraction(done: usize, total: usize) -> String {
    format!("{done}/{total}")
}

/// Page window counter (e.g. `1-12 of 124`).
/// Uses ASCII hyphen, not an en dash, so snapshots stay pure ASCII where possible.
pub fn format_page_range(start: usize, end: usize, total: usize) -> String {
    format!("{start}-{end} of {total}")
}

/// Live work state for the content status band (never the bottom statusline).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LiveState {
    Running,
    Waiting,
    Complete,
    NeedsAttention,
}

impl LiveState {
    pub const fn marker(self) -> char {
        match self {
            Self::Running => '●',
            Self::Waiting => '○',
            Self::Complete => '✓',
            Self::NeedsAttention => '✗',
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Running => "Running",
            Self::Waiting => "Waiting",
            Self::Complete => "Complete",
            Self::NeedsAttention => "Needs attention",
        }
    }

    /// Live running alone may spend one restrained ACCENT use.
    pub const fn accent_uses(self) -> usize {
        match self {
            Self::Running => 1,
            _ => 0,
        }
    }

    fn style(self) -> TextStyle {
        match self {
            Self::Running => TextStyle::accent(),
            Self::Complete => TextStyle {
                foreground: crate::chrome::tokens::ColorToken::Ok,
                background: None,
                bold: false,
            },
            Self::NeedsAttention => TextStyle {
                foreground: crate::chrome::tokens::ColorToken::Bad,
                background: None,
                bold: false,
            },
            Self::Waiting => TextStyle::muted(),
        }
    }
}

/// Content status band: `● Running · stage            16/22`.
///
/// Renders as body content above panels. Never paints a filled statusline bar.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatusBand {
    state: LiveState,
    stage: Option<String>,
    done: Option<usize>,
    total: Option<usize>,
}

impl StatusBand {
    pub fn new(state: LiveState) -> Self {
        Self {
            state,
            stage: None,
            done: None,
            total: None,
        }
    }

    pub fn with_stage(mut self, stage: impl Into<String>) -> Self {
        self.stage = Some(stage.into());
        self
    }

    pub fn with_fraction(mut self, done: usize, total: usize) -> Result<Self, String> {
        if done > total {
            return Err(format!(
                "status band fraction done ({done}) exceeds total ({total})"
            ));
        }
        self.done = Some(done);
        self.total = Some(total);
        Ok(self)
    }
}

impl Component for StatusBand {
    fn lines(&self) -> Vec<ComponentLine> {
        let mut left = format!("{} {}", self.state.marker(), self.state.label());
        if let Some(stage) = &self.stage {
            if !stage.trim().is_empty() {
                left.push_str(" · ");
                left.push_str(stage.trim());
            }
        }
        if let (Some(done), Some(total)) = (self.done, self.total) {
            let counter = format_fraction(done, total);
            // Prefer a single readable band line; keep within 120 columns via truncate.
            let max = 118usize;
            let combined = format!("{left}  {counter}");
            let text = if combined.chars().count() <= max {
                combined
            } else {
                // Drop stage first, keep live marker + counter.
                format!("{} {}  {counter}", self.state.marker(), self.state.label())
            };
            vec![ComponentLine::new(text, self.state.style())]
        } else {
            vec![ComponentLine::new(left, self.state.style())]
        }
    }

    fn anatomy(&self) -> ComponentAnatomy {
        ComponentAnatomy {
            primary_action_count: 0,
            secondary_action_count: 0,
            has_local_keys: false,
            accent_uses: self.state.accent_uses(),
        }
    }
}

/// Counter form attached to a panel header.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PanelCounter {
    Fraction { done: usize, total: usize },
    PageRange {
        start: usize,
        end: usize,
        total: usize,
    },
    /// Free-form label already vetted by the builder (e.g. `Time 8m 36s`).
    Label(String),
}

impl PanelCounter {
    pub fn render(&self) -> Result<String, String> {
        let text = match self {
            Self::Fraction { done, total } => {
                if *done > *total {
                    return Err(format!(
                        "panel counter done ({done}) exceeds total ({total})"
                    ));
                }
                format_fraction(*done, *total)
            }
            Self::PageRange { start, end, total } => {
                if *start == 0 || *end < *start || *end > *total {
                    return Err(format!(
                        "panel page range is invalid: {start}-{end} of {total}"
                    ));
                }
                format_page_range(*start, *end, *total)
            }
            Self::Label(label) => {
                if label.trim().is_empty() {
                    return Err("panel counter label must not be empty".to_owned());
                }
                if contains_bracket_key_atom(label) {
                    return Err("panel counter label may not use bracket key atoms".to_owned());
                }
                label.trim().to_owned()
            }
        };
        if contains_bracket_key_atom(&text) {
            return Err("panel counter may not use bracket key atoms".to_owned());
        }
        Ok(text)
    }
}

/// Panel header with optional Mission Control counter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PanelHeader {
    title: String,
    counter: Option<PanelCounter>,
}

impl PanelHeader {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            counter: None,
        }
    }

    pub fn with_counter(mut self, counter: PanelCounter) -> Self {
        self.counter = Some(counter);
        self
    }
}

impl Component for PanelHeader {
    fn lines(&self) -> Vec<ComponentLine> {
        let title = self.title.trim();
        let text = if let Some(counter) = &self.counter {
            match counter.render() {
                Ok(value) => format!("{title} · {value}"),
                Err(_) => title.to_owned(),
            }
        } else {
            title.to_owned()
        };
        vec![ComponentLine::new(text, TextStyle::bright())]
    }

    fn anatomy(&self) -> ComponentAnatomy {
        ComponentAnatomy::passive()
    }
}

/// One checklist / milestone row in a selection surface.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectionSurfaceRow {
    label: String,
    /// Functional status glyph (`✓`, `●`, `○`, `✗`) or none.
    status: Option<char>,
}

impl SelectionSurfaceRow {
    pub fn new(label: impl Into<String>, status: Option<char>) -> Self {
        Self {
            label: label.into(),
            status,
        }
    }
}

/// Selection surface that marks focus with `›` and optional status glyphs.
/// Never nests bracket key atoms such as `› [enter] …`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectionSurface {
    rows: Vec<SelectionSurfaceRow>,
    selected_index: usize,
    selected_detail: Option<String>,
}

impl SelectionSurface {
    pub fn new(rows: Vec<SelectionSurfaceRow>, selected_index: usize) -> Result<Self, String> {
        if rows.is_empty() || selected_index >= rows.len() {
            return Err("selection surface requires one selected row".to_owned());
        }
        for row in &rows {
            if row.label.trim().is_empty() {
                return Err("selection surface rows need a label".to_owned());
            }
            if contains_bracket_key_atom(&row.label) {
                return Err("selection surface labels may not use bracket key atoms".to_owned());
            }
            if let Some(glyph) = row.status {
                if !"●○✓✗".contains(glyph) {
                    return Err(format!(
                        "selection surface status glyph must be functional: {glyph}"
                    ));
                }
            }
        }
        Ok(Self {
            rows,
            selected_index,
            selected_detail: None,
        })
    }

    pub fn with_selected_detail(mut self, detail: impl Into<String>) -> Self {
        self.selected_detail = Some(detail.into());
        self
    }
}

impl Component for SelectionSurface {
    fn lines(&self) -> Vec<ComponentLine> {
        let mut lines = Vec::with_capacity(self.rows.len() + 1);
        for (index, row) in self.rows.iter().enumerate() {
            let selected = index == self.selected_index;
            let marker = if selected { "›" } else { " " };
            let body = match row.status {
                Some(glyph) => format!("{marker} {glyph} {}", row.label.trim()),
                None => format!("{marker} {}", row.label.trim()),
            };
            lines.push(ComponentLine::new(
                body,
                if selected {
                    TextStyle::accent()
                } else {
                    TextStyle::text()
                },
            ));
            if selected {
                if let Some(detail) = &self.selected_detail {
                    if !detail.trim().is_empty() {
                        lines.push(ComponentLine::new(
                            format!("  {}", detail.trim()),
                            TextStyle::muted(),
                        ));
                    }
                }
            }
        }
        lines
    }

    fn anatomy(&self) -> ComponentAnatomy {
        ComponentAnatomy {
            primary_action_count: 0,
            secondary_action_count: 0,
            has_local_keys: false,
            accent_uses: 1,
        }
    }
}

/// One compressed event/log fine.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompressedLogLine {
    text: String,
    age: Option<String>,
}

impl CompressedLogLine {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            age: None,
        }
    }

    pub fn with_age(mut self, age: impl Into<String>) -> Self {
        self.age = Some(age.into());
        self
    }
}

/// Progress / event log with optional relative ages and page counter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompressedLog {
    lines: Vec<CompressedLogLine>,
    page: Option<(usize, usize, usize)>,
    /// When true (default), the first line restates the page counter for
    /// standalone use. When embedded under a panel that already shows
    /// `PanelHeader` pagination, builders may suppress the header via
    /// [`Self::without_header`].
    show_header: bool,
}

impl CompressedLog {
    pub fn new(lines: Vec<CompressedLogLine>) -> Self {
        Self {
            lines,
            page: None,
            show_header: true,
        }
    }

    pub fn with_page(mut self, start: usize, end: usize, total: usize) -> Self {
        self.page = Some((start, end, total));
        self
    }

    pub fn without_header(mut self) -> Self {
        self.show_header = false;
        self
    }
}

impl Component for CompressedLog {
    fn lines(&self) -> Vec<ComponentLine> {
        let mut out = Vec::new();
        if self.show_header {
            if let Some((start, end, total)) = self.page {
                out.push(ComponentLine::new(
                    format!("Progress · {}", format_page_range(start, end, total)),
                    TextStyle::muted(),
                ));
            }
        }
        for line in &self.lines {
            let body = line.text.trim();
            let text = if let Some(age) = &line.age {
                format!("  {body} · {}", age.trim())
            } else {
                format!("  {body}")
            };
            out.push(ComponentLine::new(text, TextStyle::muted()));
        }
        out
    }

    fn anatomy(&self) -> ComponentAnatomy {
        ComponentAnatomy::passive()
    }
}

/// Body of a zone: free lines, a selection list, or a compressed log.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ZoneBody {
    Lines(Vec<String>),
    Selection(SelectionSurface),
    Log(CompressedLog),
}

/// One zoned region: header + body.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Zone {
    header: PanelHeader,
    body: ZoneBody,
}

impl Zone {
    pub fn new(header: PanelHeader, body: ZoneBody) -> Self {
        Self { header, body }
    }
}

/// Multi-zone Mission Control panel body (active work | checklist | log).
///
/// Zones render sequentially with a single blank separator between them.
/// Builders place this inside a Temporary detail view or mission layout; the
/// panel itself never invents a floating popup or filled statusline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ZonedPanel {
    zones: Vec<Zone>,
}

impl ZonedPanel {
    pub fn new(zones: Vec<Zone>) -> Self {
        Self { zones }
    }
}

impl Component for ZonedPanel {
    fn lines(&self) -> Vec<ComponentLine> {
        let mut out = Vec::new();
        for (idx, zone) in self.zones.iter().enumerate() {
            if idx > 0 {
                out.push(ComponentLine::new(String::new(), TextStyle::text()));
            }
            out.extend(zone.header.lines());
            match &zone.body {
                ZoneBody::Lines(body) => {
                    for line in body {
                        out.push(ComponentLine::new(line.clone(), TextStyle::text()));
                    }
                }
                ZoneBody::Selection(surface) => out.extend(surface.lines()),
                ZoneBody::Log(log) => {
                    // Suppress nested header so PanelHeader is the single counter source.
                    let lines = if log.show_header {
                        CompressedLog {
                            lines: log.lines.clone(),
                            page: log.page,
                            show_header: false,
                        }
                        .lines()
                    } else {
                        log.lines()
                    };
                    out.extend(lines);
                }
            }
        }
        out
    }

    fn anatomy(&self) -> ComponentAnatomy {
        // Selection surfaces inside zones may spend one accent on the focus row.
        let accent: usize = self
            .zones
            .iter()
            .map(|zone| match &zone.body {
                ZoneBody::Selection(surface) => surface.anatomy().accent_uses,
                _ => 0,
            })
            .sum();
        ComponentAnatomy {
            primary_action_count: 0,
            secondary_action_count: 0,
            has_local_keys: false,
            accent_uses: accent.min(1),
        }
    }
}

/// Mission-scoped local key strip (temporary detail / mission card only).
///
/// Confined to mission space. Must not be painted as a global chat footer (N-9).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MissionLocalKeyStrip {
    byline: ShortcutByline,
}

impl MissionLocalKeyStrip {
    pub fn new(byline: ShortcutByline) -> Self {
        Self { byline }
    }
}

impl Component for MissionLocalKeyStrip {
    fn lines(&self) -> Vec<ComponentLine> {
        vec![self.byline.as_component_line(TextStyle::muted())]
    }

    fn anatomy(&self) -> ComponentAnatomy {
        ComponentAnatomy {
            primary_action_count: 0,
            secondary_action_count: 0,
            has_local_keys: true,
            accent_uses: 0,
        }
    }
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn fraction_and_page_helpers() {
        assert_eq!(format_fraction(16, 22), "16/22");
        assert_eq!(format_page_range(1, 12, 124), "1-12 of 124");
    }

    #[test]
    fn status_band_rejects_invalid_fraction() {
        assert!(StatusBand::new(LiveState::Running)
            .with_fraction(5, 3)
            .is_err());
    }

    #[test]
    fn selection_rejects_bracketed_labels() {
        assert!(SelectionSurface::new(
            vec![SelectionSurfaceRow::new("[enter] Apply", Some('●'))],
            0
        )
        .is_err());
    }
}
