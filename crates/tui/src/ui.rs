//! Rendering for the Cortex Code TUI: a transcript area (the streamed conversation, with a distinct
//! reasoning region and a busy indicator while a turn is in flight), a bottom prompt box, and a
//! status footer that reflects token/cost usage. Layout is recomputed every frame so the screen
//! reflows on resize, and the transcript auto-scrolls to keep the latest content visible.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};

use tui_components::borders::ROUNDED_BORDER;
use tui_components::style;
use tui_components::toast::ToastLevel;

use crate::app::{
    App, AssistantTurn, Dialog, DialogKind, DiffViewMode, DiffViewerFocus, Entry, UsageSummary,
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
    if app.transcript.is_empty() && !app.is_busy() {
        draw_home(frame, app, area, theme);
        return;
    }

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
    let visible: Vec<Line> = lines.into_iter().skip(start).take(height).collect();

    frame.render_widget(
        Paragraph::new(visible).style(Style::default().fg(theme.text).bg(theme.background)),
        area,
    );
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

/// Append a user prompt: a bold marker header followed by the wrapped prompt body.
fn push_user(lines: &mut Vec<Line<'static>>, text: &str, width: usize, theme: &style::ThemeColors) {
    lines.push(Line::from(Span::styled(
        "› you",
        Style::default()
            .fg(theme.primary)
            .add_modifier(Modifier::BOLD),
    )));
    for wrapped in wrap(text, width) {
        lines.push(Line::from(Span::raw(wrapped)));
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

    lines.push(Line::from(Span::styled(
        "• cortex",
        Style::default()
            .fg(theme.secondary)
            .add_modifier(Modifier::BOLD),
    )));
    // Render the assistant answer as markdown so headings, emphasis, lists, inline code, fenced
    // code blocks, tables, and blockquotes display as structured elements rather than raw markup.
    // Re-rendered every frame from the accumulated answer, so it updates incrementally as the
    // turn streams.
    if turn.answer.trim().is_empty() {
        lines.push(Line::from(""));
    } else {
        let md_width = width.clamp(1, u16::MAX as usize) as u16;
        let renderer = frame::markdown::MarkdownRenderer::with_theme(
            frame::markdown::MarkdownTheme::from_name(theme_name),
        )
        .with_width(md_width);
        lines.extend(renderer.render(&turn.answer));
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
        let marker = if first { "▸ " } else { "  " };
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

fn draw_home(frame: &mut Frame, app: &App, area: Rect, theme: &style::ThemeColors) {
    let heading_style = Style::default()
        .fg(theme.primary)
        .add_modifier(Modifier::BOLD);
    let dim = Style::default()
        .fg(theme.text_dim)
        .add_modifier(Modifier::DIM);

    let leader = app.keybinds().leader.display();
    let body = Text::from(vec![
        Line::from(Span::styled("Cortex Code", heading_style)),
        Line::from(""),
        Line::from("Your terminal coding agent — start a new conversation."),
        Line::from(""),
        Line::from(Span::styled(
            "Type a message and press Enter to begin.",
            dim,
        )),
        Line::from(Span::styled(
            "Shift+Enter inserts a newline · /exit or Ctrl+C to quit.",
            dim,
        )),
        Line::from(Span::styled(
            format!("Press {leader} for keybindings (which-key)."),
            dim,
        )),
    ]);

    let block = Block::default()
        .style(Style::default().bg(theme.surface[0]))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border))
        .title(" Cortex Code ")
        .title_style(
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        );

    let paragraph = Paragraph::new(body)
        .block(block)
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true });

    frame.render_widget(paragraph, area);
}

fn draw_prompt(frame: &mut Frame, app: &App, area: Rect, theme: &style::ThemeColors) {
    // Shell mode wears a distinct title and border color so it is unmistakably set apart from chat.
    let shell = app.shell_mode();
    let mut block = Block::default()
        .style(Style::default().bg(theme.surface[1]).fg(theme.text))
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
    } else {
        block = block.title(" Prompt ").title_style(
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        );
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

    let value = app.input_value();
    if value.is_empty() {
        let placeholder = if shell {
            "$ run a shell command…  (Esc to exit)"
        } else {
            "Type a message…  (/ for commands, ! for shell)"
        };
        frame.render_widget(
            Paragraph::new(Span::styled(
                placeholder,
                Style::default()
                    .fg(theme.text_dim)
                    .add_modifier(Modifier::DIM),
            )),
            input_area,
        );
    } else {
        let lines: Vec<Line> = app
            .input
            .lines()
            .iter()
            .map(|l| Line::from(l.clone()))
            .collect();
        frame.render_widget(Paragraph::new(lines), input_area);
    }

    let cursor = app.cursor();
    let cx = input_area.x + (cursor.col as u16).min(input_area.width - 1);
    let cy = input_area.y + (cursor.row as u16).min(input_area.height - 1);
    frame.set_cursor_position((cx, cy));
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
/// Each row shows `/name  description`; the highlighted row carries a `▸` marker and bold name.
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
        let marker = if is_selected { "▸ " } else { "  " };
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
/// Each row shows a file path; the highlighted row carries a `▸` marker and bold text. A query that
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
        let marker = if is_selected { "▸ " } else { "  " };
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

/// Build a modal dialog's item rows: each selectable item with a `▸` marker and bold name on the
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
        let marker = if is_selected { "▸ " } else { "  " };
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
                let marker = if index == viewer.selected_file {
                    "▸ "
                } else {
                    "  "
                };
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
    // `▸` marker and bold styling, and the options keep their semantic colors (allow = success/green,
    // deny = error/red) so the choice is unmistakable in a snapshot.
    let option = |label: &str, index: usize, color| {
        let marked = if selected == index {
            format!("▸ {label}")
        } else {
            format!("  {label}")
        };
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
        let marker = if is_selected { "▸ " } else { "  " };
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
    let footer = Paragraph::new(footer_line(app))
        .style(Style::default().fg(theme.text).bg(theme.surface[0]));
    frame.render_widget(footer, area);
}

/// Build the status footer: model / agent / mode / session plus the token-and-cost usage segment.
/// Before any turn reports usage the segment reads `usage —`; after a turn it shows the running
/// token counts (and cost when reported), so a usage figure appears/increments versus before.
///
/// The label/value/separator colors come from the active theme, so switching theme visibly
/// recolors the status bar (the comparable element validated for a theme change).
pub fn footer_line(app: &App) -> Line<'static> {
    let theme = style::ThemeColors::from_name(app.theme());
    let label_color = theme.primary;
    let value_color = theme.text;
    let sep_color = theme.text_muted;
    let label = Style::default()
        .fg(label_color)
        .add_modifier(Modifier::BOLD);
    let value = Style::default().fg(value_color);
    let sep = move || Span::styled(" · ", Style::default().fg(sep_color));

    let mut spans = Vec::new();
    if let Some(attention) = app.attention_badge() {
        spans.push(Span::styled("attention ", label));
        spans.push(Span::styled(
            attention.to_owned(),
            Style::default()
                .fg(theme.warning)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(sep());
    }
    spans.extend([
        Span::styled("model ", label),
        Span::styled(app.status.model.clone(), value),
        sep(),
        Span::styled("agent ", label),
        Span::styled(app.status.agent.clone(), value),
        sep(),
        Span::styled("mode ", label),
        Span::styled(app.status.mode.clone(), value),
        sep(),
        Span::styled("variant ", label),
        Span::styled(app.status.variant.clone(), value),
        sep(),
        Span::styled("session ", label),
        Span::styled(app.status.session.clone(), value),
        sep(),
        Span::styled("usage ", label),
    ]);
    spans.push(Span::styled(usage_text(&app.usage), value));
    let (sub_total, sub_running, sub_done) = app.subagent_footer_summary();
    spans.push(sep());
    spans.push(Span::styled("subagents ", label));
    spans.push(Span::styled(
        format!("{sub_total} ({sub_running} running, {sub_done} done)"),
        value,
    ));
    spans.push(sep());
    spans.push(Span::styled("queued ", label));
    spans.push(Span::styled(app.queued_count().to_string(), value));
    Line::from(spans)
}

/// The footer's usage text: a placeholder dash before any turn, otherwise the token totals and
/// (when reported) the accumulated cost.
fn usage_text(usage: &UsageSummary) -> String {
    if !usage.reported {
        return "—".to_owned();
    }
    let mut text = format!("↑{} ↓{} tok", usage.input_tokens, usage.output_tokens);
    if let Some(cost) = usage.cost {
        text.push_str(&format!(" · ${cost:.4}"));
    }
    text
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
        let user = lines
            .iter()
            .find(|line| plain(line).starts_with("› you"))
            .and_then(|line| line.spans.first())
            .and_then(|span| span.style.fg);
        let assistant = lines
            .iter()
            .find(|line| plain(line).starts_with("• cortex"))
            .and_then(|line| line.spans.first())
            .and_then(|span| span.style.fg);

        assert_eq!(user, Some(theme.primary), "user marker uses theme primary");
        assert_eq!(
            assistant,
            Some(theme.secondary),
            "assistant marker uses theme secondary"
        );
    }

    #[test]
    fn draw_uses_theme_background_layers() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut a = app();
        a.set_theme("light".to_owned());
        a.transcript.push(Entry::User("hello".to_owned()));

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
            "prompt/overlay surfaces use themed surface background"
        );
        assert!(
            cells.iter().any(|c| c.bg == theme.surface[0]),
            "footer uses themed footer background"
        );
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
        push_tool(&mut running_lines, &running, 80, false, Some("⠋"), &theme);
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
            Some("⠋"),
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
        // The highlighted row carries the selection marker.
        assert!(text.contains("▸"), "selection marker shown:\n{text}");
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
        assert!(text.contains("▸"), "notice marker shown:\n{text}");
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
    fn footer_usage_appears_after_a_turn() {
        let mut a = app();
        let before = plain(&footer_line(&a));
        assert!(
            before.contains("usage —"),
            "no figure before a turn: {before}"
        );

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

        let after = plain(&footer_line(&a));
        assert!(after.contains("↑12"), "input tokens shown: {after}");
        assert!(after.contains("↓34"), "output tokens shown: {after}");
        assert!(after.contains("$0.0021"), "cost shown: {after}");
    }

    #[test]
    fn constrained_footer_keeps_attention_cue_visible_and_quiet_suppresses_it() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut enabled = app();
        enabled.note_error("network disabled".to_owned());
        let enabled_footer = plain(&footer_line(&enabled));
        assert!(
            enabled_footer.starts_with("attention error"),
            "attention cue should be prioritized at the start of the footer: {enabled_footer}"
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
            text.contains("▸"),
            "selection marker on the highlight:\n{text}"
        );
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
        assert!(text.contains("▸"), "the highlight marker is drawn:\n{text}");
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
        let backend = TestBackend::new(40, 12);
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
}
