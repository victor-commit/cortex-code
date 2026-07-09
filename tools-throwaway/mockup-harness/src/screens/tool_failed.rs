//! Session screen - one failed tool call (patterns 1 + 8): BAD `●` on the
//! tool line paired with a `✗` verdict on the `⎿` elbow (glyph + color, so
//! failure survives color-stripping), the failure evidence and BOTH recovery
//! keys (`r retry`, `y apply fix`) forming one block marked by a BAD `│`
//! left bar - never in a global footer. Below it a plain-English explanation
//! proposes the concrete fix. Red appears only on the verdict elements
//! (dot, ✗, bar).

use ratatui::{
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, c, m, t, StatusLine, ToolStatus, BRIGHT, FAINT};

/// Code identifier styling (single treatment for the whole answer).
fn ident(s: &str) -> Span<'static> {
    c(s, BRIGHT)
}

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let body = chrome::cli_page(
        f,
        "reply to Cortex, or ask a follow-up\u{2026}",
        &status,
    );

    let mut verdict_row = chrome::verdict(false, "build failed before any test ran");
    verdict_row.push(m(&format!(
        " {} exit 101 {} {}",
        chrome::SEP,
        chrome::SEP,
        chrome::timefmt(6.0)
    )));

    let mut recovery: Vec<Span> = chrome::key_span("r", "retry");
    recovery.push(c(&format!(" {} ", chrome::SEP), FAINT));
    recovery.extend(chrome::key_span("y", "apply fix"));

    let mut lines = vec![
        chrome::user_cmd("run the test suite", Some("3m ago")),
        Line::from(""),
        chrome::agent_head("Cortex"),
        Line::from(""),
        para(vec![t("Running the workspace suite.")]),
        Line::from(""),
        chrome::tool_line(ToolStatus::Failed, "Bash", "cargo test --workspace"),
    ];
    lines.extend(chrome::elbow(vec![verdict_row]));
    lines.extend(chrome::fail_rows(vec![
        vec![m("error[E0432]: unresolved import `serde_json`")],
        vec![m("crates/server/src/api.rs:3")],
        recovery,
    ]));
    lines.push(Line::from(""));
    lines.extend(vec![
        para(vec![
            t("The suite never started, the build broke first. "),
            ident("api.rs"),
            t(" imports "),
            ident("serde_json"),
            t(", but that"),
        ]),
        para(vec![
            t("crate is not listed in "),
            ident("crates/server/Cargo.toml"),
            t(". I can add the dependency and re-run the suite."),
        ]),
        Line::from(""),
    ]);

    chrome::transcript(f, body, lines);
}

/// A prose row indented to column 4, under the `agent_head` heading.
fn para(mut spans: Vec<Span<'static>>) -> Line<'static> {
    let mut row = vec![Span::raw("   ")];
    row.append(&mut spans);
    Line::from(row)
}
