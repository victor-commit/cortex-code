//! input/permission-prompt — the agent asks to run a concrete command; the user
//! answers Allow once / Allow always / Deny. Instead of a floating modal the
//! decision DOCKS in place of the input composer (cortex-code-design N-7): it
//! replaces the line you would otherwise type into. Fail-closed by construction
//! (`crates/tui/src/permission.rs`); the cursor sits on `Allow once`.

use ratatui::{
    style::Color,
    text::{Line, Span},
    Frame,
};

use crate::chrome::{
    self, cb, m, ACCENT, AMBER, BAD, BRIGHT, DIM, FAINT, OK, ORANGE, TXT, WARN,
};

pub fn render(f: &mut Frame) {
    let (body, dock) = chrome::cli_action(
        f,
        "Session",
        11,
        &[
            ("enter", "confirm"),
            ("\u{2191}\u{2193}", "choose"),
            ("esc", "deny"),
        ],
    );
    chrome::transcript(f, body, backdrop());

    let inner = chrome::card(f, dock, "Permission required", AMBER, &[]);
    let mut lines: Vec<Line> = vec![
        Line::from(vec![
            chrome::t("Allow "),
            cb("run", ACCENT),
            chrome::t(" to execute a shell command?"),
        ]),
        Line::from(""),
        Line::from(vec![
            chrome::c("  $ ", ORANGE),
            chrome::c("cargo test --all", BRIGHT),
            m("    in "),
            chrome::c("~/projects/cortex-code", FAINT),
        ]),
        Line::from(""),
    ];

    let opts: [(&str, &str, Color); 3] = [
        ("Allow once", "run this command a single time", OK),
        ("Allow always", "remember for this session", WARN),
        ("Deny", "refuse and tell the agent why", BAD),
    ];
    for (i, (label, desc, dot)) in opts.iter().enumerate() {
        lines.push(option_row(i == 0, label, desc, *dot));
    }

    f.render_widget(ratatui::widgets::Paragraph::new(lines), inner);
}

const LABEL_W: usize = 13;

fn option_row(sel: bool, label: &str, desc: &str, dot: Color) -> Line<'static> {
    let marker = if sel {
        cb(" \u{203a} ", ACCENT)
    } else {
        Span::raw("   ")
    };
    let label_span = if sel {
        cb(&format!("{label:<LABEL_W$}"), BRIGHT)
    } else {
        chrome::c(&format!("{label:<LABEL_W$}"), TXT)
    };
    Line::from(vec![
        marker,
        chrome::c("\u{25cf} ", dot),
        label_span,
        chrome::c("  ", DIM),
        chrome::c(desc, if sel { TXT } else { DIM }),
    ])
}

fn backdrop() -> Vec<Line<'static>> {
    let dim = |s: &str| Line::from(Span::styled(s.to_string(), ratatui::style::Style::default().fg(FAINT)));
    vec![
        chrome::user_cmd("run the full test suite and report the result", "14:41:02"),
        Line::from(""),
        dim("   Prepared command: cargo test --all"),
        dim("   Paused — waiting for you to allow the command below"),
    ]
}
