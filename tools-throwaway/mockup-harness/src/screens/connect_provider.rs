//! auth/connect-provider — pick a provider to connect. A centered panel listing
//! the connectable providers (name + short description + a connected / not-
//! connected status dot), one selected (`›` marker + warm selection band +
//! brighter text), over the shared CLI chrome.

use ratatui::{
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{self, cb, m, ACCENT, AMBER, BRIGHT, DIM, FAINT, INFO_DOT, OK, TXT};

/// One provider offered in the catalog: name, one-line description, connected?.
struct Prov {
    name: &'static str,
    desc: &'static str,
    connected: bool,
}

const PROVIDERS: [Prov; 6] = [
    Prov { name: "OpenRouter", desc: "unified gateway \u{00b7} many models", connected: true },
    Prov { name: "Anthropic", desc: "Claude Opus \u{00b7} Sonnet \u{00b7} Haiku", connected: false },
    Prov { name: "OpenAI", desc: "GPT-4o \u{00b7} o-series reasoning", connected: true },
    Prov { name: "Google", desc: "Gemini 2.5 Pro \u{00b7} Flash", connected: false },
    Prov { name: "Azure OpenAI", desc: "enterprise-hosted OpenAI", connected: false },
    Prov { name: "GitHub Copilot", desc: "OAuth \u{00b7} editor models", connected: false },
];

const SELECTED: usize = 1;

pub fn render(f: &mut Frame) {
    let (body, dock) = chrome::cli_action(
        f,
        "auth \u{00b7} connect",
        8,
        &[
            ("enter", "connect"),
            ("\u{2191}\u{2193}", "select"),
            ("d", "disconnect"),
            ("esc", "close"),
        ],
    );
    chrome::transcript(f, body, backdrop());

    let inner = chrome::card(f, dock, "Connect a provider", AMBER, &[]);
    let w = inner.width as usize;
    let lines: Vec<Line> = PROVIDERS
        .iter()
        .enumerate()
        .map(|(i, p)| prov_row(w, i == SELECTED, p))
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}

const NAME_W: usize = 16;

fn prov_row(width: usize, sel: bool, p: &Prov) -> Line<'static> {
    let (glyph_color, status, status_color) = if p.connected {
        (OK, "connected", OK)
    } else {
        (INFO_DOT, "not connected", DIM)
    };

    let marker = if sel {
        cb(" \u{203a} ", ACCENT)
    } else {
        Span::raw("   ")
    };
    let name_span = if sel {
        cb(&format!("{:<NAME_W$}", p.name), BRIGHT)
    } else {
        chrome::c(&format!("{:<NAME_W$}", p.name), TXT)
    };

    let used = 3 + NAME_W + 2 + p.desc.chars().count();
    let meta_len = 2 + status.chars().count();
    let pad = width.saturating_sub(used + meta_len);

    Line::from(vec![
        marker,
        name_span,
        m("  "),
        chrome::c(p.desc, if sel { TXT } else { DIM }),
        Span::raw(" ".repeat(pad)),
        chrome::c(chrome::state_glyph(glyph_color), glyph_color),
        chrome::c(&format!(" {status}"), status_color),
    ])
}

fn backdrop() -> Vec<Line<'static>> {
    let dim = |s: &str| Line::from(Span::styled(s.to_string(), Style::default().fg(FAINT)));
    vec![
        chrome::user_cmd("/connect", "14:22:07"),
        Line::from(""),
        dim("   Pick a provider to connect for this session"),
    ]
}
