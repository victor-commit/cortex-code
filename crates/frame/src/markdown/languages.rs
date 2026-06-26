// SPDX-License-Identifier: MIT
// Provenance: ported source (direct-port-allowed), adapted to this workspace
// Source-Repo: CortexLM/cortex-code
// Source-Commit: 7954d0218faf6f26aa84df0f6d8b09ea9f2e5ac9
// Source-Path: src/cortex-core/src/markdown/languages.rs
// Target-Path: crates/frame/src/markdown/languages.rs
// Port-Mode: adapt
// Adaptation: crate renamed cortex-frame -> frame; syntax crate renamed cortex-tui-syntax ->
//   tui-syntax. `register_common_languages` now registers the high-priority grammar set required
//   for common fenced code blocks (bash/rust/python/javascript/typescript/go/json/yaml/toml/
//   markdown/c/cpp/java); tests adjusted accordingly.
// Upstream-License: MIT
// See: NOTICE (Direct-Port Provenance Ledger)

//! Tree-sitter language registration for syntax highlighting.
//!
//! This module provides:
//! - Language registration for syntax highlighting
//! - Language name normalization (aliases)
//! - A global default highlighter singleton
//! - Helper functions for language detection

use once_cell::sync::Lazy;
use std::sync::Arc;
use tui_syntax::{Highlighter, LanguageConfig};

// ============================================================================
// Language Registration
// ============================================================================

/// Register commonly used languages for syntax highlighting.
///
/// This registers the high-priority grammar set used by markdown fenced code blocks in the TUI.
pub fn register_common_languages(highlighter: &Highlighter) {
    fn register_language(
        highlighter: &Highlighter,
        name: &str,
        language: tree_sitter::Language,
        highlights_query: &str,
    ) {
        highlighter.register_language(name, LanguageConfig::new(language, highlights_query));
    }

    register_language(
        highlighter,
        "bash",
        tree_sitter_bash::LANGUAGE.into(),
        tree_sitter_bash::HIGHLIGHT_QUERY,
    );
    register_language(
        highlighter,
        "rust",
        tree_sitter_rust::LANGUAGE.into(),
        tree_sitter_rust::HIGHLIGHTS_QUERY,
    );
    register_language(
        highlighter,
        "python",
        tree_sitter_python::LANGUAGE.into(),
        tree_sitter_python::HIGHLIGHTS_QUERY,
    );
    register_language(
        highlighter,
        "javascript",
        tree_sitter_javascript::LANGUAGE.into(),
        tree_sitter_javascript::HIGHLIGHT_QUERY,
    );
    register_language(
        highlighter,
        "typescript",
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        tree_sitter_typescript::HIGHLIGHTS_QUERY,
    );
    register_language(
        highlighter,
        "go",
        tree_sitter_go::LANGUAGE.into(),
        tree_sitter_go::HIGHLIGHTS_QUERY,
    );
    register_language(
        highlighter,
        "json",
        tree_sitter_json::LANGUAGE.into(),
        tree_sitter_json::HIGHLIGHTS_QUERY,
    );
    register_language(
        highlighter,
        "yaml",
        tree_sitter_yaml::LANGUAGE.into(),
        tree_sitter_yaml::HIGHLIGHTS_QUERY,
    );
    register_language(
        highlighter,
        "toml",
        tree_sitter_toml_ng::LANGUAGE.into(),
        tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
    );
    register_language(
        highlighter,
        "markdown",
        tree_sitter_md::LANGUAGE.into(),
        tree_sitter_md::HIGHLIGHT_QUERY_BLOCK,
    );
    register_language(
        highlighter,
        "c",
        tree_sitter_c::LANGUAGE.into(),
        tree_sitter_c::HIGHLIGHT_QUERY,
    );
    register_language(
        highlighter,
        "cpp",
        tree_sitter_cpp::LANGUAGE.into(),
        tree_sitter_cpp::HIGHLIGHT_QUERY,
    );
    register_language(
        highlighter,
        "java",
        tree_sitter_java::LANGUAGE.into(),
        tree_sitter_java::HIGHLIGHTS_QUERY,
    );
}

// ============================================================================
// Language Aliases
// ============================================================================

/// Normalize language name to canonical form.
///
/// Maps common aliases and code fence names to standard language identifiers.
/// Returns `None` for unrecognized languages.
///
/// # Examples
///
/// ```
/// use frame::markdown::languages::normalize_language_name;
///
/// assert_eq!(normalize_language_name("rs"), Some("rust"));
/// assert_eq!(normalize_language_name("python3"), Some("python"));
/// assert_eq!(normalize_language_name("sh"), Some("bash"));
/// assert_eq!(normalize_language_name("unknown"), None);
/// ```
pub fn normalize_language_name(lang: &str) -> Option<&'static str> {
    match lang.to_lowercase().as_str() {
        // Rust
        "rust" | "rs" => Some("rust"),

        // Python
        "python" | "py" | "python3" => Some("python"),

        // JavaScript
        "javascript" | "js" | "jsx" => Some("javascript"),

        // TypeScript
        "typescript" | "ts" | "tsx" => Some("typescript"),

        // Shell
        "bash" | "sh" | "shell" | "zsh" => Some("bash"),

        // Data formats
        "json" | "jsonc" => Some("json"),
        "yaml" | "yml" => Some("yaml"),
        "toml" => Some("toml"),
        "xml" => Some("xml"),

        // Web
        "html" | "htm" => Some("html"),
        "css" | "scss" | "less" => Some("css"),

        // Systems
        "c" => Some("c"),
        "cpp" | "c++" | "cxx" | "cc" => Some("cpp"),
        "go" | "golang" => Some("go"),

        // Other
        "sql" => Some("sql"),
        "markdown" | "md" => Some("markdown"),
        "diff" | "patch" => Some("diff"),
        "makefile" | "make" => Some("make"),
        "dockerfile" | "docker" => Some("dockerfile"),

        // Unknown
        _ => None,
    }
}

/// Extract the language token from a fenced-code info string.
///
/// Markdown info strings may include language aliases and trailing attributes, for example:
/// - `python`
/// - `python title="demo"`
/// - `language-javascript`
/// - `{.typescript .numberLines}`
///
/// This helper extracts and normalizes the first language-like token so fenced code blocks can
/// resolve the correct renderer in streaming and final markdown paths.
pub fn extract_fence_language(info: &str) -> Option<String> {
    let mut token = info.split_whitespace().next()?.trim();
    if token.is_empty() {
        return None;
    }

    // Handle markdown attribute forms like `{.python}` or `{.python`
    token = token.trim_matches(|c: char| c == '{' || c == '}');

    // Handle class-style forms like `.python`
    token = token.strip_prefix('.').unwrap_or(token);

    // Handle `language-foo` commonly emitted by markdown processors.
    if token
        .get(..9)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("language-"))
    {
        token = &token[9..];
    }

    // Ignore trailing punctuation from loose attribute notation.
    token = token.trim_matches(|c: char| c == ',' || c == ';');

    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

// ============================================================================
// Default Highlighter Singleton
// ============================================================================

/// Global default highlighter with common languages registered.
///
/// This singleton is lazily initialized and shared across the application.
/// Use [`get_default_highlighter`] to access it.
pub static DEFAULT_HIGHLIGHTER: Lazy<Arc<Highlighter>> = Lazy::new(|| {
    let highlighter = Highlighter::new();
    register_common_languages(&highlighter);
    Arc::new(highlighter)
});

/// Get the default highlighter.
///
/// Returns a clone of the `Arc` to the global default highlighter.
/// The highlighter is initialized with common languages on first access.
///
/// # Examples
///
/// ```
/// use frame::markdown::languages::get_default_highlighter;
///
/// let highlighter = get_default_highlighter();
/// // Use highlighter for syntax highlighting...
/// ```
pub fn get_default_highlighter() -> Arc<Highlighter> {
    Arc::clone(&DEFAULT_HIGHLIGHTER)
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Check if a language is available for highlighting.
///
/// Returns `true` if the language (or an alias of it) is registered
/// with the default highlighter.
///
/// # Examples
///
/// ```
/// use frame::markdown::languages::is_language_available;
///
/// assert!(is_language_available("bash"));
/// assert!(is_language_available("sh")); // Alias for bash
/// assert!(!is_language_available("nonexistent"));
/// ```
pub fn is_language_available(lang: &str) -> bool {
    if let Some(normalized) = normalize_language_name(lang) {
        get_default_highlighter().has_language(normalized)
    } else {
        // Try the raw language name in case it's registered directly
        get_default_highlighter().has_language(lang)
    }
}

/// Get display name for a language.
///
/// Returns a human-readable name for the language suitable for
/// display in UI elements like code block headers.
///
/// # Examples
///
/// ```
/// use frame::markdown::languages::get_language_display_name;
///
/// assert_eq!(get_language_display_name("rs"), "Rust");
/// assert_eq!(get_language_display_name("cpp"), "C++");
/// assert_eq!(get_language_display_name("unknown"), "unknown");
/// ```
pub fn get_language_display_name(lang: &str) -> &'static str {
    match normalize_language_name(lang) {
        Some("rust") => "Rust",
        Some("python") => "Python",
        Some("javascript") => "JavaScript",
        Some("typescript") => "TypeScript",
        Some("bash") => "Bash",
        Some("json") => "JSON",
        Some("yaml") => "YAML",
        Some("toml") => "TOML",
        Some("html") => "HTML",
        Some("css") => "CSS",
        Some("c") => "C",
        Some("cpp") => "C++",
        Some("go") => "Go",
        Some("sql") => "SQL",
        Some("markdown") => "Markdown",
        Some("diff") => "Diff",
        Some("make") => "Make",
        Some("dockerfile") => "Dockerfile",
        Some("xml") => "XML",
        Some(other) => other,
        None => {
            // Return the original string for unknown languages.
            // We leak it to get a 'static lifetime - acceptable since this is typically
            // called with a small set of strings; prefer matching on known languages.
            Box::leak(lang.to_string().into_boxed_str())
        }
    }
}

/// List all supported language names.
///
/// Returns a static slice of canonical language names that can be
/// used with the highlighting system. Note that aliases (like "rs" for "rust")
/// are normalized via [`normalize_language_name`] and are not included here.
///
/// # Examples
///
/// ```
/// use frame::markdown::languages::supported_languages;
///
/// let langs = supported_languages();
/// assert!(langs.contains(&"bash"));
/// assert!(langs.contains(&"rust"));
/// ```
pub fn supported_languages() -> &'static [&'static str] {
    &[
        "bash",
        "c",
        "cpp",
        "css",
        "diff",
        "dockerfile",
        "go",
        "html",
        "javascript",
        "json",
        "make",
        "markdown",
        "python",
        "rust",
        "sql",
        "toml",
        "typescript",
        "xml",
        "yaml",
    ]
}

/// Get all known aliases for a canonical language name.
///
/// Useful for documentation or auto-completion features.
///
/// # Examples
///
/// ```
/// use frame::markdown::languages::get_language_aliases;
///
/// let rust_aliases = get_language_aliases("rust");
/// assert!(rust_aliases.contains(&"rs"));
/// ```
pub fn get_language_aliases(canonical: &str) -> &'static [&'static str] {
    match canonical {
        "rust" => &["rust", "rs"],
        "python" => &["python", "py", "python3"],
        "javascript" => &["javascript", "js", "jsx"],
        "typescript" => &["typescript", "ts", "tsx"],
        "bash" => &["bash", "sh", "shell", "zsh"],
        "json" => &["json", "jsonc"],
        "yaml" => &["yaml", "yml"],
        "toml" => &["toml"],
        "xml" => &["xml"],
        "html" => &["html", "htm"],
        "css" => &["css", "scss", "less"],
        "c" => &["c"],
        "cpp" => &["cpp", "c++", "cxx", "cc"],
        "go" => &["go", "golang"],
        "sql" => &["sql"],
        "markdown" => &["markdown", "md"],
        "diff" => &["diff", "patch"],
        "make" => &["makefile", "make"],
        "dockerfile" => &["dockerfile", "docker"],
        _ => &[],
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_language_name_rust() {
        assert_eq!(normalize_language_name("rust"), Some("rust"));
        assert_eq!(normalize_language_name("rs"), Some("rust"));
        assert_eq!(normalize_language_name("RUST"), Some("rust"));
        assert_eq!(normalize_language_name("Rs"), Some("rust"));
    }

    #[test]
    fn test_normalize_language_name_python() {
        assert_eq!(normalize_language_name("python"), Some("python"));
        assert_eq!(normalize_language_name("py"), Some("python"));
        assert_eq!(normalize_language_name("python3"), Some("python"));
        assert_eq!(normalize_language_name("PYTHON"), Some("python"));
    }

    #[test]
    fn test_normalize_language_name_javascript() {
        assert_eq!(normalize_language_name("javascript"), Some("javascript"));
        assert_eq!(normalize_language_name("js"), Some("javascript"));
        assert_eq!(normalize_language_name("jsx"), Some("javascript"));
    }

    #[test]
    fn test_normalize_language_name_typescript() {
        assert_eq!(normalize_language_name("typescript"), Some("typescript"));
        assert_eq!(normalize_language_name("ts"), Some("typescript"));
        assert_eq!(normalize_language_name("tsx"), Some("typescript"));
    }

    #[test]
    fn test_normalize_language_name_shell() {
        assert_eq!(normalize_language_name("bash"), Some("bash"));
        assert_eq!(normalize_language_name("sh"), Some("bash"));
        assert_eq!(normalize_language_name("shell"), Some("bash"));
        assert_eq!(normalize_language_name("zsh"), Some("bash"));
    }

    #[test]
    fn test_normalize_language_name_data_formats() {
        assert_eq!(normalize_language_name("json"), Some("json"));
        assert_eq!(normalize_language_name("jsonc"), Some("json"));
        assert_eq!(normalize_language_name("yaml"), Some("yaml"));
        assert_eq!(normalize_language_name("yml"), Some("yaml"));
        assert_eq!(normalize_language_name("toml"), Some("toml"));
        assert_eq!(normalize_language_name("xml"), Some("xml"));
    }

    #[test]
    fn test_normalize_language_name_web() {
        assert_eq!(normalize_language_name("html"), Some("html"));
        assert_eq!(normalize_language_name("htm"), Some("html"));
        assert_eq!(normalize_language_name("css"), Some("css"));
        assert_eq!(normalize_language_name("scss"), Some("css"));
        assert_eq!(normalize_language_name("less"), Some("css"));
    }

    #[test]
    fn test_normalize_language_name_systems() {
        assert_eq!(normalize_language_name("c"), Some("c"));
        assert_eq!(normalize_language_name("cpp"), Some("cpp"));
        assert_eq!(normalize_language_name("c++"), Some("cpp"));
        assert_eq!(normalize_language_name("cxx"), Some("cpp"));
        assert_eq!(normalize_language_name("cc"), Some("cpp"));
        assert_eq!(normalize_language_name("go"), Some("go"));
        assert_eq!(normalize_language_name("golang"), Some("go"));
    }

    #[test]
    fn test_normalize_language_name_other() {
        assert_eq!(normalize_language_name("sql"), Some("sql"));
        assert_eq!(normalize_language_name("markdown"), Some("markdown"));
        assert_eq!(normalize_language_name("md"), Some("markdown"));
        assert_eq!(normalize_language_name("diff"), Some("diff"));
        assert_eq!(normalize_language_name("patch"), Some("diff"));
        assert_eq!(normalize_language_name("makefile"), Some("make"));
        assert_eq!(normalize_language_name("make"), Some("make"));
        assert_eq!(normalize_language_name("dockerfile"), Some("dockerfile"));
        assert_eq!(normalize_language_name("docker"), Some("dockerfile"));
    }

    #[test]
    fn test_normalize_language_name_unknown() {
        assert_eq!(normalize_language_name("unknown"), None);
        assert_eq!(normalize_language_name("random"), None);
        assert_eq!(normalize_language_name(""), None);
        assert_eq!(normalize_language_name("xyz123"), None);
    }

    #[test]
    fn test_extract_fence_language() {
        assert_eq!(extract_fence_language("python"), Some("python".to_string()));
        assert_eq!(
            extract_fence_language("python title=\"adder\""),
            Some("python".to_string())
        );
        assert_eq!(
            extract_fence_language("language-javascript"),
            Some("javascript".to_string())
        );
        assert_eq!(
            extract_fence_language("Language-TypeScript"),
            Some("TypeScript".to_string())
        );
        assert_eq!(
            extract_fence_language("{.typescript .numberLines}"),
            Some("typescript".to_string())
        );
        assert_eq!(extract_fence_language(".go"), Some("go".to_string()));
        assert_eq!(extract_fence_language("{.}"), None);
        assert_eq!(extract_fence_language(""), None);
        assert_eq!(extract_fence_language("   "), None);
    }

    #[test]
    fn test_is_language_available_bash() {
        // Bash is registered
        assert!(is_language_available("bash"));
        assert!(is_language_available("sh"));
        assert!(is_language_available("shell"));
        assert!(is_language_available("zsh"));
    }

    #[test]
    fn test_is_language_available_rust() {
        // Rust is registered (alongside bash)
        assert!(is_language_available("rust"));
        assert!(is_language_available("rs"));
    }

    #[test]
    fn test_is_language_available_unknown() {
        // Random/unknown languages should not be available
        assert!(!is_language_available("nonexistent"));
        assert!(!is_language_available("random_lang"));
    }

    #[test]
    fn test_is_language_available_aliases() {
        assert!(is_language_available("python3"));
        assert!(is_language_available("js"));
        assert!(is_language_available("ts"));
        assert!(is_language_available("golang"));
        assert!(is_language_available("yml"));
        assert!(is_language_available("md"));
        assert!(is_language_available("c++"));
    }

    #[test]
    fn test_is_language_available_high_priority_languages() {
        // VAL-GAP-TUI-001 requires these languages to be highlighted via tree-sitter.
        for language in [
            "rust",
            "bash",
            "python",
            "javascript",
            "typescript",
            "go",
            "json",
            "yaml",
            "toml",
            "markdown",
            "c",
            "cpp",
            "java",
        ] {
            assert!(
                is_language_available(language),
                "expected language '{language}' to be registered"
            );
        }
    }

    #[test]
    fn test_get_language_display_name() {
        assert_eq!(get_language_display_name("rust"), "Rust");
        assert_eq!(get_language_display_name("rs"), "Rust");
        assert_eq!(get_language_display_name("python"), "Python");
        assert_eq!(get_language_display_name("py"), "Python");
        assert_eq!(get_language_display_name("javascript"), "JavaScript");
        assert_eq!(get_language_display_name("js"), "JavaScript");
        assert_eq!(get_language_display_name("typescript"), "TypeScript");
        assert_eq!(get_language_display_name("ts"), "TypeScript");
        assert_eq!(get_language_display_name("bash"), "Bash");
        assert_eq!(get_language_display_name("sh"), "Bash");
        assert_eq!(get_language_display_name("cpp"), "C++");
        assert_eq!(get_language_display_name("c++"), "C++");
        assert_eq!(get_language_display_name("go"), "Go");
        assert_eq!(get_language_display_name("golang"), "Go");
    }

    #[test]
    fn test_get_language_display_name_data_formats() {
        assert_eq!(get_language_display_name("json"), "JSON");
        assert_eq!(get_language_display_name("yaml"), "YAML");
        assert_eq!(get_language_display_name("yml"), "YAML");
        assert_eq!(get_language_display_name("toml"), "TOML");
        assert_eq!(get_language_display_name("xml"), "XML");
    }

    #[test]
    fn test_get_language_display_name_web() {
        assert_eq!(get_language_display_name("html"), "HTML");
        assert_eq!(get_language_display_name("htm"), "HTML");
        assert_eq!(get_language_display_name("css"), "CSS");
    }

    #[test]
    fn test_supported_languages() {
        let langs = supported_languages();

        // Check it returns a non-empty list
        assert!(!langs.is_empty());

        // Check it contains expected languages
        assert!(langs.contains(&"bash"));
        assert!(langs.contains(&"rust"));
        assert!(langs.contains(&"python"));
        assert!(langs.contains(&"javascript"));
        assert!(langs.contains(&"typescript"));
        assert!(langs.contains(&"json"));
        assert!(langs.contains(&"yaml"));
        assert!(langs.contains(&"toml"));
        assert!(langs.contains(&"html"));
        assert!(langs.contains(&"css"));
        assert!(langs.contains(&"c"));
        assert!(langs.contains(&"cpp"));
        assert!(langs.contains(&"go"));
        assert!(langs.contains(&"sql"));
        assert!(langs.contains(&"markdown"));
        assert!(langs.contains(&"diff"));
        assert!(langs.contains(&"dockerfile"));
        assert!(langs.contains(&"make"));
        assert!(langs.contains(&"xml"));
    }

    #[test]
    fn test_supported_languages_is_sorted() {
        let langs = supported_languages();
        let mut sorted = langs.to_vec();
        sorted.sort();
        assert_eq!(langs, sorted.as_slice());
    }

    #[test]
    fn test_default_highlighter_initialization() {
        let highlighter = get_default_highlighter();

        for language in [
            "rust",
            "bash",
            "python",
            "javascript",
            "typescript",
            "go",
            "json",
            "yaml",
            "toml",
            "markdown",
            "c",
            "cpp",
            "java",
        ] {
            assert!(
                highlighter.has_language(language),
                "expected language '{language}' to be registered"
            );
        }

        // Multiple calls should return the same instance
        let highlighter2 = get_default_highlighter();
        assert!(Arc::ptr_eq(&highlighter, &highlighter2));
    }

    #[test]
    fn test_default_highlighter_can_highlight_bash() {
        let highlighter = get_default_highlighter();

        // Should be able to highlight bash code
        let result = highlighter.highlight("echo 'hello world'", "bash");
        assert!(result.is_ok());

        let highlighted = result.unwrap();
        assert!(!highlighted.is_empty());
    }

    #[test]
    fn test_default_highlighter_can_highlight_rust() {
        let highlighter = get_default_highlighter();

        let result = highlighter.highlight("fn main() { let x = 42; }", "rust");
        assert!(result.is_ok());

        let highlighted = result.unwrap();
        assert!(!highlighted.is_empty());
    }

    #[test]
    fn test_default_highlighter_can_highlight_gap_languages() {
        let highlighter = get_default_highlighter();

        let fixtures = [
            ("python", "def add(a, b):\n    return a + b"),
            ("javascript", "function add(a, b) { return a + b; }"),
            ("typescript", "type User = { id: number; name: string };"),
            ("go", "package main\nfunc main() { println(\"hi\") }"),
            ("json", "{ \"name\": \"cortex\", \"version\": 1 }"),
            ("yaml", "name: cortex\nversion: 1"),
            ("toml", "name = \"cortex\"\nversion = 1"),
            ("markdown", "# Header\n\n`inline`"),
            ("c", "int main(void) { return 0; }"),
            ("cpp", "class Widget { public: int value; };"),
            (
                "java",
                "class Main { public static void main(String[] args) { String s = \"hi\"; } }",
            ),
        ];

        for (language, sample) in fixtures {
            let highlighted = highlighter
                .highlight(sample, language)
                .unwrap_or_else(|error| panic!("failed to highlight '{language}': {error}"));

            assert!(
                highlighted
                    .spans()
                    .iter()
                    .any(|span| !span.style.is_empty()),
                "expected styled spans for language '{language}'"
            );
        }
    }

    #[test]
    fn test_get_language_aliases() {
        assert!(get_language_aliases("rust").contains(&"rs"));
        assert!(get_language_aliases("python").contains(&"py"));
        assert!(get_language_aliases("python").contains(&"python3"));
        assert!(get_language_aliases("bash").contains(&"sh"));
        assert!(get_language_aliases("bash").contains(&"shell"));
        assert!(get_language_aliases("cpp").contains(&"c++"));
        assert!(get_language_aliases("go").contains(&"golang"));
        assert!(get_language_aliases("markdown").contains(&"md"));
        assert!(get_language_aliases("yaml").contains(&"yml"));
    }

    #[test]
    fn test_get_language_aliases_unknown() {
        assert!(get_language_aliases("unknown").is_empty());
        assert!(get_language_aliases("random").is_empty());
    }

    #[test]
    fn test_register_common_languages() {
        // Create a fresh highlighter and register languages
        let highlighter = Highlighter::new();
        assert!(!highlighter.has_language("bash"));
        assert!(!highlighter.has_language("rust"));
        assert!(!highlighter.has_language("python"));
        assert!(!highlighter.has_language("javascript"));
        assert!(!highlighter.has_language("typescript"));
        assert!(!highlighter.has_language("go"));
        assert!(!highlighter.has_language("json"));
        assert!(!highlighter.has_language("yaml"));
        assert!(!highlighter.has_language("toml"));
        assert!(!highlighter.has_language("markdown"));
        assert!(!highlighter.has_language("c"));
        assert!(!highlighter.has_language("cpp"));
        assert!(!highlighter.has_language("java"));

        register_common_languages(&highlighter);
        assert!(highlighter.has_language("bash"));
        assert!(highlighter.has_language("rust"));
        assert!(highlighter.has_language("python"));
        assert!(highlighter.has_language("javascript"));
        assert!(highlighter.has_language("typescript"));
        assert!(highlighter.has_language("go"));
        assert!(highlighter.has_language("json"));
        assert!(highlighter.has_language("yaml"));
        assert!(highlighter.has_language("toml"));
        assert!(highlighter.has_language("markdown"));
        assert!(highlighter.has_language("c"));
        assert!(highlighter.has_language("cpp"));
        assert!(highlighter.has_language("java"));
    }
}
