//! Session screen — the agent proposes a file edit and shows it as a unified
//! diff: a filename header, a hunk header, context lines dim, additions in green
//! and removals in red, with a `+N/-M` tally. The user approves or rejects from
//! the composer; the dots and colors carry the add/remove state, no icons.

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, c, m, t, ADD_BG, BAD, DEL_BG, DIM, FAINT, INFO, OK, TITLE};

pub fn render(f: &mut Frame) {
    let body = chrome::cli_page(
        f,
        "Chat",
        "[y/n/edit]  apply, skip, or ask for a change\u{2026}",
        &[
            ("y", "apply"),
            ("n", "skip"),
            ("ctrl+o", "collapse"),
            ("q", "quit"),
        ],
    );

    let mut lines: Vec<Line> = Vec::new();
    lines.push(chrome::user_cmd("give login rows a default token_version", "14:52:10"));
    lines.push(Line::from(""));

    lines.push(chrome::call_expanded(
        OK,
        "Proposed an edit to crates/server/src/models/session.rs",
    ));
    lines.push(chrome::call_detail(vec![
        c("+2", OK),
        m("  "),
        c("\u{2212}1", BAD),
        m("   \u{00b7}  1 hunk"),
    ]));
    lines.push(Line::from(""));

    lines.push(hunk("@@ -18,7 +18,8 @@ impl Session {"));
    lines.push(ctx("18", "18", "    pub fn new(user: UserId) -> Self {"));
    lines.push(ctx("19", "19", "        Self {"));
    lines.push(ctx("20", "20", "            user,"));
    lines.push(rem("21", "            token_version: None,"));
    lines.push(add("21", "            token_version: Some(1),"));
    lines.push(add("22", "            created_at: Clock::now(),"));
    lines.push(ctx("22", "23", "        }"));
    lines.push(ctx("23", "24", "    }"));
    lines.push(Line::from(""));

    lines.push(Line::from(vec![
        Span::raw("   "),
        t("New sessions now carry a non-null "),
        c("token_version", TITLE),
        t(", so the token-refresh query"),
    ]));
    lines.push(Line::from(vec![
        Span::raw("   "),
        t("stops panicking on returning users. Apply this edit?"),
    ]));

    chrome::transcript(f, body, lines);
}

const OLD_W: usize = 3;
const NEW_W: usize = 3;
const DIFF_W: usize = 108;

fn styled(fg: Color, bg: Option<Color>) -> Style {
    let s = Style::default().fg(fg);
    match bg {
        Some(b) => s.bg(b),
        None => s,
    }
}

fn diff_row(
    old: &str,
    new: &str,
    sign: &str,
    sign_color: Color,
    code_color: Color,
    code: &str,
    bg: Option<Color>,
) -> Line<'static> {
    let mut spans = vec![
        Span::styled(format!("  {old:>OLD_W$} "), styled(FAINT, bg)),
        Span::styled(format!("{new:>NEW_W$} "), styled(FAINT, bg)),
        Span::styled(format!("{sign} "), styled(sign_color, bg)),
        Span::styled(code.to_string(), styled(code_color, bg)),
    ];
    if let Some(b) = bg {
        let vis = 2 + OLD_W + 1 + NEW_W + 1 + 2 + code.chars().count();
        spans.push(Span::styled(
            " ".repeat(DIFF_W.saturating_sub(vis)),
            Style::default().bg(b),
        ));
    }
    Line::from(spans)
}

fn ctx(old: &str, new: &str, code: &str) -> Line<'static> {
    diff_row(old, new, " ", DIM, DIM, code, None)
}

fn add(new: &str, code: &str) -> Line<'static> {
    diff_row("", new, "+", OK, OK, code, Some(ADD_BG))
}

fn rem(old: &str, code: &str) -> Line<'static> {
    diff_row(old, "", "\u{2212}", BAD, BAD, code, Some(DEL_BG))
}

fn hunk(text: &str) -> Line<'static> {
    Line::from(vec![Span::styled(
        format!("   {text}"),
        Style::default().fg(INFO).add_modifier(Modifier::BOLD),
    )])
}
