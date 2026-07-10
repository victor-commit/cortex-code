//! Deterministic semantic capture for gallery-wide audits.
//!
//! Rendered text intentionally trims trailing cells and loses all Ratatui
//! styles. The sweep needs the source buffer to prove selection washes,
//! semantic hues, and whether a hint sits inside a dock. This module emits a
//! stable JSON projection of every registered render without writing any
//! gallery artifacts.

use ratatui::{
    buffer::Buffer,
    style::{Color, Modifier},
};

use crate::{buffer_to_text, registry, render_buffer};

/// Print a deterministic, machine-readable view of every registered buffer.
///
/// The output is deliberately dependency-free so the throwaway harness remains
/// small. Rows carry text plus runs of identical foreground, background, and
/// bold style. A sweep can therefore inspect exact cell geometry without
/// trying to infer invisible PNG styling from text.
pub fn print_json() {
    let entries = registry();
    print!("{{\"screens\":[");
    for (index, (category, name, render)) in entries.iter().enumerate() {
        if index > 0 {
            print!(",");
        }
        let buffer = render_buffer(*render);
        print_screen(category, name, &buffer);
    }
    println!("]}}");
}

fn print_screen(category: &str, name: &str, buffer: &Buffer) {
    let rendered = buffer_to_text(buffer);
    let lines: Vec<&str> = rendered.lines().collect();
    print!(
        "{{\"id\":\"{category}/{name}\",\"rows\":[",
        category = escape(category),
        name = escape(name)
    );
    for y in 0..buffer.area().height {
        if y > 0 {
            print!(",");
        }
        let text = lines.get(y as usize).copied().unwrap_or_default();
        print!(
            "{{\"line\":{},\"text\":\"{}\",\"spans\":[",
            y + 1,
            escape(text)
        );
        print_spans(buffer, y);
        print!("]}}");
    }
    print!("]}}");
}

fn print_spans(buffer: &Buffer, y: u16) {
    let width = buffer.area().width;
    let mut start = 0;
    while start < width {
        let Some(cell) = buffer.cell((start, y)) else {
            break;
        };
        let foreground = color_name(cell.fg);
        let background = color_name(cell.bg);
        let bold = cell.modifier.contains(Modifier::BOLD);
        let mut end = start + 1;
        while end < width {
            let Some(next) = buffer.cell((end, y)) else {
                break;
            };
            if color_name(next.fg) != foreground
                || color_name(next.bg) != background
                || next.modifier.contains(Modifier::BOLD) != bold
            {
                break;
            }
            end += 1;
        }
        if start > 0 {
            print!(",");
        }
        print!(
            "{{\"start\":{start},\"end\":{end},\"fg\":\"{foreground}\",\"bg\":\"{background}\",\"bold\":{bold}}}"
        );
        start = end;
    }
}

fn color_name(color: Color) -> String {
    match color {
        Color::Rgb(r, g, b) => format!("rgb({r},{g},{b})"),
        Color::Reset => "reset".to_string(),
        other => format!("{other:?}").to_ascii_lowercase(),
    }
}

fn escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\u{08}' => escaped.push_str("\\b"),
            '\u{0C}' => escaped.push_str("\\f"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            c if c.is_control() => escaped.push_str(&format!("\\u{:04x}", c as u32)),
            c => escaped.push(c),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::escape;

    #[test]
    fn json_escape_handles_control_characters() {
        assert_eq!(escape("\"\\\n"), "\\\"\\\\\\n");
    }
}
