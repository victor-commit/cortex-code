//! Full-screen hunk staging review for the second changed file.

use ratatui::{
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{
    self, c, m, DiffHunkState, DiffRow, StatusLine, BAD, OK,
};

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let body = chrome::cli_view(
        f,
        "diff review · file 2 of 6 · staged 1/6 hunks · crates/server/src/routes.rs · a stage all · enter finish review",
        &status,
    );

    let mut lines = vec![
        Line::from(""),
        Line::from(vec![Span::raw("   "), m("review each hunk before applying")]),
        Line::from(""),
        chrome::diff_hunk_header(
            "@@ -134,3 +134,4 @@ fn register_routes(app: &mut Router)",
            DiffHunkState::Pending,
            true,
        ),
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
        ],
    ));
    lines.extend([
        Line::from(""),
        chrome::diff_hunk_header(
            "@@ -148,3 +151,5 @@ async fn login(body: Json<Creds>)",
            DiffHunkState::Staged,
            false,
        ),
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
        chrome::diff_hunk_header(
            "@@ -188,2 +193,4 @@ async fn refresh(session: Session)",
            DiffHunkState::Skipped,
            false,
        ),
    ]);
    lines.extend(chrome::diff_lines(
        188,
        193,
        &[
            DiffRow::Del("    refresh::rotate(session).await?;"),
            DiffRow::Add("    refresh::rotate(session).await?;"),
            DiffRow::Add("    audit::refresh(&session).await;"),
            DiffRow::Add("    Ok(())"),
        ],
    ));
    lines.extend([
        Line::from(""),
        summary(),
    ]);

    f.render_widget(Paragraph::new(lines), body);
}

fn summary() -> Line<'static> {
    Line::from(vec![
        Span::raw("   "),
        m("summary  "),
        c("+9", OK),
        m(" "),
        c("-4", BAD),
        m(" · 3 hunks · file 2 of 6"),
    ])
}
