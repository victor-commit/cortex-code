//! Rendering for the Cortex Code TUI: a transcript area (the streamed conversation, with a distinct
//! reasoning region and a busy indicator while a turn is in flight), a bottom prompt box, and a
//! status footer that reflects token/cost usage. Layout is recomputed every frame so the screen
//! reflows on resize, and the transcript auto-scrolls to keep the latest content visible.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};

use tui_components::borders::ROUNDED_BORDER;
use tui_components::style;
use tui_components::toast::ToastLevel;

use crate::app::{
    App, AssistantTurn, Dialog, DialogKind, DiffViewMode, DiffViewerFocus, Entry,
};
use crate::diff::FileDiff;
use crate::tool_card::{ToolCard, ToolStatus};

/// Maximum number of slash-command rows shown before the menu scrolls.
const MAX_SLASH_ROWS: usize = 10;

/// Maximum number of list rows shown inside a modal dialog before it scrolls.
const MAX_DIALOG_ROWS: usize = 14;

/// Maximum number of visible prompt text rows before the box stops growing.
const MAX_PROMPT_ROWS: usize = 8;

/// Maximum number of output lines shown inside an expanded tool card before truncation.
const MAX_TOOL_DETAIL_LINES: usize = 40;

/// Maximum drawn width of a tool card's top/bottom border.
const TOOL_CARD_WIDTH: usize = 76;

/// Width (in columns) of the session sidebar panel when shown.
const SIDEBAR_WIDTH: u16 = 30;

/// Left margin (columns) that transcript content (the `›`/`✦` author marks and the scrollback
/// `▲ N lines above` marker) is inset from the viewport edge, aligning with the MOTD card frame and
/// the status-bar zones.
const CONTENT_MARGIN: usize = 2;

/// Maximum number of toasts drawn at once in the overlay (matches the app's retention cap).
const MAX_VISIBLE_TOASTS: usize = 4;
/// Width of the left file-tree pane in the `/diff` viewer.
const DIFF_TREE_WIDTH: u16 = 32;

/// Renders the whole frame for the current [`App`] state.
pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let theme = style::ThemeColors::from_name(app.theme());

    frame.render_widget(
        Block::default().style(Style::default().bg(theme.background)),
        area,
    );

    // Reserve a left column for the sidebar when it is shown, so the transcript/prompt reflow
    // beside it; otherwise the main column spans the full width.
    let (sidebar_area, main_area) = if app.sidebar_visible() && area.width > SIDEBAR_WIDTH + 8 {
        let cols =
            Layout::horizontal([Constraint::Length(SIDEBAR_WIDTH), Constraint::Min(1)]).split(area);
        (Some(cols[0]), cols[1])
    } else {
        (None, area)
    };

    let prompt_rows = prompt_rows(app, main_area.height);
    let chunks = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(prompt_rows + 2),
        Constraint::Length(1),
    ])
    .split(main_area);

    draw_transcript(frame, app, chunks[0], &theme);
    draw_prompt(frame, app, chunks[1], &theme);
    draw_footer(frame, app, chunks[2], &theme);
    // The slash menu floats above the prompt, so it is drawn last to overlay the transcript.
    draw_slash_menu(frame, app, chunks[1], &theme);
    // The `@`-file picker also floats above the prompt (mutually exclusive with the slash menu).
    draw_mention_menu(frame, app, chunks[1], &theme);
    // The find-in-chat search bar floats above the prompt while find mode is active.
    draw_find_bar(frame, app, chunks[1], &theme);
    // A transient acknowledgement (e.g. a copy confirmation) shows just above the prompt.
    draw_ack(frame, app, chunks[1], &theme);
    // The sidebar occupies its reserved column beside the main content.
    if let Some(sidebar) = sidebar_area {
        draw_sidebar(frame, app, sidebar, &theme);
    }
    // The dedicated `/diff` viewer is a modal overlay for file-tree + hunk navigation and revert.
    draw_diff_viewer(frame, app, area, &theme);
    // A modal dialog (picker/status view) overlays everything else when open.
    draw_dialog(frame, app, area, &theme);
    // The command palette overlays everything when open (mutually exclusive with a dialog).
    draw_palette(frame, app, area, &theme);
    // The which-key overlay lists the active leader bindings while the leader chord is armed.
    draw_which_key(frame, app, area, &theme);
    // An interactive permission prompt is modal: it overlays everything (it captures every key
    // while the turn worker is blocked awaiting the decision).
    draw_permission(frame, app, area, &theme);
    // Transient toast notifications float above everything else (top-right) so they remain visible
    // over any open dialog/overlay, and auto-dismiss without disturbing the layout beneath them.
    draw_toasts(frame, app, area, &theme);
}

/// Number of text rows the prompt box should show (clamped to available height).
fn prompt_rows(app: &App, total_height: u16) -> u16 {
    let attachment_rows = if app.composer_attachments().is_empty() {
        0
    } else {
        1
    };
    let logical = app.input.line_count().max(1) + attachment_rows;
    // Leave room for at least one transcript row, the prompt borders, and the footer.
    let budget = total_height.saturating_sub(1 + 2 + 1).max(1) as usize;
    logical.min(MAX_PROMPT_ROWS).min(budget).max(1) as u16
}

/// Render the transcript area: the home screen before any exchange, otherwise the streamed
/// conversation. The view is normally pinned to the latest content, but the message-selection
/// highlight, find-match highlight, and the scroll offset let the user reach and mark earlier
/// content; the most-recent content returns into view when the scroll offset is zero.
fn draw_transcript(frame: &mut Frame, app: &App, area: Rect, theme: &style::ThemeColors) {
    let width = area.width.max(1) as usize;
    let height = area.height as usize;
    // Record the viewport so key handlers can page-scroll and locate find matches/messages
    // without re-deriving the layout.
    app.set_viewport(width, height);

    let RenderedTranscript {
        mut lines,
        entry_starts,
    } = render_transcript(app, width);

    // Mark the selected message (message-level navigation) with a full-width background band so the
    // focused message is unmistakable across snapshots.
    if let Some(selected) = app.selected_message() {
        highlight_selected(&mut lines, &entry_starts, selected, width, theme.surface[3]);
    }

    // Highlight find matches; the active match carries a distinct, bolder style so next/prev
    // navigation is visible as the emphasis moves between occurrences.
    if let Some(find) = app.find() {
        if !find.query.is_empty() {
            highlight_find(
                &mut lines,
                &find.query,
                find.active,
                theme.info,
                theme.warning,
            );
        }
    }

    // Apply the scroll offset: 0 pins to the bottom (latest), larger values reveal earlier content.
    let max_scroll = lines.len().saturating_sub(height);
    let scroll = app.scroll().min(max_scroll);
    let start = max_scroll - scroll;
    let visible: Vec<Line> = if start == 0 || height == 0 {
        // Nothing is clipped above (or there is no room): draw the window verbatim, MOTD on top.
        lines.into_iter().skip(start).take(height).collect()
    } else {
        // Earlier rows are clipped above the viewport: row 0 carries a `▲ N lines above` marker
        // (GHOST) alone, and the transcript fills the remaining rows below it with the bottom still
        // pinned. The line that would have been at the top is hidden behind the marker, so the
        // count is `start + 1`.
        let above = start + 1;
        let mut out: Vec<Line<'static>> = Vec::with_capacity(height);
        out.push(scrollback_marker(above, theme));
        out.extend(lines.into_iter().skip(start + 1).take(height - 1));
        out
    };

    frame.render_widget(
        Paragraph::new(visible).style(Style::default().fg(theme.text).bg(theme.background)),
        area,
    );
}

/// The clip-aware scrollback marker shown alone on row 0 when earlier rows are clipped off the top:
/// `▲ N lines above` in GHOST, inset by [`CONTENT_MARGIN`].
fn scrollback_marker(count: usize, theme: &style::ThemeColors) -> Line<'static> {
    let plural = if count == 1 { "line" } else { "lines" };
    Line::from(vec![
        Span::raw(" ".repeat(CONTENT_MARGIN)),
        Span::styled(
            format!("{} {count} {plural} above", crate::glyphs::SCROLLBACK_ABOVE),
            Style::default().fg(theme.ghost),
        ),
    ])
}

/// The fully rendered transcript: the pre-wrapped lines plus the first line index of each transcript
/// entry (so message-level navigation can scroll a chosen message into view).
pub struct RenderedTranscript {
    /// Every rendered transcript line, top to bottom.
    pub lines: Vec<Line<'static>>,
    /// `entry_starts[i]` is the index in `lines` where transcript entry `i` begins.
    pub entry_starts: Vec<usize>,
}

/// Build the full transcript as pre-wrapped lines (so scrolling is exact), recording where each
/// entry begins. The in-progress assistant entry grows as deltas arrive, and the busy indicator is
/// appended while a turn streams.
pub fn render_transcript(app: &App, width: usize) -> RenderedTranscript {
    let theme = style::ThemeColors::from_name(app.theme());
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut entry_starts: Vec<usize> = Vec::with_capacity(app.transcript.len());
    // MOTD-as-scrollback: the welcome card is always the top block of the single buffer (regardless
    // of transcript contents), so as messages stack below it the card scrolls/clips off the top
    // behind the `▲ N lines above` marker. The home meta panels are shown only on the pristine home
    // (no transcript, empty composer) so the typing/conversation states show the card alone.
    let pristine_home = app.transcript.is_empty() && app.input_value().trim().is_empty();
    crate::motd::push_home(&mut lines, &app.status.model, width, &theme, pristine_home);
    for entry in &app.transcript {
        entry_starts.push(lines.len());
        match entry {
            Entry::User(text) => push_user(&mut lines, text, width, &theme),
            Entry::Assistant(turn) => push_assistant(&mut lines, turn, width, app.theme(), &theme),
            Entry::Diff(diff) => push_diff(&mut lines, diff, &theme),
            Entry::Tool(card) => {
                push_tool(
                    &mut lines,
                    card,
                    width,
                    app.tool_card_expanded(card),
                    app.spinner_glyph(),
                    &theme,
                );
            }
            Entry::Notice(message) => push_notice(&mut lines, message, width, &theme),
            Entry::Shell(outcome) => push_shell(&mut lines, outcome, width, &theme),
            Entry::Help => push_help(&mut lines, app, &theme),
            Entry::Error(message) => push_error(&mut lines, message, width, &theme),
        }
    }
    if let Some(glyph) = app.spinner_glyph() {
        // The hint advertises the interrupt key; a queued-message count (messages submitted
        // mid-stream) reads as accepted even after its transient toast has dismissed.
        let mut label = format!("{glyph} Thinking… (Esc to interrupt)");
        let queued = app.queued_count();
        if queued > 0 {
            let plural = if queued == 1 { "message" } else { "messages" };
            label.push_str(&format!(" · {queued} queued {plural}"));
        }
        lines.push(Line::from(Span::styled(
            label,
            Style::default()
                .fg(theme.warning)
                .add_modifier(Modifier::BOLD),
        )));
    }
    RenderedTranscript {
        lines,
        entry_starts,
    }
}

/// Build the full transcript as pre-wrapped lines (so scrolling is exact). The in-progress
/// assistant entry grows as deltas arrive, and the busy indicator is appended while a turn streams.
pub fn transcript_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    render_transcript(app, width).lines
}

/// The first line index of each transcript entry, used to scroll a selected message into view.
pub fn entry_start_lines(app: &App, width: usize) -> Vec<usize> {
    render_transcript(app, width).entry_starts
}

/// A single find-in-chat match: the rendered line it falls on and its char span within that line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FindMatch {
    /// Index into the rendered transcript lines.
    pub line: usize,
    /// Char offset of the match within the line's plain text.
    pub start: usize,
    /// Match length in chars.
    pub len: usize,
}

/// The plain text of a rendered line (its span contents concatenated).
fn line_text(line: &Line) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

/// Find every (non-overlapping, case-insensitive) occurrence of `query` across the rendered
/// transcript lines, in top-to-bottom, left-to-right order. An empty query yields no matches.
pub fn transcript_matches(lines: &[Line], query: &str) -> Vec<FindMatch> {
    let mut out: Vec<FindMatch> = Vec::new();
    let needle: Vec<char> = query.to_lowercase().chars().collect();
    if needle.is_empty() {
        return out;
    }
    for (li, line) in lines.iter().enumerate() {
        let hay: Vec<char> = line_text(line).to_lowercase().chars().collect();
        if hay.len() < needle.len() {
            continue;
        }
        let mut i = 0;
        while i + needle.len() <= hay.len() {
            if hay[i..i + needle.len()] == needle[..] {
                out.push(FindMatch {
                    line: li,
                    start: i,
                    len: needle.len(),
                });
                i += needle.len();
            } else {
                i += 1;
            }
        }
    }
    out
}

/// Re-style the lines holding find matches: every match gets a background band and the active match
/// (the `active`-th in order) gets a brighter background plus bold, so next/prev navigation visibly
/// moves the emphasis between occurrences.
fn highlight_find(
    lines: &mut [Line<'static>],
    query: &str,
    active: usize,
    base_bg: Color,
    active_bg: Color,
) {
    let matches = transcript_matches(lines, query);
    if matches.is_empty() {
        return;
    }
    let active = active.min(matches.len() - 1);
    let mut per_line: std::collections::BTreeMap<usize, Vec<(usize, usize, bool)>> =
        std::collections::BTreeMap::new();
    for (gi, m) in matches.iter().enumerate() {
        per_line
            .entry(m.line)
            .or_default()
            .push((m.start, m.start + m.len, gi == active));
    }
    for (line_idx, ranges) in per_line {
        lines[line_idx] = apply_highlights_to_line(&lines[line_idx], &ranges, base_bg, active_bg);
    }
}

/// Rebuild one line, splitting its spans at the given char ranges and applying a highlight
/// background (the active range gets `active_bg` + bold; others get `base_bg`), while preserving
/// each span's original foreground/modifiers elsewhere.
fn apply_highlights_to_line(
    line: &Line<'static>,
    ranges: &[(usize, usize, bool)],
    base_bg: Color,
    active_bg: Color,
) -> Line<'static> {
    let highlight_at = |abs: usize| -> Option<bool> {
        ranges
            .iter()
            .find(|(s, e, _)| abs >= *s && abs < *e)
            .map(|(_, _, a)| *a)
    };

    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut offset = 0usize;
    for span in &line.spans {
        let chars: Vec<char> = span.content.chars().collect();
        let style = span.style;
        let mut i = 0;
        while i < chars.len() {
            let hl = highlight_at(offset + i);
            let mut j = i + 1;
            while j < chars.len() && highlight_at(offset + j) == hl {
                j += 1;
            }
            let text: String = chars[i..j].iter().collect();
            let st = match hl {
                Some(true) => style.bg(active_bg).add_modifier(Modifier::BOLD),
                Some(false) => style.bg(base_bg),
                None => style,
            };
            spans.push(Span::styled(text, st));
            i = j;
        }
        offset += chars.len();
    }
    Line::from(spans)
}

/// Paint a full-width background band over the lines of the selected message so message-level
/// navigation shows which message is focused. Each line keeps its text styling but gains the band
/// background, padded to the full width.
fn highlight_selected(
    lines: &mut [Line<'static>],
    entry_starts: &[usize],
    selected: usize,
    width: usize,
    bg: Color,
) {
    let Some(&begin) = entry_starts.get(selected) else {
        return;
    };
    let end = entry_starts
        .get(selected + 1)
        .copied()
        .unwrap_or(lines.len())
        .min(lines.len());
    for line in lines.iter_mut().take(end).skip(begin) {
        *line = apply_row_bg(line, bg, width);
    }
}

/// Re-style one line with a background band, padded with spaces to `width`.
fn apply_row_bg(line: &Line<'static>, bg: Color, width: usize) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = line
        .spans
        .iter()
        .map(|s| Span::styled(s.content.to_string(), s.style.bg(bg)))
        .collect();
    let used = line.width();
    if used < width {
        spans.push(Span::styled(
            " ".repeat(width - used),
            Style::default().bg(bg),
        ));
    }
    Line::from(spans)
}

/// Append a user prompt: a `›` mark (ORANGE) at the content-left column followed by the message text
/// (TXT) on the same line, with wrapped continuation lines hanging-indented under the text column.
fn push_user(lines: &mut Vec<Line<'static>>, text: &str, width: usize, theme: &style::ThemeColors) {
    let mark = Style::default().fg(theme.primary);
    let body = Style::default().fg(theme.text);
    // Content margin + `› ` (mark + space) before the text column; continuation lines hang under it.
    let text_indent = CONTENT_MARGIN + 2;
    let body_width = width.saturating_sub(text_indent).max(1);
    let wrapped = wrap(text, body_width);
    if wrapped.is_empty() {
        lines.push(Line::from(vec![
            Span::raw(" ".repeat(CONTENT_MARGIN)),
            Span::styled(format!("{} ", crate::glyphs::PROMPT), mark),
        ]));
    }
    for (i, line) in wrapped.into_iter().enumerate() {
        if i == 0 {
            lines.push(Line::from(vec![
                Span::raw(" ".repeat(CONTENT_MARGIN)),
                Span::styled(format!("{} ", crate::glyphs::PROMPT), mark),
                Span::styled(line, body),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::raw(" ".repeat(text_indent)),
                Span::styled(line, body),
            ]));
        }
    }
    lines.push(Line::from(""));
}

/// Append an assistant reply: an optional distinct reasoning region, then the answer body.
fn push_assistant(
    lines: &mut Vec<Line<'static>>,
    turn: &AssistantTurn,
    width: usize,
    theme_name: &str,
    theme: &style::ThemeColors,
) {
    // Reasoning, when present, renders in a distinct dimmed/labeled region separate from the answer.
    // When absent, nothing is emitted (no empty-reasoning artifact).
    if !turn.reasoning.trim().is_empty() {
        let label = Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::ITALIC);
        let body = Style::default()
            .fg(theme.text_dim)
            .add_modifier(Modifier::ITALIC);
        lines.push(Line::from(Span::styled("  reasoning", label)));
        // Reserve two columns for the "│ " quote bar when wrapping.
        for wrapped in wrap(&turn.reasoning, width.saturating_sub(2).max(1)) {
            lines.push(Line::from(vec![
                Span::styled("│ ", body),
                Span::styled(wrapped, body),
            ]));
        }
        lines.push(Line::from(""));
    }

    // The assistant reply leads with a `✦` author mark (AMBER) at the content-left column, inline
    // with the first line of the reply; wrapped/subsequent lines hang-indent under the text column
    // (one column past the mark). The answer is markdown-rendered so headings, emphasis, lists,
    // inline code, fenced code blocks, tables, and blockquotes display as structured elements, with
    // plain prose promoted to BRIGHT to match the reply tone. Re-rendered every frame from the
    // accumulated answer, so it updates incrementally as the turn streams.
    let mark = Style::default().fg(theme.secondary);
    let text_indent = CONTENT_MARGIN + 2;
    let body_lines: Vec<Line<'static>> = if turn.answer.trim().is_empty() {
        vec![Line::from("")]
    } else {
        let md_width = width.saturating_sub(text_indent).clamp(1, u16::MAX as usize) as u16;
        let renderer = frame::markdown::MarkdownRenderer::with_theme(
            frame::markdown::MarkdownTheme::from_name(theme_name)
                .with_text(Style::default().fg(theme.text_bright)),
        )
        .with_width(md_width);
        renderer.render(&turn.answer)
    };
    for (i, line) in body_lines.into_iter().enumerate() {
        let mut spans: Vec<Span<'static>> = Vec::with_capacity(line.spans.len() + 1);
        if i == 0 {
            spans.push(Span::raw(" ".repeat(CONTENT_MARGIN)));
            spans.push(Span::styled(format!("{} ", crate::glyphs::BRAND_STAR), mark));
        } else {
            spans.push(Span::raw(" ".repeat(text_indent)));
        }
        spans.extend(line.spans);
        lines.push(Line::from(spans));
    }
    lines.push(Line::from(""));
}

/// Append a file-change diff: a labeled marker line, then the colored unified-diff lines (file-path
/// header, `@@` hunks, and green `+` / red `-` change lines), then a trailing blank line.
fn push_diff(lines: &mut Vec<Line<'static>>, diff: &FileDiff, theme: &style::ThemeColors) {
    let verb = if diff.is_new { "created" } else { "edited" };
    lines.push(Line::from(Span::styled(
        format!("✎ {verb} ({}+ {}-)", diff.additions, diff.removals),
        Style::default().fg(theme.info).add_modifier(Modifier::BOLD),
    )));
    lines.extend(diff.render_with_theme(theme));
    lines.push(Line::from(""));
}

/// Append an agent tool call as a labeled card: a colored top border carrying the tool name and its
/// running/completed/error status, a one-line argument summary, and — when expanded — the full
/// arguments and output. A failed card is rendered in a distinct error state (red border + glyph)
/// and shows its failure message inline so the error is legible without expanding.
fn push_tool(
    lines: &mut Vec<Line<'static>>,
    card: &ToolCard,
    width: usize,
    expanded: bool,
    spinner: Option<&str>,
    theme: &style::ThemeColors,
) {
    let box_w = width.clamp(8, TOOL_CARD_WIDTH);
    let (accent, glyph, state) = match card.status {
        ToolStatus::Running => (theme.info, "⚙", "running"),
        ToolStatus::Completed => (theme.success, "✓", "done"),
        ToolStatus::Failed => (theme.error, "✗", "error"),
    };
    let border = Style::default().fg(accent);
    let border_bold = border.add_modifier(Modifier::BOLD);
    let text_style = Style::default().fg(theme.text);
    let detail_style = Style::default().fg(theme.text_dim);
    let error_style = Style::default().fg(theme.error);

    // Top border carries the card label: tool name + status (with a spinner while running).
    let mut header = format!("{glyph} {} · {state}", card.name);
    if matches!(card.status, ToolStatus::Running) {
        if let Some(s) = spinner {
            header.push(' ');
            header.push_str(s);
        }
    }
    let prefix = format!(
        "{}{} {header} ",
        ROUNDED_BORDER.top_left, ROUNDED_BORDER.horizontal_top
    );
    let fill = box_w.saturating_sub(prefix.chars().count());
    lines.push(Line::from(Span::styled(
        format!("{prefix}{}", ROUNDED_BORDER.horizontal_top.repeat(fill)),
        border_bold,
    )));

    // Body lines are prefixed with the left border bar and wrapped to the inner width.
    let bar = format!("{} ", ROUNDED_BORDER.vertical_left);
    let inner = box_w.saturating_sub(bar.chars().count()).max(1);
    let emit = |lines: &mut Vec<Line<'static>>, content: &str, st: Style| {
        for wrapped in wrap(content, inner) {
            lines.push(Line::from(vec![
                Span::styled(bar.clone(), border),
                Span::styled(wrapped, st),
            ]));
        }
    };

    emit(lines, &card.summary, text_style);

    if expanded {
        if !card.args_detail.is_empty() {
            emit(lines, "arguments", detail_style);
            for line in card.args_detail.lines() {
                emit(lines, line, text_style);
            }
        }
        if !card.output.is_empty() {
            let (label, body_style) = match card.status {
                ToolStatus::Failed => ("error", error_style),
                _ => ("output", text_style),
            };
            emit(lines, label, detail_style);
            let out_lines: Vec<&str> = card.output.lines().collect();
            let shown = out_lines.len().min(MAX_TOOL_DETAIL_LINES);
            for line in &out_lines[..shown] {
                emit(lines, line, body_style);
            }
            if out_lines.len() > shown {
                emit(
                    lines,
                    &format!("… ({} more lines)", out_lines.len() - shown),
                    detail_style,
                );
            }
        }
    } else if matches!(card.status, ToolStatus::Failed) && !card.output.is_empty() {
        // Show the failure message inline so the error is legible without expanding.
        emit(
            lines,
            card.output.lines().next().unwrap_or_default(),
            error_style,
        );
        emit(lines, "Ctrl+O for detail", detail_style);
    } else {
        emit(lines, "Ctrl+O to expand", detail_style);
    }

    lines.push(Line::from(Span::styled(
        format!(
            "{}{}",
            ROUNDED_BORDER.bottom_left,
            ROUNDED_BORDER
                .horizontal_bottom
                .repeat(box_w.saturating_sub(1))
        ),
        border,
    )));
    lines.push(Line::from(""));
}

/// Append a system notice (e.g. a dispatched slash command or an unknown-command message) as a
/// distinct, muted line set apart from chat content.
fn push_notice(
    lines: &mut Vec<Line<'static>>,
    message: &str,
    width: usize,
    theme: &style::ThemeColors,
) {
    let style = Style::default().fg(theme.info);
    let body: Vec<String> = wrap(message, width.saturating_sub(2).max(1));
    let mut first = true;
    for wrapped in body {
        let marker = row_marker(first);
        first = false;
        lines.push(Line::from(vec![
            Span::styled(marker, style),
            Span::styled(wrapped, style),
        ]));
    }
    lines.push(Line::from(""));
}

/// Append a `!` shell-mode command and its output: a `$ command` header (distinct from chat) then
/// the captured output, dimmed on success and red-styled when the command failed.
fn push_shell(
    lines: &mut Vec<Line<'static>>,
    outcome: &crate::shell::ShellOutcome,
    width: usize,
    theme: &style::ThemeColors,
) {
    let prompt_style = Style::default()
        .fg(theme.warning)
        .add_modifier(Modifier::BOLD);
    lines.push(Line::from(vec![
        Span::styled("$ ", prompt_style),
        Span::styled(
            outcome.command.clone(),
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
        ),
    ]));
    let body = if outcome.success {
        Style::default().fg(theme.text_dim)
    } else {
        Style::default().fg(theme.error)
    };
    for wrapped in wrap(&outcome.output, width) {
        lines.push(Line::from(Span::styled(wrapped, body)));
    }
    lines.push(Line::from(""));
}

/// The keybindings shown in the `/help` reference, paired with what each does.
const KEYBINDINGS: &[(&str, &str)] = &[
    ("Enter", "Send the prompt / run the highlighted command"),
    ("Shift+Enter", "Insert a newline without sending"),
    ("!", "Enter shell mode to run a command (Esc to exit)"),
    ("/", "Open the slash-command menu"),
    ("Up / Down", "Move the slash-menu highlight"),
    ("Esc", "Close the slash-command menu"),
    ("Ctrl+A / Ctrl+E", "Jump to line start / end"),
    ("Ctrl+O", "Expand / collapse tool cards"),
    ("Ctrl+F", "Find in chat (Enter/Up next/prev, Esc close)"),
    ("PageUp / PageDown", "Scroll the transcript up / down"),
    ("Ctrl+K / Ctrl+N", "Select previous / next message"),
    ("Ctrl+Y", "Copy the selected message"),
    ("Ctrl+C", "Quit Cortex Code"),
];

/// Append the `/help` command reference: a titled panel listing every currently-discoverable slash
/// command (built-ins, config custom commands, and MCP prompt commands) with its description, then
/// the keybindings (including the active leader bindings).
/// This is the visible effect of `/help`.
fn push_help(lines: &mut Vec<Line<'static>>, app: &App, theme: &style::ThemeColors) {
    let title = Style::default().fg(theme.info).add_modifier(Modifier::BOLD);
    let section = Style::default()
        .fg(theme.warning)
        .add_modifier(Modifier::BOLD);
    let name = Style::default().fg(theme.text).add_modifier(Modifier::BOLD);
    let desc = Style::default().fg(theme.text_dim);

    lines.push(Line::from(Span::styled(
        "Help — commands & keybindings",
        title,
    )));
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled("Commands", section)));
    for (name_text, description_text) in app.slash_help_entries() {
        lines.push(Line::from(vec![
            Span::styled(format!("  /{:<18}", name_text), name),
            Span::styled(format!("  {}", description_text), desc),
        ]));
    }
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled("Keybindings", section)));
    for (keys, what) in KEYBINDINGS {
        lines.push(Line::from(vec![
            Span::styled(format!("  {keys:<16}"), name),
            Span::styled(format!("  {what}"), desc),
        ]));
    }
    lines.push(Line::from(""));

    let leader = app.keybinds().leader.display();
    lines.push(Line::from(Span::styled(
        format!("Leader bindings ({leader} then a key)"),
        section,
    )));
    for (chord, action) in app.keybinds().entries() {
        let mut desc_text = action.label().to_owned();
        if let Some(reason) = app.keybind_disabled_reason(*action) {
            desc_text.push_str(&format!(" (disabled: {reason})"));
        }
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {:<16}", format!("{leader} {}", chord.display())),
                name,
            ),
            Span::styled(format!("  {desc_text}"), desc),
        ]));
    }
    if !app.keybind_diagnostics().is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("Keybind diagnostics", section)));
        for message in app.keybind_diagnostics() {
            lines.push(Line::from(vec![
                Span::styled("  • ", desc),
                Span::styled(message.clone(), Style::default().fg(theme.warning)),
            ]));
        }
    }
    lines.push(Line::from(""));
}

/// Append a turn-level error as a distinct, red-styled block.
fn push_error(
    lines: &mut Vec<Line<'static>>,
    message: &str,
    width: usize,
    theme: &style::ThemeColors,
) {
    let style = Style::default()
        .fg(theme.error)
        .add_modifier(Modifier::BOLD);
    lines.push(Line::from(Span::styled("✗ error", style)));
    for wrapped in wrap(message, width) {
        lines.push(Line::from(Span::styled(
            wrapped,
            Style::default().fg(theme.error),
        )));
    }
    lines.push(Line::from(""));
}

/// Word-wrap `text` to `width` columns, preserving explicit line breaks and hard-splitting words
/// longer than the available width. A non-positive width yields the text split only on newlines.
fn wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return text.split('\n').map(str::to_owned).collect();
    }
    let mut out: Vec<String> = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut current = String::new();
        for word in paragraph.split(' ') {
            if word.chars().count() > width {
                // Flush the current line, then hard-split the over-long word.
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
                let mut chunk = String::new();
                for ch in word.chars() {
                    if chunk.chars().count() == width {
                        out.push(std::mem::take(&mut chunk));
                    }
                    chunk.push(ch);
                }
                current = chunk;
                continue;
            }
            let projected = if current.is_empty() {
                word.chars().count()
            } else {
                current.chars().count() + 1 + word.chars().count()
            };
            if projected > width {
                out.push(std::mem::take(&mut current));
                current.push_str(word);
            } else {
                if !current.is_empty() {
                    current.push(' ');
                }
                current.push_str(word);
            }
        }
        out.push(current);
    }
    out
}

fn draw_prompt(frame: &mut Frame, app: &App, area: Rect, theme: &style::ThemeColors) {
    // Shell mode wears a distinct title and border color so it is unmistakably set apart from chat.
    let shell = app.shell_mode();
    // The composer is a plain rounded `ORANGE_D` box on the brand canvas (no title); shell mode is
    // the one exception, wearing a distinct title + warning border so it is unmistakably set apart.
    let mut block = Block::default()
        .style(Style::default().bg(theme.background).fg(theme.text))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border));
    if shell {
        block = block
            .title(" ! shell ")
            .title_style(
                Style::default()
                    .fg(theme.warning)
                    .add_modifier(Modifier::BOLD),
            )
            .border_style(Style::default().fg(theme.warning));
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let (attachment_area, input_area) = if app.composer_attachments().is_empty() {
        (None, inner)
    } else if inner.height > 1 {
        let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(inner);
        (Some(rows[0]), rows[1])
    } else {
        (Some(inner), inner)
    };

    if let Some(attachment_area) = attachment_area {
        let chips = app
            .composer_attachments()
            .iter()
            .map(|attachment| format!("📎 {}", attachment.chip_label()))
            .collect::<Vec<_>>()
            .join("  ");
        frame.render_widget(
            Paragraph::new(Span::styled(
                chips,
                Style::default().fg(theme.info).add_modifier(Modifier::BOLD),
            )),
            attachment_area,
        );
    }

    // Composer accent edge `▌` + `›` prompt glyph at the left inner column (both ORANGE).
    let accent = Style::default().fg(theme.primary);
    let prefix_w = inner.width.min(3);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(crate::glyphs::ACCENT_BAR, accent),
            Span::raw(" "),
            Span::styled(crate::glyphs::PROMPT, accent),
        ])),
        Rect {
            x: inner.x,
            y: input_area.y,
            width: prefix_w,
            height: 1,
        },
    );

    // Right-aligned FAINT action hint on the input row (reserved out of the text region). The hint
    // tracks the composer's active state: a shell command or an open `/` palette both submit with
    // `↵`, an open `@` palette inserts the highlighted file with `⇥`, and the plain chat composer
    // sends with `↵`.
    let hint = if shell || app.slash_menu_open() {
        format!("{} run   esc dismiss", crate::glyphs::KEY_ENTER)
    } else if app.mention_open() {
        format!("{} insert   esc dismiss", crate::glyphs::KEY_TAB)
    } else {
        format!("{} send", crate::glyphs::KEY_ENTER)
    };
    let hint_w = hint.chars().count() as u16;
    let hint_reserve = if inner.width > hint_w + 8 {
        frame.render_widget(
            Paragraph::new(Span::styled(
                hint,
                Style::default().fg(theme.text_muted),
            )),
            Rect {
                x: inner.x + inner.width - hint_w,
                y: input_area.y,
                width: hint_w,
                height: 1,
            },
        );
        hint_w + 1
    } else {
        0
    };

    // The text region begins after the `▌ › ` prefix (4 columns) so typed text and the caret sit at
    // the input column and continuation lines hang under it.
    let prefix = 4u16;
    let text_x = inner.x + prefix.min(inner.width);
    let text_w = inner.width.saturating_sub(prefix + hint_reserve).max(1);
    let input_area = Rect {
        x: text_x,
        y: input_area.y,
        width: text_w,
        height: input_area.height,
    };

    let value = app.input_value();
    if value.is_empty() {
        let placeholder = if shell {
            "run a shell command…"
        } else if app.transcript.is_empty() {
            crate::motd::HOME_PLACEHOLDER
        } else {
            "reply to cortex…"
        };
        frame.render_widget(
            Paragraph::new(Span::styled(
                placeholder,
                Style::default().fg(theme.text_muted),
            )),
            input_area,
        );
        // An empty composer keeps the hardware cursor at the input start so a caret is still shown.
        frame.set_cursor_position((input_area.x, input_area.y));
    } else {
        // Typed text renders in BRIGHT (replacing the FAINT placeholder).
        let bright = Style::default().fg(theme.text_bright);
        let lines: Vec<Line> = app
            .input
            .lines()
            .iter()
            .map(|l| Line::from(Span::styled(l.clone(), bright)))
            .collect();
        frame.render_widget(
            Paragraph::new(lines).style(Style::default().bg(theme.background)),
            input_area,
        );
        // A `▏` caret (ORANGE) sits at the cursor — immediately after the last typed character with
        // no gap. It is gated by the blink state (solid under reduced motion / when idle), and the
        // hardware cursor is left unset so exactly one caret is drawn.
        if app.cursor_blink_visible() {
            let cursor = app.cursor();
            let cx = input_area.x + (cursor.col as u16).min(input_area.width.saturating_sub(1));
            let cy = input_area.y + (cursor.row as u16).min(input_area.height.saturating_sub(1));
            frame.render_widget(
                Paragraph::new(Span::styled(
                    crate::glyphs::CURSOR,
                    Style::default().fg(theme.primary),
                )),
                Rect {
                    x: cx,
                    y: cy,
                    width: 1,
                    height: 1,
                },
            );
        }
    }
}

/// The two-column leading marker for a list/menu/dialog row: the brand `›` selection marker
/// ([`glyphs::SELECT_MARKER`]) on a highlighted row (drawn on the inverse selection bar), or two
/// blanks otherwise (see [`crate::glyphs::SELECT_MARKER`]). The legacy thin disclosure triangle is
/// never used (it antialiases to a thin mark / tofu).
fn row_marker(selected: bool) -> String {
    if selected {
        format!("{} ", crate::glyphs::SELECT_MARKER)
    } else {
        "  ".to_string()
    }
}

/// The style for a highlighted picker/list row: a solid inverse pair (an explicit `selection_fg`
/// drawn on an explicit `selection_bg`), never `Modifier::REVERSED`, so a selection reads as a
/// solid fill with a contrasting foreground in a snapshot.
fn selected_row_style(theme: &style::ThemeColors) -> Style {
    Style::default()
        .fg(theme.selection_fg)
        .bg(theme.selection_bg)
        .add_modifier(Modifier::BOLD)
}

/// Build the slash-command menu's rows for the current query, or `None` when the menu is closed.
/// Each row shows `/name  description`; the highlighted row carries a `›` marker and bold name.
/// A non-empty query that matches nothing yields a single graceful "no matching commands" row.
fn slash_menu_lines(app: &App) -> Option<Vec<Line<'static>>> {
    let theme = style::ThemeColors::from_name(app.theme());
    let _ = app.slash_query()?;
    let matches = app.slash_matches();
    if matches.is_empty() {
        return Some(vec![Line::from(Span::styled(
            "  no matching commands",
            Style::default()
                .fg(theme.text_dim)
                .add_modifier(Modifier::ITALIC),
        ))]);
    }

    let selected = app.slash_selected();
    let mut lines = Vec::with_capacity(matches.len());
    for (i, command) in matches.iter().enumerate() {
        let is_selected = i == selected;
        let marker = row_marker(is_selected);
        if is_selected {
            let sel = selected_row_style(&theme);
            lines.push(Line::from(vec![
                Span::styled(marker, sel),
                Span::styled(format!("/{}", command.name), sel),
                Span::styled("  ", sel),
                Span::styled(command.description.to_owned(), sel),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::styled(marker, Style::default().fg(theme.info)),
                Span::styled(format!("/{}", command.name), Style::default().fg(theme.text)),
                Span::raw("  "),
                Span::styled(
                    command.description.to_owned(),
                    Style::default().fg(theme.text_dim),
                ),
            ]));
        }
    }
    Some(lines)
}

/// Render the slash-command autocomplete menu as a bordered popup floating directly above the
/// prompt box. The view scrolls to keep the highlighted row visible when the match list is long.
fn draw_slash_menu(frame: &mut Frame, app: &App, prompt_area: Rect, theme: &style::ThemeColors) {
    let Some(rows) = slash_menu_lines(app) else {
        return;
    };
    if prompt_area.width == 0 {
        return;
    }

    // Keep the highlighted row inside the visible window.
    let selected = app.slash_selected();
    let visible = rows.len().min(MAX_SLASH_ROWS);
    let offset = if selected < visible {
        0
    } else {
        selected + 1 - visible
    };
    let shown: Vec<Line<'static>> = rows[offset..offset + visible].to_vec();

    let height = (visible as u16).saturating_add(2);
    let width = prompt_area.width;
    let y = prompt_area.y.saturating_sub(height);
    let popup = Rect {
        x: prompt_area.x,
        y,
        width,
        height: height.min(prompt_area.y.saturating_sub(y).max(height)),
    };

    let block = Block::default()
        .style(Style::default().bg(theme.surface[1]))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.info))
        .title(" / commands ")
        .title_style(Style::default().fg(theme.info).add_modifier(Modifier::BOLD));
    let inner = block.inner(popup);

    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);
    frame.render_widget(
        Paragraph::new(shown).style(Style::default().fg(theme.text).bg(theme.surface[1])),
        inner,
    );
}

/// Build the `@`-file picker's rows for the current query, or `None` when the picker is closed.
/// Each row shows a file path; the highlighted row carries a `›` marker and bold text. A query that
/// matches no file yields a single graceful "no matching files" row.
fn mention_menu_lines(app: &App) -> Option<Vec<Line<'static>>> {
    let theme = style::ThemeColors::from_name(app.theme());
    if !app.mention_open() {
        return None;
    }
    let matches = app.mention_matches();
    if matches.is_empty() {
        return Some(vec![Line::from(Span::styled(
            "  no matching files",
            Style::default()
                .fg(theme.text_dim)
                .add_modifier(Modifier::ITALIC),
        ))]);
    }

    let selected = app.mention_selected();
    let mut lines = Vec::with_capacity(matches.len());
    for (i, path) in matches.iter().enumerate() {
        let is_selected = i == selected;
        let marker = row_marker(is_selected);
        if is_selected {
            let sel = selected_row_style(&theme);
            lines.push(Line::from(vec![
                Span::styled(marker, sel),
                Span::styled(path.clone(), sel),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::styled(marker, Style::default().fg(theme.info)),
                Span::styled(path.clone(), Style::default().fg(theme.text)),
            ]));
        }
    }
    Some(lines)
}

/// Render the `@`-file picker as a bordered popup floating directly above the prompt box, scrolling
/// to keep the highlighted row visible when the match list is long.
fn draw_mention_menu(frame: &mut Frame, app: &App, prompt_area: Rect, theme: &style::ThemeColors) {
    let Some(rows) = mention_menu_lines(app) else {
        return;
    };
    if prompt_area.width == 0 {
        return;
    }

    let selected = app.mention_selected();
    let visible = rows.len().min(MAX_SLASH_ROWS);
    let offset = if selected < visible {
        0
    } else {
        selected + 1 - visible
    };
    let shown: Vec<Line<'static>> = rows[offset..offset + visible].to_vec();

    let height = (visible as u16).saturating_add(2);
    let width = prompt_area.width;
    let y = prompt_area.y.saturating_sub(height);
    let popup = Rect {
        x: prompt_area.x,
        y,
        width,
        height: height.min(prompt_area.y.saturating_sub(y).max(height)),
    };

    let block = Block::default()
        .style(Style::default().bg(theme.surface[1]))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.info))
        .title(" @ files ")
        .title_style(Style::default().fg(theme.info).add_modifier(Modifier::BOLD));
    let inner = block.inner(popup);

    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);
    frame.render_widget(
        Paragraph::new(shown).style(Style::default().fg(theme.text).bg(theme.surface[1])),
        inner,
    );
}

/// Render the find-in-chat search bar as a single-row bordered popup floating above the prompt. It
/// shows the live query and a `match N of M` indicator (or `no matches` / a hint), so the search
/// affordance and the active-match position are both visible in a snapshot.
fn draw_find_bar(frame: &mut Frame, app: &App, prompt_area: Rect, theme: &style::ThemeColors) {
    let Some(find) = app.find() else {
        return;
    };
    if prompt_area.width == 0 {
        return;
    }

    let (active, total) = app.find_status();
    let status = if find.query.is_empty() {
        "type to search".to_owned()
    } else if total == 0 {
        "no matches".to_owned()
    } else {
        format!("match {} of {}", active + 1, total)
    };

    let row = Line::from(vec![
        Span::styled(
            "Find: ",
            Style::default().fg(theme.info).add_modifier(Modifier::BOLD),
        ),
        Span::styled(find.query.clone(), Style::default().fg(theme.text)),
        Span::raw("  "),
        Span::styled(status, Style::default().fg(theme.text_dim)),
    ]);

    let height = 3u16;
    let width = prompt_area.width;
    let y = prompt_area.y.saturating_sub(height);
    let popup = Rect {
        x: prompt_area.x,
        y,
        width,
        height: height.min(prompt_area.y.saturating_sub(y).max(height)),
    };

    let block = Block::default()
        .style(Style::default().bg(theme.surface[1]))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.info))
        .title(" find ")
        .title_style(Style::default().fg(theme.info).add_modifier(Modifier::BOLD));
    let inner = block.inner(popup);

    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);
    frame.render_widget(
        Paragraph::new(row).style(Style::default().fg(theme.text).bg(theme.surface[1])),
        inner,
    );
}

/// Render a transient acknowledgement (e.g. a copy confirmation) as a single highlighted row
/// directly above the prompt. Shown only in ordinary mode (never while the find bar is open) so the
/// two affordances do not collide.
fn draw_ack(frame: &mut Frame, app: &App, prompt_area: Rect, theme: &style::ThemeColors) {
    if app.find().is_some() {
        return;
    }
    let Some(message) = app.ack() else {
        return;
    };
    if prompt_area.width == 0 || prompt_area.y == 0 {
        return;
    }

    let area = Rect {
        x: prompt_area.x,
        y: prompt_area.y - 1,
        width: prompt_area.width,
        height: 1,
    };
    let line = Line::from(Span::styled(
        format!("✓ {message}"),
        Style::default()
            .fg(theme.success)
            .add_modifier(Modifier::BOLD),
    ));
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(line).style(Style::default().bg(theme.background)),
        area,
    );
}

/// The fg color and human label for a toast level. Error toasts use the error color (a distinct red)
/// so an error notification is unmistakable from a success/info one; the level is also encoded in the
/// ASCII icon and the label text, so the distinction survives a monochrome snapshot.
fn toast_decor(level: ToastLevel, theme: &style::ThemeColors) -> (Color, &'static str) {
    match level {
        ToastLevel::Success => (theme.success, "Success"),
        ToastLevel::Info => (theme.info, "Info"),
        ToastLevel::Warning => (theme.warning, "Warning"),
        ToastLevel::Error => (theme.error, "Error"),
    }
}

/// Pad with trailing spaces (or truncate) so the string occupies exactly `width` columns. Used so a
/// toast row paints a solid, fixed-width band regardless of message length.
fn pad_or_truncate(text: &str, width: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() >= width {
        chars[..width].iter().collect()
    } else {
        let mut out: String = chars.iter().collect();
        out.extend(std::iter::repeat_n(' ', width - chars.len()));
        out
    }
}

/// Render a depleting progress bar `width` columns wide for the given remaining fraction (1.0 = full,
/// 0.0 = empty), using eighth-block glyphs for sub-cell resolution so the bar visibly shrinks every
/// frame. The continuous change is also what keeps the toast "busy" until it auto-dismisses, so a
/// `wait-idle` observer settles exactly when the toast disappears rather than while it is still shown.
fn toast_bar(fraction: f32, width: usize) -> String {
    const EIGHTHS: [char; 9] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉', '█'];
    let fraction = fraction.clamp(0.0, 1.0);
    let total = (fraction * width as f32 * 8.0).round() as usize;
    let full = (total / 8).min(width);
    let mut out = String::with_capacity(width);
    for _ in 0..full {
        out.push('█');
    }
    if full < width {
        out.push(EIGHTHS[total % 8]);
        for _ in (full + 1)..width {
            out.push(' ');
        }
    }
    out
}

/// Render the active toasts as an auto-dismissing overlay stacked at the top-right corner. Each toast
/// is a two-row band — a colored header (`[icon] Level: message`) over a depleting progress bar — so
/// it reads clearly as a transient notification, with error toasts styled distinctly (error color +
/// `[x]` icon + `Error` label). Drawn with `Clear` over the underlying cells, so once a toast expires
/// the layout beneath is repainted intact.
fn draw_toasts(frame: &mut Frame, app: &App, area: Rect, theme: &style::ThemeColors) {
    let toasts = app.toasts();
    if toasts.is_empty() || area.width < 18 || area.height < 4 {
        return;
    }

    let bg = theme.surface[2];
    let max_width = ((area.width as usize) * 2 / 5)
        .clamp(24, 54)
        .min(area.width.saturating_sub(2) as usize);

    let mut y = area.y;
    for toast in toasts.iter().take(MAX_VISIBLE_TOASTS) {
        // Two content rows plus a one-row gap; stop before overrunning the frame.
        if y + 2 > area.y + area.height {
            break;
        }

        let (color, label) = toast_decor(toast.level, theme);
        let header_text = format!(" {} {label}: {} ", toast.level.icon(), toast.message);
        let width = header_text.chars().count().clamp(12, max_width);
        let header = pad_or_truncate(&header_text, width);

        let duration = toast.duration.as_millis().max(1) as f32;
        let remaining = toast.remaining().as_millis() as f32;
        let bar = toast_bar(remaining / duration, width);

        let x = area.x + area.width.saturating_sub(width as u16 + 1);
        let header_area = Rect {
            x,
            y,
            width: width as u16,
            height: 1,
        };
        let bar_area = Rect {
            x,
            y: y + 1,
            width: width as u16,
            height: 1,
        };

        frame.render_widget(Clear, header_area);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                header,
                Style::default()
                    .fg(color)
                    .bg(bg)
                    .add_modifier(Modifier::BOLD),
            ))),
            header_area,
        );
        frame.render_widget(Clear, bar_area);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                bar,
                Style::default().fg(color).bg(bg),
            ))),
            bar_area,
        );

        y += 3;
    }
}

/// Build a modal dialog's item rows: each selectable item with a `›` marker and bold name on the
/// highlighted row, or a single italic empty-state row when the list is empty (e.g. the MCP
/// "no servers configured" view).
fn dialog_lines(dialog: &Dialog, theme: &style::ThemeColors) -> Vec<Line<'static>> {
    if dialog.items.is_empty() {
        let message = dialog
            .empty
            .clone()
            .unwrap_or_else(|| "(nothing to show)".to_owned());
        return vec![Line::from(Span::styled(
            format!("  {message}"),
            Style::default()
                .fg(theme.text_dim)
                .add_modifier(Modifier::ITALIC),
        ))];
    }

    let selected = dialog.selected();
    let mut lines = Vec::with_capacity(dialog.items.len());
    for (i, item) in dialog.items.iter().enumerate() {
        let is_selected = i == selected;
        let marker = row_marker(is_selected);
        if is_selected {
            let sel = selected_row_style(theme);
            lines.push(Line::from(vec![
                Span::styled(marker, sel),
                Span::styled(item.clone(), sel),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::styled(marker, Style::default().fg(theme.info)),
                Span::styled(item.clone(), Style::default().fg(theme.text)),
            ]));
        }
    }
    lines
}

/// Render the `/diff` viewer overlay: a file tree on the left and a per-file diff pane on the
/// right, with unified/split toggle, hunk navigation, and revert hinting.
fn draw_diff_viewer(frame: &mut Frame, app: &App, area: Rect, theme: &style::ThemeColors) {
    let Some(viewer) = app.diff_viewer() else {
        return;
    };
    if area.width < 20 || area.height < 10 {
        return;
    }

    let width = (((area.width as usize) * 94) / 100).clamp(60, area.width as usize) as u16;
    let height = (((area.height as usize) * 88) / 100).clamp(14, area.height as usize) as u16;
    let popup = centered_rect(width, height, area);
    let block = Block::default()
        .style(Style::default().bg(theme.surface[1]))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_focus))
        .title(" Diff viewer ")
        .title_style(
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        );
    let inner = block.inner(popup);
    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);
    if inner.width < 8 || inner.height < 4 {
        return;
    }

    let tree_width = DIFF_TREE_WIDTH.min(inner.width.saturating_sub(8)).max(20);
    let cols =
        Layout::horizontal([Constraint::Length(tree_width), Constraint::Min(1)]).split(inner);

    draw_diff_file_tree(frame, viewer, cols[0], theme);
    draw_diff_file_pane(frame, viewer, cols[1], theme);
}

fn draw_diff_file_tree(
    frame: &mut Frame,
    viewer: &crate::app::DiffViewer,
    area: Rect,
    theme: &style::ThemeColors,
) {
    if area.width < 4 || area.height < 3 {
        return;
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border))
        .title(" Files ");
    let inner = block.inner(area);

    let mut rows: Vec<Line<'static>> = if viewer.files.is_empty() {
        vec![Line::from(Span::styled(
            "  (no file diffs)",
            Style::default()
                .fg(theme.text_dim)
                .add_modifier(Modifier::ITALIC),
        ))]
    } else {
        viewer
            .files
            .iter()
            .enumerate()
            .map(|(index, file)| {
                let marker = row_marker(index == viewer.selected_file);
                let label = format!(
                    "{} (+{} -{}, {} hunk{})",
                    file.path,
                    file.additions,
                    file.removals,
                    file.hunks.len(),
                    if file.hunks.len() == 1 { "" } else { "s" }
                );
                let active_focus =
                    viewer.focus == DiffViewerFocus::Files && index == viewer.selected_file;
                let style = if index == viewer.selected_file {
                    Style::default().fg(theme.info).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.text)
                };
                let row_style = if active_focus {
                    style.bg(theme.surface[2])
                } else {
                    style
                };
                Line::from(vec![
                    Span::styled(marker, Style::default().fg(theme.info)),
                    Span::styled(label, row_style),
                ])
            })
            .collect()
    };

    let budget = inner.height.saturating_sub(1) as usize;
    let visible = budget.max(1).min(rows.len().max(1));
    let selected = viewer.selected_file.min(rows.len().saturating_sub(1));
    let offset = selected.saturating_sub(visible.saturating_sub(1));
    let end = (offset + visible).min(rows.len());
    rows = rows[offset..end].to_vec();
    rows.push(Line::from(Span::styled(
        "  ↑↓ file · Tab focus",
        Style::default().fg(theme.text_dim),
    )));

    frame.render_widget(block, area);
    frame.render_widget(
        Paragraph::new(rows).style(Style::default().fg(theme.text).bg(theme.surface[1])),
        inner,
    );
}

fn draw_diff_file_pane(
    frame: &mut Frame,
    viewer: &crate::app::DiffViewer,
    area: Rect,
    theme: &style::ThemeColors,
) {
    if area.width < 6 || area.height < 4 {
        return;
    }
    let Some(file) = viewer.selected_file() else {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border))
            .title(" Diff ");
        let inner = block.inner(area);
        frame.render_widget(block, area);
        frame.render_widget(
            Paragraph::new("No selected diff").style(Style::default().fg(theme.text_dim)),
            inner,
        );
        return;
    };

    let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(area);
    let mode = match viewer.mode {
        DiffViewMode::Unified => "Unified",
        DiffViewMode::Split => "Split",
    };
    let hunk_count = file.hunks.len();
    let hunk_label = if hunk_count == 0 {
        "No hunks".to_owned()
    } else {
        format!("Hunk {}/{}", viewer.selected_hunk + 1, hunk_count)
    };
    let focus_label = match viewer.focus {
        DiffViewerFocus::Files => "Files",
        DiffViewerFocus::Hunks => "Hunks",
    };
    let status = format!(
        "{mode} · {hunk_label} · focus {focus_label} · ↑↓ navigate · Tab switch · v mode · r revert · Esc close"
    );
    frame.render_widget(
        Paragraph::new(status).style(
            Style::default()
                .fg(theme.text_dim)
                .bg(theme.surface[1])
                .add_modifier(Modifier::ITALIC),
        ),
        rows[0],
    );

    match viewer.mode {
        DiffViewMode::Unified => draw_unified_diff_pane(frame, viewer, file, rows[1], theme),
        DiffViewMode::Split => draw_split_diff_pane(frame, viewer, file, rows[1], theme),
    }
}

fn draw_unified_diff_pane(
    frame: &mut Frame,
    viewer: &crate::app::DiffViewer,
    file: &FileDiff,
    area: Rect,
    theme: &style::ThemeColors,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border))
        .title(format!(" {} ", file.path));
    let inner = block.inner(area);
    let mut lines = file.render_with_theme(theme);
    let ranges = file.hunk_line_ranges();
    let selected_start = ranges
        .get(viewer.selected_hunk)
        .map_or(0, |range| range.start);
    if let Some(range) = ranges.get(viewer.selected_hunk) {
        let highlight = if viewer.focus == DiffViewerFocus::Hunks {
            theme.surface[2]
        } else {
            theme.surface[1]
        };
        for index in range.clone() {
            if let Some(line) = lines.get(index).cloned() {
                lines[index] =
                    apply_row_bg(&line, highlight, inner.width.saturating_sub(1) as usize);
            }
        }
    }
    let budget = inner.height as usize;
    let visible = budget.max(1).min(lines.len().max(1));
    let offset = selected_start.saturating_sub(visible.saturating_sub(1));
    let end = (offset + visible).min(lines.len());
    let shown: Vec<Line<'static>> = lines[offset..end].to_vec();

    frame.render_widget(block, area);
    frame.render_widget(
        Paragraph::new(shown).style(Style::default().fg(theme.text).bg(theme.surface[1])),
        inner,
    );
}

fn draw_split_diff_pane(
    frame: &mut Frame,
    viewer: &crate::app::DiffViewer,
    file: &FileDiff,
    area: Rect,
    theme: &style::ThemeColors,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border))
        .title(format!(" {} ", file.path));
    let inner = block.inner(area);
    let panes =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(inner);

    let rows = file.split_rows();
    let ranges = file.split_hunk_row_ranges();
    let selected_start = ranges
        .get(viewer.selected_hunk)
        .map_or(0, |range| range.start);
    let budget = panes[0].height as usize;
    let visible = budget.max(1).min(rows.len().max(1));
    let offset = selected_start.saturating_sub(visible.saturating_sub(1));
    let end = (offset + visible).min(rows.len());
    let focus_highlight = if viewer.focus == DiffViewerFocus::Hunks {
        theme.surface[2]
    } else {
        theme.surface[1]
    };

    let mut left_lines = Vec::new();
    let mut right_lines = Vec::new();
    for (index, row) in rows[offset..end].iter().enumerate() {
        let absolute = offset + index;
        let selected = ranges
            .get(viewer.selected_hunk)
            .is_some_and(|range| range.contains(&absolute));
        let left_style = split_style(row.kind, true, theme);
        let right_style = split_style(row.kind, false, theme);
        let mut left = Line::from(Span::styled(
            prefix_split_text(&row.left, row.kind, true),
            left_style,
        ));
        let mut right = Line::from(Span::styled(
            prefix_split_text(&row.right, row.kind, false),
            right_style,
        ));
        if selected {
            left = apply_row_bg(
                &left,
                focus_highlight,
                panes[0].width.saturating_sub(2) as usize,
            );
            right = apply_row_bg(
                &right,
                focus_highlight,
                panes[1].width.saturating_sub(2) as usize,
            );
        }
        left_lines.push(left);
        right_lines.push(right);
    }

    frame.render_widget(block, area);
    let left_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .title(" Before ");
    let right_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .title(" After ");
    frame.render_widget(
        Paragraph::new(left_lines).style(Style::default().bg(theme.surface[1])),
        left_block.inner(panes[0]),
    );
    frame.render_widget(
        Paragraph::new(right_lines).style(Style::default().bg(theme.surface[1])),
        right_block.inner(panes[1]),
    );
    frame.render_widget(left_block, panes[0]);
    frame.render_widget(right_block, panes[1]);
}

fn split_style(kind: crate::diff::DiffLineKind, left: bool, theme: &style::ThemeColors) -> Style {
    match kind {
        crate::diff::DiffLineKind::Header => {
            Style::default().fg(theme.info).add_modifier(Modifier::BOLD)
        }
        crate::diff::DiffLineKind::Context => Style::default().fg(theme.text_dim),
        crate::diff::DiffLineKind::Removed => {
            if left {
                Style::default().fg(theme.error)
            } else {
                Style::default().fg(theme.text_dim)
            }
        }
        crate::diff::DiffLineKind::Added => {
            if left {
                Style::default().fg(theme.text_dim)
            } else {
                Style::default().fg(theme.success)
            }
        }
    }
}

fn prefix_split_text(text: &str, kind: crate::diff::DiffLineKind, left: bool) -> String {
    match kind {
        crate::diff::DiffLineKind::Header => text.to_owned(),
        crate::diff::DiffLineKind::Context => format!("  {text}"),
        crate::diff::DiffLineKind::Removed => {
            if left {
                format!("- {text}")
            } else {
                String::new()
            }
        }
        crate::diff::DiffLineKind::Added => {
            if left {
                String::new()
            } else {
                format!("+ {text}")
            }
        }
    }
}

/// Render the open modal dialog as a centered bordered popup over the whole frame. The list scrolls
/// to keep the highlighted row visible, and a footer hint reminds the user how to navigate/close.
fn draw_dialog(frame: &mut Frame, app: &App, area: Rect, theme: &style::ThemeColors) {
    let Some(dialog) = app.dialog() else {
        return;
    };
    if area.width < 8 || area.height < 6 {
        return;
    }

    let rows = dialog_lines(dialog, theme);
    let widest = rows.iter().map(Line::width).max().unwrap_or(0);
    // The session picker, when renaming, shows the in-progress new title above the hint.
    let rename = (dialog.kind == DialogKind::Sessions)
        .then(|| app.session_rename())
        .flatten();
    let hint = dialog_hint(dialog.kind, rename.is_some());
    let content_w = widest
        .max(hint.chars().count() + 2)
        .max(rename.map_or(0, |text| text.chars().count() + 12));
    let max_w = (area.width as usize) * 9 / 10;
    let width = (content_w + 4).clamp(24, max_w.max(24)) as u16;

    let row_budget = (area.height as usize)
        .saturating_sub(4)
        .clamp(1, MAX_DIALOG_ROWS);
    let visible = rows.len().min(row_budget).max(1);
    // Borders (2) + the hint row (1) on top of the visible item rows, plus the rename input row
    // (1) while renaming.
    let extra = u16::from(rename.is_some());
    let height = (visible as u16).saturating_add(3).saturating_add(extra);

    let popup = centered_rect(width, height, area);

    let block = Block::default()
        .style(Style::default().bg(theme.surface[1]))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_focus))
        .title(format!(" {} ", dialog.title))
        .title_style(
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        );
    let inner = block.inner(popup);

    // Scroll so the highlighted row stays inside the visible window.
    let selected = dialog.selected();
    let offset = if selected < visible {
        0
    } else {
        selected + 1 - visible
    };
    let end = (offset + visible).min(rows.len());
    let mut shown: Vec<Line<'static>> = rows[offset..end].to_vec();
    if let Some(text) = rename {
        // A text-input row reading as `Rename: <typed title>█`, with a block cursor.
        shown.push(Line::from(vec![
            Span::styled(
                "  Rename: ",
                Style::default().fg(theme.info).add_modifier(Modifier::BOLD),
            ),
            Span::styled(text.to_owned(), Style::default().fg(theme.text)),
            Span::styled("█", Style::default().fg(theme.info)),
        ]));
    }
    shown.push(Line::from(Span::styled(
        format!("  {hint}"),
        Style::default().fg(theme.text_dim),
    )));

    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);
    frame.render_widget(
        Paragraph::new(shown).style(Style::default().fg(theme.text).bg(theme.surface[1])),
        inner,
    );
}

/// Render the interactive permission prompt: a centered modal naming the ask-gated tool and its
/// target with selectable Allow/Deny options, drawn over everything while a decision is pending.
fn draw_permission(frame: &mut Frame, app: &App, area: Rect, theme: &style::ThemeColors) {
    let Some((tool, subject, selected, feedback)) = app.permission_prompt() else {
        return;
    };
    if area.width < 8 || area.height < 6 {
        return;
    }

    // The option row: Allow once · Allow always (remember) · Deny. The highlighted choice carries a
    // `›` marker and bold styling, and the options keep their semantic colors (allow = success/green,
    // deny = error/red) so the choice is unmistakable in a snapshot.
    let option = |label: &str, index: usize, color| {
        let marked = format!("{}{label}", row_marker(selected == index));
        Span::styled(marked, option_style(color, selected == index, theme))
    };
    let is_question = tool == "question";
    let allow_once_label = if is_question {
        "Submit once"
    } else {
        "Allow once"
    };
    let allow_always_label = if is_question {
        "Submit always"
    } else {
        "Allow always"
    };
    let deny_label = if is_question { "Cancel" } else { "Deny" };
    let options = Line::from(vec![
        option(allow_once_label, 0, theme.success),
        Span::raw("  "),
        option(allow_always_label, 1, theme.success),
        Span::raw("  "),
        option(deny_label, 2, theme.error),
    ]);
    let hint = if is_question {
        "type answer · ←/→ select · Enter confirm · Esc cancel"
    } else {
        "y once · a always · n deny · ←/→ select · type feedback on Deny · Enter confirm · Esc deny"
    };

    let feedback_line = if is_question || selected == 2 || !feedback.is_empty() {
        Some(Line::from(vec![
            Span::styled(
                if is_question {
                    "Answer: "
                } else {
                    "Feedback: "
                },
                Style::default()
                    .fg(theme.warning)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(feedback.to_owned(), Style::default().fg(theme.text)),
            Span::styled("▏", Style::default().fg(theme.warning)),
        ]))
    } else {
        None
    };

    let mut rows: Vec<Line<'static>> = vec![
        Line::from(vec![
            Span::styled(
                tool.to_owned(),
                Style::default()
                    .fg(theme.warning)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                " requests permission to act on:",
                Style::default().fg(theme.text),
            ),
        ]),
        Line::from(Span::styled(
            format!("  {subject}"),
            Style::default().fg(theme.text),
        )),
        Line::from(""),
        options,
    ];
    if let Some(feedback_line) = feedback_line {
        rows.push(feedback_line);
    }
    rows.push(Line::from(Span::styled(
        hint,
        Style::default().fg(theme.text_dim),
    )));

    let content_w = rows.iter().map(Line::width).max().unwrap_or(0);
    let max_w = (area.width as usize) * 9 / 10;
    let width = (content_w + 4).clamp(28, max_w.max(28)) as u16;
    let height = (rows.len() as u16).saturating_add(2);
    let popup = centered_rect(width, height, area);

    let block = Block::default()
        .style(Style::default().bg(theme.surface[1]))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.warning))
        .title(" Permission required ")
        .title_style(
            Style::default()
                .fg(theme.warning)
                .add_modifier(Modifier::BOLD),
        );
    let inner = block.inner(popup);

    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);
    frame.render_widget(
        Paragraph::new(rows).style(Style::default().fg(theme.text).bg(theme.surface[1])),
        inner,
    );
}

/// The style for a permission option: its semantic color when idle, and an explicit solid inverse
/// pair when highlighted (near-black text on the semantic color), so the active option stands out
/// as a solid fill rather than a terminal `REVERSED` swap.
fn option_style(color: Color, selected: bool, theme: &style::ThemeColors) -> Style {
    if selected {
        Style::default()
            .fg(theme.background)
            .bg(color)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(color).add_modifier(Modifier::DIM)
    }
}

/// The footer hint for a dialog: the session picker advertises its switch/rename/delete actions
/// (and the commit/cancel keys while renaming); every other picker shows the plain navigation hint.
fn dialog_hint(kind: DialogKind, renaming: bool) -> &'static str {
    match (kind, renaming) {
        (DialogKind::Sessions, true) => "type a name · Enter to save · Esc to cancel",
        (DialogKind::Sessions, false) => {
            "↑/↓ move · Enter switch · r rename · d delete · Esc close"
        }
        (DialogKind::Timeline, _) => "↑/↓ move · Enter select · f fork · Esc close",
        (DialogKind::Fork, _) => "↑/↓ move · Enter fork here · Esc close",
        _ => "↑/↓ to move · Esc to close",
    }
}

/// Build the command palette's command rows for the current query: each matching command as
/// `/name  description` with the highlighted row marked, or a single graceful "no matching
/// commands" row when the query matches nothing.
fn palette_command_lines(app: &App) -> Vec<Line<'static>> {
    let theme = style::ThemeColors::from_name(app.theme());
    let matches = app.palette_matches();
    if matches.is_empty() {
        return vec![Line::from(Span::styled(
            "  no matching commands",
            Style::default()
                .fg(theme.text_dim)
                .add_modifier(Modifier::ITALIC),
        ))];
    }
    let selected = app.palette_selected();
    let mut lines = Vec::with_capacity(matches.len());
    for (i, command) in matches.iter().enumerate() {
        let is_selected = i == selected;
        let marker = row_marker(is_selected);
        let disabled = command.disabled_reason.is_some();
        let mut description = command.description.clone();
        if let Some(reason) = &command.disabled_reason {
            if !description.is_empty() {
                description.push_str(" · ");
            }
            description.push_str(reason);
        }
        // A highlighted (and enabled) row reads as a solid inverse pair; disabled rows stay dim
        // even when highlighted so they read as unavailable.
        if is_selected && !disabled {
            let sel = selected_row_style(&theme);
            lines.push(Line::from(vec![
                Span::styled(marker, sel),
                Span::styled(command.label.clone(), sel),
                Span::styled("  ", sel),
                Span::styled(description, sel),
            ]));
        } else {
            let name_style = if disabled {
                Style::default()
                    .fg(theme.text_dim)
                    .add_modifier(Modifier::DIM)
            } else {
                Style::default().fg(theme.text)
            };
            lines.push(Line::from(vec![
                Span::styled(marker, Style::default().fg(theme.info)),
                Span::styled(command.label.clone(), name_style),
                Span::raw("  "),
                Span::styled(description, Style::default().fg(theme.text_dim)),
            ]));
        }
    }
    lines
}

/// Render the command palette as a centered bordered popup: a search line reflecting the typed
/// query sits atop the filtered command list, which scrolls to keep the highlighted row visible.
fn draw_palette(frame: &mut Frame, app: &App, area: Rect, theme: &style::ThemeColors) {
    let Some(query) = app.palette_query() else {
        return;
    };
    if area.width < 8 || area.height < 6 {
        return;
    }

    let command_rows = palette_command_lines(app);
    // The search line reads as a text input: the typed query followed by a block cursor.
    let search_line = Line::from(vec![
        Span::styled(
            "› ",
            Style::default().fg(theme.info).add_modifier(Modifier::BOLD),
        ),
        Span::styled(query.to_owned(), Style::default().fg(theme.text)),
        Span::styled("▏", Style::default().fg(theme.info)),
    ]);
    let hint = if app
        .palette_scope()
        .is_some_and(|scope| scope != crate::app::PaletteScope::Root)
    {
        "↑/↓ move · Enter run · Esc back"
    } else {
        "↑/↓ move · Enter run · Esc close"
    };

    let widest = command_rows
        .iter()
        .map(Line::width)
        .max()
        .unwrap_or(0)
        .max(search_line.width())
        .max(hint.chars().count() + 2);
    let max_w = (area.width as usize) * 9 / 10;
    let width = (widest + 4).clamp(28, max_w.max(28)) as u16;

    let row_budget = (area.height as usize)
        .saturating_sub(6)
        .clamp(1, MAX_DIALOG_ROWS);
    let visible = command_rows.len().min(row_budget).max(1);
    // Borders (2) + the search line (1) + the hint row (1) on top of the visible command rows.
    let height = (visible as u16).saturating_add(4);

    let popup = centered_rect(width, height, area);

    let scope_title = match app
        .palette_scope()
        .unwrap_or(crate::app::PaletteScope::Root)
    {
        crate::app::PaletteScope::Root => "Command Palette".to_owned(),
        crate::app::PaletteScope::Models => "Command Palette · Models".to_owned(),
        crate::app::PaletteScope::Skills => "Command Palette · Skills".to_owned(),
        crate::app::PaletteScope::Variants => "Command Palette · Variants".to_owned(),
        crate::app::PaletteScope::Subagents => "Command Palette · Subagents".to_owned(),
        crate::app::PaletteScope::QueuedPrompts => "Command Palette · Queued prompts".to_owned(),
    };

    let block = Block::default()
        .style(Style::default().bg(theme.surface[1]))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_focus))
        .title(format!(" {scope_title} "))
        .title_style(
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        );
    let inner = block.inner(popup);

    // Scroll so the highlighted command stays inside the visible window.
    let selected = app.palette_selected();
    let offset = if selected < visible {
        0
    } else {
        selected + 1 - visible
    };
    let end = (offset + visible).min(command_rows.len());

    let mut shown: Vec<Line<'static>> = Vec::with_capacity(visible + 2);
    shown.push(search_line);
    shown.extend(command_rows[offset..end].iter().cloned());
    shown.push(Line::from(Span::styled(
        format!("  {hint}"),
        Style::default().fg(theme.text_dim),
    )));

    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);
    frame.render_widget(
        Paragraph::new(shown).style(Style::default().fg(theme.text).bg(theme.surface[1])),
        inner,
    );
}

/// Render the session sidebar: a bordered panel beside the main content listing the active session
/// context (model / agent / mode / session) and live counters (transcript messages, token usage),
/// so its presence is unambiguous in a snapshot.
fn draw_sidebar(frame: &mut Frame, app: &App, area: Rect, theme: &style::ThemeColors) {
    if area.width < 4 || area.height < 3 {
        return;
    }
    let dim = Style::default().fg(theme.text_dim);
    let selected = app.sidebar_panel();
    let mut tab_spans = Vec::new();
    for (index, panel) in app.sidebar_panels().iter().enumerate() {
        if index > 0 {
            tab_spans.push(Span::styled(" ", Style::default().fg(theme.text_muted)));
        }
        let style = if *panel == selected {
            // An explicit solid inverse pair (near-black text on the brand accent), not a terminal
            // `REVERSED` swap, so the active tab reads as a solid fill in a snapshot.
            Style::default()
                .fg(theme.selection_fg)
                .bg(theme.selection_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.text_muted)
        };
        tab_spans.push(Span::styled(panel.short().to_owned(), style));
    }

    let mut lines: Vec<Line<'static>> = vec![Line::from(tab_spans), Line::from("")];
    lines.extend(app.sidebar_rows().into_iter().map(Line::from));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!(
            "{} B toggle · Ctrl+[ / Ctrl+] cycle · Ctrl+1..7 tabs",
            app.keybinds().leader.display()
        ),
        dim,
    )));

    let block = Block::default()
        .style(Style::default().bg(theme.surface[1]))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_focus))
        .title(format!(" Sidebar · {} ", selected.title()))
        .title_style(
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        );
    let inner = block.inner(area);

    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    frame.render_widget(
        Paragraph::new(lines)
            .style(Style::default().fg(theme.text).bg(theme.surface[1]))
            .wrap(Wrap { trim: false }),
        inner,
    );
}

/// Render the which-key overlay while the leader chord is armed: a centered popup listing each
/// active leader binding (its key and the action it runs) so the bindings are discoverable.
fn draw_which_key(frame: &mut Frame, app: &App, area: Rect, theme: &style::ThemeColors) {
    if !app.leader_pending() {
        return;
    }
    if area.width < 8 || area.height < 6 {
        return;
    }

    let leader = app.keybinds().leader.display();
    let key_style = Style::default().fg(theme.info).add_modifier(Modifier::BOLD);
    let label_style = Style::default().fg(theme.text);

    let mut rows: Vec<Line<'static>> = Vec::new();
    for (chord, action) in app.keybinds().entries() {
        let mut label = action.label().to_owned();
        if let Some(reason) = app.keybind_disabled_reason(*action) {
            label.push_str(&format!(" (disabled: {reason})"));
        }
        rows.push(Line::from(vec![
            Span::styled(format!("  {:<3}", chord.display()), key_style),
            Span::styled(format!("  {label}"), label_style),
        ]));
    }

    let hint = "Esc to cancel";
    let widest = rows
        .iter()
        .map(Line::width)
        .max()
        .unwrap_or(0)
        .max(hint.chars().count() + 2);
    let max_w = (area.width as usize) * 9 / 10;
    let width = (widest + 4).clamp(28, max_w.max(28)) as u16;

    let row_budget = (area.height as usize)
        .saturating_sub(4)
        .clamp(1, MAX_DIALOG_ROWS);
    let visible = rows.len().min(row_budget).max(1);
    // Borders (2) + the hint row (1) on top of the visible binding rows.
    let height = (visible as u16).saturating_add(3);

    let popup = centered_rect(width, height, area);

    let block = Block::default()
        .style(Style::default().bg(theme.surface[1]))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_focus))
        .title(format!(" Keybindings · leader {leader} "))
        .title_style(
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        );
    let inner = block.inner(popup);

    let end = rows.len().min(visible);
    let mut shown: Vec<Line<'static>> = rows[..end].to_vec();
    shown.push(Line::from(Span::styled(
        format!("  {hint}"),
        Style::default().fg(theme.text_dim),
    )));

    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);
    frame.render_widget(
        Paragraph::new(shown).style(Style::default().fg(theme.text).bg(theme.surface[1])),
        inner,
    );
}

/// A rectangle of the given size centered within `area` (clamped to fit).
fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect {
        x,
        y,
        width,
        height,
    }
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect, theme: &style::ThemeColors) {
    let width = area.width.max(1) as usize;
    let footer = Paragraph::new(status_bar_line(app, width))
        .style(Style::default().fg(theme.text).bg(theme.surface[0]));
    frame.render_widget(footer, area);
}

/// Build the 3-zone status bar: left `? for help` (or an attention cue when one is active), a
/// mathematically-centered autonomy/permission segment, and the real active model id flush to the
/// right margin. It is painted on the `BAR` surface by [`draw_footer`]. The right zone shows the
/// live binary default model id (never the mockup `opus-4.8`), so switching model recolors/relabels
/// it; the colors come from the active theme so a theme switch visibly recolors the bar.
pub fn status_bar_line(app: &App, width: usize) -> Line<'static> {
    let theme = style::ThemeColors::from_name(app.theme());
    let w = width.max(1);
    let base = Style::default().fg(theme.text);
    let mut cells: Vec<(char, Style)> = vec![(' ', base); w];

    // Left zone: an attention cue takes precedence over the help hint when one is active, so the
    // existing attention surface is preserved within the new bar.
    let left: Vec<(String, Style)> = if let Some(attention) = app.attention_badge() {
        vec![
            (
                "attention ".to_owned(),
                Style::default().fg(theme.text_muted),
            ),
            (
                attention.to_owned(),
                Style::default()
                    .fg(theme.warning)
                    .add_modifier(Modifier::BOLD),
            ),
        ]
    } else {
        vec![
            ("?".to_owned(), Style::default().fg(theme.border)),
            (" for help".to_owned(), Style::default().fg(theme.text_muted)),
        ]
    };
    place_segments(&mut cells, 2, &left);

    // Center zone: `auto` (AMBER head) + ` (high)` (FAINT tail) + ` · ` (GHOST) + permission (DIM),
    // centered so its midpoint sits at the terminal's horizontal center.
    let center: Vec<(String, Style)> = vec![
        ("auto".to_owned(), Style::default().fg(theme.secondary)),
        (" (high)".to_owned(), Style::default().fg(theme.text_muted)),
        (" · ".to_owned(), Style::default().fg(theme.ghost)),
        (
            "allow all commands".to_owned(),
            Style::default().fg(theme.text_dim),
        ),
    ];
    let center_len: usize = center.iter().map(|(t, _)| t.chars().count()).sum();
    let center_start = w.saturating_sub(center_len) / 2;
    place_segments(&mut cells, center_start, &center);

    // Right zone: the real active model id, flush to the right margin.
    let model = app.status.model.clone();
    let model_w = model.chars().count();
    let right_start = w.saturating_sub(2 + model_w);
    place_segments(&mut cells, right_start, &[(model, base)]);

    cells_to_line(&cells)
}

/// Overwrite `cells` starting at column `start` with the characters of each styled segment, clipping
/// at the right edge.
fn place_segments(cells: &mut [(char, Style)], start: usize, segments: &[(String, Style)]) {
    let mut col = start;
    for (text, style) in segments {
        for ch in text.chars() {
            if col >= cells.len() {
                return;
            }
            cells[col] = (ch, *style);
            col += 1;
        }
    }
}

/// Collapse a styled cell buffer into a [`Line`], grouping runs of equal style into single spans.
fn cells_to_line(cells: &[(char, Style)]) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut run = String::new();
    let mut run_style: Option<Style> = None;
    for (ch, style) in cells {
        match run_style {
            Some(s) if s == *style => run.push(*ch),
            _ => {
                if let Some(s) = run_style.take() {
                    spans.push(Span::styled(std::mem::take(&mut run), s));
                }
                run.push(*ch);
                run_style = Some(*style);
            }
        }
    }
    if let Some(s) = run_style {
        spans.push(Span::styled(run, s));
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, AssistantTurn, DialogKind, Status};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::mpsc::channel;

    static ATTACH_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn app() -> App {
        App::new(Status {
            model: "google/gemini-2.5-flash".to_owned(),
            agent: "build".to_owned(),
            mode: "build".to_owned(),
            variant: "default".to_owned(),
            session: "new".to_owned(),
        })
    }

    fn temp_png() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "cortex-tui-ui-attach-{}-{}.png",
            std::process::id(),
            ATTACH_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(
            &path,
            [
                0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n', 0, 0, 0, 0x0d, b'I', b'H', b'D',
                b'R', 0, 0, 0, 0x01, 0, 0, 0, 0x01, 0x08, 0x02, 0, 0, 0, 0x90, 0x77, 0x53, 0xde, 0,
                0, 0, 0x0a, b'I', b'D', b'A', b'T', 0x08, 0xd7, 0x63, 0xf8, 0x0f, 0x00, 0x01, 0x05,
                0x01, 0x02, 0x60, 0x5d, 0xe9, 0x2f, 0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xae, 0x42,
                0x60, 0x82,
            ],
        )
        .expect("write temp png");
        path
    }

    /// A fresh, unset interrupt flag for tests that begin a turn without a real worker.
    fn flag() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    fn plain(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn rendered(lines: &[Line]) -> String {
        lines.iter().map(plain).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn transcript_markers_follow_active_theme_palette() {
        let mut a = app();
        a.set_theme("light".to_owned());
        a.transcript.push(Entry::User("hello".to_owned()));
        a.transcript.push(Entry::Assistant(AssistantTurn {
            reasoning: String::new(),
            answer: "world".to_owned(),
        }));

        let lines = transcript_lines(&a, 80);
        let theme = style::ThemeColors::light();
        // The user `›` mark is inline with the message text and uses the theme primary (ORANGE).
        let user = lines
            .iter()
            .find(|line| plain(line).contains("› hello"))
            .and_then(|line| line.spans.iter().find(|s| s.content.starts_with('›')))
            .and_then(|span| span.style.fg);
        // The assistant `✦` mark is inline with the reply and uses the theme secondary (AMBER).
        let assistant = lines
            .iter()
            .find(|line| plain(line).contains("✦ world"))
            .and_then(|line| line.spans.iter().find(|s| s.content.starts_with('✦')))
            .and_then(|span| span.style.fg);

        assert_eq!(user, Some(theme.primary), "user marker uses theme primary");
        assert_eq!(
            assistant,
            Some(theme.secondary),
            "assistant marker uses theme secondary"
        );
    }

    #[test]
    fn user_line_uses_inline_orange_prompt_mark() {
        // VAL-CONV-017: a sent user message renders as a `›` mark (ORANGE) at the content-left
        // column followed by the message text (TXT) on the same line.
        let mut a = app();
        a.transcript
            .push(Entry::User("the streaming buffer grows unbounded".to_owned()));
        let theme = style::ThemeColors::from_name(a.theme());
        let lines = transcript_lines(&a, 80);
        let line = lines
            .iter()
            .find(|l| plain(l).contains("› the streaming buffer"))
            .expect("user line with inline prompt mark");
        // The mark is ORANGE; the body that follows is TXT.
        let mark = line
            .spans
            .iter()
            .find(|s| s.content.starts_with('›'))
            .expect("prompt mark span");
        assert_eq!(mark.style.fg, Some(theme.primary), "mark is ORANGE");
        let body = line
            .spans
            .iter()
            .find(|s| s.content.contains("streaming"))
            .expect("body span");
        assert_eq!(body.style.fg, Some(theme.text), "body is TXT");
        // No legacy `› you` author label leads the line.
        assert!(!plain(line).contains("you"), "no legacy author label");
    }

    #[test]
    fn assistant_line_uses_inline_amber_star_with_hanging_indent() {
        // VAL-CONV-018/019: the assistant reply renders a `✦` mark (AMBER) at the content-left
        // column followed by the first line, and wrapped continuation lines hang-indent under the
        // text column (two columns past the mark), not under the mark itself.
        let mut a = app();
        let answer =
            "I'll add a buffer-size limit to StreamProcessor and trim the oldest chunks past the cap. Reading the processor first.";
        a.transcript.push(Entry::Assistant(AssistantTurn {
            reasoning: String::new(),
            answer: answer.to_owned(),
        }));
        let theme = style::ThemeColors::from_name(a.theme());
        let lines = transcript_lines(&a, 60);

        // First reply line carries the `✦` mark in AMBER, with the reply text in BRIGHT. (Matched
        // by reply content so the MOTD card's own `✦ Cortex Code` title is not picked up.)
        let star_idx = lines
            .iter()
            .position(|l| plain(l).contains("✦ I'll"))
            .expect("assistant line with star mark");
        let first = &lines[star_idx];
        let star = first
            .spans
            .iter()
            .find(|s| s.content.starts_with('✦'))
            .expect("star mark span");
        assert_eq!(star.style.fg, Some(theme.secondary), "star mark is AMBER");
        assert!(
            first.spans.iter().any(|s| s.style.fg == Some(theme.text_bright)),
            "reply body is BRIGHT:\n{}",
            plain(first)
        );

        // The reply wraps; the continuation line indents under the text column (4 cols), aligning
        // past the mark, and carries no `✦`.
        let cont = &lines[star_idx + 1];
        let cont_text = plain(cont);
        assert!(
            cont_text.starts_with("    ") && !cont_text.trim_start().is_empty(),
            "continuation hangs under the text column: {cont_text:?}"
        );
        assert!(!cont_text.contains('✦'), "continuation has no mark");
    }

    #[test]
    fn motd_card_leads_the_buffer_even_with_a_transcript() {
        // VAL-CONV-021 / VAL-CROSS-001: the MOTD welcome card is the top of the single scrollback
        // buffer regardless of transcript contents (not gated on an empty transcript), with the
        // user line appended directly beneath it.
        let mut a = app();
        a.transcript.push(Entry::User("hello there".to_owned()));
        let lines = transcript_lines(&a, 92);
        let card_idx = lines
            .iter()
            .position(|l| plain(l).contains("Cortex Code"))
            .expect("MOTD card wordmark present at the top");
        let user_idx = lines
            .iter()
            .position(|l| plain(l).contains("› hello there"))
            .expect("user line present");
        assert!(
            card_idx < user_idx,
            "MOTD card precedes the user line in the same buffer"
        );
        // The card-only home omits the meta panels once a conversation exists.
        let text = rendered(&lines);
        assert!(!text.contains("Directory"), "no meta panels with a transcript");
    }

    #[test]
    fn deep_session_clips_motd_behind_an_above_marker() {
        // VAL-CONV-021 / VAL-CONV-063: in a session deep enough to overflow the viewport, the MOTD
        // upper rows scroll off the top and a `▲ N lines above` marker (GHOST) occupies row 0
        // alone, with the transcript below it.
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut a = app();
        for i in 0..40 {
            a.transcript.push(Entry::User(format!("question number {i}")));
            a.transcript.push(Entry::Assistant(AssistantTurn {
                reasoning: String::new(),
                answer: format!("answer number {i}"),
            }));
        }
        let theme = style::ThemeColors::from_name(a.theme());
        let backend = TestBackend::new(92, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &a)).unwrap();
        let buffer = terminal.backend().buffer().clone();

        // Row 0 holds the `▲ … lines above` marker, in GHOST.
        let row0: String = (0..92)
            .map(|x| buffer[(x, 0)].symbol())
            .collect::<Vec<_>>()
            .concat();
        assert!(
            row0.contains('▲') && row0.contains("lines above"),
            "row 0 shows the scrollback marker: {row0:?}"
        );
        let marker_cell = (0..92)
            .map(|x| buffer[(x, 0)].clone())
            .find(|c| c.symbol() == "▲")
            .expect("the ▲ marker glyph");
        assert_eq!(marker_cell.fg, theme.ghost, "marker is GHOST");

        // The MOTD wordmark has scrolled off the top (it is not on screen anymore).
        let screen: String = buffer
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect();
        assert!(
            !screen.contains("Cortex Code"),
            "the MOTD wordmark is clipped off the top in a deep session"
        );
    }

    #[test]
    fn above_marker_count_tracks_scroll_position() {
        // VAL-CONV-063: scrolling up reveals earlier rows and the `▲ N lines above` count drops as
        // earlier content comes into view; scrolling back to the bottom re-pins the larger count.
        let mut a = app();
        for i in 0..60 {
            a.transcript.push(Entry::Notice(format!("notice line {i}")));
        }
        a.set_viewport(92, 20);

        let count_above = |a: &App| -> usize {
            let lines = transcript_lines(a, 92);
            let height = 20usize;
            let max_scroll = lines.len().saturating_sub(height);
            let scroll = a.scroll().min(max_scroll);
            let start = max_scroll - scroll;
            if start == 0 { 0 } else { start + 1 }
        };

        let pinned = count_above(&a);
        assert!(pinned > 0, "deep session clips lines above when pinned");
        a.on_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::PageUp,
            crossterm::event::KeyModifiers::NONE,
        ));
        let scrolled = count_above(&a);
        assert!(
            scrolled < pinned,
            "scrolling up reduces the lines-above count ({scrolled} < {pinned})"
        );
    }

    #[test]
    fn typing_shows_bright_text_and_orange_caret_over_full_motd() {
        // VAL-CONV-014/015/016: typed input renders in BRIGHT after the prompt with a `▏` ORANGE
        // caret at the tail, while the full MOTD card stays at the top and the transcript region is
        // empty (no meta panels, no transcript blocks).
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut a = app();
        // Type the message the way a user would, so the cursor ends at the tail of the text.
        for ch in "the streaming buffer grows unbounded on long responses".chars() {
            a.on_key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char(ch),
                crossterm::event::KeyModifiers::NONE,
            ));
        }
        let theme = style::ThemeColors::from_name(a.theme());
        let backend = TestBackend::new(92, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &a)).unwrap();
        let buffer = terminal.backend().buffer().clone();

        // Exactly one `▏` caret is on screen; locate it.
        let mut caret_pos = None;
        for y in 0..30u16 {
            for x in 0..92u16 {
                if buffer[(x, y)].symbol() == "▏" {
                    assert!(caret_pos.is_none(), "exactly one caret on screen");
                    caret_pos = Some((x, y));
                }
            }
        }
        let (cx, cy) = caret_pos.expect("a ▏ caret at the typed-text tail");
        assert_eq!(buffer[(cx, cy)].fg, theme.primary, "caret is ORANGE");
        // The cell immediately before the caret is the tail of the typed text (`responses`), in
        // BRIGHT — confirming both the BRIGHT typed text and the no-gap caret placement.
        let before = &buffer[(cx - 1, cy)];
        assert_eq!(before.symbol(), "s", "caret sits at the tail of the typed text");
        assert_eq!(before.fg, theme.text_bright, "typed text renders in BRIGHT");

        // The full MOTD card is still visible (nothing scrolled) and no meta panels / transcript.
        let screen: String = buffer
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect();
        assert!(screen.contains("Cortex Code"), "MOTD card still visible while typing");
        assert!(!screen.contains("Directory"), "no meta panels while typing");
        assert!(!screen.contains('▲'), "nothing clipped off the top while typing");
    }

    #[test]
    fn draw_uses_theme_background_layers() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut a = app();
        a.set_theme("light".to_owned());
        a.transcript.push(Entry::User("hello".to_owned()));
        // Open the slash menu so the elevated overlay surface (PANEL) is exercised: the composer
        // itself is flat on the CANVAS in the brand design, so the PANEL layer comes from overlays.
        a.input.set_value("/");

        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &a)).unwrap();
        let cells = terminal.backend().buffer().content();
        let theme = style::ThemeColors::light();

        assert!(
            cells.iter().any(|c| c.bg == theme.background),
            "transcript background uses the theme background"
        );
        assert!(
            cells.iter().any(|c| c.bg == theme.surface[1]),
            "overlay surfaces use themed surface background"
        );
        assert!(
            cells.iter().any(|c| c.bg == theme.surface[0]),
            "footer uses themed footer background"
        );
    }

    #[test]
    fn home_screen_renders_the_brand_lockup_and_brain_mascot() {
        // VAL-FOUND-005/006/007: the home/MOTD shows the `✦ Cortex Code` wordmark and the brain
        // mascot block-art in the brand accent, with no `>_` cliché and zero tofu.
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let a = app(); // empty transcript -> home screen
        let theme = style::ThemeColors::from_name(a.theme());
        let backend = TestBackend::new(92, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &a)).unwrap();
        let buffer = terminal.backend().buffer().clone();

        let screen: String = buffer
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect();

        assert!(screen.contains('✦'), "the brand star renders:\n{screen}");
        assert!(
            screen.contains("Cortex Code"),
            "the wordmark name renders:\n{screen}"
        );
        assert!(!screen.contains(">_"), "the `>_` cliché never appears");
        assert!(
            screen.contains("██ ▌  ▐ ██"),
            "the brain mascot block-art renders:\n{screen}"
        );

        // The `✦` brand star carries the brand primary color (ORANGE on the default theme).
        let star = buffer
            .content()
            .iter()
            .find(|c| c.symbol() == "✦")
            .expect("the brand star is on screen");
        assert_eq!(star.fg, theme.primary, "the brand star is the brand accent");

        // A mascot block cell is the brand accent too (the mascot is mono-orange, not tofu).
        let block = buffer
            .content()
            .iter()
            .find(|c| c.symbol() == "▐")
            .expect("a mascot block cell is on screen");
        assert_eq!(block.fg, theme.primary, "the mascot is the brand accent");
    }

    #[test]
    fn home_body_background_is_canvas_not_bar_on_dark() {
        // VAL-FOUND-002: under the default `dark` theme the home/MOTD body fills with the CANVAS
        // near-black (#0d0d0d, the theme background), not the BAR surface; BAR (surface[0]) is
        // reserved for the status/footer bar.
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let a = app(); // empty transcript -> home screen, default `dark` theme
        assert_eq!(a.theme(), "dark", "the default theme under test is `dark`");
        let theme = style::ThemeColors::from_name(a.theme());
        assert_ne!(
            theme.background, theme.surface[0],
            "CANVAS and BAR must differ for this assertion to be meaningful"
        );

        let backend = TestBackend::new(92, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &a)).unwrap();
        let buffer = terminal.backend().buffer().clone();

        // The brand star is part of the welcome-card body; its cell background is the card's
        // interior fill, which must be the CANVAS background rather than the BAR surface.
        let star = buffer
            .content()
            .iter()
            .find(|c| c.symbol() == "✦")
            .expect("the brand star is on the home screen");
        assert_eq!(
            star.bg, theme.background,
            "the home body background is the CANVAS near-black (theme.background)"
        );
        assert_ne!(
            star.bg, theme.surface[0],
            "the home body background is not the BAR surface (surface[0])"
        );

        // BAR (surface[0]) is still used for the status/footer bar on the home screen.
        let width = buffer.area.width as usize;
        let height = buffer.area.height as usize;
        let footer_bg = buffer.content()[(height - 1) * width].bg;
        assert_eq!(
            footer_bg,
            theme.surface[0],
            "the status/footer bar still carries the BAR surface"
        );
    }

    #[test]
    fn no_legacy_thin_triangles_in_the_render_path() {
        // VAL-FOUND-028 (glyph-safety slice): the thin triangles `▸`/`▾` never appear in the app's
        // rendered output across the home, transcript menus, dialogs, and the command palette; the
        // selection/row marker is the brand `›`.
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        fn screen_of(a: &App) -> String {
            let backend = TestBackend::new(92, 30);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal.draw(|frame| draw(frame, a)).unwrap();
            terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .map(ratatui::buffer::Cell::symbol)
                .collect()
        }

        fn assert_no_thin_triangle(label: &str, screen: &str) {
            assert!(!screen.contains('▸'), "{label} renders a thin `▸`:\n{screen}");
            assert!(!screen.contains('▾'), "{label} renders a thin `▾`:\n{screen}");
        }

        // Home screen.
        assert_no_thin_triangle("home", &screen_of(&app()));

        // A system notice in the transcript + the `/` command menu open (carries a row marker).
        let mut convo = app();
        convo
            .transcript
            .push(Entry::Notice("/models opens the model picker".to_owned()));
        convo.input.set_value("/");
        let convo_screen = screen_of(&convo);
        assert_no_thin_triangle("slash menu + notice", &convo_screen);
        assert!(
            convo_screen.contains('›'),
            "the brand `›` marker is used instead:\n{convo_screen}"
        );

        // The model picker dialog.
        let mut dialog = app();
        for c in "/models".chars() {
            dialog.on_key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char(c),
                crossterm::event::KeyModifiers::NONE,
            ));
        }
        dialog.on_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        ));
        assert_no_thin_triangle("model dialog", &screen_of(&dialog));

        // The command palette.
        let mut palette = app();
        palette.on_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('p'),
            crossterm::event::KeyModifiers::CONTROL,
        ));
        assert_no_thin_triangle("command palette", &screen_of(&palette));
    }

    #[test]
    fn every_cell_carries_an_explicit_background_on_all_themes() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        use ratatui::style::Color;

        // VAL-FOUND-002 / VAL-FOUND-015: no cell may fall through to the terminal-default
        // background. On the brand `dark` theme the page is near-black #0d0d0d; on `light` (and the
        // other chrome themes) every cell must still carry an explicit bg so there are no black
        // bars. Assert across the home screen and an active transcript.
        for theme_name in ["dark", "light", "monokai", "ocean", "vscode"] {
            for with_transcript in [false, true] {
                let mut a = app();
                a.set_theme(theme_name.to_owned());
                if with_transcript {
                    a.transcript.push(Entry::User("hello".to_owned()));
                }

                let backend = TestBackend::new(92, 30);
                let mut terminal = Terminal::new(backend).unwrap();
                terminal.draw(|frame| draw(frame, &a)).unwrap();
                let buffer = terminal.backend().buffer();

                for (i, cell) in buffer.content().iter().enumerate() {
                    assert_ne!(
                        cell.bg,
                        Color::Reset,
                        "{theme_name} (transcript={with_transcript}): cell {i} \
                         (x={}, y={}, sym={:?}) has no explicit background",
                        i as u16 % 92,
                        i as u16 / 92,
                        cell.symbol(),
                    );
                }
            }
        }
    }

    #[test]
    fn slash_palette_and_dialog_rows_use_theme_colors() {
        let mut a = app();
        a.set_theme("monokai".to_owned());
        let theme = style::ThemeColors::monokai();

        a.input.set_value("/");
        let slash_rows = slash_menu_lines(&a).expect("slash menu open");
        assert!(slash_rows.len() > 1, "multiple slash commands match `/`");
        // The highlighted (first) row is a solid inverse pair: explicit selection fg on bg, not a
        // REVERSED swap (VAL-FOUND-017).
        assert_eq!(slash_rows[0].spans[0].style.fg, Some(theme.selection_fg));
        assert_eq!(slash_rows[0].spans[0].style.bg, Some(theme.selection_bg));
        // A non-highlighted row keeps the themed accent marker and dim description.
        assert_eq!(slash_rows[1].spans[0].style.fg, Some(theme.info));
        assert_eq!(slash_rows[1].spans[3].style.fg, Some(theme.text_dim));

        a.on_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('p'),
            crossterm::event::KeyModifiers::CONTROL,
        ));
        let palette_rows = palette_command_lines(&a);
        assert_eq!(palette_rows[0].spans[0].style.fg, Some(theme.selection_fg));
        assert_eq!(palette_rows[0].spans[0].style.bg, Some(theme.selection_bg));

        let dialog = Dialog {
            kind: DialogKind::Models,
            title: "Models".to_owned(),
            items: vec!["alpha".to_owned(), "beta".to_owned()],
            selected: 0,
            empty: None,
        };
        let dialog_rows = dialog_lines(&dialog, &theme);
        // The selected dialog row is the solid inverse pair; the other keeps the accent marker.
        assert_eq!(dialog_rows[0].spans[0].style.fg, Some(theme.selection_fg));
        assert_eq!(dialog_rows[0].spans[0].style.bg, Some(theme.selection_bg));
        assert_eq!(dialog_rows[1].spans[0].style.fg, Some(theme.info));
    }

    #[test]
    fn tool_card_colors_follow_theme_semantics() {
        let theme = style::ThemeColors::ocean_dark();

        let running = tool_card("read", r#"{"path":"a.txt"}"#);
        let mut running_lines = Vec::new();
        push_tool(
            &mut running_lines,
            &running,
            80,
            false,
            Some(crate::glyphs::SPINNER_ROTATE[0]),
            &theme,
        );
        assert_eq!(running_lines[0].spans[0].style.fg, Some(theme.info));

        let mut completed = tool_card("read", r#"{"path":"a.txt"}"#);
        completed.complete(&[protocol::ToolResultPart::text("ok")], false);
        let mut completed_lines = Vec::new();
        push_tool(&mut completed_lines, &completed, 80, false, None, &theme);
        assert_eq!(completed_lines[0].spans[0].style.fg, Some(theme.success));

        let mut failed = tool_card("read", r#"{"path":"a.txt"}"#);
        failed.complete(&[protocol::ToolResultPart::text("boom")], true);
        let mut failed_lines = Vec::new();
        push_tool(&mut failed_lines, &failed, 80, false, None, &theme);
        assert_eq!(failed_lines[0].spans[0].style.fg, Some(theme.error));
    }

    #[test]
    fn wrap_breaks_on_width_and_keeps_words() {
        let wrapped = wrap("the quick brown fox", 9);
        assert_eq!(
            wrapped,
            vec!["the quick".to_owned(), "brown fox".to_owned()]
        );
    }

    #[test]
    fn wrap_hard_splits_overlong_words() {
        let wrapped = wrap("abcdefghij", 4);
        assert_eq!(
            wrapped,
            vec!["abcd".to_owned(), "efgh".to_owned(), "ij".to_owned()]
        );
    }

    #[test]
    fn wrap_preserves_blank_lines() {
        assert_eq!(wrap("a\n\nb", 10), vec!["a", "", "b"]);
    }

    #[test]
    fn transcript_shows_user_and_growing_answer() {
        let mut a = app();
        let (tx, rx) = channel();
        a.begin_turn("hello there".to_owned(), rx, flag());
        tx.send(crate::turn::TurnMessage::Event(
            engine::RunnerEvent::TextDelta {
                text: "partial".to_owned(),
            },
        ))
        .unwrap();
        a.poll_turn();

        let text = rendered(&transcript_lines(&a, 80));
        assert!(text.contains("hello there"), "user prompt present:\n{text}");
        assert!(text.contains("partial"), "partial answer present:\n{text}");
        // The busy indicator is shown while streaming.
        assert!(text.contains("Thinking"), "busy indicator present:\n{text}");
    }

    #[test]
    fn busy_indicator_absent_after_completion() {
        let mut a = app();
        let (tx, rx) = channel();
        a.begin_turn("q".to_owned(), rx, flag());
        tx.send(crate::turn::TurnMessage::Event(
            engine::RunnerEvent::TextDelta {
                text: "the full reply".to_owned(),
            },
        ))
        .unwrap();
        tx.send(crate::turn::TurnMessage::Finished).unwrap();
        a.poll_turn();

        let text = rendered(&transcript_lines(&a, 80));
        assert!(!text.contains("Thinking"), "indicator cleared:\n{text}");
        assert!(text.contains("the full reply"), "reply present:\n{text}");
    }

    #[test]
    fn assistant_answer_is_markdown_rendered() {
        let mut lines = Vec::new();
        let answer = "## Overview\n\nThis is **bold** and `code`.\n\n- one\n- two\n- three";
        push_assistant(
            &mut lines,
            &AssistantTurn {
                reasoning: String::new(),
                answer: answer.to_owned(),
            },
            80,
            "dark",
            &style::ThemeColors::dark(),
        );
        let text = rendered(&lines);
        // Markers are consumed; structured elements render.
        assert!(text.contains("Overview"), "heading text:\n{text}");
        assert!(!text.contains("##"), "heading markers consumed:\n{text}");
        assert!(
            text.contains("bold") && !text.contains("**"),
            "bold markers consumed:\n{text}"
        );
        assert!(
            text.contains("code") && !text.contains('`'),
            "inline code backticks consumed:\n{text}"
        );
        assert!(
            text.contains("• one") && text.contains("• two") && text.contains("• three"),
            "three bulleted list items:\n{text}"
        );
    }

    #[test]
    fn reasoning_renders_in_a_labeled_region() {
        let mut lines = Vec::new();
        push_assistant(
            &mut lines,
            &AssistantTurn {
                reasoning: "weighing options".to_owned(),
                answer: "final".to_owned(),
            },
            80,
            "dark",
            &style::ThemeColors::dark(),
        );
        let text = rendered(&lines);
        assert!(text.contains("reasoning"), "reasoning labeled:\n{text}");
        assert!(text.contains("weighing options"));
        assert!(text.contains("final"));
    }

    #[test]
    fn absent_reasoning_emits_no_region() {
        let mut lines = Vec::new();
        push_assistant(
            &mut lines,
            &AssistantTurn {
                reasoning: String::new(),
                answer: "just the answer".to_owned(),
            },
            80,
            "dark",
            &style::ThemeColors::dark(),
        );
        let text = rendered(&lines);
        assert!(
            !text.contains("reasoning"),
            "no empty reasoning region:\n{text}"
        );
        assert!(text.contains("just the answer"));
    }

    #[test]
    fn diff_entry_renders_path_header_and_colored_markers() {
        let mut a = app();
        let diff = crate::diff::compute_file_diff(
            "src/main.rs",
            Some("fn main() {\n    println!(\"Hello\");\n    let x = 1;\n}\n"),
            Some("fn main() {\n    println!(\"Cortex\");\n    let x = 1;\n}\n"),
        )
        .expect("a diff");
        a.transcript.push(Entry::Diff(diff));

        let lines = transcript_lines(&a, 80);
        let text = rendered(&lines);
        // The file-path header is visible (VAL-TUI-022).
        assert!(text.contains("src/main.rs"), "path header present:\n{text}");
        // A context line surrounds the change (VAL-TUI-022).
        assert!(
            text.contains("fn main() {"),
            "context line present:\n{text}"
        );
        // Added and removed lines carry their markers (VAL-TUI-021).
        assert!(
            text.contains("+ ") && text.contains(r#"println!("Cortex");"#),
            "added line present:\n{text}"
        );
        assert!(
            text.contains("- ") && text.contains(r#"println!("Hello");"#),
            "removed line present:\n{text}"
        );

        // Added and removed lines are distinctly colored.
        let mut added = None;
        let mut removed = None;
        for line in &lines {
            if let Some(span) = line.spans.first() {
                let content = span.content.as_ref();
                if content.starts_with("+ ") {
                    added = span.style.fg;
                } else if content.starts_with("- ") {
                    removed = span.style.fg;
                }
            }
        }
        assert!(added.is_some() && removed.is_some(), "both markers colored");
        assert_ne!(added, removed, "added and removed distinctly colored");
    }

    #[test]
    fn new_file_diff_renders_a_new_file_indicator() {
        let mut a = app();
        let diff =
            crate::diff::compute_file_diff("notes.txt", None, Some("one\ntwo\n")).expect("a diff");
        a.transcript.push(Entry::Diff(diff));

        let text = rendered(&transcript_lines(&a, 80));
        assert!(
            text.contains("notes.txt") && text.contains("new file"),
            "new-file indicator present (VAL-TUI-023):\n{text}"
        );
        assert!(
            text.contains("+ one") && text.contains("+ two"),
            "all-added:\n{text}"
        );
        assert!(
            !text.contains("- "),
            "no removed lines for a new file:\n{text}"
        );
    }

    fn tool_card(name: &str, args: &str) -> ToolCard {
        ToolCard::new("call_1", name, args)
    }

    #[test]
    fn running_tool_card_renders_label_and_summary() {
        // VAL-TUI-024 / VAL-TUI-025 (running): the card names the tool, summarizes its args, and
        // shows a running state.
        let card = tool_card("read", r#"{"path":"src/main.rs"}"#);
        let mut lines = Vec::new();
        push_tool(
            &mut lines,
            &card,
            80,
            false,
            Some(crate::glyphs::SPINNER_ROTATE[0]),
            &style::ThemeColors::dark(),
        );
        let text = rendered(&lines);
        assert!(text.contains("read"), "tool name labeled:\n{text}");
        assert!(text.contains("running"), "running status:\n{text}");
        assert!(text.contains("src/main.rs"), "arg summary:\n{text}");
        // Distinct card structure: rounded border corners are drawn.
        assert!(
            text.contains('╭') && text.contains('╰'),
            "card border:\n{text}"
        );
    }

    #[test]
    fn completed_tool_card_shows_done_state() {
        // VAL-TUI-025 (completed): the card transitions to a completed state.
        let mut card = tool_card("grep", r#"{"pattern":"TODO","path":"src"}"#);
        card.complete(&[protocol::ToolResultPart::text("src/a.rs:1: TODO")], false);
        let mut lines = Vec::new();
        push_tool(
            &mut lines,
            &card,
            80,
            false,
            None,
            &style::ThemeColors::dark(),
        );
        let text = rendered(&lines);
        assert!(text.contains("grep"), "tool labeled:\n{text}");
        assert!(text.contains("done"), "completed status:\n{text}");
        assert!(!text.contains("running"), "no longer running:\n{text}");
    }

    #[test]
    fn tool_card_expands_to_reveal_output_then_collapses() {
        // VAL-TUI-026: collapsed shows only the summary; expanded reveals the full output.
        let mut card = tool_card("read", r#"{"path":"a.txt"}"#);
        card.complete(
            &[protocol::ToolResultPart::text("1: secret detail line")],
            false,
        );

        let mut collapsed = Vec::new();
        push_tool(
            &mut collapsed,
            &card,
            80,
            false,
            None,
            &style::ThemeColors::dark(),
        );
        let collapsed_text = rendered(&collapsed);
        assert!(
            !collapsed_text.contains("secret detail line"),
            "collapsed hides output:\n{collapsed_text}"
        );
        assert!(
            collapsed_text.contains("expand"),
            "collapsed hints at expansion:\n{collapsed_text}"
        );

        let mut expanded = Vec::new();
        push_tool(
            &mut expanded,
            &card,
            80,
            true,
            None,
            &style::ThemeColors::dark(),
        );
        let expanded_text = rendered(&expanded);
        assert!(
            expanded_text.contains("secret detail line"),
            "expanded reveals output:\n{expanded_text}"
        );
        assert!(
            expanded_text.contains("a.txt"),
            "expanded still shows summary:\n{expanded_text}"
        );
        // The expanded card occupies strictly more lines than the collapsed one.
        assert!(
            expanded.len() > collapsed.len(),
            "expanded is taller: {} vs {}",
            expanded.len(),
            collapsed.len()
        );
    }

    #[test]
    fn failed_tool_card_renders_distinct_error_state() {
        // VAL-TUI-027: a failed call shows a distinct error state with the failure message and is
        // colored differently from a success card.
        let mut failed = tool_card("read", r#"{"path":"nope.txt"}"#);
        failed.complete(
            &[protocol::ToolResultPart::text("file not found: nope.txt")],
            true,
        );
        let mut lines = Vec::new();
        push_tool(
            &mut lines,
            &failed,
            80,
            false,
            None,
            &style::ThemeColors::dark(),
        );
        let text = rendered(&lines);
        assert!(text.contains("error"), "error indicator:\n{text}");
        assert!(
            text.contains("file not found"),
            "failure message shown inline:\n{text}"
        );

        // The error border color differs from a completed card's border color.
        let mut ok = tool_card("read", r#"{"path":"a.txt"}"#);
        ok.complete(&[protocol::ToolResultPart::text("ok")], false);
        let mut ok_lines = Vec::new();
        push_tool(
            &mut ok_lines,
            &ok,
            80,
            false,
            None,
            &style::ThemeColors::dark(),
        );

        let failed_border = lines[0].spans.first().map(|s| s.style.fg);
        let ok_border = ok_lines[0].spans.first().map(|s| s.style.fg);
        assert!(failed_border.is_some() && ok_border.is_some());
        assert_ne!(
            failed_border, ok_border,
            "failed and completed cards are colored distinctly"
        );
    }

    #[test]
    fn slash_menu_lists_builtins_when_just_slash_typed() {
        // VAL-TUI-028: the open menu lists recognizable built-ins.
        let mut a = app();
        a.input.set_value("/");
        let lines = slash_menu_lines(&a).expect("menu open");
        let text = rendered(&lines);
        assert!(text.contains("/help"), "help listed:\n{text}");
        assert!(text.contains("/new"), "new listed:\n{text}");
        assert!(text.contains("/models"), "models listed:\n{text}");
    }

    #[test]
    fn slash_menu_filters_and_highlights_the_match() {
        // VAL-TUI-029 / VAL-TUI-030: "/mod" narrows to /models, and the top match is highlighted.
        let mut a = app();
        a.input.set_value("/mod");
        let lines = slash_menu_lines(&a).expect("menu open");
        let text = rendered(&lines);
        assert!(text.contains("/models"), "models present:\n{text}");
        assert!(!text.contains("/help"), "help filtered out:\n{text}");
        // The highlighted row carries the brand `›` selection marker (never the thin triangle).
        assert!(text.contains("›"), "selection marker shown:\n{text}");
        assert!(!text.contains("▸"), "no legacy thin triangle:\n{text}");
    }

    #[test]
    fn slash_menu_shows_a_graceful_no_match_row() {
        // VAL-TUI-048: an unknown command renders a clear no-match state, not an empty/absent menu.
        let mut a = app();
        a.input.set_value("/zzznotreal");
        let lines = slash_menu_lines(&a).expect("menu open even with no matches");
        let text = rendered(&lines);
        assert!(
            text.contains("no matching commands"),
            "no-match row:\n{text}"
        );
    }

    #[test]
    fn slash_menu_is_closed_for_plain_text() {
        let mut a = app();
        a.input.set_value("hello world");
        assert!(slash_menu_lines(&a).is_none(), "no menu for a chat message");
    }

    #[test]
    fn help_entry_renders_commands_and_keybindings() {
        // VAL-TUI-032: the `/help` view lists commands and keybindings.
        let mut a = app();
        a.transcript.push(Entry::Help);
        let text = rendered(&transcript_lines(&a, 80));
        assert!(text.contains("Commands"), "commands section:\n{text}");
        assert!(text.contains("/help"), "help command listed:\n{text}");
        assert!(text.contains("/export"), "export command listed:\n{text}");
        assert!(text.contains("Keybindings"), "keybindings section:\n{text}");
        assert!(text.contains("Ctrl+C"), "a keybinding listed:\n{text}");
    }

    #[test]
    fn model_switching_is_discoverable_in_both_help_and_palette() {
        // VAL-CROSS-035: a randomly chosen capability (model switching) must be findable through
        // an in-product listing on the TUI surface, not only via hidden means. It surfaces in the
        // `/help` reference AND the command palette, both backed by the shared slash registry.
        let mut a = app();

        a.transcript.push(Entry::Help);
        let help_text = rendered(&transcript_lines(&a, 80));
        assert!(
            help_text.contains("/models"),
            "/help lists the model-switching command:\n{help_text}"
        );

        a.on_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('p'),
            crossterm::event::KeyModifiers::CONTROL,
        ));
        let palette_text = rendered(&palette_command_lines(&a));
        assert!(
            palette_text.contains("/models"),
            "the palette lists the model-switching action:\n{palette_text}"
        );
    }

    #[test]
    fn notice_entry_renders_distinctly() {
        let mut a = app();
        a.transcript
            .push(Entry::Notice("/models — Open the model picker".to_owned()));
        let text = rendered(&transcript_lines(&a, 80));
        assert!(text.contains("/models"), "notice text shown:\n{text}");
        assert!(text.contains("›"), "notice marker shown:\n{text}");
        assert!(!text.contains("▸"), "no legacy thin triangle:\n{text}");
    }

    #[test]
    fn shell_entry_renders_command_and_output() {
        // VAL-TUI-056: a shell-mode command and its output render in the transcript.
        let mut a = app();
        a.transcript.push(Entry::Shell(crate::shell::ShellOutcome {
            command: "echo cortex-shell-ok".to_owned(),
            output: "cortex-shell-ok\n[exit code: 0]".to_owned(),
            success: true,
        }));
        let text = rendered(&transcript_lines(&a, 80));
        assert!(
            text.contains("$ echo cortex-shell-ok"),
            "command header shown:\n{text}"
        );
        assert!(
            text.contains("cortex-shell-ok"),
            "command output shown:\n{text}"
        );
    }

    #[test]
    fn shell_mode_prompt_shows_a_distinct_affordance() {
        // VAL-TUI-055: the prompt wears a visible "shell" affordance while in shell mode.
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut a = app();
        a.on_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('!'),
            crossterm::event::KeyModifiers::NONE,
        ));
        assert!(a.shell_mode(), "shell mode engaged");

        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &a)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let screen: String = buffer
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect();
        assert!(
            screen.contains("shell"),
            "shell affordance drawn in the prompt:\n{screen}"
        );
    }

    #[test]
    fn prompt_renders_attachment_chips_above_the_input() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut a = app();
        let png = temp_png();
        a.set_input_value(&format!("/attach {}", png.display()));
        a.on_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        ));
        assert_eq!(a.composer_attachments().len(), 1, "attachment staged");

        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &a)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let screen: String = buffer
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect();
        assert!(
            screen.contains("📎") && screen.contains("image/png"),
            "composer chip line is visible in the prompt:\n{screen}"
        );
    }

    #[test]
    fn usage_totals_accumulate_after_a_turn() {
        let mut a = app();
        assert!(!a.usage.reported, "no usage is reported before a turn");

        let (tx, rx) = channel();
        a.begin_turn("q".to_owned(), rx, flag());
        tx.send(crate::turn::TurnMessage::Event(engine::RunnerEvent::Usage(
            llm::Usage {
                input_tokens: 12,
                output_tokens: 34,
                total_tokens: 46,
                cache_creation_input_tokens: None,
                cache_read_input_tokens: None,
                reasoning_output_tokens: None,
                cost: Some(0.0021),
            },
        )))
        .unwrap();
        a.poll_turn();

        assert!(a.usage.reported, "usage is reported after a turn");
        assert_eq!(a.usage.input_tokens, 12, "input tokens tracked");
        assert_eq!(a.usage.output_tokens, 34, "output tokens tracked");
        assert_eq!(a.usage.cost, Some(0.0021), "cost tracked");
    }

    #[test]
    fn constrained_footer_keeps_attention_cue_visible_and_quiet_suppresses_it() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut enabled = app();
        enabled.note_error("network disabled".to_owned());
        let enabled_footer = plain(&status_bar_line(&enabled, 70));
        assert!(
            enabled_footer.trim_start().starts_with("attention error"),
            "attention cue should be prioritized in the status bar's left zone: {enabled_footer}"
        );

        let enabled_backend = TestBackend::new(70, 12);
        let mut enabled_terminal = Terminal::new(enabled_backend).unwrap();
        enabled_terminal
            .draw(|frame| draw(frame, &enabled))
            .unwrap();
        let enabled_screen = screen(&enabled_terminal);
        assert!(
            enabled_screen.contains("attention error"),
            "narrow screens should still show the attention cue: {enabled_screen}"
        );

        let mut quiet = app();
        quiet.set_attention_config(crate::app::AttentionConfig {
            quiet: true,
            ..Default::default()
        });
        quiet.note_error("network disabled".to_owned());
        let quiet_backend = TestBackend::new(70, 12);
        let mut quiet_terminal = Terminal::new(quiet_backend).unwrap();
        quiet_terminal.draw(|frame| draw(frame, &quiet)).unwrap();
        let quiet_screen = screen(&quiet_terminal);
        assert!(
            !quiet_screen.contains("attention error"),
            "quiet mode should suppress the same attention cue on narrow screens: {quiet_screen}"
        );
        assert!(
            quiet_screen.contains("network disabled"),
            "quiet mode should still surface the normal error content: {quiet_screen}"
        );
        assert_ne!(
            enabled_screen, quiet_screen,
            "enabled and quiet snapshots should differ clearly"
        );
    }

    #[test]
    fn dialog_lines_list_items_and_highlight_the_selection() {
        // VAL-TUI-035: the model picker lists models (incl. the default) with the active highlighted.
        let dialog = Dialog {
            kind: DialogKind::Models,
            title: "Models".to_owned(),
            items: vec![
                "google/gemini-2.5-flash".to_owned(),
                "anthropic/claude-sonnet-4".to_owned(),
            ],
            selected: 0,
            empty: None,
        };
        let text = rendered(&dialog_lines(&dialog, &style::ThemeColors::dark()));
        assert!(
            text.contains("google/gemini-2.5-flash"),
            "default model listed:\n{text}"
        );
        assert!(
            text.contains("anthropic/claude-sonnet-4"),
            "second model listed:\n{text}"
        );
        assert!(
            text.contains("›"),
            "selection marker on the highlight:\n{text}"
        );
        assert!(!text.contains("▸"), "no legacy thin triangle:\n{text}");
    }

    #[test]
    fn dialog_lines_render_an_empty_state() {
        // VAL-TUI-046: the MCP view opens to a clear empty state when no servers are configured.
        let dialog = Dialog {
            kind: DialogKind::Mcp,
            title: "MCP servers".to_owned(),
            items: Vec::new(),
            selected: 0,
            empty: Some("No MCP servers configured.".to_owned()),
        };
        let text = rendered(&dialog_lines(&dialog, &style::ThemeColors::dark()));
        assert!(
            text.contains("No MCP servers configured."),
            "empty state shown:\n{text}"
        );
    }

    #[test]
    fn opened_dialog_renders_over_the_frame() {
        // The model picker, once opened, renders as a titled overlay listing models.
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut a = app();
        // Open the model picker through dispatch.
        for c in "/models".chars() {
            a.on_key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char(c),
                crossterm::event::KeyModifiers::NONE,
            ));
        }
        a.on_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        ));
        assert!(a.dialog_open(), "the model picker opened");

        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &a)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let rendered: String = buffer
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect();
        assert!(
            rendered.contains("Models"),
            "the dialog title is drawn:\n{rendered}"
        );
        assert!(
            rendered.contains("google/gemini-2.5-flash"),
            "the default model is drawn in the overlay"
        );
    }

    #[test]
    fn palette_command_lines_list_commands_and_highlight_the_selection() {
        // VAL-TUI-058: the palette lists command entries, with the highlighted row marked.
        let mut a = app();
        a.on_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('p'),
            crossterm::event::KeyModifiers::CONTROL,
        ));
        let text = rendered(&palette_command_lines(&a));
        assert!(text.contains("/help"), "a command entry is listed:\n{text}");
        assert!(text.contains("/models"), "model command listed:\n{text}");
        assert!(text.contains("›"), "the highlight marker is drawn:\n{text}");
        assert!(!text.contains("▸"), "no legacy thin triangle:\n{text}");
    }

    #[test]
    fn palette_command_lines_filter_to_the_query() {
        // VAL-TUI-059: typing narrows the rendered list to matching commands.
        let mut a = app();
        a.on_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('p'),
            crossterm::event::KeyModifiers::CONTROL,
        ));
        for c in "theme".chars() {
            a.on_key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char(c),
                crossterm::event::KeyModifiers::NONE,
            ));
        }
        let text = rendered(&palette_command_lines(&a));
        assert!(text.contains("/themes"), "the match is listed:\n{text}");
        assert!(
            !text.contains("/help"),
            "non-matches are filtered out:\n{text}"
        );
    }

    #[test]
    fn opened_palette_renders_over_the_frame() {
        // VAL-TUI-058: the palette, once opened, renders as a titled overlay listing commands.
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut a = app();
        a.on_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('p'),
            crossterm::event::KeyModifiers::CONTROL,
        ));
        assert!(a.palette_open(), "the palette opened");

        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &a)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let rendered: String = buffer
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect();
        assert!(
            rendered.contains("Command Palette"),
            "the palette title is drawn:\n{rendered}"
        );
        assert!(
            rendered.contains("/help"),
            "command entries are drawn in the overlay"
        );
    }

    fn press(a: &mut App, code: crossterm::event::KeyCode) {
        a.on_key(crossterm::event::KeyEvent::new(
            code,
            crossterm::event::KeyModifiers::NONE,
        ));
    }

    fn press_ctrl(a: &mut App, c: char) {
        a.on_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char(c),
            crossterm::event::KeyModifiers::CONTROL,
        ));
    }

    fn screen(terminal: &ratatui::Terminal<ratatui::backend::TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect()
    }

    #[test]
    fn transcript_matches_finds_all_occurrences() {
        let mut a = app();
        a.transcript
            .push(Entry::Notice("apple and apple".to_owned()));
        a.transcript.push(Entry::Notice("banana".to_owned()));
        let lines = transcript_lines(&a, 80);
        assert_eq!(transcript_matches(&lines, "apple").len(), 2);
        assert_eq!(transcript_matches(&lines, "banana").len(), 1);
        assert_eq!(transcript_matches(&lines, "missing").len(), 0);
        assert_eq!(transcript_matches(&lines, "").len(), 0);
    }

    #[test]
    fn find_highlight_applies_background_to_matches() {
        // VAL-TUI-077: the matched text carries a highlight background.
        let mut a = app();
        a.transcript
            .push(Entry::Notice("find ZTOKEN now".to_owned()));
        let mut lines = transcript_lines(&a, 80);
        highlight_find(&mut lines, "ztoken", 0, style::INFO, style::WARNING);
        let highlighted = lines
            .iter()
            .flat_map(|l| &l.spans)
            .any(|s| s.content.eq_ignore_ascii_case("ztoken") && s.style.bg.is_some());
        assert!(
            highlighted,
            "the matched token carries a highlight background"
        );
    }

    #[test]
    fn active_find_match_is_styled_distinctly() {
        // VAL-TUI-078: the active match is visually distinct from the other matches.
        let mut a = app();
        a.transcript.push(Entry::Notice("ZTOKEN one".to_owned()));
        a.transcript.push(Entry::Notice("ZTOKEN two".to_owned()));
        let mut lines = transcript_lines(&a, 80);
        highlight_find(&mut lines, "ztoken", 1, style::INFO, style::WARNING);
        let mut active = Vec::new();
        let mut base = Vec::new();
        for s in lines.iter().flat_map(|l| &l.spans) {
            if s.content.eq_ignore_ascii_case("ztoken") {
                if s.style.add_modifier.contains(Modifier::BOLD) {
                    active.push(s.style.bg);
                } else {
                    base.push(s.style.bg);
                }
            }
        }
        assert_eq!(
            active,
            vec![Some(style::WARNING)],
            "exactly one active match with the active background"
        );
        assert_eq!(
            base,
            vec![Some(style::INFO)],
            "the other match uses the base background"
        );
    }

    #[test]
    fn selected_message_gets_a_full_width_band() {
        // VAL-TUI-079: the focused message is marked with a full-width background band.
        let mut a = app();
        a.transcript.push(Entry::User("first".to_owned()));
        a.transcript.push(Entry::User("second".to_owned()));
        let width = 40;
        let RenderedTranscript {
            mut lines,
            entry_starts,
        } = render_transcript(&a, width);
        highlight_selected(&mut lines, &entry_starts, 1, width, style::SURFACE_2);
        let begin = entry_starts[1];
        let banded = lines[begin]
            .spans
            .iter()
            .all(|s| s.style.bg == Some(style::SURFACE_2));
        assert!(banded, "the selected message's first row is fully banded");
        let line_width: usize = lines[begin]
            .spans
            .iter()
            .map(|s| s.content.chars().count())
            .sum();
        assert_eq!(line_width, width, "the band is padded to the full width");
    }

    #[test]
    fn find_bar_renders_query_and_match_count() {
        // VAL-TUI-077/078: the find bar shows the live query and the active-match indicator.
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut a = app();
        a.transcript.push(Entry::Notice("apple one".to_owned()));
        a.transcript.push(Entry::Notice("apple two".to_owned()));
        press_ctrl(&mut a, 'f');
        for c in "apple".chars() {
            press(&mut a, crossterm::event::KeyCode::Char(c));
        }

        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &a)).unwrap();
        let rendered = screen(&terminal);
        assert!(
            rendered.contains("Find:"),
            "the find bar is drawn:\n{rendered}"
        );
        assert!(rendered.contains("apple"), "the query text is shown");
        assert!(
            rendered.contains("match 1 of 2"),
            "the active-match indicator is shown:\n{rendered}"
        );
    }

    #[test]
    fn scrolling_reveals_earlier_and_later_content() {
        // VAL-TUI-080: page up reveals earlier content; page down returns to the latest.
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut a = app();
        for i in 0..50 {
            a.transcript.push(Entry::Notice(format!("LINE{i:02}")));
        }
        // The MOTD card is now the always-present top of the scroll buffer, so the viewport must be
        // tall enough that scrolling to the top reveals the earliest transcript line beneath it.
        let backend = TestBackend::new(40, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| draw(frame, &a)).unwrap();
        assert!(
            screen(&terminal).contains("LINE49"),
            "the latest content is shown initially"
        );

        for _ in 0..40 {
            press(&mut a, crossterm::event::KeyCode::PageUp);
        }
        terminal.draw(|frame| draw(frame, &a)).unwrap();
        let scrolled = screen(&terminal);
        assert!(
            scrolled.contains("LINE00"),
            "earlier content is revealed after scrolling up:\n{scrolled}"
        );

        for _ in 0..40 {
            press(&mut a, crossterm::event::KeyCode::PageDown);
        }
        terminal.draw(|frame| draw(frame, &a)).unwrap();
        assert!(
            screen(&terminal).contains("LINE49"),
            "the latest content returns after scrolling down"
        );
    }

    #[test]
    fn copy_acknowledgement_renders_above_the_prompt() {
        // VAL-TUI-081: copying a message shows a confirmation acknowledgement.
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut a = app();
        a.transcript.push(Entry::User("hello world".to_owned()));
        press_ctrl(&mut a, 'y');

        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &a)).unwrap();
        let rendered = screen(&terminal);
        assert!(
            rendered.contains("Copied message"),
            "a copy acknowledgement is shown:\n{rendered}"
        );
    }

    fn render_92x30(a: &App) -> ratatui::buffer::Buffer {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(92, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, a)).unwrap();
        terminal.backend().buffer().clone()
    }

    fn screen_92x30(a: &App) -> String {
        render_92x30(a)
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect()
    }

    #[test]
    fn status_bar_three_zone_geometry_and_tokens() {
        // VAL-CONV-001/002/003: the bottom status bar shows three independently-positioned zones —
        // left `? for help` (`?` ORANGE_D + label FAINT), a mathematically-centered autonomy/perm
        // segment (`auto` AMBER + ` (high)` FAINT + ` · ` GHOST + perm DIM), and the REAL default
        // model id (never the mockup `opus-4.8`).
        let a = app(); // status.model = google/gemini-2.5-flash
        let theme = style::ThemeColors::from_name(a.theme());
        let w = 92usize;
        let line = status_bar_line(&a, w);
        let text = plain(&line);

        assert!(text.contains("? for help"), "left zone help hint:\n{text}");
        assert!(
            text.contains("auto (high) · allow all commands"),
            "center autonomy/permission segment:\n{text}"
        );
        assert!(
            text.contains("google/gemini-2.5-flash"),
            "right zone shows the real default model id:\n{text}"
        );
        assert!(!text.contains("opus-4.8"), "never the mockup model literal:\n{text}");

        // Each zone's segments are distinct style runs; verify the brand tokens per segment.
        let fg_of = |needle: &str| {
            line.spans
                .iter()
                .find(|s| s.content.as_ref() == needle)
                .and_then(|s| s.style.fg)
        };
        assert_eq!(fg_of("?"), Some(theme.border), "`?` is ORANGE_D");
        assert_eq!(fg_of(" for help"), Some(theme.text_muted), "help label is FAINT");
        assert_eq!(fg_of("auto"), Some(theme.secondary), "`auto` head is AMBER");
        assert_eq!(fg_of(" (high)"), Some(theme.text_muted), "`(high)` tail is FAINT");
        assert_eq!(fg_of(" · "), Some(theme.ghost), "separator is GHOST");
        assert_eq!(
            fg_of("allow all commands"),
            Some(theme.text_dim),
            "permission word is DIM"
        );

        // The center segment's midpoint sits at the terminal's horizontal center (±1 cell).
        let center = "auto (high) · allow all commands";
        let byte_start = text.find(center).expect("center segment present");
        let char_start = text[..byte_start].chars().count();
        let midpoint = char_start + center.chars().count() / 2;
        assert!(
            (midpoint as isize - (w / 2) as isize).abs() <= 1,
            "center midpoint {midpoint} is at W/2 {} (±1)",
            w / 2
        );
    }

    #[test]
    fn status_bar_model_is_flush_right_on_a_full_width_bar() {
        // VAL-CONV-003/004: the real model id is flush to the right margin (2-col gutter) and the
        // whole bottom row is painted edge-to-edge with the BAR surface (no bg=None black gap).
        let mut a = app();
        a.transcript.push(Entry::User("hi".to_owned())); // a conversation screen, not just home
        let theme = style::ThemeColors::from_name(a.theme());
        let buf = render_92x30(&a);
        let w = buf.area.width as usize;
        let h = buf.area.height as usize;

        for x in 0..w {
            assert_eq!(
                buf.content()[(h - 1) * w + x].bg,
                theme.surface[0],
                "status-bar cell at column {x} carries the BAR background"
            );
        }

        let row: Vec<char> = (0..w)
            .flat_map(|x| buf.content()[(h - 1) * w + x].symbol().chars())
            .collect();
        assert_eq!(row.len(), w, "one rendered char per status-bar column");
        let model = "google/gemini-2.5-flash";
        let model_w = model.chars().count();
        assert_eq!(row[w - 2], ' ', "right gutter column 1 is blank");
        assert_eq!(row[w - 1], ' ', "right gutter column 2 is blank");
        let model_cols: String = row[w - 2 - model_w..w - 2].iter().collect();
        assert_eq!(model_cols, model, "the model id is flush right");
    }

    #[test]
    fn composer_renders_rounded_orange_accent_and_prompt() {
        // VAL-CONV-005: the composer is a fixed rounded ORANGE_D box above the status bar, with a
        // `▌` ORANGE accent edge at the left inner column and a `›` ORANGE prompt glyph.
        let mut a = app();
        a.transcript.push(Entry::User("hi".to_owned())); // conversation screen
        let theme = style::ThemeColors::from_name(a.theme());
        let buf = render_92x30(&a);
        let w = buf.area.width as usize;
        let h = buf.area.height as usize;

        // Restrict the search to the composer rows (just above the status bar) so transcript
        // `›` marks and the MOTD card corners do not interfere.
        let find = |sym: &str| {
            ((h - 4)..(h - 1))
                .flat_map(|y| (0..w).map(move |x| (x, y)))
                .map(|(x, y)| &buf.content()[y * w + x])
                .find(|c| c.symbol() == sym)
        };

        let accent = find("▌").expect("composer accent edge present");
        assert_eq!(accent.fg, theme.primary, "`▌` accent is ORANGE");
        let prompt = find("›").expect("composer prompt present");
        assert_eq!(prompt.fg, theme.primary, "`›` prompt is ORANGE");

        for corner in ["╭", "╮", "╰", "╯"] {
            let cell = find(corner).unwrap_or_else(|| panic!("composer corner {corner} present"));
            assert_eq!(cell.fg, theme.border, "composer border {corner} is ORANGE_D");
        }
    }

    #[test]
    fn composer_hint_matches_active_state() {
        // VAL-CONV-006: the composer's right-aligned hint tracks the active state — `↵ send` in
        // chat, `↵ run   esc dismiss` with the `/` palette open, `⇥ insert   esc dismiss` with `@`.
        let mut chat = app();
        chat.transcript.push(Entry::User("hi".to_owned()));
        let s = screen_92x30(&chat);
        assert!(s.contains("↵ send"), "chat composer shows the send hint:\n{s}");

        let mut slash = app();
        slash.input.set_value("/");
        assert!(slash.slash_menu_open(), "the `/` palette is open");
        let s = screen_92x30(&slash);
        assert!(
            s.contains("↵ run   esc dismiss"),
            "the `/` composer shows the run hint:\n{s}"
        );

        let mut mention = app();
        mention.on_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('@'),
            crossterm::event::KeyModifiers::NONE,
        ));
        assert!(mention.mention_open(), "the `@` palette is open");
        let s = screen_92x30(&mention);
        assert!(
            s.contains("⇥ insert   esc dismiss"),
            "the `@` composer shows the insert hint:\n{s}"
        );
    }

    #[test]
    fn composer_placeholder_switches_home_vs_conversation() {
        // The composer placeholder is the home prompt on a fresh session and the reply prompt once
        // a conversation exists (the placeholder switches between home and conversation contexts).
        let home = app(); // empty transcript -> home
        let s = screen_92x30(&home);
        assert!(
            s.contains(crate::motd::HOME_PLACEHOLDER),
            "home placeholder on a fresh session:\n{s}"
        );
        assert!(!s.contains("reply to cortex"), "no reply placeholder on home:\n{s}");

        let mut convo = app();
        convo.transcript.push(Entry::User("hi".to_owned()));
        convo.transcript.push(Entry::Assistant(AssistantTurn {
            reasoning: String::new(),
            answer: "ok".to_owned(),
        }));
        let s = screen_92x30(&convo);
        assert!(
            s.contains("reply to cortex"),
            "conversation placeholder after an exchange:\n{s}"
        );
        assert!(
            !s.contains(crate::motd::HOME_PLACEHOLDER),
            "home placeholder gone once a conversation exists:\n{s}"
        );
    }

    #[test]
    fn composer_and_status_bar_are_fixed_chrome_across_conversation_screens() {
        // The composer (`▌`/`›`) and the 3-zone status bar render on every Wave 0 screen, not just
        // home: the typing state (text, no transcript) and a multi-turn conversation both show them.
        let mut typing = app();
        typing
            .input
            .set_value("the streaming buffer grows unbounded");
        let s = screen_92x30(&typing);
        assert!(s.contains("▌"), "composer accent present while typing:\n{s}");
        assert!(
            s.contains("? for help") && s.contains("google/gemini-2.5-flash"),
            "status bar present while typing:\n{s}"
        );

        let mut convo = app();
        for i in 0..4 {
            convo.transcript.push(Entry::User(format!("question {i}")));
            convo.transcript.push(Entry::Assistant(AssistantTurn {
                reasoning: String::new(),
                answer: format!("answer {i}"),
            }));
        }
        let s = screen_92x30(&convo);
        assert!(
            s.contains("▌") && s.contains("›"),
            "composer present in a conversation:\n{s}"
        );
        assert!(
            s.contains("auto (high) · allow all commands") && s.contains("google/gemini-2.5-flash"),
            "3-zone status bar present in a conversation:\n{s}"
        );
    }
}
