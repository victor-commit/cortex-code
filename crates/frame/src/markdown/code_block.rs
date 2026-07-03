// SPDX-License-Identifier: MIT
// Provenance: ported source (direct-port-allowed), adapted to this workspace
// Source-Repo: CortexLM/cortex-code
// Source-Commit: 7954d0218faf6f26aa84df0f6d8b09ea9f2e5ac9
// Source-Path: src/cortex-core/src/markdown/code_block.rs
// Target-Path: crates/frame/src/markdown/code_block.rs
// Port-Mode: adapt
// Adaptation: crate renamed cortex-frame -> frame; syntax crate renamed cortex-tui-syntax ->
//   tui-syntax (cortex_tui_syntax -> tui_syntax) and cortex-tui-text -> tui-text
//   (cortex_tui_text -> tui_text); the unused upstream `pad_to_width` helper + its test are
//   dropped. Re-introduces the `CodeBlockRenderer` highlighting seam that markdown-render removed.
// Upstream-License: MIT
// See: NOTICE (Direct-Port Provenance Ledger)

//! Code block rendering with syntax highlighting.
//!
//! This module provides rendering for fenced code blocks with:
//! - Syntax highlighting via tui_syntax
//! - Bordered display with language tags
//! - Optional line numbers
//! - Incremental rendering for streaming content
//!
//! # Example
//!
//! ```rust,ignore
//! use frame::markdown::code_block::{CodeBlockRenderer, IncrementalCodeBlock};
//! use tui_syntax::Highlighter;
//! use std::sync::Arc;
//!
//! let highlighter = Arc::new(Highlighter::new());
//! let renderer = Arc::new(CodeBlockRenderer::new(highlighter));
//!
//! // One-shot rendering
//! let lines = renderer.render("fn main() {}", Some("rust"), 80);
//!
//! // Streaming rendering
//! let mut block = IncrementalCodeBlock::new(renderer, Some("rust".to_string()));
//! block.append("fn main");
//! block.append("() {}");
//! let lines = block.get_lines(80);
//! ```

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use std::sync::Arc;
use tui_syntax::{HighlightSpan, Highlighter};

use crate::style::{BORDER_BRAND, CODEBG, FAINT, TXT};

// ============================================================
// Copy affordance
// ============================================================

/// The copy hint shown at the right of a code-block header bar.
const COPY_HINT: &str = "copy ^C";

// ============================================================
// CodeBlockRenderer
// ============================================================

/// Renderer for code blocks with syntax highlighting.
///
/// This renderer draws a fenced code block as a solid `CODEBG` panel (no box-drawing border):
/// - a header bar carrying the language tag (left) and a copy affordance (right)
/// - content rows with a right-justified line-number gutter and syntax highlighting
/// - every emitted cell carries an explicit background so the panel stays a solid filled
///   rectangle on any theme (never a terminal-default black bar on a light theme)
#[derive(Debug, Clone)]
pub struct CodeBlockRenderer {
    /// The syntax highlighter.
    highlighter: Arc<Highlighter>,
    /// Color for the header/gutter accents (kept for API compatibility).
    border_color: Color,
    /// Background color for the whole code panel.
    background_color: Color,
    /// Style for unhighlighted text.
    text_style: Style,
    /// Style for the language tag (and the copy affordance) in the header bar.
    lang_tag_style: Style,
    /// Style for the right-justified line-number gutter.
    line_number_style: Style,
    /// Whether to show the line-number gutter.
    show_line_numbers: bool,
    /// The right-aligned hint shown in the header bar. `None` uses the default `copy ^C` affordance;
    /// `Some(text)` overrides it (the Wave-7 themes gallery passes `theme: <name>`).
    right_hint: Option<String>,
}

impl CodeBlockRenderer {
    /// Creates a new code block renderer with default (brand) styling.
    pub fn new(highlighter: Arc<Highlighter>) -> Self {
        Self {
            highlighter,
            border_color: BORDER_BRAND,
            background_color: CODEBG,
            text_style: Style::default().fg(TXT),
            lang_tag_style: Style::default().fg(FAINT).add_modifier(Modifier::ITALIC),
            line_number_style: Style::default().fg(FAINT),
            show_line_numbers: false,
            right_hint: None,
        }
    }

    /// Sets the border/accent color.
    pub fn with_border_color(mut self, color: Color) -> Self {
        self.border_color = color;
        self
    }

    /// Sets the background color.
    pub fn with_background(mut self, color: Color) -> Self {
        self.background_color = color;
        self
    }

    /// Sets the default text style.
    pub fn with_text_style(mut self, style: Style) -> Self {
        self.text_style = style;
        self
    }

    /// Sets the language tag style.
    pub fn with_lang_tag_style(mut self, style: Style) -> Self {
        self.lang_tag_style = style;
        self
    }

    /// Sets the line-number gutter style.
    pub fn with_line_number_style(mut self, style: Style) -> Self {
        self.line_number_style = style;
        self
    }

    /// Enables or disables the line-number gutter.
    pub fn with_line_numbers(mut self, show: bool) -> Self {
        self.show_line_numbers = show;
        self
    }

    /// Overrides the header's right-aligned hint (default `copy ^C`). The Wave-7 themes gallery uses
    /// this to show a `theme: <name>` tag instead of the copy affordance.
    pub fn with_right_hint(mut self, hint: impl Into<String>) -> Self {
        self.right_hint = Some(hint.into());
        self
    }

    /// Renders a code block as a solid `CODEBG` panel.
    ///
    /// The output format is:
    /// ```text
    ///  rust                                          copy ^C
    ///  1 use serde_json::Value;
    ///  2
    ///  3 /// validate args against the tool schema
    /// ```
    pub fn render(&self, code: &str, language: Option<&str>, max_width: u16) -> Vec<Line<'static>> {
        let bg = self.background_color;
        let bg_style = Style::default().bg(bg);
        // The header language tag + copy affordance are forced onto the panel background so they
        // never fall through to a terminal-default black bar on a light theme.
        let lang_style = self.lang_tag_style.patch(bg_style);
        let lnum_style = self.line_number_style.patch(bg_style);

        let total_width = (max_width.max(8) as usize).max(COPY_HINT.len() + 4);

        // Highlight the code into per-line span vectors.
        let highlighted_lines =
            highlight_code(&self.highlighter, code, language, self.text_style);
        let line_count = highlighted_lines.len();

        // Right-justified gutter width from the largest line number.
        let number_width = if self.show_line_numbers {
            line_count.max(1).to_string().len()
        } else {
            0
        };

        let mut lines = Vec::with_capacity(line_count + 1);

        // Header bar: language tag (left) + copy affordance (right), on CODEBG.
        lines.push(render_header(
            language,
            self.right_hint.as_deref(),
            total_width,
            bg_style,
            lang_style,
        ));

        // Content rows: gutter + syntax spans, padded to the panel width on CODEBG.
        for (idx, spans) in highlighted_lines.into_iter().enumerate() {
            lines.push(render_code_row(
                spans,
                idx + 1,
                number_width,
                total_width,
                bg_style,
                lnum_style,
            ));
        }

        lines
    }

    /// Renders code without the panel chrome (just highlighted lines).
    ///
    /// Useful when you want syntax highlighting without the header bar / gutter panel.
    pub fn render_bare(&self, code: &str, language: Option<&str>) -> Vec<Line<'static>> {
        let mut highlighted_lines =
            highlight_code(&self.highlighter, code, language, self.text_style);

        // Add line numbers if enabled
        if self.show_line_numbers {
            highlighted_lines = render_with_line_numbers(highlighted_lines, self.line_number_style);
        }

        highlighted_lines.into_iter().map(Line::from).collect()
    }

    /// Returns a reference to the highlighter.
    pub fn highlighter(&self) -> &Highlighter {
        &self.highlighter
    }
}

/// Render the Wave-7 themes-gallery canonical code block for `theme_name`.
///
/// This is the single production render path the gallery screen uses for its fenced Rust snippet.
/// It resolves the theme's code-block surface/tag/gutter colors from [`crate::MarkdownTheme`] and
/// its syntax palette from [`tui_syntax::Theme::gallery`] (so the `dark` snippet uses the
/// mono-orange brand syntax while `light`/`monokai`/`ocean`/`vscode` use their own palettes), draws
/// the language tag on the left and a right-aligned `theme: <name>` hint (replacing `copy ^C`), and
/// forces an explicit background on every cell so a light theme never shows a black bar.
#[must_use]
pub fn render_gallery_code_block(
    code: &str,
    language: &str,
    theme_name: &str,
    max_width: u16,
) -> Vec<Line<'static>> {
    use crate::markdown::MarkdownTheme;
    use crate::markdown::languages::{get_default_highlighter, normalize_language_name};

    let md = MarkdownTheme::from_name(theme_name);
    let syntax = tui_syntax::Theme::gallery(theme_name);
    let highlighter = Arc::new(get_default_highlighter().themed_clone(syntax));
    let canonical = normalize_language_name(language).unwrap_or("rust");
    let renderer = CodeBlockRenderer::new(highlighter)
        .with_background(md.code_block_bg)
        .with_border_color(md.code_block_border)
        .with_text_style(md.code_block_text)
        .with_lang_tag_style(md.code_lang_tag)
        .with_line_number_style(md.code_line_number)
        .with_line_numbers(true)
        .with_right_hint(format!("theme: {theme_name}"));
    renderer.render(code, Some(canonical), max_width)
}

// ============================================================
// IncrementalCodeBlock
// ============================================================

/// Incremental code block for streaming content.
///
/// Caches rendered output and only re-renders when content changes.
/// This is optimized for streaming scenarios where content is appended
/// frequently and you want to minimize re-rendering overhead.
#[derive(Debug)]
pub struct IncrementalCodeBlock {
    /// The source code content.
    source: String,
    /// The language for syntax highlighting.
    language: Option<String>,
    /// Cached rendered lines.
    cached_lines: Vec<Line<'static>>,
    /// Whether the cache is dirty (needs re-render).
    dirty: bool,
    /// The renderer to use.
    renderer: Arc<CodeBlockRenderer>,
    /// Last rendered width (for detecting width changes).
    last_width: u16,
}

impl IncrementalCodeBlock {
    /// Creates a new incremental code block.
    pub fn new(renderer: Arc<CodeBlockRenderer>, language: Option<String>) -> Self {
        Self {
            source: String::new(),
            language,
            cached_lines: Vec::new(),
            dirty: true,
            renderer,
            last_width: 0,
        }
    }

    /// Appends content to the code block.
    ///
    /// Marks the cache as dirty so the next call to `get_lines` will re-render.
    pub fn append(&mut self, content: &str) {
        self.source.push_str(content);
        self.dirty = true;
    }

    /// Sets the entire source content.
    ///
    /// Uses smart diffing to only mark as dirty if content actually changed.
    pub fn set_source(&mut self, source: &str) {
        if self.source != source {
            self.source.clear();
            self.source.push_str(source);
            self.dirty = true;
        }
    }

    /// Gets the rendered lines, re-rendering if necessary.
    ///
    /// This method caches the result, so subsequent calls with the same
    /// content and width will return the cached lines.
    pub fn get_lines(&mut self, max_width: u16) -> &[Line<'static>] {
        // Check if width changed
        if self.last_width != max_width {
            self.dirty = true;
            self.last_width = max_width;
        }

        self.rehighlight(max_width);
        &self.cached_lines
    }

    /// Forces a re-render on the next `get_lines` call.
    pub fn invalidate(&mut self) {
        self.dirty = true;
    }

    /// Clears all content.
    pub fn clear(&mut self) {
        self.source.clear();
        self.cached_lines.clear();
        self.dirty = true;
    }

    /// Returns whether the content has changed since last render.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Returns the current source content.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Returns the language.
    pub fn language(&self) -> Option<&str> {
        self.language.as_deref()
    }

    /// Sets the language for highlighting.
    pub fn set_language(&mut self, language: Option<String>) {
        if self.language != language {
            self.language = language;
            self.dirty = true;
        }
    }

    /// Re-highlights the content if dirty.
    fn rehighlight(&mut self, max_width: u16) {
        if !self.dirty {
            return;
        }

        self.cached_lines = self
            .renderer
            .render(&self.source, self.language.as_deref(), max_width);
        self.dirty = false;
    }
}

// ============================================================
// Syntax Highlighting Helpers
// ============================================================

/// Converts a tui_syntax HighlightSpan to a ratatui Span.
fn highlight_span_to_ratatui(span: &HighlightSpan) -> Span<'static> {
    // Convert tui_text::Style to ratatui::style::Style
    let mut style = Style::default();

    // Extract foreground color
    if let Some(fg) = span.style.fg {
        style = style.fg(tui_color_to_ratatui(fg));
    }

    // Extract background color
    if let Some(bg) = span.style.bg {
        style = style.bg(tui_color_to_ratatui(bg));
    }

    // Extract modifiers from attributes
    if span.style.attributes.is_bold() {
        style = style.add_modifier(Modifier::BOLD);
    }
    if span.style.attributes.is_italic() {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if span.style.attributes.is_underline() {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    if span.style.attributes.is_strikethrough() {
        style = style.add_modifier(Modifier::CROSSED_OUT);
    }
    if span.style.attributes.is_dim() {
        style = style.add_modifier(Modifier::DIM);
    }

    Span::styled(span.text.clone(), style)
}

/// Converts a tui_text Color to a ratatui Color.
///
/// tui_text::Color uses f32 values (0.0-1.0), while ratatui uses u8 (0-255).
fn tui_color_to_ratatui(color: tui_text::Color) -> Color {
    let (r, g, b) = color.to_rgb_u8();
    Color::Rgb(r, g, b)
}

/// Highlights code using the highlighter, falling back to plain text if language is unknown.
fn highlight_code(
    highlighter: &Highlighter,
    code: &str,
    language: Option<&str>,
    fallback_style: Style,
) -> Vec<Vec<Span<'static>>> {
    // Split code into lines first
    let code_lines: Vec<&str> = code.lines().collect();

    // Handle empty code
    if code_lines.is_empty() {
        return vec![vec![Span::styled("", fallback_style)]];
    }

    // Try to highlight if we have a language
    if let Some(lang) = language {
        if highlighter.has_language(lang) {
            // Highlight the entire code
            match highlighter.highlight(code, lang) {
                Ok(highlighted) => {
                    // Convert HighlightSpan to ratatui Span, split by line
                    return split_highlighted_spans_by_line(highlighted.spans(), code);
                }
                Err(_) => {
                    // Fall through to plain text rendering
                }
            }
        }
    }

    // Fallback: render as plain text with the fallback style
    code_lines
        .into_iter()
        .map(|line| vec![Span::styled(line.to_string(), fallback_style)])
        .collect()
}

/// Splits highlighted spans into lines.
///
/// This handles the case where a span may cross multiple lines.
fn split_highlighted_spans_by_line(
    spans: &[HighlightSpan],
    source: &str,
) -> Vec<Vec<Span<'static>>> {
    let mut lines: Vec<Vec<Span<'static>>> = Vec::new();
    let mut current_line: Vec<Span<'static>> = Vec::new();

    for span in spans {
        let text = &span.text;

        // Split span text by newlines
        let mut first = true;
        for part in text.split('\n') {
            if !first {
                // Start a new line
                lines.push(std::mem::take(&mut current_line));
            }
            first = false;

            if !part.is_empty() {
                // Create a new span with the same style but different text
                let mut new_span = span.clone();
                new_span.text = part.to_string();
                current_line.push(highlight_span_to_ratatui(&new_span));
            }
        }
    }

    // Don't forget the last line
    if !current_line.is_empty() || lines.is_empty() {
        lines.push(current_line);
    }

    // Handle trailing newline in source
    if source.ends_with('\n') && !lines.is_empty() {
        lines.push(Vec::new());
    }

    lines
}

// ============================================================
// Header and Row Rendering
// ============================================================

/// Renders the header bar: language tag (left) + a right-aligned hint (`copy ^C` by default, or the
/// caller's `right_hint` such as the gallery's `theme: <name>`), on CODEBG.
///
/// Format: `rust                                          copy ^C`
fn render_header(
    language: Option<&str>,
    right_hint: Option<&str>,
    width: usize,
    bg_style: Style,
    lang_style: Style,
) -> Line<'static> {
    use unicode_width::UnicodeWidthStr;

    let lang_text = language.unwrap_or("");
    let hint_text = right_hint.unwrap_or(COPY_HINT);
    let lang_len = UnicodeWidthStr::width(lang_text);
    let hint_len = UnicodeWidthStr::width(hint_text);

    // Compute fill to right-justify the hint.
    let fill_len = width.saturating_sub(lang_len).saturating_sub(hint_len);

    let mut spans = Vec::with_capacity(3);
    if !lang_text.is_empty() {
        spans.push(Span::styled(lang_text.to_string(), lang_style));
    }
    if fill_len > 0 {
        spans.push(Span::styled(" ".repeat(fill_len), bg_style));
    }
    spans.push(Span::styled(hint_text.to_string(), lang_style));

    Line::from(spans)
}

/// Renders one code row: line-number gutter (right-justified) + syntax spans, padded to width.
///
/// Format: `  1 use serde_json::Value;`
fn render_code_row(
    code_spans: Vec<Span<'static>>,
    line_num: usize,
    number_width: usize,
    total_width: usize,
    bg_style: Style,
    lnum_style: Style,
) -> Line<'static> {
    let mut result = Vec::with_capacity(code_spans.len() + 3);

    // Line-number gutter (right-justified) with a single trailing space separating it from code.
    if number_width > 0 {
        let num_str = format!("{:>width$} ", line_num, width = number_width);
        result.push(Span::styled(num_str, lnum_style));
    }

    // Syntax spans, each forced onto the CODEBG so no token falls through to the terminal default.
    let code_width = spans_width(&code_spans);
    for mut span in code_spans {
        span.style = span.style.patch(bg_style);
        result.push(span);
    }

    // Padding to fill the panel width so the CODEBG rectangle is solid on every theme.
    let consumed = number_width + usize::from(number_width > 0) + code_width;
    let pad = total_width.saturating_sub(consumed);
    if pad > 0 {
        result.push(Span::styled(" ".repeat(pad), bg_style));
    }

    Line::from(result)
}

// ============================================================
// Line Number Gutter (for render_bare)
// ============================================================

/// Adds line numbers to highlighted lines (for `render_bare` when line numbers are enabled).
///
/// Format: `  1 content`
fn render_with_line_numbers(
    lines: Vec<Vec<Span<'static>>>,
    line_number_style: Style,
) -> Vec<Vec<Span<'static>>> {
    let line_count = lines.len();
    let number_width = line_count.to_string().len();

    lines
        .into_iter()
        .enumerate()
        .map(|(idx, mut spans)| {
            let line_num = idx + 1;
            let prefix = format!("{:>width$} ", line_num, width = number_width);

            let mut new_spans = Vec::with_capacity(spans.len() + 1);
            new_spans.push(Span::styled(prefix, line_number_style));
            new_spans.append(&mut spans);
            new_spans
        })
        .collect()
}

// ============================================================
// Utility Functions
// ============================================================

/// Calculates the display width of a slice of spans.
///
/// Uses unicode_width for accurate width calculation with CJK and emoji.
fn spans_width(spans: &[Span]) -> usize {
    use unicode_width::UnicodeWidthStr;
    spans.iter().map(|s| s.content.width()).sum()
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_highlighter() -> Arc<Highlighter> {
        Arc::new(Highlighter::new())
    }

    fn create_test_renderer() -> Arc<CodeBlockRenderer> {
        Arc::new(CodeBlockRenderer::new(create_test_highlighter()))
    }

    #[test]
    fn test_render_simple_code_block() {
        let renderer = create_test_renderer();
        let lines = renderer.render("hello world", None, 40);

        // Header bar + one content row.
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn every_emitted_cell_has_explicit_background() {
        // On a light (non-black) theme, every span in the rendered box (borders, language tag,
        // code, and padding) must carry an explicit background so the terminal default never
        // shows through as a black bar.
        let light_bg = Color::Rgb(245, 245, 245);
        let renderer = CodeBlockRenderer::new(create_test_highlighter())
            .with_background(light_bg)
            .with_border_color(Color::Rgb(200, 200, 200))
            .with_text_style(Style::default().fg(Color::Rgb(30, 30, 30)));

        let lines = renderer.render("fn main() {}\nlet x = 1;", Some("rust"), 48);
        for (row, line) in lines.iter().enumerate() {
            for span in &line.spans {
                assert_eq!(
                    span.style.bg,
                    Some(light_bg),
                    "row {row} span {:?} has no explicit background",
                    span.content
                );
            }
        }
    }

    #[test]
    fn test_render_with_language_tag() {
        let renderer = create_test_renderer();
        let lines = renderer.render("fn main() {}", Some("rust"), 40);

        // The header bar (first line) carries the language tag.
        let header_line = &lines[0];
        let header_content: String =
            header_line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(header_content.contains("rust"));
        assert!(header_content.contains("copy"));
    }

    #[test]
    fn test_render_multiline_code() {
        let renderer = create_test_renderer();
        let code = "line 1\nline 2\nline 3";
        let lines = renderer.render(code, None, 40);

        // Header bar + 3 content rows.
        assert_eq!(lines.len(), 4);
    }

    #[test]
    fn test_render_with_unknown_language() {
        let renderer = create_test_renderer();
        // "unknown_lang" is not registered, should fall back to plain text
        let lines = renderer.render("some code", Some("unknown_lang"), 40);

        // Header bar + one content row.
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn test_render_bare() {
        let renderer = create_test_renderer();
        let lines = renderer.render_bare("line 1\nline 2", None);

        // Should have 2 lines, no header/gutter chrome.
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn test_render_with_line_numbers() {
        let renderer = CodeBlockRenderer::new(create_test_highlighter()).with_line_numbers(true);
        let lines = renderer.render_bare("line 1\nline 2\nline 3", None);

        assert_eq!(lines.len(), 3);

        // Check that line numbers are present
        let first_line: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(first_line.contains("1"));
    }

    #[test]
    fn test_render_empty_code() {
        let renderer = create_test_renderer();
        let lines = renderer.render("", None, 40);

        // Header bar + one empty content row.
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn test_incremental_append() {
        let renderer = create_test_renderer();
        let mut block = IncrementalCodeBlock::new(renderer, None);

        block.append("hello");
        assert!(block.is_dirty());
        assert_eq!(block.source(), "hello");

        block.append(" world");
        assert_eq!(block.source(), "hello world");
    }

    #[test]
    fn test_incremental_set_source() {
        let renderer = create_test_renderer();
        let mut block = IncrementalCodeBlock::new(renderer, None);

        block.set_source("first");
        assert!(block.is_dirty());

        let _ = block.get_lines(40);
        assert!(!block.is_dirty());

        // Setting same source shouldn't mark dirty
        block.set_source("first");
        assert!(!block.is_dirty());

        // Setting different source should mark dirty
        block.set_source("second");
        assert!(block.is_dirty());
    }

    #[test]
    fn test_incremental_get_lines_caching() {
        let renderer = create_test_renderer();
        let mut block = IncrementalCodeBlock::new(renderer, None);

        block.set_source("test code");
        let lines1 = block.get_lines(40);
        let len1 = lines1.len();

        // Should not be dirty after get_lines
        assert!(!block.is_dirty());

        // Calling again should return cached
        let lines2 = block.get_lines(40);
        assert_eq!(lines2.len(), len1);
    }

    #[test]
    fn test_incremental_width_change() {
        let renderer = create_test_renderer();
        let mut block = IncrementalCodeBlock::new(renderer, None);

        block.set_source("test code");
        let _ = block.get_lines(40);
        assert!(!block.is_dirty());

        // Width change should trigger re-render
        let _ = block.get_lines(80);
        // After render, should not be dirty
        assert!(!block.is_dirty());
    }

    #[test]
    fn test_incremental_clear() {
        let renderer = create_test_renderer();
        let mut block = IncrementalCodeBlock::new(renderer, None);

        block.append("some content");
        block.clear();

        assert!(block.source().is_empty());
        assert!(block.is_dirty());
    }

    #[test]
    fn test_incremental_invalidate() {
        let renderer = create_test_renderer();
        let mut block = IncrementalCodeBlock::new(renderer, None);

        block.set_source("test");
        let _ = block.get_lines(40);
        assert!(!block.is_dirty());

        block.invalidate();
        assert!(block.is_dirty());
    }

    #[test]
    fn test_unicode_in_code() {
        let renderer = create_test_renderer();
        let code = "let emoji = '🦀';\nlet chinese = '中文';";
        let lines = renderer.render(code, None, 60);

        // Should render without panicking: header bar + 2 content rows.
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn test_various_widths() {
        let renderer = create_test_renderer();
        let code = "hello world";

        // Very narrow
        let lines = renderer.render(code, None, 10);
        assert!(lines.len() >= 2);

        // Normal
        let lines = renderer.render(code, None, 80);
        assert!(lines.len() >= 2);

        // Very wide
        let lines = renderer.render(code, None, 200);
        assert!(lines.len() >= 2);
    }

    #[test]
    fn header_bar_shows_lang_left_and_copy_hint_right() {
        let bg_style = Style::default().bg(CODEBG);
        let lang_style = Style::default().fg(FAINT);
        let line = render_header(Some("rust"), None, 40, bg_style, lang_style);

        let content: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(content.starts_with("rust"));
        assert!(content.trim_end().ends_with(COPY_HINT));
        // The whole header fills the panel width so CODEBG stays solid.
        use unicode_width::UnicodeWidthStr;
        assert_eq!(content.width(), 40);
    }

    #[test]
    fn header_bar_right_hint_overrides_copy_affordance() {
        // The Wave-7 gallery passes a `theme: <name>` right hint instead of `copy ^C`.
        let bg_style = Style::default().bg(CODEBG);
        let lang_style = Style::default().fg(FAINT);
        let line = render_header(Some("rust"), Some("theme: dark"), 40, bg_style, lang_style);

        let content: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(content.starts_with("rust"));
        assert!(content.trim_end().ends_with("theme: dark"));
        assert!(!content.contains(COPY_HINT), "copy affordance replaced by the theme tag");
        use unicode_width::UnicodeWidthStr;
        assert_eq!(content.width(), 40);
    }

    #[test]
    fn header_bar_without_language_still_shows_copy_hint() {
        let bg_style = Style::default().bg(CODEBG);
        let lang_style = Style::default().fg(FAINT);
        let line = render_header(None, None, 30, bg_style, lang_style);

        let content: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(content.trim_end().ends_with(COPY_HINT));
    }

    #[test]
    fn code_row_has_right_justified_line_number_gutter() {
        let bg_style = Style::default().bg(CODEBG);
        let lnum_style = Style::default().fg(FAINT);
        let line = render_code_row(
            vec![Span::raw("hello")],
            7,
            2,
            30,
            bg_style,
            lnum_style,
        );

        let content: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        // Line number 7 right-justified into a 2-wide gutter, then a space, then code.
        assert!(content.starts_with(" 7 hello"));
        // Row fills the panel width.
        use unicode_width::UnicodeWidthStr;
        assert_eq!(content.width(), 30);
    }

    #[test]
    fn test_builder_pattern() {
        let highlighter = create_test_highlighter();
        let renderer = CodeBlockRenderer::new(highlighter)
            .with_border_color(Color::Red)
            .with_background(Color::Black)
            .with_text_style(Style::default().fg(Color::White))
            .with_lang_tag_style(Style::default().fg(Color::Yellow))
            .with_line_numbers(true);

        assert!(renderer.show_line_numbers);
    }

    #[test]
    fn test_spans_width() {
        let spans = vec![Span::raw("hello"), Span::raw(" "), Span::raw("world")];
        assert_eq!(spans_width(&spans), 11);
    }

    #[test]
    fn test_line_numbers_format() {
        let lines = vec![
            vec![Span::raw("line 1")],
            vec![Span::raw("line 2")],
            vec![Span::raw("line 3")],
        ];
        let numbered = render_with_line_numbers(lines, Style::default());

        assert_eq!(numbered.len(), 3);

        // Check first line has line number 1 in the gutter (no box border).
        let first: String = numbered[0].iter().map(|s| s.content.as_ref()).collect();
        assert!(first.starts_with("1 "));
    }

    #[test]
    fn test_incremental_language_change() {
        let renderer = create_test_renderer();
        let mut block = IncrementalCodeBlock::new(renderer, Some("rust".to_string()));

        let _ = block.get_lines(40);
        assert!(!block.is_dirty());

        block.set_language(Some("python".to_string()));
        assert!(block.is_dirty());
        assert_eq!(block.language(), Some("python"));
    }
}
