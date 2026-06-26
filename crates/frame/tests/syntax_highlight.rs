//! Black-box behavior tests for tree-sitter syntax highlighting of fenced code blocks, mapped to
//! the TUI syntax-highlight validation assertions (VAL-TUI-016, VAL-TUI-017, VAL-TUI-018).
//! Clean-room original (Apache-2.0).
//!
//! Each test asserts *observable* rendered output: distinct syntax colors appear on language
//! tokens, partial/streaming blocks stay legible (bordered, no raw escapes), and unknown
//! languages degrade gracefully to a plain bordered container.

use frame::markdown::render_markdown;
use ratatui::text::Line;
use std::collections::BTreeSet;

const W: u16 = 80;

/// Box-drawing characters used by the code-block container.
const BORDER_CHARS: &[char] = &['┌', '┐', '└', '┘', '─', '│'];

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

/// The interior content lines of a bordered code block (top + bottom borders stripped).
fn content_lines<'a>(lines: &'a [Line<'static>]) -> &'a [Line<'static>] {
    if lines.len() >= 2 {
        &lines[1..lines.len() - 1]
    } else {
        lines
    }
}

/// Collect distinct foreground colors of "token" spans inside a code block: spans whose trimmed
/// content is non-empty and is not a border/structural glyph. Colors are keyed by their debug
/// form so the set is comparable regardless of `Color` variant.
fn distinct_token_colors(lines: &[Line<'static>]) -> BTreeSet<String> {
    let mut colors = BTreeSet::new();
    for line in content_lines(lines) {
        for span in &line.spans {
            let content = span.content.as_ref();
            let trimmed = content.trim();
            if trimmed.is_empty() {
                continue;
            }
            // Skip pure border/structural spans (the "│ " frame).
            if trimmed.chars().all(|c| BORDER_CHARS.contains(&c)) {
                continue;
            }
            if let Some(fg) = span.style.fg {
                colors.insert(format!("{fg:?}"));
            }
        }
    }
    colors
}

/// True if any span content carries a raw ANSI escape byte (would corrupt the frame).
fn has_raw_escape(lines: &[Line<'static>]) -> bool {
    lines
        .iter()
        .flat_map(|l| l.spans.iter())
        .any(|s| s.content.contains('\u{1b}'))
}

// VAL-TUI-016: tree-sitter syntax highlighting colors a Rust code block.
#[test]
fn rust_code_block_has_multiple_distinct_syntax_colors() {
    let md = "```rust\nfn main() { let x = 42; println!(\"{x}\"); }\n```";
    let lines = render_markdown(md, W);
    let text = flatten(&lines);

    // Fences consumed, distinct bordered container.
    assert!(!text.contains("```"), "fence markers consumed:\n{text}");
    assert!(
        text.contains('┌') && text.contains('└') && text.contains('│'),
        "bordered container:\n{text}"
    );
    // Content preserved.
    assert!(text.contains("fn main()"), "code content present:\n{text}");

    // Tree-sitter highlighting: keywords/literals/identifiers in distinct colors.
    let colors = distinct_token_colors(&lines);
    assert!(
        colors.len() >= 2,
        "expected >= 2 distinct syntax colors in the rust block, got {}: {:?}\n{text}",
        colors.len(),
        colors
    );
}

// VAL-TUI-017: tree-sitter syntax highlighting colors a Bash code block.
#[test]
fn bash_code_block_has_multiple_distinct_syntax_colors() {
    let md = "```bash\nfor f in *.txt; do echo \"$f\"; done\n```";
    let lines = render_markdown(md, W);
    let text = flatten(&lines);

    assert!(!text.contains("```"), "fence markers consumed:\n{text}");
    assert!(
        text.contains('┌') && text.contains('└') && text.contains('│'),
        "bordered container:\n{text}"
    );
    assert!(text.contains("for"), "code content present:\n{text}");

    let colors = distinct_token_colors(&lines);
    assert!(
        colors.len() >= 2,
        "expected >= 2 distinct syntax colors in the bash block, got {}: {:?}\n{text}",
        colors.len(),
        colors
    );
}

// VAL-TUI-017 (companion): the `sh` alias also highlights (language-aware via fence info string).
#[test]
fn bash_alias_sh_is_highlighted() {
    let md = "```sh\nif true; then echo hi; fi\n```";
    let lines = render_markdown(md, W);
    let colors = distinct_token_colors(&lines);
    assert!(
        colors.len() >= 2,
        "the `sh` alias should map to bash highlighting; got {:?}",
        colors
    );
}

// VAL-GAP-TUI-001: common non-Rust/Bash language fences are rendered with syntax styling.
#[test]
fn common_language_fences_render_non_plain_styles() {
    let fixtures = [
        (
            "python",
            "def add(a: int, b: int) -> int:\n    return a + b  # comment",
            2usize,
        ),
        (
            "javascript",
            "function add(a, b) { return `${a + b}`; }",
            2usize,
        ),
        (
            "typescript",
            "interface User { id: number; name: string }\nconst user: User = { id: 1, name: \"Ada\" };",
            2usize,
        ),
        (
            "go",
            "package main\nimport \"fmt\"\nfunc main() { fmt.Println(42) }",
            2usize,
        ),
        (
            "json",
            "{ \"name\": \"cortex\", \"enabled\": true, \"count\": 3 }",
            2usize,
        ),
        ("yaml", "name: cortex\nenabled: true\ncount: 3", 2usize),
        (
            "toml",
            "name = \"cortex\"\nenabled = true\ncount = 3",
            2usize,
        ),
        ("markdown", "# Header\n\n- item\n\n`inline`", 1usize),
        (
            "c",
            "#include <stdio.h>\nint main(void) { printf(\"%d\", 42); return 0; }",
            2usize,
        ),
        (
            "cpp",
            "#include <string>\nclass Widget { public: std::string name = \"w\"; };",
            1usize,
        ),
        (
            "java",
            "class Main { public static void main(String[] args) { String s = \"hi\"; int n = 1; } }",
            1usize,
        ),
        ("sh", "if true; then echo ok; fi", 2usize),
    ];

    for (language, code, min_colors) in fixtures {
        let md = format!("```{language}\n{code}\n```");
        let lines = render_markdown(&md, W);
        let text = flatten(&lines);
        let colors = distinct_token_colors(&lines);

        assert!(
            text.contains('┌') && text.contains('└') && text.contains('│'),
            "expected bordered code block for language '{language}':\n{text}"
        );
        assert!(
            colors.len() >= min_colors,
            "expected language-specific syntax colors for '{language}' (min {min_colors}), got {:?}\n{text}",
            colors
        );
    }
}

// VAL-GAP-TUI-001: code-fence info strings with attributes and aliases still resolve to the
// correct language renderer.
#[test]
fn fence_info_strings_with_attributes_use_language_renderer() {
    let fixtures = [
        (
            "python title=\"adder\"",
            "def add(a, b):\n    return a + b",
            2usize,
            "python",
        ),
        (
            "language-javascript",
            "function add(a, b) { return `${a + b}`; }",
            2usize,
            "javascript",
        ),
        (
            "{.typescript .numberLines}",
            "interface User { id: number; name: string }\nconst user: User = { id: 1, name: \"Ada\" };",
            2usize,
            "typescript",
        ),
        (
            ".go",
            "package main\nimport \"fmt\"\nfunc main() { fmt.Println(42) }",
            2usize,
            "go",
        ),
        ("c++", "class Widget { public: int value; };", 1usize, "cpp"),
    ];

    for (fence_info, code, min_colors, expected_label) in fixtures {
        let md = format!("```{fence_info}\n{code}\n```");
        let lines = render_markdown(&md, W);
        let colors = distinct_token_colors(&lines);
        let text = flatten(&lines);

        assert!(
            text.contains(&format!("┌─ {expected_label}")),
            "expected normalized code-fence label '{expected_label}' for info '{fence_info}':\n{text}"
        );
        assert!(
            colors.len() >= min_colors,
            "expected highlighted colors for fence info '{fence_info}' (min {min_colors}), got {:?}\n{text}",
            colors
        );
    }
}

// VAL-TUI-018: streaming markdown does not corrupt partially-received blocks.
#[test]
fn partial_code_block_is_legible_and_final_is_formatted() {
    // Mid-stream: the closing fence has NOT arrived yet.
    let partial = "Here is some code:\n\n```rust\nfn main() {\n    let x = 4";
    let partial_lines = render_markdown(partial, W);
    let partial_text = flatten(&partial_lines);

    // Legible bordered render with no raw escapes bleeding into the frame.
    assert!(
        !has_raw_escape(&partial_lines),
        "no raw ANSI escape bytes in the partial render:\n{partial_text}"
    );
    assert!(
        partial_text.contains('┌') && partial_text.contains('│'),
        "partial block is shown in a bordered container:\n{partial_text}"
    );
    assert!(
        partial_text.contains("fn main()"),
        "partial code content present:\n{partial_text}"
    );
    assert!(
        !partial_text.contains("```"),
        "no literal fence leaks into the partial render:\n{partial_text}"
    );
    // Every code-frame line is a complete bordered row (starts with a border glyph), i.e. the
    // partial block is not a half-built line.
    for line in content_lines(&partial_lines) {
        let s: String = line.spans.iter().map(|sp| sp.content.as_ref()).collect();
        if s.contains("fn main()") || s.contains("let x") {
            assert!(
                s.trim_start().starts_with('│'),
                "code line is framed by the left border:\n{s:?}"
            );
        }
    }

    // Final: the closing fence has arrived; the block reflows into the correct final formatting.
    let final_md = "Here is some code:\n\n```rust\nfn main() {\n    let x = 4;\n}\n```";
    let final_lines = render_markdown(final_md, W);
    let final_text = flatten(&final_lines);

    assert!(
        !has_raw_escape(&final_lines),
        "no raw escapes in the final render:\n{final_text}"
    );
    assert!(
        final_text.contains('┌') && final_text.contains('└') && final_text.contains('│'),
        "final block is a complete bordered container:\n{final_text}"
    );
    assert!(
        !final_text.contains("```"),
        "fences consumed:\n{final_text}"
    );
    assert!(
        final_text.contains("fn main()") && final_text.contains("let x = 4;"),
        "final code content correct:\n{final_text}"
    );
    // Final block is still highlighted.
    let colors = distinct_token_colors(&final_lines);
    assert!(
        colors.len() >= 2,
        "final block remains syntax-highlighted; got {:?}",
        colors
    );
}

// Unknown languages degrade gracefully to a plain bordered container (no panic, no corruption).
#[test]
fn unknown_language_degrades_to_plain_bordered_block() {
    let md = "```wat-lang\nsome arbitrary text\nmore text\n```";
    let lines = render_markdown(md, W);
    let text = flatten(&lines);

    assert!(!has_raw_escape(&lines), "no raw escapes:\n{text}");
    assert!(!text.contains("```"), "fences consumed:\n{text}");
    assert!(
        text.contains('┌') && text.contains('└') && text.contains('│'),
        "unknown language still renders in a bordered container:\n{text}"
    );
    assert!(
        text.contains("some arbitrary text") && text.contains("more text"),
        "content preserved verbatim:\n{text}"
    );
}
