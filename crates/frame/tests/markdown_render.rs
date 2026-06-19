//! Black-box behavior tests for the markdown renderer, mapped to the TUI markdown-render
//! validation assertions (VAL-TUI-011..015, VAL-TUI-094). Clean-room original (Apache-2.0).
//!
//! Each test asserts the *observable* rendered output: the markdown markers are consumed and the
//! element is rendered as a distinct structured element rather than raw markup.

use frame::markdown::render_markdown;
use ratatui::style::Modifier;
use ratatui::text::Line;

const W: u16 = 80;

/// Flatten rendered lines into a single string (one rendered line per text line).
fn flatten(lines: &[Line<'static>]) -> String {
    lines
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Find the first span whose (trimmed) content equals `needle`, returning its style modifier/bg.
fn span_with<'a>(
    lines: &'a [Line<'static>],
    needle: &str,
) -> Option<&'a ratatui::text::Span<'static>> {
    lines
        .iter()
        .flat_map(|l| l.spans.iter())
        .find(|s| s.content.trim() == needle)
}

// VAL-TUI-011: headings render distinctly, markers consumed.
#[test]
fn heading_is_styled_distinctly_and_consumes_markers() {
    let lines = render_markdown("## Overview\n\nFollowing prose here.", W);
    let text = flatten(&lines);

    assert!(text.contains("Overview"), "heading text present:\n{text}");
    assert!(
        text.contains("Following prose here."),
        "body present:\n{text}"
    );
    assert!(!text.contains('#'), "heading markers consumed:\n{text}");

    let heading = span_with(&lines, "Overview").expect("heading span");
    assert!(
        heading.style.add_modifier.contains(Modifier::BOLD),
        "heading is styled (bold), distinct from prose"
    );

    // The following prose should NOT carry the heading's bold emphasis.
    let prose = lines
        .iter()
        .flat_map(|l| l.spans.iter())
        .find(|s| s.content.contains("Following"))
        .expect("prose span");
    assert!(
        !prose.style.add_modifier.contains(Modifier::BOLD),
        "prose is rendered distinctly from the heading"
    );
}

// VAL-TUI-012: bold and italic render with emphasis, asterisks consumed.
#[test]
fn bold_and_italic_emphasis_consume_markers() {
    let lines = render_markdown("Text with **bold** and *italic* words.", W);
    let text = flatten(&lines);

    assert!(
        text.contains("bold") && text.contains("italic"),
        "words present:\n{text}"
    );
    assert!(!text.contains('*'), "asterisk markers consumed:\n{text}");

    let bold = span_with(&lines, "bold").expect("bold span");
    assert!(
        bold.style.add_modifier.contains(Modifier::BOLD),
        "bold word carries BOLD emphasis"
    );
    let italic = span_with(&lines, "italic").expect("italic span");
    assert!(
        italic.style.add_modifier.contains(Modifier::ITALIC),
        "italic word carries ITALIC emphasis"
    );
}

// VAL-TUI-013: bulleted and numbered lists render as aligned items with glyphs/ordinals.
#[test]
fn bulleted_and_numbered_lists_render_as_items() {
    let bullets = render_markdown("- alpha\n- beta\n- gamma", W);
    let btext = flatten(&bullets);
    assert_eq!(
        btext.matches('•').count(),
        3,
        "three bullet glyphs:\n{btext}"
    );
    assert!(btext.contains("alpha") && btext.contains("beta") && btext.contains("gamma"));

    let numbered = render_markdown("1. one\n2. two\n3. three", W);
    let ntext = flatten(&numbered);
    assert!(
        ntext.contains("1.") && ntext.contains("2.") && ntext.contains("3."),
        "ordinals:\n{ntext}"
    );
    // Items are on their own (vertically aligned) lines, not a run-on paragraph.
    let item_lines = numbered
        .iter()
        .filter(|l| {
            let t: String = l.spans.iter().map(|s| s.content.as_ref()).collect();
            t.contains("one") || t.contains("two") || t.contains("three")
        })
        .count();
    assert_eq!(
        item_lines, 3,
        "each numbered item on its own line:\n{ntext}"
    );
}

// VAL-TUI-014: inline code span renders distinctly, backticks consumed.
#[test]
fn inline_code_is_distinct_and_consumes_backticks() {
    let lines = render_markdown("Call the `compute` function now.", W);
    let text = flatten(&lines);

    assert!(text.contains("compute"), "code text present:\n{text}");
    assert!(!text.contains('`'), "backticks consumed:\n{text}");

    let code = span_with(&lines, "compute").expect("inline code span");
    assert!(
        code.style.bg.is_some() || code.style.fg.is_some(),
        "inline code is set apart with distinct styling"
    );
}

// VAL-TUI-015: fenced code block renders in a distinct container, preserving formatting, fences consumed.
#[test]
fn fenced_code_block_renders_in_distinct_container() {
    let md = "```rust\nfn main() {\n    let x = 1;\n}\n```";
    let lines = render_markdown(md, W);
    let text = flatten(&lines);

    assert!(!text.contains("```"), "fence markers consumed:\n{text}");
    // Distinct container: box-drawing borders.
    assert!(
        text.contains('┌') && text.contains('└') && text.contains('│'),
        "bordered container:\n{text}"
    );
    // Content preserved including indentation.
    assert!(text.contains("fn main()"), "code content present:\n{text}");
    assert!(
        lines.iter().any(|l| {
            let t: String = l.spans.iter().map(|s| s.content.as_ref()).collect();
            t.contains("    let x = 1;")
        }),
        "indentation preserved (4 leading spaces):\n{text}"
    );
    // Multi-line: top border + 3 code lines + bottom border = at least 5 lines.
    assert!(lines.len() >= 5, "multi-line block:\n{text}");
}

// VAL-TUI-094: tables and blockquotes render as structured elements, raw markup consumed.
#[test]
fn tables_and_blockquotes_render_as_structured_elements() {
    let table = render_markdown(
        "| Name | Age |\n|------|-----|\n| Ann | 30 |\n| Bob | 25 |",
        W,
    );
    let ttext = flatten(&table);
    assert!(
        ttext.contains("Name") && ttext.contains("Age"),
        "headers present:\n{ttext}"
    );
    assert!(
        ttext.contains("Ann") && ttext.contains("Bob"),
        "cells present:\n{ttext}"
    );
    // Structured: column separators between cells and a header rule with column junctions.
    assert!(ttext.contains('|'), "column separators present:\n{ttext}");
    assert!(
        ttext.contains('+'),
        "header rule with column junctions present:\n{ttext}"
    );
    // The raw single-line markdown row is reformatted (not echoed as plain prose).
    assert!(
        !ttext.contains("| Name | Age |"),
        "raw table markup reformatted:\n{ttext}"
    );

    let quote = render_markdown("> a wise remark", W);
    let qtext = flatten(&quote);
    assert!(
        qtext.contains("a wise remark"),
        "quote text present:\n{qtext}"
    );
    assert!(
        qtext.contains('│'),
        "blockquote bar styling present:\n{qtext}"
    );
    assert!(
        !qtext.contains('>'),
        "raw blockquote marker consumed:\n{qtext}"
    );
}
