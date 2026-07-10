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
use std::cell::RefCell;

use crate::{buffer_to_text, registry, render_buffer};

/// Semantic constructor responsible for a rendered live activity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LiveActivitySource {
    WorkingBand,
    ToolStatusRunning,
    ShellStatusRunning,
    InlineLiveState,
}

impl LiveActivitySource {
    fn as_str(self) -> &'static str {
        match self {
            Self::WorkingBand => "working_band",
            Self::ToolStatusRunning => "tool_running",
            Self::ShellStatusRunning => "shell_running",
            Self::InlineLiveState => "inline_live_state",
        }
    }
}

/// The one semantic location allowed to own an activity's interrupt affordance.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LiveActivityInterrupt {
    Band,
    Inline,
    None,
}

impl LiveActivityInterrupt {
    fn as_str(self) -> &'static str {
        match self {
            Self::Band => "band",
            Self::Inline => "inline",
            Self::None => "none",
        }
    }
}

/// Render-time semantic evidence for one live activity constructor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LiveActivityEvent {
    pub source: LiveActivitySource,
    pub verb: String,
    pub elapsed: Option<String>,
    pub interrupt: LiveActivityInterrupt,
}

impl LiveActivityEvent {
    pub(crate) fn new(
        source: LiveActivitySource,
        verb: &str,
        elapsed: Option<String>,
        interrupt: LiveActivityInterrupt,
    ) -> Self {
        Self {
            source,
            verb: verb.to_owned(),
            elapsed,
            interrupt,
        }
    }

    /// Test-only semantic helper for an unregistered, differently worded
    /// inline activity. Its call path must still be captured through the
    /// canonical inline constructor.
    #[cfg(test)]
    pub(crate) fn unregistered_catalogue_state() -> String {
        crate::chrome::inline_live_state("cataloguing", 12.0)
    }
}

thread_local! {
    /// Capture is enabled only while `--audit-json` renders one registered
    /// screen. Normal rendering stays free of audit-only allocation and data.
    static LIVE_ACTIVITY_CAPTURE: RefCell<Option<Vec<LiveActivityEvent>>> = const {
        RefCell::new(None)
    };
}

/// Record semantic activity only when a render-time audit capture is active.
pub(crate) fn record_live_activity(event: LiveActivityEvent) {
    LIVE_ACTIVITY_CAPTURE.with(|capture| {
        if let Some(events) = capture.borrow_mut().as_mut() {
            events.push(event);
        }
    });
}

fn capture_live_activity<T>(render: impl FnOnce() -> T) -> (T, Vec<LiveActivityEvent>) {
    LIVE_ACTIVITY_CAPTURE.with(|capture| {
        let mut capture = capture.borrow_mut();
        assert!(
            capture.is_none(),
            "live activity audit capture must not nest renders"
        );
        *capture = Some(Vec::new());
    });
    let rendered = render();
    let events = LIVE_ACTIVITY_CAPTURE.with(|capture| {
        capture
            .borrow_mut()
            .take()
            .expect("live activity audit capture must remain active")
    });
    (rendered, events)
}

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
        let (buffer, events) = capture_live_activity(|| render_buffer(*render));
        print_screen(category, name, &buffer, &events);
    }
    println!("]}}");
}

fn print_screen(category: &str, name: &str, buffer: &Buffer, events: &[LiveActivityEvent]) {
    let rendered = buffer_to_text(buffer);
    let lines: Vec<&str> = rendered.lines().collect();
    print!(
        "{{\"id\":\"{category}/{name}\",\"events\":[",
        category = escape(category),
        name = escape(name)
    );
    for (index, event) in events.iter().enumerate() {
        if index > 0 {
            print!(",");
        }
        print!(
            "{{\"source\":\"{}\",\"verb\":\"{}\",\"elapsed\":{},\"interrupt\":\"{}\"}}",
            event.source.as_str(),
            escape(&event.verb),
            event
                .elapsed
                .as_deref()
                .map(|elapsed| format!("\"{}\"", escape(elapsed)))
                .unwrap_or_else(|| "null".to_owned()),
            event.interrupt.as_str(),
        );
    }
    print!("],\"rows\":[");
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
    use super::{
        LiveActivityEvent, LiveActivityInterrupt, LiveActivitySource, capture_live_activity, escape,
    };

    #[test]
    fn json_escape_handles_control_characters() {
        assert_eq!(escape("\"\\\n"), "\\\"\\\\\\n");
    }

    #[test]
    fn inline_live_helper_records_an_indirect_activity_event() {
        let (state, events) =
            capture_live_activity(LiveActivityEvent::unregistered_catalogue_state);

        assert_eq!(state, "● cataloguing 12s");
        assert_eq!(
            events,
            vec![LiveActivityEvent::new(
                LiveActivitySource::InlineLiveState,
                "cataloguing",
                Some("12s".to_owned()),
                LiveActivityInterrupt::None,
            )]
        );
    }
}
