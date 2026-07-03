//! Rendering for the Cortex Code TUI: a transcript area (the streamed conversation, with a distinct
//! reasoning region and a busy indicator while a turn is in flight), a bottom prompt box, and a
//! status footer that reflects token/cost usage. Layout is recomputed every frame so the screen
//! reflows on resize, and the transcript auto-scrolls to keep the latest content visible.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

use tui_components::style;
use tui_components::toast::ToastLevel;

use crate::app::{App, AssistantTurn, DialogKind, DiffViewMode, Entry};
use crate::attachments::InputAttachment;
use crate::diff::FileDiff;
use crate::glyphs;
use crate::tool_card::{ToolCard, ToolStatus};

/// Maximum number of slash-command rows shown before the menu scrolls.
const MAX_SLASH_ROWS: usize = 10;

/// Maximum number of list rows shown inside a modal dialog before it scrolls.
const MAX_DIALOG_ROWS: usize = 14;

/// Maximum number of visible prompt text rows before the box stops growing.
const MAX_PROMPT_ROWS: usize = 8;

/// Maximum number of output lines shown inside an expanded tool card before truncation.
const MAX_TOOL_DETAIL_LINES: usize = 40;

/// Maximum number of diff rows shown inside an expanded diff card before truncation.
const MAX_DIFF_BODY_ROWS: usize = 40;

/// Width (in columns) of the right-hand sidebar panel box when the split is shown (`SBW`).
const SIDEBAR_PANEL_WIDTH: u16 = 32;

/// Left content margin (`MAINX`) of the conversation pane, and the right margin reserved past the
/// sidebar box, when the split is shown. The sidebar's left edge is `SBX = W - RIGHT_MARGIN - SBW`
/// and the conversation width is `MAINW = SBX - MAINX - 1` (one clean gap column between the panes).
const SPLIT_LEFT_MARGIN: u16 = 2;
const SPLIT_RIGHT_MARGIN: u16 = 2;

/// Minimum terminal width required to open the split; below this the conversation stays full-width
/// (so the left pane keeps a usable width). At 92 cols this yields the reference `MAINW = 55`.
const MIN_SPLIT_WIDTH: u16 = 56;

/// Left margin (columns) that transcript content (the `›`/`✦` author marks and the scrollback
/// `▲ N lines above` marker) is inset from the viewport edge, aligning with the MOTD card frame and
/// the status-bar zones.
const CONTENT_MARGIN: usize = 2;

/// Maximum number of toasts drawn at once in the overlay (matches the app's retention cap).
const MAX_VISIBLE_TOASTS: usize = 4;
/// Left indent (columns) of a hunk row's content within the `/diff` viewer accordion: the line
/// number sits at this origin, the `+`/`-`/space sign at origin+6, and the code at origin+8.
const DIFF_HUNK_ORIGIN: usize = 3;

/// Renders the whole frame for the current [`App`] state.
pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let theme = style::ThemeColors::from_name(app.theme());

    frame.render_widget(
        Block::default().style(Style::default().bg(theme.background)),
        area,
    );

    // A full-screen Wave-2 view (status/mcp/…) takes over the whole frame with the shared view
    // chrome; it fully replaces the conversation until dismissed (Esc), so nothing else is drawn.
    if app.active_view().is_some() {
        draw_full_screen_view(frame, app, area, &theme);
        return;
    }

    // When the sidebar is open the screen splits into a live conversation on the LEFT and the
    // sidebar column on the RIGHT, wrapped in the shared `✦ Cortex Code · <panel>` chrome (header +
    // rule + a full-width status bar). Otherwise the conversation spans the full width and draws its
    // own one-line status bar — except in plan mode, which wraps the conversation in the same shared
    // chrome under a `plan mode` breadcrumb (and gains an amber footer banner above the composer).
    let split = sidebar_split(app, area);
    let plan_mode = app.plan_mode();
    let shell_mode = app.shell_mode();
    let (main_area, own_footer) = if let Some(split) = split {
        // Draw the breadcrumb header + rule + full-width status bar; the conversation only uses the
        // returned left region (the sidebar box occupies the right region).
        crate::view::render_chrome(frame, area, app, app.sidebar_panel().crumb(), &theme);
        (split.conversation, false)
    } else if plan_mode {
        // Plan mode: wrap the conversation in the shared chrome under a `plan mode` crumb; the shared
        // status bar (which reads `read-only · plan`) is drawn by the chrome, so no own footer.
        let body = crate::view::render_chrome(frame, area, app, "plan mode", &theme);
        (body, false)
    } else if shell_mode {
        // `!` shell mode: wrap the conversation in the shared chrome under a `shell` crumb; the
        // shared status bar (which reads `manual · ! shell mode`) is drawn by the chrome.
        let body = crate::view::render_chrome(frame, area, app, "shell", &theme);
        (body, false)
    } else if !app.composer_attachments().is_empty() {
        // Attachments staged: wrap the conversation in the shared chrome under an `attachments`
        // crumb (perm_attach.png); the shared 3-zone status bar is drawn by the chrome.
        let body = crate::view::render_chrome(frame, area, app, "attachments", &theme);
        (body, false)
    } else if markdown_rendering_active(app) {
        // A rendered/streaming markdown answer: wrap the conversation in the shared chrome under a
        // `chat · rendering` crumb (markdown_stream.png); the shared 3-zone status bar (with the real
        // model id) is drawn by the chrome, so the conversation draws no own footer.
        let body = crate::view::render_chrome(frame, area, app, "chat · rendering", &theme);
        (body, false)
    } else if app.find().is_some() {
        // In-chat find: wrap the conversation in the shared chrome under a `chat · find` crumb
        // (markdown_search.png); the find bar replaces the composer (pinned just above the shared
        // status bar), so the conversation draws no own footer.
        let body = crate::view::render_chrome(frame, area, app, "chat · find", &theme);
        (body, false)
    } else {
        (area, true)
    };

    let find_mode = app.find().is_some();
    let prompt_rows = prompt_rows(app, main_area.height);
    // While a turn is in flight a Working band sits directly above the composer, separated from it
    // by one blank gap row (so the band area is two rows: the band itself, then the gap). It is only
    // reserved when there is room to keep at least one transcript row. Not shown while find is open
    // (the composer, and everything anchored to it, is replaced by the find bar).
    let band_rows: u16 = if !find_mode && app.is_busy() && main_area.height > prompt_rows + 2 + 1 + 2
    {
        2
    } else {
        0
    };
    // In plan mode a persistent amber footer banner sits directly above the composer, separated from
    // it by one blank gap row (band-style: banner row + gap row), when there is room to keep at least
    // one transcript row.
    let plan_banner_rows: u16 = if !find_mode
        && plan_mode
        && main_area.height > prompt_rows + 2 + band_rows + 1 + 2
    {
        2
    } else {
        0
    };
    // In the split (or plan chrome) the status bar is the full-width bar drawn by the shared chrome,
    // so the conversation column does not reserve a footer row of its own.
    let footer_rows: u16 = if own_footer { 1 } else { 0 };
    // The composer box is fully replaced by the find bar while find mode is active (markdown_search.png
    // shows no composer at all — the find bar is the only thing pinned above the shared status bar).
    let prompt_box_rows: u16 = if find_mode { 0 } else { prompt_rows + 2 };
    // The find bar is a single row, pinned directly above the shared status bar (the last row of the
    // chrome body — `render_chrome` already excludes the status-bar row from `main_area`).
    let find_bar_rows: u16 = if find_mode && main_area.height > 0 { 1 } else { 0 };
    let chunks = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(band_rows),
        Constraint::Length(plan_banner_rows),
        Constraint::Length(prompt_box_rows),
        Constraint::Length(footer_rows),
        Constraint::Length(find_bar_rows),
    ])
    .split(main_area);
    let transcript_area = chunks[0];
    let band_area = chunks[1];
    let plan_banner_area = chunks[2];
    let prompt_area = chunks[3];
    let footer_area = chunks[4];
    let find_bar_area = chunks[5];

    draw_transcript(frame, app, transcript_area, &theme);
    if band_rows > 0 {
        draw_working_band(frame, app, band_area, &theme);
    }
    if plan_banner_rows > 0 {
        draw_plan_banner(frame, plan_banner_area, &theme);
    }
    if !find_mode {
        draw_prompt(frame, app, prompt_area, &theme);
    }
    if own_footer {
        draw_footer(frame, app, footer_area, &theme);
    }
    // The slash menu floats above the prompt, so it is drawn last to overlay the transcript.
    draw_slash_menu(frame, app, prompt_area, &theme);
    // The `@`-file picker also floats above the prompt (mutually exclusive with the slash menu).
    draw_mention_menu(frame, app, prompt_area, &theme);
    // The find-in-chat search bar occupies its own pinned row while find mode is active.
    draw_find_bar(frame, app, find_bar_area, &theme);
    // A transient acknowledgement (e.g. a copy confirmation) shows just above the prompt.
    draw_ack(frame, app, prompt_area, &theme);
    // The sidebar box occupies the reserved right-hand column beside the conversation.
    if let Some(split) = split {
        draw_sidebar(frame, app, split.sidebar, &theme);
    }
    // The dedicated `/diff` viewer is a full-screen Wave-2 view (shared chrome + `Changes`
    // accordion) that fully repaints the frame, so it overlays the conversation beneath it.
    draw_diff_viewer(frame, app, area, &theme);
    // A modal picker/dialog (or the command palette) floats over a dimmed session: fade the
    // backdrop first, then draw the overlay.
    if app.dialog_open()
        || app.palette_open()
        || app.rename_open()
        || app.compact_open()
        || app.init_open()
    {
        dim_backdrop(frame, area, &theme);
    }
    // A modal dialog (picker/status view) overlays everything else when open.
    draw_dialog(frame, app, area, &theme);
    // The `/rename` dialog (centered input over the dimmed conversation) overlays when open.
    draw_rename(frame, app, area, &theme);
    // The `/compact` and `/init` dialogs (centered over the dimmed conversation) overlay when open.
    draw_compact(frame, app, area, &theme);
    draw_init(frame, app, area, &theme);
    // The command palette overlays everything when open (mutually exclusive with a dialog).
    draw_palette(frame, app, area, &theme);
    // The which-key overlay lists the active leader bindings while the leader chord is armed.
    draw_which_key(frame, app, area, &theme);
    // An interactive permission prompt is modal: it floats over a dimmed backdrop (the faded prior
    // conversation) and captures every key while the turn worker is blocked awaiting the decision.
    if app.permission_open() {
        dim_backdrop(frame, area, &theme);
    }
    draw_permission(frame, app, area, &theme);
    // Transient toast notifications float above everything else (top-right) so they remain visible
    // over any open dialog/overlay, and auto-dismiss without disturbing the layout beneath them.
    draw_toasts(frame, app, area, &theme);
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

    if let Some(find) = app.find() {
        if !find.query.is_empty() {
            // Every match carries the HIT background; the active match carries the distinct
            // HITCUR background + BRIGHT foreground so next/prev navigation is visible as the
            // emphasis moves between occurrences (VAL-MD-021/022).
            highlight_find(
                &mut lines,
                &find.query,
                find.active,
                theme.search_hit,
                theme.search_hit_current,
                theme.text_bright,
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
    // The streaming caret trails the in-progress assistant reply: only while a turn is live and the
    // last transcript entry is the growing answer (a running tool card supersedes it). Its blink is
    // decided by the animation substrate (`cursor_blink_visible` is solid under reduced motion and
    // toggles on the blink cadence during a live turn with animations on), so the caret is drawn
    // only when that predicate is true and disappears once the turn completes.
    let last_idx = app.transcript.len().saturating_sub(1);
    let stream_caret = app.is_busy()
        && matches!(app.transcript.last(), Some(Entry::Assistant(_)))
        && app.cursor_blink_visible();
    for (idx, entry) in app.transcript.iter().enumerate() {
        entry_starts.push(lines.len());
        match entry {
            Entry::User(text) => push_user(&mut lines, text, width, &theme),
            Entry::Assistant(turn) => push_assistant(
                &mut lines,
                turn,
                width,
                app.theme(),
                &theme,
                stream_caret && idx == last_idx,
                app.is_busy() && idx == last_idx,
            ),
            Entry::Diff(diff) => push_diff(&mut lines, diff, &theme),
            Entry::Tool(card) => {
                push_tool(
                    &mut lines,
                    card,
                    width,
                    app.tool_card_expanded(card),
                    &theme,
                );
            }
            Entry::Notice(message) => push_notice(&mut lines, message, width, &theme),
            Entry::Shell(outcome) => push_shell(&mut lines, outcome, width, &theme),
            Entry::Blocked { command, reason } => {
                push_blocked(&mut lines, command, reason, width, &theme);
            }
            Entry::Help => push_help(&mut lines, app, &theme),
            Entry::Error(message) => push_error(&mut lines, message, width, &theme),
        }
    }
    // The busy/working indicator is no longer appended to the scrollback: it renders as a dedicated
    // Working band fixed above the composer (see `draw`/`working_band_line`).
    //
    // Attachment cards: while files are staged in the composer, their cards render as the bottom
    // block of the scrollback (a bordered card per file + an aggregate summary), so the attach state
    // shows the files that will be sent — matching `perm_attach.png`. They are not transcript
    // entries, so they are appended after the entry loop without an `entry_starts` slot.
    if !app.composer_attachments().is_empty() {
        push_attachment_cards(&mut lines, app.composer_attachments(), width, &theme);
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

/// Re-style the lines holding find matches: every match gets a background band (`base_bg`, HIT) and
/// the active match (the `active`-th in order) gets the distinct `active_bg` (HITCUR) plus
/// `active_fg` (BRIGHT) foreground, so next/prev navigation visibly moves the emphasis between
/// occurrences.
fn highlight_find(
    lines: &mut [Line<'static>],
    query: &str,
    active: usize,
    base_bg: Color,
    active_bg: Color,
    active_fg: Color,
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
        lines[line_idx] =
            apply_highlights_to_line(&lines[line_idx], &ranges, base_bg, active_bg, active_fg);
    }
}

/// Rebuild one line, splitting its spans at the given char ranges and applying a highlight
/// background (the active range gets `active_bg` + `active_fg` + bold; others get `base_bg`), while
/// preserving each span's original foreground/modifiers elsewhere.
fn apply_highlights_to_line(
    line: &Line<'static>,
    ranges: &[(usize, usize, bool)],
    base_bg: Color,
    active_bg: Color,
    active_fg: Color,
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
                Some(true) => style.bg(active_bg).fg(active_fg).add_modifier(Modifier::BOLD),
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
    caret: bool,
    streaming: bool,
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

    // The assistant reply leads with a `✦` author mark (AMBER) at the content-left column. A plain
    // prose reply renders inline with the first line (promoted to BRIGHT to match the reply tone),
    // with wrapped lines hang-indented under the text column. A *structured* markdown answer
    // (headings, lists, tables, blockquotes, fenced code) instead leads with a `✦ Cortex` author
    // line and renders the markdown body beneath it in the theme's TXT prose tone, so headings sit
    // BRIGHT above a GHOST rule, bullets carry `•` accents, inline code chips keep their explicit
    // background, and bold spans stay distinctly brighter than body TXT. Re-rendered every frame
    // from the accumulated answer, so it updates incrementally as the turn streams.
    let mark = Style::default().fg(theme.secondary);
    let text_indent = CONTENT_MARGIN + 2;
    let structured = is_structured_markdown(&turn.answer);
    let base_prose = if structured {
        // Structured markdown keeps the theme's TXT body tone so headings/bold read as brighter.
        frame::markdown::MarkdownTheme::from_name(theme_name)
    } else {
        // Plain prose is promoted to BRIGHT to match the conversational reply tone (VAL-CONV-018).
        frame::markdown::MarkdownTheme::from_name(theme_name)
            .with_text(Style::default().fg(theme.text_bright))
    };
    let body_lines: Vec<Line<'static>> = if turn.answer.trim().is_empty() {
        vec![Line::from("")]
    } else {
        let md_width = width.saturating_sub(text_indent).clamp(1, u16::MAX as usize) as u16;
        let renderer = frame::markdown::MarkdownRenderer::with_theme(base_prose).with_width(md_width);
        renderer.render(&turn.answer)
    };

    if structured {
        // Author line: `✦ Cortex` (✦ AMBER, `Cortex` BRIGHT), with the markdown block beneath it
        // (all body rows hang-indented under the text column). While the turn is actively
        // streaming, a DIM status suffix (`synthesizing answer…`) trails the name so the
        // in-progress reply reads `✦ Cortex synthesizing answer…` (VAL-MD-002); it drops away once
        // the turn completes, leaving the bare `✦ Cortex` label.
        let mut author_spans = vec![
            Span::raw(" ".repeat(CONTENT_MARGIN)),
            Span::styled(format!("{} ", crate::glyphs::BRAND_STAR), mark),
            Span::styled(
                "Cortex".to_owned(),
                Style::default().fg(theme.text_bright),
            ),
        ];
        if streaming {
            author_spans.push(Span::styled(
                " synthesizing answer…".to_owned(),
                Style::default().fg(theme.text_dim),
            ));
        }
        lines.push(Line::from(author_spans));
        for line in body_lines {
            let mut spans: Vec<Span<'static>> = Vec::with_capacity(line.spans.len() + 1);
            spans.push(Span::raw(" ".repeat(text_indent)));
            spans.extend(line.spans);
            lines.push(Line::from(spans));
        }
    } else {
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
    }
    // While the turn streams, a `▏` caret (ORANGE) trails the last line of the growing answer. The
    // caller passes `caret` only for the in-progress reply and only when the blink state is visible,
    // so it toggles during a live turn and is absent once the turn completes.
    if caret && let Some(last) = lines.last_mut() {
        last.spans.push(Span::styled(
            crate::glyphs::CURSOR,
            Style::default().fg(theme.primary),
        ));
    }
    lines.push(Line::from(""));
}

/// Whether the conversation is currently showing a rendered/streaming markdown answer, i.e. the
/// most recent transcript entry is an assistant reply whose body is structured markdown AND the
/// turn is still actively streaming. Drives the `chat · rendering` breadcrumb chrome (VAL-MD-001)
/// only during the live streaming state, without disturbing static/loaded conversations.
fn markdown_rendering_active(app: &App) -> bool {
    app.is_busy()
        && matches!(
            app.transcript.last(),
            Some(Entry::Assistant(turn)) if is_structured_markdown(&turn.answer)
        )
}

/// Whether an assistant answer contains structured markdown (a heading, list, table, blockquote, or
/// fenced code block) versus plain prose. Structured answers render under a `✦ Cortex` author line
/// with the markdown body in the theme prose tone; plain prose keeps the inline `✦ <reply>` form.
fn is_structured_markdown(answer: &str) -> bool {
    answer.lines().any(|raw| {
        let line = raw.trim_start();
        // ATX heading (`# `..`###### `).
        (line.starts_with('#')
            && line.trim_start_matches('#').starts_with(' '))
            // Fenced code block.
            || line.starts_with("```")
            // Blockquote.
            || line.starts_with("> ")
            // GFM table row / separator.
            || line.starts_with('|')
            // Unordered list item (`- `, `* `, `+ `).
            || line.starts_with("- ")
            || line.starts_with("* ")
            || line.starts_with("+ ")
            // Ordered list item (`N. ` / `N) `).
            || ordered_list_marker(line)
    })
}

/// Whether `line` begins with an ordered-list marker (`12. ` or `12) `).
fn ordered_list_marker(line: &str) -> bool {
    let digits: String = line.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return false;
    }
    let rest = &line[digits.len()..];
    rest.starts_with(". ") || rest.starts_with(") ")
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

/// The plural noun for a tool card's `… (N more <noun>)` expanded-output overflow row. The unit is
/// per-tool so the overflow reads naturally against the payload shape: `glob` lists matched paths so
/// it reports `matches` (VAL-TOOL-020) and `websearch` lists results so it reports `results`
/// (VAL-TOOL-026); everything else (numbered `read` output, `grep` `path:line:` hits, generic
/// output) counts `lines` (VAL-TOOL-011/015/019).
fn overflow_noun(name: &str) -> &'static str {
    match name {
        "glob" | "list" => "matches",
        "websearch" => "results",
        _ => "lines",
    }
}

/// Append an agent tool call as a fully-closed rounded-rectangle card. The label rides the top
/// border as `{glyph} {name} · {state}` (VAL-TOOL-001/002), tinted uniformly by the status accent:
/// running `⚙` ORANGE, done `✓` OKG-green, error `✗` ERR-red (VAL-TOOL-003/004/005/006). The first
/// interior row is the per-tool summary (VAL-TOOL-008); a collapsed card follows it with a FAINT
/// `Ctrl+O to expand` hint and no payload (VAL-TOOL-009), while an expanded card replaces the hint
/// with a DIM `output` header and its payload rows (VAL-TOOL-010), diff-carrying cards revealing a
/// red/green line-numbered diff whose +/- coloring is confined to the payload (VAL-TOOL-031).
fn push_tool(
    lines: &mut Vec<Line<'static>>,
    card: &ToolCard,
    width: usize,
    expanded: bool,
    theme: &style::ThemeColors,
) {
    let bx = CONTENT_MARGIN;
    let right = width.saturating_sub(CONTENT_MARGIN);
    if right <= bx + 4 {
        lines.push(Line::from(""));
        return;
    }
    let bw = right - bx;
    let inner_w = bw.saturating_sub(4);
    let base = Style::default().bg(theme.background);

    let (accent, glyph, state_word) = match card.status {
        ToolStatus::Running => (theme.primary, glyphs::MARK_RUNNING, "running"),
        ToolStatus::Completed => (theme.success, glyphs::MARK_DONE, "done"),
        ToolStatus::Failed => (theme.error, glyphs::MARK_ERROR, "error"),
    };
    let border_style = Style::default().fg(accent).bg(theme.background);
    let label_style = Style::default().fg(accent).bg(theme.background);

    let title = format!("{} {} {} {}", glyph, card.name, glyphs::SEPARATOR, state_word);
    let title_text = format!(" {} ", clip_text(&title, inner_w.saturating_sub(4)));

    let mut top: Vec<(char, Style)> = vec![(' ', base); width];
    top[bx] = (glyphs::BOX_TOP_LEFT.chars().next().unwrap(), border_style);
    top[bx + bw - 1] = (glyphs::BOX_TOP_RIGHT.chars().next().unwrap(), border_style);
    for cell in top.iter_mut().take(bx + bw - 1).skip(bx + 1) {
        *cell = (glyphs::BOX_HORIZONTAL.chars().next().unwrap(), border_style);
    }
    place_segments(&mut top, bx + 2, &[(title_text, label_style)]);
    lines.push(cells_to_line(&top));

    let origin = bx + 2;
    let text_w = inner_w.saturating_sub(2);

    let summary_text = clip_text(&card.summary, text_w);
    let mut summary_row: Vec<(char, Style)> = vec![(' ', base); width];
    summary_row[bx] = (glyphs::BOX_VERTICAL.chars().next().unwrap(), border_style);
    summary_row[bx + bw - 1] = (glyphs::BOX_VERTICAL.chars().next().unwrap(), border_style);
    place_segments(
        &mut summary_row,
        origin,
        &[(summary_text, Style::default().fg(theme.text).bg(theme.background))],
    );
    lines.push(cells_to_line(&summary_row));

    let interior_row = |seg: &[(String, Style)]| -> Line<'static> {
        let mut row: Vec<(char, Style)> = vec![(' ', base); width];
        row[bx] = (glyphs::BOX_VERTICAL.chars().next().unwrap(), border_style);
        row[bx + bw - 1] = (glyphs::BOX_VERTICAL.chars().next().unwrap(), border_style);
        place_segments(&mut row, origin, seg);
        cells_to_line(&row)
    };

    if card.name == "question" && card.status == ToolStatus::Running {
        // VAL-TOOL-030: a running `question` card is awaiting the user's answer — its summary is the
        // question text and its body is a single DIM awaiting-answer line (no output section or
        // expand hint, since there is no result yet).
        let awaiting_style = Style::default().fg(theme.text_dim).bg(theme.background);
        lines.push(interior_row(&[(
            format!("waiting for your answer{}", glyphs::ELLIPSIS),
            awaiting_style,
        )]));
    } else if expanded {
        let header_style = Style::default().fg(theme.text_dim).bg(theme.background);
        lines.push(interior_row(&[("output".to_owned(), header_style)]));

        if let Some(diff) = card.diff.as_deref() {
            push_card_diff_rows(lines, diff, bx, bw, border_style, base, theme);
        } else if card.name == "todo" {
            // VAL-TOOL-025: todo items render with three-state markers: ✓ done (OKG green),
            // ▶ in-progress (ORANGE), ○ pending (FAINT).
            push_tool_todo_items(card, &interior_row, text_w, theme, lines);
        } else {
            // Generic output: per-line with tool-specific coloring.
            push_tool_generic_output(card, &interior_row, text_w, header_style, theme, lines);
        }
    } else if card.status == ToolStatus::Failed {
        // VAL-TOOL-029: a failed card (e.g. an `lsp` diagnostics query) previews the error inline
        // while collapsed — an ERR-red `error` header, the first diagnostic line in ERR red, and a
        // FAINT `Ctrl+O for detail` hint pointing at the full expanded output.
        push_tool_error_preview(card, &interior_row, text_w, theme, lines);
    } else if card.is_background_running() {
        // VAL-TOOL-023: a live background bash card streams its in-progress output rows (the tail
        // of the job log) in the collapsed body, above the `esc to background` footer hint, while
        // it keeps the ORANGE running chrome until the job's pid exits.
        push_tool_background_stream(card, &interior_row, text_w, theme, lines);
    } else {
        let hint_style = Style::default().fg(theme.text_muted).bg(theme.background);
        let hint_text = if card.status == ToolStatus::Running
            && card.name == "bash"
            && card.summary.contains("(background)")
        {
            // VAL-TOOL-023: background bash footer hint is "Ctrl+O to expand · esc to background"
            // while the job is still running; once it completes the card reverts to the plain hint.
            "Ctrl+O to expand · esc to background".to_owned()
        } else {
            "Ctrl+O to expand".to_owned()
        };
        lines.push(interior_row(&[(hint_text, hint_style)]));
    }

    let mut bottom: Vec<(char, Style)> = vec![(' ', base); width];
    bottom[bx] = (glyphs::BOX_BOTTOM_LEFT.chars().next().unwrap(), border_style);
    bottom[bx + bw - 1] = (glyphs::BOX_BOTTOM_RIGHT.chars().next().unwrap(), border_style);
    for cell in bottom.iter_mut().take(bx + bw - 1).skip(bx + 1) {
        *cell = (glyphs::BOX_HORIZONTAL.chars().next().unwrap(), border_style);
    }
    lines.push(cells_to_line(&bottom));
    lines.push(Line::from(""));
}

/// Render the generic tool output payload: per-line with tool-specific coloring (bash success lines
/// in OKG green per VAL-TOOL-022, failed tools in ERR red), truncated with an overflow row.
fn push_tool_generic_output<F>(
    card: &ToolCard,
    interior_row: &F,
    text_w: usize,
    header_style: Style,
    theme: &style::ThemeColors,
    lines: &mut Vec<Line<'static>>,
) where
    F: Fn(&[(String, Style)]) -> Line<'static>,
{
    let out_lines: Vec<&str> = card.output.lines().collect();
    let shown = out_lines.len().min(MAX_TOOL_DETAIL_LINES);
    for line in &out_lines[..shown] {
        let style = tool_output_line_style(&card.name, line, card.status, theme);
        lines.push(interior_row(&[(clip_text(line, text_w), style)]));
    }
    if out_lines.len() > shown {
        let overflow = format!(
            "{} ({} more {})",
            glyphs::ELLIPSIS,
            out_lines.len() - shown,
            overflow_noun(&card.name),
        );
        lines.push(interior_row(&[(overflow, header_style)]));
    }
}

/// Render the collapsed error preview of a failed tool card (VAL-TOOL-029): an ERR-red `error`
/// header followed by the first non-empty diagnostic line (also ERR red), then a FAINT
/// `Ctrl+O for detail` hint. This gives a failure (e.g. an `lsp` diagnostics error) an at-a-glance
/// diagnostic without expanding, while the full output stays behind Ctrl+O.
fn push_tool_error_preview<F>(
    card: &ToolCard,
    interior_row: &F,
    text_w: usize,
    theme: &style::ThemeColors,
    lines: &mut Vec<Line<'static>>,
) where
    F: Fn(&[(String, Style)]) -> Line<'static>,
{
    let err_style = Style::default().fg(theme.error).bg(theme.background);
    lines.push(interior_row(&[("error".to_owned(), err_style)]));
    if let Some(first) = card.output.lines().find(|line| !line.trim().is_empty()) {
        lines.push(interior_row(&[(clip_text(first, text_w), err_style)]));
    }
    let hint_style = Style::default().fg(theme.text_muted).bg(theme.background);
    lines.push(interior_row(&[("Ctrl+O for detail".to_owned(), hint_style)]));
}

/// The number of trailing in-progress output rows shown in a live background bash card's collapsed
/// body, keeping the streaming card compact while it tails the job log (VAL-TOOL-023).
const BG_STREAM_ROWS: usize = 3;

/// Render a live background bash card's collapsed body (VAL-TOOL-023): the last few non-empty rows
/// of the tailed job log in DIM text, followed by the `Ctrl+O to expand · esc to background` footer
/// hint. Every cell carries the explicit canvas background via `interior_row`.
fn push_tool_background_stream<F>(
    card: &ToolCard,
    interior_row: &F,
    text_w: usize,
    theme: &style::ThemeColors,
    lines: &mut Vec<Line<'static>>,
) where
    F: Fn(&[(String, Style)]) -> Line<'static>,
{
    let body_style = Style::default().fg(theme.text_dim).bg(theme.background);
    let out_lines: Vec<&str> = card
        .output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let start = out_lines.len().saturating_sub(BG_STREAM_ROWS);
    for line in &out_lines[start..] {
        lines.push(interior_row(&[(clip_text(line, text_w), body_style)]));
    }
    let hint_style = Style::default().fg(theme.text_muted).bg(theme.background);
    lines.push(interior_row(&[(
        "Ctrl+O to expand · esc to background".to_owned(),
        hint_style,
    )]));
}

/// The style for one output line inside a tool card, with tool-specific semantic coloring:
/// - bash (completed): a "test result: ok" success line is OKG green (VAL-TOOL-022), ordinary
///   lines are TXT.
/// - failed tools: all lines are ERR red.
/// - default: TXT.
fn tool_output_line_style(
    tool_name: &str,
    line: &str,
    status: ToolStatus,
    theme: &style::ThemeColors,
) -> Style {
    let bg = theme.background;
    if status == ToolStatus::Failed {
        return Style::default().fg(theme.error).bg(bg);
    }
    if tool_name == "bash" && is_shell_success_line(line.trim_start()) {
        return Style::default().fg(theme.success).bg(bg);
    }
    Style::default().fg(theme.text).bg(bg)
}

/// Render the todo tool's expanded output as three-state item rows (VAL-TOOL-025): each item line
/// from the todo tool output (`N. [status][priority] content`) is parsed and rendered with a
/// state-colored marker: ✓ done (OKG green), ▶ in-progress (ORANGE), ○ pending (FAINT).
fn push_tool_todo_items<F>(
    card: &ToolCard,
    interior_row: &F,
    text_w: usize,
    theme: &style::ThemeColors,
    lines: &mut Vec<Line<'static>>,
) where
    F: Fn(&[(String, Style)]) -> Line<'static>,
{
    if card.output.trim() == "(no todos)" {
        lines.push(interior_row(&[(
            "(no todos)".to_owned(),
            Style::default().fg(theme.text_dim).bg(theme.background),
        )]));
        return;
    }
    for line in card.output.lines() {
        if line.trim().is_empty() {
            continue;
        }
        // Parse the todo tool output format: "N. [status][priority] content"
        let (marker, marker_color, text_color, content) = parse_todo_item_line(line, theme);
        let clipped = clip_text(&content, text_w.saturating_sub(2).max(1));
        lines.push(interior_row(&[
            (
                format!("{marker} "),
                Style::default().fg(marker_color).bg(theme.background),
            ),
            (
                clipped,
                Style::default().fg(text_color).bg(theme.background),
            ),
        ]));
    }
}

/// Parse a todo tool output line ("N. [status][priority] content") and return the appropriate
/// three-state marker, colors, and the content text.
fn parse_todo_item_line(
    line: &str,
    theme: &style::ThemeColors,
) -> (&'static str, Color, Color, String) {
    // The todo tool output format: "1. [pending][medium] do the thing"
    let after_dot = line.split_once(". ").map(|(_, rest)| rest).unwrap_or(line);
    
    let (marker, marker_color, text_color) = if after_dot.contains("[completed]") {
        (glyphs::MARK_DONE, theme.success, theme.text_dim)
    } else if after_dot.contains("[in_progress]") {
        (
            glyphs::DISCLOSURE_COLLAPSED,
            theme.primary,
            theme.text_bright,
        )
    } else if after_dot.contains("[cancelled]") {
        (glyphs::MARK_ERROR, theme.text_muted, theme.text_muted)
    } else {
        // pending or unknown
        (glyphs::DOT_HOLLOW, theme.ghost, theme.text)
    };
    
    // Extract content after the status and priority brackets
    let content = after_dot
        .rsplit(']')
        .next()
        .unwrap_or(after_dot)
        .trim()
        .to_owned();
    
    (marker, marker_color, text_color, content)
}

/// Render an expanded diff card's payload rows *inside* the tool card's own box: one line-numbered
/// row per diff line, carried on the card's accent border. Each row is a cell buffer so the per-line
/// background (`ADD_BG`/`DEL_BG` for additions/deletions, none for context) fills the box interior;
/// the right-justified line number sits in `LNUM`, the `+`/`-`/space sign at a fixed sign column,
/// and the code text at a fixed code column. This keeps diff +/- coloring confined to the expanded
/// payload (VAL-TOOL-031) while the surrounding card chrome stays mono-orange + grey.
fn push_card_diff_rows(
    lines: &mut Vec<Line<'static>>,
    diff: &FileDiff,
    bx: usize,
    bw: usize,
    border_style: Style,
    base: Style,
    theme: &style::ThemeColors,
) {
    let width = base_width(bx, bw);
    let origin = bx + 2;
    let mut shown = 0usize;
    let mut truncated = false;
    'outer: for hunk in &diff.hunks {
        let start = if hunk.new_len > 0 {
            hunk.new_start
        } else {
            hunk.old_start
        };
        for (i, dline) in hunk.lines.iter().enumerate() {
            if shown >= MAX_DIFF_BODY_ROWS {
                truncated = true;
                break 'outer;
            }
            let (sign, fg, row_bg) = match dline.kind {
                crate::diff::DiffLineKind::Added => ('+', style::ADD_FG, Some(style::ADD_BG)),
                crate::diff::DiffLineKind::Removed => ('-', style::DEL_FG, Some(style::DEL_BG)),
                crate::diff::DiffLineKind::Context | crate::diff::DiffLineKind::Header => {
                    (' ', theme.text_dim, None)
                }
            };
            let row_bg_color = row_bg.unwrap_or(theme.background);
            let row_style = Style::default().fg(fg).bg(row_bg_color);
            let mut cells: Vec<(char, Style)> = vec![(' ', base); width];
            for cell in cells.iter_mut().take(bx + bw - 1).skip(bx + 1) {
                *cell = (' ', row_style);
            }
            cells[bx] = (glyphs::BOX_VERTICAL.chars().next().unwrap(), border_style);
            cells[bx + bw - 1] = (glyphs::BOX_VERTICAL.chars().next().unwrap(), border_style);
            let lnum = format!("{:>3}", start + i);
            place_segments(
                &mut cells,
                origin,
                &[(lnum, Style::default().fg(style::LNUM).bg(row_bg_color))],
            );
            place_segments(&mut cells, origin + 6, &[(sign.to_string(), row_style)]);
            let code_col = origin + 8;
            let code_room = (bx + bw - 1).saturating_sub(code_col + 1);
            let code = clip_text(&dline.content, code_room);
            place_segments(&mut cells, code_col, &[(code, row_style)]);
            lines.push(cells_to_line(&cells));
            shown += 1;
        }
    }
    if truncated {
        let mut row: Vec<(char, Style)> = vec![(' ', base); width];
        row[bx] = (glyphs::BOX_VERTICAL.chars().next().unwrap(), border_style);
        row[bx + bw - 1] = (glyphs::BOX_VERTICAL.chars().next().unwrap(), border_style);
        place_segments(
            &mut row,
            origin,
            &[(
                format!("{} more diff lines", glyphs::ELLIPSIS),
                Style::default().fg(theme.text_muted).bg(theme.background),
            )],
        );
        lines.push(cells_to_line(&row));
    }
}

/// The full row width implied by a box at `bx` spanning `bw` columns (`bx + bw`), used to size the
/// cell buffers for interior rows.
fn base_width(bx: usize, bw: usize) -> usize {
    bx + bw
}

/// Width (columns) of one attachment card (`perm_attach.png`: three 28-col cards tile the 88-col
/// content width with a 2-col gap between them).
const ATTACHMENT_CARD_WIDTH: usize = 28;
/// Blank columns between adjacent attachment cards on a row.
const ATTACHMENT_CARD_GAP: usize = 2;

/// Append the attachment cards + aggregate summary for the staged composer attachments (the
/// `perm_attach.png` attach state). Each card is a rounded bordered box holding an inverse AMBER
/// type-badge pill, the filename in BRIGHT, and a DIM meta line (dimensions/pages/vector · size);
/// cards tile side by side across the content width and wrap to further rows when there are more
/// than fit. A `N attachments · <total size>` summary in DIM closes the block.
fn push_attachment_cards(
    lines: &mut Vec<Line<'static>>,
    attachments: &[InputAttachment],
    width: usize,
    theme: &style::ThemeColors,
) {
    if attachments.is_empty() {
        return;
    }
    let base = Style::default().bg(theme.background);
    let content_width = width.saturating_sub(CONTENT_MARGIN * 2).max(1);
    // Shrink the card to the content width on a narrow terminal so a single card always fits.
    let card_w = ATTACHMENT_CARD_WIDTH.min(content_width).max(5);
    let per_row = ((content_width + ATTACHMENT_CARD_GAP) / (card_w + ATTACHMENT_CARD_GAP)).max(1);

    lines.push(Line::from(""));
    for chunk in attachments.chunks(per_row) {
        // Build all 5 rows of a card row in a shared cell buffer so the cards tile side by side
        // (top border, badge+name, blank interior, meta, bottom border).
        let mut rows: [Vec<(char, Style)>; 5] = std::array::from_fn(|_| vec![(' ', base); width]);
        for (ci, attachment) in chunk.iter().enumerate() {
            let cx = CONTENT_MARGIN + ci * (card_w + ATTACHMENT_CARD_GAP);
            render_attachment_card(&mut rows, cx, card_w, attachment, theme, base);
        }
        for row in rows {
            lines.push(cells_to_line(&row));
        }
    }

    let total: u64 = attachments.iter().map(InputAttachment::byte_size).sum();
    let count = attachments.len();
    let noun = if count == 1 {
        "attachment"
    } else {
        "attachments"
    };
    let summary = format!(
        "{count} {noun} {} {}",
        crate::glyphs::SEPARATOR,
        crate::attachments::human_size(total)
    );
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::raw(" ".repeat(CONTENT_MARGIN)),
        Span::styled(summary, Style::default().fg(theme.text_dim).bg(theme.background)),
    ]));
    lines.push(Line::from(""));
}

/// Paint one attachment card into `rows` (a 5-row shared cell buffer) with its top-left corner at
/// column `cx`. The card interior carries the canvas background on every cell (no unset bg), the
/// badge is an inverse pill (AMBER text on the SELBG surface), the name is BRIGHT, and the meta is
/// DIM — matching `perm_attach.png`.
fn render_attachment_card(
    rows: &mut [Vec<(char, Style)>; 5],
    cx: usize,
    card_w: usize,
    attachment: &InputAttachment,
    theme: &style::ThemeColors,
    base: Style,
) {
    let width = rows[0].len();
    let right = cx + card_w - 1;
    if right >= width || card_w < 5 {
        return;
    }
    let border = Style::default().fg(theme.border).bg(theme.background);
    let hz = crate::glyphs::BOX_HORIZONTAL.chars().next().unwrap();
    let vt = crate::glyphs::BOX_VERTICAL.chars().next().unwrap();

    // Top + bottom rounded borders.
    rows[0][cx] = (crate::glyphs::BOX_TOP_LEFT.chars().next().unwrap(), border);
    rows[0][right] = (crate::glyphs::BOX_TOP_RIGHT.chars().next().unwrap(), border);
    rows[4][cx] = (crate::glyphs::BOX_BOTTOM_LEFT.chars().next().unwrap(), border);
    rows[4][right] = (crate::glyphs::BOX_BOTTOM_RIGHT.chars().next().unwrap(), border);
    for cell in rows[0].iter_mut().take(right).skip(cx + 1) {
        *cell = (hz, border);
    }
    for cell in rows[4].iter_mut().take(right).skip(cx + 1) {
        *cell = (hz, border);
    }
    // Vertical edges + explicit-bg interior on the three content rows.
    for row in rows.iter_mut().take(4).skip(1) {
        row[cx] = (vt, border);
        row[right] = (vt, border);
        for cell in row.iter_mut().take(right).skip(cx + 1) {
            *cell = (' ', base);
        }
    }

    let inner_left = cx + 2;
    let inner_right = right - 1;
    let inner_w = inner_right.saturating_sub(inner_left) + 1;

    // Row 1: an inverse AMBER badge pill, then the filename in BRIGHT.
    let badge = format!(" {} ", attachment.badge_label());
    let badge_style = Style::default()
        .fg(theme.secondary)
        .bg(theme.surface[3])
        .add_modifier(Modifier::BOLD);
    place_segments(&mut rows[1], inner_left, &[(badge.clone(), badge_style)]);
    let name_x = inner_left + badge.chars().count() + 1;
    let name_room = inner_right.saturating_sub(name_x) + 1;
    let name = clip_text(&attachment.file_name(), name_room);
    place_segments(
        &mut rows[1],
        name_x,
        &[(name, Style::default().fg(theme.text_bright).bg(theme.background))],
    );

    // Row 3: the meta line (descriptor · size) in DIM.
    let meta = clip_text(&attachment.meta_line(), inner_w);
    place_segments(
        &mut rows[3],
        inner_left,
        &[(meta, Style::default().fg(theme.text_dim).bg(theme.background))],
    );
}

/// Truncate `text` to at most `max` display columns, appending an ellipsis when shortened.
fn clip_text(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    if max == 0 {
        return String::new();
    }
    let kept: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{kept}{}", crate::glyphs::ELLIPSIS)
}

/// Append a system notice (e.g. a dispatched slash command or an unknown-command message) as a
/// distinct, muted line set apart from chat content.
fn push_notice(
    lines: &mut Vec<Line<'static>>,
    message: &str,
    width: usize,
    theme: &style::ThemeColors,
) {
    let style = Style::default().fg(theme.text_dim);
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

/// Whether an output line is a command/test success summary — the brand's success signal, rendered
/// in green ONLY when the command succeeded (so green stays success-only, VAL-PERM-028/052). The
/// canonical form is the Rust/cargo test summary `test result: ok …`.
fn is_shell_success_line(trimmed: &str) -> bool {
    trimmed.starts_with("test result: ok")
}

/// Whether an output line is an error indication (a failed command's stderr/exit markers), rendered
/// in ERR red so a non-zero run reads as an error rather than an ordinary log line (VAL-PERM-052).
fn is_shell_error_line(trimmed: &str) -> bool {
    trimmed.starts_with("[stderr]")
        || trimmed.starts_with("[exit code:")
        || trimmed.starts_with("test result: FAILED")
        || trimmed.starts_with("failed to run command")
}

/// The style for one line inside the shell `output` pane: OKG green for a success summary line (only
/// when the command succeeded), ERR red for a failed command's error/exit lines, and DIM grey for
/// ordinary log lines. Every cell carries an explicit background.
fn shell_line_style(line: &str, success: bool, theme: &style::ThemeColors) -> Style {
    let bg = theme.background;
    let trimmed = line.trim_start();
    if success && is_shell_success_line(trimmed) {
        Style::default().fg(theme.success).bg(bg)
    } else if !success && is_shell_error_line(trimmed) {
        Style::default().fg(theme.error).bg(bg)
    } else {
        Style::default().fg(theme.text_dim).bg(bg)
    }
}

/// Append a `!` shell-mode command and its output. The command line renders as `! <command>` (`!`
/// ORANGE, command BRIGHT); the captured output is shown inside a rounded ORANGE_D pane whose top
/// border embeds the AMBER title `output`. Ordinary log lines are DIM, a `test result: ok …` success
/// summary is OKG green (success-only), and a failed command's error/exit lines are ERR red.
fn push_shell(
    lines: &mut Vec<Line<'static>>,
    outcome: &crate::shell::ShellOutcome,
    width: usize,
    theme: &style::ThemeColors,
) {
    // Command line: `! <command>` at the content margin (`!` ORANGE, command BRIGHT).
    let bang = Style::default()
        .fg(theme.primary)
        .add_modifier(Modifier::BOLD);
    lines.push(Line::from(vec![
        Span::raw(" ".repeat(CONTENT_MARGIN)),
        Span::styled(format!("{} ", glyphs::SHELL_PREFIX), bang),
        Span::styled(outcome.command.clone(), Style::default().fg(theme.text_bright)),
    ]));

    let bx = CONTENT_MARGIN;
    let right = width.saturating_sub(CONTENT_MARGIN);
    // Too narrow for a bordered pane: fall back to plain lines with the same semantic coloring.
    if right <= bx + 4 {
        for raw in outcome.output.split('\n') {
            let style = shell_line_style(raw, outcome.success, theme);
            lines.push(Line::from(Span::styled(clip_text(raw, width), style)));
        }
        lines.push(Line::from(""));
        return;
    }
    let bw = right - bx;
    let inner_w = bw.saturating_sub(4); // border + one padding column on each side
    let base = Style::default().bg(theme.background);
    let border_style = Style::default().fg(theme.border).bg(theme.background);

    // Top border with the AMBER `output` title embedded: `╭─ output ─────╮`.
    let mut top: Vec<(char, Style)> = vec![(' ', base); width];
    top[bx] = (glyphs::BOX_TOP_LEFT.chars().next().unwrap(), border_style);
    top[bx + bw - 1] = (glyphs::BOX_TOP_RIGHT.chars().next().unwrap(), border_style);
    for cell in top.iter_mut().take(bx + bw - 1).skip(bx + 1) {
        *cell = (glyphs::BOX_HORIZONTAL.chars().next().unwrap(), border_style);
    }
    place_segments(
        &mut top,
        bx + 2,
        &[(
            " output ".to_owned(),
            Style::default().fg(theme.secondary).bg(theme.background),
        )],
    );
    lines.push(cells_to_line(&top));

    // Content rows on the near-black interior, one per output line (clipped to the inner width).
    let origin = bx + 2;
    for raw in outcome.output.split('\n') {
        let mut cells: Vec<(char, Style)> = vec![(' ', base); width];
        cells[bx] = (glyphs::BOX_VERTICAL.chars().next().unwrap(), border_style);
        cells[bx + bw - 1] = (glyphs::BOX_VERTICAL.chars().next().unwrap(), border_style);
        let text = clip_text(raw, inner_w);
        let style = shell_line_style(&text, outcome.success, theme);
        place_segments(&mut cells, origin, &[(text, style)]);
        lines.push(cells_to_line(&cells));
    }

    // Bottom border: `╰────────╯`.
    let mut bottom: Vec<(char, Style)> = vec![(' ', base); width];
    bottom[bx] = (glyphs::BOX_BOTTOM_LEFT.chars().next().unwrap(), border_style);
    bottom[bx + bw - 1] = (glyphs::BOX_BOTTOM_RIGHT.chars().next().unwrap(), border_style);
    for cell in bottom.iter_mut().take(bx + bw - 1).skip(bx + 1) {
        *cell = (glyphs::BOX_HORIZONTAL.chars().next().unwrap(), border_style);
    }
    lines.push(cells_to_line(&bottom));
    lines.push(Line::from(""));
}

/// Append the plan-mode `blocked` banner for a shell/build command hard-denied while planning: a
/// full-width rounded box (content-margin inset) whose single content row reads
/// `▌ blocked  <command>  —  <reason>` with the `▌` edge + `blocked` word in DENY red (ERR), the
/// command in BRIGHT, and the reason in DIM. Distinct in hue from the amber plan-mode footer banner
/// (VAL-PERM-019/021).
fn push_blocked(
    lines: &mut Vec<Line<'static>>,
    command: &str,
    reason: &str,
    width: usize,
    theme: &style::ThemeColors,
) {
    let bx = CONTENT_MARGIN;
    let right = width.saturating_sub(CONTENT_MARGIN);
    if right <= bx + 4 {
        return;
    }
    let bw = right - bx;
    let base = Style::default().bg(theme.background);
    let border_style = Style::default().fg(theme.border).bg(theme.background);
    let deny = Style::default().fg(theme.error).bg(theme.background);
    let deny_bold = deny.add_modifier(Modifier::BOLD);
    let cmd_style = Style::default().fg(theme.text_bright).bg(theme.background);
    let reason_style = Style::default().fg(theme.text_dim).bg(theme.background);
    let sep_style = Style::default().fg(theme.ghost).bg(theme.background);

    // Top border `╭────────╮`.
    let mut top: Vec<(char, Style)> = vec![(' ', base); width];
    top[bx] = (glyphs::BOX_TOP_LEFT.chars().next().unwrap(), border_style);
    top[bx + bw - 1] = (glyphs::BOX_TOP_RIGHT.chars().next().unwrap(), border_style);
    for cell in top.iter_mut().take(bx + bw - 1).skip(bx + 1) {
        *cell = (glyphs::BOX_HORIZONTAL.chars().next().unwrap(), border_style);
    }
    lines.push(cells_to_line(&top));

    // Content row: `│ ▌ blocked  <command>  —  <reason> │` on the canvas background.
    let mut row: Vec<(char, Style)> = vec![(' ', base); width];
    row[bx] = (glyphs::BOX_VERTICAL.chars().next().unwrap(), border_style);
    row[bx + bw - 1] = (glyphs::BOX_VERTICAL.chars().next().unwrap(), border_style);
    let origin = bx + 2;
    let inner_room = (bx + bw - 1).saturating_sub(origin + 1);
    let segments: Vec<(String, Style)> = vec![
        (format!("{} ", glyphs::ACCENT_BAR), deny),
        ("blocked".to_owned(), deny_bold),
        ("  ".to_owned(), base),
        (clip_text(command, inner_room), cmd_style),
        (format!("  {}  ", glyphs::EM_DASH), sep_style),
        (reason.to_owned(), reason_style),
    ];
    place_segments(&mut row, origin, &segments);
    lines.push(cells_to_line(&row));

    // Bottom border `╰────────╯`.
    let mut bottom: Vec<(char, Style)> = vec![(' ', base); width];
    bottom[bx] = (glyphs::BOX_BOTTOM_LEFT.chars().next().unwrap(), border_style);
    bottom[bx + bw - 1] = (glyphs::BOX_BOTTOM_RIGHT.chars().next().unwrap(), border_style);
    for cell in bottom.iter_mut().take(bx + bw - 1).skip(bx + 1) {
        *cell = (glyphs::BOX_HORIZONTAL.chars().next().unwrap(), border_style);
    }
    lines.push(cells_to_line(&bottom));
    lines.push(Line::from(""));
}
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

/// The composer's typed input rendered as styled lines. Text is BRIGHT (`base`); while the
/// `@`-mention popup is open, the active `@<partial>` token on the cursor's line (the `@` plus the
/// query typed up to the cursor) is drawn in AMBER (`theme.secondary`) so the in-progress mention
/// reads as a distinct chip. Every other line, and the rest of the cursor's line, keep `base`.
fn composer_input_lines(
    app: &App,
    base: Style,
    theme: &style::ThemeColors,
) -> Vec<Line<'static>> {
    let amber = Style::default().fg(theme.secondary);
    // The active token span on the cursor's line: `@`.. up to the cursor (chars), when a mention is
    // open. `mention_query` guarantees the run starts at an `@` at a word boundary.
    let mention_query = if app.mention_open() {
        app.mention_query()
    } else {
        None
    };
    let token = mention_query.map(|query| {
        let cursor = app.cursor();
        let end = cursor.col;
        let start = end.saturating_sub(query.chars().count() + 1);
        (cursor.row, start, end)
    });

    app.input
        .lines()
        .iter()
        .enumerate()
        .map(|(row, line)| {
            if let Some((trow, start, end)) = token {
                let chars: Vec<char> = line.chars().collect();
                if row == trow
                    && end <= chars.len()
                    && start < end
                    && chars.get(start) == Some(&'@')
                {
                    let before: String = chars[..start].iter().collect();
                    let tok: String = chars[start..end].iter().collect();
                    let after: String = chars[end..].iter().collect();
                    let mut spans = Vec::new();
                    if !before.is_empty() {
                        spans.push(Span::styled(before, base));
                    }
                    spans.push(Span::styled(tok, amber));
                    if !after.is_empty() {
                        spans.push(Span::styled(after, base));
                    }
                    return Line::from(spans);
                }
            }
            Line::from(Span::styled(line.clone(), base))
        })
        .collect()
}

fn draw_prompt(frame: &mut Frame, app: &App, area: Rect, theme: &style::ThemeColors) {
    // The composer is a plain rounded `ORANGE_D` box on the brand canvas (no title) in every state,
    // including `!` shell mode — the shell affordance is carried by the `shell` breadcrumb, the
    // `manual · ! shell mode` status bar, the `!` prompt prefix, and the shell placeholder/hints.
    let shell = app.shell_mode();
    let block = Block::default()
        .style(Style::default().bg(theme.background).fg(theme.text))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    // Staged attachments are shown as bordered cards in the transcript (the attach state), so the
    // composer itself stays a clean input box — the whole inner area is the input region.
    let input_area = inner;

    // Composer prefix at the left inner column (ORANGE): the `!` shell prompt in shell mode, else
    // the `▌ ›` accent edge + prompt glyph.
    let accent = Style::default().fg(theme.primary);
    let prefix_w = inner.width.min(3);
    let prefix_line = if shell {
        Line::from(vec![Span::styled(crate::glyphs::SHELL_PREFIX, accent)])
    } else {
        Line::from(vec![
            Span::styled(crate::glyphs::ACCENT_BAR, accent),
            Span::raw(" "),
            Span::styled(crate::glyphs::PROMPT, accent),
        ])
    };
    frame.render_widget(
        Paragraph::new(prefix_line),
        Rect {
            x: inner.x,
            y: input_area.y,
            width: prefix_w,
            height: 1,
        },
    );

    // Right-aligned FAINT action hint on the input row (reserved out of the text region). The hint
    // tracks the composer's active state: shell mode advertises `↵ to send · esc to exit`, an open
    // `/` palette submits with `↵`, an open `@` palette inserts the highlighted file with `⇥`, and
    // the plain chat composer sends with `↵`.
    let hint = if shell {
        format!("{} to send · esc to exit", crate::glyphs::KEY_ENTER)
    } else if app.slash_menu_open() {
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

    // The text region begins after the prefix so typed text and the caret sit at the input column
    // and continuation lines hang under it: `▌ › ` is 4 columns in chat, `! ` is 2 in shell mode.
    let prefix = if shell { 2u16 } else { 4u16 };
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
            "run another shell command…"
        } else if app.plan_mode() {
            "ask a question or refine the plan…"
        } else if !app.composer_attachments().is_empty() {
            "add a message, or drop more files…"
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
        // Typed text renders in BRIGHT (replacing the FAINT placeholder); while the `@`-mention
        // popup is open the in-progress `@<partial>` token on the cursor's line is drawn in AMBER so
        // the live mention chip reads apart from the surrounding prose.
        let bright = Style::default().fg(theme.text_bright);
        let lines: Vec<Line> = composer_input_lines(app, bright, theme);
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

/// Background fill for a highlighted floating-palette row — the warm `SELBG(44,35,26)` inverse bar
/// from the `/` and `@` reference mockups. This is the *subtle* selection (a warm tint that keeps
/// the row text readable) used by the composer-anchored `/` and `@` dropdowns; it is the theme's
/// highest surface tone so it tracks every palette.
fn palette_selection_bg(theme: &style::ThemeColors) -> Color {
    theme.surface[3]
}

/// Total display width (columns) of a row's spans.
fn spans_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(Span::width).sum()
}

/// Append a trailing run of spaces (carrying `fill`) so `spans` occupies exactly `width` columns —
/// used to paint a full-width selection bar behind a palette row.
fn pad_spans_to_width(spans: &mut Vec<Span<'static>>, width: usize, fill: Style) {
    let used = spans_width(spans);
    if width > used {
        spans.push(Span::styled(" ".repeat(width - used), fill));
    }
}

/// Truncate `s` to at most `max` columns, appending `…` when it is cut.
fn clip_cols(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        return s.to_owned();
    }
    if max == 0 {
        return String::new();
    }
    let mut out: String = s.chars().take(max - 1).collect();
    out.push('…');
    out
}

/// Lay out a palette row as `left` spans, a `fill` run, then a right-aligned `meta` span, occupying
/// exactly `width` columns with a one-column right margin before the border. Used by the `@`-file
/// palette so the `type · size` meta column is flush-right on every row (and on the selection bar).
fn palette_meta_row(
    mut left: Vec<Span<'static>>,
    meta: Span<'static>,
    width: usize,
    fill: Style,
) -> Line<'static> {
    const MARGIN: usize = 1;
    let reserved = spans_width(&left) + meta.width() + MARGIN;
    let pad = width.saturating_sub(reserved).max(1);
    left.push(Span::styled(" ".repeat(pad), fill));
    left.push(meta);
    left.push(Span::styled(" ".repeat(MARGIN), fill));
    Line::from(left)
}

/// The file name (last path segment) shown as the left column of a `@`-mention row, e.g.
/// `buffer.rs` for `crates/tui/src/buffer.rs`. Falls back to the whole path when it has no segment.
fn mention_filename(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| path.to_owned())
}

/// The `dir · size` meta string for a file path: its parent directory (`.` at the repo root) and
/// its real on-disk size (resolved against the working directory), or just the directory when the
/// size cannot be read. Every row carries the same `dir · size` shape so the meta column is uniform.
fn mention_meta(path: &str) -> String {
    let dir = std::path::Path::new(path)
        .parent()
        .map(|parent| parent.to_string_lossy().into_owned())
        .filter(|dir| !dir.is_empty())
        .unwrap_or_else(|| ".".to_owned());
    let size = std::env::current_dir()
        .ok()
        .and_then(|cwd| std::fs::metadata(cwd.join(path)).ok())
        .map(|meta| crate::mention::format_size(meta.len()));
    match size {
        Some(size) => format!("{dir} {} {size}", crate::glyphs::SEPARATOR),
        None => dir,
    }
}

/// Build the `/` command-palette rows for the current query, laid out for an `width`-column inner
/// area, or `None` when the menu is closed. Each row shows `/name  description` (command ORANGE,
/// description FAINT); the highlighted row is a full-width warm `SELBG` bar with a `›` ORANGE marker
/// and a BRIGHT command label. A non-empty query that matches nothing yields a single graceful
/// "no matching commands" row.
fn slash_menu_lines(app: &App, width: usize) -> Option<Vec<Line<'static>>> {
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
        let name = format!("/{}", command.name);
        let desc = command.description.to_owned();
        if i == selected {
            let bg = palette_selection_bg(&theme);
            let on_bar = Style::default().bg(bg);
            let mut spans = vec![
                Span::styled(
                    format!("{} ", crate::glyphs::SELECT_MARKER),
                    Style::default().fg(theme.primary).bg(bg),
                ),
                Span::styled(
                    name,
                    Style::default()
                        .fg(theme.text_bright)
                        .bg(bg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("  ", on_bar),
                Span::styled(desc, Style::default().fg(theme.text_muted).bg(bg)),
            ];
            pad_spans_to_width(&mut spans, width, on_bar);
            lines.push(Line::from(spans));
        } else {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(name, Style::default().fg(theme.primary)),
                Span::raw("  "),
                Span::styled(desc, Style::default().fg(theme.text_muted)),
            ]));
        }
    }
    Some(lines)
}

/// Render the `/` command palette as a floating rounded dropdown anchored directly above the
/// composer, spanning the content width at the left content margin. The frame is the brand
/// `ORANGE_D` border with an AMBER `Commands` title-in-border over the near-black canvas; the view
/// scrolls to keep the highlighted row visible when the match list is long.
fn draw_slash_menu(frame: &mut Frame, app: &App, prompt_area: Rect, theme: &style::ThemeColors) {
    if prompt_area.width < 2 {
        return;
    }
    let inner_width = prompt_area.width.saturating_sub(2) as usize;
    let Some(rows) = slash_menu_lines(app, inner_width) else {
        return;
    };

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
        .style(Style::default().bg(theme.background))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border))
        .title(" Commands ")
        .title_style(
            Style::default()
                .fg(theme.secondary)
                .add_modifier(Modifier::BOLD),
        );
    let inner = block.inner(popup);

    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);
    frame.render_widget(
        Paragraph::new(shown).style(Style::default().fg(theme.text).bg(theme.background)),
        inner,
    );
}

/// Build the `@`-file mention rows for the current query, laid out for an `width`-column inner area,
/// or `None` when the picker is closed. Each row shows the file name + a right-aligned uniform
/// `dir · size` meta column (FAINT). The highlighted row is a full-width warm `SELBG` bar with a `›`
/// ORANGE marker and a BRIGHT file name. A query that matches no file yields a single graceful
/// "no matching files" row.
fn mention_menu_lines(app: &App, width: usize) -> Option<Vec<Line<'static>>> {
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
        let meta = mention_meta(path);
        let name = mention_filename(path);
        // Clip the file name so the marker (2) + a 2-col gap + meta + 1-col margin fit.
        let name_budget = width
            .saturating_sub(2 + meta.chars().count() + 2 + 1)
            .max(4);
        let name_text = clip_cols(&name, name_budget);
        if i == selected {
            let bg = palette_selection_bg(&theme);
            let on_bar = Style::default().bg(bg);
            let left = vec![
                Span::styled(
                    format!("{} ", crate::glyphs::SELECT_MARKER),
                    Style::default().fg(theme.primary).bg(bg),
                ),
                Span::styled(
                    name_text,
                    Style::default()
                        .fg(theme.text_bright)
                        .bg(bg)
                        .add_modifier(Modifier::BOLD),
                ),
            ];
            let meta_span = Span::styled(meta, Style::default().fg(theme.text_muted).bg(bg));
            lines.push(palette_meta_row(left, meta_span, width, on_bar));
        } else {
            let left = vec![
                Span::raw("  "),
                Span::styled(name_text, Style::default().fg(theme.text)),
            ];
            let meta_span = Span::styled(meta, Style::default().fg(theme.text_muted));
            lines.push(palette_meta_row(left, meta_span, width, Style::default()));
        }
    }
    Some(lines)
}

/// Render the `@`-file mention popup as a floating rounded dropdown anchored directly above the
/// composer (same anchor/width/border as the `/` palette) with an AMBER `@ files` title-in-border
/// and a right-aligned `<query> shown/total` match-count tag in the top border, over the near-black
/// canvas, scrolling to keep the highlighted row visible when the match list is long.
fn draw_mention_menu(frame: &mut Frame, app: &App, prompt_area: Rect, theme: &style::ThemeColors) {
    if prompt_area.width < 2 {
        return;
    }
    let inner_width = prompt_area.width.saturating_sub(2) as usize;
    let Some(rows) = mention_menu_lines(app, inner_width) else {
        return;
    };

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
        .style(Style::default().bg(theme.background))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border))
        .title(" @ files ")
        .title_style(
            Style::default()
                .fg(theme.secondary)
                .add_modifier(Modifier::BOLD),
        );
    let inner = block.inner(popup);

    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);
    frame.render_widget(
        Paragraph::new(shown).style(Style::default().fg(theme.text).bg(theme.background)),
        inner,
    );

    // Right-aligned match-count tag in the top border: the live query and `shown/total` file counts
    // (FAINT), so the popup advertises how many workspace files match what has been typed.
    let query = app.mention_query().unwrap_or_default();
    let matched = app.mention_matches().len();
    let total = app.mention_file_total();
    let count = if query.is_empty() {
        format!(" {matched}/{total} ")
    } else {
        format!(" {query}  {matched}/{total} ")
    };
    let count_len = count.chars().count() as u16;
    if count_len + 4 <= popup.width {
        let count_area = Rect {
            x: popup.x + popup.width - 1 - count_len,
            y: popup.y,
            width: count_len,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                count,
                Style::default().fg(theme.text_muted).bg(theme.background),
            ))),
            count_area,
        );
    }
}

fn draw_find_bar(frame: &mut Frame, app: &App, area: Rect, theme: &style::ThemeColors) {
    let Some(find) = app.find() else {
        return;
    };
    if area.width == 0 || area.height == 0 {
        return;
    }

    let width = area.width as usize;
    let (active, total) = app.find_status();
    let base = Style::default().fg(theme.text).bg(theme.surface[0]);
    let mut cells: Vec<(char, Style)> = vec![(' ', base); width.max(1)];

    let label = vec![(
        "find:".to_owned(),
        Style::default().fg(theme.border).bg(theme.surface[0]),
    )];
    place_segments(&mut cells, 2, &label);

    let query_start = 8;
    if find.query.is_empty() {
        let hint = vec![(
            "type to search".to_owned(),
            Style::default()
                .fg(theme.text_muted)
                .bg(theme.surface[0]),
        )];
        place_segments(&mut cells, query_start, &hint);
    } else {
        let q_len = find.query.chars().count();
        let avail = width.saturating_sub(query_start + 1);
        let q_display = if q_len > avail {
            find.query.chars().take(avail).collect()
        } else {
            find.query.clone()
        };
        let q_w = q_display.chars().count();
        let query_bg = theme.selection_bg;
        for i in 0..q_w.min(width.saturating_sub(query_start)) {
            cells[query_start + i].1 = cells[query_start + i].1.bg(query_bg);
        }
        let query = vec![(
            q_display,
            Style::default().fg(theme.text_bright).bg(query_bg),
        )];
        place_segments(&mut cells, query_start, &query);

        let caret_col = query_start + q_w;
        if caret_col < width {
            cells[caret_col] = (
                glyphs::CURSOR.chars().next().unwrap_or('▏'),
                Style::default().fg(theme.primary).bg(query_bg),
            );
        }

        let counter_start = query_start + q_w + 2;
        if counter_start < width {
            let counter_text = if total == 0 {
                "0/0".to_owned()
            } else {
                format!("{}/{}", active + 1, total)
            };
            let counter = vec![
                (
                    counter_text,
                    Style::default().fg(theme.secondary).bg(theme.surface[0]),
                ),
                (
                    " matches".to_owned(),
                    Style::default()
                        .fg(theme.text_muted)
                        .bg(theme.surface[0]),
                ),
            ];
            place_segments(&mut cells, counter_start, &counter);
        }
    }

    let hints_text = "n next   N prev   esc close";
    let hints_len = hints_text.chars().count();
    if width > hints_len + 2 {
        let hints_start = width.saturating_sub(hints_len + 2);
        let hints = vec![(
            hints_text.to_owned(),
            Style::default()
                .fg(theme.text_muted)
                .bg(theme.surface[0]),
        )];
        place_segments(&mut cells, hints_start, &hints);
    }

    let line = cells_to_line(&cells);
    frame.render_widget(
        Paragraph::new(line).style(base),
        area,
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

/// Draw an open full-screen Wave-2 view: the shared chrome (a `✦ Cortex Code · <crumb>` header, a
/// rule, and the 3-zone status bar) wrapping a titled panel whose body is rendered per view kind.
/// The chrome, crumb, status bar, and brand palette are shared across every view so the surfaces
/// read consistently; a view opened with no data renders an explicit empty state, never a blank
/// panel.
fn draw_full_screen_view(frame: &mut Frame, app: &App, area: Rect, theme: &style::ThemeColors) {
    use crate::view::{self, ViewKind};
    let Some(kind) = app.active_view() else {
        return;
    };
    let body = view::render_chrome(frame, area, app, kind.crumb(), theme);
    if body.width <= view::VIEW_MARGIN * 2 || body.height < 2 {
        return;
    }
    let panel_area = Rect {
        x: body.x + view::VIEW_MARGIN,
        y: body.y,
        width: body.width - view::VIEW_MARGIN * 2,
        height: body.height,
    };
    let note = view_note(app, kind);
    let inner = view::render_panel(frame, panel_area, kind.title(), note.as_deref(), theme);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    match kind {
        ViewKind::Status => draw_status_view_body(frame, app, inner, theme),
        ViewKind::Timeline => draw_timeline_view_body(frame, app, inner, theme),
        ViewKind::Mcp => draw_mcp_view_body(frame, app, inner, theme),
        ViewKind::Queue => draw_queue_view_body(frame, app, inner, theme),
        ViewKind::Help => draw_help_view_body(frame, inner, theme),
        ViewKind::Review => draw_review_view_body(frame, app, inner, theme),
        _ => draw_view_placeholder(frame, inner, theme),
    }
}

/// The `/help` view body: a two-column command-reference grid (Session/Model & agent/Views/Editing)
/// listing real Cortex slash builtins + key affordances, rendered via the shared
/// [`crate::view::help_body_lines`] helper (theme tokens + safe glyphs only).
fn draw_help_view_body(frame: &mut Frame, area: Rect, theme: &style::ThemeColors) {
    let target = view_body_inset(area);
    if target.width == 0 || target.height == 0 {
        return;
    }
    let lines =
        crate::view::help_body_lines(target.width as usize, target.height as usize, theme);
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().fg(theme.text).bg(theme.background)),
        target,
    );
}

/// The top-right panel note for a view (the MCP server/tool totals, the timeline event count, or
/// the queued-prompt count), or `None` for views without a summary note.
fn view_note(app: &App, kind: crate::view::ViewKind) -> Option<String> {
    match kind {
        crate::view::ViewKind::Mcp => {
            let rows = app.mcp_view_rows();
            let servers = rows.len();
            let tools: usize = rows.iter().filter_map(|row| row.tool_count).sum();
            Some(format!(
                "{servers} server{} · {tools} tool{}",
                if servers == 1 { "" } else { "s" },
                if tools == 1 { "" } else { "s" }
            ))
        }
        crate::view::ViewKind::Timeline => {
            let count = app.timeline_snapshot().len();
            Some(format!("{count} event{}", if count == 1 { "" } else { "s" }))
        }
        crate::view::ViewKind::Queue => {
            let count = app.queue_snapshot().len();
            Some(format!("{count} queued"))
        }
        crate::view::ViewKind::Review => app.review_snapshot().map(crate::view::ReviewData::note),
        _ => None,
    }
}

/// Left-inset the panel inner region by one column so body text does not abut the border.
fn view_body_inset(area: Rect) -> Rect {
    Rect {
        x: area.x + 1,
        y: area.y,
        width: area.width.saturating_sub(2),
        height: area.height,
    }
}

/// The `/status` view body: aligned key/value rows (session/started/agent/model), a context-usage
/// bar, token/cost rows, and permits/workspace rows, from the snapshot captured when the view
/// opened. Rendered via the shared [`crate::view::status_body_lines`] helper (theme tokens only).
fn draw_status_view_body(frame: &mut Frame, app: &App, area: Rect, theme: &style::ThemeColors) {
    let target = view_body_inset(area);
    let lines = match app.status_snapshot() {
        Some(data) => crate::view::status_body_lines(data, target.width as usize, theme),
        None => vec![crate::view::empty_state_line("Session status unavailable.", theme)],
    };
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().fg(theme.text).bg(theme.background)),
        target,
    );
}

/// The `/timeline` view body: the persisted session's event rail (`●/○/◆` dots linked by `│`
/// connectors, a faint timestamp column, a colored author column, and a dim description) from the
/// snapshot captured when the view opened. Rendered via [`crate::view::timeline_body_lines`].
fn draw_timeline_view_body(frame: &mut Frame, app: &App, area: Rect, theme: &style::ThemeColors) {
    let target = view_body_inset(area);
    let lines = crate::view::timeline_body_lines(
        app.timeline_snapshot(),
        target.width as usize,
        target.height as usize,
        theme,
    );
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().fg(theme.text).bg(theme.background)),
        target,
    );
}

/// The `/mcp` view body: a faint `server/status/tools/exposes` column header + rule, then one
/// aligned row per configured/connected server (name, `●` status dot + word colored by state, tool
/// count, and the clipped exposed-capabilities list), or the explicit empty state when no servers
/// are configured. Rendered via the shared [`crate::view::mcp_body_lines`] helper (theme tokens only).
fn draw_mcp_view_body(frame: &mut Frame, app: &App, area: Rect, theme: &style::ThemeColors) {
    let target = view_body_inset(area);
    let rows = app.mcp_view_rows();
    let lines = crate::view::mcp_body_lines(&rows, target.width as usize, theme);
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().fg(theme.text).bg(theme.background)),
        target,
    );
}

/// The `/queue` view body: an intro line, the numbered queued-prompt rows (selection bar + AMBER
/// `next` badge on the head of the queue), and a footer key-hint row pinned to the bottom of the
/// panel. Rendered via the shared [`crate::view::queue_body_lines`] / [`crate::view::footer_hint_line`]
/// helpers (theme tokens + safe glyphs only).
fn draw_queue_view_body(frame: &mut Frame, app: &App, area: Rect, theme: &style::ThemeColors) {
    let target = view_body_inset(area);
    if target.width == 0 || target.height == 0 {
        return;
    }
    let base = Style::default().fg(theme.text).bg(theme.background);

    // Reserve the bottom row for the footer hints when there is room for a body above it.
    let footer_rows = u16::from(target.height >= 3);
    let body_area = Rect {
        height: target.height - footer_rows,
        ..target
    };
    let lines = crate::view::queue_body_lines(
        app.queue_snapshot(),
        app.queue_selection(),
        target.width as usize,
        theme,
    );
    frame.render_widget(Paragraph::new(lines).style(base), body_area);

    if footer_rows == 1 {
        let footer_area = Rect {
            y: target.y + target.height - 1,
            height: 1,
            ..target
        };
        let reorder_key = format!("^{}{}", crate::glyphs::KEY_UP, crate::glyphs::KEY_DOWN);
        let hints = vec![
            (crate::glyphs::KEY_ENTER.to_owned(), "edit".to_owned()),
            (reorder_key, "reorder".to_owned()),
            ("dd".to_owned(), "remove".to_owned()),
            ("esc".to_owned(), "close".to_owned()),
        ];
        frame.render_widget(
            Paragraph::new(crate::view::footer_hint_line(
                &hints,
                target.width as usize,
                theme,
            ))
            .style(base),
            footer_area,
        );
    }
}

/// The `/review` view body: an intro line, the code-review findings (`●` severity dot + severity
/// word colored by level, an AMBER `file:line` location, and the finding text) at 2-row spacing, a
/// rule, and a severity-count summary line, with a footer key-hint row pinned to the bottom of the
/// panel. Rendered via the shared [`crate::view::review_body_lines`] / [`crate::view::footer_hint_line`]
/// helpers (theme tokens + safe glyphs only), from the snapshot captured when the view opened.
fn draw_review_view_body(frame: &mut Frame, app: &App, area: Rect, theme: &style::ThemeColors) {
    let target = view_body_inset(area);
    if target.width == 0 || target.height == 0 {
        return;
    }
    let base = Style::default().fg(theme.text).bg(theme.background);

    // Reserve the bottom row for the footer hints when there is room for a body above it.
    let footer_rows = u16::from(target.height >= 3);
    let body_area = Rect {
        height: target.height - footer_rows,
        ..target
    };
    let lines = match app.review_snapshot() {
        Some(data) => crate::view::review_body_lines(
            data,
            target.width as usize,
            body_area.height as usize,
            theme,
        ),
        None => vec![crate::view::empty_state_line(
            "No review data available.",
            theme,
        )],
    };
    frame.render_widget(Paragraph::new(lines).style(base), body_area);

    if footer_rows == 1 {
        let footer_area = Rect {
            y: target.y + target.height - 1,
            height: 1,
            ..target
        };
        let hints = vec![
            (crate::glyphs::KEY_ENTER.to_owned(), "open finding".to_owned()),
            ("f".to_owned(), "fix with agent".to_owned()),
            ("esc".to_owned(), "close".to_owned()),
        ];
        frame.render_widget(
            Paragraph::new(crate::view::footer_hint_line(
                &hints,
                target.width as usize,
                theme,
            ))
            .style(base),
            footer_area,
        );
    }
}

/// The default body for a view whose rich content is built by a later Wave-2 feature: an explicit
/// placeholder so the shared chrome is still exercised without a blank panel.
fn draw_view_placeholder(frame: &mut Frame, area: Rect, theme: &style::ThemeColors) {
    frame.render_widget(
        Paragraph::new(vec![crate::view::empty_state_line(
            "Nothing to show yet.",
            theme,
        )])
        .style(Style::default().fg(theme.text).bg(theme.background)),
        view_body_inset(area),
    );
}

/// Render the `/diff` viewer as a full-screen Wave-2 view: the shared chrome (`✦ Cortex Code ·
/// /diff` header + rule + 3-zone status bar) wrapping a titled `Changes` panel. The panel body is a
/// multi-file diff accordion — one disclosure row per changed file (`▼` open / `▶` collapsed) with
/// the open file's hunk body unfolded inline (line-numbered red/green rows reusing the wave0
/// conv-diff-card tokens). The selected file row carries the shared inverse selection bar.
fn draw_diff_viewer(frame: &mut Frame, app: &App, area: Rect, theme: &style::ThemeColors) {
    let Some(viewer) = app.diff_viewer() else {
        return;
    };
    let body = crate::view::render_chrome(
        frame,
        area,
        app,
        crate::view::ViewKind::Diff.crumb(),
        theme,
    );
    if body.width <= crate::view::VIEW_MARGIN * 2 || body.height < 2 {
        return;
    }
    let panel_area = Rect {
        x: body.x + crate::view::VIEW_MARGIN,
        y: body.y,
        width: body.width - crate::view::VIEW_MARGIN * 2,
        height: body.height,
    };
    let note = diff_view_note(viewer);
    let inner = crate::view::render_panel(
        frame,
        panel_area,
        crate::view::ViewKind::Diff.title(),
        Some(&note),
        theme,
    );
    let target = view_body_inset(inner);
    if target.width == 0 || target.height == 0 {
        return;
    }
    let base = Style::default().fg(theme.text).bg(theme.background);

    // Reserve the bottom row for the footer key-hints when there is room for a body above it.
    let footer_rows = u16::from(target.height >= 3);
    let body_area = Rect {
        height: target.height - footer_rows,
        ..target
    };

    let (lines, selected_row) = diff_accordion_lines(viewer, target.width as usize, theme);
    // Scroll so the selected file's disclosure row (and the hunk body below it) stays in view; the
    // body is rebuilt every frame, so the offset is derived, not stored.
    let visible = (body_area.height as usize).max(1);
    let max_offset = lines.len().saturating_sub(visible);
    let offset = selected_row.min(max_offset);
    let end = (offset + visible).min(lines.len());
    let shown: Vec<Line<'static>> = lines[offset..end].to_vec();
    frame.render_widget(Paragraph::new(shown).style(base), body_area);

    if footer_rows == 1 {
        let footer_area = Rect {
            y: target.y + target.height - 1,
            height: 1,
            ..target
        };
        let hints = vec![
            (crate::glyphs::KEY_ENTER.to_owned(), "toggle file".to_owned()),
            (
                format!("{}{}", crate::glyphs::KEY_UP, crate::glyphs::KEY_DOWN),
                "file".to_owned(),
            ),
            (crate::glyphs::KEY_TAB.to_owned(), "hunks".to_owned()),
            ("v".to_owned(), "split".to_owned()),
            ("r".to_owned(), "revert".to_owned()),
            ("esc".to_owned(), "close".to_owned()),
        ];
        frame.render_widget(
            Paragraph::new(crate::view::footer_hint_line(
                &hints,
                target.width as usize,
                theme,
            ))
            .style(base),
            footer_area,
        );
    }
}

/// The `Changes` panel note summarizing the diff totals: `N files  +A -R` across every changed file.
fn diff_view_note(viewer: &crate::app::DiffViewer) -> String {
    let files = viewer.files.len();
    let additions: usize = viewer.files.iter().map(|file| file.additions).sum();
    let removals: usize = viewer.files.iter().map(|file| file.removals).sum();
    format!(
        "{files} file{}  +{additions} -{removals}",
        if files == 1 { "" } else { "s" }
    )
}

/// Build the `/diff` accordion body: one disclosure row per changed file, with the hunk body of each
/// expanded file unfolded inline beneath it. Returns the lines plus the row index of the selected
/// file's header (so the caller can scroll it into view).
fn diff_accordion_lines(
    viewer: &crate::app::DiffViewer,
    width: usize,
    theme: &style::ThemeColors,
) -> (Vec<Line<'static>>, usize) {
    if viewer.files.is_empty() {
        return (
            vec![crate::view::empty_state_line("No file changes to show.", theme)],
            0,
        );
    }
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut selected_row = 0usize;
    for (index, file) in viewer.files.iter().enumerate() {
        let expanded = viewer.is_expanded(index);
        if index == viewer.selected_file {
            selected_row = lines.len();
        }
        lines.push(diff_file_row(
            file,
            expanded,
            index == viewer.selected_file,
            width,
            theme,
        ));
        if expanded {
            lines.extend(diff_hunk_lines(file, viewer.mode, width, theme));
            lines.push(Line::from(""));
        }
    }
    (lines, selected_row)
}

/// One diff-viewer file row: a `▼`/`▶` disclosure at the row start, the file path, and a
/// right-aligned `+A -R` count (AMBER for the open/expanded file, DIM for a collapsed file). The
/// selected file is drawn as the shared inverse selection bar (`surface[3]` fill + BRIGHT path),
/// matching the slash/dialog/palette selection rows (not the old ad-hoc `info`+`surface[2]`).
fn diff_file_row(
    file: &FileDiff,
    expanded: bool,
    selected: bool,
    width: usize,
    theme: &style::ThemeColors,
) -> Line<'static> {
    let width = width.max(1);
    let row_bg = if selected {
        theme.surface[3]
    } else {
        theme.background
    };
    let mut cells: Vec<(char, Style)> =
        vec![(' ', Style::default().fg(theme.text).bg(row_bg)); width];

    let disclosure = if expanded {
        crate::glyphs::DISCLOSURE_EXPANDED
    } else {
        crate::glyphs::DISCLOSURE_COLLAPSED
    };
    let disclosure_color = if expanded {
        theme.secondary
    } else {
        theme.border
    };
    let path_style = Style::default()
        .fg(if expanded || selected {
            theme.text_bright
        } else {
            theme.text
        })
        .bg(row_bg);

    // Per VAL-VIEW-024 the `+N -N` count is a single amber (open) / dim (collapsed) meta, and per
    // VAL-VIEW-026 the ONLY green/red cells in the view are the hunk insert/delete rows — so the meta
    // is deliberately NOT the success/error pair the inline card uses.
    let meta = format!("+{} -{}", file.additions, file.removals);
    let meta_len = meta.chars().count();
    let meta_color = if expanded {
        theme.secondary
    } else {
        theme.text_dim
    };
    let path_room = width.saturating_sub(meta_len + 4).max(1);
    let path = clip_text(&file.path, path_room);
    place_segments(
        &mut cells,
        0,
        &[
            (
                format!("{disclosure} "),
                Style::default().fg(disclosure_color).bg(row_bg),
            ),
            (path, path_style),
        ],
    );
    if width > meta_len + 1 {
        place_segments(
            &mut cells,
            width - meta_len - 1,
            &[(meta, Style::default().fg(meta_color).bg(row_bg))],
        );
    }
    cells_to_line(&cells)
}

/// The hunk body for an expanded diff-viewer file, in the active render mode.
fn diff_hunk_lines(
    file: &FileDiff,
    mode: DiffViewMode,
    width: usize,
    theme: &style::ThemeColors,
) -> Vec<Line<'static>> {
    match mode {
        DiffViewMode::Unified => diff_unified_hunk_lines(file, width, theme),
        DiffViewMode::Split => diff_split_hunk_lines(file, width, theme),
    }
}

/// The unified hunk body for an expanded diff-viewer file: one line-numbered row per diff line,
/// reusing the EXACT wave0 conv-diff-card tokens — additions in `ADD_FG` on `ADD_BG`, deletions in
/// `DEL_FG` on `DEL_BG`, context neutral — with the line number (`LNUM`) at the origin, the
/// `+`/`-`/space sign at origin+6, and the code at origin+8 (the shared diff row geometry).
fn diff_unified_hunk_lines(
    file: &FileDiff,
    width: usize,
    theme: &style::ThemeColors,
) -> Vec<Line<'static>> {
    let width = width.max(1);
    let base = Style::default().bg(theme.background);
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut shown = 0usize;
    for hunk in &file.hunks {
        let start = if hunk.new_len > 0 {
            hunk.new_start
        } else {
            hunk.old_start
        };
        for (i, dline) in hunk.lines.iter().enumerate() {
            if shown >= MAX_DIFF_BODY_ROWS {
                lines.push(Line::from(vec![
                    Span::raw(" ".repeat(DIFF_HUNK_ORIGIN)),
                    Span::styled(
                        format!("{} more diff lines", crate::glyphs::ELLIPSIS),
                        Style::default().fg(theme.text_muted),
                    ),
                ]));
                return lines;
            }
            let (sign, fg, row_bg) = match dline.kind {
                crate::diff::DiffLineKind::Added => ('+', style::ADD_FG, Some(style::ADD_BG)),
                crate::diff::DiffLineKind::Removed => ('-', style::DEL_FG, Some(style::DEL_BG)),
                crate::diff::DiffLineKind::Context | crate::diff::DiffLineKind::Header => {
                    (' ', theme.text_dim, None)
                }
            };
            let row_bg_color = row_bg.unwrap_or(theme.background);
            let row_style = Style::default().fg(fg).bg(row_bg_color);
            let mut cells: Vec<(char, Style)> = vec![(' ', base); width];
            if row_bg.is_some() {
                // Paint the whole row background so the add/delete bar spans the panel width.
                for cell in cells.iter_mut() {
                    *cell = (' ', row_style);
                }
            }
            let origin = DIFF_HUNK_ORIGIN;
            let lnum = format!("{:>3}", start + i);
            place_segments(
                &mut cells,
                origin,
                &[(lnum, Style::default().fg(style::LNUM).bg(row_bg_color))],
            );
            place_segments(&mut cells, origin + 6, &[(sign.to_string(), row_style)]);
            let code_col = origin + 8;
            let code_room = width.saturating_sub(code_col + 1).max(1);
            let code = clip_text(&dline.content, code_room);
            place_segments(&mut cells, code_col, &[(code, row_style)]);
            lines.push(cells_to_line(&cells));
            shown += 1;
        }
    }
    lines
}

/// The split (before | after) hunk body for an expanded diff-viewer file: the old side (context +
/// deletions in `DEL_FG`/`DEL_BG`) in the left half and the new side (context + additions in
/// `ADD_FG`/`ADD_BG`) in the right half, reusing the same diff tokens as the unified body. Falls
/// back to the unified body when the panel is too narrow to split.
fn diff_split_hunk_lines(
    file: &FileDiff,
    width: usize,
    theme: &style::ThemeColors,
) -> Vec<Line<'static>> {
    let width = width.max(1);
    let origin = DIFF_HUNK_ORIGIN;
    let half = width.saturating_sub(origin).saturating_sub(1) / 2;
    if half < 6 {
        return diff_unified_hunk_lines(file, width, theme);
    }
    let left_x = origin;
    let right_x = origin + half + 1;
    let base = Style::default().bg(theme.background);
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut shown = 0usize;
    for row in file.split_rows() {
        if row.kind == crate::diff::DiffLineKind::Header {
            continue;
        }
        if shown >= MAX_DIFF_BODY_ROWS {
            break;
        }
        let mut cells: Vec<(char, Style)> = vec![(' ', base); width];
        paint_split_half(&mut cells, left_x, half, &row.left, row.kind, true, theme);
        paint_split_half(&mut cells, right_x, half, &row.right, row.kind, false, theme);
        lines.push(cells_to_line(&cells));
        shown += 1;
    }
    lines
}

/// Paint one half of a split diff row: fill `[x, x+half)` with the side's diff background (DEL for a
/// deletion on the left, ADD for an addition on the right, none for context) and place the
/// `+`/`-`/space sign + clipped code text on it, reusing the wave0 diff tokens.
fn paint_split_half(
    cells: &mut [(char, Style)],
    x: usize,
    half: usize,
    text: &str,
    kind: crate::diff::DiffLineKind,
    left: bool,
    theme: &style::ThemeColors,
) {
    let (sign, fg, bg) = match (kind, left) {
        (crate::diff::DiffLineKind::Removed, true) => ('-', style::DEL_FG, Some(style::DEL_BG)),
        (crate::diff::DiffLineKind::Added, false) => ('+', style::ADD_FG, Some(style::ADD_BG)),
        (crate::diff::DiffLineKind::Context, _) => (' ', theme.text_dim, None),
        // A deletion has no right side and an addition no left side: leave an empty gutter.
        _ => return,
    };
    let bg_color = bg.unwrap_or(theme.background);
    let style = Style::default().fg(fg).bg(bg_color);
    if bg.is_some() {
        let stop = (x + half).min(cells.len());
        for cell in cells.iter_mut().take(stop).skip(x) {
            *cell = (' ', style);
        }
    }
    place_segments(cells, x, &[(sign.to_string(), style)]);
    let code_room = half.saturating_sub(2).max(1);
    let code = clip_text(text, code_room);
    place_segments(cells, x + 2, &[(code, style)]);
}

/// Render the open modal dialog through the shared rich modal renderer: a centered card over the
/// dimmed session, a rounded dim-orange border with an AMBER title, an inverse selection bar with a
/// `›` marker, a footer key-hint row, scrolling, and an explicit no-results state. Moving the
/// selection plays a brief highlight transition when animations are enabled.
fn draw_dialog(frame: &mut Frame, app: &App, area: Rect, theme: &style::ThemeColors) {
    let Some(dialog) = app.dialog() else {
        return;
    };
    if area.width < 8 || area.height < 6 {
        return;
    }

    // Rich pickers (session/model/agent/skill/variant) render structured rows (key + desc +
    // right-aligned meta) supplied by the app with a fixed key column so descriptions line up; only
    // session/model add a `›` search row that filters in place.
    let rich = matches!(
        dialog.kind,
        DialogKind::Sessions
            | DialogKind::Models
            | DialogKind::Agents
            | DialogKind::Skills
            | DialogKind::Variants
            | DialogKind::Themes
            | DialogKind::Subagents
            | DialogKind::Fork
            | DialogKind::Export
    );
    let searchable = matches!(dialog.kind, DialogKind::Sessions | DialogKind::Models);

    // The session picker, when renaming, shows the in-progress new title as an in-frame input row.
    let rename = (dialog.kind == DialogKind::Sessions)
        .then(|| app.session_rename())
        .flatten();

    let (rows, search, key_width, key_color): (
        Vec<crate::modal::ModalRow>,
        Option<crate::modal::ModalSearch>,
        Option<usize>,
        Color,
    ) = if rich {
        let search = (searchable && rename.is_none()).then(|| crate::modal::ModalSearch {
            query: app.picker_search_query().to_owned(),
            placeholder: "type to filter…".to_owned(),
            caret_when_empty: false,
        });
        // Agents/variants/subagents/fork/export use an AMBER key; the other rich pickers use
        // BRIGHT keys.
        let key_color = match dialog.kind {
            DialogKind::Agents
            | DialogKind::Variants
            | DialogKind::Subagents
            | DialogKind::Fork
            | DialogKind::Export => theme.secondary,
            _ => theme.text_bright,
        };
        (
            app.picker_rows().to_vec(),
            search,
            app.picker_key_width(),
            key_color,
        )
    } else {
        (
            dialog
                .items
                .iter()
                .map(|item| crate::modal::ModalRow::new(item.clone()))
                .collect(),
            None,
            None,
            theme.text,
        )
    };

    let hints = dialog_hints(dialog.kind, rename.is_some());

    let empty = if rows.is_empty() {
        Some(
            dialog
                .empty
                .clone()
                .unwrap_or_else(|| "(nothing to show)".to_owned()),
        )
    } else {
        None
    };

    // Some dialogs carry a right-aligned footer note: the theme picker advertises its live preview,
    // the fork dialog notes it creates a new branch, and the export dialog shows its destination.
    let note = match dialog.kind {
        DialogKind::Themes => Some("live preview".to_owned()),
        DialogKind::Fork => Some("new branch".to_owned()),
        DialogKind::Export => Some("→ ~/cortex/exports".to_owned()),
        _ => None,
    };

    let width = dialog_width(
        &rows,
        key_width,
        &dialog.title,
        &hints,
        note.as_deref(),
        rename,
        area.width,
    );

    let input = rename.map(|text| crate::modal::ModalInput {
        prefix: "Rename: ".to_owned(),
        value: text.to_owned(),
    });

    let spec = crate::modal::ModalSpec {
        title: dialog.title.clone(),
        rows,
        selected: dialog.selected(),
        hints,
        note,
        search,
        input,
        empty,
        width,
        key_width,
        key_color,
    };

    crate::modal::render_modal(frame, area, &spec, theme, app.selection_anim_state());
}

/// Render the `/rename` dialog: a centered card (rounded `ORANGE_D` border + AMBER title) over the
/// dimmed conversation, showing the current session title in straight quotes, an ORANGE-bordered
/// input box holding the new title with a `▏` caret, and a `↵ save   esc cancel` footer.
fn draw_rename(frame: &mut Frame, app: &App, area: Rect, theme: &style::ThemeColors) {
    let Some(dialog) = app.rename_dialog() else {
        return;
    };
    if area.width < 12 || area.height < 9 {
        return;
    }

    let width = 60u16.min(area.width.saturating_sub(2)).max(30);
    let height = 9u16;
    let popup = centered_rect(width, height, area);

    let block = Block::default()
        .style(Style::default().bg(theme.background))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border))
        .title(" Rename session ")
        .title_style(
            Style::default()
                .fg(theme.secondary)
                .add_modifier(Modifier::BOLD),
        );

    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);

    let content_x = popup.x + 2;
    let content_w = width.saturating_sub(4);

    // The `current "title"` prompt line (straight quotes only).
    let current_line = match &dialog.current_title {
        Some(title) => Line::from(vec![
            Span::styled("current ", Style::default().fg(theme.text_dim)),
            Span::styled(format!("\"{title}\""), Style::default().fg(theme.text)),
        ]),
        None => Line::from(vec![
            Span::styled("current ", Style::default().fg(theme.text_dim)),
            Span::styled("(untitled)", Style::default().fg(theme.text_muted)),
        ]),
    };
    frame.render_widget(
        Paragraph::new(current_line).style(Style::default().bg(theme.background)),
        Rect {
            x: content_x,
            y: popup.y + 2,
            width: content_w,
            height: 1,
        },
    );

    // The ORANGE-bordered input box holding the new title with a trailing `▏` caret.
    let input_rect = Rect {
        x: content_x,
        y: popup.y + 4,
        width: content_w,
        height: 3,
    };
    let input_block = Block::default()
        .style(Style::default().bg(theme.background))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.primary));
    let input_inner = input_block.inner(input_rect);
    frame.render_widget(input_block, input_rect);

    let field_width = input_inner.width.saturating_sub(1) as usize;
    let shown: String = {
        let chars: Vec<char> = dialog.buffer.chars().collect();
        if chars.len() > field_width {
            chars[chars.len() - field_width..].iter().collect()
        } else {
            dialog.buffer.clone()
        }
    };
    let input_line = Line::from(vec![
        Span::styled(shown, Style::default().fg(theme.text_bright)),
        Span::styled(crate::glyphs::CURSOR, Style::default().fg(theme.primary)),
    ]);
    frame.render_widget(
        Paragraph::new(input_line).style(Style::default().bg(theme.background)),
        input_inner,
    );

    // The footer key-hint row: keys in dim orange, labels in faint grey.
    let footer = Line::from(vec![
        Span::styled(crate::glyphs::KEY_ENTER, Style::default().fg(theme.border)),
        Span::styled(" save", Style::default().fg(theme.text_muted)),
        Span::styled("   ", Style::default().fg(theme.text_muted)),
        Span::styled("esc", Style::default().fg(theme.border)),
        Span::styled(" cancel", Style::default().fg(theme.text_muted)),
    ]);
    frame.render_widget(
        Paragraph::new(footer).style(Style::default().bg(theme.background)),
        Rect {
            x: content_x,
            y: popup.y + 7,
            width: content_w,
            height: 1,
        },
    );
}

/// Draw a note on the top border of a centered dialog, right-aligned in the dim-orange border tone,
/// matching the title-in-border note style of the shared full-screen view panels.
fn dialog_note(frame: &mut Frame, popup: Rect, note: &str, theme: &style::ThemeColors) {
    let text = format!(" {note} ");
    let len = text.chars().count() as u16;
    let max = popup.width.saturating_sub(2);
    if len == 0 || len > max {
        return;
    }
    let note_area = Rect {
        x: popup.x + popup.width - 1 - len,
        y: popup.y,
        width: len,
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            text,
            Style::default().fg(theme.border).bg(theme.background),
        ))),
        note_area,
    );
}

/// The `/compact` dialog: a centered rounded card (over the dimmed conversation) titled `Compact
/// conversation` with a `frees context` note in the top border, showing the before/after context
/// comparison as key/value rows and the kept-summary bullets, with a `↵ compact   esc cancel`
/// footer. All colors come from theme tokens and glyphs from [`crate::glyphs`].
fn draw_compact(frame: &mut Frame, app: &App, area: Rect, theme: &style::ThemeColors) {
    let Some(dialog) = app.compact_dialog() else {
        return;
    };
    if area.width < 24 || area.height < 12 {
        return;
    }
    let width = 64u16.min(area.width.saturating_sub(2)).max(44);
    let height = 14u16.min(area.height.saturating_sub(2)).max(12);
    let popup = centered_rect(width, height, area);

    let block = Block::default()
        .style(Style::default().bg(theme.background))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border))
        .title(" Compact conversation ")
        .title_style(
            Style::default()
                .fg(theme.secondary)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);
    dialog_note(frame, popup, "frees context", theme);

    let content_x = popup.x + 2;
    let content_w = width.saturating_sub(4);
    let label_w = 10usize;
    let dim = Style::default().fg(theme.text_dim);
    let faint = Style::default().fg(theme.text_muted);
    let bright = Style::default().fg(theme.text_bright);
    let amber = Style::default().fg(theme.secondary);
    let txt = Style::default().fg(theme.text);
    let orange_d = Style::default().fg(theme.border);

    let kv = |label: &str, value: Vec<Span<'static>>| -> Line<'static> {
        let pad = label_w.saturating_sub(label.chars().count());
        let mut out = vec![Span::styled(format!("{label}{}", " ".repeat(pad)), faint)];
        out.extend(value);
        Line::from(out)
    };
    let sep = || Span::styled(format!("  {}  ", crate::glyphs::SEPARATOR), faint);

    let mut lines: Vec<Line<'static>> = vec![
        Line::from(Span::styled(
            "Summarize older messages to reclaim context window.",
            dim,
        )),
        Line::from(""),
        kv(
            "before",
            vec![
                Span::styled(dialog.before_tokens.clone(), bright),
                sep(),
                Span::styled(dialog.before_detail.clone(), dim),
            ],
        ),
        kv(
            "after",
            vec![
                Span::styled(dialog.after_tokens.clone(), amber),
                sep(),
                Span::styled(dialog.after_detail.clone(), dim),
            ],
        ),
        Line::from(""),
    ];
    for (i, keep) in dialog.keeps.iter().enumerate() {
        let label = if i == 0 { "keeps" } else { "" };
        lines.push(kv(
            label,
            vec![
                Span::styled(format!("{} ", crate::glyphs::BULLET), orange_d),
                Span::styled(keep.clone(), txt),
            ],
        ));
    }

    let body_area = Rect {
        x: content_x,
        y: popup.y + 1,
        width: content_w,
        height: height.saturating_sub(3),
    };
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.background)),
        body_area,
    );

    let footer = Line::from(vec![
        Span::styled(crate::glyphs::KEY_ENTER, orange_d),
        Span::styled(" compact", faint),
        Span::styled("   ", faint),
        Span::styled("esc", orange_d),
        Span::styled(" cancel", faint),
    ]);
    frame.render_widget(
        Paragraph::new(footer).style(Style::default().bg(theme.background)),
        Rect {
            x: content_x,
            y: popup.y + height - 2,
            width: content_w,
            height: 1,
        },
    );
}

/// The `/init` dialog: a centered rounded card (over the dimmed conversation) titled `Initialize
/// project` with an `AGENTS.md` note in the top border, showing the real project scan log — done
/// steps marked `✓` in OKG green, the active/pending step marked `›` in orange — with a `↵ create
/// e edit first   esc cancel` footer. Colors come from theme tokens and glyphs from [`crate::glyphs`].
fn draw_init(frame: &mut Frame, app: &App, area: Rect, theme: &style::ThemeColors) {
    let Some(dialog) = app.init_dialog() else {
        return;
    };
    if area.width < 24 || area.height < 12 {
        return;
    }
    let width = 66u16.min(area.width.saturating_sub(2)).max(44);
    let height = 16u16.min(area.height.saturating_sub(2)).max(12);
    let popup = centered_rect(width, height, area);

    let block = Block::default()
        .style(Style::default().bg(theme.background))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border))
        .title(" Initialize project ")
        .title_style(
            Style::default()
                .fg(theme.secondary)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);
    dialog_note(frame, popup, "AGENTS.md", theme);

    let content_x = popup.x + 2;
    let content_w = width.saturating_sub(4);
    let dim = Style::default().fg(theme.text_dim);
    let faint = Style::default().fg(theme.text_muted);
    let txt = Style::default().fg(theme.text);
    let okg = Style::default().fg(theme.success);
    let orange = Style::default().fg(theme.primary);
    let orange_d = Style::default().fg(theme.border);

    let mut lines: Vec<Line<'static>> = vec![
        Line::from(Span::styled(
            format!("Scanning {} {}", dialog.root, crate::glyphs::ELLIPSIS),
            dim,
        )),
        Line::from(""),
    ];
    for step in &dialog.steps {
        let (glyph, glyph_style) = if step.done {
            (crate::glyphs::MARK_DONE, okg)
        } else {
            (crate::glyphs::PROMPT, orange)
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{glyph} "), glyph_style),
            Span::styled(step.label.clone(), txt),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Writes build/test commands & conventions for the agent.",
        dim,
    )));

    let body_area = Rect {
        x: content_x,
        y: popup.y + 1,
        width: content_w,
        height: height.saturating_sub(3),
    };
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.background)),
        body_area,
    );

    let footer = Line::from(vec![
        Span::styled(crate::glyphs::KEY_ENTER, orange_d),
        Span::styled(" create", faint),
        Span::styled("   ", faint),
        Span::styled("e", orange_d),
        Span::styled(" edit first", faint),
        Span::styled("   ", faint),
        Span::styled("esc", orange_d),
        Span::styled(" cancel", faint),
    ]);
    frame.render_widget(
        Paragraph::new(footer).style(Style::default().bg(theme.background)),
        Rect {
            x: content_x,
            y: popup.y + height - 2,
            width: content_w,
            height: 1,
        },
    );
}

/// The footer key-hints for a dialog: the searchable pickers advertise their real actions with safe
/// key glyphs (`↵`, `⇥`, `⌫`), while every other dialog reuses its plain hint string.
fn dialog_hints(kind: DialogKind, renaming: bool) -> Vec<crate::modal::ModalHint> {
    use crate::modal::ModalHint;
    if renaming {
        return parse_hints(dialog_hint(kind, true));
    }
    match kind {
        DialogKind::Sessions => vec![
            ModalHint::new(crate::glyphs::KEY_ENTER, "open"),
            ModalHint::new("^N", "new"),
            ModalHint::new(crate::glyphs::KEY_DELETE, "delete"),
            ModalHint::new("esc", "close"),
        ],
        DialogKind::Models => vec![
            ModalHint::new(crate::glyphs::KEY_ENTER, "select"),
            ModalHint::new(crate::glyphs::KEY_TAB, "provider"),
            ModalHint::new("/", "filter"),
            ModalHint::new("esc", "close"),
        ],
        DialogKind::Agents => vec![
            ModalHint::new(crate::glyphs::KEY_ENTER, "use"),
            ModalHint::new("esc", "close"),
        ],
        DialogKind::Skills => vec![
            ModalHint::new("space", "toggle"),
            ModalHint::new(crate::glyphs::KEY_ENTER, "apply"),
            ModalHint::new("esc", "close"),
        ],
        DialogKind::Variants => vec![
            ModalHint::new(crate::glyphs::KEY_ENTER, "select"),
            ModalHint::new("esc", "close"),
        ],
        DialogKind::Themes => vec![
            ModalHint::new(
                format!("{}{}", crate::glyphs::KEY_UP, crate::glyphs::KEY_DOWN),
                "preview",
            ),
            ModalHint::new(crate::glyphs::KEY_ENTER, "apply"),
            ModalHint::new("esc", "close"),
        ],
        DialogKind::Subagents => vec![
            ModalHint::new(crate::glyphs::KEY_ENTER, "view"),
            ModalHint::new("^C", "cancel"),
            ModalHint::new("esc", "close"),
        ],
        DialogKind::Fork => vec![
            ModalHint::new(crate::glyphs::KEY_ENTER, "fork here"),
            ModalHint::new(
                format!("{}{}", crate::glyphs::KEY_UP, crate::glyphs::KEY_DOWN),
                "pick",
            ),
            ModalHint::new("esc", "cancel"),
        ],
        DialogKind::Export => vec![
            ModalHint::new(crate::glyphs::KEY_ENTER, "export"),
            ModalHint::new(
                format!("{}{}", crate::glyphs::KEY_UP, crate::glyphs::KEY_DOWN),
                "format",
            ),
            ModalHint::new("esc", "cancel"),
        ],
        _ => parse_hints(dialog_hint(kind, false)),
    }
}

/// The clamped modal width for a rich picker: room for the marker, fixed key column, description,
/// and the right-aligned meta column, but never wider than the available area.
fn dialog_width(
    rows: &[crate::modal::ModalRow],
    key_width: Option<usize>,
    title: &str,
    hints: &[crate::modal::ModalHint],
    note: Option<&str>,
    rename: Option<&str>,
    area_width: u16,
) -> u16 {
    let key_col = key_width.unwrap_or_else(|| {
        rows.iter()
            .map(|row| row.key.chars().count())
            .max()
            .unwrap_or(0)
    });
    let max_desc = rows
        .iter()
        .filter_map(|row| row.desc.as_ref())
        .map(|desc| desc.chars().count())
        .max()
        .unwrap_or(0);
    let max_meta = rows
        .iter()
        .filter_map(|row| row.meta.as_ref())
        .map(|meta| meta.chars().count())
        .max()
        .unwrap_or(0);
    // marker(2) + key + gap(2) + desc + gap(2) + meta + right border gap(1).
    let content = 2 + key_col + 2 + max_desc + 2 + max_meta + 1;
    let hint_w: usize = hints
        .iter()
        .map(|hint| hint.key.chars().count() + hint.label.chars().count() + 4)
        .sum();
    // The footer must fit the hints plus any right-aligned note without collision.
    let footer_w = hint_w + note.map_or(0, |text| text.chars().count() + 3);
    let title_w = title.chars().count() + 4;
    let rename_w = rename.map_or(0, |text| text.chars().count() + 12);
    // +2 for the left/right borders.
    let width = content.max(footer_w + 2).max(title_w).max(rename_w) + 2;
    (width as u16).min(area_width)
}

/// Split a `key label · key label` hint string into structured [`crate::modal::ModalHint`] pairs
/// (each segment's first token is the key glyph, the remainder is the action label).
fn parse_hints(hint: &str) -> Vec<crate::modal::ModalHint> {
    hint.split('·')
        .filter_map(|segment| {
            let segment = segment.trim();
            if segment.is_empty() {
                return None;
            }
            match segment.split_once(' ') {
                Some((key, label)) => Some(crate::modal::ModalHint::new(key, label.trim())),
                None => Some(crate::modal::ModalHint::new(segment, "")),
            }
        })
        .collect()
}

/// One permission option: its label, its right-hand description, and whether it is the destructive
/// `Deny` choice (rendered in red).
struct PermissionOption {
    label: &'static str,
    desc: &'static str,
    deny: bool,
}

/// Render the interactive permission prompt (`perm_permission.png`): a centered rounded modal over a
/// faded backdrop. The top border embeds `Permission required` (AMBER) with a right-edge context tag
/// (FAINT); the body shows the requested command in a bordered `$`-prompt box, the real working
/// directory, and three selectable options (`Allow once` [selected], `Allow for this session`,
/// `Deny` in red), plus the `↵ choose · ↑↓ move · esc deny` key hints.
fn draw_permission(frame: &mut Frame, app: &App, area: Rect, theme: &style::ThemeColors) {
    let Some((tool, subject, selected, feedback)) = app.permission_prompt() else {
        return;
    };
    if area.width < 12 || area.height < 8 {
        return;
    }

    let is_question = tool == "question";
    let is_shell = tool == "bash";
    let tag = if is_shell {
        "shell".to_owned()
    } else {
        tool.to_owned()
    };
    let prompt_text = if is_question {
        "the agent asks:"
    } else if is_shell {
        "the agent wants to run a command:"
    } else {
        "the agent wants to run a tool:"
    };
    let options = if is_question {
        [
            PermissionOption { label: "Submit once", desc: "answer just this question", deny: false },
            PermissionOption { label: "Submit for this session", desc: "remember this answer", deny: false },
            PermissionOption { label: "Cancel", desc: "reject and tell the agent why", deny: true },
        ]
    } else {
        [
            PermissionOption { label: "Allow once", desc: "run just this command", deny: false },
            PermissionOption { label: "Allow for this session", desc: "don't ask again this session", deny: false },
            PermissionOption { label: "Deny", desc: "reject and tell the agent why", deny: true },
        ]
    };

    // Column geometry: the `›` marker sits at col 0, labels at col 2, and the descriptions align in
    // a fixed column two cells past the widest label (a right-hand description column).
    let label_col = 2usize;
    let longest_label = options.iter().map(|o| o.label.chars().count()).max().unwrap_or(0);
    let desc_col = label_col + longest_label + 2;
    let longest_desc = options.iter().map(|o| o.desc.chars().count()).max().unwrap_or(0);

    let cwd = crate::motd::current_dir_display();
    let cwd_line = format!("in {cwd}");
    let cmd_lead = if is_shell { 2 } else { 0 }; // `$ `
    let move_keys = format!("{}{}", glyphs::KEY_UP, glyphs::KEY_DOWN);
    let footer: [(String, &str); 3] = [
        (glyphs::KEY_ENTER.to_owned(), if is_question { "submit" } else { "choose" }),
        (move_keys, "move"),
        ("esc".to_owned(), if is_question { "cancel" } else { "deny" }),
    ];
    let footer_w: usize = footer
        .iter()
        .map(|(k, l)| k.chars().count() + 1 + l.chars().count())
        .sum::<usize>()
        + 3 * footer.len().saturating_sub(1);

    // Interior width: the widest of the option block, the command box, the prompt/cwd lines, and the
    // footer — floored to a comfortable modal width and capped at 90% of the canvas.
    let content = [
        desc_col + longest_desc,
        4 + cmd_lead + subject.chars().count() + 1,
        prompt_text.chars().count() + 2,
        cwd_line.chars().count() + 2,
        footer_w + 2,
    ]
    .into_iter()
    .max()
    .unwrap_or(60);
    let max_interior = ((area.width as usize) * 9 / 10).saturating_sub(2).max(40);
    let interior_width = content.clamp(60.min(max_interior), max_interior);

    let bg = theme.background;
    let sel_bg = theme.surface[3];
    let blank = || -> Vec<(char, Style)> {
        vec![(' ', Style::default().fg(theme.text).bg(bg)); interior_width]
    };

    let mut rows: Vec<Line<'static>> = Vec::new();
    rows.push(cells_to_line(&blank()));

    // Prompt line.
    let mut prompt_cells = blank();
    place_segments(
        &mut prompt_cells,
        2,
        &[(prompt_text.to_owned(), Style::default().fg(theme.text).bg(bg))],
    );
    rows.push(cells_to_line(&prompt_cells));
    rows.push(cells_to_line(&blank()));

    // Command box: a rounded ORANGE_D inner box holding `$ <command>` (`$` ORANGE, command BRIGHT).
    let box_indent = 2usize;
    let box_width = interior_width.saturating_sub(box_indent * 2).max(4);
    let border = Style::default().fg(theme.border).bg(bg);
    let mut top = blank();
    let top_seg = format!(
        "{}{}{}",
        glyphs::BOX_TOP_LEFT,
        glyphs::BOX_HORIZONTAL.repeat(box_width.saturating_sub(2)),
        glyphs::BOX_TOP_RIGHT
    );
    place_segments(&mut top, box_indent, &[(top_seg, border)]);
    rows.push(cells_to_line(&top));

    let mut mid = blank();
    place_segments(&mut mid, box_indent, &[(glyphs::BOX_VERTICAL.to_owned(), border)]);
    place_segments(
        &mut mid,
        box_indent + box_width - 1,
        &[(glyphs::BOX_VERTICAL.to_owned(), border)],
    );
    let mut cmd_col = box_indent + 2;
    if is_shell {
        place_segments(
            &mut mid,
            cmd_col,
            &[("$".to_owned(), Style::default().fg(theme.primary).bg(bg))],
        );
        cmd_col += 2;
    }
    let cmd_room = (box_indent + box_width - 1).saturating_sub(cmd_col).saturating_sub(1);
    let command = clip_to(subject, cmd_room);
    place_segments(
        &mut mid,
        cmd_col,
        &[(command, Style::default().fg(theme.text_bright).bg(bg))],
    );
    rows.push(cells_to_line(&mid));

    let mut bottom = blank();
    let bottom_seg = format!(
        "{}{}{}",
        glyphs::BOX_BOTTOM_LEFT,
        glyphs::BOX_HORIZONTAL.repeat(box_width.saturating_sub(2)),
        glyphs::BOX_BOTTOM_RIGHT
    );
    place_segments(&mut bottom, box_indent, &[(bottom_seg, border)]);
    rows.push(cells_to_line(&bottom));
    rows.push(cells_to_line(&blank()));

    // Working-directory context line (real cwd), dim.
    let mut cwd_cells = blank();
    place_segments(
        &mut cwd_cells,
        2,
        &[(clip_to(&cwd_line, interior_width.saturating_sub(3)), Style::default().fg(theme.text_dim).bg(bg))],
    );
    rows.push(cells_to_line(&cwd_cells));
    rows.push(cells_to_line(&blank()));

    // Option rows: the selected row is a solid SELBG inverse bar with a `›` ORANGE marker + bright
    // label; `Deny` is red in every state. Descriptions sit in the fixed right-hand column (FAINT).
    for (index, option) in options.iter().enumerate() {
        let is_selected = index == selected;
        let row_bg = if is_selected { sel_bg } else { bg };
        let mut cells: Vec<(char, Style)> =
            vec![(' ', Style::default().fg(theme.text).bg(row_bg)); interior_width];
        if is_selected {
            place_segments(
                &mut cells,
                0,
                &[(glyphs::SELECT_MARKER.to_owned(), Style::default().fg(theme.primary).bg(row_bg))],
            );
        }
        let mut label_style = if option.deny {
            Style::default().fg(theme.error).bg(row_bg)
        } else if is_selected {
            Style::default().fg(theme.text_bright).bg(row_bg)
        } else {
            Style::default().fg(theme.text).bg(row_bg)
        };
        if is_selected {
            label_style = label_style.add_modifier(Modifier::BOLD);
        }
        place_segments(&mut cells, label_col, &[(option.label.to_owned(), label_style)]);
        place_segments(
            &mut cells,
            desc_col,
            &[(
                clip_to(option.desc, interior_width.saturating_sub(desc_col + 1)),
                Style::default().fg(theme.text_muted).bg(row_bg),
            )],
        );
        rows.push(cells_to_line(&cells));
    }

    // Feedback capture line (Deny corrective note / question answer): shown only while feedback is
    // active, so the default Allow-once view matches the reference exactly.
    if is_question || selected == 2 {
        rows.push(cells_to_line(&blank()));
        let mut fb = blank();
        let label = if is_question { "answer: " } else { "reason: " };
        place_segments(
            &mut fb,
            2,
            &[
                (label.to_owned(), Style::default().fg(theme.secondary).bg(bg)),
                (feedback.to_owned(), Style::default().fg(theme.text).bg(bg)),
                (glyphs::CURSOR.to_owned(), Style::default().fg(theme.primary).bg(bg)),
            ],
        );
        rows.push(cells_to_line(&fb));
    }

    rows.push(cells_to_line(&blank()));

    // Footer key hints: key glyphs in dim orange, labels in faint grey.
    let mut footer_cells = blank();
    let mut footer_segs: Vec<(String, Style)> = Vec::new();
    for (i, (key, label)) in footer.iter().enumerate() {
        if i > 0 {
            footer_segs.push(("   ".to_owned(), Style::default().fg(theme.text_muted).bg(bg)));
        }
        footer_segs.push((key.clone(), Style::default().fg(theme.border).bg(bg)));
        footer_segs.push((" ".to_owned(), Style::default().fg(theme.text_muted).bg(bg)));
        footer_segs.push(((*label).to_owned(), Style::default().fg(theme.text_muted).bg(bg)));
    }
    place_segments(&mut footer_cells, 2, &footer_segs);
    rows.push(cells_to_line(&footer_cells));

    let width = (interior_width + 2) as u16;
    let height = (rows.len() + 2) as u16;
    let popup = centered_rect(width, height, area);

    let block = Block::default()
        .style(Style::default().bg(bg))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border))
        .title(" Permission required ")
        .title_style(
            Style::default()
                .fg(theme.secondary)
                .add_modifier(Modifier::BOLD),
        );
    let inner = block.inner(popup);

    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);
    frame.render_widget(
        Paragraph::new(rows).style(Style::default().fg(theme.text).bg(bg)),
        inner,
    );

    // The context tag rides the top border, right-aligned, like the shared view/panel note.
    let tag_text = format!(" {tag} ");
    let tag_len = tag_text.chars().count() as u16;
    if tag_len + 2 < popup.width {
        let tag_area = Rect {
            x: popup.x + popup.width - 1 - tag_len,
            y: popup.y,
            width: tag_len,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                tag_text,
                Style::default().fg(theme.text_muted).bg(bg),
            ))),
            tag_area,
        );
    }
}

/// Truncate `s` to at most `max` display columns, appending `…` when it is cut.
fn clip_to(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let count = s.chars().count();
    if count <= max {
        return s.to_owned();
    }
    let keep = max.saturating_sub(1);
    let mut out: String = s.chars().take(keep).collect();
    out.push_str(glyphs::ELLIPSIS);
    out
}

/// The footer hint for a dialog: the session picker advertises its switch/rename/delete actions
/// (and the commit/cancel keys while renaming); every other picker shows the plain navigation hint.
fn dialog_hint(kind: DialogKind, renaming: bool) -> &'static str {
    match (kind, renaming) {
        (DialogKind::Sessions, true) => "type a name · Enter to save · Esc to cancel",
        (DialogKind::Sessions, false) => {
            "↑/↓ move · Enter switch · r rename · d delete · Esc close"
        }
        (DialogKind::Fork, _) => "↑/↓ move · Enter fork here · Esc close",
        _ => "↑/↓ move · Enter select · Esc close",
    }
}

/// The title embedded in the command palette's border for `scope`: `Commands` at the root, with a
/// `· submenu` crumb for the model/skill/variant/subagent/queue submenus.
fn palette_scope_title(scope: crate::app::PaletteScope) -> String {
    use crate::app::PaletteScope;
    match scope {
        PaletteScope::Root => "Commands",
        PaletteScope::Models => "Commands · Models",
        PaletteScope::Skills => "Commands · Skills",
        PaletteScope::Variants => "Commands · Variants",
        PaletteScope::Subagents => "Commands · Subagents",
        PaletteScope::QueuedPrompts => "Commands · Queued prompts",
    }
    .to_owned()
}

/// Build the command palette's [`crate::modal::ModalSpec`]: a `Commands` modal with a search row
/// (`›` prompt + query + `▏` caret + underline rule), one `/name  description` row per matching
/// builtin, the inverse selection bar, and the `↑↓ move · ↵ run · esc close` footer. Reused by the
/// renderer and the rendering tests so both see the identical structure.
fn palette_modal_spec(app: &App, theme: &style::ThemeColors) -> crate::modal::ModalSpec {
    use crate::modal::{ModalHint, ModalRow, ModalSearch, ModalSpec};

    let scope = app
        .palette_scope()
        .unwrap_or(crate::app::PaletteScope::Root);
    let is_root = scope == crate::app::PaletteScope::Root;
    let title = palette_scope_title(scope);

    let rows: Vec<ModalRow> = app
        .palette_matches()
        .into_iter()
        .map(|command| {
            let mut desc = command.description;
            if let Some(reason) = command.disabled_reason {
                if !desc.is_empty() {
                    desc.push_str(" · ");
                }
                desc.push_str(&reason);
            }
            ModalRow {
                key: command.label,
                desc: (!desc.is_empty()).then_some(desc),
                meta: None,
                meta_color: None,
            }
        })
        .collect();

    let hints = vec![
        ModalHint::new(
            format!("{}{}", crate::glyphs::KEY_UP, crate::glyphs::KEY_DOWN),
            "move",
        ),
        ModalHint::new(crate::glyphs::KEY_ENTER, "run"),
        ModalHint::new("esc", if is_root { "close" } else { "back" }),
    ];

    let empty = rows
        .is_empty()
        .then(|| "no matching commands".to_owned());

    let search = Some(ModalSearch {
        query: app.palette_query().unwrap_or_default().to_owned(),
        placeholder: String::new(),
        caret_when_empty: true,
    });

    // Size to the widest row/footer/title but keep a reference-like floor so the modal stays a
    // stable, comfortable width as the query filters the list (u16::MAX defers the area clamp to
    // `render_modal`).
    let content = dialog_width(&rows, None, &title, &hints, None, None, u16::MAX);
    let width = content.max(60);

    ModalSpec {
        title,
        rows,
        selected: app.palette_selected(),
        hints,
        note: None,
        search,
        input: None,
        empty,
        width,
        key_width: None,
        key_color: theme.secondary,
    }
}

/// Render the command palette as a centered rich modal over the dimmed session: a `Commands` frame
/// with a searchable, fuzzy-filtered list of real builtin commands, an inverse selection bar, and a
/// `↑↓ move · ↵ run · esc close` footer. Running a highlighted row executes the real command.
fn draw_palette(frame: &mut Frame, app: &App, area: Rect, theme: &style::ThemeColors) {
    if app.palette_query().is_none() {
        return;
    }
    if area.width < 8 || area.height < 6 {
        return;
    }

    let spec = palette_modal_spec(app, theme);
    crate::modal::render_modal(frame, area, &spec, theme, app.selection_anim_state());
}

/// The geometry of an open sidebar split: the live conversation on the left and the sidebar box on
/// the right, wrapped in the shared `✦ Cortex Code · <panel>` chrome (header + rule + status bar).
#[derive(Debug, Clone, Copy)]
struct SidebarSplit {
    /// The left conversation region (`MAINX`/`MAINW`), below the chrome header + rule.
    conversation: Rect,
    /// The right sidebar box region (`SBX`/`SBW`), aligned with the conversation top.
    sidebar: Rect,
}

/// Compute the sidebar split geometry for `area` when the sidebar is open, or `None` when it is
/// closed or the terminal is too small to keep a usable conversation width.
///
/// At the reference 92×30 this yields `MAINX=2, MAINW=55` (the conversation), a single clean gap
/// column, then `SBX=58, SBW=32` (the sidebar) — with the header (row 0), rule (row 1), a bottom
/// gap row, and the full-width status bar (last row) reserved by the shared chrome.
fn sidebar_split(app: &App, area: Rect) -> Option<SidebarSplit> {
    if !app.sidebar_visible() || area.width < MIN_SPLIT_WIDTH || area.height < 10 {
        return None;
    }
    let sbx = area.x + area.width - SPLIT_RIGHT_MARGIN - SIDEBAR_PANEL_WIDTH;
    let mainx = area.x + SPLIT_LEFT_MARGIN;
    let mainw = sbx.saturating_sub(mainx + 1);
    // Body rows sit below the header (row 0) + rule (row 1); the sidebar box is `H-4` tall so a gap
    // row separates it from the full-width status bar the chrome paints on the last row.
    let body_y = area.y + 2;
    let body_height = area.height.saturating_sub(4);
    Some(SidebarSplit {
        conversation: Rect {
            x: mainx,
            y: body_y,
            width: mainw,
            height: body_height,
        },
        sidebar: Rect {
            x: sbx,
            y: body_y,
            width: SIDEBAR_PANEL_WIDTH,
            height: body_height,
        },
    })
}

/// Render the right-hand sidebar box for the active panel: a rounded `ORANGE_D` frame with an AMBER
/// title-in-border and a right-aligned FAINT note, a faint subtitle row, a ghost-grey divider rule,
/// and the panel body rows. The chrome (title/note/subtitle/rule) is this feature's responsibility;
/// the per-panel body content is real/dynamic session state. Colors come only from theme tokens and
/// glyphs only from the safe module, so the box stays brand-pure and tofu-free.
fn draw_sidebar(frame: &mut Frame, app: &App, area: Rect, theme: &style::ThemeColors) {
    if area.width < 8 || area.height < 4 {
        return;
    }
    let panel = app.sidebar_panel();

    // The rounded box + AMBER title-in-border + right-aligned FAINT note (reusing the shared view
    // panel chrome so the sidebar and full-screen views frame consistently).
    let note = app.sidebar_note();
    let inner = crate::view::render_panel(frame, area, panel.title(), Some(&note), theme);
    if inner.width < 3 || inner.height < 2 {
        return;
    }

    // Content is inset one column inside the box (the `SBX+2` content origin) and starts one row
    // below the top border, leaving a blank padding row under the title.
    let content = Rect {
        x: inner.x + 1,
        y: inner.y + 1,
        width: inner.width.saturating_sub(2),
        height: inner.height.saturating_sub(1),
    };
    if content.width == 0 || content.height == 0 {
        return;
    }
    let cw = content.width as usize;

    // Row 0: the faint subtitle/header. Row 1: the ghost divider rule. Rows 2+: the panel body.
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(content.height as usize);
    lines.push(Line::from(Span::styled(
        clip_columns(&app.sidebar_subtitle(), cw),
        Style::default().fg(theme.text_muted),
    )));
    lines.push(Line::from(Span::styled(
        glyphs::BOX_HORIZONTAL.repeat(cw),
        Style::default().fg(theme.ghost),
    )));
    match panel {
        crate::app::SidebarPanel::Context => sidebar_context_body(app, theme, cw, &mut lines),
        crate::app::SidebarPanel::Files => {
            // The tree body gets the rows below the subtitle + divider; scroll it to keep the
            // focused node visible when the tree is taller than the panel.
            let body_rows = (content.height as usize).saturating_sub(2);
            sidebar_files_body(app, theme, cw, body_rows, &mut lines);
        }
        crate::app::SidebarPanel::Lsp => {
            sidebar_lsp_body(app.sidebar_lsp_files(), theme, cw, &mut lines);
        }
        crate::app::SidebarPanel::Mcp => {
            let rows = app.mcp_view_rows();
            sidebar_mcp_body(&rows, theme, cw, &mut lines);
        }
        crate::app::SidebarPanel::Todo => {
            sidebar_todo_body(&app.sidebar_todo_items(), theme, cw, &mut lines);
        }
        crate::app::SidebarPanel::Subagents => {
            sidebar_subagents_body(&app.sidebar_subagent_entries(), theme, cw, &mut lines);
        }
        crate::app::SidebarPanel::Queue => {
            sidebar_queue_body(&app.sidebar_queue_entries(), theme, cw, &mut lines);
        }
    }

    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.background)),
        content,
    );
}

/// Append the styled `context` panel body: one row per context file (`›` ORANGE_D marker + name +
/// a right-aligned DIM token count, aligned to a common column), then a ghost rule and the
/// `@ add files to context` affordance (`@` in ORANGE).
fn sidebar_context_body(
    app: &App,
    theme: &style::ThemeColors,
    cw: usize,
    lines: &mut Vec<Line<'static>>,
) {
    let files = app.context_files();
    for file in &files {
        // Right-align the token count to the panel edge, leaving the name a left-aligned column
        // and at least one space of gap before the count.
        let marker_cols = 2; // `› `
        let count_cols = file.tokens.chars().count();
        let name_budget = cw.saturating_sub(marker_cols + count_cols + 1);
        let name = clip_plain(&file.name, name_budget.max(1));
        let used = marker_cols + name.chars().count() + count_cols;
        let gap = cw.saturating_sub(used).max(1);
        lines.push(Line::from(vec![
            Span::styled(
                format!("{} ", glyphs::PROMPT),
                Style::default().fg(theme.border),
            ),
            Span::styled(name, Style::default().fg(theme.text)),
            Span::styled(" ".repeat(gap), Style::default()),
            Span::styled(file.tokens.clone(), Style::default().fg(theme.text_dim)),
        ]));
    }
    // A blank spacer, a ghost rule, then the add-files affordance under it.
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        glyphs::BOX_HORIZONTAL.repeat(cw),
        Style::default().fg(theme.ghost),
    )));
    lines.push(Line::from(vec![
        Span::styled("@ ", Style::default().fg(theme.primary)),
        Span::styled(
            clip_plain("add files to context", cw.saturating_sub(2)),
            Style::default().fg(theme.text_dim),
        ),
    ]));
}

/// Append the styled `files` collapsible-tree body: each row indented by `depth*2`, directories
/// carrying a heavy `▼`/`▶` disclosure glyph (ORANGE_D) and modified files an inline `●` marker
/// (AMBER); the focused row is drawn as an inverse selection bar once navigation is active.
fn sidebar_files_body(
    app: &App,
    theme: &style::ThemeColors,
    cw: usize,
    body_rows: usize,
    lines: &mut Vec<Line<'static>>,
) {
    let rows = app.file_tree_rows();
    if rows.is_empty() {
        lines.push(Line::from(Span::styled(
            clip_plain("No files to show.", cw),
            Style::default().fg(theme.text_muted),
        )));
        return;
    }
    let cursor = app.files_cursor();
    let nav = app.files_nav_active();
    // Scroll the window so the focused node stays visible when the tree overflows the panel.
    let scroll = if body_rows > 0 && cursor >= body_rows {
        cursor - body_rows + 1
    } else {
        0
    };
    for (index, row) in rows.iter().enumerate().skip(scroll) {
        let selected = nav && index == cursor;
        let indent = "  ".repeat(row.depth);
        let (marker, marker_style, name_style) = match row.kind {
            crate::app::FileTreeKind::DirExpanded => (
                glyphs::DISCLOSURE_EXPANDED,
                Style::default().fg(theme.border),
                Style::default().fg(theme.text),
            ),
            crate::app::FileTreeKind::DirCollapsed => (
                glyphs::DISCLOSURE_COLLAPSED,
                Style::default().fg(theme.border),
                Style::default().fg(theme.text),
            ),
            crate::app::FileTreeKind::FileModified => (
                glyphs::DOT_FILLED,
                Style::default().fg(theme.secondary),
                Style::default().fg(theme.text_bright),
            ),
            crate::app::FileTreeKind::File => (
                " ",
                Style::default(),
                Style::default().fg(theme.text),
            ),
        };
        // Fit `<indent><marker> <name>` in the content width without a premature `…`: only the
        // name (never the indent/glyph) is clipped, and only when it genuinely overruns.
        let prefix_cols = indent.chars().count() + 2;
        let name = clip_plain(&row.name, cw.saturating_sub(prefix_cols).max(1));
        let mut spans = vec![
            Span::styled(indent, Style::default()),
            Span::styled(format!("{marker} "), marker_style),
            Span::styled(name, name_style),
        ];
        if selected {
            let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
            if used < cw {
                spans.push(Span::styled(" ".repeat(cw - used), Style::default()));
            }
            for span in &mut spans {
                span.style = span.style.bg(theme.surface[3]);
            }
        }
        lines.push(Line::from(spans));
    }
}

/// Append the styled `lsp` panel body: one `●` file row per diagnosed file (dot colored by the
/// file's worst severity — ERR red / AMBER warning / neutral clean — and the filename in BRIGHT),
/// followed by its diagnostics as `<line#> <E|W> <message>` rows (`E` ERR-red, `W` AMBER); a clean
/// file adds a green `○ no problems` row (VAL-SIDE-020/021/022/023).
fn sidebar_lsp_body(
    groups: &[crate::app::LspFileGroup],
    theme: &style::ThemeColors,
    cw: usize,
    lines: &mut Vec<Line<'static>>,
) {
    if groups.is_empty() {
        lines.push(Line::from(Span::styled(
            clip_columns("No recent LSP diagnostics.", cw),
            Style::default().fg(theme.text_muted),
        )));
        return;
    }
    for group in groups {
        // `● <filename>` — the dot's color is the file's worst diagnostic severity; a clean file
        // gets a neutral (FAINT) dot so it reads distinct from the green `○ no problems` row below.
        let dot_color = match group.severity() {
            crate::app::LspFileSeverity::Error => theme.error,
            crate::app::LspFileSeverity::Warning => theme.warning,
            crate::app::LspFileSeverity::Clean => theme.text_muted,
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("{} ", glyphs::DOT_FILLED),
                Style::default().fg(dot_color),
            ),
            Span::styled(
                clip_plain(&group.file, cw.saturating_sub(2).max(1)),
                Style::default().fg(theme.text_bright),
            ),
        ]));
        if group.diagnostics.is_empty() {
            // A checked-clean file: a hollow green dot + `no problems`, both in OKG success green.
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{} ", glyphs::DOT_HOLLOW),
                    Style::default().fg(theme.success),
                ),
                Span::styled(
                    clip_plain("no problems", cw.saturating_sub(2).max(1)),
                    Style::default().fg(theme.success),
                ),
            ]));
            continue;
        }
        for diagnostic in &group.diagnostics {
            // `<line#>  <E|W> <message>`: a right-aligned dim line-number column, the severity
            // letter (ERR red / AMBER warning), then the message in TXT.
            let (letter, letter_color) = if diagnostic.is_error {
                ("E", theme.error)
            } else {
                ("W", theme.warning)
            };
            let number = format!("{:>3}", diagnostic.line);
            let prefix_cols = 1 + number.chars().count() + 2 + 1 + 1; // " " + num + "  " + letter + " "
            let message = clip_plain(&diagnostic.message, cw.saturating_sub(prefix_cols).max(1));
            lines.push(Line::from(vec![
                Span::styled(format!(" {number}  "), Style::default().fg(theme.text_dim)),
                Span::styled(letter.to_owned(), Style::default().fg(letter_color)),
                Span::styled(format!(" {message}"), Style::default().fg(theme.text)),
            ]));
        }
    }
}

/// Append the styled `mcp` mini-panel body: one row per configured/connected MCP server, each a
/// leading `●` status dot + server name + a right-aligned status word, with the dot and word
/// colored by the server's connection state (ready=green, starting=amber, error=red, else faint)
/// (VAL-SIDE-024/025).
fn sidebar_mcp_body(
    rows: &[crate::view::McpServerRow],
    theme: &style::ThemeColors,
    cw: usize,
    lines: &mut Vec<Line<'static>>,
) {
    if rows.is_empty() {
        lines.push(Line::from(Span::styled(
            clip_columns("No MCP servers configured.", cw),
            Style::default().fg(theme.text_muted),
        )));
        return;
    }
    for row in rows {
        let color = row.tone.color(theme);
        let status = clip_plain(&row.status_word, cw.saturating_sub(3).max(1));
        let status_cols = status.chars().count();
        // `● <name>` on the left, `<status>` flush right, ≥1 column of gap between them.
        let name_budget = cw.saturating_sub(2 + status_cols + 1).max(1);
        let name = clip_plain(&row.name, name_budget);
        let used = 2 + name.chars().count() + status_cols;
        let gap = cw.saturating_sub(used).max(1);
        lines.push(Line::from(vec![
            Span::styled(
                format!("{} ", glyphs::DOT_FILLED),
                Style::default().fg(color),
            ),
            Span::styled(name, Style::default().fg(theme.text)),
            Span::styled(" ".repeat(gap), Style::default()),
            Span::styled(status, Style::default().fg(color)),
        ]));
    }
}

/// Append the styled `todo` panel body: one row per task in three visually distinct states — a
/// completed task as a green `✓` (OKG) with DIM text, an in-progress task as an ORANGE `▶` (the
/// heavy triangle, never the thin `▸`) with BRIGHT text, and a pending task as a ghost-grey `○`
/// with normal TXT text. A cancelled task reads as a FAINT `✗` so it stays distinct
/// (VAL-SIDE-027).
fn sidebar_todo_body(
    items: &[protocol::TodoItem],
    theme: &style::ThemeColors,
    cw: usize,
    lines: &mut Vec<Line<'static>>,
) {
    if items.is_empty() {
        lines.push(Line::from(Span::styled(
            clip_columns("No todo items for this session.", cw),
            Style::default().fg(theme.text_muted),
        )));
        return;
    }
    for item in items {
        let (marker, marker_color, text_color) = match item.status {
            protocol::TodoStatus::Completed => (glyphs::MARK_DONE, theme.success, theme.text_dim),
            protocol::TodoStatus::InProgress => (
                glyphs::DISCLOSURE_COLLAPSED,
                theme.primary,
                theme.text_bright,
            ),
            protocol::TodoStatus::Pending => (glyphs::DOT_HOLLOW, theme.ghost, theme.text),
            protocol::TodoStatus::Cancelled => {
                (glyphs::MARK_ERROR, theme.text_muted, theme.text_muted)
            }
        };
        // `<marker> <content>`: only the content is clipped, and only when it genuinely overruns
        // the panel interior (never a premature `…`).
        let content = clip_plain(&item.content, cw.saturating_sub(2).max(1));
        lines.push(Line::from(vec![
            Span::styled(format!("{marker} "), Style::default().fg(marker_color)),
            Span::styled(content, Style::default().fg(text_color)),
        ]));
    }
}

/// Append the styled `subagents` panel body: each child subagent is an `◆ name` header row (AMBER
/// diamond + BRIGHT name) with a right-aligned status word colored by state (running → ORANGE,
/// queued → FAINT, done → OKG green, error → ERR red), then a DIM task line indented under the
/// name, with a blank spacer between entries (the reference's *3 row spacing) (VAL-SIDE-029).
fn sidebar_subagents_body(
    rows: &[crate::app::SidebarSubagentRow],
    theme: &style::ThemeColors,
    cw: usize,
    lines: &mut Vec<Line<'static>>,
) {
    if rows.is_empty() {
        lines.push(Line::from(Span::styled(
            clip_columns("No subagents for this session.", cw),
            Style::default().fg(theme.text_muted),
        )));
        return;
    }
    for (index, row) in rows.iter().enumerate() {
        if index > 0 {
            // A blank spacer row separates entries (the reference's *3 row spacing).
            lines.push(Line::from(""));
        }
        let status_color = match row.state {
            crate::app::SubagentState::Running => theme.primary,
            crate::app::SubagentState::Queued => theme.text_muted,
            crate::app::SubagentState::Done => theme.success,
            crate::app::SubagentState::Error => theme.error,
        };
        let status = clip_plain(&row.status, cw.saturating_sub(3).max(1));
        let status_cols = status.chars().count();
        // `◆ <name>` on the left, the status word flush right, ≥1 column of gap between them.
        let name_budget = cw.saturating_sub(2 + status_cols + 1).max(1);
        let name = clip_plain(&row.name, name_budget);
        let used = 2 + name.chars().count() + status_cols;
        let gap = cw.saturating_sub(used).max(1);
        lines.push(Line::from(vec![
            Span::styled(
                format!("{} ", glyphs::DOT_DIAMOND),
                Style::default().fg(theme.secondary),
            ),
            Span::styled(name, Style::default().fg(theme.text_bright)),
            Span::styled(" ".repeat(gap), Style::default()),
            Span::styled(status, Style::default().fg(status_color)),
        ]));
        // The task description on the following line, indented two columns to align under the name.
        let task = clip_plain(&row.task, cw.saturating_sub(2).max(1));
        lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(task, Style::default().fg(theme.text_dim)),
        ]));
    }
}

/// Append the styled `queue` panel body: queued prompts as numbered rows. The run-next (first) row
/// uses a brighter ORANGE number + BRIGHT text; later rows use a dimmer ORANGE_D number + normal
/// TXT text. Prompt text that fits within the panel is rendered in full (no premature `…`)
/// (VAL-SIDE-031/032).
fn sidebar_queue_body(
    prompts: &[String],
    theme: &style::ThemeColors,
    cw: usize,
    lines: &mut Vec<Line<'static>>,
) {
    if prompts.is_empty() {
        lines.push(Line::from(Span::styled(
            clip_columns("No queued prompts.", cw),
            Style::default().fg(theme.text_muted),
        )));
        return;
    }
    for (index, prompt) in prompts.iter().enumerate() {
        let run_next = index == 0;
        let (number_color, text_color) = if run_next {
            (theme.primary, theme.text_bright)
        } else {
            (theme.border, theme.text)
        };
        let number = format!("{} ", index + 1);
        let prefix_cols = number.chars().count();
        // Only the prompt text is clipped, and only when it genuinely overruns the panel width.
        let text = clip_plain(prompt, cw.saturating_sub(prefix_cols).max(1));
        lines.push(Line::from(vec![
            Span::styled(number, Style::default().fg(number_color)),
            Span::styled(text, Style::default().fg(text_color)),
        ]));
    }
}

/// Clip `s` to at most `max` columns WITHOUT appending an ellipsis when it fits (used where a
/// premature `…` would be a defect); only overrunning text is hard-truncated.
fn clip_plain(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    s.chars().take(max).collect()
}

/// Clip `s` to at most `max` columns, appending a `…` when it is cut, so a sidebar row never
/// overruns the box interior.
fn clip_columns(s: &str, max: usize) -> String {
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

/// Fade every cell in `area` toward the page background so an open picker reads as a floating
/// overlay over a dimmed session: the transcript, composer, and status bar stay visible but recede
/// behind the modal. The modal itself is drawn afterwards (with `Clear`), so it is unaffected.
fn dim_backdrop(frame: &mut Frame, area: Rect, theme: &style::ThemeColors) {
    let bg = theme.background;
    let buffer = frame.buffer_mut();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(cell) = buffer.cell_mut((x, y)) {
                cell.fg = dim_color(cell.fg, bg);
                cell.bg = dim_color(cell.bg, bg);
            }
        }
    }
}

/// Blend `color` most of the way toward the backdrop `bg` (keeping a faint trace visible).
fn dim_color(color: Color, bg: Color) -> Color {
    const FADE: f32 = 0.62;
    match (color, bg) {
        (Color::Rgb(cr, cg, cb), Color::Rgb(br, bgc, bb)) => {
            let mix = |c: u8, b: u8| -> u8 {
                (f32::from(c) + (f32::from(b) - f32::from(c)) * FADE).round() as u8
            };
            Color::Rgb(mix(cr, br), mix(cg, bgc), mix(cb, bb))
        }
        _ => color,
    }
}

/// A rectangle of the given size centered within `area` (clamped to fit).
pub(crate) fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
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

/// Render the Working band on the first row of `area` (the second row is the gap to the composer).
/// The band is a `SOFT`-filled status row shown while a turn is in flight.
fn draw_working_band(frame: &mut Frame, app: &App, area: Rect, theme: &style::ThemeColors) {
    if area.height == 0 {
        return;
    }
    let band_row = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: 1,
    };
    let line = working_band_line(app, area.width as usize, theme);
    frame.render_widget(
        Paragraph::new(line).style(Style::default().bg(theme.soft)),
        band_row,
    );
}

/// Render the persistent plan-mode footer banner on the first row of `area` (the second row is the
/// gap to the composer). Shown while plan mode is active, directly above the composer.
fn draw_plan_banner(frame: &mut Frame, area: Rect, theme: &style::ThemeColors) {
    if area.height == 0 {
        return;
    }
    let banner_row = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: 1,
    };
    let line = plan_banner_line(area.width as usize, theme);
    frame.render_widget(
        Paragraph::new(line).style(Style::default().bg(theme.background)),
        banner_row,
    );
}

/// Build the plan-mode footer banner line: an AMBER `▌` edge + `plan mode` (AMBER) + a GHOST `·`
/// separator + a FAINT read-only note (`read-only — edits & shell paused.  ⇥ switch to build to
/// apply.`), inset by [`CONTENT_MARGIN`]. Amber hue keeps it distinct from the red `blocked` banner
/// (VAL-PERM-016/021). Every cell carries the explicit canvas background.
pub fn plan_banner_line(width: usize, theme: &style::ThemeColors) -> Line<'static> {
    let w = width.max(1);
    let base = Style::default().fg(theme.text).bg(theme.background);
    let mut cells: Vec<(char, Style)> = vec![(' ', base); w];
    let amber = Style::default().fg(theme.secondary).bg(theme.background);
    let faint = Style::default().fg(theme.text_muted).bg(theme.background);
    let ghost = Style::default().fg(theme.ghost).bg(theme.background);
    let segments: Vec<(String, Style)> = vec![
        (format!("{} ", glyphs::ACCENT_BAR), amber),
        ("plan mode".to_owned(), amber),
        (format!(" {} ", glyphs::SEPARATOR), ghost),
        (
            format!("read-only {} edits & shell paused.  ", glyphs::EM_DASH),
            faint,
        ),
        (
            format!("{} switch to build to apply.", glyphs::KEY_TAB),
            faint,
        ),
    ];
    place_segments(&mut cells, CONTENT_MARGIN, &segments);
    cells_to_line(&cells)
}

/// Build the Working band line: a breathing spinner (ORANGE) + `Working` (BRIGHT) + ` · ` (FAINT) +
/// a dynamic detail label (DIM), with a right-aligned `<N>s · esc to interrupt` (FAINT), all on the
/// `SOFT` band surface. Every cell carries the explicit `soft` background so the band reads as a
/// continuous surface.
pub fn working_band_line(app: &App, width: usize, theme: &style::ThemeColors) -> Line<'static> {
    let w = width.max(1);
    let soft = theme.soft;
    let base = Style::default().fg(theme.text).bg(soft);
    let mut cells: Vec<(char, Style)> = vec![(' ', base); w];

    // The breathing spinner uses the safe `· ✢ ✻ ✽` set (distinct from the tool-card rotation); a
    // single static frame is shown when not animating so the band still reads while busy.
    let glyph = app
        .spinner_glyph()
        .unwrap_or(crate::glyphs::SPINNER_BREATHE[0]);
    let left: Vec<(String, Style)> = vec![
        (format!("{glyph} "), Style::default().fg(theme.primary).bg(soft)),
        (
            "Working".to_owned(),
            Style::default()
                .fg(theme.text_bright)
                .bg(soft)
                .add_modifier(Modifier::BOLD),
        ),
        (" · ".to_owned(), Style::default().fg(theme.text_muted).bg(soft)),
        (
            app.working_detail(),
            Style::default().fg(theme.text_dim).bg(soft),
        ),
    ];
    place_segments(&mut cells, CONTENT_MARGIN, &left);

    // Right zone: a live elapsed timer + the interrupt hint, flush to the content margin.
    let right_text = format!(
        "{}s {} esc to interrupt",
        app.turn_elapsed_secs(),
        crate::glyphs::SEPARATOR
    );
    let right_w = right_text.chars().count();
    let right_start = w.saturating_sub(CONTENT_MARGIN + right_w);
    place_segments(
        &mut cells,
        right_start,
        &[(right_text, Style::default().fg(theme.text_muted).bg(soft))],
    );

    cells_to_line(&cells)
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
    // centered so its midpoint sits at the terminal's horizontal center. In plan mode the permission
    // word reads `read-only · plan` instead of `allow all commands` (VAL-PERM-017); in `!` shell mode
    // the autonomy head reads `manual` and the permission word reads `! shell mode` (VAL-PERM-024).
    let center: Vec<(String, Style)> = if app.shell_mode() && !app.plan_mode() {
        vec![
            ("manual".to_owned(), Style::default().fg(theme.secondary)),
            (" · ".to_owned(), Style::default().fg(theme.ghost)),
            (
                "! shell mode".to_owned(),
                Style::default().fg(theme.text_dim),
            ),
        ]
    } else {
        let permission = if app.plan_mode() {
            "read-only · plan"
        } else {
            "allow all commands"
        };
        vec![
            ("auto".to_owned(), Style::default().fg(theme.secondary)),
            (" (high)".to_owned(), Style::default().fg(theme.text_muted)),
            (" · ".to_owned(), Style::default().fg(theme.ghost)),
            (permission.to_owned(), Style::default().fg(theme.text_dim)),
        ]
    };
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
pub(crate) fn place_segments(cells: &mut [(char, Style)], start: usize, segments: &[(String, Style)]) {
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
pub(crate) fn cells_to_line(cells: &[(char, Style)]) -> Line<'static> {
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
    use crate::app::{App, AssistantTurn, Status};
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

    /// A two-file `/diff` viewer fixture: file 0 (a context + one delete + one add hunk) opens
    /// expanded, file 1 (a substitution) stays collapsed — the diff-viewer reference composition.
    fn diff_viewer_fixture() -> crate::app::DiffViewer {
        let f0 = crate::diff::compute_file_diff(
            "crates/tui/src/stream_processor.rs",
            Some("keep_a\nremove_me\nkeep_b\n"),
            Some("keep_a\nadd_me\nkeep_b\n"),
        )
        .expect("file 0 diff");
        let f1 = crate::diff::compute_file_diff(
            "crates/tui/src/buffer.rs",
            Some("alpha\nbeta\n"),
            Some("ALPHA\nbeta\n"),
        )
        .expect("file 1 diff");
        crate::app::DiffViewer {
            files: vec![f0, f1],
            selected_file: 0,
            selected_hunk: 0,
            mode: DiffViewMode::Unified,
            focus: crate::app::DiffViewerFocus::Files,
            expanded: vec![true, false],
        }
    }

    #[test]
    fn diff_view_note_summarizes_file_and_line_totals() {
        // VAL-VIEW-023: the `Changes` panel note summarizes the file and line totals.
        let viewer = diff_viewer_fixture();
        assert_eq!(diff_view_note(&viewer), "2 files  +2 -2");
    }

    #[test]
    fn diff_accordion_file_rows_show_disclosure_and_colored_meta() {
        // VAL-VIEW-024/027: each file row begins with a `▼` (expanded) / `▶` (collapsed) disclosure
        // and a right-aligned `+N -N` count — AMBER for the open file, DIM for a collapsed file.
        let theme = style::ThemeColors::dark();
        let viewer = diff_viewer_fixture();
        let (lines, selected_row) = diff_accordion_lines(&viewer, 80, &theme);
        assert_eq!(selected_row, 0, "file 0 is the selected/open row");

        let row0 = plain(&lines[0]);
        assert!(
            row0.starts_with(crate::glyphs::DISCLOSURE_EXPANDED),
            "open file row begins with `▼`:\n{row0}"
        );
        assert!(row0.contains("stream_processor.rs"), "open file path:\n{row0}");
        assert!(row0.contains("+1 -1"), "open file meta:\n{row0}");
        // VAL-VIEW-024: the `+N -N` meta is a single AMBER count on the open file (NOT the inline
        // card's green/red pair — VAL-VIEW-026 reserves green/red for the hunk rows only).
        let meta0 = lines[0]
            .spans
            .iter()
            .find(|s| s.content.contains("+1 -1"))
            .expect("open file meta span");
        assert_eq!(meta0.style.fg, Some(theme.secondary), "open file meta is AMBER");
        assert_ne!(meta0.style.fg, Some(theme.success), "open file meta is not green");
        assert_ne!(meta0.style.fg, Some(theme.error), "open file meta is not red");

        let collapsed = lines
            .iter()
            .find(|l| plain(l).contains("buffer.rs"))
            .expect("collapsed file row");
        let collapsed_text = plain(collapsed);
        assert!(
            collapsed_text.starts_with(crate::glyphs::DISCLOSURE_COLLAPSED),
            "collapsed file row begins with `▶`:\n{collapsed_text}"
        );
        assert!(
            !collapsed_text.contains(crate::glyphs::DISCLOSURE_EXPANDED),
            "no `▼` on the collapsed row"
        );
        let meta1 = collapsed
            .spans
            .iter()
            .find(|s| s.content.contains("+1 -1"))
            .expect("collapsed file meta span");
        assert_eq!(meta1.style.fg, Some(theme.text_dim), "collapsed file meta is DIM");
    }

    #[test]
    fn diff_accordion_hunk_rows_reuse_wave0_add_del_tokens() {
        // VAL-VIEW-025/026: expanded hunk rows use the EXACT wave0 conv-diff-card tokens — additions
        // in ADD_FG on ADD_BG, deletions in DEL_FG on DEL_BG, context neutral — with a LNUM line
        // number at the origin, the sign at origin+6, and the code at origin+8.
        let theme = style::ThemeColors::dark();
        let viewer = diff_viewer_fixture();
        let (lines, _) = diff_accordion_lines(&viewer, 80, &theme);

        let added = lines
            .iter()
            .find(|l| plain(l).contains("add_me"))
            .expect("addition row");
        let added_code = added
            .spans
            .iter()
            .find(|s| s.content.contains("add_me"))
            .expect("added code span");
        assert_eq!(added_code.style.fg, Some(style::ADD_FG), "addition text ADD_FG");
        assert_eq!(added_code.style.bg, Some(style::ADD_BG), "addition bg ADD_BG");

        let removed = lines
            .iter()
            .find(|l| plain(l).contains("remove_me"))
            .expect("deletion row");
        let removed_code = removed
            .spans
            .iter()
            .find(|s| s.content.contains("remove_me"))
            .expect("removed code span");
        assert_eq!(removed_code.style.fg, Some(style::DEL_FG), "deletion text DEL_FG");
        assert_eq!(removed_code.style.bg, Some(style::DEL_BG), "deletion bg DEL_BG");

        // Context rows carry no red/green background.
        let context = lines
            .iter()
            .find(|l| plain(l).contains("keep_a"))
            .expect("context row");
        for span in &context.spans {
            assert_ne!(span.style.bg, Some(style::ADD_BG), "context has no ADD_BG");
            assert_ne!(span.style.bg, Some(style::DEL_BG), "context has no DEL_BG");
        }

        // A hunk row shows a right-justified LNUM line number, and the sign sits at origin+6.
        let has_lnum = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .any(|s| s.style.fg == Some(style::LNUM) && s.content.trim().parse::<usize>().is_ok());
        assert!(has_lnum, "a hunk row shows a LNUM line number");
        let added_plain = plain(added);
        let sign_char = added_plain.chars().nth(DIFF_HUNK_ORIGIN + 6);
        assert_eq!(sign_char, Some('+'), "the `+` sign sits at origin+6:\n{added_plain}");
    }

    #[test]
    fn diff_accordion_collapsed_file_hides_its_hunks() {
        // VAL-VIEW-027: a collapsed (`▶`) file renders only its summary row, no hunk body.
        let theme = style::ThemeColors::dark();
        let viewer = diff_viewer_fixture();
        let (lines, _) = diff_accordion_lines(&viewer, 80, &theme);
        let text = rendered(&lines);
        assert!(
            !text.contains("ALPHA"),
            "collapsed file 1's hunk body is hidden:\n{text}"
        );
        assert!(
            text.contains("add_me"),
            "expanded file 0's hunk body is shown:\n{text}"
        );
    }

    #[test]
    fn diff_accordion_selected_row_uses_the_shared_inverse_selection_bar() {
        // Consistency fix: the selected file row uses the shared inverse selection bar
        // (`surface[3]` fill), matching slash/dialog/palette rows — NOT the old ad-hoc `info`-fg +
        // `surface[2]` styling.
        let theme = style::ThemeColors::dark();
        let viewer = diff_viewer_fixture();
        let selected = diff_file_row(&viewer.files[0], true, true, 80, &theme);
        assert!(
            selected.spans.iter().all(|s| s.style.bg == Some(theme.surface[3])),
            "selected row is a full-width surface[3] selection bar"
        );
        assert!(
            selected.spans.iter().all(|s| s.style.bg != Some(theme.surface[2])),
            "selection bar does not use the old surface[2] fill"
        );
        assert!(
            selected.spans.iter().all(|s| s.style.fg != Some(theme.info)),
            "selection bar does not use the old info foreground"
        );

        let unselected = diff_file_row(&viewer.files[1], false, false, 80, &theme);
        assert!(
            unselected.spans.iter().all(|s| s.style.bg != Some(theme.surface[3])),
            "an unselected row has no selection fill"
        );
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
        // Open the sidebar so the split chrome (breadcrumb header + rule + full-width status bar)
        // and the flat-on-canvas sidebar box are drawn. In the brand design the sidebar box sits on
        // the near-black canvas (theme.background), not an elevated PANEL surface; the themed BAR
        // surface comes from the status bar the shared chrome paints (Ctrl+1 shows the context panel).
        a.on_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('1'),
            crossterm::event::KeyModifiers::CONTROL,
        ));

        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &a)).unwrap();
        let cells = terminal.backend().buffer().content();
        let theme = style::ThemeColors::light();

        assert!(
            cells.iter().any(|c| c.bg == theme.background),
            "canvas/sidebar background uses the theme background"
        );
        assert!(
            cells.iter().any(|c| c.bg == theme.surface[0]),
            "the status bar uses the themed BAR surface"
        );
        // On the light theme every cell must carry an explicit themed background — no band falls
        // through to terminal-default black (no black bars under the split).
        assert!(
            cells.iter().all(|c| c.bg != ratatui::style::Color::Reset),
            "every cell carries an explicit themed background (no default-black fallthrough)"
        );
    }

    // --- Wave 3: sidebar split layout ---------------------------------------------------------

    /// Open the sidebar on the default context panel via the leader toggle (`Ctrl+G` then `b`). This
    /// avoids the info toast that the `Ctrl+1..7` panel shortcuts pop, keeping the snapshot clean for
    /// per-cell color assertions.
    fn open_context_sidebar(a: &mut App) {
        a.on_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('g'),
            crossterm::event::KeyModifiers::CONTROL,
        ));
        a.on_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('b'),
            crossterm::event::KeyModifiers::NONE,
        ));
    }

    /// Render `a` into a 92×30 truecolor `TestBackend` and return the buffer.
    fn draw_92x30(a: &App) -> ratatui::buffer::Buffer {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(92, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, a)).unwrap();
        terminal.backend().buffer().clone()
    }

    /// The full text of buffer row `y`.
    fn buf_row(buf: &ratatui::buffer::Buffer, y: u16) -> String {
        (0..buf.area.width)
            .map(|x| buf[(x, y)].symbol())
            .collect::<Vec<_>>()
            .concat()
    }

    /// The whole buffer flattened to text.
    fn buf_text(buf: &ratatui::buffer::Buffer) -> String {
        buf.content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect()
    }

    /// The fg color of the first cell of the first row whose text run contains `needle`.
    fn first_fg(buf: &ratatui::buffer::Buffer, needle: &str) -> Option<Color> {
        for y in 0..buf.area.height {
            let row = buf_row(buf, y);
            if let Some(byte_idx) = row.find(needle) {
                let col = row[..byte_idx].chars().count() as u16;
                return Some(buf[(col, y)].fg);
            }
        }
        None
    }

    #[test]
    fn sidebar_split_geometry_is_mainw_55_and_sbw_32_at_sbx_58() {
        // VAL-SIDE-001/003: at 92×30 the split puts the live conversation at MAINX=2 / MAINW=55 and
        // the sidebar box at SBX=58 / SBW=32, with a single clean gap column between the panes;
        // closing the sidebar removes the split entirely.
        let mut a = app();
        open_context_sidebar(&mut a);
        let area = Rect {
            x: 0,
            y: 0,
            width: 92,
            height: 30,
        };
        let split = sidebar_split(&a, area).expect("an open sidebar yields a split");
        assert_eq!(split.conversation.x, 2, "MAINX = 2");
        assert_eq!(split.conversation.width, 55, "MAINW = 55");
        assert_eq!(split.sidebar.x, 58, "SBX = 58");
        assert_eq!(split.sidebar.width, 32, "SBW = 32");
        // Clean separation: the conversation ends at col 57, the sidebar starts at col 58.
        assert_eq!(
            split.conversation.x + split.conversation.width,
            57,
            "conversation right edge is one clean column before the sidebar"
        );
        assert!(
            split.conversation.x + split.conversation.width < split.sidebar.x,
            "at least one gap column separates the panes (no overlap)"
        );
        // Both panes share the same top row and height (aligned split).
        assert_eq!(split.conversation.y, split.sidebar.y, "panes top-aligned");
        assert_eq!(split.conversation.height, split.sidebar.height, "equal height");

        let closed = app();
        assert!(
            sidebar_split(&closed, area).is_none(),
            "a closed sidebar yields no split (full-width conversation)"
        );
    }

    #[test]
    fn sidebar_open_renders_breadcrumb_box_subtitle_rule_and_status_bar() {
        // VAL-SIDE-001/004/005/006/007: opening the sidebar draws the `✦ Cortex Code · context`
        // breadcrumb, a rounded sidebar box titled `Context` with a subtitle + ghost rule, and the
        // 3-zone status bar (real model id) on the bottom row.
        let mut a = app();
        a.transcript.push(Entry::User("cap the streaming buffer".to_owned()));
        open_context_sidebar(&mut a);
        let buf = draw_92x30(&a);

        let row0 = buf_row(&buf, 0);
        assert!(row0.contains('✦'), "breadcrumb star on row 0:\n{row0}");
        assert!(row0.contains("Cortex Code"), "breadcrumb wordmark:\n{row0}");
        assert!(
            row0.contains("context"),
            "breadcrumb crumb equals the active panel:\n{row0}"
        );

        let screen = buf_text(&buf);
        assert!(
            screen.contains('╭') && screen.contains('╮'),
            "rounded sidebar top corners"
        );
        assert!(
            screen.contains('╰') && screen.contains('╯'),
            "rounded sidebar bottom corners"
        );
        assert!(screen.contains("Context"), "sidebar title `Context`");
        assert!(
            screen.contains("in context"),
            "faint subtitle `N files in context`"
        );

        let status = buf_row(&buf, 29);
        assert!(status.contains("? for help"), "status left zone:\n{status}");
        assert!(
            status.contains("google/gemini-2.5-flash"),
            "status right zone shows the real model id:\n{status}"
        );
        assert!(!status.contains("opus-4.8"), "never the mockup model id:\n{status}");
    }

    #[test]
    fn sidebar_left_pane_stays_live_beside_the_sidebar() {
        // VAL-SIDE-002: while the sidebar is open the left pane still renders the user `›` mark and
        // the composer, confined to the conversation column (nothing blanked/collapsed).
        let mut a = app();
        a.transcript.push(Entry::User("cap the streaming buffer".to_owned()));
        open_context_sidebar(&mut a);
        let buf = draw_92x30(&a);
        let screen = buf_text(&buf);
        assert!(screen.contains('›'), "the user `›` mark renders in the left pane");
        assert!(
            screen.contains("reply to cortex"),
            "the composer stays live in the left pane"
        );
        // The user message text sits within the conversation column (left of the sidebar at col 58).
        let mut found_in_left = false;
        for y in 0..buf.area.height {
            let row = buf_row(&buf, y);
            if let Some(idx) = row.find("cap the streaming") {
                let col = row[..idx].chars().count();
                assert!(col < 58, "transcript text stays left of the sidebar (col {col})");
                found_in_left = true;
            }
        }
        assert!(found_in_left, "the transcript message renders in the left pane");
    }

    #[test]
    fn sidebar_title_is_amber_in_border_with_a_faint_right_aligned_note() {
        // VAL-SIDE-005: the title rides the top border in AMBER; a FAINT note is right-aligned in
        // the same border with padding before the corner (not hugging it).
        let mut a = app();
        open_context_sidebar(&mut a);
        let buf = draw_92x30(&a);
        let theme = style::ThemeColors::from_name(a.theme());

        // The title sits on the sidebar box top border (row 2).
        let top = buf_row(&buf, 2);
        assert!(top.contains("Context"), "title embedded in the top border:\n{top}");
        assert_eq!(first_fg(&buf, "Context"), Some(theme.secondary), "title is AMBER");

        // The top-right rounded corner exists with a blank pad column immediately before it (the
        // note never hugs the corner).
        let corner_col = top.chars().position(|c| c == '╮').expect("top-right corner");
        assert!(corner_col >= 1, "corner is not at column 0");
        let before: char = top.chars().nth(corner_col - 1).unwrap();
        assert_eq!(before, ' ', "≥1 blank column pads the note from the corner");
    }

    #[test]
    fn sidebar_switching_panels_swaps_body_not_geometry() {
        // VAL-SIDE-008: cycling to another panel changes the breadcrumb crumb + sidebar title while
        // the conversation region and the sidebar region (widths/separation) stay identical.
        let mut a = app();
        open_context_sidebar(&mut a);
        let area = Rect {
            x: 0,
            y: 0,
            width: 92,
            height: 30,
        };
        let ctx_split = sidebar_split(&a, area).unwrap();
        let ctx_buf = draw_92x30(&a);
        assert!(buf_row(&ctx_buf, 0).contains("context"), "crumb starts as context");
        assert!(buf_text(&ctx_buf).contains("Context"), "title starts as Context");

        // Switch to the files panel (Ctrl+2).
        a.on_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('2'),
            crossterm::event::KeyModifiers::CONTROL,
        ));
        let files_split = sidebar_split(&a, area).unwrap();
        let files_buf = draw_92x30(&a);
        assert!(buf_row(&files_buf, 0).contains("files"), "crumb switched to files");
        assert!(buf_text(&files_buf).contains("Files"), "title switched to Files");

        assert_eq!(
            ctx_split.conversation, files_split.conversation,
            "the conversation region is unchanged across panel switches"
        );
        assert_eq!(
            ctx_split.sidebar, files_split.sidebar,
            "the sidebar region is unchanged across panel switches"
        );
    }

    #[test]
    fn sidebar_toggle_round_trips_to_full_width() {
        // VAL-SIDE-034: toggling the sidebar open then closed restores the full-width conversation
        // (no right column) and preserves the left conversation content across the round-trip.
        let leader = || {
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char('g'),
                crossterm::event::KeyModifiers::CONTROL,
            )
        };
        let toggle = || {
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char('b'),
                crossterm::event::KeyModifiers::NONE,
            )
        };
        let area = Rect {
            x: 0,
            y: 0,
            width: 92,
            height: 30,
        };

        let mut a = app();
        a.transcript.push(Entry::User("keep this line".to_owned()));

        // Toggle ON.
        a.on_key(leader());
        a.on_key(toggle());
        assert!(a.sidebar_visible(), "sidebar shown after the first toggle");
        assert!(sidebar_split(&a, area).is_some(), "split present while open");
        let open_buf = draw_92x30(&a);
        assert!(buf_row(&open_buf, 0).contains("context"), "breadcrumb present while open");
        assert!(buf_text(&open_buf).contains('╭'), "sidebar box present while open");

        // Toggle OFF.
        a.on_key(leader());
        a.on_key(toggle());
        assert!(!a.sidebar_visible(), "sidebar hidden after the second toggle");
        assert!(
            sidebar_split(&a, area).is_none(),
            "no split when closed (full-width conversation)"
        );
        let closed_buf = draw_92x30(&a);
        assert!(
            buf_text(&closed_buf).contains("keep this line"),
            "the left conversation content is preserved across the toggle"
        );
        // The composer/status bar span the full width again; the status right zone is flush-right.
        let status = buf_row(&closed_buf, 29);
        assert!(status.contains("? for help"), "full-width status bar restored:\n{status}");
        assert!(
            status.trim_end().ends_with("google/gemini-2.5-flash"),
            "status right zone is flush to the full width:\n{status}"
        );
    }

    #[test]
    fn sidebar_chrome_is_glyph_safe_and_brand_pure() {
        // VAL-SIDE-009/010: the sidebar chrome never renders the thin `▾`/`▸` triangles or braille,
        // and its border/title/note/subtitle/rule use only orange-family + neutral grey tokens.
        let mut a = app();
        open_context_sidebar(&mut a);
        let buf = draw_92x30(&a);
        let screen = buf_text(&buf);
        assert!(!screen.contains('▾'), "no thin `▾` disclosure triangle");
        assert!(!screen.contains('▸'), "no thin `▸` disclosure triangle");
        assert!(!glyphs::contains_braille(&screen), "no braille anywhere");
        assert!(!screen.contains(">_"), "no `>_` cliché");

        let theme = style::ThemeColors::from_name(a.theme());
        // Title AMBER, subtitle FAINT — chrome tokens only.
        assert_eq!(first_fg(&buf, "Context"), Some(theme.secondary), "title AMBER");
        assert_eq!(
            first_fg(&buf, "in context"),
            Some(theme.text_muted),
            "subtitle FAINT"
        );
        // The sidebar box top-left corner (SBX=58, row 2) is drawn in the dim-orange border token
        // (no blue/cyan chrome).
        assert_eq!(buf[(58, 2)].symbol(), "╭", "sidebar top-left rounded corner at SBX");
        assert_eq!(
            buf[(58, 2)].fg,
            theme.border,
            "sidebar border is the dim-orange (ORANGE_D) token"
        );
    }

    /// The flattened text of a rendered `Line`.
    fn line_text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    /// The fg color of the first span in `line` whose content, trimmed, equals `needle`.
    fn span_fg(line: &Line<'_>, needle: &str) -> Option<Color> {
        line.spans
            .iter()
            .find(|s| s.content.trim() == needle)
            .and_then(|s| s.style.fg)
    }

    #[test]
    fn lsp_body_renders_worst_severity_dots_ew_rows_and_clean_indicator() {
        // VAL-SIDE-020/021/023: each diagnosed file row shows a `●` dot colored by its worst
        // diagnostic (red error / amber warning) + BRIGHT filename; per-diagnostic rows show a line
        // number, an `E` (red) / `W` (amber) severity letter, and the message.
        let theme = style::ThemeColors::from_name("dark");
        let groups = vec![
            crate::app::LspFileGroup {
                file: "buffer.rs".to_owned(),
                errors: 1,
                warnings: 1,
                diagnostics: vec![
                    crate::app::LspDiagnosticRow {
                        line: 42,
                        is_error: true,
                        message: "cap unused when 0".to_owned(),
                    },
                    crate::app::LspDiagnosticRow {
                        line: 18,
                        is_error: false,
                        message: "unused import Read".to_owned(),
                    },
                ],
            },
            crate::app::LspFileGroup {
                file: "mod.rs".to_owned(),
                errors: 0,
                warnings: 1,
                diagnostics: vec![crate::app::LspDiagnosticRow {
                    line: 90,
                    is_error: false,
                    message: "redundant clone()".to_owned(),
                }],
            },
        ];
        let mut lines = Vec::new();
        sidebar_lsp_body(&groups, &theme, 30, &mut lines);

        // buffer.rs: worst = error -> red `●`, BRIGHT filename.
        assert_eq!(lines[0].spans[0].content.trim(), glyphs::DOT_FILLED);
        assert_eq!(span_fg(&lines[0], glyphs::DOT_FILLED), Some(theme.error));
        assert_eq!(span_fg(&lines[0], "buffer.rs"), Some(theme.text_bright));
        // error diagnostic row: `E` in red + line number + message.
        assert!(line_text(&lines[1]).contains("42"));
        assert!(line_text(&lines[1]).contains("cap unused when 0"));
        assert_eq!(span_fg(&lines[1], "E"), Some(theme.error));
        // warning diagnostic row: `W` in amber.
        assert!(line_text(&lines[2]).contains("18"));
        assert_eq!(span_fg(&lines[2], "W"), Some(theme.warning));
        // mod.rs: worst = warning -> amber `●` (VAL-SIDE-023: amber for a warning-only file).
        assert_eq!(span_fg(&lines[3], glyphs::DOT_FILLED), Some(theme.warning));
        assert_eq!(span_fg(&lines[4], "W"), Some(theme.warning));
    }

    #[test]
    fn lsp_body_clean_file_shows_green_no_problems_row() {
        // VAL-SIDE-022: a file with no diagnostics shows a green `○ no problems` row (OKG success),
        // distinct from the error/warning dot styling (its file dot is a neutral, non-severity tone).
        let theme = style::ThemeColors::from_name("dark");
        let groups = vec![crate::app::LspFileGroup {
            file: "stream_processor.rs".to_owned(),
            errors: 0,
            warnings: 0,
            diagnostics: Vec::new(),
        }];
        let mut lines = Vec::new();
        sidebar_lsp_body(&groups, &theme, 30, &mut lines);

        // The clean file's `●` dot is a neutral (FAINT) tone — not red or amber.
        assert_eq!(span_fg(&lines[0], glyphs::DOT_FILLED), Some(theme.text_muted));
        assert_ne!(span_fg(&lines[0], glyphs::DOT_FILLED), Some(theme.error));
        assert_ne!(span_fg(&lines[0], glyphs::DOT_FILLED), Some(theme.warning));
        // The `○ no problems` row is OKG green (both glyph and label).
        assert_eq!(lines[1].spans[0].content.trim(), glyphs::DOT_HOLLOW);
        assert_eq!(span_fg(&lines[1], glyphs::DOT_HOLLOW), Some(theme.success));
        assert!(line_text(&lines[1]).contains("no problems"));
        assert_eq!(span_fg(&lines[1], "no problems"), Some(theme.success));
    }

    #[test]
    fn mcp_body_renders_status_dots_and_state_colored_words() {
        // VAL-SIDE-025: each server row shows a leading `●` status dot + name + a right-aligned
        // status word, with the dot and word colored by state (ready=green, starting=amber,
        // error=red).
        let theme = style::ThemeColors::from_name("dark");
        let rows = vec![
            crate::view::McpServerRow {
                name: "context7".to_owned(),
                status_word: "ready".to_owned(),
                tone: crate::view::McpStatusTone::Ready,
                tool_count: Some(5),
                exposes: String::new(),
            },
            crate::view::McpServerRow {
                name: "playwright".to_owned(),
                status_word: "starting".to_owned(),
                tone: crate::view::McpStatusTone::Degraded,
                tool_count: None,
                exposes: String::new(),
            },
            crate::view::McpServerRow {
                name: "github".to_owned(),
                status_word: "error".to_owned(),
                tone: crate::view::McpStatusTone::Error,
                tool_count: None,
                exposes: String::new(),
            },
        ];
        let width = 30usize;
        let mut lines = Vec::new();
        sidebar_mcp_body(&rows, &theme, width, &mut lines);

        // ready -> green dot + green `ready` flush-right.
        assert_eq!(lines[0].spans[0].content.trim(), glyphs::DOT_FILLED);
        assert_eq!(span_fg(&lines[0], glyphs::DOT_FILLED), Some(theme.success));
        assert!(line_text(&lines[0]).contains("context7"));
        assert_eq!(span_fg(&lines[0], "ready"), Some(theme.success));
        // The status word sits flush against the right edge of the panel width.
        assert!(line_text(&lines[0]).trim_end().ends_with("ready"));
        assert_eq!(line_text(&lines[0]).chars().count(), width, "row fills the panel width");
        // starting -> amber dot + word.
        assert_eq!(span_fg(&lines[1], glyphs::DOT_FILLED), Some(theme.secondary));
        assert_eq!(span_fg(&lines[1], "starting"), Some(theme.secondary));
        // error -> red dot + word.
        assert_eq!(span_fg(&lines[2], glyphs::DOT_FILLED), Some(theme.error));
        assert_eq!(span_fg(&lines[2], "error"), Some(theme.error));
    }

    #[test]
    fn lsp_panel_chrome_shows_title_ew_note_and_server_subtitle() {
        // VAL-SIDE-019: selecting the lsp panel shows crumb `· lsp`, title `LSP`, a right-aligned
        // `<E>E · <W>W` note (real counts), and a faint `rust-analyzer` subtitle.
        let mut a = app();
        a.transcript.push(Entry::User("cap the buffer".to_owned()));
        a.capture_lsp_sidebar_note(
            "edit",
            None,
            "Diagnostics:\n\
             lsp diagnostics for src/buffer.rs: 1 error(s), 1 warning(s), 0 other\n\
             - error [E0433] (rustc) at line 42, character 5-10: cap unused when 0\n\
             - warning at line 18, character 1-3: unused import Read\n\
             lsp diagnostics for src/mod.rs: 0 error(s), 1 warning(s), 0 other\n\
             - warning at line 90, character 2-8: redundant clone()",
        );
        open_context_sidebar(&mut a);
        a.on_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('3'),
            crossterm::event::KeyModifiers::CONTROL,
        ));
        let buf = draw_92x30(&a);
        assert!(buf_row(&buf, 0).contains("lsp"), "breadcrumb crumb is `lsp`");
        let screen = buf_text(&buf);
        assert!(screen.contains("LSP"), "sidebar title `LSP`");
        assert!(screen.contains("1E"), "note shows the real error count");
        assert!(screen.contains("2W"), "note shows the real warning count");
        assert!(screen.contains("rust-analyzer"), "faint subtitle names the language server");
        assert!(screen.contains("buffer.rs"), "the diagnosed file renders in the body");
        assert!(!screen.contains('▾') && !screen.contains('▸'), "no thin triangles");
        assert!(!glyphs::contains_braille(&screen), "no braille in the lsp panel");
    }

    #[test]
    fn mcp_panel_chrome_shows_title_tools_note_and_servers_subtitle() {
        // VAL-SIDE-024: selecting the mcp panel shows crumb `· mcp`, title `MCP`, a right-aligned
        // `<N> tools` note, and a faint `<N> servers connected` subtitle (real counts; the offline
        // seeded session honestly shows zero).
        let mut a = app();
        a.transcript.push(Entry::User("check servers".to_owned()));
        open_context_sidebar(&mut a);
        a.on_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('4'),
            crossterm::event::KeyModifiers::CONTROL,
        ));
        let buf = draw_92x30(&a);
        assert!(buf_row(&buf, 0).contains("mcp"), "breadcrumb crumb is `mcp`");
        let screen = buf_text(&buf);
        assert!(screen.contains("MCP"), "sidebar title `MCP`");
        assert!(screen.contains("tools"), "right-aligned `<N> tools` note");
        assert!(
            screen.contains("servers connected") || screen.contains("server connected"),
            "faint `<N> servers connected` subtitle"
        );
        assert!(!glyphs::contains_braille(&screen), "no braille in the mcp panel");
    }

    #[test]
    fn todo_body_renders_three_distinct_task_states() {
        // VAL-SIDE-027: completed -> green `✓` + DIM text, in-progress -> ORANGE `▶` (heavy, not
        // `▸`) + BRIGHT text, pending -> ghost-grey `○` + normal TXT text.
        let theme = style::ThemeColors::from_name("dark");
        let items = vec![
            protocol::TodoItem::new("cap the streaming buffer", protocol::TodoStatus::Completed),
            protocol::TodoItem::new("write tests for the cap", protocol::TodoStatus::InProgress),
            protocol::TodoItem::new("open a pull request", protocol::TodoStatus::Pending),
        ];
        let mut lines = Vec::new();
        sidebar_todo_body(&items, &theme, 28, &mut lines);
        assert_eq!(lines.len(), 3, "one row per task");

        // Completed: green `✓` marker, DIM body text.
        assert_eq!(lines[0].spans[0].content.trim(), glyphs::MARK_DONE);
        assert_eq!(span_fg(&lines[0], glyphs::MARK_DONE), Some(theme.success));
        assert_eq!(
            span_fg(&lines[0], "cap the streaming buffer"),
            Some(theme.text_dim)
        );
        // In-progress: ORANGE `▶` marker, BRIGHT body text.
        assert_eq!(lines[1].spans[0].content.trim(), glyphs::DISCLOSURE_COLLAPSED);
        assert_eq!(
            span_fg(&lines[1], glyphs::DISCLOSURE_COLLAPSED),
            Some(theme.primary)
        );
        assert_eq!(
            span_fg(&lines[1], "write tests for the cap"),
            Some(theme.text_bright)
        );
        // Pending: ghost-grey `○` marker, normal TXT body text.
        assert_eq!(lines[2].spans[0].content.trim(), glyphs::DOT_HOLLOW);
        assert_eq!(span_fg(&lines[2], glyphs::DOT_HOLLOW), Some(theme.ghost));
        assert_eq!(span_fg(&lines[2], "open a pull request"), Some(theme.text));

        // Glyph safety: the in-progress marker is the heavy `▶`, never the thin `▸`.
        let rendered: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(rendered.contains('▶'), "heavy disclosure triangle present");
        assert!(!rendered.contains('▸'), "no thin `▸` triangle");
    }

    #[test]
    fn subagents_body_renders_marker_name_status_and_task() {
        // VAL-SIDE-029: each entry is an AMBER `◆` + BRIGHT name + right-aligned status colored by
        // state (running -> ORANGE, queued -> FAINT, done -> OKG green) + a DIM task on the next
        // line, with a blank spacer between entries.
        let theme = style::ThemeColors::from_name("dark");
        let rows = vec![
            crate::app::SidebarSubagentRow {
                name: "explore".to_owned(),
                task: "find auth middleware".to_owned(),
                status: "running 8s".to_owned(),
                state: crate::app::SubagentState::Running,
            },
            crate::app::SidebarSubagentRow {
                name: "plan".to_owned(),
                task: "design token-refresh flow".to_owned(),
                status: "queued".to_owned(),
                state: crate::app::SubagentState::Queued,
            },
            crate::app::SidebarSubagentRow {
                name: "general".to_owned(),
                task: "summarize redaction audit".to_owned(),
                status: "done 1.2s".to_owned(),
                state: crate::app::SubagentState::Done,
            },
        ];
        let mut lines = Vec::new();
        sidebar_subagents_body(&rows, &theme, 30, &mut lines);
        // 3 entries * 2 rows + 2 blank spacers = 8 lines.
        assert_eq!(lines.len(), 8, "header+task per entry with a spacer between");

        // explore: AMBER `◆` + BRIGHT name + ORANGE `running 8s`, DIM task on the next line.
        assert_eq!(lines[0].spans[0].content.trim(), glyphs::DOT_DIAMOND);
        assert_eq!(span_fg(&lines[0], glyphs::DOT_DIAMOND), Some(theme.secondary));
        assert_eq!(span_fg(&lines[0], "explore"), Some(theme.text_bright));
        assert_eq!(span_fg(&lines[0], "running 8s"), Some(theme.primary));
        assert!(line_text(&lines[0]).trim_end().ends_with("running 8s"));
        assert!(line_text(&lines[1]).contains("find auth middleware"));
        assert_eq!(span_fg(&lines[1], "find auth middleware"), Some(theme.text_dim));
        // Blank spacer between entries.
        assert_eq!(line_text(&lines[2]).trim(), "");
        // plan: queued -> FAINT status.
        assert_eq!(span_fg(&lines[3], "queued"), Some(theme.text_muted));
        // general: done -> OKG green status.
        assert_eq!(span_fg(&lines[6], "done 1.2s"), Some(theme.success));
    }

    #[test]
    fn queue_body_numbers_rows_and_highlights_run_next() {
        // VAL-SIDE-031/032: numbered rows; the run-next (first) row uses an ORANGE number + BRIGHT
        // text; later rows a dimmer ORANGE_D number + normal TXT text; fitting text is not
        // truncated.
        let theme = style::ThemeColors::from_name("dark");
        let prompts = vec![
            "add tests for buffer cap".to_owned(),
            "update CHANGELOG.md".to_owned(),
            "run the full test suite".to_owned(),
        ];
        let mut lines = Vec::new();
        sidebar_queue_body(&prompts, &theme, 30, &mut lines);
        assert_eq!(lines.len(), 3, "one row per queued prompt");

        // Run-next row: ORANGE `1` number + BRIGHT text, rendered in full (no `…`).
        assert_eq!(lines[0].spans[0].content.trim(), "1");
        assert_eq!(span_fg(&lines[0], "1"), Some(theme.primary));
        assert_eq!(
            span_fg(&lines[0], "add tests for buffer cap"),
            Some(theme.text_bright)
        );
        assert!(!line_text(&lines[0]).contains(glyphs::ELLIPSIS), "no premature `…`");
        // Later rows: dimmer ORANGE_D number + normal TXT text.
        assert_eq!(lines[1].spans[0].content.trim(), "2");
        assert_eq!(span_fg(&lines[1], "2"), Some(theme.border));
        assert_eq!(span_fg(&lines[1], "update CHANGELOG.md"), Some(theme.text));
        assert_eq!(span_fg(&lines[2], "3"), Some(theme.border));
    }

    #[test]
    fn queue_body_clips_only_overrunning_prompt_text() {
        // VAL-SIDE-032: a prompt wider than the panel is clipped (with `…`), but a prompt that fits
        // is rendered verbatim without a trailing ellipsis.
        let theme = style::ThemeColors::from_name("dark");
        let prompts = vec![
            "short prompt".to_owned(),
            "this queued prompt is far too long to fit inside the narrow sidebar column".to_owned(),
        ];
        let mut lines = Vec::new();
        sidebar_queue_body(&prompts, &theme, 20, &mut lines);
        // The short prompt fits and keeps no ellipsis.
        assert!(line_text(&lines[0]).contains("short prompt"));
        assert!(!line_text(&lines[0]).contains(glyphs::ELLIPSIS), "fitting text is verbatim");
        // The overrunning prompt is hard-clipped to the panel interior (never overruns the width).
        assert!(line_text(&lines[1]).chars().count() <= 20, "row stays within the panel");
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
        let slash_rows = slash_menu_lines(&a, 60).expect("slash menu open");
        assert!(slash_rows.len() > 1, "multiple slash commands match `/`");
        // The highlighted (first) row is the warm SELBG inverse bar (VAL-CONV-036): an explicit
        // ORANGE `›` marker and a BRIGHT command label, all on the surface[3] selection background.
        assert_eq!(slash_rows[0].spans[0].content, "› ");
        assert_eq!(slash_rows[0].spans[0].style.fg, Some(theme.primary));
        assert_eq!(slash_rows[0].spans[0].style.bg, Some(theme.surface[3]));
        assert_eq!(slash_rows[0].spans[1].style.fg, Some(theme.text_bright));
        assert_eq!(slash_rows[0].spans[1].style.bg, Some(theme.surface[3]));
        // A non-highlighted row shows the ORANGE command and a FAINT description with no row fill.
        assert_eq!(slash_rows[1].spans[1].style.fg, Some(theme.primary));
        assert_eq!(slash_rows[1].spans[3].style.fg, Some(theme.text_muted));
        assert_eq!(slash_rows[1].spans[1].style.bg, None);

        a.on_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('p'),
            crossterm::event::KeyModifiers::CONTROL,
        ));
        // The command palette now renders through the shared rich modal: its selected row is the
        // warm SELBG (surface[3]) inverse bar with an ORANGE `›` marker and BRIGHT key text.
        let palette_spec = palette_modal_spec(&a, &theme);
        let palette_rows = crate::modal::build_interior_lines(&palette_spec, 58, 6, None, &theme);
        // rows[0] is the search line, rows[1] the underline rule, rows[2] the first command row.
        let palette_sel = &palette_rows[2];
        assert_eq!(palette_sel.spans[0].content, crate::glyphs::SELECT_MARKER);
        assert_eq!(palette_sel.spans[0].style.fg, Some(theme.primary));
        assert_eq!(palette_sel.spans[0].style.bg, Some(theme.surface[3]));

        // The rich modal picker's selected row is the warm inverse bar (surface[3]) carrying an
        // ORANGE `›` marker and BRIGHT key text; the other row has no marker and no fill.
        let spec = crate::modal::ModalSpec {
            title: "Models".to_owned(),
            rows: vec![
                crate::modal::ModalRow::new("alpha"),
                crate::modal::ModalRow::new("beta"),
            ],
            selected: 0,
            hints: vec![crate::modal::ModalHint::new("esc", "close")],
            note: None,
            search: None,
            input: None,
            empty: None,
            width: 66,
            key_width: None,
            key_color: theme.text,
        };
        let dialog_rows = crate::modal::build_interior_lines(&spec, 60, 2, None, &theme);
        assert_eq!(dialog_rows[0].spans[0].content, crate::glyphs::SELECT_MARKER);
        assert_eq!(dialog_rows[0].spans[0].style.fg, Some(theme.primary));
        assert_eq!(dialog_rows[0].spans[0].style.bg, Some(theme.surface[3]));
        assert!(
            !dialog_rows[1]
                .spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
                .contains(crate::glyphs::SELECT_MARKER)
        );
    }

    /// Type a slash command into an empty prompt and dispatch it (open its picker/view).
    fn open_via_slash(a: &mut App, command: &str) {
        for c in command.chars() {
            a.on_key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char(c),
                crossterm::event::KeyModifiers::NONE,
            ));
        }
        a.on_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        ));
    }

    /// Render the full frame at 92×30 and flatten the buffer to a single string for text assertions.
    fn rendered_frame(a: &App) -> String {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(92, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, a)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn model_picker_renders_rich_columns_and_footer() {
        // VAL-PICK-014..019/067: a titled rounded modal with a search row, real `provider/model`
        // ids, a `ctx · price` column, and the select/provider/filter/close footer with safe glyphs.
        let mut a = app();
        open_via_slash(&mut a, "/models");
        let text = rendered_frame(&a);
        assert!(text.contains("Models"), "titled modal:\n{text}");
        assert!(text.contains("type to filter…"), "search row placeholder present");
        assert!(
            text.contains("google/gemini-2.5-flash"),
            "real model id rendered:\n{text}"
        );
        assert!(text.contains('$'), "price column rendered");
        assert!(
            text.contains(crate::glyphs::KEY_ENTER) && text.contains("select"),
            "select footer hint"
        );
        assert!(
            text.contains(crate::glyphs::KEY_TAB) && text.contains("provider"),
            "provider footer hint"
        );
        assert!(
            text.contains("filter") && text.contains("close"),
            "filter/close footer hints"
        );
        assert!(text.contains('╭') && text.contains('╯'), "rounded border");
        assert!(
            !crate::glyphs::contains_braille(&text),
            "no braille anywhere in the model picker"
        );
    }

    #[test]
    fn session_picker_renders_search_and_footer_chrome() {
        // VAL-PICK-006/008: a titled rounded modal with a `›` search row and the open/new/delete/
        // close footer (safe `⌫` glyph). Real seeded rows are covered by the app-level tests.
        let mut a = app();
        open_via_slash(&mut a, "/sessions");
        let text = rendered_frame(&a);
        assert!(text.contains("Sessions"), "titled modal:\n{text}");
        assert!(text.contains("type to filter…"), "search row placeholder present");
        assert!(
            text.contains("open")
                && text.contains("new")
                && text.contains("delete")
                && text.contains("close"),
            "session footer hints present:\n{text}"
        );
        assert!(
            text.contains(crate::glyphs::KEY_DELETE),
            "delete advertised with the safe ⌫ glyph"
        );
        assert!(text.contains('╭') && text.contains('╯'), "rounded border");
    }

    #[test]
    fn agent_picker_renders_personas_meta_and_footer() {
        // VAL-PICK-023/024/025/026: titled `Agents` modal, real personas with descriptions, a
        // `primary`/`subagent` kind meta, a `↵ use` / `esc close` footer, no search row, safe glyphs.
        let mut a = app();
        open_via_slash(&mut a, "/agents");
        let text = rendered_frame(&a);
        assert!(text.contains("Agents"), "titled modal:\n{text}");
        for persona in ["build", "plan", "general", "explore"] {
            assert!(text.contains(persona), "agent {persona} listed:\n{text}");
        }
        assert!(text.contains("primary"), "primary kind meta present");
        assert!(text.contains("subagent"), "subagent kind meta present");
        assert!(
            text.contains(crate::glyphs::KEY_ENTER) && text.contains("use"),
            "↵ use footer hint:\n{text}"
        );
        assert!(text.contains("close"), "esc close footer hint");
        assert!(
            !text.contains("type to filter…"),
            "agent picker has no search row"
        );
        assert!(text.contains('╭') && text.contains('╯'), "rounded border");
        assert!(!crate::glyphs::contains_braille(&text), "no braille");
    }

    #[test]
    fn variant_picker_renders_title_descriptions_and_footer() {
        // VAL-PICK-037/038/039: titled `Reasoning variant`, key + description rows, no meta column,
        // no search row, and a `↵ select` / `esc close` footer.
        let mut a = app();
        open_via_slash(&mut a, "/variant");
        let text = rendered_frame(&a);
        assert!(text.contains("Reasoning variant"), "titled modal:\n{text}");
        assert!(text.contains("reasoning"), "descriptions rendered:\n{text}");
        assert!(
            text.contains(crate::glyphs::KEY_ENTER) && text.contains("select"),
            "↵ select footer hint:\n{text}"
        );
        assert!(
            !text.contains("type to filter…"),
            "variant picker has no search row"
        );
        assert!(!crate::glyphs::contains_braille(&text), "no braille");
    }

    #[test]
    fn skill_picker_renders_source_meta_and_toggle_footer() {
        // VAL-PICK-030/032/034/035: titled `Skills` modal with a `space toggle` / `↵ apply` /
        // `esc close` footer and a source meta column; no search row.
        let mut a = app();
        open_via_slash(&mut a, "/skills");
        let text = rendered_frame(&a);
        assert!(text.contains("Skills"), "titled modal:\n{text}");
        assert!(
            text.contains("space") && text.contains("toggle"),
            "space toggle footer hint:\n{text}"
        );
        assert!(
            text.contains(crate::glyphs::KEY_ENTER) && text.contains("apply"),
            "↵ apply footer hint:\n{text}"
        );
        assert!(
            !text.contains("type to filter…"),
            "skill picker has no search row"
        );
        assert!(!crate::glyphs::contains_braille(&text), "no braille");
    }

    #[test]
    fn theme_picker_renders_descriptors_characters_and_live_preview_footer() {
        // VAL-PICK-043..046/050: titled `Themes` modal, the five theme names + descriptors + a
        // right-aligned character column, a `↑↓ preview` / `↵ apply` / `esc close` footer and a
        // `live preview` note; no search row, no braille.
        let mut a = app();
        open_via_slash(&mut a, "/themes");
        let text = rendered_frame(&a);
        assert!(text.contains("Themes"), "titled modal:\n{text}");
        for name in ["dark", "light", "vscode-dark", "monokai", "ocean"] {
            assert!(text.contains(name), "theme {name} listed:\n{text}");
        }
        assert!(text.contains("cortex default"), "descriptor rendered:\n{text}");
        for character in ["warm", "cool", "neutral", "high-contrast", "calm"] {
            assert!(
                text.contains(character),
                "character {character} rendered:\n{text}"
            );
        }
        assert!(
            text.contains(crate::glyphs::KEY_UP) && text.contains("preview"),
            "↑↓ preview footer hint:\n{text}"
        );
        assert!(
            text.contains(crate::glyphs::KEY_ENTER) && text.contains("apply"),
            "↵ apply footer hint:\n{text}"
        );
        assert!(text.contains("live preview"), "live preview note:\n{text}");
        assert!(
            !text.contains("type to filter…"),
            "theme picker has no search row"
        );
        assert!(text.contains('╭') && text.contains('╯'), "rounded border");
        assert!(!crate::glyphs::contains_braille(&text), "no braille");
    }

    #[test]
    fn subagent_picker_renders_footer_hints_and_empty_state() {
        // VAL-PICK-051/056/057: `/subagents` opens the titled `Subagents` modal with a `↵ view` /
        // `^C cancel` / `esc close` footer; with no children it shows a clear empty state.
        let mut a = app();
        open_via_slash(&mut a, "/subagents");
        let text = rendered_frame(&a);
        assert!(text.contains("Subagents"), "titled modal:\n{text}");
        assert!(
            text.contains("view") && text.contains("cancel") && text.contains("close"),
            "subagent footer hints present:\n{text}"
        );
        assert!(text.contains("^C"), "^C cancel advertised:\n{text}");
        assert!(
            text.contains("No subagents"),
            "empty state shown when there are no children:\n{text}"
        );
        assert!(!crate::glyphs::contains_braille(&text), "no braille");
    }

    #[test]
    fn tool_card_colors_follow_theme_semantics() {
        // VAL-TOOL-003/004/005/006: the status accent (running ORANGE, done OKG, error ERR)
        // uniformly tints the top-border label glyph + name + state word.
        let theme = style::ThemeColors::ocean_dark();
        let glyph_fg = |line: &Line, glyph: &str| -> Option<Color> {
            line.spans
                .iter()
                .find(|s| s.content.contains(glyph))
                .and_then(|s| s.style.fg)
        };

        // Running: `⚙` gear on an ORANGE (brand primary) border/label, with the `running` word.
        let running = tool_card("read", r#"{"path":"a.txt"}"#);
        let mut running_lines = Vec::new();
        push_tool(&mut running_lines, &running, 80, false, &theme);
        assert_eq!(
            glyph_fg(&running_lines[0], crate::glyphs::MARK_RUNNING),
            Some(theme.primary)
        );
        assert!(rendered(&running_lines).contains("running"));
        assert!(rendered(&running_lines).contains(crate::glyphs::MARK_RUNNING));

        // Completed: `✓` on the OKG success accent, with the `done` word (no `running`).
        let mut completed = tool_card("read", r#"{"path":"a.txt"}"#);
        completed.complete(&[protocol::ToolResultPart::text("ok")], false);
        let mut completed_lines = Vec::new();
        push_tool(&mut completed_lines, &completed, 80, false, &theme);
        assert_eq!(
            glyph_fg(&completed_lines[0], crate::glyphs::MARK_DONE),
            Some(theme.success)
        );
        assert!(rendered(&completed_lines).contains("done"));
        assert!(!rendered(&completed_lines).contains("running"));

        // Failed: `✗` on the ERR accent, with the `error` word.
        let mut failed = tool_card("read", r#"{"path":"a.txt"}"#);
        failed.complete(&[protocol::ToolResultPart::text("boom")], true);
        let mut failed_lines = Vec::new();
        push_tool(&mut failed_lines, &failed, 80, false, &theme);
        assert_eq!(
            glyph_fg(&failed_lines[0], crate::glyphs::MARK_ERROR),
            Some(theme.error)
        );
        assert!(rendered(&failed_lines).contains("error"));
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
        // The busy indicator is now the Working band above the composer, not a transcript line.
        assert!(!text.contains("Thinking"), "no transcript busy line:\n{text}");
        assert!(a.is_busy(), "the turn is streaming");
        let band = rendered(&[working_band_line(&a, 80, &style::ThemeColors::dark())]);
        assert!(band.contains("Working"), "working band present:\n{band}");
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
        // The Working band is gated on the busy state, so it is gone once the turn completes.
        assert!(!a.is_busy(), "turn completed, no working band");
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
            false,
            false,
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
    fn structured_markdown_renders_under_a_cortex_author_line() {
        // VAL-MD-002/004: a structured markdown answer renders under a `✦ Cortex` author line
        // (✦ AMBER, `Cortex` BRIGHT) with the markdown body beneath it; paragraph body text is TXT
        // and inline bold is distinctly BRIGHT (brighter than the surrounding body).
        let theme = style::ThemeColors::dark();
        let mut lines = Vec::new();
        let answer = "# Tool Registry\n\nThe registry resolves each tool to a **golden schema** before dispatch.";
        push_assistant(
            &mut lines,
            &AssistantTurn {
                reasoning: String::new(),
                answer: answer.to_owned(),
            },
            80,
            "dark",
            &theme,
            false,
            false,
        );

        // Author line: `✦ Cortex` — the star is AMBER (secondary) and `Cortex` is BRIGHT.
        let author = lines
            .iter()
            .find(|l| plain(l).contains("✦ Cortex"))
            .expect("`✦ Cortex` author line");
        let star = author
            .spans
            .iter()
            .find(|s| s.content.starts_with('✦'))
            .expect("star span");
        assert_eq!(star.style.fg, Some(theme.secondary), "star is AMBER");
        let name = author
            .spans
            .iter()
            .find(|s| s.content.as_ref() == "Cortex")
            .expect("Cortex label span");
        assert_eq!(name.style.fg, Some(theme.text_bright), "Cortex is BRIGHT");

        // Body paragraph is TXT; the inline bold run is BRIGHT (distinct from body TXT).
        let body_span = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .find(|s| s.content.contains("registry") || s.content.contains("The"))
            .expect("body prose span");
        assert_eq!(body_span.style.fg, Some(theme.text), "body prose is TXT");
        let bold = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .find(|s| s.content.contains("golden"))
            .expect("bold span");
        assert_eq!(bold.style.fg, Some(theme.text_bright), "bold run is BRIGHT");
        assert_ne!(
            body_span.style.fg, bold.style.fg,
            "bold is distinct from body prose"
        );
    }

    #[test]
    fn streaming_markdown_author_line_shows_synthesizing_status() {
        // VAL-MD-002: while a structured markdown answer is actively streaming, the author line
        // reads `✦ Cortex synthesizing answer…` (`Cortex` BRIGHT); once the turn completes the
        // status suffix drops away, leaving the bare `✦ Cortex` label.
        let theme = style::ThemeColors::dark();
        let turn = AssistantTurn {
            reasoning: String::new(),
            answer: "# Tool Registry\n\n- registry.rs\n- schema.rs".to_owned(),
        };

        let mut streaming_lines = Vec::new();
        push_assistant(&mut streaming_lines, &turn, 80, "dark", &theme, false, true);
        let author = streaming_lines
            .iter()
            .find(|l| plain(l).contains("✦ Cortex"))
            .expect("`✦ Cortex` author line");
        assert!(
            plain(author).contains("Cortex synthesizing answer…"),
            "streaming status suffix present: {:?}",
            plain(author)
        );
        let name = author
            .spans
            .iter()
            .find(|s| s.content.as_ref() == "Cortex")
            .expect("Cortex label span");
        assert_eq!(name.style.fg, Some(theme.text_bright), "Cortex is BRIGHT");

        let mut idle_lines = Vec::new();
        push_assistant(&mut idle_lines, &turn, 80, "dark", &theme, false, false);
        let idle_author = idle_lines
            .iter()
            .find(|l| plain(l).contains("✦ Cortex"))
            .expect("`✦ Cortex` author line");
        assert!(
            !plain(idle_author).contains("synthesizing"),
            "no status suffix once the turn completes: {:?}",
            plain(idle_author)
        );
    }

    #[test]
    fn plain_prose_reply_keeps_inline_star_without_cortex_label() {
        // Regression guard for VAL-CONV-018: a plain (non-structured) reply keeps the inline
        // `✦ <reply>` form (no `✦ Cortex` author line) with the body in BRIGHT.
        let theme = style::ThemeColors::dark();
        let mut lines = Vec::new();
        push_assistant(
            &mut lines,
            &AssistantTurn {
                reasoning: String::new(),
                answer: "Sure, I'll cap the buffer and trim the oldest chunks.".to_owned(),
            },
            80,
            "dark",
            &theme,
            false,
            false,
        );
        let text = rendered(&lines);
        assert!(!text.contains("✦ Cortex"), "no author label on plain prose:\n{text}");
        let star_line = lines
            .iter()
            .find(|l| plain(l).contains("✦ Sure"))
            .expect("inline `✦ <reply>` line");
        assert!(
            star_line
                .spans
                .iter()
                .any(|s| s.style.fg == Some(theme.text_bright)),
            "plain reply body is BRIGHT"
        );
    }

    #[test]
    fn markdown_answer_shows_chat_rendering_breadcrumb() {
        // VAL-MD-001: while a structured markdown answer is actively streaming, the screen wraps
        // the conversation in the shared chrome with a `✦ Cortex Code · chat · rendering` header
        // (✦ ORANGE, `Cortex Code` BRIGHT, ` · ` GHOST, trailing `rendering` AMBER). This is a
        // streaming-state affordance only (VAL-CONV-021/063 lock the static `▲ N lines above`
        // row-0 for a non-streaming/loaded conversation), so the turn must still be busy.
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut a = app();
        a.transcript.push(Entry::User("show me the tool registry".to_owned()));
        let (_tx, rx) = channel();
        a.begin_turn("show me the tool registry".to_owned(), rx, flag());
        if let Some(Entry::Assistant(turn)) = a.transcript.last_mut() {
            turn.answer.push_str("# Tool Registry\n\n- registry.rs\n- schema.rs");
        }
        assert!(a.is_busy(), "the turn is streaming");

        let theme = style::ThemeColors::from_name(a.theme());
        let backend = TestBackend::new(92, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &a)).unwrap();
        let buffer = terminal.backend().buffer().clone();

        let row0: String = (0..92).map(|x| buffer[(x, 0)].symbol()).collect::<Vec<_>>().concat();
        assert!(
            row0.contains("Cortex Code") && row0.contains("chat") && row0.contains("rendering"),
            "row 0 shows the chat·rendering breadcrumb: {row0:?}"
        );
        // The trailing `rendering` crumb is AMBER; `Cortex Code` is BRIGHT; the star is ORANGE.
        let cell_fg = |needle: char| {
            (0..92)
                .map(|x| buffer[(x, 0)].clone())
                .find(|c| c.symbol() == needle.to_string())
                .map(|c| c.fg)
        };
        assert_eq!(cell_fg('✦'), Some(theme.primary), "star is ORANGE");
        let r_cell = (0..92)
            .map(|x| buffer[(x, 0)].clone())
            .filter(|c| c.symbol() == "r")
            .find(|c| c.fg == theme.secondary);
        assert!(r_cell.is_some(), "the `rendering` crumb is AMBER");
    }

    #[test]
    fn static_loaded_markdown_conversation_keeps_the_locked_scroll_marker() {
        // VAL-MD-001 non-regression: a NON-streaming (static/loaded) conversation whose last entry
        // is a completed structured-markdown answer must NOT show the `chat · rendering` breadcrumb
        // — row 0 stays the locked `▲ N lines above` scroll marker (VAL-CONV-021/063) once the
        // transcript overflows the viewport.
        let mut a = app();
        for i in 0..12 {
            a.transcript.push(Entry::User(format!("question {i}")));
            a.transcript.push(Entry::Assistant(AssistantTurn {
                reasoning: String::new(),
                answer: format!("plain reply {i}"),
            }));
        }
        a.transcript.push(Entry::Assistant(AssistantTurn {
            reasoning: String::new(),
            answer: "# Tool Registry\n\n- registry.rs\n- schema.rs".to_owned(),
        }));
        assert!(!a.is_busy(), "the conversation is static, not streaming");

        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(92, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &a)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let row0: String = (0..92).map(|x| buffer[(x, 0)].symbol()).collect::<Vec<_>>().concat();
        assert!(
            !row0.contains("chat") && !row0.contains("rendering"),
            "no chat·rendering breadcrumb on a static conversation: {row0:?}"
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
            false,
            false,
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
            false,
            false,
        );
        let text = rendered(&lines);
        assert!(
            !text.contains("reasoning"),
            "no empty reasoning region:\n{text}"
        );
        assert!(text.contains("just the answer"));
    }

    #[test]
    fn streaming_reply_shows_blinking_tail_caret_while_busy() {
        // VAL-CONV-054: while a live turn streams, a single `▏` caret (ORANGE) trails the tail of
        // the latest assistant text. `begin_turn` opens the in-progress assistant entry; a delta
        // grows it. At turn start elapsed≈0 so the blink is in its visible phase.
        let mut a = app();
        let (_tx, rx) = channel();
        a.begin_turn("q".to_owned(), rx, flag());
        if let Some(Entry::Assistant(turn)) = a.transcript.last_mut() {
            turn.answer.push_str("Reading the processor");
        }
        assert!(a.is_busy(), "the turn is streaming");
        assert!(
            a.cursor_blink_visible(),
            "caret is in its visible phase at turn start"
        );

        let theme = style::ThemeColors::from_name(a.theme());
        let lines = transcript_lines(&a, 80);
        let caret_lines: Vec<&Line> = lines.iter().filter(|l| plain(l).contains('▏')).collect();
        assert_eq!(caret_lines.len(), 1, "exactly one streaming caret on screen");
        let line = caret_lines[0];
        assert!(
            plain(line).contains("Reading the processor"),
            "caret trails the assistant text: {:?}",
            plain(line)
        );
        let caret = line
            .spans
            .iter()
            .find(|s| s.content.as_ref() == "▏")
            .expect("a caret span");
        assert_eq!(caret.style.fg, Some(theme.primary), "caret is ORANGE");
        // The span immediately before the caret is the tail of the streamed text (no gap).
        let before = line
            .spans
            .iter()
            .rev()
            .nth(1)
            .expect("a span before the caret");
        assert!(
            before.content.ends_with('r'),
            "caret sits at the tail of the text: {:?}",
            before.content
        );
    }

    #[test]
    fn completed_reply_has_no_tail_caret() {
        // VAL-CONV-054 / VAL-CONV-058: the caret disappears once the turn completes (idle state).
        let mut a = app();
        a.transcript.push(Entry::User("q".to_owned()));
        a.transcript.push(Entry::Assistant(AssistantTurn {
            reasoning: String::new(),
            answer: "All done.".to_owned(),
        }));
        assert!(!a.is_busy(), "no turn in progress");
        let lines = transcript_lines(&a, 80);
        assert!(
            !rendered(&lines).contains('▏'),
            "no streaming caret when the turn is complete"
        );
    }

    #[test]
    fn reduced_motion_streaming_caret_is_present_and_solid() {
        // VAL-CONV-058 / VAL-FOUND-023: under --no-anim the tail caret is shown but solid (its
        // blink state is always visible, so it never toggles off between frames).
        let mut a = app();
        a.set_reduced_motion(true);
        let (_tx, rx) = channel();
        a.begin_turn("q".to_owned(), rx, flag());
        if let Some(Entry::Assistant(turn)) = a.transcript.last_mut() {
            turn.answer.push_str("streaming under no-anim");
        }
        assert!(a.is_busy());
        assert!(
            a.cursor_blink_visible(),
            "the caret is always solid under reduced motion"
        );
        let lines = transcript_lines(&a, 80);
        let carets = lines.iter().filter(|l| plain(l).contains('▏')).count();
        assert_eq!(
            carets, 1,
            "a solid caret trails the streaming text under --no-anim"
        );
    }

    #[test]
    fn running_tool_card_supersedes_the_stream_caret() {
        // When the in-progress turn is executing a tool (the last entry is a running tool card, not
        // the assistant answer), the streaming caret is not drawn on the earlier assistant text.
        let mut a = app();
        let (_tx, rx) = channel();
        a.begin_turn("q".to_owned(), rx, flag());
        if let Some(Entry::Assistant(turn)) = a.transcript.last_mut() {
            turn.answer.push_str("Let me read the file.");
        }
        a.transcript.push(Entry::Tool(crate::tool_card::ToolCard::new(
            "t1".to_owned(),
            "bash".to_owned(),
            r#"{"command":"ls"}"#,
        )));
        assert!(a.is_busy());
        let lines = transcript_lines(&a, 80);
        assert!(
            !rendered(&lines).contains('▏'),
            "no stream caret while a running tool card is the transcript tail"
        );
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

    /// The `(text, fg)` of the top-border label span (the span holding the tool name) of a card,
    /// for label-accent assertions.
    fn card_label_span(lines: &[Line]) -> (String, Option<Color>) {
        let top = &lines[0];
        let span = top
            .spans
            .iter()
            .find(|s| s.content.contains(crate::glyphs::SEPARATOR) || s.content.trim().len() > 2)
            .expect("a label span on the top border");
        (span.content.to_string(), span.style.fg)
    }

    #[test]
    fn running_tool_card_is_a_closed_box_with_running_label() {
        // VAL-TOOL-001/002/003: the card is a closed rounded rectangle whose top border carries
        // `⚙ read · running` in the ORANGE accent; the summary is the target path.
        let theme = style::ThemeColors::dark();
        let card = tool_card("read", r#"{"path":"src/main.rs"}"#);
        let mut lines = Vec::new();
        push_tool(&mut lines, &card, 80, false, &theme);
        let text = rendered(&lines);
        // Top-border label `⚙ read · running`.
        let top = plain(&lines[0]);
        assert!(top.contains(crate::glyphs::BOX_TOP_LEFT), "top-left corner:\n{top}");
        assert!(top.contains(crate::glyphs::BOX_TOP_RIGHT), "top-right corner:\n{top}");
        assert!(top.contains(crate::glyphs::MARK_RUNNING), "gear glyph:\n{top}");
        assert!(top.contains("read"), "tool name:\n{top}");
        assert!(top.contains("running"), "state word:\n{top}");
        // The label rides the border in the ORANGE accent.
        let (label, fg) = card_label_span(&lines);
        assert!(label.contains("running"), "label text: {label:?}");
        assert_eq!(fg, Some(theme.primary), "running label is ORANGE");
        // Summary is the target path.
        assert!(text.contains("src/main.rs"), "target summary:\n{text}");
        // Bottom border closes the box.
        assert!(
            plain(lines.last().unwrap()).is_empty(),
            "trailing gap row after the card"
        );
        assert!(
            lines.iter().any(|l| plain(l).contains(crate::glyphs::BOX_BOTTOM_LEFT)),
            "bottom-left corner:\n{text}"
        );
    }

    #[test]
    fn completed_tool_card_shows_done_state() {
        // VAL-TOOL-004: a completed card carries `✓ grep · done` in the OKG accent, no `running`.
        let theme = style::ThemeColors::dark();
        let mut card = tool_card("grep", r#"{"pattern":"TODO","path":"src"}"#);
        card.complete(&[protocol::ToolResultPart::text("src/a.rs:1: TODO")], false);
        let mut lines = Vec::new();
        push_tool(&mut lines, &card, 80, false, &theme);
        let top = plain(&lines[0]);
        assert!(top.contains("grep"), "tool name:\n{top}");
        assert!(top.contains(crate::glyphs::MARK_DONE), "done check:\n{top}");
        assert!(top.contains("done"), "done word:\n{top}");
        assert!(!top.contains("running"), "no longer running:\n{top}");
        let (_, fg) = card_label_span(&lines);
        assert_eq!(fg, Some(theme.success), "done label is OKG-green");
    }

    #[test]
    fn collapsed_card_summary_then_expand_hint_no_payload() {
        // VAL-TOOL-008/009: the first interior row is the summary (TXT), followed by a FAINT
        // `Ctrl+O to expand` hint and NO output payload.
        let theme = style::ThemeColors::dark();
        let mut card = tool_card("read", r#"{"path":"stream_processor.rs"}"#);
        card.complete(
            &[protocol::ToolResultPart::text("1: use ratatui;\n2: fn main() {}")],
            false,
        );
        let mut lines = Vec::new();
        push_tool(&mut lines, &card, 92, false, &theme);
        // Summary row is the target path in TXT.
        let summary = &lines[1];
        let summary_text: String = summary.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(summary_text.contains("stream_processor.rs"), "summary: {summary_text:?}");
        let path_span = summary
            .spans
            .iter()
            .find(|s| s.content.contains("stream_processor.rs"))
            .expect("summary span");
        assert_eq!(path_span.style.fg, Some(theme.text), "summary is TXT");
        // Expand hint in FAINT, and NO payload.
        let text = rendered(&lines);
        assert!(text.contains("Ctrl+O to expand"), "expand hint:\n{text}");
        let hint_span = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .find(|s| s.content.contains("Ctrl+O to expand"))
            .expect("hint span");
        assert_eq!(hint_span.style.fg, Some(theme.text_muted), "hint is FAINT");
        assert!(!text.contains("output"), "no output header while collapsed:\n{text}");
        assert!(!text.contains("use ratatui"), "no payload while collapsed:\n{text}");
    }

    #[test]
    fn expanded_card_shows_output_header_and_payload() {
        // VAL-TOOL-010: expanding replaces the hint with a DIM `output` header + payload rows (TXT).
        let theme = style::ThemeColors::dark();
        let mut card = tool_card("read", r#"{"path":"a.txt"}"#);
        card.complete(
            &[protocol::ToolResultPart::text("1: use ratatui;\n2: fn main() {}")],
            false,
        );
        let mut lines = Vec::new();
        push_tool(&mut lines, &card, 92, true, &theme);
        let text = rendered(&lines);
        assert!(text.contains("output"), "output header:\n{text}");
        let header_span = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .find(|s| s.content.contains("output"))
            .expect("output header span");
        assert_eq!(header_span.style.fg, Some(theme.text_dim), "output header is DIM");
        assert!(text.contains("use ratatui"), "payload row revealed:\n{text}");
        assert!(!text.contains("Ctrl+O to expand"), "no hint while expanded:\n{text}");
    }

    #[test]
    fn truncated_output_ends_in_dim_overflow_line() {
        // VAL-TOOL-011: an over-long expanded output ends in a DIM `… (N more lines)` row.
        let theme = style::ThemeColors::dark();
        let mut card = tool_card("read", r#"{"path":"a.txt"}"#);
        let body: String = (1..=MAX_TOOL_DETAIL_LINES + 10)
            .map(|i| format!("{i}: line"))
            .collect::<Vec<_>>()
            .join("\n");
        card.complete(&[protocol::ToolResultPart::text(&body)], false);
        let mut lines = Vec::new();
        push_tool(&mut lines, &card, 92, true, &theme);
        let overflow = lines
            .iter()
            .find(|l| plain(l).contains("more lines"))
            .expect("an overflow row");
        let over_text = plain(overflow);
        assert!(over_text.contains(crate::glyphs::ELLIPSIS), "ellipsis prefix: {over_text:?}");
        assert!(over_text.contains("(10 more lines)"), "count: {over_text:?}");
        let over_span = overflow
            .spans
            .iter()
            .find(|s| s.content.contains("more lines"))
            .expect("overflow span");
        assert_eq!(over_span.style.fg, Some(theme.text_dim), "overflow is DIM");
    }

    #[test]
    fn card_body_is_clipped_inside_the_right_bar() {
        // VAL-TOOL-012: a very long summary is clipped (with `…`) so no content touches or overruns
        // the right `│` bar; the card stays rectangular.
        let theme = style::ThemeColors::dark();
        let long = "x".repeat(500);
        let args = serde_json::json!({ "command": long }).to_string();
        let mut card = tool_card("bash", &args);
        card.complete(&[protocol::ToolResultPart::text("ok")], false);
        let mut lines = Vec::new();
        push_tool(&mut lines, &card, 92, false, &theme);
        // Every rendered card line is exactly the same display width and ends in a border/space,
        // never in payload text bleeding past the right bar.
        let widths: Vec<usize> = lines
            .iter()
            .filter(|l| !plain(l).is_empty())
            .map(|l| plain(l).chars().count())
            .collect();
        // The right border sits at column `width - CONTENT_MARGIN - 1`; all box rows share it.
        let border_col = 92 - CONTENT_MARGIN - 1;
        for line in lines.iter().filter(|l| !plain(l).is_empty()) {
            let chars: Vec<char> = plain(line).chars().collect();
            assert!(
                chars.get(border_col) == Some(&'│')
                    || chars.get(border_col) == Some(&'╮')
                    || chars.get(border_col) == Some(&'╯'),
                "right bar at col {border_col}: {:?}",
                plain(line)
            );
        }
        assert!(!widths.is_empty(), "the card rendered some rows");
    }

    /// A completed `edit` card with a real diff attached (1 deletion, 2 additions), for the
    /// expandable diff-card tests.
    fn diff_card() -> ToolCard {
        let before = "line1\nline2\nline3\nold_a\nline5\n";
        let after = "line1\nline2\nline3\nnew_a\nnew_b\nline5\n";
        let diff = crate::diff::compute_file_diff("stream_processor.rs", Some(before), Some(after))
            .expect("a diff");
        let mut card = tool_card(
            "edit",
            r#"{"path":"stream_processor.rs","oldString":"old_a","newString":"new_a\nnew_b"}"#,
        );
        card.complete(
            &[protocol::ToolResultPart::text("edit: replaced 1 occurrence")],
            false,
        );
        card.duration_ms = Some(22);
        card.attach_diff(diff);
        card
    }

    #[test]
    fn collapsed_diff_card_shows_summary_and_no_diff_body() {
        // A collapsed diff-carrying card shows the summary + expand hint and renders NO diff rows.
        let theme = style::ThemeColors::dark();
        let card = diff_card();
        let mut lines = Vec::new();
        push_tool(&mut lines, &card, 92, false, &theme);
        let text = rendered(&lines);
        assert!(text.contains("stream_processor.rs"), "summary target:\n{text}");
        assert!(text.contains("Ctrl+O to expand"), "expand hint:\n{text}");
        assert!(
            !text.contains("new_a") && !text.contains("old_a"),
            "collapsed hides the diff body:\n{text}"
        );
    }

    #[test]
    fn expanded_diff_card_reveals_line_numbered_diff_rows() {
        // VAL-TOOL-031: an expanded diff card reveals red/green line-numbered rows *inside* the
        // card box (not a separate frame), with the diff +/- coloring confined to the payload.
        let theme = style::ThemeColors::dark();
        let card = diff_card();
        let mut lines = Vec::new();
        push_tool(&mut lines, &card, 92, true, &theme);
        let text = rendered(&lines);
        assert!(text.contains("output"), "output header:\n{text}");
        assert!(text.contains("new_a") && text.contains("new_b"), "additions shown:\n{text}");
        assert!(text.contains("old_a"), "deletion shown:\n{text}");
        // A row carries a right-justified line number in the LNUM tone.
        let has_lnum = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .any(|s| s.style.fg == Some(style::LNUM) && s.content.trim().parse::<usize>().is_ok());
        assert!(has_lnum, "a diff row shows a LNUM line number:\n{text}");
    }

    #[test]
    fn expanded_diff_rows_use_red_and_green_backgrounds() {
        // The only non-mono-orange coloring is inside the expanded diff payload: deletion rows use
        // DEL_FG/DEL_BG, addition rows ADD_FG/ADD_BG, context rows carry no diff background.
        let theme = style::ThemeColors::dark();
        let card = diff_card();
        let mut lines = Vec::new();
        push_tool(&mut lines, &card, 92, true, &theme);

        let added = lines
            .iter()
            .find(|l| plain(l).contains("new_a"))
            .expect("an addition row");
        let added_code = added
            .spans
            .iter()
            .find(|s| s.content.contains("new_a"))
            .expect("added code span");
        assert_eq!(added_code.style.fg, Some(style::ADD_FG), "addition text ADD_FG");
        assert_eq!(added_code.style.bg, Some(style::ADD_BG), "addition bg ADD_BG");

        let removed = lines
            .iter()
            .find(|l| plain(l).contains("old_a"))
            .expect("a deletion row");
        let removed_code = removed
            .spans
            .iter()
            .find(|s| s.content.contains("old_a"))
            .expect("removed code span");
        assert_eq!(removed_code.style.fg, Some(style::DEL_FG), "deletion text DEL_FG");
        assert_eq!(removed_code.style.bg, Some(style::DEL_BG), "deletion bg DEL_BG");

        // A context row (e.g. `line1`) has no red/green background.
        let context = lines
            .iter()
            .find(|l| plain(l).contains("line1"))
            .expect("a context row");
        for span in &context.spans {
            assert_ne!(span.style.bg, Some(style::ADD_BG), "context has no ADD_BG");
            assert_ne!(span.style.bg, Some(style::DEL_BG), "context has no DEL_BG");
        }
    }

    #[test]
    fn card_label_accent_is_stable_across_toggle() {
        // VAL-TOOL-032: toggling collapsed<->expanded keeps the top-border label (glyph + name +
        // state accent) identical; only the body below changes.
        let theme = style::ThemeColors::dark();
        let card = diff_card();
        let mut collapsed = Vec::new();
        push_tool(&mut collapsed, &card, 92, false, &theme);
        let mut expanded = Vec::new();
        push_tool(&mut expanded, &card, 92, true, &theme);

        assert_eq!(
            plain(&collapsed[0]),
            plain(&expanded[0]),
            "top-border label is stable across the toggle"
        );
        assert_eq!(
            card_label_span(&collapsed).1,
            card_label_span(&expanded).1,
            "label accent is stable across the toggle"
        );
        // The expanded form is strictly taller (it adds the diff body).
        assert!(
            expanded.len() > collapsed.len(),
            "expanded adds a diff body: {} vs {}",
            expanded.len(),
            collapsed.len()
        );
    }

    #[test]
    fn tool_card_expands_to_reveal_output_then_collapses() {
        // VAL-TOOL-009/010/032: collapsed hides the raw output behind the summary + hint; expanding
        // (Ctrl+O) reveals the full output under an `output` header, and the two forms round-trip.
        let mut card = tool_card("read", r#"{"path":"a.txt"}"#);
        card.complete(
            &[protocol::ToolResultPart::text("1: secret detail line")],
            false,
        );

        let mut collapsed = Vec::new();
        push_tool(&mut collapsed, &card, 80, false, &style::ThemeColors::dark());
        let collapsed_text = rendered(&collapsed);
        assert!(
            !collapsed_text.contains("secret detail line"),
            "collapsed hides output:\n{collapsed_text}"
        );
        assert!(
            collapsed_text.contains("Ctrl+O to expand"),
            "collapsed shows the expand hint:\n{collapsed_text}"
        );

        let mut expanded = Vec::new();
        push_tool(&mut expanded, &card, 80, true, &style::ThemeColors::dark());
        let expanded_text = rendered(&expanded);
        assert!(
            expanded_text.contains("secret detail line"),
            "expanded reveals output:\n{expanded_text}"
        );
        assert!(
            expanded_text.contains("a.txt"),
            "expanded still shows the summary target:\n{expanded_text}"
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
        // VAL-TOOL-005: a failed call carries `✗ read · error` in the ERR accent, distinct in hue
        // from a completed card, and surfaces the failure message in the expanded output.
        let theme = style::ThemeColors::dark();
        let mut failed = tool_card("read", r#"{"path":"nope.txt"}"#);
        failed.complete(
            &[protocol::ToolResultPart::text("file not found: nope.txt")],
            true,
        );
        let mut lines = Vec::new();
        push_tool(&mut lines, &failed, 80, true, &theme);
        let text = rendered(&lines);
        let top = plain(&lines[0]);
        assert!(top.contains(crate::glyphs::MARK_ERROR), "error mark:\n{top}");
        assert!(top.contains("error"), "error word:\n{top}");
        assert!(
            text.contains("file not found"),
            "failure message shown in the payload:\n{text}"
        );
        assert_eq!(card_label_span(&lines).1, Some(theme.error), "error label is ERR");

        let mut ok = tool_card("read", r#"{"path":"a.txt"}"#);
        ok.complete(&[protocol::ToolResultPart::text("ok")], false);
        let mut ok_lines = Vec::new();
        push_tool(&mut ok_lines, &ok, 80, false, &theme);
        assert_eq!(
            card_label_span(&ok_lines).1,
            Some(theme.success),
            "a completed card's label is OKG (distinct from error)"
        );
    }

    #[test]
    fn task_subagent_card_summary_and_expanded_result() {
        // VAL-TOOL-028: a `task` (subagent) card's summary is the delegated objective, and its
        // expanded output is the subagent's result summary followed by a `… (N more lines)` row.
        let theme = style::ThemeColors::dark();
        let mut card = tool_card(
            "task",
            r#"{"subagent_type":"explore","prompt":"where are tool cards rendered?"}"#,
        );
        let mut result = String::from(
            "push_tool() in crates/tui/src/ui.rs builds the card lines;\n\
             the data model is crates/tui/src/tool_card.rs.\n",
        );
        for i in 0..MAX_TOOL_DETAIL_LINES {
            result.push_str(&format!("detail line {i}\n"));
        }
        card.complete(&[protocol::ToolResultPart::text(&result)], false);

        // Collapsed: `✓ task · done` header + the objective summary.
        let mut collapsed = Vec::new();
        push_tool(&mut collapsed, &card, 92, false, &theme);
        let top = plain(&collapsed[0]);
        assert!(top.contains("task") && top.contains("done"), "task done header:\n{top}");
        assert!(
            rendered(&collapsed).contains("explore: where are tool cards rendered?"),
            "summary is the delegated objective:\n{}",
            rendered(&collapsed)
        );

        // Expanded: `output` header, the result summary lines, and a `… (N more lines)` overflow row.
        let mut expanded = Vec::new();
        push_tool(&mut expanded, &card, 92, true, &theme);
        let text = rendered(&expanded);
        assert!(text.contains("output"), "output header:\n{text}");
        assert!(text.contains("push_tool()"), "result summary shown:\n{text}");
        assert!(
            text.contains("more lines)"),
            "overflow row summarizes the remaining lines:\n{text}"
        );
    }

    #[test]
    fn lsp_failure_renders_red_error_card_with_inline_diagnostic() {
        // VAL-TOOL-029: a failed `lsp` card shows the ERR chrome (`✗ lsp · error`), a
        // `diagnostics {path}` summary, an ERR-red `error` header, the first diagnostic line in ERR
        // red, and a FAINT `Ctrl+O for detail` hint — all while collapsed.
        let theme = style::ThemeColors::dark();
        let mut card = tool_card("lsp", r#"{"action":"documentSymbols","filePath":"crates/tui"}"#);
        card.complete(
            &[protocol::ToolResultPart::text(
                "ui.rs:560:21 mismatched types: expected &str, found String\nsecond diagnostic line",
            )],
            true,
        );
        assert_eq!(card.summary, "diagnostics crates/tui");

        let mut lines = Vec::new();
        push_tool(&mut lines, &card, 92, false, &theme);

        // Header: `✗ lsp · error` in the ERR accent.
        let top = plain(&lines[0]);
        assert!(top.contains(crate::glyphs::MARK_ERROR), "error mark:\n{top}");
        assert!(top.contains("lsp") && top.contains("error"), "lsp error header:\n{top}");
        assert_eq!(card_label_span(&lines).1, Some(theme.error), "error label is ERR red");

        // Summary + inline error preview.
        let text = rendered(&lines);
        assert!(text.contains("diagnostics crates/tui"), "summary:\n{text}");
        // The `error` header row is ERR red.
        let error_header = lines
            .iter()
            .find(|l| plain(l).trim() == format!("{} error", crate::glyphs::BOX_VERTICAL).trim());
        let error_span_fg = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .find(|s| s.content.trim() == "error")
            .and_then(|s| s.style.fg);
        assert!(error_header.is_some() || error_span_fg.is_some(), "an `error` header row:\n{text}");
        assert_eq!(error_span_fg, Some(theme.error), "`error` header is ERR red");
        // The first diagnostic line is shown in ERR red.
        let diag_fg = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .find(|s| s.content.contains("mismatched types"))
            .and_then(|s| s.style.fg);
        assert_eq!(diag_fg, Some(theme.error), "first diagnostic line is ERR red:\n{text}");
        // A FAINT `Ctrl+O for detail` hint, not the plain expand hint.
        assert!(text.contains("Ctrl+O for detail"), "detail hint:\n{text}");
        assert!(!text.contains("Ctrl+O to expand"), "not the plain expand hint:\n{text}");
        assert!(!crate::glyphs::contains_braille(&text), "no braille:\n{text}");
    }

    #[test]
    fn question_running_card_awaits_the_answer() {
        // VAL-TOOL-030: a running `question` card carries the running chrome (`⚙ question ·
        // running`), a summary that is the question text, and a DIM/FAINT awaiting-answer line.
        let theme = style::ThemeColors::dark();
        let card = tool_card(
            "question",
            r#"{"questions":[{"prompt":"Default provider: Anthropic or OpenAI?","options":["Anthropic","OpenAI"]}]}"#,
        );
        // A question card is surfaced while its call is still running (awaiting the answer).
        assert_eq!(card.status, ToolStatus::Running);

        let mut lines = Vec::new();
        push_tool(&mut lines, &card, 92, false, &theme);

        // Header: `⚙ question · running` in the ORANGE (primary) accent.
        let top = plain(&lines[0]);
        assert!(top.contains(crate::glyphs::MARK_RUNNING), "gear glyph:\n{top}");
        assert!(top.contains("question") && top.contains("running"), "running header:\n{top}");
        assert_eq!(card_label_span(&lines).1, Some(theme.primary), "running label is ORANGE");

        // Summary is the question; body is the awaiting-answer line in DIM (no expand hint).
        let text = rendered(&lines);
        assert!(
            text.contains("Default provider: Anthropic or OpenAI?"),
            "summary is the question:\n{text}"
        );
        let awaiting_fg = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .find(|s| s.content.contains("waiting for your answer"))
            .and_then(|s| s.style.fg);
        assert_eq!(awaiting_fg, Some(theme.text_dim), "awaiting line is DIM:\n{text}");
        assert!(!text.contains("Ctrl+O to expand"), "no expand hint while awaiting:\n{text}");
        assert!(!crate::glyphs::contains_braille(&text), "no braille:\n{text}");
    }

    #[test]
    fn card_frame_uses_only_safe_glyphs() {
        // VAL-TOOL-007: the card frame uses only DejaVu-safe glyphs — rounded box drawing plus the
        // status marks — with no braille anywhere (in particular a running card uses `⚙`, never a
        // braille spinner frame).
        let theme = style::ThemeColors::dark();
        for (name, args, complete_err) in [
            ("bash", r#"{"command":"cargo build"}"#, None),
            ("read", r#"{"path":"a.txt"}"#, Some(false)),
            ("read", r#"{"path":"a.txt"}"#, Some(true)),
        ] {
            let mut card = tool_card(name, args);
            if let Some(is_err) = complete_err {
                card.complete(&[protocol::ToolResultPart::text("out")], is_err);
            }
            let mut lines = Vec::new();
            push_tool(&mut lines, &card, 92, false, &theme);
            let text = rendered(&lines);
            assert!(
                !crate::glyphs::contains_braille(&text),
                "no braille in a card frame:\n{text}"
            );
            assert!(!text.contains('▸') && !text.contains('▾'), "no thin triangles:\n{text}");
        }
    }

    #[test]
    fn cards_stack_as_separate_boxes_in_transcript_order() {
        // VAL-TOOL-013: multiple tool cards in one turn render as separate stacked closed boxes in
        // invocation order, each its own frame (not merged).
        let theme = style::ThemeColors::dark();
        let mut first = tool_card("read", r#"{"path":"first.rs"}"#);
        first.complete(&[protocol::ToolResultPart::text("a")], false);
        let mut second = tool_card("grep", r#"{"pattern":"TODO","path":"second"}"#);
        second.complete(&[protocol::ToolResultPart::text("second:1: TODO")], false);
        let mut lines = Vec::new();
        push_tool(&mut lines, &first, 92, false, &theme);
        push_tool(&mut lines, &second, 92, false, &theme);
        let text = rendered(&lines);
        // Two top borders and two bottom borders → two distinct closed boxes.
        let tops = lines.iter().filter(|l| plain(l).contains(crate::glyphs::BOX_TOP_LEFT)).count();
        let bottoms = lines
            .iter()
            .filter(|l| plain(l).contains(crate::glyphs::BOX_BOTTOM_LEFT))
            .count();
        assert_eq!(tops, 2, "two top borders:\n{text}");
        assert_eq!(bottoms, 2, "two bottom borders:\n{text}");
        // Order preserved: `first.rs` appears before `second`.
        let first_at = text.find("first.rs").expect("first card present");
        let second_at = text.find("second").expect("second card present");
        assert!(first_at < second_at, "cards stack in invocation order:\n{text}");
    }

    #[test]
    fn read_card_summary_is_path_and_expanded_output_is_numbered() {
        // VAL-TOOL-015: a read card's summary is the target path, and its expanded output payload is
        // the real tool's numbered source lines (`N: …`) with a `… (N more lines)` overflow row.
        let theme = style::ThemeColors::dark();
        let mut card = tool_card("read", r#"{"path":"crates/tui/src/ui.rs"}"#);
        assert_eq!(card.summary, "crates/tui/src/ui.rs");
        let body: String = (1..=MAX_TOOL_DETAIL_LINES + 5)
            .map(|i| format!("{i}: source line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        card.complete(&[protocol::ToolResultPart::text(&body)], false);
        let mut lines = Vec::new();
        push_tool(&mut lines, &card, 92, true, &theme);
        let text = rendered(&lines);
        assert!(text.contains("crates/tui/src/ui.rs"), "summary path:\n{text}");
        assert!(text.contains("1: source line 1"), "numbered rows:\n{text}");
        assert!(text.contains("2: source line 2"), "numbered rows:\n{text}");
        assert!(text.contains("(5 more lines)"), "read overflow reads lines:\n{text}");
    }

    #[test]
    fn edit_write_apply_patch_summaries_are_the_file_path() {
        // VAL-TOOL-016/017/018: the edit/write/apply_patch summary row is the edited/written/patched
        // file path (the collapsed form shows that path + the expand hint).
        let theme = style::ThemeColors::dark();
        let patch = "*** Update File: crates/engine/src/stream.rs\n@@\n-old\n+new";
        let cases = [
            (
                "edit",
                r#"{"path":"crates/engine/src/runner.rs","oldString":"a","newString":"b"}"#
                    .to_owned(),
                "crates/engine/src/runner.rs",
            ),
            (
                "write",
                r#"{"path":"crates/engine/tests/cap.rs","content":"x\ny"}"#.to_owned(),
                "crates/engine/tests/cap.rs",
            ),
            (
                "apply_patch",
                serde_json::json!({ "patch": patch }).to_string(),
                "crates/engine/src/stream.rs",
            ),
        ];
        for (name, args, expected_path) in cases {
            let mut card = tool_card(name, &args);
            card.complete(&[protocol::ToolResultPart::text("ok")], false);
            assert_eq!(card.summary, expected_path, "{name} summary is the file path");
            let mut lines = Vec::new();
            push_tool(&mut lines, &card, 92, false, &theme);
            let summary_text = plain(&lines[1]);
            assert!(
                summary_text.contains(expected_path),
                "{name} summary row shows the path:\n{summary_text}"
            );
            assert!(
                rendered(&lines).contains("Ctrl+O to expand"),
                "{name} collapsed shows the expand hint"
            );
        }
    }

    #[test]
    fn apply_patch_summary_resolves_every_patch_key_alias() {
        // VAL-TOOL-018: the patched-file summary holds regardless of which patch key alias the model
        // uses (the tool accepts patch/patchText/patch_text/input).
        let patch = "*** Add File: src/new_mod.rs\n@@\n+content";
        for key in ["patch", "patchText", "patch_text", "input"] {
            let args = serde_json::json!({ key: patch }).to_string();
            let card = tool_card("apply_patch", &args);
            assert_eq!(card.summary, "src/new_mod.rs", "alias {key} resolves the target");
        }
    }

    #[test]
    fn grep_card_summary_and_expanded_rows_are_path_line_match() {
        // VAL-TOOL-019: a grep card's summary reads `{pattern} in {path}`, and its expanded output
        // lists the real `path:line: matched-text` hit rows.
        let theme = style::ThemeColors::dark();
        let mut card = tool_card("grep", r#"{"pattern":"ToolStatus","path":"crates/tui"}"#);
        assert_eq!(card.summary, "ToolStatus in crates/tui");
        card.complete(
            &[protocol::ToolResultPart::text(
                "crates/tui/src/ui.rs:20: use crate::tool_card::ToolStatus;\n\
                 crates/tui/src/tool_card.rs:16: pub enum ToolStatus {",
            )],
            false,
        );
        let mut lines = Vec::new();
        push_tool(&mut lines, &card, 92, true, &theme);
        let text = rendered(&lines);
        assert!(text.contains("ToolStatus in crates/tui"), "grep summary:\n{text}");
        assert!(
            text.contains("crates/tui/src/ui.rs:20: use crate::tool_card::ToolStatus;"),
            "path:line: match row:\n{text}"
        );
        assert!(
            text.contains("crates/tui/src/tool_card.rs:16: pub enum ToolStatus {"),
            "second hit row:\n{text}"
        );
    }

    #[test]
    fn glob_card_summary_is_pattern_and_overflow_reads_matches() {
        // VAL-TOOL-020: a glob card's summary is the glob pattern, its expanded output lists matched
        // paths, and a truncated body ends in a `… (N more matches)` overflow row (not `lines`).
        let theme = style::ThemeColors::dark();
        let mut card = tool_card("glob", r#"{"pattern":"**/*.rs"}"#);
        assert_eq!(card.summary, "**/*.rs");
        let body: String = (0..MAX_TOOL_DETAIL_LINES + 8)
            .map(|i| format!("crates/pkg{i}/src/lib.rs"))
            .collect::<Vec<_>>()
            .join("\n");
        card.complete(&[protocol::ToolResultPart::text(&body)], false);
        let mut lines = Vec::new();
        push_tool(&mut lines, &card, 92, true, &theme);
        let text = rendered(&lines);
        assert!(text.contains("**/*.rs"), "glob summary is the pattern:\n{text}");
        assert!(text.contains("crates/pkg0/src/lib.rs"), "matched-path rows:\n{text}");
        let overflow = lines
            .iter()
            .find(|l| plain(l).contains("more matches"))
            .expect("a `more matches` overflow row");
        let over_text = plain(overflow);
        assert!(over_text.contains(crate::glyphs::ELLIPSIS), "ellipsis prefix: {over_text:?}");
        assert!(over_text.contains("(8 more matches)"), "match count: {over_text:?}");
        assert!(!text.contains("more lines"), "glob overflow noun is matches, not lines:\n{text}");
    }

    #[test]
    fn overflow_noun_is_per_tool() {
        // VAL-TOOL-011/020/026: the overflow unit is chosen per tool.
        assert_eq!(overflow_noun("glob"), "matches");
        assert_eq!(overflow_noun("list"), "matches");
        assert_eq!(overflow_noun("websearch"), "results");
        assert_eq!(overflow_noun("read"), "lines");
        assert_eq!(overflow_noun("grep"), "lines");
        assert_eq!(overflow_noun("bash"), "lines");
    }

    #[test]
    fn bash_card_summary_is_command_line() {
        // VAL-TOOL-021: a bash card's summary row is the executed command line.
        let card = tool_card("bash", r#"{"command":"cargo test --workspace"}"#);
        assert_eq!(card.summary, "cargo test --workspace");
    }

    #[test]
    fn bash_card_completed_shows_green_ok_line() {
        // VAL-TOOL-022: a done bash card shows captured output with a success line
        // (`test result: ok`) colored OKG green (the sanctioned output-coloring exception).
        let theme = style::ThemeColors::dark();
        let mut card = tool_card("bash", r#"{"command":"cargo test"}"#);
        card.complete(
            &[protocol::ToolResultPart::text(
                "running 3 tests\ntest result: ok. 3 passed; 0 failed",
            )],
            false,
        );
        let mut lines = Vec::new();
        push_tool(&mut lines, &card, 92, true, &theme);
        let text = rendered(&lines);
        assert!(text.contains("test result: ok"), "success line present:\n{text}");
        
        // The "test result: ok" line is the only OKG-green span in the output.
        let ok_span = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .find(|s| s.content.contains("test result: ok"))
            .expect("success line span");
        assert_eq!(
            ok_span.style.fg,
            Some(theme.success),
            "success line is OKG green"
        );
        
        // Other output lines are TXT, not green.
        let running_span = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .find(|s| s.content.contains("running 3 tests"))
            .expect("ordinary log span");
        assert_eq!(running_span.style.fg, Some(theme.text), "log line is TXT");
    }

    #[test]
    fn background_bash_card_shows_background_in_summary_and_special_hint() {
        // VAL-TOOL-023: a background bash card carries the running chrome, its summary marks the
        // job as "(background)", and its footer hint reads "Ctrl+O to expand · esc to background".
        let theme = style::ThemeColors::dark();
        let card = tool_card("bash", r#"{"command":"cargo build --release","background":true}"#);
        assert_eq!(card.summary, "cargo build --release (background)");
        assert_eq!(card.status, ToolStatus::Running);
        
        let mut lines = Vec::new();
        push_tool(&mut lines, &card, 92, false, &theme);
        let text = rendered(&lines);
        assert!(
            text.contains("cargo build --release (background)"),
            "summary includes (background):\n{text}"
        );
        assert!(
            text.contains("esc to background"),
            "footer hint mentions esc to background:\n{text}"
        );
        assert!(
            text.contains(glyphs::MARK_RUNNING),
            "running card has the gear glyph:\n{text}"
        );
    }

    #[test]
    fn live_background_bash_card_streams_tailed_output_rows_in_the_collapsed_body() {
        // VAL-TOOL-023: a live background bash card (running, with a tracked pid) streams its
        // in-progress output rows in the collapsed body, above the `esc to background` footer.
        use std::path::PathBuf;
        let theme = style::ThemeColors::dark();
        let mut card = tool_card("bash", r#"{"command":"cargo build --release","background":true}"#);
        card.start_background(4242, PathBuf::from("/tmp/cortex-bg.log"));
        card.output = "Compiling cortex v0.1.0\n   Compiling deps\n    Building [===>   ]".to_owned();
        assert_eq!(card.status, ToolStatus::Running);

        let mut lines = Vec::new();
        push_tool(&mut lines, &card, 92, false, &theme);
        let text = rendered(&lines);
        assert!(
            text.contains("Building [===>"),
            "the collapsed body shows the latest streamed output row:\n{text}"
        );
        assert!(
            text.contains("esc to background"),
            "the streaming card keeps the background footer hint:\n{text}"
        );
        assert!(
            text.contains(glyphs::MARK_RUNNING),
            "the streaming card keeps the running gear glyph:\n{text}"
        );
    }

    #[test]
    fn todo_card_summary_is_item_count_and_in_progress_count() {
        // VAL-TOOL-024: a todo card's summary row is "{N} items · {M} in progress".
        let theme = style::ThemeColors::dark();
        let mut card = tool_card("todo", r#"{}"#);
        let output = "1. [completed][medium] cap the streaming buffer\n\
                      2. [in_progress][high] write tests for the cap\n\
                      3. [pending][medium] open a pull request";
        card.complete(&[protocol::ToolResultPart::text(output)], false);
        assert_eq!(card.summary, "3 items · 1 in progress");
        
        let mut lines = Vec::new();
        push_tool(&mut lines, &card, 92, false, &theme);
        let summary_text = plain(&lines[1]);
        assert!(
            summary_text.contains("3 items"),
            "summary shows item count:\n{summary_text}"
        );
        assert!(
            summary_text.contains("1 in progress"),
            "summary shows in-progress count:\n{summary_text}"
        );
    }

    #[test]
    fn todo_card_body_shows_three_state_markers() {
        // VAL-TOOL-025: a todo card's body lists each item with one of three distinct status markers:
        // ✓ done in OKG green, ▶ in-progress in ORANGE, ○ pending in FAINT.
        let theme = style::ThemeColors::dark();
        let mut card = tool_card("todo", r#"{}"#);
        let output = "1. [completed][medium] cap the streaming buffer\n\
                      2. [in_progress][high] add a dropped-chunk counter\n\
                      3. [pending][medium] write the regression test";
        card.complete(&[protocol::ToolResultPart::text(output)], false);
        
        let mut lines = Vec::new();
        push_tool(&mut lines, &card, 92, true, &theme);
        let text = rendered(&lines);
        
        // Each state marker appears with its color
        assert!(text.contains(glyphs::MARK_DONE), "done marker ✓ present:\n{text}");
        assert!(
            text.contains(glyphs::DISCLOSURE_COLLAPSED),
            "in-progress marker ▶ present:\n{text}"
        );
        assert!(text.contains(glyphs::DOT_HOLLOW), "pending marker ○ present:\n{text}");
        
        // The done item marker is OKG green
        let done_marker = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .find(|s| s.content.contains(glyphs::MARK_DONE))
            .expect("done marker span");
        assert_eq!(done_marker.style.fg, Some(theme.success), "done marker is OKG");
        
        // The in-progress marker is ORANGE
        let progress_marker = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .find(|s| s.content.contains(glyphs::DISCLOSURE_COLLAPSED))
            .expect("in-progress marker span");
        assert_eq!(
            progress_marker.style.fg,
            Some(theme.primary),
            "in-progress marker is ORANGE"
        );
        
        // The pending marker is FAINT (ghost)
        let pending_marker = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .find(|s| s.content.contains(glyphs::DOT_HOLLOW))
            .expect("pending marker span");
        assert_eq!(pending_marker.style.fg, Some(theme.ghost), "pending marker is FAINT");
    }

    #[test]
    fn websearch_card_summary_is_query() {
        // VAL-TOOL-026: a websearch card's summary is the search query.
        let card = tool_card("websearch", r#"{"query":"ratatui table widget"}"#);
        assert_eq!(card.summary, "ratatui table widget");
    }

    #[test]
    fn websearch_card_expanded_rows_are_results_with_overflow() {
        // VAL-TOOL-026: a websearch card's expanded output lists result rows followed by a
        // "... (N more results)" overflow row (not "lines").
        let theme = style::ThemeColors::dark();
        let mut card = tool_card("websearch", r#"{"query":"ratatui table widget"}"#);
        let body: String = (0..MAX_TOOL_DETAIL_LINES + 4)
            .map(|i| format!("Result {i}: ratatui docs page"))
            .collect::<Vec<_>>()
            .join("\n");
        card.complete(&[protocol::ToolResultPart::text(&body)], false);
        
        let mut lines = Vec::new();
        push_tool(&mut lines, &card, 92, true, &theme);
        let text = rendered(&lines);
        assert!(text.contains("ratatui table widget"), "summary query:\n{text}");
        assert!(text.contains("Result 0:"), "result rows present:\n{text}");
        assert!(text.contains("(4 more results)"), "overflow reads results:\n{text}");
        assert!(
            !text.contains("more lines"),
            "websearch overflow noun is results, not lines:\n{text}"
        );
    }

    #[test]
    fn webfetch_card_summary_is_url() {
        // VAL-TOOL-027: a webfetch card's summary row is the fetched URL.
        let card = tool_card("webfetch", r#"{"url":"https://docs.rs/ratatui/latest"}"#);
        assert_eq!(card.summary, "https://docs.rs/ratatui/latest");
    }

    #[test]
    fn working_band_line_carries_soft_bg_and_breathing_spinner() {
        // VAL-CONV-029/030/061: breathing spinner (ORANGE) + Working (BRIGHT) + detail + right-aligned
        // `<N>s · esc to interrupt`, with every cell painted on the soft band surface.
        let theme = style::ThemeColors::dark();
        let mut a = app();
        let (_tx, rx) = channel();
        a.begin_turn("q".to_owned(), rx, flag());
        let line = working_band_line(&a, 92, &theme);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("Working"), "label: {text:?}");
        assert!(text.contains("esc to interrupt"), "interrupt hint: {text:?}");

        // Every cell carries the explicit soft background (no black-bar fall-through).
        for span in &line.spans {
            assert_eq!(
                span.style.bg,
                Some(theme.soft),
                "band span {:?} has the soft bg",
                span.content
            );
        }
        // The spinner is the only primary-colored span and its glyph is from the breathing set.
        let spinner = line
            .spans
            .iter()
            .find(|s| s.style.fg == Some(theme.primary))
            .expect("spinner span");
        let glyph = spinner.content.trim();
        assert!(
            crate::glyphs::SPINNER_BREATHE.contains(&glyph),
            "breathing spinner glyph, got {glyph:?}"
        );
        // `Working` is BRIGHT.
        let working = line
            .spans
            .iter()
            .find(|s| s.content.contains("Working"))
            .expect("Working span");
        assert_eq!(working.style.fg, Some(theme.text_bright));
    }

    #[test]
    fn working_band_renders_above_composer_with_a_gap_row() {
        // VAL-CONV-028/032: while busy, a Working band sits above the composer with exactly one
        // blank gap row between the band and the composer's top border.
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut a = app();
        let (_tx, rx) = channel();
        a.begin_turn("do something".to_owned(), rx, flag());
        assert!(a.is_busy());
        let theme = style::ThemeColors::from_name(a.theme());
        let backend = TestBackend::new(92, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &a)).unwrap();
        let buffer = terminal.backend().buffer().clone();

        let row_text = |y: u16| -> String {
            (0..92).map(|x| buffer[(x, y)].symbol()).collect::<Vec<_>>().concat()
        };
        let band_y = (0..30u16)
            .find(|&y| row_text(y).contains("Working"))
            .expect("a Working band row");
        // The band row is painted on the soft surface.
        assert_eq!(buffer[(2, band_y)].bg, theme.soft, "band bg is soft");
        // Exactly one blank gap row, then the composer's rounded top border.
        let gap = row_text(band_y + 1);
        assert!(!gap.contains("Working"), "gap row holds no band content: {gap:?}");
        let composer_top = row_text(band_y + 2);
        assert!(
            composer_top.contains('╭'),
            "composer top border one gap row below the band: {composer_top:?}"
        );
    }

    #[test]
    fn slash_menu_lists_builtins_when_just_slash_typed() {
        // VAL-TUI-028: the open menu lists recognizable built-ins.
        let mut a = app();
        a.input.set_value("/");
        let lines = slash_menu_lines(&a, 60).expect("menu open");
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
        let lines = slash_menu_lines(&a, 60).expect("menu open");
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
        let lines = slash_menu_lines(&a, 60).expect("menu open even with no matches");
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
        assert!(
            slash_menu_lines(&a, 60).is_none(),
            "no menu for a chat message"
        );
    }

    #[test]
    fn slash_palette_selected_row_is_full_width_selbg_bar() {
        // VAL-CONV-035/036: the command in ORANGE + FAINT description, and the selected row is a
        // full-width SELBG bar with a `›` ORANGE marker and a BRIGHT command label.
        let mut a = app();
        let theme = style::ThemeColors::from_name(a.theme());
        a.input.set_value("/");
        let width = 80usize;
        let rows = slash_menu_lines(&a, width).expect("slash palette open");
        assert!(rows.len() > 1, "multiple commands listed");

        let sel = &rows[0];
        assert_eq!(sel.spans[0].content, "› ");
        assert_eq!(sel.spans[0].style.fg, Some(theme.primary));
        assert_eq!(sel.spans[0].style.bg, Some(theme.surface[3]));
        assert_eq!(sel.spans[1].style.fg, Some(theme.text_bright));
        assert_eq!(sel.width(), width, "selection bar spans the full inner width");
        for span in &sel.spans {
            assert_eq!(
                span.style.bg,
                Some(theme.surface[3]),
                "selected-row span {:?} is painted on the SELBG bar",
                span.content
            );
        }

        // A non-selected row: ORANGE command, FAINT description, no row fill.
        let other = &rows[1];
        assert_eq!(other.spans[1].style.fg, Some(theme.primary));
        assert_eq!(other.spans[1].style.bg, None);
        assert_eq!(other.spans[3].style.fg, Some(theme.text_muted));
    }

    #[test]
    fn mention_rows_show_filename_dir_size_meta_and_selection_bar() {
        // VAL-PERM-034/035: each `@`-mention row shows a file name + a right-aligned uniform
        // `dir · size` meta (no bullet); the selected row is a full-width SELBG bar with a `›`
        // ORANGE marker and a BRIGHT file name.
        let mut a = app();
        let theme = style::ThemeColors::from_name(a.theme());
        a.on_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('@'),
            crossterm::event::KeyModifiers::NONE,
        ));
        let width = 80usize;
        let rows = mention_menu_lines(&a, width).expect("file mention popup open");
        assert!(!rows.is_empty(), "the @ popup lists real workspace files");

        let sel = &rows[0];
        assert_eq!(sel.spans[0].content, "› ");
        assert_eq!(sel.spans[0].style.fg, Some(theme.primary));
        assert_eq!(sel.spans[0].style.bg, Some(theme.surface[3]));
        assert_eq!(sel.spans[1].style.fg, Some(theme.text_bright), "selected file name is BRIGHT");
        assert_eq!(sel.spans[1].style.bg, Some(theme.surface[3]));
        assert!(
            !sel.spans[1].content.contains('/'),
            "the left column is the file name, not the full path: {:?}",
            sel.spans[1].content
        );
        assert_eq!(sel.width(), width, "selection bar spans the full inner width");
        // The meta column (FAINT) sits just before the one-column right margin and is `dir · size`.
        let meta = &sel.spans[sel.spans.len() - 2];
        assert_eq!(meta.style.fg, Some(theme.text_muted));
        assert!(
            meta.content.contains(crate::glyphs::SEPARATOR),
            "meta is the uniform `dir · size` form: {:?}",
            meta.content
        );
        // The `▪` bullet from the interim wave-0 design is gone.
        assert!(
            !rows.iter().any(|row| row.spans.iter().any(|s| s.content.contains('▪'))),
            "no `▪` bullet in the refined mention rows"
        );

        if rows.len() > 1 {
            let other = &rows[1];
            assert_eq!(other.spans[0].content, "  ", "idle rows have no `›` marker");
            assert_eq!(other.spans[1].style.fg, Some(theme.text));
            assert_eq!(other.spans[1].style.bg, None);
            let other_meta = &other.spans[other.spans.len() - 2];
            assert!(
                other_meta.content.contains(crate::glyphs::SEPARATOR),
                "every row carries the `dir · size` meta: {:?}",
                other_meta.content
            );
        }
    }

    #[test]
    fn mention_popup_shows_at_files_title_and_count_tag() {
        // VAL-PERM-032/033: the `@`-mention popup is a bordered box above the composer whose top
        // border carries the `@ files` title and a right-aligned `shown/total` match-count tag.
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut a = app();
        let theme = style::ThemeColors::from_name(a.theme());
        a.on_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('@'),
            crossterm::event::KeyModifiers::NONE,
        ));
        assert!(a.mention_open(), "the `@` popup is open");
        let matched = a.mention_matches().len();
        let total = a.mention_file_total();
        assert!(total > 0, "the popup is choosing from real workspace files");

        let backend = TestBackend::new(92, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &a)).unwrap();
        let buffer = terminal.backend().buffer();
        let screen: String = buffer
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect();

        assert!(screen.contains("@ files"), "`@ files` title-in-border:\n{screen}");
        let tag = format!("{matched}/{total}");
        assert!(screen.contains(&tag), "the `{tag}` count tag rides the top border:\n{screen}");
        assert!(
            buffer
                .content()
                .iter()
                .any(|c| c.symbol() == "╭" && c.fg == theme.border),
            "the popup has a rounded ORANGE_D border"
        );
    }

    #[test]
    fn composer_renders_the_active_mention_token_in_amber() {
        // VAL-PERM-036: while the popup is open the composer draws the in-progress `@<partial>`
        // token in AMBER, with the surrounding prose staying BRIGHT.
        let mut a = app();
        let theme = style::ThemeColors::from_name(a.theme());
        for ch in "look @Ca".chars() {
            a.on_key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char(ch),
                crossterm::event::KeyModifiers::NONE,
            ));
        }
        assert!(a.mention_open(), "the `@` popup is open while typing the token");
        assert_eq!(a.mention_query().as_deref(), Some("Ca"));

        let base = Style::default().fg(theme.text_bright);
        let lines = composer_input_lines(&a, base, &theme);
        let spans: Vec<_> = lines.iter().flat_map(|l| l.spans.iter()).collect();
        let token = spans
            .iter()
            .find(|s| s.content == "@Ca")
            .expect("the `@Ca` token is its own span");
        assert_eq!(token.style.fg, Some(theme.secondary), "the mention token is AMBER");
        let prose = spans
            .iter()
            .find(|s| s.content.contains("look"))
            .expect("the leading prose is present");
        assert_eq!(prose.style.fg, Some(theme.text_bright), "prose stays BRIGHT");
    }

    #[test]
    fn slash_palette_dropdown_renders_above_composer() {
        // VAL-CONV-033/034/036/061: a floating Commands dropdown with an ORANGE_D rounded border,
        // an AMBER title-in-border, and the SELBG selection bar, painted over the brand canvas.
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut a = app();
        let theme = style::ThemeColors::from_name(a.theme());
        a.input.set_value("/");
        let backend = TestBackend::new(92, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &a)).unwrap();
        let buffer = terminal.backend().buffer();

        let screen: String = buffer
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect();
        assert!(screen.contains("Commands"), "Commands title-in-border present");
        assert!(screen.contains("/help"), "real command rows listed");

        assert!(
            buffer.content().iter().any(|c| c.bg == theme.surface[3]),
            "the selected row paints the SELBG inverse bar"
        );
        assert!(
            buffer
                .content()
                .iter()
                .any(|c| c.symbol() == "╭" && c.fg == theme.border),
            "the dropdown has a rounded ORANGE_D border"
        );
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
        let theme = style::ThemeColors::from_name(a.theme());
        let palette_keys = palette_modal_spec(&a, &theme)
            .rows
            .iter()
            .map(|row| row.key.clone())
            .collect::<Vec<_>>();
        assert!(
            palette_keys.iter().any(|key| key == "/models"),
            "the palette lists the model-switching action:\n{palette_keys:?}"
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
    fn notice_line_has_no_info_blue_foreground() {
        // VAL-CONV-060 (brand purity): a system notice ("Model set to ...") is brand-safe chrome.
        // It must never render in the info-blue token; its foreground is the neutral-grey text_dim.
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut a = app();
        a.transcript
            .push(Entry::Notice("Model set to google/gemini-2.5-flash".to_owned()));
        let theme = style::ThemeColors::from_name(a.theme());
        let backend = TestBackend::new(92, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &a)).unwrap();
        let buffer = terminal.backend().buffer().clone();

        let notice_row = (0..30)
            .find(|&y| {
                (0..92)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<Vec<_>>()
                    .concat()
                    .contains("Model set to")
            })
            .expect("the notice row is on screen");

        let notice_cells: Vec<_> = (0..92).map(|x| buffer[(x, notice_row)].clone()).collect();
        assert!(
            notice_cells
                .iter()
                .all(|c| c.fg != theme.info),
            "notice chrome must carry no info-blue foreground (VAL-CONV-060)"
        );
        assert!(
            notice_cells
                .iter()
                .any(|c| c.symbol() != " " && c.fg == theme.text_dim),
            "notice chrome renders in the neutral-grey text_dim token"
        );
    }

    #[test]
    fn shell_entry_renders_command_and_output() {
        // VAL-PERM-025/026/027: a shell-mode command renders as `! <command>` above a bordered
        // `output` pane containing the real command output.
        let mut a = app();
        a.transcript.push(Entry::Shell(crate::shell::ShellOutcome {
            command: "echo cortex-shell-ok".to_owned(),
            output: "cortex-shell-ok\n[exit code: 0]".to_owned(),
            success: true,
        }));
        let text = rendered(&transcript_lines(&a, 80));
        assert!(
            text.contains("! echo cortex-shell-ok"),
            "command line shown with the `!` prefix:\n{text}"
        );
        assert!(
            text.contains("output"),
            "output pane title-in-border present:\n{text}"
        );
        assert!(
            text.contains("cortex-shell-ok"),
            "command output shown:\n{text}"
        );
        // The output pane is a rounded box.
        assert!(
            text.contains('╭') && text.contains('╰'),
            "output pane has rounded borders:\n{text}"
        );
    }

    #[test]
    fn shell_output_pane_colors_command_success_and_logs() {
        // VAL-PERM-025/027/028: the `!` prefix is ORANGE, the command is BRIGHT, ordinary log lines
        // are DIM, and a `test result: ok` success summary is the sole green line in the pane.
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut a = app();
        a.transcript.push(Entry::Shell(crate::shell::ShellOutcome {
            command: "cargo test".to_owned(),
            output: "Compiling cortex-core\ntest result: ok. 3 passed; 0 failed".to_owned(),
            success: true,
        }));
        let theme = style::ThemeColors::from_name(a.theme());
        let backend = TestBackend::new(92, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &a)).unwrap();
        let buffer = terminal.backend().buffer().clone();

        let fg_of = |needle: &str| cell_at(&buffer, needle).map(|c| c.fg);
        assert_eq!(fg_of("!"), Some(theme.primary), "`!` prefix is ORANGE");
        assert_eq!(
            fg_of("cargo test"),
            Some(theme.text_bright),
            "command text is BRIGHT"
        );
        assert_eq!(
            fg_of("Compiling cortex-core"),
            Some(theme.text_dim),
            "ordinary log line is DIM"
        );
        assert_eq!(
            fg_of("test result: ok"),
            Some(theme.success),
            "the success summary line is green"
        );
        // Green is success-only: no other non-success text in the pane is green.
        assert_ne!(
            fg_of("Compiling cortex-core"),
            Some(theme.success),
            "ordinary log text is not green"
        );
    }

    #[test]
    fn shell_failed_command_shows_error_not_success() {
        // VAL-PERM-052: a non-zero command surfaces a red error line and never a green success line.
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut a = app();
        a.transcript.push(Entry::Shell(crate::shell::ShellOutcome {
            command: "exit 3".to_owned(),
            output: "[exit code: 3]".to_owned(),
            success: false,
        }));
        let theme = style::ThemeColors::from_name(a.theme());
        let backend = TestBackend::new(92, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &a)).unwrap();
        let buffer = terminal.backend().buffer().clone();

        assert_eq!(
            cell_at(&buffer, "[exit code: 3]").map(|c| c.fg),
            Some(theme.error),
            "the failing exit line is red"
        );
        assert!(
            !buffer.content().iter().any(|c| c.fg == theme.success),
            "no green success line for a failed command"
        );
    }

    #[test]
    fn shell_mode_status_bar_reads_manual_shell_mode() {
        // VAL-PERM-023/024: entering `!` shell mode shows the `shell` breadcrumb and a status bar
        // center reading `manual · ! shell mode` (replacing the normal autonomy/permission text).
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut a = app();
        a.on_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('!'),
            crossterm::event::KeyModifiers::NONE,
        ));
        assert!(a.shell_mode(), "shell mode engaged");

        let backend = TestBackend::new(92, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &a)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let row0 = buf_row(&buffer, 0);
        let last = buf_row(&buffer, 29);
        assert!(row0.contains("shell"), "breadcrumb crumb is `shell`:\n{row0}");
        assert!(
            last.contains("manual") && last.contains("! shell mode"),
            "status bar reads the shell state:\n{last}"
        );
        assert!(
            !last.contains("allow all commands"),
            "normal permission text is replaced in shell mode:\n{last}"
        );
    }

    #[test]
    fn shell_mode_composer_shows_bang_prefix_placeholder_and_hints() {
        // VAL-PERM-029: the shell composer shows an orange `!` prefix, the shell placeholder, and
        // the `↵ to send · esc to exit` hints.
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut a = app();
        a.on_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('!'),
            crossterm::event::KeyModifiers::NONE,
        ));
        let backend = TestBackend::new(92, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &a)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let screen: String = buffer
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect();
        assert!(
            screen.contains("run another shell command"),
            "shell placeholder shown:\n{screen}"
        );
        assert!(
            screen.contains("to send") && screen.contains("esc to exit"),
            "shell send/exit hints shown:\n{screen}"
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
    fn staged_attachments_render_cards_and_breadcrumb() {
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

        let backend = TestBackend::new(92, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &a)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let rows: Vec<String> = (0..buffer.area().height)
            .map(|y| {
                (0..buffer.area().width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect();
        let screen = rows.join("\n");
        let name = png.file_name().unwrap().to_string_lossy().into_owned();
        // The card clips a long filename; assert on a visible prefix that survives the clip.
        let name_prefix: String = name.chars().take(12).collect();

        // VAL-PERM-041: the chrome breadcrumb reads `attachments`.
        assert!(
            rows[0].contains("Cortex Code") && rows[0].contains("attachments"),
            "attachments breadcrumb on row 0:\n{}",
            rows[0]
        );
        // VAL-PERM-042/043: a card with a `PNG` type badge + the real filename.
        assert!(
            screen.contains("PNG") && screen.contains(&name_prefix),
            "attachment card shows badge + filename:\n{screen}"
        );
        // VAL-PERM-042: rounded card chrome.
        assert!(
            screen.contains('╭') && screen.contains('╰'),
            "card is drawn with rounded borders:\n{screen}"
        );
        // VAL-PERM-045: the aggregate summary line.
        assert!(
            screen.contains("1 attachment ") && screen.contains(" · "),
            "aggregate summary present:\n{screen}"
        );
        // The emoji chip is gone (brand purity).
        assert!(!screen.contains('📎'), "no emoji chip:\n{screen}");
    }

    #[test]
    fn attachment_badge_is_an_inverse_amber_pill() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut a = app();
        let png = temp_png();
        a.set_input_value(&format!("/attach {}", png.display()));
        a.on_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        ));

        let theme = style::ThemeColors::from_name(a.theme());
        let backend = TestBackend::new(92, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &a)).unwrap();
        let buffer = terminal.backend().buffer().clone();

        // VAL-PERM-043: the badge cells carry AMBER text on the SELBG surface (a filled pill), not
        // plain text on the canvas.
        let mut found_pill = false;
        for y in 0..buffer.area().height {
            for x in 0..buffer.area().width.saturating_sub(2) {
                if buffer[(x, y)].symbol() == "P"
                    && buffer[(x + 1, y)].symbol() == "N"
                    && buffer[(x + 2, y)].symbol() == "G"
                {
                    let cell = &buffer[(x, y)];
                    if cell.fg == theme.secondary && cell.bg == theme.surface[3] {
                        found_pill = true;
                    }
                }
            }
        }
        assert!(found_pill, "PNG badge is an inverse amber pill (amber fg on SELBG)");
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

    /// Render an open dialog through the modal system into a text snapshot (via `draw`).
    fn dialog_screen(a: &App) -> String {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
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

    #[test]
    fn modal_lists_items_and_highlights_the_selection() {
        // VAL-TUI-035 / VAL-PICK-004: the model picker lists models (incl. the default) with the
        // active row carrying the inverse `›` selection marker.
        let mut a = app();
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
        let text = dialog_screen(&a);
        assert!(
            text.contains("google/gemini-2.5-flash"),
            "default model listed:\n{text}"
        );
        assert!(text.contains("›"), "selection marker on the highlight:\n{text}");
        assert!(!text.contains("▸"), "no legacy thin triangle:\n{text}");
        assert!(text.contains("Models"), "title-in-border drawn:\n{text}");
    }

    #[test]
    fn modal_renders_an_empty_state() {
        // VAL-TUI-046 / VAL-PICK-065: the MCP view opens to a clear empty state (no highlight) when
        // no servers are configured.
        let spec = crate::modal::ModalSpec {
            title: "MCP servers".to_owned(),
            rows: Vec::new(),
            selected: 0,
            hints: vec![crate::modal::ModalHint::new("esc", "close")],
            note: None,
            search: None,
            input: None,
            empty: Some("No MCP servers configured.".to_owned()),
            width: 66,
            key_width: None,
            key_color: style::ThemeColors::dark().text,
        };
        let lines = crate::modal::build_interior_lines(
            &spec,
            60,
            3,
            None,
            &style::ThemeColors::dark(),
        );
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.to_string()))
            .collect();
        assert!(
            text.contains("No MCP servers configured."),
            "empty state shown:\n{text}"
        );
        assert!(!text.contains("›"), "no highlight marker when empty:\n{text}");
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

    /// Open the command palette and return its interior lines (search, rule, rows, footer) rendered
    /// through the shared rich-modal renderer.
    fn palette_interior_lines(a: &App) -> Vec<Line<'static>> {
        let theme = style::ThemeColors::from_name(a.theme());
        let spec = palette_modal_spec(a, &theme);
        crate::modal::build_interior_lines(&spec, 58, 14, None, &theme)
    }

    #[test]
    fn palette_lists_builtin_commands_with_descriptions() {
        // VAL-PICK-060: the palette lists real builtin slash commands with their descriptions.
        let mut a = app();
        a.on_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('p'),
            crossterm::event::KeyModifiers::CONTROL,
        ));
        let theme = style::ThemeColors::from_name(a.theme());
        let rows = palette_modal_spec(&a, &theme).rows;
        let pair = |name: &str| -> (String, String) {
            let row = rows
                .iter()
                .find(|row| row.key == name)
                .unwrap_or_else(|| panic!("palette lists {name}: {rows:?}"));
            (row.key.clone(), row.desc.clone().unwrap_or_default())
        };
        for (name, desc) in [
            ("/new", "Start a fresh session"),
            ("/sessions", "Open the session picker"),
            ("/models", "Open the model picker"),
            ("/agents", "Open the agent picker"),
            ("/themes", "Open the theme picker"),
            ("/status", "Open the status view"),
            ("/diff", "Open the diff viewer"),
            ("/compact", "Compact the conversation"),
            ("/export", "Export the current session"),
            ("/init", "Initialize the project (AGENTS.md)"),
        ] {
            assert_eq!(pair(name), (name.to_owned(), desc.to_owned()));
        }
    }

    #[test]
    fn palette_leads_with_the_reference_primary_commands() {
        // VAL-PICK-060: the palette opens leading with the everyday commands in the reference order
        // (pick_palette.png), with no alias duplicates (`/model`, `/theme`).
        let mut a = app();
        a.on_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('p'),
            crossterm::event::KeyModifiers::CONTROL,
        ));
        let theme = style::ThemeColors::from_name(a.theme());
        let keys = palette_modal_spec(&a, &theme)
            .rows
            .iter()
            .map(|row| row.key.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            keys[..10],
            [
                "/new",
                "/sessions",
                "/models",
                "/agents",
                "/themes",
                "/status",
                "/diff",
                "/compact",
                "/export",
                "/init",
            ]
        );
        assert_eq!(keys.iter().filter(|k| *k == "/models").count(), 1);
        assert!(!keys.iter().any(|k| k == "/model"), "no alias duplicate");
        assert!(!keys.iter().any(|k| k == "/theme"), "no alias duplicate");
    }

    #[test]
    fn palette_search_row_has_orange_prompt_query_and_caret() {
        // VAL-PICK-059: the search row shows a `›` prompt in orange, the typed query, and a `▏`
        // caret, followed by a faint underline rule.
        let mut a = app();
        a.on_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('p'),
            crossterm::event::KeyModifiers::CONTROL,
        ));
        for c in "dif".chars() {
            a.on_key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char(c),
                crossterm::event::KeyModifiers::NONE,
            ));
        }
        let theme = style::ThemeColors::from_name(a.theme());
        let lines = palette_interior_lines(&a);
        let search = &lines[0];
        assert_eq!(search.spans[0].content, crate::glyphs::PROMPT);
        assert_eq!(search.spans[0].style.fg, Some(theme.primary));
        let text = rendered(&[search.clone()]);
        assert!(text.contains("dif"), "typed query shown: {text}");
        assert!(text.contains(crate::glyphs::CURSOR), "caret shown: {text}");
        // The underline rule sits directly under the search row.
        let rule = rendered(&[lines[1].clone()]);
        assert!(
            rule.chars().all(|c| c == '─' || c == ' '),
            "faint underline rule under the search: {rule}"
        );
    }

    #[test]
    fn palette_fuzzy_filters_and_clearing_restores_the_list() {
        // VAL-PICK-061: typing narrows to fuzzy-matching commands; clearing restores the full list.
        let mut a = app();
        a.on_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('p'),
            crossterm::event::KeyModifiers::CONTROL,
        ));
        let theme = style::ThemeColors::from_name(a.theme());
        let full = palette_modal_spec(&a, &theme).rows.len();
        for c in "diff".chars() {
            a.on_key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char(c),
                crossterm::event::KeyModifiers::NONE,
            ));
        }
        let keys = palette_modal_spec(&a, &theme)
            .rows
            .iter()
            .map(|row| row.key.clone())
            .collect::<Vec<_>>();
        assert!(keys.iter().any(|k| k == "/diff"), "match kept: {keys:?}");
        assert!(!keys.iter().any(|k| k == "/new"), "non-match hidden: {keys:?}");
        assert!(keys.len() < full, "the list narrowed");
        // Clearing the query restores the full list.
        for _ in 0..4 {
            a.on_key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Backspace,
                crossterm::event::KeyModifiers::NONE,
            ));
        }
        assert_eq!(palette_modal_spec(&a, &theme).rows.len(), full);
    }

    #[test]
    fn palette_selected_row_is_the_inverse_bar_with_marker() {
        // VAL-PICK-062: exactly one row carries the inverse selection bar (SELBG + `›` marker).
        let mut a = app();
        a.on_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('p'),
            crossterm::event::KeyModifiers::CONTROL,
        ));
        let theme = style::ThemeColors::from_name(a.theme());
        let lines = palette_interior_lines(&a);
        // rows[0] search, rows[1] rule; the first command row (rows[2]) is the highlighted row.
        let sel = &lines[2];
        assert_eq!(sel.spans[0].content, crate::glyphs::SELECT_MARKER);
        assert_eq!(sel.spans[0].style.fg, Some(theme.primary));
        assert!(
            sel.spans.iter().all(|sp| sp.style.bg == Some(theme.surface[3])),
            "the selected row is a full-width inverse bar"
        );
        // No other command row carries the selection fill.
        let bars = lines
            .iter()
            .skip(2)
            .filter(|line| {
                !line.spans.is_empty()
                    && line.spans.iter().all(|sp| sp.style.bg == Some(theme.surface[3]))
            })
            .count();
        assert_eq!(bars, 1, "exactly one highlighted row");
    }

    #[test]
    fn palette_footer_advertises_move_run_and_close() {
        // VAL-PICK-062: the footer advertises `↑↓ move`, `↵ run`, and `esc close`.
        let mut a = app();
        a.on_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('p'),
            crossterm::event::KeyModifiers::CONTROL,
        ));
        let theme = style::ThemeColors::from_name(a.theme());
        let hints = palette_modal_spec(&a, &theme).hints;
        let footer = hints
            .iter()
            .map(|h| format!("{} {}", h.key, h.label))
            .collect::<Vec<_>>()
            .join("  ");
        assert!(footer.contains("↑↓ move"), "move hint: {footer}");
        assert!(footer.contains("↵ run"), "run hint: {footer}");
        assert!(footer.contains("esc close"), "close hint: {footer}");
    }

    #[test]
    fn opened_palette_renders_over_the_frame() {
        // VAL-PICK-058: the palette, once opened, renders as a `Commands` modal listing commands.
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
            rendered.contains("Commands"),
            "the palette title is drawn:\n{rendered}"
        );
        assert!(
            rendered.contains("/new"),
            "command entries are drawn in the overlay"
        );
        assert!(!rendered.contains("▸"), "no legacy thin triangle");
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
        // VAL-MD-021: the matched text carries the HIT highlight background.
        let mut a = app();
        a.transcript
            .push(Entry::Notice("find ZTOKEN now".to_owned()));
        let mut lines = transcript_lines(&a, 80);
        highlight_find(&mut lines, "ztoken", 0, style::HIT, style::HITCUR, style::BRIGHT);
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
        // VAL-MD-022: exactly one match (the active one) carries the HITCUR bg + BRIGHT fg; the
        // rest carry the plain HIT bg.
        let mut a = app();
        a.transcript.push(Entry::Notice("ZTOKEN one".to_owned()));
        a.transcript.push(Entry::Notice("ZTOKEN two".to_owned()));
        let mut lines = transcript_lines(&a, 80);
        highlight_find(&mut lines, "ztoken", 1, style::HIT, style::HITCUR, style::BRIGHT);
        let mut active = Vec::new();
        let mut base = Vec::new();
        for s in lines.iter().flat_map(|l| &l.spans) {
            if s.content.eq_ignore_ascii_case("ztoken") {
                if s.style.add_modifier.contains(Modifier::BOLD) {
                    active.push((s.style.bg, s.style.fg));
                } else {
                    base.push(s.style.bg);
                }
            }
        }
        assert_eq!(
            active,
            vec![(Some(style::HITCUR), Some(style::BRIGHT))],
            "exactly one active match with the HITCUR bg + BRIGHT fg"
        );
        assert_eq!(
            base,
            vec![Some(style::HIT)],
            "the other match uses the plain HIT background"
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
        // VAL-MD-020/023/024: the find bar shows `find:` label, the query, a cur/total counter,
        // and navigation hints.
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
            rendered.contains("find:"),
            "the find bar shows the find: label:\n{rendered}"
        );
        assert!(rendered.contains("apple"), "the query text is shown");
        assert!(
            rendered.contains("1/2") || rendered.contains("2/2"),
            "the cur/total counter is shown:\n{rendered}"
        );
        assert!(
            rendered.contains("matches"),
            "the matches label is shown:\n{rendered}"
        );
        assert!(
            rendered.contains("n next") && rendered.contains("N prev") && rendered.contains("esc close"),
            "the navigation hints are shown:\n{rendered}"
        );
    }

    #[test]
    fn find_mode_shows_chat_find_crumb() {
        // VAL-MD-019: opening find renders the `chat · find` breadcrumb.
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut a = app();
        a.transcript.push(Entry::Notice("search me".to_owned()));
        press_ctrl(&mut a, 'f');

        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &a)).unwrap();
        let rendered = screen(&terminal);
        let row0 = rendered.lines().next().unwrap_or("");
        assert!(
            row0.contains('✦') && row0.contains("Cortex Code") && row0.contains("find"),
            "row 0 shows the chat·find breadcrumb: {row0}"
        );
    }

    #[test]
    fn find_zero_state_shows_zero_counter() {
        // VAL-MD-037: a query with no matches shows a `0/0` counter.
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut a = app();
        a.transcript.push(Entry::Notice("apple pie".to_owned()));
        press_ctrl(&mut a, 'f');
        for c in "ztoken".chars() {
            press(&mut a, crossterm::event::KeyCode::Char(c));
        }

        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &a)).unwrap();
        let rendered = screen(&terminal);
        assert!(
            rendered.contains("0/0"),
            "the zero-state counter is shown:\n{rendered}"
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
    fn plan_mode_status_bar_center_reads_read_only_plan() {
        // VAL-PERM-017: in plan mode the status-bar center permission word is `read-only · plan`
        // (DIM), replacing the normal `allow all commands`, while `auto (high)` is unchanged.
        let mut a = app();
        a.status.agent = "plan".to_owned();
        let theme = style::ThemeColors::from_name(a.theme());
        let line = status_bar_line(&a, 92);
        let text = plain(&line);
        assert!(
            text.contains("auto (high) · read-only · plan"),
            "center shows the plan permission state:\n{text}"
        );
        assert!(
            !text.contains("allow all commands"),
            "the normal permission word is gone in plan mode:\n{text}"
        );
        let fg_of = |needle: &str| {
            line.spans
                .iter()
                .find(|s| s.content.as_ref() == needle)
                .and_then(|s| s.style.fg)
        };
        assert_eq!(
            fg_of("read-only · plan"),
            Some(theme.text_dim),
            "the plan permission word is DIM"
        );
    }

    #[test]
    fn plan_banner_line_is_amber_edge_plus_faint_note() {
        // VAL-PERM-016: the footer banner is an amber `▌` edge + `plan mode` (AMBER) followed by a
        // faint read-only note; amber keeps it distinct from the red blocked banner (VAL-PERM-021).
        let theme = style::ThemeColors::dark();
        let line = plan_banner_line(92, &theme);
        let text = plain(&line);
        assert!(text.contains("▌"), "the accent edge glyph is present:\n{text}");
        assert!(text.contains("plan mode"), "the plan-mode label is present:\n{text}");
        assert!(
            text.contains("read-only — edits & shell paused."),
            "the read-only note is present:\n{text}"
        );
        assert!(
            text.contains("⇥ switch to build to apply."),
            "the switch-to-build hint is present:\n{text}"
        );
        let amber = line
            .spans
            .iter()
            .find(|s| s.content.contains("plan mode"))
            .expect("plan mode span");
        assert_eq!(amber.style.fg, Some(theme.secondary), "`plan mode` is AMBER");
        let note = line
            .spans
            .iter()
            .find(|s| s.content.contains("read-only"))
            .expect("note span");
        assert_eq!(note.style.fg, Some(theme.text_muted), "the note is FAINT grey");
        // The amber banner hue must differ from the red blocked hue.
        assert_ne!(theme.secondary, theme.error, "amber banner != red blocked");
    }

    #[test]
    fn blocked_banner_is_a_red_marker_word_with_dim_reason_in_a_box() {
        // VAL-PERM-019/021: the blocked banner is a bordered box whose row shows a red `▌` + red
        // `blocked`, the command in BRIGHT, and a DIM reason.
        let theme = style::ThemeColors::dark();
        let mut lines: Vec<Line<'static>> = Vec::new();
        push_blocked(
            &mut lines,
            "cargo build --release",
            crate::app::SHELL_BLOCKED_WHILE_PLANNING,
            92,
            &theme,
        );
        let joined: String = lines.iter().map(plain).collect::<Vec<_>>().join("\n");
        assert!(joined.contains('╭') && joined.contains('╰'), "rounded box:\n{joined}");
        assert!(joined.contains("blocked"), "the blocked word:\n{joined}");
        assert!(
            joined.contains("cargo build --release"),
            "the command is named:\n{joined}"
        );
        assert!(
            joined.contains("shell is disabled while planning"),
            "the dim reason is shown:\n{joined}"
        );
        // The content row carries the red marker/word, bright command, and dim reason as distinct
        // style runs.
        let content = lines
            .iter()
            .find(|l| plain(l).contains("blocked"))
            .expect("content row");
        let fg_of = |needle: &str| {
            content
                .spans
                .iter()
                .find(|s| s.content.contains(needle))
                .and_then(|s| s.style.fg)
        };
        assert_eq!(fg_of("blocked"), Some(theme.error), "`blocked` is DENY red");
        assert_eq!(
            fg_of("cargo build --release"),
            Some(theme.text_bright),
            "the command is BRIGHT"
        );
        assert_eq!(
            fg_of("shell is disabled while planning"),
            Some(theme.text_dim),
            "the reason is DIM"
        );
    }

    #[test]
    fn plan_mode_draws_chrome_banner_and_placeholder() {
        // VAL-PERM-015/016/017: entering plan mode wraps the conversation in the shared chrome under
        // a `plan mode` breadcrumb, shows the amber footer banner above the composer, the plan
        // composer placeholder, and the `read-only · plan` status bar; a normal build screen shows
        // none of these.
        let mut a = app();
        a.status.agent = "plan".to_owned();
        a.transcript.push(Entry::User("add the buffer cap and build it".to_owned()));
        let buf = draw_92x30(&a);

        let row0 = buf_row(&buf, 0);
        assert!(row0.contains('✦'), "breadcrumb star on row 0:\n{row0}");
        assert!(row0.contains("Cortex Code"), "breadcrumb wordmark:\n{row0}");
        assert!(row0.contains("plan mode"), "breadcrumb crumb is `plan mode`:\n{row0}");

        let screen = buf_text(&buf);
        assert!(
            screen.contains("read-only — edits & shell paused."),
            "the amber footer banner is present:\n{screen}"
        );
        assert!(
            screen.contains("read-only · plan"),
            "the status bar reads the plan permission state:\n{screen}"
        );

        // A normal (build) conversation shows neither the plan crumb nor the plan banner.
        let mut b = app();
        b.transcript.push(Entry::User("hello".to_owned()));
        let build_screen = buf_text(&draw_92x30(&b));
        assert!(!build_screen.contains("plan mode"), "no plan crumb on the build screen");
        assert!(
            !build_screen.contains("edits & shell paused"),
            "no plan banner on the build screen"
        );
    }

    #[test]
    fn plan_mode_composer_shows_the_plan_placeholder() {
        // VAL-PERM-015: the plan composer placeholder invites a question/plan refinement (distinct
        // from the home/reply placeholders).
        let mut a = app();
        a.status.agent = "plan".to_owned();
        let screen = buf_text(&draw_92x30(&a));
        assert!(
            screen.contains("ask a question or refine the plan"),
            "the plan composer placeholder is shown:\n{screen}"
        );
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

    /// Open a bash permission prompt over a live turn and return the rendered 92x30 buffer.
    fn permission_buffer(subject: &str) -> ratatui::buffer::Buffer {
        use crate::app::Entry;
        use crate::permission::PermissionAsk;
        let mut a = app();
        a.transcript.push(Entry::User("clean the build".to_owned()));
        let (reply, wait) = channel();
        a.open_permission_for_test(PermissionAsk::new("bash", subject, reply));
        // Keep the receiver alive so the ask stays open for the duration of the render.
        std::mem::forget(wait);
        render_92x30(&a)
    }

    fn cell_at<'b>(buf: &'b ratatui::buffer::Buffer, needle: &str) -> Option<&'b ratatui::buffer::Cell> {
        let w = buf.area.width as usize;
        let h = buf.area.height as usize;
        for y in 0..h {
            let row: String = (0..w).map(|x| buf.content()[y * w + x].symbol()).collect();
            if let Some(byte_idx) = row.find(needle) {
                let col = row[..byte_idx].chars().count();
                return Some(&buf.content()[y * w + col]);
            }
        }
        None
    }

    /// The 92x30 buffer rendered as text, one row per line (so a needle that sits on one row is
    /// found without being split across a wrap boundary).
    fn buffer_rows(buf: &ratatui::buffer::Buffer) -> String {
        let w = buf.area.width as usize;
        let h = buf.area.height as usize;
        (0..h)
            .map(|y| (0..w).map(|x| buf.content()[y * w + x].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn permission_dialog_shows_command_box_options_and_hints() {
        // VAL-PERM-001/003/004/006/009: the modal titled `Permission required` with a `shell` tag,
        // a `$`-prompted command box holding the real command, the three ordered options with
        // right-hand descriptions, and the `choose/move/deny` key hints.
        let theme = style::ThemeColors::dark();
        let buf = permission_buffer("rm -rf target/ && cargo build --release");
        let text = buffer_rows(&buf);

        assert!(text.contains("Permission required"), "modal title:\n{text}");
        assert!(text.contains("shell"), "right-edge shell tag:\n{text}");
        assert!(
            text.contains("the agent wants to run a command:"),
            "command prompt line:\n{text}"
        );
        assert!(
            text.contains("rm -rf target/ && cargo build --release"),
            "the real requested command is shown in the box:\n{text}"
        );
        // The `$` prompt is ORANGE.
        let dollar = cell_at(&buf, "$").expect("`$` prompt present");
        assert_eq!(dollar.fg, theme.primary, "`$` prompt is ORANGE");

        for (opt, desc) in [
            ("Allow once", "run just this command"),
            ("Allow for this session", "don't ask again this session"),
            ("Deny", "reject and tell the agent why"),
        ] {
            assert!(text.contains(opt), "option `{opt}` present:\n{text}");
            assert!(text.contains(desc), "description `{desc}` present:\n{text}");
        }
        // Option order: Allow once, then Allow for this session, then Deny.
        let once = text.find("Allow once").expect("allow once");
        let session = text.find("Allow for this session").expect("allow session");
        let deny = text.find("Deny").expect("deny");
        assert!(once < session && session < deny, "options in order:\n{text}");

        // Footer key hints.
        assert!(text.contains("choose"), "choose hint:\n{text}");
        assert!(text.contains("move"), "move hint:\n{text}");
        assert!(text.contains("deny"), "deny hint:\n{text}");
    }

    #[test]
    fn permission_dialog_deny_is_red_first_option_selected() {
        // VAL-PERM-007/008: the first option (`Allow once`) is the selected inverse SELBG bar with a
        // `›` ORANGE marker, and `Deny` is rendered in the ERR red while the allow options are not.
        let theme = style::ThemeColors::dark();
        let buf = permission_buffer("rm -rf target/");

        // `Deny` label is red.
        let deny = cell_at(&buf, "Deny").expect("Deny label present");
        assert_eq!(deny.fg, theme.error, "Deny is red");
        // The allow options are NOT red.
        let allow = cell_at(&buf, "Allow once").expect("Allow once present");
        assert_ne!(allow.fg, theme.error, "Allow once is not red");

        // The selected first option carries the SELBG inverse bar and a `›` marker in ORANGE.
        let allow_bar = cell_at(&buf, "Allow once").expect("Allow once cell");
        assert_eq!(allow_bar.bg, theme.surface[3], "selected row is the SELBG inverse bar");
        // The `›` marker sits on the selected row (the transcript `›` is dimmed by the backdrop, so
        // find the one on the SELBG bar).
        let w = buf.area.width as usize;
        let h = buf.area.height as usize;
        let marker = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .map(|(x, y)| &buf.content()[y * w + x])
            .find(|c| c.symbol() == "›" && c.bg == theme.surface[3]);
        assert!(marker.is_some(), "the selected row carries a `›` marker on the SELBG bar");
        assert_eq!(marker.unwrap().fg, theme.primary, "`›` marker is ORANGE");
    }

    #[test]
    fn permission_dialog_shows_real_cwd_and_dims_backdrop() {
        // VAL-PERM-002/005: the working-directory line reflects the real cwd, and the prior
        // conversation behind the modal is dimmed (not full-brightness).
        let buf = permission_buffer("rm -rf target/");
        let text = buffer_rows(&buf);
        let cwd = crate::motd::current_dir_display();
        assert!(
            text.contains(&format!("in {cwd}")),
            "real cwd context line present (`in {cwd}`):\n{text}"
        );

        // The backdrop user line `clean the build` is faded: its fg is blended toward the page bg,
        // so it is no longer the crisp TXT token.
        let theme = style::ThemeColors::dark();
        let faded = cell_at(&buf, "clean the build");
        if let Some(cell) = faded {
            assert_ne!(cell.fg, theme.text, "backdrop text is dimmed, not full-brightness");
        }
    }
}
