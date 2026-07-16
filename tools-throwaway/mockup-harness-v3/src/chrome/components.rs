//! Shared screen anatomy for the v3 conversation surfaces.
//!
//! These types deliberately model readable user-facing content, local keys,
//! focus, and a single primary action. Screens compose them instead of
//! hand-rolling detached pickers, recovery cards, or mission dashboards.
//!
//! Shortcut and return copy uses Claude-style prose bylines
//! (`Enter to apply · Esc to cancel`) — never bracket key-chord atoms.
//!
//! Cortex foundation chrome (F-11..F-14 / N-14..N-15):
//! - [`PromptJewel`] — single rounded-border idle composer
//! - [`TurnStatusBand`] — busy-only band between transcript and jewel
//! - [`user_turn_surface_strip`] — user-turn surface with closed `›`
//! - outer pad helpers live in `chrome::layout` (`OUTER_HPAD`, `with_outer_hpad`)

use crate::{
    chrome::{
        layout::OUTER_CONTENT_WIDTH,
        tokens::{validate_glyphs, ColorToken, TextStyle, ACCENT_USE_BUDGET},
    },
    Surface,
};

/// Closed prompt prefix glyph. Cortex uses `❯`; Cortex maps to `›` only.
pub const PROMPT_PREFIX: char = '›';

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

/// True when paint shows a primary Enter control in **either** Claude prose form
/// (`Enter to …`) or residual Phase-B bracket form (`[enter] …` / `› [enter] …`).
///
/// Prefer prose-only assertions on rewritten Phase-A surfaces. Use this dual-branch
/// helper only when the screen set still includes true Phase-B residuals.
pub fn has_enter_control(text: &str) -> bool {
    text.contains("Enter to") || text.contains("[enter]") || text.contains("› [enter]")
}

/// True when paint shows a local Esc/cancel control as prose (`Esc to …`) or
/// residual Phase-B brackets (`[esc] …` / `[esc] Cancel` / `[esc] Back`).
pub fn has_esc_control(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    text.contains("Esc to")
        || text.contains("[esc]")
        || lower.contains("esc to")
        || lower.contains("[esc]")
}

/// Primary action label check that accepts:
/// - Phase-A dock: `› {label}` and/or `Enter to {lowercased label…}`
/// - Phase-B residual: `[enter] {label}` / `› [enter] {label}`
///
/// `label` is the human action noun as painted on the primary row (e.g. "Retry reply",
/// "Apply model", "Fork", "Finish"). Matching is substring-based against visible TXT.
pub fn has_primary_action_label(text: &str, label: &str) -> bool {
    let label = label.trim();
    if label.is_empty() {
        return false;
    }
    let lower_label = noun_clause(label);
    text.contains(&format!("› {label}"))
        || text.contains(&format!("Enter to {lower_label}"))
        || text.contains(&format!("Enter to {label}"))
        || text.contains(&format!("[enter] {label}"))
        || text.contains(&format!("› [enter] {label}"))
        // Prose bylines may keep label casing after "Enter to" depending on constructor.
        || text
            .to_ascii_lowercase()
            .contains(&format!("enter to {}", label.to_ascii_lowercase()))
}

/// Local secondary key clause that accepts Phase-A prose (`c for Continue…`) or
/// residual Phase-B brackets (`[c] Continue…`).
pub fn has_secondary_key_clause(text: &str, key: &str, label_fragment: &str) -> bool {
    let key = key.trim();
    let fragment = label_fragment.trim();
    if key.is_empty() || fragment.is_empty() {
        return false;
    }
    text.contains(&format!("[{key}] {fragment}"))
        || text.contains(&format!("{key} for {fragment}"))
        || text.contains(fragment)
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
        "Work configuration · theme choice" => "Theme choice",
        "Provider and MCP · connect" | "Provider and MCP · connections" => "Connections",
        // Flow 4 provider origin labels (painted subject titles; audit keeps journey tags).
        "Provider connection · blocked model action" => "Connections",
        "Provider connection · onboarding" => "First-run setup",
        "Provider connection · conversation" => "Current conversation",
        "MCP · conversation capabilities" => "MCP capabilities",
        "Recovery · paused conversation" => "Recovery",
        "Mission · current conversation" => "Mission",
        "Start · empty conversation" => "Empty conversation",
        // Already-human subjects and unknown custom titles pass through via a small set
        // of known fixtures; callers with non-static copy should paint their own subject.
        "Current conversation" => "Current conversation",
        "Paused conversation" => "Paused conversation",
        "Work configuration" => "Work configuration",
        "Theme choice" => "Theme choice",
        "First-run setup" => "First-run setup",
        "Connections" => "Connections",
        "MCP capabilities" => "MCP capabilities",
        "Empty conversation" => "Empty conversation",
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
                        "theme choice" => "Theme choice",
                        "blocked model action" => "Connections",
                        "conversation capabilities" => "MCP capabilities",
                        other if other.eq_ignore_ascii_case("connect") => "Connections",
                        _ => "Conversation",
                    };
                }
            }
            "Conversation"
        }
        // Provider-connection family is not in the historical FORBIDDEN prefix list but
        // still functions as a painted journey crumb — strip to a subject.
        _ if trimmed.starts_with("Provider connection ·") => match trimmed {
            "Provider connection · blocked model action" => "Connections",
            "Provider connection · onboarding" => "First-run setup",
            "Provider connection · conversation" => "Current conversation",
            _ => "Connections",
        },
        _ if trimmed.starts_with("MCP ·") => "MCP capabilities",
        _ => "Conversation",
    }
}

/// Closed prompt prefix used by jewels and user-turn strips (Cortex `❯` → `›`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PromptPrefix;

impl PromptPrefix {
    pub const fn closed() -> Self {
        Self
    }

    pub const fn as_char(self) -> char {
        PROMPT_PREFIX
    }
}

/// Map a Cortex-source glyph into the Cortex closed set, when one exists.
///
/// Unmapped glyphs return `None` so callers fail closed rather than inventing art.
pub fn map_product_glyph(glyph: char) -> Option<char> {
    match glyph {
        '❯' | '›' => Some(PROMPT_PREFIX),
        '◆' | '●' => Some('●'),
        '◇' | '○' => Some('○'),
        other if "✓ ✗ → ↑ ↓ ─ │ ╭ ╮ ╰ ╯ █ ░ · ⎿ ▸ ▾ …".contains(other) => Some(other),
        _ => None,
    }
}

/// True when `text` matches Cortex's always-on putty `Key: verb` footer language
/// (VAL-V3-DS-037 / N-14). Claude prose bylines do **not** match.
pub fn contains_global_putty_footer(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    if lower.contains("shift+tab:")
        || lower.contains("ctrl+c:")
        || lower.contains("ctrl+.:")
        || lower.contains("ctrl+.;")
    {
        return true;
    }
    // Permanent filmstrip: multiple `Key: verb` segments separated by `│`.
    if text.contains('│') {
        let segments: Vec<&str> = text.split('│').map(str::trim).filter(|s| !s.is_empty()).collect();
        if segments.len() >= 2 {
            let looks_putty = segments.iter().filter(|seg| {
                let s = seg.to_ascii_lowercase();
                (s.contains("ctrl+") || s.contains("shift+") || s.contains("alt+") || s.contains("meta+"))
                    && s.contains(':')
            }).count();
            if looks_putty >= 2 {
                return true;
            }
        }
    }
    false
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

/// Cortex-like user-turn surface strip with a single closed `›` prefix and SURFACE wash
/// (VAL-V3-DS-034 / F-14). Decorative stars and open Cortex `❯` are forbidden.
pub fn user_turn_surface_strip(text: &str) -> Result<ComponentLine, String> {
    let body = text.trim();
    if body.is_empty() {
        return Err("user turn surface strip requires text".to_owned());
    }
    if contains_bracket_key_atom(body) {
        return Err("user turn surface strip may not use bracket key atoms".to_owned());
    }
    if is_journey_breadcrumb_title(body) {
        return Err("user turn surface strip may not be a journey breadcrumb title".to_owned());
    }
    if body.contains('❯') || body.contains('✦') || body.contains('✧') || body.contains('◆') {
        return Err("user turn surface strip may not use forbidden Cortex decorative glyphs".to_owned());
    }
    validate_glyphs(body)?;
    // One closed prefix only; if the caller already led with ›, do not double it.
    let painted = if body.starts_with('›') {
        format!(" {body} ")
    } else {
        format!(" {PROMPT_PREFIX} {body} ")
    };
    validate_glyphs(&painted)?;
    Ok(ComponentLine::new(painted, TextStyle::user_turn()))
}

/// Idle conversation composer as a single Cortex-inspired rounded prompt jewel
/// (VAL-V3-DS-031 / F-11).
///
/// Geometry:
/// ```text
/// ╭──────────────────────────────────────────────╮
/// │ › draft / placeholder                         │
/// │   Claude Opus 4.8 · Work mode                 │  ← muted info
/// ╰──────────────────────────────────────────────╯
/// ```
///
/// No status strap fill, no debug meta, no permanent putty footer. Copper
/// accent is reserved for the prompt prefix (and optional focused emphasis).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PromptJewel {
    draft: String,
    model: String,
    work_mode: String,
    focused: bool,
    width: u16,
    /// Optional content-meta prefix for multi-line jewels (`Draft · 3 lines`).
    draft_meta: Option<String>,
    /// When true, draft newlines/indents expand into interior body rows.
    multiline: bool,
}

impl PromptJewel {
    pub fn new(draft: &str, model: &str, work_mode: &str) -> Result<Self, String> {
        let draft = draft.trim().to_owned();
        let model = model.trim().to_owned();
        let work_mode = work_mode.trim().to_owned();
        if draft.is_empty() {
            return Err("prompt jewel requires a draft or placeholder".to_owned());
        }
        if model.is_empty() || work_mode.is_empty() {
            return Err("prompt jewel requires a human model name and work mode".to_owned());
        }
        // Reject raw identifier-style model IDs (e.g. claude-opus-4-8).
        // Human names like "Claude Opus 4.8" contain spaces and pass.
        let looks_raw_id = model.contains('/')
            || (!model.contains(' ')
                && model.contains('-')
                && model
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.'));
        if looks_raw_id {
            return Err(format!(
                "prompt jewel model must be a human-readable display name: {model}"
            ));
        }
        reject_composer_debug_meta(&draft)?;
        reject_composer_debug_meta(&model)?;
        reject_composer_debug_meta(&work_mode)?;
        if draft.contains('❯') || model.contains('❯') || work_mode.contains('❯') {
            return Err("prompt jewel may not use open Cortex ❯; map to ›".to_owned());
        }
        if contains_global_putty_footer(&draft)
            || contains_global_putty_footer(&model)
            || contains_global_putty_footer(&work_mode)
        {
            return Err("prompt jewel may not embed putty footer copy".to_owned());
        }
        let info = format!("{model} · {work_mode}");
        validate_glyphs(&draft)?;
        validate_glyphs(&info)?;
        Ok(Self {
            draft,
            model,
            work_mode,
            focused: false,
            width: OUTER_CONTENT_WIDTH,
            draft_meta: None,
            multiline: false,
        })
    }

    /// Multi-line jewel: newlines become distinct body rows; a muted
    /// `Draft · N lines` meta sits above the prefix draft. Info line still
    /// holds human model · work mode (composer-multiline / VAL-V3-DS-036).
    pub fn multiline(mut self, authored_line_count: usize) -> Result<Self, String> {
        let meta = composer_header_meta(authored_line_count)?;
        reject_composer_debug_meta(&meta)?;
        validate_glyphs(&meta)?;
        self.draft_meta = Some(meta);
        self.multiline = true;
        Ok(self)
    }

    /// Marks the jewel as focused so the interior prefix may spend ACCENT.
    pub fn focused(mut self) -> Self {
        self.focused = true;
        self
    }

    pub fn width(&self) -> u16 {
        self.width
    }

    pub fn info_line(&self) -> String {
        format!("{} · {}", self.model, self.work_mode)
    }

    pub fn draft_meta(&self) -> Option<&str> {
        self.draft_meta.as_deref()
    }

    /// Join jewel rows with newlines for TXT-snapshot tests.
    pub fn render_txt(&self) -> String {
        self.lines()
            .into_iter()
            .map(|line| line.text)
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn pad_inner(content: &str, inner_width: usize) -> String {
        let mut chars: Vec<char> = content.chars().collect();
        if chars.len() > inner_width {
            chars.truncate(inner_width.saturating_sub(1));
            chars.push('…');
        }
        let mut out: String = chars.into_iter().collect();
        while out.chars().count() < inner_width {
            out.push(' ');
        }
        out
    }

    fn render_lines(&self) -> Vec<ComponentLine> {
        let width = self.width as usize;
        let rail = "─".repeat(width.saturating_sub(2));
        let top = format!("╭{rail}╮");
        let bottom = format!("╰{rail}╯");
        // Interior content width between the two `│` side walls.
        let inner = width.saturating_sub(2);
        let mut body: Vec<ComponentLine> = Vec::new();

        if let Some(meta) = &self.draft_meta {
            let meta_body = Self::pad_inner(meta, inner.saturating_sub(1));
            body.push(ComponentLine::new(
                Self::pad_to_width(&format!("│ {meta_body}"), width),
                TextStyle::bright(),
            ));
        }

        if self.multiline {
            for (index, part) in self.draft.split('\n').enumerate() {
                let content = if index == 0 {
                    format!("{PROMPT_PREFIX} {part}")
                } else {
                    format!("  {part}")
                };
                let draft_body = Self::pad_inner(&content, inner.saturating_sub(1));
                let style = if index == 0 {
                    TextStyle::bright()
                } else if part.trim_start().starts_with("include ") || part.starts_with("    ") {
                    TextStyle::muted()
                } else {
                    TextStyle::text()
                };
                body.push(ComponentLine::new(
                    Self::pad_to_width(&format!("│ {draft_body}"), width),
                    style,
                ));
            }
        } else {
            let draft_body =
                Self::pad_inner(&format!("{PROMPT_PREFIX} {}", self.draft), inner.saturating_sub(1));
            // Leading space after `│` so `│ › draft…` reads as jewel.
            let draft_row = Self::pad_to_width(&format!("│ {draft_body}"), width);
            body.push(ComponentLine::new(draft_row, TextStyle::bright()));
        }

        let info_body = Self::pad_inner(&format!("  {}", self.info_line()), inner.saturating_sub(1));
        let info_row = Self::pad_to_width(&format!("│ {info_body}"), width);

        // Borders stay muted/transparent; accent budget lives on the focused prefix
        // only (tracked via anatomy.accent_uses), never a filled strap.
        let mut lines = vec![ComponentLine::new(top, TextStyle::muted())];
        lines.extend(body);
        lines.push(ComponentLine::new(info_row, TextStyle::muted()));
        lines.push(ComponentLine::new(bottom, TextStyle::muted()));
        lines
    }

    fn pad_to_width(row: &str, width: usize) -> String {
        let mut out = row.to_owned();
        // Ensure right wall is present and total char width matches.
        if !out.ends_with('│') {
            while out.chars().count() < width.saturating_sub(1) {
                out.push(' ');
            }
            out.push('│');
        }
        let mut chars: Vec<char> = out.chars().collect();
        if chars.len() > width {
            chars.truncate(width.saturating_sub(1));
            chars.push('│');
        }
        while chars.len() < width {
            // Insert space before the trailing wall.
            if let Some(last) = chars.last().copied() {
                if last == '│' {
                    chars.pop();
                    chars.push(' ');
                    chars.push('│');
                    continue;
                }
            }
            chars.push(' ');
        }
        chars.into_iter().collect()
    }
}

impl Component for PromptJewel {
    fn lines(&self) -> Vec<ComponentLine> {
        self.render_lines()
    }

    fn anatomy(&self) -> ComponentAnatomy {
        ComponentAnatomy {
            primary_action_count: 0,
            secondary_action_count: 0,
            has_local_keys: false,
            // Copper accent on the prompt prefix only when focused is true; still 1.
            accent_uses: if self.focused { 1 } else { 0 },
        }
    }
}

/// Live activity kind for the turn-status band between transcript and jewel.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TurnBusyKind {
    Responding,
    Running,
}

impl TurnBusyKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Responding => "Responding",
            Self::Running => "Running",
        }
    }

    pub const fn marker(self) -> char {
        '●'
    }
}

/// One-row turn-status band between transcript and jewel when busy; collapses
/// empty when idle (VAL-V3-DS-032 / F-12). Not a filled footer strap.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TurnStatusBand {
    kind: Option<TurnBusyKind>,
    timer: Option<String>,
    detail: Option<String>,
    stop_verb: Option<String>,
}

impl TurnStatusBand {
    /// Idle band occupies zero rows and emits no paint.
    pub fn idle() -> Self {
        Self {
            kind: None,
            timer: None,
            detail: None,
            stop_verb: None,
        }
    }

    pub fn busy(kind: TurnBusyKind, timer: &str) -> Result<Self, String> {
        let timer = timer.trim().to_owned();
        if timer.is_empty() {
            return Err("busy turn-status band requires a timer label".to_owned());
        }
        if contains_bracket_key_atom(&timer) {
            return Err("turn-status timer may not use bracket key atoms".to_owned());
        }
        validate_glyphs(&timer)?;
        Ok(Self {
            kind: Some(kind),
            timer: Some(timer),
            detail: None,
            stop_verb: None,
        })
    }

    pub fn with_detail(mut self, detail: &str) -> Self {
        let trimmed = detail.trim();
        if !trimmed.is_empty() {
            self.detail = Some(trimmed.to_owned());
        }
        self
    }

    /// Stop is always prose (`Esc to stop`), never `[stop]`.
    pub fn with_stop(mut self, verb: &str) -> Result<Self, String> {
        let verb = verb.trim();
        if verb.is_empty() {
            return Err("turn-status stop verb requires text".to_owned());
        }
        if contains_bracket_key_atom(verb) || verb.contains('[') {
            return Err("turn-status stop must be prose, never bracket atoms".to_owned());
        }
        self.stop_verb = Some(verb.to_owned());
        Ok(self)
    }

    pub fn is_idle(&self) -> bool {
        self.kind.is_none()
    }

    /// Busy paints one row; idle collapses to height 0.
    pub fn height(&self) -> u16 {
        if self.is_idle() {
            0
        } else {
            1
        }
    }

    pub fn render_txt(&self) -> String {
        self.lines()
            .into_iter()
            .map(|line| line.text)
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn render_lines(&self) -> Result<Vec<ComponentLine>, String> {
        let Some(kind) = self.kind else {
            return Ok(Vec::new());
        };
        let mut parts = vec![format!("{} {}", kind.marker(), kind.label())];
        if let Some(detail) = &self.detail {
            reject_composer_debug_meta(detail)?;
            validate_glyphs(detail)?;
            if contains_bracket_key_atom(detail) {
                return Err("turn-status detail may not use bracket key atoms".to_owned());
            }
            parts.push(detail.clone());
        }
        if let Some(timer) = &self.timer {
            parts.push(timer.clone());
        }
        if let Some(verb) = &self.stop_verb {
            let byline = byline_or_empty(
                ShortcutHint::new("Esc", verb).and_then(|hint| ShortcutByline::new(vec![hint])),
            );
            if byline.is_empty() {
                return Err("turn-status stop byline failed to render".to_owned());
            }
            if byline.contains('[') {
                return Err("turn-status stop byline leaked a bracket".to_owned());
            }
            parts.push(byline);
        }
        let text = parts.join(" · ");
        if contains_global_putty_footer(&text) {
            return Err("turn-status must not emit a putty footer".to_owned());
        }
        if text.contains("[stop]") {
            return Err("turn-status must not emit [stop]".to_owned());
        }
        validate_glyphs(&text)?;
        // Transparent canvas — never a filled footer strap (N-1 / F-12).
        Ok(vec![ComponentLine::new(text, TextStyle::text())])
    }
}

impl Component for TurnStatusBand {
    fn lines(&self) -> Vec<ComponentLine> {
        self.render_lines()
            .unwrap_or_else(|_| Vec::new())
    }

    fn anatomy(&self) -> ComponentAnatomy {
        ComponentAnatomy {
            primary_action_count: 0,
            secondary_action_count: 0,
            has_local_keys: self.stop_verb.is_some(),
            accent_uses: if self.is_idle() { 0 } else { 1 },
        }
    }
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
    if token.is_empty() || token.len() > 24 {
        return false;
    }
    // File/content chips such as `[attachment: path]` are not key chords.
    if token.contains(':') || token.contains(' ') || token.contains('/') {
        return false;
    }
    let lower = token.to_ascii_lowercase();
    // Any modifier+key chord inside brackets is banned (N-11), not only the
    // small historical ctrl+k/g/c/d denylist. Covers [ctrl+u], [alt+enter],
    // [shift+tab], [meta+k], multi-modifier chords, etc.
    if is_modifier_key_chord(&lower) {
        return true;
    }
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
    ) || token
        .chars()
        .all(|c| "↑↓←→".contains(c) && !c.is_whitespace())
        || (token.len() <= 3
            && token
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '^'))
}

/// True for tokens like `ctrl+u`, `alt+enter`, `shift+tab`, `meta+k`, `cmd+s`,
/// or multi-modifier forms such as `ctrl+shift+p`. Plain content tokens and
/// bare keys were already handled by `is_key_chord_token`.
fn is_modifier_key_chord(lower: &str) -> bool {
    const MODIFIERS: &[&str] = &[
        "ctrl", "control", "alt", "option", "shift", "meta", "cmd", "super",
    ];
    let parts: Vec<&str> = lower.split('+').collect();
    if parts.len() < 2 {
        return false;
    }
    let (mods, key_parts) = parts.split_at(parts.len() - 1);
    let key = key_parts[0];
    if key.is_empty() || !mods.iter().all(|m| MODIFIERS.contains(m)) {
        return false;
    }
    // Key side: short alphanumeric chord key or a named control key / arrow.
    key.chars().all(|c| c.is_ascii_alphanumeric() || "↑↓←→".contains(c))
        || matches!(
            key,
            "enter"
                | "esc"
                | "escape"
                | "tab"
                | "space"
                | "return"
                | "up"
                | "down"
                | "left"
                | "right"
                | "home"
                | "end"
                | "pgup"
                | "pgdn"
                | "pageup"
                | "pagedown"
                | "del"
                | "delete"
                | "backspace"
        )
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

/// Present/past tool verbs for the Cortex-informed activity stream (VAL-V3-DS-033).
///
/// Caller supplies the object phrase (`project code`, `session rules`); the
/// component conjugating creates `Searching project code` / `Searched project code`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolVerb {
    Read,
    Search,
    Run,
    Edit,
    Write,
    Shell,
    Task,
    Respond,
}

impl ToolVerb {
    pub const fn present(self) -> &'static str {
        match self {
            Self::Read => "Reading",
            Self::Search => "Searching",
            Self::Run => "Running",
            Self::Edit => "Editing",
            Self::Write => "Writing",
            Self::Shell => "Running",
            Self::Task => "Working on",
            Self::Respond => "Writing",
        }
    }

    pub const fn past(self) -> &'static str {
        match self {
            Self::Read => "Read",
            Self::Search => "Searched",
            Self::Run => "Ran",
            Self::Edit => "Edited",
            Self::Write => "Wrote",
            Self::Shell => "Ran",
            Self::Task => "Finished",
            Self::Respond => "Wrote",
        }
    }

    /// Compose present/past + object into the user-visible tool header.
    pub fn phrase(self, object: &str, past: bool) -> String {
        let verb = if past { self.past() } else { self.present() };
        let object = object.trim();
        if object.is_empty() {
            verb.to_owned()
        } else {
            format!("{verb} {object}")
        }
    }
}

/// Elbow disclosure line under a tool row (`⎿ detail`), closed glyph only.
pub fn tool_elbow_line(detail: &str) -> Result<ComponentLine, String> {
    let detail = detail.trim();
    if detail.is_empty() {
        return Err("tool elbow requires detail".to_owned());
    }
    if contains_bracket_key_atom(detail) {
        return Err("tool elbow may not use bracket key atoms".to_owned());
    }
    let text = format!("  ⎿ {detail}");
    validate_glyphs(&text)?;
    Ok(ComponentLine::new(text, TextStyle::muted()))
}

/// Collapsed fold-group summary such as `Read 3 files` (VAL-V3-DS-033).
pub fn tool_fold_summary(
    verb: ToolVerb,
    count: u32,
    unit: &str,
) -> Result<ComponentLine, String> {
    if count == 0 {
        return Err("tool fold summary requires a non-zero count".to_owned());
    }
    let unit = unit.trim();
    if unit.is_empty() {
        return Err("tool fold summary requires a unit".to_owned());
    }
    let text = format!("✓ {} {count} {unit}", verb.past());
    validate_glyphs(&text)?;
    Ok(ComponentLine::new(
        text,
        TextStyle {
            foreground: ColorToken::Ok,
            background: None,
            bold: false,
        },
    ))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActivityRow {
    label: String,
    state: ActivityState,
    detail: String,
    elapsed: Option<String>,
    interrupt: Option<String>,
    compact: bool,
    /// When true, the label already carries present/past verb tense so the
    /// state noun (`Running`/`Complete`) is omitted from the header (DS-033).
    verb_first: bool,
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
            verb_first: false,
        }
    }

    /// Build a Cortex-style verb-first tool row from a typed verb + object.
    ///
    /// Running/waiting use present tense; success/failure/expanded use past.
    pub fn tool(verb: ToolVerb, object: &str, state: ActivityState, detail: &str) -> Self {
        let past = matches!(
            state,
            ActivityState::Succeeded | ActivityState::Failed | ActivityState::Expanded
        );
        Self {
            label: verb.phrase(object, past),
            state,
            detail: detail.to_owned(),
            elapsed: None,
            interrupt: None,
            compact: false,
            verb_first: true,
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

    /// Mark the label as already verb-conjugated (omit Running/Complete noun).
    pub fn verb_first(mut self) -> Self {
        self.verb_first = true;
        self
    }
}

impl Component for ActivityRow {
    fn lines(&self) -> Vec<ComponentLine> {
        let mut summary = if self.verb_first {
            format!("{} {}", self.state.marker(), self.label)
        } else {
            format!(
                "{} {} · {}",
                self.state.marker(),
                self.label,
                self.state.label()
            )
        };
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

/// Canonical bottom-dock picker row (VAL-V3-DS-036).
///
/// Anatomy is always `title · tradeoff` (or title alone when tradeoff is empty),
/// with focus marked by the restrained `›` glyph and ACCENT text — never a filled
/// selection strap. Selected tradeoffs may also paint under the title as a muted
/// byline when `expanded` is true (multi-line catalog rows).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PickerRow {
    title: String,
    tradeoff: String,
    selected: bool,
    /// When true, tradeoff sits on its own muted secondary line (catalog rows).
    /// When false, the row is a single `title · tradeoff` line (compact docks).
    expanded: bool,
}

impl PickerRow {
    pub fn new(title: &str, tradeoff: &str) -> Result<Self, String> {
        let title = title.trim().to_owned();
        let tradeoff = tradeoff.trim().to_owned();
        if title.is_empty() {
            return Err("picker row requires a subject title".to_owned());
        }
        if is_journey_breadcrumb_title(&title) {
            return Err(format!(
                "picker row title must not be a journey breadcrumb: {title}"
            ));
        }
        if contains_bracket_key_atom(&title) || contains_bracket_key_atom(&tradeoff) {
            return Err("picker row may not use bracket key atoms".to_owned());
        }
        if contains_global_putty_footer(&title) || contains_global_putty_footer(&tradeoff) {
            return Err("picker row may not embed putty footer copy".to_owned());
        }
        validate_glyphs(&title)?;
        validate_glyphs(&tradeoff)?;
        Ok(Self {
            title,
            tradeoff,
            selected: false,
            expanded: false,
        })
    }

    pub fn selected(mut self) -> Self {
        self.selected = true;
        self
    }

    pub fn expanded(mut self) -> Self {
        self.expanded = true;
        self
    }

    /// Single interior canvas line used by compact dock lists: `› Title · tradeoff`.
    pub fn compact_line(&self) -> String {
        let marker = if self.selected { "›" } else { " " };
        if self.tradeoff.is_empty() {
            format!("{marker} {}", self.title)
        } else {
            format!("{marker} {} · {}", self.title, self.tradeoff)
        }
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn tradeoff(&self) -> &str {
        &self.tradeoff
    }

    pub fn is_selected(&self) -> bool {
        self.selected
    }
}

impl Component for PickerRow {
    fn lines(&self) -> Vec<ComponentLine> {
        let marker = if self.selected { "›" } else { " " };
        let title_style = if self.selected {
            TextStyle::accent()
        } else {
            TextStyle::text()
        };
        if self.expanded {
            let mut out = vec![ComponentLine::new(
                format!("{marker} {}", self.title),
                title_style,
            )];
            if !self.tradeoff.is_empty() {
                out.push(ComponentLine::new(
                    format!("  {}", self.tradeoff),
                    TextStyle::muted(),
                ));
            }
            out
        } else {
            vec![ComponentLine::new(self.compact_line(), title_style)]
        }
    }

    fn anatomy(&self) -> ComponentAnatomy {
        ComponentAnatomy {
            primary_action_count: 0,
            secondary_action_count: 0,
            has_local_keys: false,
            accent_uses: if self.selected { 1 } else { 0 },
        }
    }
}

/// Canonical DS-036 picker byline: `Enter to apply · Esc to cancel · ↑↓ to choose`.
///
/// Callers may specialize the Enter verb (`confirm`, `run`, `attach`, …) while
/// keeping the shared Esc + navigate clauses.
pub fn picker_apply_byline(enter_verb: &str) -> Result<String, String> {
    let byline = ShortcutByline::enter_esc_choose(enter_verb, "cancel")?.render();
    if contains_bracket_key_atom(&byline) {
        return Err("picker byline may not emit bracket key atoms".to_owned());
    }
    if contains_global_putty_footer(&byline) {
        return Err("picker byline may not look like a putty footer".to_owned());
    }
    Ok(byline)
}

/// Mild fuzzy-match paint spans: Info (not copper) for matched characters so
/// accent budget stays free for the selected row marker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FuzzyMatchSpan {
    pub text: String,
    pub matched: bool,
}

/// Split `candidate` into contiguous matched / unmatched runs under a fuzzy
/// subsequence against `query` (case-insensitive). Empty query yields one
/// unmatched span of the full candidate.
pub fn fuzzy_match_spans(candidate: &str, query: &str) -> Vec<FuzzyMatchSpan> {
    let fragment = query.trim_start_matches(['@', '/']).to_ascii_lowercase();
    if fragment.is_empty() {
        return vec![FuzzyMatchSpan {
            text: candidate.to_owned(),
            matched: false,
        }];
    }
    let wanted: Vec<char> = fragment.chars().collect();
    let mut want_idx = 0usize;
    let mut spans: Vec<FuzzyMatchSpan> = Vec::new();
    let mut buf = String::new();
    let mut matched_mode: Option<bool> = None;
    for ch in candidate.chars() {
        let is_match = want_idx < wanted.len() && ch.to_ascii_lowercase() == wanted[want_idx];
        if is_match {
            want_idx += 1;
        }
        match matched_mode {
            Some(mode) if mode == is_match => buf.push(ch),
            Some(_) => {
                spans.push(FuzzyMatchSpan {
                    text: std::mem::take(&mut buf),
                    matched: matched_mode.unwrap_or(false),
                });
                buf.push(ch);
                matched_mode = Some(is_match);
            }
            None => {
                buf.push(ch);
                matched_mode = Some(is_match);
            }
        }
    }
    if !buf.is_empty() {
        spans.push(FuzzyMatchSpan {
            text: buf,
            matched: matched_mode.unwrap_or(false),
        });
    }
    if spans.is_empty() {
        spans.push(FuzzyMatchSpan {
            text: candidate.to_owned(),
            matched: false,
        });
    }
    spans
}

/// Style for a mild fuzzy highlight: Info for match runs, plain Text otherwise.
pub fn fuzzy_match_style(matched: bool, selected_row: bool) -> TextStyle {
    if matched {
        TextStyle {
            foreground: ColorToken::Info,
            background: None,
            bold: false,
        }
    } else if selected_row {
        TextStyle::accent()
    } else {
        TextStyle::text()
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
