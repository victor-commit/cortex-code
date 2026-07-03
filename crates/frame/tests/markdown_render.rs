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
        qtext.contains('▎'),
        "blockquote bar styling present:\n{qtext}"
    );
    assert!(
        !qtext.contains('>'),
        "raw blockquote marker consumed:\n{qtext}"
    );
}

// VAL-MD-003: an H1 renders in BRIGHT and is immediately followed by a GHOST `─` rule spanning
// the content width (a rule line, not a text underline).
#[test]
fn h1_is_bright_and_followed_by_a_ghost_rule() {
    use frame::style::{BRIGHT, GHOST};
    let lines = render_markdown("# Tool Registry\n\nBody prose.", W);

    let heading = span_with(&lines, "Tool Registry").expect("h1 span");
    assert_eq!(heading.style.fg, Some(BRIGHT), "H1 is BRIGHT");
    assert!(
        !heading.style.add_modifier.contains(Modifier::UNDERLINED),
        "H1 uses a rule line, not a text underline"
    );

    // The line directly under the heading is a full-width GHOST rule.
    let h_idx = lines
        .iter()
        .position(|l| {
            l.spans
                .iter()
                .any(|s| s.content.contains("Tool Registry"))
        })
        .expect("heading line index");
    let rule = &lines[h_idx + 1];
    let rule_text: String = rule.spans.iter().map(|s| s.content.as_ref()).collect();
    assert!(
        rule_text.chars().all(|c| c == '─') && !rule_text.is_empty(),
        "rule row is all ─:\n{rule_text:?}"
    );
    let rule_span = rule.spans.iter().find(|s| s.content.contains('─')).unwrap();
    assert_eq!(rule_span.style.fg, Some(GHOST), "rule is GHOST");
}

// VAL-MD-005: an ordered `term — description` item renders the index in the number tone, the term
// in the accent (AMBER) tone, the ` — ` separator in GHOST, and the description in the body tone.
#[test]
fn numbered_term_dash_description_is_recolored() {
    use frame::style::{AMBER, GHOST, ORANGE_D, TXT};
    let lines = render_markdown("1. parse — decode JSON args against the schema", W);

    let index = span_with(&lines, "1.").expect("ordered index span");
    assert_eq!(index.style.fg, Some(ORANGE_D), "index is ORANGE_D");

    let term = span_with(&lines, "parse").expect("term span");
    assert_eq!(term.style.fg, Some(AMBER), "term is AMBER");

    let sep = lines
        .iter()
        .flat_map(|l| l.spans.iter())
        .find(|s| s.content.as_ref() == " — ")
        .expect("separator span");
    assert_eq!(sep.style.fg, Some(GHOST), "separator is GHOST");

    let desc = lines
        .iter()
        .flat_map(|l| l.spans.iter())
        .find(|s| s.content.contains("decode JSON args"))
        .expect("description span");
    assert_eq!(desc.style.fg, Some(TXT), "description is TXT");
}

// VAL-MD-006: an unordered list renders `•` bullets in ORANGE_D.
#[test]
fn bullets_render_in_orange_d() {
    use frame::style::ORANGE_D;
    let lines = render_markdown("- registry.rs\n- schema.rs", W);
    let bullet = lines
        .iter()
        .flat_map(|l| l.spans.iter())
        .find(|s| s.content.contains('•'))
        .expect("bullet span");
    assert_eq!(bullet.style.fg, Some(ORANGE_D), "bullet is ORANGE_D");
}

// VAL-MD-007: inline code chips render on the explicit INLBG background with the VS Code Dark+
// variable light-blue (VAR) foreground on every chip cell.
#[test]
fn inline_code_chip_has_var_fg_on_inlbg() {
    use frame::style::{INLBG, VAR};
    let lines = render_markdown("Sources: `registry.rs` and `schema.rs`.", W);
    let chip = span_with(&lines, "registry.rs").expect("inline code chip span");
    assert_eq!(chip.style.fg, Some(VAR), "chip fg is VAR light-blue");
    assert_eq!(chip.style.bg, Some(INLBG), "chip bg is INLBG");
}

// VAL-MD-008: a blockquote renders with a leading `▎` ORANGE_D bar and DIM quote text.
#[test]
fn blockquote_has_orange_d_bar_and_dim_text() {
    use frame::style::{DIM, ORANGE_D};
    let lines = render_markdown("> Note: unknown tools fall back to the generic card renderer.", W);
    let bar = lines
        .iter()
        .flat_map(|l| l.spans.iter())
        .find(|s| s.content.as_ref() == "▎")
        .expect("blockquote bar span");
    assert_eq!(bar.style.fg, Some(ORANGE_D), "bar is ORANGE_D");
    let text = lines
        .iter()
        .flat_map(|l| l.spans.iter())
        .find(|s| s.content.contains("unknown"))
        .expect("quote text span");
    assert_eq!(text.style.fg, Some(DIM), "quote text is DIM");
}

// VAL-MD-010 (glyph half): the markdown render path never emits the forbidden thin triangles
// `▸`/`▾` at any nesting depth (the deepest bullet level is repointed to the safe `·` glyph; the
// `get_bullet` unit tests lock the exact per-depth mapping).
#[test]
fn deep_bullets_use_safe_glyphs_no_thin_triangles() {
    let md = "- a\n    - b\n        - c\n            - d\n                - e";
    let lines = render_markdown(md, W);
    let text = flatten(&lines);
    assert!(!text.contains('▸'), "no ▸ in the render path:\n{text}");
    assert!(!text.contains('▾'), "no ▾ in the render path:\n{text}");
}
