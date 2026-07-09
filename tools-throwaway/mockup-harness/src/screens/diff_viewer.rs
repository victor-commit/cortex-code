//! Full-screen unified diff for the second changed file.

use ratatui::{
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{self, c, m, DiffRow, StatusLine, BAD, OK};

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let body = chrome::cli_view(
        f,
        "diff · file 2 of 6 · crates/server/src/routes.rs · n next file · p prev file",
        &status,
    );

    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("   "),
            m("pending change"),
        ]),
        Line::from(""),
        hunk("@@ -134,4 +134,7 @@ fn register_routes(app: &mut Router)"),
    ];
    lines.extend(chrome::diff_lines(
        134,
        134,
        &[
            DiffRow::Ctx("    app.route(\"/health\", get(health));"),
            DiffRow::Del("    // TODO: rate limiting"),
            DiffRow::Add("    let limiter = RateLimiter::per_ip(5, Duration::from_secs(60));"),
            DiffRow::Add("    app.layer(limiter.clone());"),
            DiffRow::Ctx("    app.route(\"/logout\", post(logout));"),
            DiffRow::Del("    app.route(\"/login\", post(login));"),
            DiffRow::Add("    app.route(\"/login\", post(login).layer(limiter));"),
            DiffRow::Add("    app.route(\"/refresh\", post(refresh));"),
            DiffRow::Add("    app.route(\"/session\", get(session));"),
        ],
    ));
    lines.extend([
        Line::from(""),
        hunk("@@ -148,3 +151,5 @@ async fn login(body: Json<Creds>)"),
    ]);
    lines.extend(chrome::diff_lines(
        148,
        151,
        &[
            DiffRow::Ctx("    let user = verify(&body).await?;"),
            DiffRow::Del("    let token = Token::issue(user);"),
            DiffRow::Del("    Ok(Json(token))"),
            DiffRow::Add("    let token = Token::issue(user);"),
            DiffRow::Add("    audit::login(&user).await;"),
            DiffRow::Add("    metrics::count(\"login.success\", 1);"),
            DiffRow::Add("    Ok(Json(token))"),
        ],
    ));
    lines.extend([
        Line::from(""),
        summary(),
    ]);

    f.render_widget(Paragraph::new(lines), body);
}

fn hunk(text: &str) -> Line<'static> {
    Line::from(vec![Span::raw("   "), m(text)])
}

fn summary() -> Line<'static> {
    Line::from(vec![
        Span::raw("   "),
        m("summary  "),
        c("+9", OK),
        m(" "),
        c("-4", BAD),
        m(" · 2 hunks · next: crates/server/src/util.rs (file 3 of 6)"),
    ])
}
