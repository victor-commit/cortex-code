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

use crate::app::{App, AssistantTurn, Dialog, DialogKind, Entry, UsageSummary};
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

/// Renders the whole frame for the current [`App`] state.
pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();

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

    draw_transcript(frame, app, chunks[0]);
    draw_prompt(frame, app, chunks[1]);
    draw_footer(frame, app, chunks[2]);
    // The slash menu floats above the prompt, so it is drawn last to overlay the transcript.
    draw_slash_menu(frame, app, chunks[1]);
    // The `@`-file picker also floats above the prompt (mutually exclusive with the slash menu).
    draw_mention_menu(frame, app, chunks[1]);
    // The find-in-chat search bar floats above the prompt while find mode is active.
    draw_find_bar(frame, app, chunks[1]);
    // A transient acknowledgement (e.g. a copy confirmation) shows just above the prompt.
    draw_ack(frame, app, chunks[1]);
    // The sidebar occupies its reserved column beside the main content.
    if let Some(sidebar) = sidebar_area {
        draw_sidebar(frame, app, sidebar);
    }
    // A modal dialog (picker/status view) overlays everything else when open.
    draw_dialog(frame, app, area);
    // The command palette overlays everything when open (mutually exclusive with a dialog).
    draw_palette(frame, app, area);
    // The which-key overlay lists the active leader bindings while the leader chord is armed.
    draw_which_key(frame, app, area);
    // An interactive permission prompt is modal: it overlays everything (it captures every key
    // while the turn worker is blocked awaiting the decision).
    draw_permission(frame, app, area);
    // Transient toast notifications float above everything else (top-right) so they remain visible
    // over any open dialog/overlay, and auto-dismiss without disturbing the layout beneath them.
    draw_toasts(frame, app, area);
}

/// Number of text rows the prompt box should show (clamped to available height).
fn prompt_rows(app: &App, total_height: u16) -> u16 {
    let logical = app.input.line_count().max(1);
    // Leave room for at least one transcript row, the prompt borders, and the footer.
    let budget = total_height.saturating_sub(1 + 2 + 1).max(1) as usize;
    logical.min(MAX_PROMPT_ROWS).min(budget).max(1) as u16
}

/// Render the transcript area: the home screen before any exchange, otherwise the streamed
/// conversation. The view is normally pinned to the latest content, but the message-selection
/// highlight, find-match highlight, and the scroll offset let the user reach and mark earlier
/// content; the most-recent content returns into view when the scroll offset is zero.
fn draw_transcript(frame: &mut Frame, app: &App, area: Rect) {
    if app.transcript.is_empty() && !app.is_busy() {
        draw_home(frame, app, area);
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
        let theme = style::ThemeColors::from_name(app.theme());
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
                style::INFO,
                style::WARNING,
            );
        }
    }

    // Apply the scroll offset: 0 pins to the bottom (latest), larger values reveal earlier content.
    let max_scroll = lines.len().saturating_sub(height);
    let scroll = app.scroll().min(max_scroll);
    let start = max_scroll - scroll;
    let visible: Vec<Line> = lines.into_iter().skip(start).take(height).collect();

    frame.render_widget(Paragraph::new(visible), area);
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
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut entry_starts: Vec<usize> = Vec::with_capacity(app.transcript.len());
    for entry in &app.transcript {
        entry_starts.push(lines.len());
        match entry {
            Entry::User(text) => push_user(&mut lines, text, width),
            Entry::Assistant(turn) => push_assistant(&mut lines, turn, width),
            Entry::Diff(diff) => push_diff(&mut lines, diff),
            Entry::Tool(card) => {
                push_tool(
                    &mut lines,
                    card,
                    width,
                    app.expand_tools,
                    app.spinner_glyph(),
                );
            }
            Entry::Notice(message) => push_notice(&mut lines, message, width),
            Entry::Shell(outcome) => push_shell(&mut lines, outcome, width),
            Entry::Help => push_help(&mut lines, app),
            Entry::Error(message) => push_error(&mut lines, message, width),
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
                .fg(Color::Yellow)
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
fn push_user(lines: &mut Vec<Line<'static>>, text: &str, width: usize) {
    lines.push(Line::from(Span::styled(
        "› you",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )));
    for wrapped in wrap(text, width) {
        lines.push(Line::from(Span::raw(wrapped)));
    }
    lines.push(Line::from(""));
}

/// Append an assistant reply: an optional distinct reasoning region, then the answer body.
fn push_assistant(lines: &mut Vec<Line<'static>>, turn: &AssistantTurn, width: usize) {
    // Reasoning, when present, renders in a distinct dimmed/labeled region separate from the answer.
    // When absent, nothing is emitted (no empty-reasoning artifact).
    if !turn.reasoning.trim().is_empty() {
        let label = Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::ITALIC);
        let body = Style::default()
            .fg(Color::DarkGray)
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
            .fg(Color::Green)
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
        lines.extend(frame::markdown::render_markdown(&turn.answer, md_width));
    }
    lines.push(Line::from(""));
}

/// Append a file-change diff: a labeled marker line, then the colored unified-diff lines (file-path
/// header, `@@` hunks, and green `+` / red `-` change lines), then a trailing blank line.
fn push_diff(lines: &mut Vec<Line<'static>>, diff: &FileDiff) {
    let verb = if diff.is_new { "created" } else { "edited" };
    lines.push(Line::from(Span::styled(
        format!("✎ {verb} ({}+ {}-)", diff.additions, diff.removals),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )));
    lines.extend(diff.render());
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
) {
    let box_w = width.clamp(8, TOOL_CARD_WIDTH);
    let (accent, glyph, state) = match card.status {
        ToolStatus::Running => (style::INFO, "⚙", "running"),
        ToolStatus::Completed => (style::SUCCESS, "✓", "done"),
        ToolStatus::Failed => (style::ERROR, "✗", "error"),
    };
    let border = Style::default().fg(accent);
    let border_bold = border.add_modifier(Modifier::BOLD);
    let text_style = Style::default().fg(style::TEXT);
    let detail_style = Style::default().fg(style::TEXT_DIM);
    let error_style = Style::default().fg(style::ERROR);

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
fn push_notice(lines: &mut Vec<Line<'static>>, message: &str, width: usize) {
    let style = Style::default().fg(style::INFO);
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
fn push_shell(lines: &mut Vec<Line<'static>>, outcome: &crate::shell::ShellOutcome, width: usize) {
    let prompt_style = Style::default()
        .fg(style::WARNING)
        .add_modifier(Modifier::BOLD);
    lines.push(Line::from(vec![
        Span::styled("$ ", prompt_style),
        Span::styled(
            outcome.command.clone(),
            Style::default()
                .fg(style::TEXT)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    let body = if outcome.success {
        Style::default().fg(style::TEXT_DIM)
    } else {
        Style::default().fg(style::ERROR)
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

/// Append the `/help` command reference: a titled panel listing every built-in slash command (from
/// the registry) with its description, then the keybindings (including the active leader bindings).
/// This is the visible effect of `/help`.
fn push_help(lines: &mut Vec<Line<'static>>, app: &App) {
    let title = Style::default()
        .fg(style::INFO)
        .add_modifier(Modifier::BOLD);
    let section = Style::default()
        .fg(style::WARNING)
        .add_modifier(Modifier::BOLD);
    let name = Style::default()
        .fg(style::TEXT)
        .add_modifier(Modifier::BOLD);
    let desc = Style::default().fg(style::TEXT_DIM);

    lines.push(Line::from(Span::styled(
        "Help — commands & keybindings",
        title,
    )));
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled("Commands", section)));
    for command in crate::slash::BUILTINS {
        lines.push(Line::from(vec![
            Span::styled(format!("  /{:<9}", command.name), name),
            Span::styled(format!("  {}", command.description), desc),
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
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {:<16}", format!("{leader} {}", chord.display())),
                name,
            ),
            Span::styled(format!("  {}", action.label()), desc),
        ]));
    }
    lines.push(Line::from(""));
}

/// Append a turn-level error as a distinct, red-styled block.
fn push_error(lines: &mut Vec<Line<'static>>, message: &str, width: usize) {
    let style = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);
    lines.push(Line::from(Span::styled("✗ error", style)));
    for wrapped in wrap(message, width) {
        lines.push(Line::from(Span::styled(
            wrapped,
            Style::default().fg(Color::Red),
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

fn draw_home(frame: &mut Frame, app: &App, area: Rect) {
    let heading_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let dim = Style::default().add_modifier(Modifier::DIM);

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
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(" Cortex Code ");

    let paragraph = Paragraph::new(body)
        .block(block)
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true });

    frame.render_widget(paragraph, area);
}

fn draw_prompt(frame: &mut Frame, app: &App, area: Rect) {
    // Shell mode wears a distinct title and border color so it is unmistakably set apart from chat.
    let shell = app.shell_mode();
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded);
    if shell {
        block = block
            .title(" ! shell ")
            .title_style(
                Style::default()
                    .fg(style::WARNING)
                    .add_modifier(Modifier::BOLD),
            )
            .border_style(Style::default().fg(style::WARNING));
    } else {
        block = block.title(" Prompt ");
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);

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
                Style::default().add_modifier(Modifier::DIM),
            )),
            inner,
        );
    } else {
        let lines: Vec<Line> = app
            .input
            .lines()
            .iter()
            .map(|l| Line::from(l.clone()))
            .collect();
        frame.render_widget(Paragraph::new(lines), inner);
    }

    if inner.width > 0 && inner.height > 0 {
        let cursor = app.cursor();
        let cx = inner.x + (cursor.col as u16).min(inner.width - 1);
        let cy = inner.y + (cursor.row as u16).min(inner.height - 1);
        frame.set_cursor_position((cx, cy));
    }
}

/// Build the slash-command menu's rows for the current query, or `None` when the menu is closed.
/// Each row shows `/name  description`; the highlighted row carries a `▸` marker and bold name.
/// A non-empty query that matches nothing yields a single graceful "no matching commands" row.
fn slash_menu_lines(app: &App) -> Option<Vec<Line<'static>>> {
    let _ = app.slash_query()?;
    let matches = app.slash_matches();
    if matches.is_empty() {
        return Some(vec![Line::from(Span::styled(
            "  no matching commands",
            Style::default()
                .fg(style::TEXT_DIM)
                .add_modifier(Modifier::ITALIC),
        ))]);
    }

    let selected = app.slash_selected();
    let mut lines = Vec::with_capacity(matches.len());
    for (i, command) in matches.iter().enumerate() {
        let is_selected = i == selected;
        let marker = if is_selected { "▸ " } else { "  " };
        let name_style = if is_selected {
            Style::default()
                .fg(style::INFO)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(style::TEXT)
        };
        lines.push(Line::from(vec![
            Span::styled(marker, Style::default().fg(style::INFO)),
            Span::styled(format!("/{}", command.name), name_style),
            Span::raw("  "),
            Span::styled(
                command.description.to_owned(),
                Style::default().fg(style::TEXT_DIM),
            ),
        ]));
    }
    Some(lines)
}

/// Render the slash-command autocomplete menu as a bordered popup floating directly above the
/// prompt box. The view scrolls to keep the highlighted row visible when the match list is long.
fn draw_slash_menu(frame: &mut Frame, app: &App, prompt_area: Rect) {
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
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(style::INFO))
        .title(" / commands ")
        .title_style(
            Style::default()
                .fg(style::INFO)
                .add_modifier(Modifier::BOLD),
        );
    let inner = block.inner(popup);

    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);
    frame.render_widget(Paragraph::new(shown), inner);
}

/// Build the `@`-file picker's rows for the current query, or `None` when the picker is closed.
/// Each row shows a file path; the highlighted row carries a `▸` marker and bold text. A query that
/// matches no file yields a single graceful "no matching files" row.
fn mention_menu_lines(app: &App) -> Option<Vec<Line<'static>>> {
    if !app.mention_open() {
        return None;
    }
    let matches = app.mention_matches();
    if matches.is_empty() {
        return Some(vec![Line::from(Span::styled(
            "  no matching files",
            Style::default()
                .fg(style::TEXT_DIM)
                .add_modifier(Modifier::ITALIC),
        ))]);
    }

    let selected = app.mention_selected();
    let mut lines = Vec::with_capacity(matches.len());
    for (i, path) in matches.iter().enumerate() {
        let is_selected = i == selected;
        let marker = if is_selected { "▸ " } else { "  " };
        let path_style = if is_selected {
            Style::default()
                .fg(style::INFO)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(style::TEXT)
        };
        lines.push(Line::from(vec![
            Span::styled(marker, Style::default().fg(style::INFO)),
            Span::styled(path.clone(), path_style),
        ]));
    }
    Some(lines)
}

/// Render the `@`-file picker as a bordered popup floating directly above the prompt box, scrolling
/// to keep the highlighted row visible when the match list is long.
fn draw_mention_menu(frame: &mut Frame, app: &App, prompt_area: Rect) {
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
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(style::INFO))
        .title(" @ files ")
        .title_style(
            Style::default()
                .fg(style::INFO)
                .add_modifier(Modifier::BOLD),
        );
    let inner = block.inner(popup);

    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);
    frame.render_widget(Paragraph::new(shown), inner);
}

/// Render the find-in-chat search bar as a single-row bordered popup floating above the prompt. It
/// shows the live query and a `match N of M` indicator (or `no matches` / a hint), so the search
/// affordance and the active-match position are both visible in a snapshot.
fn draw_find_bar(frame: &mut Frame, app: &App, prompt_area: Rect) {
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
            Style::default()
                .fg(style::INFO)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(find.query.clone(), Style::default().fg(style::TEXT)),
        Span::raw("  "),
        Span::styled(status, Style::default().fg(style::TEXT_DIM)),
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
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(style::INFO))
        .title(" find ")
        .title_style(
            Style::default()
                .fg(style::INFO)
                .add_modifier(Modifier::BOLD),
        );
    let inner = block.inner(popup);

    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);
    frame.render_widget(Paragraph::new(row), inner);
}

/// Render a transient acknowledgement (e.g. a copy confirmation) as a single highlighted row
/// directly above the prompt. Shown only in ordinary mode (never while the find bar is open) so the
/// two affordances do not collide.
fn draw_ack(frame: &mut Frame, app: &App, prompt_area: Rect) {
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
            .fg(style::SUCCESS)
            .add_modifier(Modifier::BOLD),
    ));
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(line), area);
}

/// The fg color and human label for a toast level. Error toasts use the error color (a distinct red)
/// so an error notification is unmistakable from a success/info one; the level is also encoded in the
/// ASCII icon and the label text, so the distinction survives a monochrome snapshot.
fn toast_decor(level: ToastLevel) -> (Color, &'static str) {
    match level {
        ToastLevel::Success => (style::SUCCESS, "Success"),
        ToastLevel::Info => (style::INFO, "Info"),
        ToastLevel::Warning => (style::WARNING, "Warning"),
        ToastLevel::Error => (style::ERROR, "Error"),
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
fn draw_toasts(frame: &mut Frame, app: &App, area: Rect) {
    let toasts = app.toasts();
    if toasts.is_empty() || area.width < 18 || area.height < 4 {
        return;
    }

    let theme = style::ThemeColors::from_name(app.theme());
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

        let (color, label) = toast_decor(toast.level);
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
fn dialog_lines(dialog: &Dialog) -> Vec<Line<'static>> {
    if dialog.items.is_empty() {
        let message = dialog
            .empty
            .clone()
            .unwrap_or_else(|| "(nothing to show)".to_owned());
        return vec![Line::from(Span::styled(
            format!("  {message}"),
            Style::default()
                .fg(style::TEXT_DIM)
                .add_modifier(Modifier::ITALIC),
        ))];
    }

    let selected = dialog.selected();
    let mut lines = Vec::with_capacity(dialog.items.len());
    for (i, item) in dialog.items.iter().enumerate() {
        let is_selected = i == selected;
        let marker = if is_selected { "▸ " } else { "  " };
        let item_style = if is_selected {
            Style::default()
                .fg(style::INFO)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(style::TEXT)
        };
        lines.push(Line::from(vec![
            Span::styled(marker, Style::default().fg(style::INFO)),
            Span::styled(item.clone(), item_style),
        ]));
    }
    lines
}

/// Render the open modal dialog as a centered bordered popup over the whole frame. The list scrolls
/// to keep the highlighted row visible, and a footer hint reminds the user how to navigate/close.
fn draw_dialog(frame: &mut Frame, app: &App, area: Rect) {
    let Some(dialog) = app.dialog() else {
        return;
    };
    if area.width < 8 || area.height < 6 {
        return;
    }

    let rows = dialog_lines(dialog);
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
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(style::INFO))
        .title(format!(" {} ", dialog.title))
        .title_style(
            Style::default()
                .fg(style::INFO)
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
                Style::default()
                    .fg(style::INFO)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(text.to_owned(), Style::default().fg(style::TEXT)),
            Span::styled("█", Style::default().fg(style::INFO)),
        ]));
    }
    shown.push(Line::from(Span::styled(
        format!("  {hint}"),
        Style::default().fg(style::TEXT_DIM),
    )));

    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);
    frame.render_widget(Paragraph::new(shown), inner);
}

/// Render the interactive permission prompt: a centered modal naming the ask-gated tool and its
/// target with selectable Allow/Deny options, drawn over everything while a decision is pending.
fn draw_permission(frame: &mut Frame, app: &App, area: Rect) {
    let Some((tool, subject, selected)) = app.permission_prompt() else {
        return;
    };
    if area.width < 8 || area.height < 6 {
        return;
    }

    // The Allow/Deny option row: the highlighted choice carries a `▸` marker and bold styling, and
    // the options keep their semantic colors (allow = success/green, deny = error/red) so the choice
    // is unmistakable in a snapshot.
    let allow_selected = selected == 0;
    let options = Line::from(vec![
        Span::styled(
            if allow_selected {
                "▸ Allow"
            } else {
                "  Allow"
            },
            option_style(style::SUCCESS, allow_selected),
        ),
        Span::raw("    "),
        Span::styled(
            if allow_selected { "  Deny" } else { "▸ Deny" },
            option_style(style::ERROR, !allow_selected),
        ),
    ]);
    let hint = "y allow · n deny · ←/→ select · Enter confirm · Esc deny";

    let rows: Vec<Line<'static>> = vec![
        Line::from(vec![
            Span::styled(
                tool.to_owned(),
                Style::default()
                    .fg(style::WARNING)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                " requests permission to act on:",
                Style::default().fg(style::TEXT),
            ),
        ]),
        Line::from(Span::styled(
            format!("  {subject}"),
            Style::default()
                .fg(style::TEXT)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        options,
        Line::from(Span::styled(hint, Style::default().fg(style::TEXT_DIM))),
    ];

    let content_w = rows.iter().map(Line::width).max().unwrap_or(0);
    let max_w = (area.width as usize) * 9 / 10;
    let width = (content_w + 4).clamp(28, max_w.max(28)) as u16;
    let height = (rows.len() as u16).saturating_add(2);
    let popup = centered_rect(width, height, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(style::WARNING))
        .title(" Permission required ")
        .title_style(
            Style::default()
                .fg(style::WARNING)
                .add_modifier(Modifier::BOLD),
        );
    let inner = block.inner(popup);

    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);
    frame.render_widget(Paragraph::new(rows), inner);
}

/// The style for a permission option: its semantic color, bold and reversed when it is the
/// highlighted choice so the active option stands out against the modal background.
fn option_style(color: Color, selected: bool) -> Style {
    let base = Style::default().fg(color);
    if selected {
        base.add_modifier(Modifier::BOLD | Modifier::REVERSED)
    } else {
        base.add_modifier(Modifier::DIM)
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
        _ => "↑/↓ to move · Esc to close",
    }
}

/// Build the command palette's command rows for the current query: each matching command as
/// `/name  description` with the highlighted row marked, or a single graceful "no matching
/// commands" row when the query matches nothing.
fn palette_command_lines(app: &App) -> Vec<Line<'static>> {
    let matches = app.palette_matches();
    if matches.is_empty() {
        return vec![Line::from(Span::styled(
            "  no matching commands",
            Style::default()
                .fg(style::TEXT_DIM)
                .add_modifier(Modifier::ITALIC),
        ))];
    }
    let selected = app.palette_selected();
    let mut lines = Vec::with_capacity(matches.len());
    for (i, command) in matches.iter().enumerate() {
        let is_selected = i == selected;
        let marker = if is_selected { "▸ " } else { "  " };
        let name_style = if is_selected {
            Style::default()
                .fg(style::INFO)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(style::TEXT)
        };
        lines.push(Line::from(vec![
            Span::styled(marker, Style::default().fg(style::INFO)),
            Span::styled(format!("/{}", command.name), name_style),
            Span::raw("  "),
            Span::styled(
                command.description.to_owned(),
                Style::default().fg(style::TEXT_DIM),
            ),
        ]));
    }
    lines
}

/// Render the command palette as a centered bordered popup: a search line reflecting the typed
/// query sits atop the filtered command list, which scrolls to keep the highlighted row visible.
fn draw_palette(frame: &mut Frame, app: &App, area: Rect) {
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
            Style::default()
                .fg(style::INFO)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(query.to_owned(), Style::default().fg(style::TEXT)),
        Span::styled("▏", Style::default().fg(style::INFO)),
    ]);
    let hint = "↑/↓ to move · Enter to run · Esc to close";

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

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(style::INFO))
        .title(" Command Palette ")
        .title_style(
            Style::default()
                .fg(style::INFO)
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
        Style::default().fg(style::TEXT_DIM),
    )));

    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);
    frame.render_widget(Paragraph::new(shown), inner);
}

/// Render the session sidebar: a bordered panel beside the main content listing the active session
/// context (model / agent / mode / session) and live counters (transcript messages, token usage),
/// so its presence is unambiguous in a snapshot.
fn draw_sidebar(frame: &mut Frame, app: &App, area: Rect) {
    if area.width < 4 || area.height < 3 {
        return;
    }
    let label = Style::default()
        .fg(style::INFO)
        .add_modifier(Modifier::BOLD);
    let value = Style::default().fg(style::TEXT);
    let dim = Style::default().fg(style::TEXT_DIM);

    let mut lines: Vec<Line<'static>> = Vec::new();
    let field = |lines: &mut Vec<Line<'static>>, name: &str, val: String| {
        lines.push(Line::from(vec![
            Span::styled(format!("{name}: "), label),
            Span::styled(val, value),
        ]));
    };
    field(&mut lines, "Model", app.status.model.clone());
    field(&mut lines, "Agent", app.status.agent.clone());
    field(&mut lines, "Mode", app.status.mode.clone());
    field(&mut lines, "Session", app.status.session.clone());
    lines.push(Line::from(""));
    field(&mut lines, "Messages", app.transcript.len().to_string());
    field(
        &mut lines,
        "Tokens",
        if app.usage.reported {
            app.usage.total_tokens.to_string()
        } else {
            "—".to_owned()
        },
    );
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("{} B  toggle", app.keybinds().leader.display()),
        dim,
    )));

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(style::INFO))
        .title(" Session ")
        .title_style(
            Style::default()
                .fg(style::INFO)
                .add_modifier(Modifier::BOLD),
        );
    let inner = block.inner(area);

    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

/// Render the which-key overlay while the leader chord is armed: a centered popup listing each
/// active leader binding (its key and the action it runs) so the bindings are discoverable.
fn draw_which_key(frame: &mut Frame, app: &App, area: Rect) {
    if !app.leader_pending() {
        return;
    }
    if area.width < 8 || area.height < 6 {
        return;
    }

    let leader = app.keybinds().leader.display();
    let key_style = Style::default()
        .fg(style::INFO)
        .add_modifier(Modifier::BOLD);
    let label_style = Style::default().fg(style::TEXT);

    let mut rows: Vec<Line<'static>> = Vec::new();
    for (chord, action) in app.keybinds().entries() {
        rows.push(Line::from(vec![
            Span::styled(format!("  {:<3}", chord.display()), key_style),
            Span::styled(format!("  {}", action.label()), label_style),
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
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(style::INFO))
        .title(format!(" Keybindings · leader {leader} "))
        .title_style(
            Style::default()
                .fg(style::INFO)
                .add_modifier(Modifier::BOLD),
        );
    let inner = block.inner(popup);

    let end = rows.len().min(visible);
    let mut shown: Vec<Line<'static>> = rows[..end].to_vec();
    shown.push(Line::from(Span::styled(
        format!("  {hint}"),
        Style::default().fg(style::TEXT_DIM),
    )));

    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);
    frame.render_widget(Paragraph::new(shown), inner);
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

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let theme = style::ThemeColors::from_name(app.theme());
    let footer = Paragraph::new(footer_line(app)).style(Style::default().bg(theme.surface[0]));
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

    let mut spans = vec![
        Span::styled("model ", label),
        Span::styled(app.status.model.clone(), value),
        sep(),
        Span::styled("agent ", label),
        Span::styled(app.status.agent.clone(), value),
        sep(),
        Span::styled("mode ", label),
        Span::styled(app.status.mode.clone(), value),
        sep(),
        Span::styled("session ", label),
        Span::styled(app.status.session.clone(), value),
        sep(),
        Span::styled("usage ", label),
    ];
    spans.push(Span::styled(usage_text(&app.usage), value));
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
    use std::sync::atomic::AtomicBool;
    use std::sync::mpsc::channel;

    fn app() -> App {
        App::new(Status {
            model: "google/gemini-2.5-flash".to_owned(),
            agent: "build".to_owned(),
            mode: "build".to_owned(),
            session: "new".to_owned(),
        })
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
        push_tool(&mut lines, &card, 80, false, Some("⠋"));
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
        push_tool(&mut lines, &card, 80, false, None);
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
        push_tool(&mut collapsed, &card, 80, false, None);
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
        push_tool(&mut expanded, &card, 80, true, None);
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
        push_tool(&mut lines, &failed, 80, false, None);
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
        push_tool(&mut ok_lines, &ok, 80, false, None);

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
        let text = rendered(&dialog_lines(&dialog));
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
        let text = rendered(&dialog_lines(&dialog));
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
