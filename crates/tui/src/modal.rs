//! The rich modal/picker renderer shared by every Wave-1 picker.
//!
//! This is the single system that replaces the old generic dialog renderer: a centered modal over
//! a dimmed backdrop, a rounded `ORANGE_D` border with an AMBER title embedded in its top edge, an
//! optional search row (`›` prompt + `▏` caret), an inverse selection bar (`surface[3]` fill + a
//! leading `›` marker + brightened key text), a right-aligned meta column with a ≥2-col gap before
//! it, semantic meta colors, and a footer key-hint row inside the bottom border. It scrolls a long
//! list to keep the selection visible and shows an explicit no-results state when the list is empty.
//!
//! When animations are enabled a selection move plays a brief highlight transition: the inverse
//! bar's fill cross-fades between the old and new row (and the `›` marker crosses at the midpoint)
//! across at least one intermediate frame. Under reduced motion the caller passes `None` and the
//! selection jumps instantly. Phase is derived from `Instant::elapsed` by the caller (see
//! [`crate::app::App::selection_anim_state`]); this module is a pure renderer of the resolved phase.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

use tui_components::style::ThemeColors;

use crate::glyphs;
use crate::ui::{cells_to_line, centered_rect, place_segments};

/// Smallest modal width (columns) so a picker always reads as a floating card, not a thin box.
const MIN_MODAL_WIDTH: u16 = 40;

/// Largest number of list rows shown before the modal body scrolls.
const MAX_MODAL_BODY_ROWS: usize = 14;

/// A resolved selection-highlight transition for the current frame.
///
/// The bar interpolates from row `from` to row `to` at `phase` in `[0, 1]`: the old row's fill
/// fades out (weight `1 - phase`) while the new row's fades in (weight `phase`), and the `›` marker
/// crosses at `phase == 0.5`. The caller only produces this when animations are enabled and the
/// move was a single adjacent step; otherwise it passes `None` and the selection is instant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SelectionAnim {
    /// The row the highlight is leaving.
    pub from: usize,
    /// The row the highlight is moving to (equals the modal's current selection).
    pub to: usize,
    /// Transition progress in `[0, 1]` (0 = at `from`, 1 = settled on `to`).
    pub phase: f32,
}

/// One selectable modal row: a key (primary label), an optional description, and an optional
/// right-aligned meta cell with an optional semantic color.
#[derive(Debug, Clone, Default)]
pub struct ModalRow {
    /// The primary row label (e.g. a model id, session title, command name).
    pub key: String,
    /// A secondary description shown after the key column (clipped to keep the meta gap).
    pub desc: Option<String>,
    /// A right-aligned meta cell (e.g. `128k`, `running`, `default`).
    pub meta: Option<String>,
    /// The semantic color for `meta`; `None` renders it in the muted tone.
    pub meta_color: Option<Color>,
}

impl ModalRow {
    /// A plain row carrying only a key label.
    #[must_use]
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            ..Self::default()
        }
    }
}

/// The optional search row: the typed query and the placeholder shown when it is empty.
#[derive(Debug, Clone, Default)]
pub struct ModalSearch {
    /// The current query text (filters the rows; filtering is the caller's responsibility).
    pub query: String,
    /// The faint placeholder shown when `query` is empty (e.g. `type to filter…`).
    pub placeholder: String,
    /// Whether the `▏` caret is shown even when `query` is empty. The command palette is a
    /// text-entry surface first, so its caret is always present; the sessions/models pickers keep
    /// the default (`false`) and show the placeholder until the user types.
    pub caret_when_empty: bool,
}

/// One footer key-hint: the key glyph (dim orange) followed by its action label (faint grey).
#[derive(Debug, Clone)]
pub struct ModalHint {
    /// The key glyph/name (e.g. `esc`, `↵`, `↑/↓`).
    pub key: String,
    /// The action label (e.g. `close`, `open`, `move`).
    pub label: String,
}

impl ModalHint {
    /// A hint pair `key label`.
    #[must_use]
    pub fn new(key: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
        }
    }
}

/// An optional in-frame single-line text input (e.g. the session-rename sub-mode).
#[derive(Debug, Clone)]
pub struct ModalInput {
    /// The label prefix (e.g. `Rename: `).
    pub prefix: String,
    /// The in-progress text value.
    pub value: String,
}

/// The full specification for one modal render.
#[derive(Debug, Clone)]
pub struct ModalSpec {
    /// The title embedded in the top border (AMBER).
    pub title: String,
    /// The selectable rows, in display order (already filtered by any active search).
    pub rows: Vec<ModalRow>,
    /// The highlighted row index (clamped to `rows`).
    pub selected: usize,
    /// The footer key-hints, left-aligned inside the bottom border.
    pub hints: Vec<ModalHint>,
    /// An optional right-aligned footer note (dim orange).
    pub note: Option<String>,
    /// The optional search row.
    pub search: Option<ModalSearch>,
    /// An optional in-frame text-input row (rename, etc.).
    pub input: Option<ModalInput>,
    /// The message shown when `rows` is empty (no-results / empty state).
    pub empty: Option<String>,
    /// The preferred modal width (clamped to the available area).
    pub width: u16,
    /// A fixed key-column width so descriptions line up across rows (`None` = per-row key length).
    pub key_width: Option<usize>,
    /// The color of a non-selected row's key text.
    pub key_color: Color,
}

/// Render `spec` as a centered modal over `area`, applying the resolved selection transition `anim`.
pub fn render_modal(
    frame: &mut Frame,
    area: Rect,
    spec: &ModalSpec,
    theme: &ThemeColors,
    anim: Option<SelectionAnim>,
) {
    if area.width < 12 || area.height < 6 {
        return;
    }

    let mw = modal_width(spec, area);
    let interior_width = (mw as usize).saturating_sub(2).max(1);

    let search_rows = usize::from(spec.search.is_some());
    let extra_search_rule = search_rows; // the underline rule under the search input
    let input_rows = usize::from(spec.input.is_some());
    // Fixed non-body rows inside the modal: search input + rule, the input row, a gap, and the
    // footer, plus the two border rows.
    let chrome_rows = 2 + search_rows + extra_search_rule + input_rows + 1 /*gap*/ + 1 /*footer*/;

    let body_wanted = spec.rows.len().max(1);
    let max_mh = ((area.height as usize) * 9 / 10)
        .max(8)
        .min(area.height as usize);
    let body_max = max_mh.saturating_sub(chrome_rows).max(1);
    let body_visible = body_wanted.min(body_max).clamp(1, MAX_MODAL_BODY_ROWS);

    let mh = (body_visible + chrome_rows) as u16;
    let popup = centered_rect(mw, mh, area);
    let inner = Rect {
        x: popup.x + 1,
        y: popup.y + 1,
        width: mw.saturating_sub(2),
        height: mh.saturating_sub(2),
    };

    let lines = build_interior_lines(spec, interior_width, body_visible, anim, theme);

    let block = Block::default()
        .style(Style::default().bg(theme.background))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border))
        .title(format!(" {} ", spec.title))
        .title_style(
            Style::default()
                .fg(theme.secondary)
                .add_modifier(Modifier::BOLD),
        );

    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().fg(theme.text).bg(theme.background)),
        inner,
    );
}

/// The clamped modal width for `spec` in `area`.
fn modal_width(spec: &ModalSpec, area: Rect) -> u16 {
    let ceiling = ((area.width as u32 * 9 / 10) as u16)
        .max(MIN_MODAL_WIDTH)
        .min(area.width);
    spec.width
        .clamp(MIN_MODAL_WIDTH.min(area.width), ceiling)
        .min(area.width)
}

/// The scroll offset that keeps `selected` inside a window of `visible` rows over `total` rows.
#[must_use]
pub(crate) fn scroll_offset(selected: usize, visible: usize, total: usize) -> usize {
    if visible == 0 || total <= visible {
        return 0;
    }
    let raw = if selected < visible {
        0
    } else {
        selected + 1 - visible
    };
    raw.min(total - visible)
}

/// Build every interior line of the modal (search, body, input, gap, footer), each exactly
/// `interior_width` columns wide with an explicit background on every cell.
#[must_use]
pub(crate) fn build_interior_lines(
    spec: &ModalSpec,
    interior_width: usize,
    body_visible: usize,
    anim: Option<SelectionAnim>,
    theme: &ThemeColors,
) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    if let Some(search) = &spec.search {
        lines.push(search_line(search, interior_width, theme));
        lines.push(rule_line(interior_width, theme));
    }

    if spec.rows.is_empty() {
        let message = spec
            .empty
            .clone()
            .unwrap_or_else(|| "no matches".to_owned());
        lines.push(empty_line(&message, interior_width, theme));
        for _ in 1..body_visible {
            lines.push(blank_line(interior_width, theme));
        }
    } else {
        let total = spec.rows.len();
        let selected = spec.selected.min(total - 1);
        let offset = scroll_offset(selected, body_visible, total);
        let end = (offset + body_visible).min(total);
        for abs in offset..end {
            lines.push(body_row_line(
                spec,
                abs,
                selected,
                interior_width,
                anim,
                theme,
            ));
        }
        for _ in (end - offset)..body_visible {
            lines.push(blank_line(interior_width, theme));
        }
    }

    if let Some(input) = &spec.input {
        lines.push(input_line(input, interior_width, theme));
    }

    lines.push(blank_line(interior_width, theme));
    lines.push(footer_line(spec, interior_width, theme));

    lines
}

/// Build one body row: `[› ]key   desc … meta`, with the inverse selection bar / transition fill
/// applied per the resolved animation phase.
fn body_row_line(
    spec: &ModalSpec,
    abs: usize,
    selected: usize,
    interior_width: usize,
    anim: Option<SelectionAnim>,
    theme: &ThemeColors,
) -> Line<'static> {
    let interior_bg = theme.background;
    let base = Style::default().fg(theme.text).bg(interior_bg);
    let mut cells: Vec<(char, Style)> = vec![(' ', base); interior_width];

    let (row_bg, is_marker) = selection_state(abs, selected, anim, theme);

    if is_marker {
        place_segments(
            &mut cells,
            0,
            &[(
                glyphs::SELECT_MARKER.to_owned(),
                Style::default().fg(theme.primary).bg(interior_bg),
            )],
        );
    }

    let row = &spec.rows[abs];
    let key_col = 2usize;
    let key_style = if is_marker {
        Style::default()
            .fg(theme.text_bright)
            .bg(interior_bg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(spec.key_color).bg(interior_bg)
    };
    place_segments(&mut cells, key_col, &[(row.key.clone(), key_style)]);

    let kw = spec.key_width.unwrap_or_else(|| row.key.chars().count());
    let desc_start = key_col + kw + 2;

    let meta_len = row.meta.as_ref().map_or(0, |m| m.chars().count());
    let meta_start = if meta_len > 0 {
        interior_width.saturating_sub(1 + meta_len)
    } else {
        interior_width
    };

    if let Some(desc) = &row.desc {
        if desc_start < meta_start {
            // Leave a ≥2-col gap between the description and the meta column (or the right border).
            let limit = meta_start.saturating_sub(desc_start).saturating_sub(2);
            let clipped = clip(desc, limit);
            if !clipped.is_empty() {
                place_segments(
                    &mut cells,
                    desc_start,
                    &[(clipped, Style::default().fg(theme.text_dim).bg(interior_bg))],
                );
            }
        }
    }

    if let Some(meta) = &row.meta {
        let color = row.meta_color.unwrap_or(theme.text_muted);
        place_segments(
            &mut cells,
            meta_start,
            &[(meta.clone(), Style::default().fg(color).bg(interior_bg))],
        );
    }

    if let Some(bg) = row_bg {
        for cell in &mut cells {
            cell.1 = cell.1.bg(bg);
        }
    }

    cells_to_line(&cells)
}

/// The selection background (if any) and whether this row carries the `›` marker, resolving the
/// steady selection or the in-flight cross-fade transition.
fn selection_state(
    abs: usize,
    selected: usize,
    anim: Option<SelectionAnim>,
    theme: &ThemeColors,
) -> (Option<Color>, bool) {
    let sel_bg = theme.surface[3];
    match anim {
        None => (
            if abs == selected { Some(sel_bg) } else { None },
            abs == selected,
        ),
        Some(a) => {
            let interior_bg = theme.background;
            let weight = if abs == a.from {
                1.0 - a.phase
            } else if abs == a.to {
                a.phase
            } else {
                0.0
            };
            let bg = (weight > 0.02).then(|| lerp_color(interior_bg, sel_bg, weight));
            let marker_row = if a.phase >= 0.5 { a.to } else { a.from };
            (bg, abs == marker_row)
        }
    }
}

/// The search input row: `› ` + query (BRIGHT) + `▏` caret, or the faint placeholder when empty.
fn search_line(search: &ModalSearch, interior_width: usize, theme: &ThemeColors) -> Line<'static> {
    let bg = theme.background;
    let mut cells: Vec<(char, Style)> =
        vec![(' ', Style::default().fg(theme.text).bg(bg)); interior_width];
    place_segments(
        &mut cells,
        0,
        &[(
            glyphs::PROMPT.to_owned(),
            Style::default().fg(theme.primary).bg(bg),
        )],
    );
    if search.query.is_empty() && !search.caret_when_empty {
        place_segments(
            &mut cells,
            2,
            &[(
                clip(&search.placeholder, interior_width.saturating_sub(2)),
                Style::default().fg(theme.text_muted).bg(bg),
            )],
        );
    } else {
        let q = clip(&search.query, interior_width.saturating_sub(3));
        let caret_col = 2 + q.chars().count();
        place_segments(
            &mut cells,
            2,
            &[(q, Style::default().fg(theme.text_bright).bg(bg))],
        );
        place_segments(
            &mut cells,
            caret_col,
            &[(
                glyphs::CURSOR.to_owned(),
                Style::default().fg(theme.primary).bg(bg),
            )],
        );
    }
    cells_to_line(&cells)
}

/// The `─` underline rule under the search row (GHOST).
fn rule_line(interior_width: usize, theme: &ThemeColors) -> Line<'static> {
    let bg = theme.background;
    let cells: Vec<(char, Style)> = vec![
        (
            glyphs::BOX_HORIZONTAL.chars().next().unwrap_or('-'),
            Style::default().fg(theme.ghost).bg(bg),
        );
        interior_width
    ];
    cells_to_line(&cells)
}

/// The in-frame text-input row: `  prefix value▏`.
fn input_line(input: &ModalInput, interior_width: usize, theme: &ThemeColors) -> Line<'static> {
    let bg = theme.background;
    let mut cells: Vec<(char, Style)> =
        vec![(' ', Style::default().fg(theme.text).bg(bg)); interior_width];
    place_segments(
        &mut cells,
        2,
        &[
            (
                input.prefix.clone(),
                Style::default()
                    .fg(theme.secondary)
                    .bg(bg)
                    .add_modifier(Modifier::BOLD),
            ),
            (
                input.value.clone(),
                Style::default().fg(theme.text_bright).bg(bg),
            ),
            (
                glyphs::CURSOR.to_owned(),
                Style::default().fg(theme.primary).bg(bg),
            ),
        ],
    );
    cells_to_line(&cells)
}

/// The no-results / empty-state row (faint, italic).
fn empty_line(message: &str, interior_width: usize, theme: &ThemeColors) -> Line<'static> {
    let bg = theme.background;
    let mut cells: Vec<(char, Style)> =
        vec![(' ', Style::default().fg(theme.text).bg(bg)); interior_width];
    place_segments(
        &mut cells,
        2,
        &[(
            clip(message, interior_width.saturating_sub(2)),
            Style::default()
                .fg(theme.text_muted)
                .bg(bg)
                .add_modifier(Modifier::ITALIC),
        )],
    );
    cells_to_line(&cells)
}

/// A blank interior row (spaces on the interior background).
fn blank_line(interior_width: usize, theme: &ThemeColors) -> Line<'static> {
    let cells: Vec<(char, Style)> =
        vec![(' ', Style::default().fg(theme.text).bg(theme.background)); interior_width];
    cells_to_line(&cells)
}

/// The footer key-hint row: `key label   key label …` (keys dim orange, labels faint) plus an
/// optional right-aligned note.
fn footer_line(spec: &ModalSpec, interior_width: usize, theme: &ThemeColors) -> Line<'static> {
    let bg = theme.background;
    let mut cells: Vec<(char, Style)> =
        vec![(' ', Style::default().fg(theme.text).bg(bg)); interior_width];

    let key_style = Style::default().fg(theme.border).bg(bg);
    let label_style = Style::default().fg(theme.text_muted).bg(bg);
    let mut segments: Vec<(String, Style)> = Vec::new();
    for (i, hint) in spec.hints.iter().enumerate() {
        if i > 0 {
            segments.push(("   ".to_owned(), label_style));
        }
        segments.push((hint.key.clone(), key_style));
        segments.push((" ".to_owned(), label_style));
        segments.push((hint.label.clone(), label_style));
    }
    place_segments(&mut cells, 0, &segments);

    if let Some(note) = &spec.note {
        let note_len = note.chars().count();
        if note_len + 1 < interior_width {
            let start = interior_width - note_len - 1;
            place_segments(
                &mut cells,
                start,
                &[(note.clone(), Style::default().fg(theme.border).bg(bg))],
            );
        }
    }

    cells_to_line(&cells)
}

/// Truncate `s` to at most `max` columns, appending `…` when it is cut.
fn clip(s: &str, max: usize) -> String {
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

/// Blend one channel from `a` to `b` at `t` in `[0, 1]`.
fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    let t = t.clamp(0.0, 1.0);
    (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8
}

/// Blend color `a` toward `b` at `t`; falls back to a hard pick for non-RGB colors.
fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    match (a, b) {
        (Color::Rgb(ar, ag, ab), Color::Rgb(br, bg, bb)) => {
            Color::Rgb(lerp_u8(ar, br, t), lerp_u8(ag, bg, t), lerp_u8(ab, bb, t))
        }
        _ => {
            if t < 0.5 {
                a
            } else {
                b
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;

    fn theme() -> ThemeColors {
        ThemeColors::dark()
    }

    fn spec(rows: Vec<ModalRow>, selected: usize) -> ModalSpec {
        let t = theme();
        ModalSpec {
            title: "Sessions".to_owned(),
            rows,
            selected,
            hints: vec![ModalHint::new("↵", "open"), ModalHint::new("esc", "close")],
            note: None,
            search: None,
            input: None,
            empty: None,
            width: 66,
            key_width: None,
            key_color: t.text,
        }
    }

    fn plain_rows(keys: &[&str]) -> Vec<ModalRow> {
        keys.iter().map(|k| ModalRow::new(*k)).collect()
    }

    fn line_text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn scroll_offset_keeps_selection_in_window() {
        // VAL-PICK-066: paging keeps the selection inside the visible window and never past the end.
        assert_eq!(scroll_offset(0, 5, 20), 0);
        assert_eq!(scroll_offset(4, 5, 20), 0);
        assert_eq!(scroll_offset(5, 5, 20), 1);
        assert_eq!(scroll_offset(10, 5, 20), 6);
        assert_eq!(scroll_offset(19, 5, 20), 15);
        // Never scrolls past the last full window.
        assert_eq!(scroll_offset(19, 5, 20), 20 - 5);
        // Fits entirely: no scroll.
        assert_eq!(scroll_offset(3, 10, 4), 0);
    }

    #[test]
    fn selected_row_is_inverse_bar_with_marker_and_bright_text() {
        // VAL-PICK-004: the highlighted row is a solid inverse bar (surface[3] fill) with a leading
        // `›` marker in orange and brightened key text; other rows have no fill and no marker.
        let t = theme();
        let s = spec(plain_rows(&["alpha", "beta", "gamma"]), 1);
        let lines = build_interior_lines(&s, 60, 3, None, &t);
        // No search → body starts at line 0.
        let sel = &lines[1];
        // Marker span is `›` in the primary (ORANGE) on the selection background.
        let marker = &sel.spans[0];
        assert_eq!(marker.content.as_ref(), glyphs::SELECT_MARKER);
        assert_eq!(marker.style.fg, Some(t.primary));
        assert_eq!(marker.style.bg, Some(t.surface[3]));
        // Every cell of the selected row carries the selection background.
        assert!(
            sel.spans.iter().all(|sp| sp.style.bg == Some(t.surface[3])),
            "selection bar fills the whole row"
        );
        // The key text is brightened.
        let key_span = sel
            .spans
            .iter()
            .find(|sp| sp.content.contains("beta"))
            .expect("selected key present");
        assert_eq!(key_span.style.fg, Some(t.text_bright));
        // A non-selected row has no selection fill and no marker glyph.
        let other = &lines[0];
        assert!(!line_text(other).contains(glyphs::SELECT_MARKER));
        assert!(
            other
                .spans
                .iter()
                .all(|sp| sp.style.bg != Some(t.surface[3]))
        );
    }

    #[test]
    fn meta_column_is_right_aligned_with_a_two_col_gap() {
        // VAL-PICK meta: the meta cell is right-aligned (1 col before the border) and the desc is
        // clipped to leave a ≥2-col gap before it, with the meta in its semantic color.
        let t = theme();
        let mut rows = plain_rows(&["model-a"]);
        rows[0].desc = Some("a very long description that must be clipped".to_owned());
        rows[0].meta = Some("128k".to_owned());
        rows[0].meta_color = Some(t.success);
        let mut s = spec(rows, 0);
        s.search = None;
        let interior = 40usize;
        let lines = build_interior_lines(&s, interior, 1, None, &t);
        let row = &lines[0];
        let text = line_text(row);
        // Meta ends one column before the interior's right edge.
        let chars: Vec<char> = text.chars().collect();
        assert_eq!(chars.len(), interior, "row is exactly the interior width");
        assert_eq!(
            chars[interior - 5..interior].iter().collect::<String>(),
            "128k "
        );
        // Meta carries its semantic color.
        let meta_span = row
            .spans
            .iter()
            .find(|sp| sp.content.contains("128k"))
            .expect("meta present");
        assert_eq!(meta_span.style.fg, Some(t.success));
        // The description is clipped with an ellipsis and leaves a gap before the meta.
        assert!(text.contains(glyphs::ELLIPSIS), "desc clipped: {text}");
        let meta_start = interior - 5; // 4 meta chars + 1 border gap
        // At least two spaces precede the meta column.
        assert_eq!(chars[meta_start - 1], ' ');
        assert_eq!(chars[meta_start - 2], ' ');
    }

    #[test]
    fn empty_list_shows_a_no_results_state_and_no_highlight() {
        // VAL-PICK-065: an empty (filtered-to-zero) list shows an explicit no-results line and no
        // highlighted row.
        let t = theme();
        let mut s = spec(Vec::new(), 0);
        s.empty = Some("no matches".to_owned());
        s.search = Some(ModalSearch {
            query: "zzz".to_owned(),
            placeholder: "type to filter…".to_owned(),
            caret_when_empty: false,
        });
        let lines = build_interior_lines(&s, 60, 3, None, &t);
        let joined: String = lines.iter().map(line_text).collect::<Vec<_>>().join("\n");
        assert!(joined.contains("no matches"), "no-results state: {joined}");
        // No row carries the inverse selection fill when the list is empty (the `›` in the search
        // row is the prompt glyph, not a selection marker, so check the fill rather than the glyph).
        assert!(
            lines
                .iter()
                .all(|l| l.spans.iter().all(|sp| sp.style.bg != Some(t.surface[3]))),
            "no highlighted row when empty: {joined}"
        );
    }

    #[test]
    fn search_row_shows_query_and_caret_or_placeholder() {
        let t = theme();
        // Empty query → placeholder, no caret.
        let mut s = spec(plain_rows(&["a"]), 0);
        s.search = Some(ModalSearch {
            query: String::new(),
            placeholder: "type to filter…".to_owned(),
            caret_when_empty: false,
        });
        let lines = build_interior_lines(&s, 60, 1, None, &t);
        let search = line_text(&lines[0]);
        assert!(
            search.starts_with(glyphs::PROMPT),
            "search prompt: {search}"
        );
        assert!(search.contains("type to filter…"));
        assert!(
            !search.contains(glyphs::CURSOR),
            "no caret when empty: {search}"
        );
        // Non-empty query → query + caret.
        s.search = Some(ModalSearch {
            query: "gem".to_owned(),
            placeholder: "type to filter…".to_owned(),
            caret_when_empty: false,
        });
        let lines = build_interior_lines(&s, 60, 1, None, &t);
        let search = line_text(&lines[0]);
        assert!(search.contains("gem"));
        assert!(search.contains(glyphs::CURSOR), "caret present: {search}");
    }

    #[test]
    fn search_caret_shows_when_empty_if_requested() {
        // VAL-PICK-059: a text-entry-first surface (the command palette) shows the `▏` caret in the
        // search row even before anything is typed, rather than a placeholder.
        let t = theme();
        let mut s = spec(plain_rows(&["a"]), 0);
        s.search = Some(ModalSearch {
            query: String::new(),
            placeholder: String::new(),
            caret_when_empty: true,
        });
        let lines = build_interior_lines(&s, 60, 1, None, &t);
        let search = line_text(&lines[0]);
        assert!(
            search.starts_with(glyphs::PROMPT),
            "search prompt: {search}"
        );
        assert!(
            search.contains(glyphs::CURSOR),
            "caret present when empty: {search}"
        );
    }

    #[test]
    fn footer_shows_key_hints_inside_the_frame() {
        // VAL-PICK-005: the footer lists hints with the key in dim orange and the label in faint
        // grey, and `esc close` is present.
        let t = theme();
        let s = spec(plain_rows(&["a", "b"]), 0);
        let lines = build_interior_lines(&s, 60, 2, None, &t);
        let footer = lines.last().expect("footer row");
        let text = line_text(footer);
        assert!(text.contains("esc"), "esc key hint: {text}");
        assert!(text.contains("close"), "close label: {text}");
        // Key glyph uses the dim-orange border tone; label uses the muted tone.
        let key_span = footer
            .spans
            .iter()
            .find(|sp| sp.content.contains("esc"))
            .expect("esc span");
        assert_eq!(key_span.style.fg, Some(t.border));
    }

    #[test]
    fn animation_cross_fades_between_old_and_new_rows() {
        // VAL-FOUND-035: with a transition in flight, the highlight is in an in-between state — the
        // old and new rows both carry a partial fill (neither the interior bg nor the full sel bg).
        let t = theme();
        let s = spec(plain_rows(&["a", "b", "c", "d"]), 2);
        let anim = Some(SelectionAnim {
            from: 1,
            to: 2,
            phase: 0.5,
        });
        let lines = build_interior_lines(&s, 60, 4, anim, &t);
        let from_bg = lines[1].spans[0].style.bg.expect("from row bg");
        let to_bg = lines[2].spans[0].style.bg.expect("to row bg");
        // Both are partial: distinct from the interior background AND from the full selection bg.
        for bg in [from_bg, to_bg] {
            assert_ne!(
                bg, t.background,
                "mid-transition fill is not the interior bg"
            );
            assert_ne!(
                bg, t.surface[3],
                "mid-transition fill is not the full sel bg"
            );
        }
    }

    #[test]
    fn animation_settles_on_destination_and_no_anim_is_instant() {
        let t = theme();
        let s = spec(plain_rows(&["a", "b", "c"]), 1);
        // Settled (phase 1.0): destination row is the full selection bg, origin has no fill.
        let anim = Some(SelectionAnim {
            from: 0,
            to: 1,
            phase: 1.0,
        });
        let lines = build_interior_lines(&s, 60, 3, anim, &t);
        assert_eq!(lines[1].spans[0].style.bg, Some(t.surface[3]));
        assert!(
            lines[0]
                .spans
                .iter()
                .all(|sp| sp.style.bg != Some(t.surface[3]))
        );
        // No animation (None) → the selection is instantly on the current row.
        let lines = build_interior_lines(&s, 60, 3, None, &t);
        assert_eq!(lines[1].spans[0].style.bg, Some(t.surface[3]));
    }

    #[test]
    fn render_modal_draws_rounded_border_and_amber_title() {
        // VAL-PICK-003: a rounded border in dim orange with an amber title embedded in the top edge.
        let t = theme();
        let s = spec(plain_rows(&["alpha", "beta"]), 0);
        let backend = TestBackend::new(92, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 92, 30);
        terminal
            .draw(|f| render_modal(f, area, &s, &t, None))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let text: String = buffer.content().iter().map(|c| c.symbol()).collect();
        assert!(text.contains("Sessions"), "title drawn: overlay");
        // The rounded corners are present somewhere in the buffer.
        assert!(text.contains('╭') && text.contains('╯'), "rounded border");
        // Find a title cell and confirm it is AMBER; find a border corner and confirm ORANGE_D.
        let mut title_amber = false;
        let mut border_dim = false;
        for cell in buffer.content() {
            if cell.symbol() == "S" && cell.fg == t.secondary {
                title_amber = true;
            }
            if cell.symbol() == "╭" && cell.fg == t.border {
                border_dim = true;
            }
        }
        assert!(title_amber, "title is amber");
        assert!(border_dim, "border is dim orange");
    }
}
