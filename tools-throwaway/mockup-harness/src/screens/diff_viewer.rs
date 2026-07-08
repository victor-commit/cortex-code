//! input/diff-viewer — the `/diff` view rendered as a live CLI turn: the user
//! asks to see the pending change, the agent prints a unified diff for one file
//! with a `@@` hunk header, `+`/`-` gutters (added green, removed red, context
//! dim) and a change summary. Mirrors `crates/tui/src/diff.rs` line hues.

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, c, m, t, ADD_BG, BAD, BRIGHT, DEL_BG, DIM, FAINT, INFO, OK};

pub fn render(f: &mut Frame) {
    let body = chrome::cli_page(
        f,
        "/diff",
        "press n / p to step files \u{2014} or type a follow-up instruction\u{2026}",
        &[
            ("n", "next file"),
            ("p", "prev file"),
            ("a", "stage"),
            ("q", "quit"),
        ],
    );

    let mut lines: Vec<Line> = Vec::new();
    lines.push(chrome::user_cmd("/diff routes.rs", "14:52:10"));
    lines.push(Line::from(""));
    lines.push(chrome::agent_head(
        "Pending change \u{2014} file 2 of 6 \u{00b7} +9 \u{2212}4",
    ));
    lines.push(Line::from(vec![
        m("   file  "),
        Span::styled(
            "crates/server/src/routes.rs",
            Style::default().fg(BRIGHT).add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(""));

    // Hunk header
    lines.push(hunk("@@ -134,7 +134,12 @@ fn register_routes(app: &mut Router)"));
    lines.push(ctx("134", "134", "    app.route(\"/health\", get(health));"));
    lines.push(ctx("135", "135", "    app.route(\"/login\", post(login));"));
    lines.push(rem("136", "    // TODO: rate limiting"));
    lines.push(add("136", "    let limiter = RateLimiter::per_ip(5, Duration::from_secs(60));"));
    lines.push(add("137", "    app.layer(limiter.clone());"));
    lines.push(ctx("137", "138", "    app.route(\"/logout\", post(logout));"));
    lines.push(Line::from(""));

    // Second hunk
    lines.push(hunk("@@ -148,3 +153,4 @@ async fn login(body: Json<Creds>)"));
    lines.push(ctx("148", "153", "    let user = verify(&body).await?;"));
    lines.push(rem("149", "    Ok(Json(Token::issue(user)))"));
    lines.push(add("154", "    let token = Token::issue(user);"));
    lines.push(add("155", "    Ok(Json(token))"));
    lines.push(Line::from(""));

    lines.push(Line::from(vec![
        m("   summary  "),
        c("+9", OK),
        m("  "),
        c("\u{2212}4", BAD),
        m("     "),
        t("2 hunks"),
        m("     next  "),
        c("util.rs", INFO),
        m(" (file 3 of 6)"),
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
