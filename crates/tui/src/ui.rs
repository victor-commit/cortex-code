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

use crate::app::{App, AssistantTurn, Entry, UsageSummary};
use crate::diff::FileDiff;
use crate::tool_card::{ToolCard, ToolStatus};

/// Maximum number of slash-command rows shown before the menu scrolls.
const MAX_SLASH_ROWS: usize = 10;

/// Maximum number of visible prompt text rows before the box stops growing.
const MAX_PROMPT_ROWS: usize = 8;

/// Maximum number of output lines shown inside an expanded tool card before truncation.
const MAX_TOOL_DETAIL_LINES: usize = 40;

/// Maximum drawn width of a tool card's top/bottom border.
const TOOL_CARD_WIDTH: usize = 76;

/// Renders the whole frame for the current [`App`] state.
pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();

    let prompt_rows = prompt_rows(app, area.height);
    let chunks = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(prompt_rows + 2),
        Constraint::Length(1),
    ])
    .split(area);

    draw_transcript(frame, app, chunks[0]);
    draw_prompt(frame, app, chunks[1]);
    draw_footer(frame, app, chunks[2]);
    // The slash menu floats above the prompt, so it is drawn last to overlay the transcript.
    draw_slash_menu(frame, app, chunks[1]);
}

/// Number of text rows the prompt box should show (clamped to available height).
fn prompt_rows(app: &App, total_height: u16) -> u16 {
    let logical = app.input.line_count().max(1);
    // Leave room for at least one transcript row, the prompt borders, and the footer.
    let budget = total_height.saturating_sub(1 + 2 + 1).max(1) as usize;
    logical.min(MAX_PROMPT_ROWS).min(budget).max(1) as u16
}

/// Render the transcript area: the home screen before any exchange, otherwise the streamed
/// conversation tail (so the newest content and the busy indicator stay visible).
fn draw_transcript(frame: &mut Frame, app: &App, area: Rect) {
    if app.transcript.is_empty() && !app.is_busy() {
        draw_home(frame, area);
        return;
    }

    let width = area.width.max(1) as usize;
    let lines = transcript_lines(app, width);
    // Keep the latest lines visible (auto-scroll to the bottom) by dropping the leading overflow.
    let height = area.height as usize;
    let start = lines.len().saturating_sub(height);
    let visible: Vec<Line> = lines.into_iter().skip(start).collect();

    frame.render_widget(Paragraph::new(visible), area);
}

/// Build the full transcript as pre-wrapped lines (so scrolling is exact). The in-progress
/// assistant entry grows as deltas arrive, and the busy indicator is appended while a turn streams.
pub fn transcript_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    for entry in &app.transcript {
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
            Entry::Help => push_help(&mut lines),
            Entry::Error(message) => push_error(&mut lines, message, width),
        }
    }
    if let Some(glyph) = app.spinner_glyph() {
        lines.push(Line::from(Span::styled(
            format!("{glyph} Thinking…"),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )));
    }
    lines
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

/// The keybindings shown in the `/help` reference, paired with what each does.
const KEYBINDINGS: &[(&str, &str)] = &[
    ("Enter", "Send the prompt / run the highlighted command"),
    ("Shift+Enter", "Insert a newline without sending"),
    ("/", "Open the slash-command menu"),
    ("Up / Down", "Move the slash-menu highlight"),
    ("Esc", "Close the slash-command menu"),
    ("Ctrl+A / Ctrl+E", "Jump to line start / end"),
    ("Ctrl+O", "Expand / collapse tool cards"),
    ("Ctrl+C", "Quit Cortex Code"),
];

/// Append the `/help` command reference: a titled panel listing every built-in slash command (from
/// the registry) with its description, then the keybindings. This is the visible effect of `/help`.
fn push_help(lines: &mut Vec<Line<'static>>) {
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

fn draw_home(frame: &mut Frame, area: Rect) {
    let heading_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let dim = Style::default().add_modifier(Modifier::DIM);

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
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(" Prompt ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let value = app.input_value();
    if value.is_empty() {
        let placeholder = Paragraph::new(Span::styled(
            "Type a message…  (/ for commands)",
            Style::default().add_modifier(Modifier::DIM),
        ));
        frame.render_widget(placeholder, inner);
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

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let footer = Paragraph::new(footer_line(app)).style(Style::default().bg(Color::Indexed(236)));
    frame.render_widget(footer, area);
}

/// Build the status footer: model / agent / mode / session plus the token-and-cost usage segment.
/// Before any turn reports usage the segment reads `usage —`; after a turn it shows the running
/// token counts (and cost when reported), so a usage figure appears/increments versus before.
pub fn footer_line(app: &App) -> Line<'static> {
    let label = Style::default()
        .fg(Color::DarkGray)
        .add_modifier(Modifier::BOLD);
    let value = Style::default().fg(Color::Gray);
    let sep = || Span::styled(" · ", Style::default().fg(Color::DarkGray));

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
    use crate::app::{App, AssistantTurn, Status};
    use std::sync::mpsc::channel;

    fn app() -> App {
        App::new(Status {
            model: "google/gemini-2.5-flash".to_owned(),
            agent: "build".to_owned(),
            mode: "build".to_owned(),
            session: "new".to_owned(),
        })
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
        a.begin_turn("hello there".to_owned(), rx);
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
        a.begin_turn("q".to_owned(), rx);
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
    fn footer_usage_appears_after_a_turn() {
        let mut a = app();
        let before = plain(&footer_line(&a));
        assert!(
            before.contains("usage —"),
            "no figure before a turn: {before}"
        );

        let (tx, rx) = channel();
        a.begin_turn("q".to_owned(), rx);
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
}
