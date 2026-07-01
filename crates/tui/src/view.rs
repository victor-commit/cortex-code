//! Shared full-screen view chrome for the Wave-2 views (status/timeline/queue/mcp/diff/help/
//! review).
//!
//! Every full-screen view wraps its body in the same chrome so the surfaces read consistently:
//! - row 0: the wordmark header `✦ Cortex Code · <crumb>` (the crumb is the slash command that
//!   opened the view),
//! - row 1: a full-width horizontal rule,
//! - the body region (rows 2..H-2), wrapped by [`render_panel`],
//! - row H-1: the shared 3-zone status bar (reused from the conversation chrome).
//!
//! It also exposes the small layout helpers the view bodies build on: [`kv_line`] (a label/value
//! row with a fixed value column), [`pbar_spans`] (a full-cell `█`/`░` progress bar), and
//! [`empty_state_line`] (the faint placeholder a data view shows when it has nothing to list).
//!
//! Colors come only from the active theme tokens (mono-orange on near-black by default) and glyphs
//! only from [`crate::glyphs`], so the chrome stays brand-pure and tofu-free across every view.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

use tui_components::style::ThemeColors;

use crate::app::App;
use crate::glyphs;
use crate::ui::{cells_to_line, place_segments, status_bar_line};

/// Left/right content margin for full-screen views: the panel is inset this many columns from each
/// edge (matching the conversation content margin and the MOTD card frame).
pub const VIEW_MARGIN: u16 = 2;

/// A full-screen Wave-2 view, each mapped to the slash-command crumb that opens it and the title of
/// the panel wrapping its body.
///
/// The variants beyond the ones already wired (`Status`/`Mcp`) are the extension points the
/// remaining Wave-2 view features (timeline/queue/diff/help/review) build their bodies on, reusing
/// the same chrome so the shared status bar, crumb, and brand palette stay consistent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewKind {
    /// Runtime session status (`/status`).
    Status,
    /// Session timeline (`/timeline`).
    Timeline,
    /// Queued-prompt manager (`/queue`).
    Queue,
    /// MCP servers status (`/mcp`).
    Mcp,
    /// Multi-file diff viewer (`/diff`).
    Diff,
    /// Command reference (`/help`).
    Help,
    /// Code review findings (`/review`).
    Review,
}

impl ViewKind {
    /// Every view kind, in catalog order.
    pub const ALL: &'static [ViewKind] = &[
        ViewKind::Status,
        ViewKind::Timeline,
        ViewKind::Queue,
        ViewKind::Mcp,
        ViewKind::Diff,
        ViewKind::Help,
        ViewKind::Review,
    ];

    /// The breadcrumb shown after `Cortex Code ·` — equal to the slash command that opens the view.
    #[must_use]
    pub const fn crumb(self) -> &'static str {
        match self {
            ViewKind::Status => "/status",
            ViewKind::Timeline => "/timeline",
            ViewKind::Queue => "/queue",
            ViewKind::Mcp => "/mcp",
            ViewKind::Diff => "/diff",
            ViewKind::Help => "/help",
            ViewKind::Review => "/review",
        }
    }

    /// The title embedded in the top border of the body panel.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            ViewKind::Status => "Session status",
            ViewKind::Timeline => "Session timeline",
            ViewKind::Queue => "Prompt queue",
            ViewKind::Mcp => "MCP servers",
            ViewKind::Diff => "Changes",
            ViewKind::Help => "Commands & shortcuts",
            ViewKind::Review => "Code review",
        }
    }
}

/// Render the shared full-screen view chrome into `area` and return the body region between the rule
/// and the status bar.
///
/// Draws the near-black brand canvas across the whole area (so no cell falls through to
/// terminal-default black), the `✦ Cortex Code · <crumb>` header on row 0, a full-width rule on row
/// 1, and the shared 3-zone status bar on the bottom row. The returned rect spans the full width for
/// rows `2..H-2`; callers inset it with [`VIEW_MARGIN`] before wrapping it in [`render_panel`].
pub fn render_chrome(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    crumb: &str,
    theme: &ThemeColors,
) -> Rect {
    if area.width == 0 || area.height == 0 {
        return area;
    }

    // The whole surface is repainted so an overlaid view fully replaces the conversation beneath it
    // and every cell carries the explicit brand background.
    frame.render_widget(Clear, area);
    frame.render_widget(
        Block::default().style(Style::default().bg(theme.background)),
        area,
    );

    let width = area.width as usize;

    // Row 0: `✦ Cortex Code · <crumb>`.
    let header_area = Rect {
        height: 1,
        ..area
    };
    frame.render_widget(
        Paragraph::new(header_line(crumb, width, theme))
            .style(Style::default().bg(theme.background)),
        header_area,
    );

    // Row 1: full-width horizontal rule in the ghost tone.
    if area.height > 1 {
        let rule = glyphs::BOX_HORIZONTAL.repeat(width);
        let rule_area = Rect {
            y: area.y + 1,
            height: 1,
            ..area
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                rule,
                Style::default().fg(theme.ghost).bg(theme.background),
            ))),
            rule_area,
        );
    }

    // Bottom row: the shared 3-zone status bar on the BAR surface.
    if area.height > 2 {
        let bar_area = Rect {
            y: area.y + area.height - 1,
            height: 1,
            ..area
        };
        frame.render_widget(
            Paragraph::new(status_bar_line(app, width))
                .style(Style::default().fg(theme.text).bg(theme.surface[0])),
            bar_area,
        );
    }

    // Body region: rows 2..H-2 (between the rule and the status bar).
    let body_height = area.height.saturating_sub(3);
    Rect {
        x: area.x,
        y: area.y + 2,
        width: area.width,
        height: body_height,
    }
}

/// The wordmark header line: `✦` (ORANGE) + `Cortex Code` (BRIGHT) + ` · ` (GHOST) + crumb (AMBER),
/// inset by [`VIEW_MARGIN`].
fn header_line(crumb: &str, width: usize, theme: &ThemeColors) -> Line<'static> {
    let base = Style::default().fg(theme.text).bg(theme.background);
    let mut cells = vec![(' ', base); width.max(1)];
    let segments: Vec<(String, Style)> = vec![
        (
            format!("{} ", glyphs::BRAND_STAR),
            Style::default().fg(theme.primary).bg(theme.background),
        ),
        (
            "Cortex Code".to_owned(),
            Style::default()
                .fg(theme.text_bright)
                .bg(theme.background)
                .add_modifier(Modifier::BOLD),
        ),
        (
            format!(" {} ", glyphs::SEPARATOR),
            Style::default().fg(theme.ghost).bg(theme.background),
        ),
        (
            crumb.to_owned(),
            Style::default().fg(theme.secondary).bg(theme.background),
        ),
    ];
    place_segments(&mut cells, VIEW_MARGIN as usize, &segments);
    cells_to_line(&cells)
}

/// Wrap a view body in a rounded panel: a `border`-toned frame with an AMBER title embedded in the
/// top-left of the top border and an optional FAINT note embedded top-right. Returns the inner
/// content region.
pub fn render_panel(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    note: Option<&str>,
    theme: &ThemeColors,
) -> Rect {
    if area.width < 2 || area.height < 2 {
        return area;
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(theme.background))
        .title(format!(" {title} "))
        .title_style(
            Style::default()
                .fg(theme.secondary)
                .add_modifier(Modifier::BOLD),
        );
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // The note is embedded in the top border, right-aligned, so it never collides with the title.
    if let Some(note) = note.filter(|n| !n.is_empty()) {
        let text = format!(" {note} ");
        let len = text.chars().count() as u16;
        let max = area.width.saturating_sub(2);
        if len <= max {
            let note_area = Rect {
                x: area.x + area.width - 1 - len,
                y: area.y,
                width: len,
                height: 1,
            };
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    text,
                    Style::default().fg(theme.text_muted).bg(theme.background),
                ))),
                note_area,
            );
        }
    }

    inner
}

/// A key/value row: the `label` in the FAINT tone padded to `label_width`, then `value` styled by
/// `value_style` so every value in a block starts at the same column.
#[must_use]
pub fn kv_line(
    label: &str,
    value: &str,
    value_style: Style,
    label_width: usize,
    theme: &ThemeColors,
) -> Line<'static> {
    let label_cells = label.chars().count();
    let padded = if label_cells >= label_width {
        format!("{label} ")
    } else {
        let pad = label_width - label_cells;
        format!("{label}{}", " ".repeat(pad))
    };
    Line::from(vec![
        Span::styled(padded, Style::default().fg(theme.text_muted)),
        Span::styled(value.to_owned(), value_style),
    ])
}

/// A full-cell progress bar of `width` cells: the filled prefix in the primary (ORANGE) tone using
/// `█` and the empty remainder in the GHOST tone using `░`. `frac` is clamped to `0.0..=1.0`.
///
/// This is the shared `pbar` helper for the Wave-2 views. Its first in-app consumer is the status
/// view's context-usage bar (`view-status-timeline`), which supplies the real context fraction.
#[must_use]
pub fn pbar_spans(width: usize, frac: f32, theme: &ThemeColors) -> Vec<Span<'static>> {
    let frac = frac.clamp(0.0, 1.0);
    let filled = ((frac * width as f32).round() as usize).min(width);
    let empty = width - filled;
    let mut spans = Vec::new();
    if filled > 0 {
        spans.push(Span::styled(
            glyphs::PROGRESS_FULL.repeat(filled),
            Style::default().fg(theme.primary),
        ));
    }
    if empty > 0 {
        spans.push(Span::styled(
            glyphs::PROGRESS_EMPTY.repeat(empty),
            Style::default().fg(theme.ghost),
        ));
    }
    spans
}

/// The faint, italic placeholder a data view shows inside its panel when it has nothing to list
/// (e.g. `/mcp` with no servers or `/queue` with nothing queued), so an empty view never renders a
/// blank panel.
#[must_use]
pub fn empty_state_line(text: &str, theme: &ThemeColors) -> Line<'static> {
    Line::from(Span::styled(
        text.to_owned(),
        Style::default()
            .fg(theme.text_muted)
            .add_modifier(Modifier::ITALIC),
    ))
}

/// The fixed left column width (in cells) for the label of every `/status` key/value row, so the
/// values all begin at the same column.
const STATUS_LABEL_WIDTH: usize = 12;

/// The git branch/dirty state shown on the `/status` workspace row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitInfo {
    /// The current branch name (e.g. `main`).
    pub branch: String,
    /// Whether the working tree has no uncommitted changes to tracked files.
    pub clean: bool,
}

/// The real runtime state rendered by the `/status` view, gathered once when the view opens (see
/// [`crate::app::App`]). Colors are applied by [`status_body_lines`]; this carries only the data.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusData {
    /// The session title (or its id / `new` when it has no title).
    pub session: String,
    /// A human age plus message count (e.g. `2h ago · 12 messages`).
    pub started: String,
    /// The active agent name.
    pub agent: String,
    /// The active model id.
    pub model: String,
    /// The active reasoning variant, shown as a dim-orange tag on the model row.
    pub variant: String,
    /// Tokens consumed against the model context window (for the usage bar + count).
    pub context_used: u64,
    /// The active model's context window (0 when unknown; the bar then reads empty).
    pub context_limit: u64,
    /// The pre-formatted token breakdown (e.g. `in 38.2k · out 4.1k · 42.3k total`).
    pub tokens: String,
    /// The pre-formatted session cost (e.g. `$0.84 this session`).
    pub cost: String,
    /// The permission posture summary (e.g. `edit allow · bash allow-all · web allow`).
    pub permits: String,
    /// The working directory, home-relative when under `$HOME`.
    pub workspace: String,
    /// The git branch/clean state, or `None` when the workspace is not a git repo.
    pub git: Option<GitInfo>,
}

/// One event row in the `/timeline` view: a status dot, a timestamp, an author/who label, and a
/// description. Colors are applied by [`timeline_body_lines`]; this carries only the data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineEvent {
    /// The status dot glyph selecting shape + color.
    pub dot: TimelineDot,
    /// The `HH:MM` timestamp.
    pub time: String,
    /// The author/who label (`you`, `assistant`, `session started`, a tool/role name…).
    pub who: String,
    /// The description text, clipped to the panel width.
    pub text: String,
}

/// The three safe timeline status dots (`●`/`○`/`◆`); no braille or tofu is ever used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimelineDot {
    /// A completed/user/system milestone event (`●`).
    Filled,
    /// An in-progress event (`○`).
    Hollow,
    /// A tool/action event (`◆`, amber).
    Diamond,
}

/// The color tone of an MCP server's status dot + word in the `/mcp` view, chosen by connection
/// state. The only non-orange hues in the view live here (green/amber/red status), so brand purity
/// holds elsewhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpStatusTone {
    /// The server is connected and serving (`ready`) — OKG success green.
    Ready,
    /// The server is starting up or needs attention (`needs auth`) — AMBER.
    Degraded,
    /// The server failed to connect (`error`) — ERR red.
    Error,
    /// A neutral state (`disabled`/`configured`) — FAINT, none of the status hues.
    Neutral,
}

impl McpStatusTone {
    /// The theme color for this tone's status dot and word.
    #[must_use]
    fn color(self, theme: &ThemeColors) -> ratatui::style::Color {
        match self {
            McpStatusTone::Ready => theme.success,
            McpStatusTone::Degraded => theme.secondary,
            McpStatusTone::Error => theme.error,
            McpStatusTone::Neutral => theme.text_muted,
        }
    }
}

/// One server row in the `/mcp` view: the server name, a status dot + word colored by
/// [`McpStatusTone`], the tool count (`None` renders as `—` for a server that is not serving), and
/// the comma-joined list of exposed capabilities (or an error detail). Colors are applied by
/// [`mcp_body_lines`]; this carries only the data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServerRow {
    /// The configured server name.
    pub name: String,
    /// The status word (`ready`/`needs auth`/`error`/`disabled`/`configured`).
    pub status_word: String,
    /// The status tone driving the dot + word color.
    pub tone: McpStatusTone,
    /// The number of tools the server exposes (`None` for a server that is not serving).
    pub tool_count: Option<usize>,
    /// The comma-joined exposed capabilities (tools/resources/prompts), or an error detail; clipped
    /// with a trailing `…` to fit the panel.
    pub exposes: String,
}

/// Format `n` with thousands separators (`42318` -> `42,318`) for the context token counts.
#[must_use]
pub fn format_thousands(n: u64) -> String {
    let digits = n.to_string();
    let len = digits.len();
    let mut out = String::with_capacity(len + len / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// A `/status` key/value row: the `label` FAINT and padded to [`STATUS_LABEL_WIDTH`], then the
/// value spans so every value begins at the same column.
fn status_row(label: &str, value: Vec<Span<'static>>, theme: &ThemeColors) -> Line<'static> {
    let label_cells = label.chars().count();
    let padded = if label_cells >= STATUS_LABEL_WIDTH {
        format!("{label} ")
    } else {
        format!("{label}{}", " ".repeat(STATUS_LABEL_WIDTH - label_cells))
    };
    let mut spans = vec![Span::styled(padded, Style::default().fg(theme.text_muted))];
    spans.extend(value);
    Line::from(spans)
}

/// A faint full-width group separator rule (`─` in the ghost tone).
fn rule_line(width: usize, theme: &ThemeColors) -> Line<'static> {
    Line::from(Span::styled(
        glyphs::BOX_HORIZONTAL.repeat(width.max(1)),
        Style::default().fg(theme.ghost),
    ))
}

/// Build the `/status` body: aligned key/value rows (session/started/agent/model), a context-usage
/// bar, token/cost rows, and the permits/workspace rows, grouped by faint separator rules. `width`
/// is the render width (used for the rules and to size the usage bar).
///
/// Colors come only from the theme tokens: labels FAINT, the agent value AMBER, the model's variant
/// tag dim-orange (`theme.border`), the usage-bar percentage AMBER, and a clean git marker (`✓
/// clean`) in the OKG success green — every other value is a neutral text tone (brand-pure; no
/// numeric value is ever green).
#[must_use]
pub fn status_body_lines(data: &StatusData, width: usize, theme: &ThemeColors) -> Vec<Line<'static>> {
    let bright = Style::default().fg(theme.text_bright);
    let txt = Style::default().fg(theme.text);
    let dim = Style::default().fg(theme.text_dim);
    let amber = Style::default().fg(theme.secondary);
    let orange_d = Style::default().fg(theme.border);
    let okg = Style::default().fg(theme.success);

    let bar_width = 40.min(width.saturating_sub(STATUS_LABEL_WIDTH + 6)).max(1);

    let mut model_value = vec![Span::styled(data.model.clone(), bright)];
    if !data.variant.trim().is_empty() {
        model_value.push(Span::styled(format!("   {}", data.variant), orange_d));
    }

    let frac = if data.context_limit > 0 {
        data.context_used as f32 / data.context_limit as f32
    } else {
        0.0
    };
    let pct = (frac.clamp(0.0, 1.0) * 100.0).round() as u64;
    let context_value = if data.context_limit > 0 {
        format!(
            "{} / {} tokens",
            format_thousands(data.context_used),
            format_thousands(data.context_limit)
        )
    } else {
        format!("{} tokens", format_thousands(data.context_used))
    };
    let mut bar_spans = vec![Span::raw(" ".repeat(STATUS_LABEL_WIDTH))];
    bar_spans.extend(pbar_spans(bar_width, frac, theme));
    bar_spans.push(Span::raw(" "));
    bar_spans.push(Span::styled(format!("{pct}%"), amber));

    let mut workspace_value = vec![Span::styled(data.workspace.clone(), txt)];
    if let Some(git) = &data.git {
        workspace_value.push(Span::styled(format!("   git: {} ", git.branch), dim));
        if git.clean {
            workspace_value.push(Span::styled(format!("{} clean", glyphs::MARK_DONE), okg));
        } else {
            workspace_value.push(Span::styled(
                format!("{} uncommitted", glyphs::DOT_FILLED),
                amber,
            ));
        }
    }

    vec![
        Line::from(""),
        kv_line("session", &data.session, bright, STATUS_LABEL_WIDTH, theme),
        kv_line("started", &data.started, txt, STATUS_LABEL_WIDTH, theme),
        kv_line("agent", &data.agent, amber, STATUS_LABEL_WIDTH, theme),
        status_row("model", model_value, theme),
        Line::from(""),
        rule_line(width, theme),
        Line::from(""),
        kv_line("context", &context_value, txt, STATUS_LABEL_WIDTH, theme),
        Line::from(bar_spans),
        Line::from(""),
        kv_line("tokens", &data.tokens, dim, STATUS_LABEL_WIDTH, theme),
        kv_line("cost", &data.cost, dim, STATUS_LABEL_WIDTH, theme),
        Line::from(""),
        rule_line(width, theme),
        Line::from(""),
        kv_line("permits", &data.permits, dim, STATUS_LABEL_WIDTH, theme),
        status_row("workspace", workspace_value, theme),
    ]
}

/// Build the `/timeline` body: a rail of status dots (`●`/`○`/`◆`) linked by `│` connectors, each
/// event on its own row with a faint fixed timestamp column, a colored author column (AMBER for a
/// `◆` tool event, BRIGHT for `you`, else normal text), and a dim description clipped to `width`.
///
/// Events are laid out at 2-row spacing (a connector row above every event after the first). When
/// there are more events than fit in `max_rows`, only the most recent that fit are rendered so no
/// row overflows the panel border.
#[must_use]
pub fn timeline_body_lines(
    events: &[TimelineEvent],
    width: usize,
    max_rows: usize,
    theme: &ThemeColors,
) -> Vec<Line<'static>> {
    if events.is_empty() {
        return vec![empty_state_line("No events in this session yet.", theme)];
    }

    // Glyph column, then the timestamp at +3 and the author at +9 (per the layout spec).
    const GLYPH_COL: usize = 1;
    const TIME_COL: usize = GLYPH_COL + 3;
    const WHO_COL: usize = GLYPH_COL + 9;

    // 2-row spacing: 1 row for the first event, then a connector + event row for each subsequent
    // one (2n-1 rows total). Keep the latest events that fit so nothing overflows the border.
    let capacity = max_rows.div_ceil(2).max(1);
    let start = events.len().saturating_sub(capacity);
    let shown = &events[start..];

    let base = Style::default().fg(theme.text);
    let ghost = Style::default().fg(theme.ghost);
    let faint = Style::default().fg(theme.text_muted);
    let dim = Style::default().fg(theme.text_dim);

    let mut lines = Vec::with_capacity(shown.len() * 2);
    for (idx, event) in shown.iter().enumerate() {
        if idx > 0 {
            let mut connector = vec![(' ', base); width.max(1)];
            place_segments(
                &mut connector,
                GLYPH_COL,
                &[(glyphs::BOX_VERTICAL.to_owned(), ghost)],
            );
            lines.push(cells_to_line(&connector));
        }

        let (glyph, dot_style) = match event.dot {
            TimelineDot::Diamond => (glyphs::DOT_DIAMOND, Style::default().fg(theme.secondary)),
            TimelineDot::Hollow => (glyphs::DOT_HOLLOW, faint),
            TimelineDot::Filled => {
                let color = if matches!(event.who.as_str(), "you" | "assistant") {
                    theme.text
                } else {
                    theme.primary
                };
                (glyphs::DOT_FILLED, Style::default().fg(color))
            }
        };
        let who_style = if matches!(event.dot, TimelineDot::Diamond) {
            Style::default().fg(theme.secondary)
        } else if event.who == "you" {
            Style::default().fg(theme.text_bright)
        } else {
            Style::default().fg(theme.text)
        };

        let mut cells = vec![(' ', base); width.max(1)];
        place_segments(&mut cells, GLYPH_COL, &[(glyph.to_owned(), dot_style)]);
        place_segments(&mut cells, TIME_COL, &[(event.time.clone(), faint)]);
        place_segments(&mut cells, WHO_COL, &[(event.who.clone(), who_style)]);
        let text_col = WHO_COL + event.who.chars().count() + 2;
        if text_col < width {
            let avail = width - text_col;
            let text: String = if event.text.chars().count() > avail {
                event.text.chars().take(avail.saturating_sub(1)).collect::<String>() + glyphs::ELLIPSIS
            } else {
                event.text.clone()
            };
            place_segments(&mut cells, text_col, &[(text, dim)]);
        }
        lines.push(cells_to_line(&cells));
    }
    lines
}

/// Clip `s` to at most `max` columns, appending an `…` when it is cut (so a truncated column always
/// signals the elision). `max == 0` yields an empty string.
#[must_use]
fn clip(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let keep = max.saturating_sub(1);
    let mut out: String = s.chars().take(keep).collect();
    out.push_str(glyphs::ELLIPSIS);
    out
}

// `/mcp` table column offsets within the panel body (server / status / tools / exposes).
const MCP_NAME_COL: usize = 0;
const MCP_STATUS_COL: usize = 16;
const MCP_TOOLS_COL: usize = 30;
const MCP_EXPOSES_COL: usize = 38;

/// Build the `/mcp` body: a faint column-header row (`server`/`status`/`tools`/`exposes`) + a rule,
/// then one aligned row per server. Each server row shows the name (BRIGHT), a `●` status dot + word
/// colored by state (ready→green, degraded→amber, error→red, else FAINT), the tool count (or `—`),
/// and the DIM exposed-capabilities list clipped with `…` so it keeps a ≥2-col gap before the border.
///
/// With no servers it returns the explicit empty state so `/mcp` never renders a blank panel.
#[must_use]
pub fn mcp_body_lines(rows: &[McpServerRow], width: usize, theme: &ThemeColors) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return vec![empty_state_line("No MCP servers configured.", theme)];
    }
    let width = width.max(1);
    let faint = Style::default().fg(theme.text_muted);

    let mut header = vec![(' ', Style::default().fg(theme.text)); width];
    place_segments(&mut header, MCP_NAME_COL, &[("server".to_owned(), faint)]);
    place_segments(&mut header, MCP_STATUS_COL, &[("status".to_owned(), faint)]);
    place_segments(&mut header, MCP_TOOLS_COL, &[("tools".to_owned(), faint)]);
    place_segments(&mut header, MCP_EXPOSES_COL, &[("exposes".to_owned(), faint)]);

    let mut lines = Vec::with_capacity(rows.len() + 2);
    lines.push(cells_to_line(&header));
    lines.push(rule_line(width, theme));

    let bright = Style::default().fg(theme.text_bright);
    let txt = Style::default().fg(theme.text);
    let dim = Style::default().fg(theme.text_dim);

    for row in rows {
        let mut cells = vec![(' ', Style::default().fg(theme.text)); width];

        let name = clip(&row.name, MCP_STATUS_COL.saturating_sub(MCP_NAME_COL + 1));
        place_segments(&mut cells, MCP_NAME_COL, &[(name, bright)]);

        let tone = Style::default().fg(row.tone.color(theme));
        place_segments(
            &mut cells,
            MCP_STATUS_COL,
            &[
                (format!("{} ", glyphs::DOT_FILLED), tone),
                (clip(&row.status_word, MCP_TOOLS_COL.saturating_sub(MCP_STATUS_COL + 3)), tone),
            ],
        );

        let tools = match row.tool_count {
            Some(n) => n.to_string(),
            None => glyphs::EM_DASH.to_owned(),
        };
        place_segments(&mut cells, MCP_TOOLS_COL, &[(tools, txt)]);

        if MCP_EXPOSES_COL + 2 < width {
            let avail = width - MCP_EXPOSES_COL - 2;
            place_segments(&mut cells, MCP_EXPOSES_COL, &[(clip(&row.exposes, avail), dim)]);
        }

        lines.push(cells_to_line(&cells));
    }
    lines
}

// `/queue` row column offsets within the panel body (number / prompt text).
const QUEUE_NUM_COL: usize = 1;
const QUEUE_TEXT_COL: usize = 5;
/// The run-next badge shown on the head-of-queue row.
const QUEUE_NEXT_BADGE: &str = "next";

/// Build the `/queue` body: an intro line noting the queue runs automatically after the current
/// turn, then one numbered row per queued prompt. The head-of-queue row carries a right-aligned
/// `next` badge (AMBER); the leading number is ORANGE on the selected/next row and dim-orange
/// otherwise; the prompt text is BRIGHT on the selected row and normal text elsewhere. The selected
/// row is painted as a full-width warm `SELBG` selection bar.
///
/// With nothing queued it returns the intro + explicit empty state so `/queue` never renders a blank
/// panel.
#[must_use]
pub fn queue_body_lines(
    prompts: &[String],
    selected: usize,
    width: usize,
    theme: &ThemeColors,
) -> Vec<Line<'static>> {
    let width = width.max(1);
    let intro = Line::from(Span::styled(
        "runs automatically after the current turn finishes".to_owned(),
        Style::default().fg(theme.text_dim),
    ));
    if prompts.is_empty() {
        return vec![
            intro,
            Line::from(""),
            empty_state_line("No queued prompts.", theme),
        ];
    }

    let mut lines = vec![intro, Line::from("")];
    let badge_len = QUEUE_NEXT_BADGE.chars().count();
    for (i, text) in prompts.iter().enumerate() {
        let is_selected = i == selected;
        let is_next = i == 0;
        let row_bg = if is_selected {
            theme.surface[3]
        } else {
            theme.background
        };
        let mut cells = vec![(' ', Style::default().fg(theme.text).bg(row_bg)); width];

        let num_color = if is_selected || is_next {
            theme.primary
        } else {
            theme.border
        };
        place_segments(
            &mut cells,
            QUEUE_NUM_COL,
            &[(
                format!("{:>2}", i + 1),
                Style::default().fg(num_color).bg(row_bg),
            )],
        );

        let text_color = if is_selected {
            theme.text_bright
        } else {
            theme.text
        };
        let reserved = if is_next { badge_len + 2 } else { 0 };
        let avail = width
            .saturating_sub(QUEUE_TEXT_COL + 2 + reserved)
            .max(1);
        place_segments(
            &mut cells,
            QUEUE_TEXT_COL,
            &[(clip(text, avail), Style::default().fg(text_color).bg(row_bg))],
        );

        if is_next && width > badge_len + 2 {
            let start = width - badge_len - 2;
            place_segments(
                &mut cells,
                start,
                &[(
                    QUEUE_NEXT_BADGE.to_owned(),
                    Style::default().fg(theme.secondary).bg(row_bg),
                )],
            );
        }

        lines.push(cells_to_line(&cells));
    }
    lines
}

/// A footer key-hint row for a full-screen view: `key label   key label …` with each key glyph in
/// the dim-orange border tone and each label in the FAINT tone (spaced apart), matching the picker
/// modal footer style. Used inside the bottom of the `/queue` panel.
#[must_use]
pub fn footer_hint_line(
    hints: &[(String, String)],
    width: usize,
    theme: &ThemeColors,
) -> Line<'static> {
    let mut cells = vec![(' ', Style::default().fg(theme.text)); width.max(1)];
    let key_style = Style::default().fg(theme.border);
    let label_style = Style::default().fg(theme.text_muted);
    let mut segments: Vec<(String, Style)> = Vec::new();
    for (i, (key, label)) in hints.iter().enumerate() {
        if i > 0 {
            segments.push(("   ".to_owned(), label_style));
        }
        segments.push((key.clone(), key_style));
        segments.push((" ".to_owned(), label_style));
        segments.push((label.clone(), label_style));
    }
    place_segments(&mut cells, 0, &segments);
    cells_to_line(&cells)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, Status};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;

    fn app() -> App {
        App::new(Status {
            model: "google/gemini-2.5-flash".to_owned(),
            agent: "build".to_owned(),
            mode: "build".to_owned(),
            variant: "default".to_owned(),
            session: "new".to_owned(),
        })
    }

    fn buffer_text(buf: &Buffer) -> String {
        buf.content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect()
    }

    fn row_text(buf: &Buffer, y: u16) -> String {
        let w = buf.area.width;
        (0..w)
            .map(|x| buf[(x, y)].symbol())
            .collect::<Vec<_>>()
            .concat()
    }

    #[test]
    fn crumb_matches_the_slash_command_for_every_view() {
        // VAL-VIEW-049: the crumb after `Cortex Code ·` equals the opening slash command.
        assert_eq!(ViewKind::Status.crumb(), "/status");
        assert_eq!(ViewKind::Timeline.crumb(), "/timeline");
        assert_eq!(ViewKind::Queue.crumb(), "/queue");
        assert_eq!(ViewKind::Mcp.crumb(), "/mcp");
        assert_eq!(ViewKind::Diff.crumb(), "/diff");
        assert_eq!(ViewKind::Help.crumb(), "/help");
        assert_eq!(ViewKind::Review.crumb(), "/review");
        // Every catalog kind carries a non-empty panel title.
        for kind in ViewKind::ALL {
            assert!(!kind.title().is_empty(), "{kind:?} has a title");
        }
    }

    #[test]
    fn chrome_renders_header_rule_and_status_bar() {
        // VAL-VIEW-048/049: row 0 is `✦ Cortex Code · <crumb>`, row 1 is a full-width rule, and the
        // bottom row is the shared 3-zone status bar with the REAL model id (never `opus-4.8`).
        let a = app();
        let theme = ThemeColors::from_name(a.theme());
        let backend = TestBackend::new(92, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut body = Rect::default();
        terminal
            .draw(|frame| {
                body = render_chrome(frame, frame.area(), &a, ViewKind::Status.crumb(), &theme);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();

        let header = row_text(&buf, 0);
        assert!(header.contains('✦'), "brand star on row 0:\n{header}");
        assert!(header.contains("Cortex Code"), "wordmark on row 0:\n{header}");
        assert!(header.contains("/status"), "crumb on row 0:\n{header}");

        let rule = row_text(&buf, 1);
        assert!(
            rule.chars().filter(|&c| c == '─').count() >= 80,
            "row 1 is a full-width rule:\n{rule}"
        );

        let status = row_text(&buf, 29);
        assert!(status.contains("? for help"), "status bar left zone:\n{status}");
        assert!(
            status.contains("google/gemini-2.5-flash"),
            "status bar right zone shows the real model id:\n{status}"
        );
        assert!(!status.contains("opus-4.8"), "never the mockup model id:\n{status}");

        // The body region sits between the rule and the status bar.
        assert_eq!(body.y, 2, "body starts below the rule");
        assert_eq!(body.height, 27, "body stops above the status bar");
    }

    #[test]
    fn chrome_header_uses_brand_tokens() {
        // VAL-VIEW-052: the star is ORANGE (primary), the wordmark BRIGHT, the separator GHOST, and
        // the crumb AMBER (secondary) — no non-orange chrome hue.
        let theme = ThemeColors::from_name("dark");
        let line = header_line("/status", 92, &theme);
        let fg_of = |needle: &str| {
            line.spans
                .iter()
                .find(|s| s.content.as_ref() == needle)
                .and_then(|s| s.style.fg)
        };
        assert_eq!(fg_of("✦ "), Some(theme.primary), "star is ORANGE");
        assert_eq!(fg_of("Cortex Code"), Some(theme.text_bright), "wordmark BRIGHT");
        assert_eq!(fg_of(" · "), Some(theme.ghost), "separator GHOST");
        assert_eq!(fg_of("/status"), Some(theme.secondary), "crumb AMBER");
    }

    #[test]
    fn chrome_paints_an_explicit_background_on_every_cell() {
        // VAL-VIEW-052 / brand background: no content cell falls through to terminal-default black.
        let a = app();
        let theme = ThemeColors::from_name(a.theme());
        let backend = TestBackend::new(92, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render_chrome(frame, frame.area(), &a, ViewKind::Status.crumb(), &theme);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        // The header/rule rows carry the near-black brand canvas (not the reset default).
        for y in [0u16, 1u16] {
            for x in 0..buf.area.width {
                assert_ne!(
                    buf[(x, y)].bg,
                    ratatui::style::Color::Reset,
                    "cell ({x},{y}) has an explicit bg"
                );
            }
        }
    }

    #[test]
    fn panel_draws_rounded_border_title_and_note() {
        // VAL-VIEW-001/007: the body is wrapped in a rounded titled panel; the note rides the top
        // border, right-aligned.
        let theme = ThemeColors::from_name("dark");
        let backend = TestBackend::new(60, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut inner = Rect::default();
        terminal
            .draw(|frame| {
                inner = render_panel(
                    frame,
                    frame.area(),
                    "Session status",
                    Some("9 events"),
                    &theme,
                );
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let text = buffer_text(&buf);
        assert!(text.contains('╭') && text.contains('╮'), "rounded top corners");
        assert!(text.contains('╰') && text.contains('╯'), "rounded bottom corners");
        assert!(text.contains("Session status"), "title embedded in the border");
        let top = row_text(&buf, 0);
        assert!(top.contains("9 events"), "note rides the top border:\n{top}");
        // The inner region excludes the border on all sides.
        assert!(inner.y >= 1 && inner.height <= 10, "inner excludes the border");
    }

    #[test]
    fn kv_line_aligns_the_value_column() {
        let theme = ThemeColors::from_name("dark");
        let value_style = Style::default().fg(theme.text_bright);
        let short = kv_line("agent", "build", value_style, 12, &theme);
        let long = kv_line("permissions", "allow all", value_style, 12, &theme);
        // The label span is padded to the fixed width so both values begin at the same column.
        assert_eq!(short.spans[0].content.chars().count(), 12);
        assert_eq!(long.spans[0].content.chars().count(), 12);
        assert_eq!(short.spans[0].style.fg, Some(theme.text_muted), "label FAINT");
        assert_eq!(short.spans[1].content.as_ref(), "build");
        assert_eq!(short.spans[1].style.fg, Some(theme.text_bright), "value styled");
    }

    #[test]
    fn pbar_fills_by_fraction_with_brand_tokens() {
        // VAL-VIEW-003: filled `█` cells (ORANGE) proportional to the fraction, empty `░` (GHOST).
        let theme = ThemeColors::from_name("dark");
        let spans = pbar_spans(40, 0.25, &theme);
        let filled: String = spans
            .iter()
            .filter(|s| s.style.fg == Some(theme.primary))
            .map(|s| s.content.as_ref())
            .collect();
        let empty: String = spans
            .iter()
            .filter(|s| s.style.fg == Some(theme.ghost))
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(filled.chars().count(), 10, "25% of 40 cells filled");
        assert_eq!(empty.chars().count(), 30, "remainder empty");
        assert!(filled.chars().all(|c| c == '█'), "filled cells are `█`");
        assert!(empty.chars().all(|c| c == '░'), "empty cells are `░`");

        // Clamped extremes are whole-bar fills.
        assert_eq!(
            pbar_spans(10, 2.0, &theme)
                .iter()
                .map(|s| s.content.chars().count())
                .sum::<usize>(),
            10
        );
        let zero = pbar_spans(10, -1.0, &theme);
        assert!(
            zero.iter().all(|s| s.style.fg == Some(theme.ghost)),
            "a clamped-zero bar is all empty"
        );
    }

    #[test]
    fn empty_state_line_is_faint() {
        // VAL-VIEW-054: the empty-state placeholder is a faint line, never a blank panel.
        let theme = ThemeColors::from_name("dark");
        let line = empty_state_line("No MCP servers configured.", &theme);
        assert_eq!(line.spans[0].content.as_ref(), "No MCP servers configured.");
        assert_eq!(line.spans[0].style.fg, Some(theme.text_muted));
    }

    #[test]
    fn chrome_glyphs_are_all_safe() {
        // VAL-VIEW-050: no forbidden glyphs (braille, thin triangles, curly quotes, `>_`) appear.
        let a = app();
        let theme = ThemeColors::from_name(a.theme());
        let backend = TestBackend::new(92, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let body = render_chrome(frame, frame.area(), &a, ViewKind::Mcp.crumb(), &theme);
                let panel = Rect {
                    x: body.x + VIEW_MARGIN,
                    y: body.y,
                    width: body.width - VIEW_MARGIN * 2,
                    height: body.height,
                };
                render_panel(frame, panel, ViewKind::Mcp.title(), Some("0 servers"), &theme);
            })
            .unwrap();
        let text = buffer_text(&terminal.backend().buffer().clone());
        assert!(!glyphs::contains_braille(&text), "no braille anywhere");
        for forbidden in ['▸', '▾', '⌘', '⬡', '“', '”', '‘', '’'] {
            assert!(!text.contains(forbidden), "forbidden glyph {forbidden:?}");
        }
        assert!(!text.contains(">_"), "no `>_` cliché");
    }

    fn render_lines(lines: Vec<Line<'static>>, w: u16, h: u16) -> Buffer {
        let backend = TestBackend::new(w, h);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                frame.render_widget(Paragraph::new(lines), frame.area());
            })
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn status_fixture() -> StatusData {
        StatusData {
            session: "fix streaming buffer limit".to_owned(),
            started: "2h ago · 12 messages".to_owned(),
            agent: "build".to_owned(),
            model: "google/gemini-2.5-flash".to_owned(),
            variant: "default".to_owned(),
            context_used: 42_318,
            context_limit: 200_000,
            tokens: "in 38.2k · out 4.1k · 42.3k total".to_owned(),
            cost: "$0.84 this session".to_owned(),
            permits: "edit allow · bash allow-all · web allow".to_owned(),
            workspace: "/projects/cortex-code".to_owned(),
            git: Some(GitInfo { branch: "main".to_owned(), clean: true }),
        }
    }

    fn cell_fg(buf: &Buffer, needle: &str) -> Option<ratatui::style::Color> {
        // Find the first cell of the first row whose text run starts `needle`, return its fg.
        for y in 0..buf.area.height {
            let row = row_text(buf, y);
            if let Some(byte_idx) = row.find(needle) {
                let col = row[..byte_idx].chars().count() as u16;
                return buf[(col, y)].fg.into();
            }
        }
        None
    }

    #[test]
    fn format_thousands_groups_by_threes() {
        assert_eq!(format_thousands(0), "0");
        assert_eq!(format_thousands(42_318), "42,318");
        assert_eq!(format_thousands(200_000), "200,000");
        assert_eq!(format_thousands(1_048_576), "1,048,576");
    }

    #[test]
    fn status_body_aligns_rows_and_marks_git_clean() {
        // VAL-VIEW-002/004/005: aligned label/value rows with an amber agent, brand-pure token/cost
        // rows, and a `✓ clean` git marker in the OKG success green.
        let theme = ThemeColors::from_name("dark");
        let data = status_fixture();
        let lines = status_body_lines(&data, 84, &theme);
        let buf = render_lines(lines, 84, 24);
        let text = buffer_text(&buf);

        for label in ["session", "started", "agent", "model", "context", "tokens", "cost", "permits", "workspace"] {
            assert!(text.contains(label), "status row {label} present");
        }
        assert!(text.contains("fix streaming buffer limit"), "real session title");
        assert!(text.contains("google/gemini-2.5-flash"), "real model id");
        assert!(text.contains("42,318 / 200,000 tokens"), "context counts");
        assert!(text.contains("✓ clean"), "git-clean marker present");

        // The agent value is AMBER (secondary).
        assert_eq!(cell_fg(&buf, "build"), Some(theme.secondary), "agent value AMBER");
        // The clean marker is the OKG success green — the only status green.
        assert_eq!(cell_fg(&buf, "✓ clean"), Some(theme.success), "clean marker OKG");
        // No numeric value is green (brand purity): the token/cost/context values are neutral.
        assert_ne!(cell_fg(&buf, "42,318"), Some(theme.success), "context number not green");
        assert_ne!(cell_fg(&buf, "$0.84"), Some(theme.success), "cost not green");
    }

    #[test]
    fn status_body_draws_a_context_usage_bar_with_percent() {
        // VAL-VIEW-003: a `█`/`░` usage bar whose filled proportion matches an AMBER percentage.
        let theme = ThemeColors::from_name("dark");
        let data = status_fixture();
        let lines = status_body_lines(&data, 84, &theme);
        let buf = render_lines(lines, 84, 24);
        let text = buffer_text(&buf);
        assert!(text.contains('█'), "usage bar has filled cells");
        assert!(text.contains('░'), "usage bar has empty cells");
        assert!(text.contains("21%"), "percentage label (42318/200000 ≈ 21%)");
        assert_eq!(cell_fg(&buf, "21%"), Some(theme.secondary), "percentage AMBER");
        assert_eq!(cell_fg(&buf, "█"), Some(theme.primary), "filled cells ORANGE");
    }

    #[test]
    fn status_body_model_row_carries_a_dim_orange_variant_tag() {
        // VAL-VIEW-002: the model row carries the reasoning-variant tag in dim-orange.
        let theme = ThemeColors::from_name("dark");
        let data = status_fixture();
        let lines = status_body_lines(&data, 84, &theme);
        let buf = render_lines(lines, 84, 24);
        assert_eq!(cell_fg(&buf, "default"), Some(theme.border), "variant tag dim-orange");
    }

    #[test]
    fn timeline_body_renders_dots_connectors_and_columns() {
        // VAL-VIEW-008/009/010: a `●/○/◆` rail linked by `│` connectors, a faint timestamp, a
        // colored author column, and dim clipped text — all within the panel.
        let theme = ThemeColors::from_name("dark");
        let events = vec![
            TimelineEvent { dot: TimelineDot::Filled, time: "10:02".to_owned(), who: "session started".to_owned(), text: "build · gemini-2.5-flash".to_owned() },
            TimelineEvent { dot: TimelineDot::Filled, time: "10:03".to_owned(), who: "you".to_owned(), text: "cap the streaming buffer".to_owned() },
            TimelineEvent { dot: TimelineDot::Hollow, time: "10:03".to_owned(), who: "assistant".to_owned(), text: "planning the change".to_owned() },
            TimelineEvent { dot: TimelineDot::Diamond, time: "10:04".to_owned(), who: "read".to_owned(), text: "stream_processor.rs".to_owned() },
        ];
        let lines = timeline_body_lines(&events, 80, 24, &theme);
        let buf = render_lines(lines, 80, 24);
        let text = buffer_text(&buf);

        assert!(text.contains('●'), "filled dot present");
        assert!(text.contains('○'), "hollow dot present");
        assert!(text.contains('◆'), "diamond dot present");
        assert!(text.contains('│'), "vertical connectors present");
        assert!(text.contains("10:02") && text.contains("session started"), "ts + who present");
        assert!(!glyphs::contains_braille(&text), "no braille in the rail");

        // Author coloring: AMBER for a ◆ event, BRIGHT for `you`, else normal text.
        assert_eq!(cell_fg(&buf, "read"), Some(theme.secondary), "◆ author AMBER");
        assert_eq!(cell_fg(&buf, "you"), Some(theme.text_bright), "you author BRIGHT");
        assert_eq!(cell_fg(&buf, "assistant"), Some(theme.text), "assistant author TXT");
        // The timestamp column is faint.
        assert_eq!(cell_fg(&buf, "10:02"), Some(theme.text_muted), "timestamp FAINT");
        // The connector is drawn in the ghost tone.
        assert_eq!(cell_fg(&buf, "│"), Some(theme.ghost), "connector GHOST");
    }

    #[test]
    fn timeline_body_clips_to_fit_without_overflowing() {
        // VAL-VIEW-010: with more events than rows, only the latest that fit are rendered (no
        // overflow past the panel border).
        let theme = ThemeColors::from_name("dark");
        let events: Vec<TimelineEvent> = (0..20)
            .map(|i| TimelineEvent {
                dot: TimelineDot::Filled,
                time: format!("10:{i:02}"),
                who: "you".to_owned(),
                text: format!("event {i}"),
            })
            .collect();
        // 6 rows fit at most (6+1)/2 = 3 events (event, connector, event, connector, event = 5 rows).
        let lines = timeline_body_lines(&events, 80, 6, &theme);
        assert!(lines.len() <= 6, "rendered rows fit within max_rows: {}", lines.len());
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.clone()))
            .collect();
        assert!(text.contains("event 19"), "keeps the latest event");
        assert!(!text.contains("event 0 "), "drops the oldest events");
    }

    #[test]
    fn timeline_body_empty_state() {
        let theme = ThemeColors::from_name("dark");
        let lines = timeline_body_lines(&[], 80, 24, &theme);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans[0].style.fg, Some(theme.text_muted));
    }

    fn mcp_fixture() -> Vec<McpServerRow> {
        vec![
            McpServerRow {
                name: "context7".to_owned(),
                status_word: "ready".to_owned(),
                tone: McpStatusTone::Ready,
                tool_count: Some(2),
                exposes: "resolve-library-id, query-docs".to_owned(),
            },
            McpServerRow {
                name: "playwright".to_owned(),
                status_word: "needs auth".to_owned(),
                tone: McpStatusTone::Degraded,
                tool_count: None,
                exposes: "connecting to browser".to_owned(),
            },
            McpServerRow {
                name: "github".to_owned(),
                status_word: "error".to_owned(),
                tone: McpStatusTone::Error,
                tool_count: None,
                exposes: "auth failed — run: cortex auth login".to_owned(),
            },
        ]
    }

    #[test]
    fn mcp_body_has_a_header_rule_and_aligned_columns() {
        // VAL-VIEW-020: a faint column-header row + rule, then server rows aligned into fixed
        // server/status/tools/exposes columns.
        let theme = ThemeColors::from_name("dark");
        let rows = mcp_fixture();
        let lines = mcp_body_lines(&rows, 84, &theme);
        let buf = render_lines(lines, 84, 12);

        let header = row_text(&buf, 0);
        for label in ["server", "status", "tools", "exposes"] {
            assert!(header.contains(label), "header has {label}:\n{header}");
        }
        // The header labels sit at their fixed columns.
        assert!(header[..].starts_with("server"), "server column at col 0");
        assert_eq!(
            header.chars().skip(MCP_STATUS_COL).take(6).collect::<String>(),
            "status",
            "status column at +16"
        );
        // Row 1 is a full-width rule.
        assert!(
            row_text(&buf, 1).chars().filter(|&c| c == '─').count() >= 80,
            "row 1 is a rule"
        );
        // The first server row aligns its name at col 0 and tools value at the tools column.
        let row = row_text(&buf, 2);
        assert!(row.starts_with("context7"), "server name at col 0:\n{row}");
        assert_eq!(
            row.chars().skip(MCP_TOOLS_COL).take(1).collect::<String>(),
            "2",
            "tool count at the tools column"
        );
        assert!(
            row.chars().skip(MCP_EXPOSES_COL).collect::<String>().starts_with("resolve-library-id"),
            "exposes at +38:\n{row}"
        );
    }

    #[test]
    fn mcp_status_dots_are_colored_by_state() {
        // VAL-VIEW-021: ready → OKG green, degraded → AMBER, error → ERR red status dot + word.
        let theme = ThemeColors::from_name("dark");
        let rows = mcp_fixture();
        let lines = mcp_body_lines(&rows, 84, &theme);
        let buf = render_lines(lines, 84, 12);

        // The status dot sits at MCP_STATUS_COL of each server row (rows start at buffer row 2).
        assert_eq!(buf[(MCP_STATUS_COL as u16, 2)].symbol(), "●", "ready dot present");
        assert_eq!(buf[(MCP_STATUS_COL as u16, 2)].fg, theme.success, "ready dot OKG green");
        assert_eq!(buf[(MCP_STATUS_COL as u16, 3)].fg, theme.secondary, "degraded dot AMBER");
        assert_eq!(buf[(MCP_STATUS_COL as u16, 4)].fg, theme.error, "error dot ERR red");
        // The status word carries the same tone.
        assert_eq!(cell_fg(&buf, "ready"), Some(theme.success), "ready word OKG");
        assert_eq!(cell_fg(&buf, "error"), Some(theme.error), "error word ERR");
    }

    #[test]
    fn mcp_tool_count_falls_back_to_em_dash_and_exposes_clips() {
        // A server that is not serving shows `—` for its tool count; a long exposes column is
        // clipped with a trailing `…` (VAL-VIEW-022) keeping a ≥2-col gap before the border.
        let theme = ThemeColors::from_name("dark");
        let rows = vec![McpServerRow {
            name: "firecrawl".to_owned(),
            status_word: "ready".to_owned(),
            tone: McpStatusTone::Ready,
            tool_count: None,
            exposes: "scrape, search, crawl, map, extract, batch, deep-research, watch, monitor, act"
                .to_owned(),
        }];
        let width = 60usize;
        let lines = mcp_body_lines(&rows, width, &theme);
        let buf = render_lines(lines, width as u16, 4);
        let row = row_text(&buf, 2);
        assert_eq!(
            row.chars().skip(MCP_TOOLS_COL).take(1).collect::<String>(),
            "—",
            "no tool count renders an em dash:\n{row}"
        );
        assert!(row.contains('…'), "long exposes is clipped with an ellipsis:\n{row}");
        // The clipped exposes keeps at least a 2-col gap before the (border) right edge.
        let exposes: String = row.chars().skip(MCP_EXPOSES_COL).collect();
        assert!(exposes.trim_end().chars().count() <= width - MCP_EXPOSES_COL - 2, "≥2-col gap kept");
    }

    #[test]
    fn mcp_body_empty_state() {
        // VAL-VIEW-054: no servers → the explicit faint empty state, never a blank panel.
        let theme = ThemeColors::from_name("dark");
        let lines = mcp_body_lines(&[], 80, &theme);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans[0].style.fg, Some(theme.text_muted));
    }

    fn queue_fixture() -> Vec<String> {
        vec![
            "add unit tests for the buffer cap".to_owned(),
            "update CHANGELOG.md for v0.4.3".to_owned(),
            "run the full workspace test suite".to_owned(),
            "open a PR titled \"cap streaming buffer\"".to_owned(),
        ]
    }

    #[test]
    fn queue_body_numbers_rows_and_badges_the_next_prompt() {
        // VAL-VIEW-016/017: numbered rows with an orange next/selected number and a right-aligned
        // AMBER `next` badge on the head-of-queue prompt.
        let theme = ThemeColors::from_name("dark");
        let prompts = queue_fixture();
        let lines = queue_body_lines(&prompts, 0, 84, &theme);
        let buf = render_lines(lines, 84, 12);
        let text = buffer_text(&buf);

        assert!(text.contains("runs automatically after the current turn finishes"), "intro line");
        for prompt in &prompts {
            let head: String = prompt.chars().take(10).collect();
            assert!(text.contains(&head), "prompt {head:?} present");
        }
        // The numbers 1..=4 are rendered.
        for n in ["1", "2", "3", "4"] {
            assert!(text.contains(n), "row number {n} present");
        }
        // The `next` badge is present and AMBER.
        assert!(text.contains("next"), "run-next badge present");
        assert_eq!(cell_fg(&buf, "next"), Some(theme.secondary), "next badge AMBER");
        // The head-of-queue number is ORANGE (selected + next).
        assert_eq!(cell_fg(&buf, "1"), Some(theme.primary), "row 1 number ORANGE");
    }

    #[test]
    fn queue_selected_row_is_a_full_width_selection_bar() {
        // VAL-VIEW-018: the selected row is a solid warm SELBG bar; the number is ORANGE and the
        // text BRIGHT on that row; a non-selected row has no fill.
        let theme = ThemeColors::from_name("dark");
        let prompts = queue_fixture();
        // Select row index 1 (not the head) so selection and next differ.
        let lines = queue_body_lines(&prompts, 1, 84, &theme);
        let buf = render_lines(lines, 84, 12);

        // Rows start at buffer row 2 (intro + blank precede them); the selected row is index 1 → row 3.
        let selected_y = 3u16;
        for x in 0..84u16 {
            assert_eq!(
                buf[(x, selected_y)].bg,
                theme.surface[3],
                "selected row cell ({x}) carries the SELBG fill"
            );
        }
        // A non-selected row (row 2) is not filled with SELBG.
        assert_ne!(buf[(10u16, 2u16)].bg, theme.surface[3], "unselected row has no SELBG fill");
        // The selected prompt text is BRIGHT.
        assert_eq!(cell_fg(&buf, "update CHANGELOG"), Some(theme.text_bright), "selected text BRIGHT");
    }

    #[test]
    fn queue_body_empty_state() {
        let theme = ThemeColors::from_name("dark");
        let lines = queue_body_lines(&[], 0, 80, &theme);
        // intro + blank + empty-state placeholder.
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[2].spans[0].style.fg, Some(theme.text_muted));
        assert_eq!(lines[2].spans[0].content.as_ref(), "No queued prompts.");
    }

    #[test]
    fn footer_hint_line_colors_keys_and_labels() {
        // VAL-VIEW-018: footer hints render keys in the dim-orange border tone and labels in FAINT.
        let theme = ThemeColors::from_name("dark");
        let hints = vec![
            ("↵".to_owned(), "edit".to_owned()),
            ("esc".to_owned(), "close".to_owned()),
        ];
        let line = footer_hint_line(&hints, 60, &theme);
        let buf = render_lines(vec![line], 60, 1);
        assert_eq!(cell_fg(&buf, "↵"), Some(theme.border), "key glyph dim-orange");
        assert_eq!(cell_fg(&buf, "edit"), Some(theme.text_muted), "label FAINT");
        assert_eq!(cell_fg(&buf, "esc"), Some(theme.border), "esc key dim-orange");
    }
}
