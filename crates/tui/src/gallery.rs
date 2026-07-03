//! The Wave-7 themes-gallery canonical chat screen.
//!
//! A single fixed-structure chat screen (`theme_dark.png` / `theme_light.png` / …) rendered in the
//! active chrome theme, used to validate the theme system: the same layout renders identically in
//! every theme and only the palette changes. It is reached via the `/theme-gallery` slash command
//! and is modal (Esc returns to the conversation).
//!
//! Structure, top-to-bottom (VAL-THEME-001):
//! - a header wordmark `✦ Cortex Code · chat` with a horizontal rule beneath it,
//! - a `› you  show me the validator` user line,
//! - a `✦ Cortex  here is the schema check:` assistant line,
//! - a fenced Rust code block (language tag + `theme: <name>` tag, per-line numbers, theme syntax),
//! - a row of three semantic status dots (`● valid` / `● 82% ctx` / `● 1 error`),
//! - a rounded composer with a `›` prompt and the `what should we build today…` placeholder,
//! - a 3-zone status bar (`? for help` / centered `theme: <name>` / real model id).
//!
//! Every emitted cell carries an explicit background so a light (non-black) theme never shows a
//! terminal-default black bar (VAL-THEME-011): the canvas is painted first, body/header cells carry
//! the theme background, the status bar carries the theme surface, and the code block carries its
//! own code surface on every cell.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

use tui_components::style::ThemeColors;

use crate::app::App;
use crate::glyphs;
use crate::ui::{cells_to_line, place_segments};

/// Left/right content margin (columns) for the gallery body, matching the conversation content
/// margin and the reference mockups.
const MARGIN: usize = 2;

/// The canonical user prompt shown on the gallery screen.
const USER_TEXT: &str = "show me the validator";
/// The canonical assistant lead-in shown on the gallery screen.
const ASSISTANT_TEXT: &str = "here is the schema check:";
/// The composer placeholder shown on the gallery screen (matches the home composer placeholder).
const COMPOSER_PLACEHOLDER: &str = "what should we build today…";

/// The canonical four-line Rust snippet rendered in the gallery's fenced code block (VAL-THEME-006):
/// `fn validate`, a `let n = a["tool"]` line with a `// name` comment, `REGISTRY.has(n)`, and the
/// closing brace.
const SNIPPET: &str = "fn validate(a: &Value) -> bool {\n    let n = a[\"tool\"];  // name\n    REGISTRY.has(n)\n}";

/// Normalize a configured theme name to its short display form (`ocean_dark` → `ocean`,
/// `vscode-dark` → `vscode`), leaving the canonical names (`dark`/`light`/`monokai`/`ocean`/
/// `vscode`) and any custom name unchanged. The same value is shown in the code-block tag and the
/// status bar so both always read identically (VAL-THEME-005).
#[must_use]
pub fn display_theme_name(name: &str) -> String {
    match name.to_lowercase().as_str() {
        "ocean_dark" => "ocean".to_owned(),
        "vscode_dark" | "vscode-dark" => "vscode".to_owned(),
        other => other.to_owned(),
    }
}

/// Render the whole themes-gallery canonical screen into `area` for the active theme.
pub fn draw_theme_gallery(frame: &mut Frame, app: &App, area: Rect, theme: &ThemeColors) {
    if area.width < 8 || area.height < 8 {
        return;
    }
    // Paint the whole surface with the theme canvas so no cell falls through to terminal-default
    // black (the base for VAL-THEME-011 "no black bars").
    frame.render_widget(
        Block::default().style(Style::default().bg(theme.background)),
        area,
    );

    let width = area.width as usize;
    let name = display_theme_name(app.theme());

    // Row 0: `✦ Cortex Code · chat`.
    frame.render_widget(
        Paragraph::new(header_line(width, theme)).style(Style::default().bg(theme.background)),
        Rect { height: 1, ..area },
    );

    // Row 1: full-width horizontal rule in the ghost tone.
    let rule = glyphs::BOX_HORIZONTAL.repeat(width);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            rule,
            Style::default().fg(theme.ghost).bg(theme.background),
        ))),
        Rect {
            y: area.y + 1,
            height: 1,
            ..area
        },
    );

    // Bottom row: the themed 3-zone status bar on the theme surface.
    let bar_y = area.y + area.height - 1;
    frame.render_widget(
        Paragraph::new(status_bar_line(app, &name, width, theme))
            .style(Style::default().fg(theme.text).bg(theme.surface[0])),
        Rect {
            y: bar_y,
            height: 1,
            ..area
        },
    );

    // Composer: a rounded box occupying the three rows directly above the status bar.
    let composer_area = Rect {
        x: area.x,
        y: bar_y.saturating_sub(3),
        width: area.width,
        height: 3,
    };
    draw_composer(frame, composer_area, theme);

    // Body region: rows between the rule and the composer.
    let body_top = area.y + 2;
    let body_bottom = composer_area.y;
    if body_bottom <= body_top {
        return;
    }
    let body_area = Rect {
        x: area.x,
        y: body_top,
        width: area.width,
        height: body_bottom - body_top,
    };
    let lines = body_lines(app, &name, width, theme);
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().fg(theme.text).bg(theme.background)),
        body_area,
    );
}

/// The header wordmark line: `✦` (primary) + `Cortex Code` (bright, bold) + ` · ` (ghost) + `chat`
/// (secondary/amber), inset by [`MARGIN`], on the theme background.
fn header_line(width: usize, theme: &ThemeColors) -> Line<'static> {
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
            "chat".to_owned(),
            Style::default().fg(theme.secondary).bg(theme.background),
        ),
    ];
    place_segments(&mut cells, MARGIN, &segments);
    cells_to_line(&cells)
}

/// Build the gallery body: the user line, a blank, the assistant line, a blank, the fenced code
/// block (with a [`MARGIN`] left inset), a blank, and the status-dots row. Every non-code line
/// carries the theme background on its full width (so no black bars), and the code rows carry their
/// own code surface.
fn body_lines(app: &App, name: &str, width: usize, theme: &ThemeColors) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = vec![
        // `› you  show me the validator`.
        marked_line(
            width,
            theme,
            glyphs::PROMPT,
            theme.primary,
            "you",
            theme.text_dim,
            USER_TEXT,
            theme.text,
        ),
        blank_line(width, theme),
        // `✦ Cortex  here is the schema check:`.
        marked_line(
            width,
            theme,
            glyphs::BRAND_STAR,
            theme.secondary,
            "Cortex",
            theme.text_bright,
            ASSISTANT_TEXT,
            theme.text_bright,
        ),
        blank_line(width, theme),
    ];

    // The fenced Rust code block, inset by MARGIN and bound to the active theme (VAL-THEME-004/006).
    let code_width = width.saturating_sub(MARGIN * 2).max(8) as u16;
    let code_lines = frame::markdown::render_gallery_code_block(SNIPPET, "rust", name, code_width);
    let margin_span = Span::styled(
        " ".repeat(MARGIN),
        Style::default().bg(theme.background),
    );
    for line in code_lines {
        let mut spans = Vec::with_capacity(line.spans.len() + 1);
        spans.push(margin_span.clone());
        spans.extend(line.spans);
        lines.push(Line::from(spans));
    }
    lines.push(blank_line(width, theme));

    // The three semantic status dots row.
    lines.push(status_dots_line(app, width, theme));

    lines
}

/// A blank body row painted with the theme background across its full width.
fn blank_line(width: usize, theme: &ThemeColors) -> Line<'static> {
    Line::from(Span::styled(
        " ".repeat(width.max(1)),
        Style::default().bg(theme.background),
    ))
}

/// A `<glyph> <who>  <text>` body row (user / assistant lines): the author `glyph` in `glyph_color`
/// at the content margin, the `who` label in `who_color`, then the message `text` in `text_color`.
/// The whole row width carries the theme background.
#[allow(clippy::too_many_arguments)]
fn marked_line(
    width: usize,
    theme: &ThemeColors,
    glyph: &str,
    glyph_color: ratatui::style::Color,
    who: &str,
    who_color: ratatui::style::Color,
    text: &str,
    text_color: ratatui::style::Color,
) -> Line<'static> {
    let base = Style::default().fg(theme.text).bg(theme.background);
    let mut cells = vec![(' ', base); width.max(1)];
    let bg = theme.background;
    let who_col = MARGIN + glyph.chars().count() + 1;
    let text_col = who_col + who.chars().count() + 2;
    place_segments(
        &mut cells,
        MARGIN,
        &[(glyph.to_owned(), Style::default().fg(glyph_color).bg(bg))],
    );
    place_segments(
        &mut cells,
        who_col,
        &[(who.to_owned(), Style::default().fg(who_color).bg(bg))],
    );
    place_segments(
        &mut cells,
        text_col,
        &[(text.to_owned(), Style::default().fg(text_color).bg(bg))],
    );
    cells_to_line(&cells)
}

/// The status-dots row: `● valid` (success), `● 82% ctx` (warning), `● 1 error` (error), each dot +
/// label colored by its semantic tone so all three stay legible in every theme (VAL-THEME-007). The
/// whole row width carries the theme background.
fn status_dots_line(_app: &App, width: usize, theme: &ThemeColors) -> Line<'static> {
    let base = Style::default().fg(theme.text).bg(theme.background);
    let mut cells = vec![(' ', base); width.max(1)];
    let bg = theme.background;
    let label = Style::default().fg(theme.text).bg(bg);
    let dot = glyphs::DOT_FILLED;

    let dots: [(ratatui::style::Color, &str); 3] = [
        (theme.success, "valid"),
        (theme.warning, "82% ctx"),
        (theme.error, "1 error"),
    ];
    let mut col = MARGIN;
    for (tone, text) in dots {
        place_segments(
            &mut cells,
            col,
            &[
                (format!("{dot} "), Style::default().fg(tone).bg(bg)),
                (text.to_owned(), label),
            ],
        );
        // 2-glyph dot+space + label + a 4-column gap before the next dot.
        col += 2 + text.chars().count() + 4;
    }
    cells_to_line(&cells)
}

/// Draw the gallery composer: a rounded box in the theme border color containing a `›` prompt glyph
/// and the `what should we build today…` placeholder, both colored from the active theme
/// (VAL-THEME-008). The box interior carries the theme background so it is never a black band.
fn draw_composer(frame: &mut Frame, area: Rect, theme: &ThemeColors) {
    if area.width < 4 || area.height < 2 {
        return;
    }
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
    let line = Line::from(vec![
        Span::styled(
            format!("{} ", glyphs::PROMPT),
            Style::default().fg(theme.primary).bg(theme.background),
        ),
        Span::styled(
            COMPOSER_PLACEHOLDER.to_owned(),
            Style::default().fg(theme.text_muted).bg(theme.background),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(line).style(Style::default().bg(theme.background)),
        inner,
    );
}

/// The gallery's 3-zone status bar: `? for help` at the left (`?` in the border tone + label
/// faint), the centered `theme: <name>` segment, and the real active model id flush-right, all on
/// the theme surface (VAL-THEME-009). Returns the styled line; the caller paints the surface bg.
fn status_bar_line(app: &App, name: &str, width: usize, theme: &ThemeColors) -> Line<'static> {
    let w = width.max(1);
    let surface = theme.surface[0];
    let base = Style::default().fg(theme.text).bg(surface);
    let mut cells: Vec<(char, Style)> = vec![(' ', base); w];

    // Left zone.
    place_segments(
        &mut cells,
        MARGIN,
        &[
            ("?".to_owned(), Style::default().fg(theme.border).bg(surface)),
            (
                " for help".to_owned(),
                Style::default().fg(theme.text_muted).bg(surface),
            ),
        ],
    );

    // Center zone: `theme: <name>` with its midpoint at the terminal center.
    let center = format!("theme: {name}");
    let center_len = center.chars().count();
    let center_start = w.saturating_sub(center_len) / 2;
    place_segments(
        &mut cells,
        center_start,
        &[(
            center,
            Style::default().fg(theme.text_dim).bg(surface),
        )],
    );

    // Right zone: the real active model id, flush to the right margin.
    let model = app.status.model.clone();
    let model_w = model.chars().count();
    let right_start = w.saturating_sub(MARGIN + model_w);
    place_segments(&mut cells, right_start, &[(model, base)]);

    cells_to_line(&cells)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, Status};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::Color;

    fn app_with_theme(theme: &str) -> App {
        let mut a = App::new(Status {
            model: "google/gemini-2.5-flash".to_owned(),
            agent: "build".to_owned(),
            mode: "build".to_owned(),
            variant: "default".to_owned(),
            session: "new".to_owned(),
        });
        a.set_theme(theme.to_owned());
        a
    }

    fn render(theme_name: &str) -> Buffer {
        let a = app_with_theme(theme_name);
        let theme = ThemeColors::from_name(a.theme());
        let backend = TestBackend::new(92, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| draw_theme_gallery(frame, &a, frame.area(), &theme))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn row_text(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width)
            .map(|x| buf[(x, y)].symbol())
            .collect::<Vec<_>>()
            .concat()
    }

    fn all_text(buf: &Buffer) -> String {
        (0..buf.area.height)
            .map(|y| row_text(buf, y))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn renders_every_required_region() {
        // VAL-THEME-001: header wordmark + rule, a `› you` line, a `✦ Cortex` line, a fenced Rust
        // code block, a status-dots row, a composer, and a status bar.
        let buf = render("dark");
        let header = row_text(&buf, 0);
        assert!(header.contains('✦'), "brand star on row 0:\n{header}");
        assert!(header.contains("Cortex Code"), "wordmark on row 0:\n{header}");
        assert!(header.contains("chat"), "chat crumb on row 0:\n{header}");

        let rule = row_text(&buf, 1);
        assert!(
            rule.chars().filter(|&c| c == '─').count() >= 80,
            "row 1 is a full-width rule:\n{rule}"
        );

        let text = all_text(&buf);
        assert!(text.contains("› you"), "user line present");
        assert!(text.contains(USER_TEXT), "user text present");
        assert!(text.contains("✦ Cortex"), "assistant line present");
        assert!(text.contains(ASSISTANT_TEXT), "assistant text present");
        assert!(text.contains("rust"), "code language label present");
        assert!(text.contains("fn validate"), "rust snippet fn present");
        assert!(text.contains("a[\"tool\"]"), "rust snippet let line present");
        assert!(text.contains("// name"), "rust snippet comment present");
        assert!(text.contains("REGISTRY.has(n)"), "rust snippet call present");
        assert!(text.contains("valid"), "status dot `valid`");
        assert!(text.contains("82% ctx"), "status dot `82% ctx`");
        assert!(text.contains("1 error"), "status dot `1 error`");
        assert!(text.contains(COMPOSER_PLACEHOLDER), "composer placeholder");
    }

    #[test]
    fn code_block_shows_line_numbers_and_theme_tag() {
        // VAL-THEME-005/006: per-line numbers 1..4 and a `theme: <name>` tag in the code header.
        let buf = render("dark");
        let text = all_text(&buf);
        for n in ["1", "2", "3", "4"] {
            assert!(text.contains(n), "line number {n} present");
        }
        assert!(text.contains("theme: dark"), "code-block theme tag present:\n{text}");
    }

    #[test]
    fn status_bar_is_themed_and_three_zoned() {
        // VAL-THEME-009: `? for help` left, `theme: <name>` centered, the real model id right.
        let buf = render("dark");
        let bar = row_text(&buf, 29);
        assert!(bar.contains("? for help"), "left zone:\n{bar}");
        assert!(bar.contains("theme: dark"), "center zone:\n{bar}");
        assert!(
            bar.contains("google/gemini-2.5-flash"),
            "right zone shows the real model id:\n{bar}"
        );
        assert!(!bar.contains("opus-4.8"), "never the mockup model id:\n{bar}");
    }

    #[test]
    fn theme_tag_matches_between_code_block_and_status_bar() {
        // VAL-THEME-005: both the code tag and the status bar read the same theme name, and it
        // updates when the theme switches.
        for (name, shown) in [
            ("dark", "theme: dark"),
            ("light", "theme: light"),
            ("ocean", "theme: ocean"),
            ("vscode", "theme: vscode"),
        ] {
            let buf = render(name);
            let text = all_text(&buf);
            let bar = row_text(&buf, 29);
            assert!(text.contains(shown), "{name}: code tag reads `{shown}`");
            assert!(bar.contains(shown), "{name}: status bar reads `{shown}`");
        }
    }

    #[test]
    fn dark_theme_uses_the_brand_palette() {
        // VAL-THEME-013: near-black canvas + ORANGE star + AMBER `chat` crumb.
        let buf = render("dark");
        // The `✦` star cell is ORANGE on the near-black canvas.
        let star = &buf[(MARGIN as u16, 0)];
        assert_eq!(star.symbol(), "✦");
        assert_eq!(star.fg, Color::Rgb(232, 146, 86), "star is brand ORANGE");
        assert_eq!(star.bg, Color::Rgb(13, 13, 13), "near-black canvas behind the star");
    }

    #[test]
    fn light_theme_has_no_black_bars() {
        // VAL-THEME-011: on the light theme every cell of the whole screen carries an explicit
        // light (non-black) background — no band/row/region falls through to terminal-default black
        // or the near-black brand canvas.
        let buf = render("light");
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                let cell = &buf[(x, y)];
                assert_ne!(
                    cell.bg,
                    Color::Reset,
                    "cell ({x},{y}) has an explicit bg (no default-black bar)"
                );
                match cell.bg {
                    Color::Rgb(r, g, b) => {
                        assert!(
                            r as u16 + g as u16 + b as u16 > 300,
                            "cell ({x},{y}) bg {:?} is a light background, not a black bar",
                            cell.bg
                        );
                    }
                    other => panic!("cell ({x},{y}) bg {other:?} is not an explicit RGB light bg"),
                }
            }
        }
    }

    #[test]
    fn light_theme_uses_white_chrome() {
        // VAL-THEME-015: white canvas + green primary accents (the `›`/`✦` marks and composer
        // border read from the light palette).
        let buf = render("light");
        let star = &buf[(MARGIN as u16, 0)];
        assert_eq!(star.symbol(), "✦");
        assert_eq!(star.bg, Color::Rgb(255, 255, 255), "white canvas");
        assert_eq!(star.fg, Color::Rgb(0, 150, 100), "light primary is green");
    }

    #[test]
    fn no_forbidden_glyphs_or_tofu() {
        // VAL-THEME (cross): only safe glyphs; no braille, thin triangles, `>_`, or curly quotes.
        for name in ["dark", "light", "monokai", "ocean", "vscode"] {
            let text = all_text(&render(name));
            assert!(!glyphs::contains_braille(&text), "{name}: no braille");
            for forbidden in ['▸', '▾', '⌘', '⬡', '“', '”', '‘', '’'] {
                assert!(!text.contains(forbidden), "{name}: forbidden glyph {forbidden:?}");
            }
            assert!(!text.contains(">_"), "{name}: no `>_` cliché");
        }
    }

    #[test]
    fn dark_and_light_code_block_syntax_differ() {
        // VAL-THEME-014/016: the `fn` keyword is orange under dark and blue under light (the syntax
        // palette follows the chrome theme).
        let dark = render("dark");
        let light = render("light");
        let kw_fg = |buf: &Buffer| {
            for y in 0..buf.area.height {
                let row = row_text(buf, y);
                if let Some(idx) = row.find("fn ") {
                    let col = row[..idx].chars().count() as u16;
                    return buf[(col, y)].fg;
                }
            }
            Color::Reset
        };
        let dark_kw = kw_fg(&dark);
        let light_kw = kw_fg(&light);
        assert_eq!(dark_kw, Color::Rgb(232, 146, 86), "dark `fn` keyword is orange");
        assert_eq!(light_kw, Color::Rgb(0, 0, 200), "light `fn` keyword is blue");
        assert_ne!(dark_kw, light_kw, "syntax palette follows the chrome theme");
    }
}
