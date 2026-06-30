// SPDX-License-Identifier: MIT
// Provenance: ported source (direct-port-allowed), adapted to this workspace
// Source-Repo: CortexLM/cortex-code
// Source-Commit: 7954d0218faf6f26aa84df0f6d8b09ea9f2e5ac9
// Source-Path: cortex-tui-framework/crates/cortex-tui-syntax/src/highlighter.rs
// Target-Path: crates/tui-syntax/src/highlighter.rs
// Port-Mode: adapt
// Adaptation: crate renamed cortex-tui-syntax -> tui-syntax (cortex_tui_text -> tui_text imports).
//   Correctness fix in `Highlighter::convert_to_spans`: the upstream boundary loop emitted the
//   active style for the PRECEDING interval and collapsed the whole source to one plain span when
//   the first token started at byte 0 (the common ```rust / ```bash case). Replaced with a
//   front-to-back boundary sweep that styles each minimal interval by its most-specific covering
//   capture. Public API and behavior are otherwise unchanged.
// Upstream-License: MIT
// See: NOTICE (Direct-Port Provenance Ledger)

//! Syntax highlighter using tree-sitter.
//!
//! Provides high-performance syntax highlighting with support for incremental updates
//! and streaming content.

use crate::languages::{LanguageRegistry, language_from_extension, language_from_path};
use crate::span::{ByteRange, HighlightSpan, HighlightedText, RawHighlight};
use crate::theme::Theme;
use ahash::AHashMap;
use parking_lot::RwLock;
use std::path::Path;
use std::sync::Arc;
use streaming_iterator::StreamingIterator;
use thiserror::Error;
use tree_sitter::{Language, Parser, Query, QueryCursor, Tree};
use tui_text::Style;

/// Errors that can occur during highlighting.
#[derive(Debug, Error)]
pub enum HighlightError {
    /// The language is not registered or supported.
    #[error("language not found: {0}")]
    LanguageNotFound(String),

    /// Failed to parse the source code.
    #[error("parse error")]
    ParseError,

    /// Failed to compile the highlight query.
    #[error("query error: {0}")]
    QueryError(#[from] tree_sitter::QueryError),

    /// The language grammar failed to load.
    #[error("failed to set language: {0}")]
    LanguageError(#[from] tree_sitter::LanguageError),

    /// Invalid byte range.
    #[error("invalid range: {0}..{1}")]
    InvalidRange(usize, usize),
}

/// Result type for highlighting operations.
pub type Result<T> = std::result::Result<T, HighlightError>;

/// Configuration for a language's highlighting.
#[derive(Debug, Clone)]
pub struct LanguageConfig {
    /// The tree-sitter language.
    pub language: Language,
    /// The highlight query.
    pub highlight_query: String,
    /// Optional injection query for embedded languages.
    pub injection_query: Option<String>,
}

impl LanguageConfig {
    /// Creates a new language configuration.
    pub fn new(language: Language, highlight_query: impl Into<String>) -> Self {
        Self {
            language,
            highlight_query: highlight_query.into(),
            injection_query: None,
        }
    }

    /// Sets the injection query.
    pub fn with_injection_query(mut self, query: impl Into<String>) -> Self {
        self.injection_query = Some(query.into());
        self
    }
}

/// Cached parser and query for a language.
struct CachedLanguage {
    parser: Parser,
    query: Query,
    #[allow(dead_code)]
    injection_query: Option<Query>,
}

impl CachedLanguage {
    fn new(config: &LanguageConfig) -> Result<Self> {
        let mut parser = Parser::new();
        parser.set_language(&config.language)?;

        let query = Query::new(&config.language, &config.highlight_query)?;
        let injection_query = config
            .injection_query
            .as_ref()
            .map(|q| Query::new(&config.language, q))
            .transpose()?;

        Ok(Self {
            parser,
            query,
            injection_query,
        })
    }
}

/// Main syntax highlighter.
///
/// Thread-safe highlighter that caches language configurations and provides
/// both one-shot and incremental highlighting.
#[derive(Debug)]
pub struct Highlighter {
    /// Theme for styling.
    theme: Arc<RwLock<Theme>>,
    /// Cached language configurations.
    configs: Arc<RwLock<AHashMap<String, LanguageConfig>>>,
    /// Language registry for name/extension lookup.
    registry: Arc<RwLock<LanguageRegistry>>,
}

impl Default for Highlighter {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for Highlighter {
    fn clone(&self) -> Self {
        Self {
            theme: Arc::clone(&self.theme),
            configs: Arc::clone(&self.configs),
            registry: Arc::clone(&self.registry),
        }
    }
}

impl Highlighter {
    /// Creates a new highlighter with the default VS Code dark theme.
    pub fn new() -> Self {
        Self {
            theme: Arc::new(RwLock::new(Theme::vscode_dark())),
            configs: Arc::new(RwLock::new(AHashMap::new())),
            registry: Arc::new(RwLock::new(LanguageRegistry::new())),
        }
    }

    /// Creates a highlighter with a custom theme.
    pub fn with_theme(theme: Theme) -> Self {
        Self {
            theme: Arc::new(RwLock::new(theme)),
            configs: Arc::new(RwLock::new(AHashMap::new())),
            registry: Arc::new(RwLock::new(LanguageRegistry::new())),
        }
    }

    /// Sets the theme.
    pub fn set_theme(&self, theme: Theme) {
        *self.theme.write() = theme;
    }

    /// Returns a highlighter that shares this one's compiled grammars/registry (cheap, no
    /// re-registration) but carries its own `theme`. Use this to render the same languages under a
    /// different (chrome-bound) syntax palette without mutating the shared default highlighter.
    pub fn themed_clone(&self, theme: Theme) -> Self {
        Self {
            theme: Arc::new(RwLock::new(theme)),
            configs: Arc::clone(&self.configs),
            registry: Arc::clone(&self.registry),
        }
    }

    /// Gets a clone of the current theme.
    pub fn theme(&self) -> Theme {
        self.theme.read().clone()
    }

    /// Returns a reference to the theme for reading.
    pub fn theme_ref(&self) -> impl std::ops::Deref<Target = Theme> + '_ {
        self.theme.read()
    }

    /// Registers a language configuration.
    pub fn register_language(&self, name: impl Into<String>, config: LanguageConfig) {
        self.configs.write().insert(name.into(), config);
    }

    /// Returns true if a language is registered.
    pub fn has_language(&self, name: &str) -> bool {
        self.configs.read().contains_key(name)
    }

    /// Detects language from file extension.
    pub fn detect_language(&self, extension: &str) -> Option<String> {
        language_from_extension(extension).map(String::from)
    }

    /// Detects language from file path.
    pub fn detect_language_from_path(&self, path: impl AsRef<Path>) -> Option<String> {
        language_from_path(path).map(String::from)
    }

    /// Highlights source code.
    ///
    /// Returns highlighted text with styled spans.
    pub fn highlight(&self, source: &str, language: &str) -> Result<HighlightedText> {
        let configs = self.configs.read();
        let config = configs
            .get(language)
            .ok_or_else(|| HighlightError::LanguageNotFound(language.to_string()))?;

        // Create parser and query
        let mut cached = CachedLanguage::new(config)?;

        // Parse
        let tree = cached
            .parser
            .parse(source, None)
            .ok_or(HighlightError::ParseError)?;

        // Query for highlights
        let raw_highlights = self.query_highlights(source, &tree, &cached.query);

        // Convert to styled spans
        let theme = self.theme.read();
        let spans = self.convert_to_spans(source, &raw_highlights, &theme);

        Ok(HighlightedText::from_parts(source, spans))
    }

    /// Highlights source code and returns raw highlight data.
    ///
    /// Useful when you need the raw ranges without text extraction.
    pub fn highlight_raw(&self, source: &str, language: &str) -> Result<Vec<RawHighlight>> {
        let configs = self.configs.read();
        let config = configs
            .get(language)
            .ok_or_else(|| HighlightError::LanguageNotFound(language.to_string()))?;

        let mut cached = CachedLanguage::new(config)?;
        let tree = cached
            .parser
            .parse(source, None)
            .ok_or(HighlightError::ParseError)?;

        Ok(self.query_highlights(source, &tree, &cached.query))
    }

    /// Queries the tree for highlight captures.
    fn query_highlights(&self, source: &str, tree: &Tree, query: &Query) -> Vec<RawHighlight> {
        let mut cursor = QueryCursor::new();
        let mut highlights = Vec::new();

        let capture_names = query.capture_names();

        let mut matches = cursor.matches(query, tree.root_node(), source.as_bytes());
        while let Some(match_) = matches.next() {
            for capture in match_.captures {
                let node = capture.node;
                let start = node.start_byte();
                let end = node.end_byte();

                // Skip empty or invalid ranges
                if start >= end || end > source.len() {
                    continue;
                }

                let capture_name = capture_names[capture.index as usize];
                highlights.push(RawHighlight::new(start, end, capture_name));
            }
        }

        // Sort by start position, then by specificity (longer capture names first)
        highlights.sort_by(|a, b| {
            a.start
                .cmp(&b.start)
                .then_with(|| b.capture.len().cmp(&a.capture.len()))
        });

        highlights
    }

    /// Converts raw highlights to styled spans.
    ///
    /// Handles overlapping ranges by using the most specific capture.
    fn convert_to_spans(
        &self,
        source: &str,
        highlights: &[RawHighlight],
        theme: &Theme,
    ) -> Vec<HighlightSpan> {
        if highlights.is_empty() {
            // Return entire source as plain text
            return if source.is_empty() {
                Vec::new()
            } else {
                vec![HighlightSpan::plain(source, 0..source.len())]
            };
        }

        // Boundary sweep over every distinct start/end position (plus 0 and len). Each adjacent
        // pair forms a minimal interval that is wholly inside or outside every highlight, so the
        // covering highlight with the longest (most specific) capture name styles it; uncovered
        // intervals are plain. This walks the source front-to-back including a token at byte 0,
        // and assigns each interval its own active style.
        let mut positions: Vec<usize> = Vec::with_capacity(highlights.len() * 2 + 2);
        positions.push(0);
        positions.push(source.len());
        for hl in highlights {
            if hl.start <= source.len() {
                positions.push(hl.start);
            }
            if hl.end <= source.len() {
                positions.push(hl.end);
            }
        }
        positions.sort_unstable();
        positions.dedup();

        let mut spans = Vec::with_capacity(positions.len());
        for window in positions.windows(2) {
            let start = window[0];
            let end = window[1];
            if start >= end || end > source.len() {
                continue;
            }

            let text = &source[start..end];
            let range = start..end;

            // The most specific (longest capture name) highlight fully covering this interval.
            let best = highlights
                .iter()
                .filter(|hl| hl.start <= start && hl.end >= end)
                .max_by_key(|hl| hl.capture.len());

            match best {
                Some(hl) => {
                    let style = theme.get(&hl.capture);
                    spans.push(HighlightSpan::with_capture(text, style, range, &hl.capture));
                }
                None => spans.push(HighlightSpan::plain(text, range)),
            }
        }

        spans
    }

    /// Returns the language registry.
    pub fn registry(&self) -> impl std::ops::Deref<Target = LanguageRegistry> + '_ {
        self.registry.read()
    }

    /// Returns mutable access to the language registry.
    pub fn registry_mut(&self) -> impl std::ops::DerefMut<Target = LanguageRegistry> + '_ {
        self.registry.write()
    }
}

/// Incremental highlighter for streaming content.
///
/// Maintains parse tree state for efficient incremental updates.
pub struct IncrementalHighlighter {
    /// The base highlighter.
    highlighter: Highlighter,
    /// Current source content.
    source: String,
    /// Current parse tree.
    tree: Option<Tree>,
    /// Cached parser.
    parser: Option<Parser>,
    /// Cached query.
    query: Option<Query>,
    /// Language name.
    language: String,
    /// Last highlight result (for streaming).
    last_highlights: Vec<RawHighlight>,
}

impl IncrementalHighlighter {
    /// Creates a new incremental highlighter.
    pub fn new(highlighter: Highlighter) -> Self {
        Self {
            highlighter,
            source: String::new(),
            tree: None,
            parser: None,
            query: None,
            language: String::new(),
            last_highlights: Vec::new(),
        }
    }

    /// Sets the language for highlighting.
    pub fn set_language(&mut self, language: &str) -> Result<()> {
        if self.language == language && self.parser.is_some() {
            return Ok(());
        }

        let configs = self.highlighter.configs.read();
        let config = configs
            .get(language)
            .ok_or_else(|| HighlightError::LanguageNotFound(language.to_string()))?;

        let mut parser = Parser::new();
        parser.set_language(&config.language)?;

        let query = Query::new(&config.language, &config.highlight_query)?;

        self.parser = Some(parser);
        self.query = Some(query);
        self.language = language.to_string();
        self.tree = None;
        self.last_highlights.clear();

        Ok(())
    }

    /// Sets the source content and parses it.
    pub fn set_source(&mut self, source: impl Into<String>) -> Result<()> {
        self.source = source.into();
        self.reparse()?;
        Ok(())
    }

    /// Appends content for streaming.
    pub fn append(&mut self, content: &str) -> Result<()> {
        let old_len = self.source.len();
        self.source.push_str(content);

        // For now, just reparse entirely
        // A more sophisticated implementation would use tree.edit()
        self.reparse()?;

        // In a full implementation, we would:
        // 1. Create an InputEdit for the appended content
        // 2. Call tree.edit(&edit)
        // 3. Reparse incrementally
        let _ = old_len;

        Ok(())
    }

    /// Reparses the current source.
    fn reparse(&mut self) -> Result<()> {
        let parser = self
            .parser
            .as_mut()
            .ok_or_else(|| HighlightError::LanguageNotFound(self.language.clone()))?;

        // Incremental parse if we have an existing tree
        self.tree = parser.parse(&self.source, self.tree.as_ref());

        if self.tree.is_none() {
            return Err(HighlightError::ParseError);
        }

        // Update highlights
        if let (Some(tree), Some(query)) = (&self.tree, &self.query) {
            self.last_highlights = self.query_highlights(tree, query);
        }

        Ok(())
    }

    /// Queries for highlights.
    fn query_highlights(&self, tree: &Tree, query: &Query) -> Vec<RawHighlight> {
        let mut cursor = QueryCursor::new();
        let mut highlights = Vec::new();

        let capture_names = query.capture_names();

        let mut matches = cursor.matches(query, tree.root_node(), self.source.as_bytes());
        while let Some(match_) = matches.next() {
            for capture in match_.captures {
                let node = capture.node;
                let start = node.start_byte();
                let end = node.end_byte();

                if start >= end || end > self.source.len() {
                    continue;
                }

                let capture_name = capture_names[capture.index as usize];
                highlights.push(RawHighlight::new(start, end, capture_name));
            }
        }

        highlights.sort_by_key(|h| (h.start, std::cmp::Reverse(h.capture.len())));
        highlights
    }

    /// Gets the current highlighted text.
    pub fn highlighted_text(&self) -> HighlightedText {
        let theme = self.highlighter.theme.read();
        let spans = self
            .highlighter
            .convert_to_spans(&self.source, &self.last_highlights, &theme);
        HighlightedText::from_parts(&self.source, spans)
    }

    /// Gets highlighted text using cached highlights.
    ///
    /// Useful during streaming when you want immediate feedback
    /// before the full reparse completes.
    pub fn highlighted_text_cached(&self) -> HighlightedText {
        let theme = self.highlighter.theme.read();
        let spans = self
            .highlighter
            .convert_to_spans(&self.source, &self.last_highlights, &theme);
        HighlightedText::from_parts(&self.source, spans)
    }

    /// Returns the current source.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Returns the last highlights.
    pub fn last_highlights(&self) -> &[RawHighlight] {
        &self.last_highlights
    }

    /// Clears all state.
    pub fn clear(&mut self) {
        self.source.clear();
        self.tree = None;
        self.last_highlights.clear();
    }
}

/// A lightweight highlighter for one-off highlighting without caching.
pub struct SimpleHighlighter<'a> {
    parser: Parser,
    query: &'a Query,
    theme: &'a Theme,
}

impl<'a> SimpleHighlighter<'a> {
    /// Creates a new simple highlighter.
    pub fn new(language: &Language, query: &'a Query, theme: &'a Theme) -> Result<Self> {
        let mut parser = Parser::new();
        parser.set_language(language)?;
        Ok(Self {
            parser,
            query,
            theme,
        })
    }

    /// Highlights source code.
    pub fn highlight(&mut self, source: &str) -> Result<HighlightedText> {
        let tree = self
            .parser
            .parse(source, None)
            .ok_or(HighlightError::ParseError)?;

        let mut cursor = QueryCursor::new();
        let mut highlights = Vec::new();

        let capture_names = self.query.capture_names();

        let mut matches = cursor.matches(self.query, tree.root_node(), source.as_bytes());
        while let Some(match_) = matches.next() {
            for capture in match_.captures {
                let node = capture.node;
                let start = node.start_byte();
                let end = node.end_byte();

                if start >= end || end > source.len() {
                    continue;
                }

                let capture_name = capture_names[capture.index as usize];
                highlights.push(RawHighlight::new(start, end, capture_name));
            }
        }

        highlights.sort_by_key(|h| (h.start, std::cmp::Reverse(h.capture.len())));

        let spans = self.convert_to_spans(source, &highlights);
        Ok(HighlightedText::from_parts(source, spans))
    }

    /// Converts raw highlights to spans.
    fn convert_to_spans(&self, source: &str, highlights: &[RawHighlight]) -> Vec<HighlightSpan> {
        if highlights.is_empty() {
            return if source.is_empty() {
                Vec::new()
            } else {
                vec![HighlightSpan::plain(source, 0..source.len())]
            };
        }

        let mut spans = Vec::new();
        let mut pos = 0;

        for hl in highlights {
            // Add unhighlighted text before this highlight
            if pos < hl.start {
                spans.push(HighlightSpan::plain(&source[pos..hl.start], pos..hl.start));
            }

            // Add highlighted span
            if hl.start < hl.end && hl.end <= source.len() {
                let text = &source[hl.start..hl.end];
                let style = self.theme.get(&hl.capture);
                spans.push(HighlightSpan::with_capture(
                    text,
                    style,
                    hl.start..hl.end,
                    &hl.capture,
                ));
                pos = hl.end;
            }
        }

        // Add remaining text
        if pos < source.len() {
            spans.push(HighlightSpan::plain(&source[pos..], pos..source.len()));
        }

        spans
    }
}

/// Applies a style to a byte range in source text.
///
/// Utility function for manual highlighting.
pub fn style_range(source: &str, range: ByteRange, style: Style) -> Option<HighlightSpan> {
    if range.start >= range.end || range.end > source.len() {
        return None;
    }
    Some(HighlightSpan::new(&source[range.clone()], style, range))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_highlighter_creation() {
        let highlighter = Highlighter::new();
        assert!(!highlighter.has_language("rust"));
    }

    #[test]
    fn test_language_detection() {
        let highlighter = Highlighter::new();
        assert_eq!(highlighter.detect_language("rs"), Some("rust".to_string()));
        assert_eq!(
            highlighter.detect_language("py"),
            Some("python".to_string())
        );
        assert_eq!(highlighter.detect_language("xyz"), None);
    }

    #[test]
    fn test_style_range() {
        let source = "hello world";
        let span = style_range(source, 0..5, Style::default()).unwrap();
        assert_eq!(span.text, "hello");
        assert_eq!(span.range, 0..5);
    }

    #[test]
    fn test_style_range_invalid() {
        let source = "hello";
        assert!(style_range(source, 5..3, Style::default()).is_none());
        assert!(style_range(source, 0..10, Style::default()).is_none());
    }
}
