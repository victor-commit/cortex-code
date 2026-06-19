//! Local functional smoke test (NOT ported upstream code; no provenance header).
//! Exercises the full highlight pipeline against a real tree-sitter Rust grammar
//! (dev-dependency only) and asserts non-empty, capture-tagged spans, since the
//! ported crate is grammar-agnostic at runtime (grammars are registered by callers).

use tui_syntax::{Highlighter, LanguageConfig};

fn rust_highlighter() -> Highlighter {
    let highlighter = Highlighter::new();
    highlighter.register_language(
        "rust",
        LanguageConfig::new(
            tree_sitter_rust::LANGUAGE.into(),
            tree_sitter_rust::HIGHLIGHTS_QUERY,
        ),
    );
    highlighter
}

#[test]
fn highlights_rust_snippet_into_nonempty_spans() {
    let highlighter = rust_highlighter();
    let source = "fn main() {\n    let x = 42;\n}\n";

    let raw = highlighter
        .highlight_raw(source, "rust")
        .expect("highlight_raw ok");
    assert!(
        !raw.is_empty(),
        "real tree-sitter grammar+query must produce capture highlights"
    );
    assert!(
        raw.iter().all(|h| !h.capture.is_empty()),
        "every raw highlight carries a capture name"
    );

    let highlighted = highlighter.highlight(source, "rust").expect("highlight ok");
    let spans = highlighted.spans();
    assert!(
        !spans.is_empty(),
        "expected non-empty spans for rust snippet"
    );
    let reconstructed: String = spans.iter().map(|s| s.text.as_str()).collect();
    assert_eq!(reconstructed, source, "spans must cover the full source");
}

#[test]
fn full_pipeline_emits_capture_tagged_spans() {
    let highlighter = rust_highlighter();
    // Leading newline keeps byte 0 unhighlighted: upstream `convert_to_spans`
    // collapses to a single plain span when the first highlight starts at byte 0
    // (faithful upstream behavior).
    let source = "\nfn main() {\n    let x = 42;\n}\n";

    let highlighted = highlighter.highlight(source, "rust").expect("highlight ok");
    let spans = highlighted.spans();
    assert!(
        spans.iter().any(|s| s.capture.is_some()),
        "full pipeline must emit capture-tagged spans for real syntax"
    );
}
