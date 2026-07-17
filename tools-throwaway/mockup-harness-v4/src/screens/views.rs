//! Pure views / side inspectors (F6).
//!
//! Status, timeline, MCP, diffs, help, review, theme galleries, and sidebars.

use crate::chrome::canvas::Canvas;
use crate::chrome::components::{
    paint_default_putty, paint_demo_top_bar, paint_detail_footer, paint_detail_header,
    paint_user_turn, reject_copper_residual, MODEL_LABEL,
};
use crate::chrome::tokens::{ColorToken, TextStyle, Theme};

pub fn render(id: &str) -> Result<Canvas, String> {
    match id {
        "status" => status(),
        "timeline" => timeline(),
        "mcp" => mcp(),
        "diff-viewer" => diff_viewer(),
        "diff-viewer-staging" => diff_viewer_staging(),
        "help" => help(),
        "review" => review(),
        "theme-gallery-dark" => theme_gallery_dark(),
        "theme-gallery-light" => theme_gallery_light(),
        "sidebar-context" => sidebar_context(),
        "sidebar-files" => sidebar_files(),
        "sidebar-lsp" => sidebar_lsp(),
        "sidebar-todo" => sidebar_todo(),
        "sidebar-subagents" => sidebar_subagents(),
        "sidebar-mcp" => sidebar_mcp(),
        "sidebar-queue" => sidebar_queue(),
        other => Err(format!("unknown views pure-Cortex id: {other}")),
    }
}

fn status() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_detail_header(
        &mut canvas,
        "Session status",
        "Active configuration · return keeps draft and turn",
    )?;
    canvas.write(7, 4, "Active now", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(
        8,
        4,
        &format!("Model · {MODEL_LABEL}"),
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        9,
        4,
        "Mode · always-approve · Shift+Tab cycles",
        TextStyle::plain(ColorToken::Secondary),
    )?;
    canvas.write(
        10,
        4,
        "Project · /projects/demo · master · clean",
        TextStyle::plain(ColorToken::Secondary),
    )?;
    canvas.write(
        11,
        4,
        "Session · Retry policy review · 17 turns",
        TextStyle::plain(ColorToken::Secondary),
    )?;
    canvas.write(
        13,
        4,
        "Needs attention",
        TextStyle::bold(ColorToken::Bright),
    )?;
    canvas.write(
        14,
        4,
        "● Docs research · needs sign-in before next lookup",
        TextStyle::plain(ColorToken::Warn),
    )?;
    canvas.write(
        15,
        4,
        "○ Local tools · ready · no blockers",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        18,
        4,
        "Context  ████████░░░░░░░░░░░░  8.5K / 1.0M",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        20,
        4,
        "Advanced raw metadata stays folded until Details",
        TextStyle::plain(ColorToken::Dim),
    )?;
    paint_detail_footer(
        &mut canvas,
        "Esc to return · keeps draft, selection, and origin",
    )?;
    finish(canvas)
}

fn timeline() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_detail_header(
        &mut canvas,
        "Timeline",
        "Turn ticks for this session · jump restores scroll target",
    )?;
    let ticks = [
        ("17", "You · Ship it. Summarize what landed", true),
        ("16", "Cortex · Retry path is guarded", false),
        ("15", "Tool · Shell cargo test retry", false),
        ("14", "You · Tight-loop the HTTP client", false),
        ("13", "Cortex · Added jittered backoff", false),
        ("12", "Tool · Edit crates/agent/src/retry.rs", false),
    ];
    for (row, (n, label, selected)) in (7u16..).zip(ticks) {
        if selected {
            canvas.write(
                row,
                4,
                &format!("› {n}  {label}"),
                TextStyle::bold(ColorToken::Accent),
            )?;
        } else {
            canvas.write(
                row,
                4,
                &format!("  {n}  {label}"),
                TextStyle::plain(ColorToken::Text),
            )?;
        }
    }
    canvas.write(
        16,
        4,
        "Hover previews the turn body · Enter jumps",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_detail_footer(
        &mut canvas,
        "Enter to jump · Esc to return · origin stays this chat",
    )?;
    finish(canvas)
}

fn mcp() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_detail_header(
        &mut canvas,
        "MCP",
        "Capability-led connections · transport detail stays folded",
    )?;
    canvas.write(7, 4, "Capabilities", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(
        9,
        4,
        "› Docs research",
        TextStyle::bold(ColorToken::Accent),
    )?;
    canvas.write(
        10,
        6,
        "Needs sign-in · Open connection",
        TextStyle::plain(ColorToken::Warn),
    )?;
    canvas.write(
        12,
        4,
        "  Local tools",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        13,
        6,
        "Ready · search, read, shell",
        TextStyle::plain(ColorToken::Ok),
    )?;
    canvas.write(
        15,
        4,
        "  Memory notes",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        16,
        6,
        "Ready · recall + write",
        TextStyle::plain(ColorToken::Ok),
    )?;
    canvas.write(
        19,
        4,
        "Exception-first · attention rows float above ready peers",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_detail_footer(
        &mut canvas,
        "Enter opens selected · Esc returns to conversation",
    )?;
    finish(canvas)
}

fn diff_viewer() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_detail_header(
        &mut canvas,
        "Diff · crates/agent/src/retry.rs",
        "Full file evidence · Esc returns without applying",
    )?;
    canvas.write(
        7,
        4,
        "@@ -40,6 +40,14 @@ fn schedule(floor_ms: u64)",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        9,
        4,
        "  let floor = floor_ms.max(1);",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        10,
        4,
        "- // trust caller floor",
        TextStyle::on(ColorToken::Bad, ColorToken::DeleteBackground),
    )?;
    canvas.write(
        11,
        4,
        "+ if floor_ms < 250 {",
        TextStyle::on(ColorToken::Ok, ColorToken::AddBackground),
    )?;
    canvas.write(
        12,
        4,
        "+     return Err(RetryError::Floor);",
        TextStyle::on(ColorToken::Ok, ColorToken::AddBackground),
    )?;
    canvas.write(
        13,
        4,
        "+ }",
        TextStyle::on(ColorToken::Ok, ColorToken::AddBackground),
    )?;
    canvas.write(
        14,
        4,
        "  rng.retry_after_ms = floor;",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        17,
        4,
        "+12 / -4 · green add · red delete",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_detail_footer(
        &mut canvas,
        "Esc to return · a apply when owned by permission dock",
    )?;
    finish(canvas)
}

fn diff_viewer_staging() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_detail_header(
        &mut canvas,
        "Staging review",
        "Select hunks to keep · rest stays unstaged",
    )?;
    canvas.write(
        7,
        4,
        "› Hunk 1 · floor guard",
        TextStyle::bold(ColorToken::Accent),
    )?;
    canvas.write(
        8,
        6,
        "+ if floor_ms < 250 { return Err(..); }",
        TextStyle::plain(ColorToken::Ok),
    )?;
    canvas.write(
        9,
        6,
        "staged · will apply on confirm",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        11,
        4,
        "  Hunk 2 · docs note",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        12,
        6,
        "+ // floors documented in module",
        TextStyle::plain(ColorToken::Ok),
    )?;
    canvas.write(
        13,
        6,
        "unstaged · skip for this apply",
        TextStyle::plain(ColorToken::Dim),
    )?;
    canvas.write(
        16,
        4,
        "Space toggles staged · Enter applies selection",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_detail_footer(
        &mut canvas,
        "Enter to apply staged · Esc cancels without writes",
    )?;
    finish(canvas)
}

fn help() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_detail_header(
        &mut canvas,
        "Shortcuts",
        "Ctrl+. cheatsheet · context-sensitive hints match the putty",
    )?;
    let rows = [
        ("Shift+Tab", "cycle mode / permission chip"),
        ("Ctrl+c", "cancel stream or clear draft step"),
        ("Ctrl+.", "open this shortcuts help"),
        ("/", "slash command menu"),
        ("@", "file mention dropdown"),
        ("[stop]", "halt the busy turn · keeps draft"),
        ("Enter", "send · or apply focused dock action"),
        ("Esc", "back / cancel · return to origin"),
    ];
    for (row, (key, meaning)) in (7u16..).zip(rows) {
        canvas.write(row, 4, key, TextStyle::bold(ColorToken::Accent))?;
        canvas.write(row, 18, meaning, TextStyle::plain(ColorToken::Text))?;
    }
    canvas.write(
        17,
        4,
        "Local dock bylines stay next to the action they control.",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_detail_footer(
        &mut canvas,
        "Esc to return · putty footer remains outside this view",
    )?;
    finish(canvas)
}

fn review() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_detail_header(
        &mut canvas,
        "Plan review",
        "Findings before continue · approve returns to the same origin",
    )?;
    canvas.write(7, 4, "Findings", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(
        9,
        4,
        "● Floor still accepts zero-ms callers",
        TextStyle::plain(ColorToken::Warn),
    )?;
    canvas.write(
        10,
        6,
        "Request: reject floor_ms under 250",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        12,
        4,
        "○ Docs mention matches the new guard",
        TextStyle::plain(ColorToken::Ok),
    )?;
    canvas.write(
        13,
        6,
        "No change required",
        TextStyle::plain(ColorToken::Dim),
    )?;
    canvas.write(
        16,
        4,
        "› Request changes",
        TextStyle::bold(ColorToken::Accent),
    )?;
    canvas.write(17, 4, "  Approve plan", TextStyle::plain(ColorToken::Text))?;
    canvas.write(18, 4, "  Continue without plan edits", TextStyle::plain(ColorToken::Text))?;
    paint_detail_footer(
        &mut canvas,
        "Enter selects · Esc returns with findings visible",
    )?;
    finish(canvas)
}

fn theme_gallery_dark() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_detail_header(
        &mut canvas,
        "Theme gallery · dark",
        "CortexNight canvas #141414 · magenta accent #bb9af7",
    )?;
    canvas.write(7, 4, "Swatches", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(9, 4, "Text", TextStyle::plain(ColorToken::Text))?;
    canvas.write(9, 20, "#e1e1e1 primary", TextStyle::plain(ColorToken::Muted))?;
    canvas.write(10, 4, "Accent", TextStyle::accent())?;
    canvas.write(10, 20, "#bb9af7 assistant", TextStyle::plain(ColorToken::Muted))?;
    canvas.write(11, 4, "Running", TextStyle::plain(ColorToken::Running))?;
    canvas.write(11, 20, "#7dcfff live tools", TextStyle::plain(ColorToken::Muted))?;
    canvas.write(12, 4, "Success", TextStyle::plain(ColorToken::Ok))?;
    canvas.write(12, 20, "#9ece6a additions", TextStyle::plain(ColorToken::Muted))?;
    canvas.write(13, 4, "Error", TextStyle::plain(ColorToken::Bad))?;
    canvas.write(13, 20, "#f7768e failures", TextStyle::plain(ColorToken::Muted))?;
    canvas.write(14, 4, "Warning", TextStyle::plain(ColorToken::Warn))?;
    canvas.write(14, 20, "#e0af68 commands", TextStyle::plain(ColorToken::Muted))?;
    canvas.write(
        17,
        4,
        "Preview jewel",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        18,
        4,
        "╭──────────────────────────────────────────────╮",
        TextStyle::plain(ColorToken::Border),
    )?;
    canvas.write(
        19,
        4,
        "│ ❯ sample draft on CortexNight                  │",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        20,
        4,
        &format!("│ {MODEL_LABEL} · always-approve                 │"),
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        21,
        4,
        "╰──────────────────────────────────────────────╯",
        TextStyle::plain(ColorToken::Border),
    )?;
    paint_detail_footer(
        &mut canvas,
        "Esc to return · Apply lives on the theme picker dock",
    )?;
    finish(canvas)
}

fn theme_gallery_light() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexDay);
    paint_detail_header(
        &mut canvas,
        "Theme gallery · light",
        "CortexDay canvas #eeeeee · deepened magenta #7d4bc6",
    )?;
    canvas.write(7, 4, "Swatches", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(9, 4, "Text", TextStyle::plain(ColorToken::Text))?;
    canvas.write(9, 20, "#262626 primary", TextStyle::plain(ColorToken::Muted))?;
    canvas.write(10, 4, "Accent", TextStyle::accent())?;
    canvas.write(10, 20, "#7d4bc6 assistant", TextStyle::plain(ColorToken::Muted))?;
    canvas.write(11, 4, "Running", TextStyle::plain(ColorToken::Running))?;
    canvas.write(12, 4, "Success", TextStyle::plain(ColorToken::Ok))?;
    canvas.write(13, 4, "Error", TextStyle::plain(ColorToken::Bad))?;
    canvas.write(
        16,
        4,
        "Light peer only · pure dump default remains CortexNight",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        18,
        4,
        "╭──────────────────────────────────────────────╮",
        TextStyle::plain(ColorToken::Border),
    )?;
    canvas.write(
        19,
        4,
        "│ ❯ sample draft on CortexDay                    │",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        20,
        4,
        &format!("│ {MODEL_LABEL} · always-approve                 │"),
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        21,
        4,
        "╰──────────────────────────────────────────────╯",
        TextStyle::plain(ColorToken::Border),
    )?;
    paint_detail_footer(
        &mut canvas,
        "Esc to return · no warm-paper Cortex residue",
    )?;
    finish(canvas)
}

/// Shared left transcript + narrow right inspector for sidebar IDs.
fn paint_sidebar_shell(
    canvas: &mut Canvas,
    divider_col: u16,
    title: &str,
    body: &[(&str, TextStyle)],
) -> Result<(), String> {
    paint_demo_top_bar(canvas)?;
    paint_user_turn(
        canvas,
        3,
        &["Keep the chat visible while inspecting context."],
    )?;
    canvas.write(7, 2, "◆ Cortex", TextStyle::plain(ColorToken::Dim))?;
    canvas.write(
        8,
        4,
        "Retry path is guarded. Sidebar stays task-local.",
        TextStyle::plain(ColorToken::Dim),
    )?;

    // Vertical divider.
    for r in 3..36 {
        canvas.put(r, divider_col, '│', TextStyle::plain(ColorToken::Border))?;
    }
    let start = divider_col + 2;
    canvas.write(3, start, title, TextStyle::bold(ColorToken::Bright))?;
    for (row, (line, style)) in (5u16..).zip(body.iter()) {
        canvas.write(row, start, line, *style)?;
    }
    canvas.write(
        37,
        4,
        "Esc closes inspector · transcript origin unchanged",
        TextStyle::plain(ColorToken::Dim),
    )?;
    paint_default_putty(canvas)?;
    Ok(())
}

fn sidebar_context() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_sidebar_shell(
        &mut canvas,
        62,
        "Context",
        &[
            ("Usage", TextStyle::bold(ColorToken::Bright)),
            ("8.5K / 1.0M", TextStyle::plain(ColorToken::Text)),
            ("████░░░░░░  1%", TextStyle::plain(ColorToken::Muted)),
            ("", TextStyle::plain(ColorToken::Dim)),
            ("Memory", TextStyle::bold(ColorToken::Bright)),
            ("· retry floors policy", TextStyle::plain(ColorToken::Secondary)),
            ("· workspace master", TextStyle::plain(ColorToken::Secondary)),
            ("", TextStyle::plain(ColorToken::Dim)),
            ("Project", TextStyle::bold(ColorToken::Bright)),
            ("/projects/demo", TextStyle::plain(ColorToken::Path)),
        ],
    )?;
    finish(canvas)
}

fn sidebar_files() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_sidebar_shell(
        &mut canvas,
        62,
        "Files",
        &[
            ("crates/agent/src/", TextStyle::bold(ColorToken::Bright)),
            ("› retry.rs", TextStyle::bold(ColorToken::Accent)),
            ("  rate_limit.rs", TextStyle::plain(ColorToken::Text)),
            ("  mod.rs", TextStyle::plain(ColorToken::Text)),
            ("", TextStyle::plain(ColorToken::Dim)),
            ("tests/", TextStyle::bold(ColorToken::Bright)),
            ("  retry_floors.rs", TextStyle::plain(ColorToken::Text)),
            ("", TextStyle::plain(ColorToken::Dim)),
            ("docs/retry.md", TextStyle::plain(ColorToken::Secondary)),
        ],
    )?;
    finish(canvas)
}

fn sidebar_lsp() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_sidebar_shell(
        &mut canvas,
        62,
        "Findings",
        &[
            ("Diagnostics", TextStyle::bold(ColorToken::Bright)),
            ("● retry.rs:42", TextStyle::plain(ColorToken::Warn)),
            ("  floor_ms may underflow", TextStyle::plain(ColorToken::Muted)),
            ("○ rate_limit.rs", TextStyle::plain(ColorToken::Ok)),
            ("  clean", TextStyle::plain(ColorToken::Dim)),
            ("", TextStyle::plain(ColorToken::Dim)),
            ("Honest analog panel", TextStyle::plain(ColorToken::Dim)),
            ("Cortex maps sparse LSP", TextStyle::plain(ColorToken::Dim)),
            ("rows onto findings.", TextStyle::plain(ColorToken::Dim)),
        ],
    )?;
    finish(canvas)
}

fn sidebar_todo() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_sidebar_shell(
        &mut canvas,
        62,
        "Todos",
        &[
            ("Plan", TextStyle::bold(ColorToken::Bright)),
            ("✓ guard floor at 250ms", TextStyle::plain(ColorToken::Ok)),
            ("› add floor integration", TextStyle::bold(ColorToken::Accent)),
            ("  document jitter order", TextStyle::plain(ColorToken::Text)),
            ("  note rate-limit ceiling", TextStyle::plain(ColorToken::Text)),
            ("", TextStyle::plain(ColorToken::Dim)),
            ("1 remaining open", TextStyle::plain(ColorToken::Muted)),
        ],
    )?;
    finish(canvas)
}

fn sidebar_subagents() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_sidebar_shell(
        &mut canvas,
        62,
        "Subagents",
        &[
            ("Live", TextStyle::bold(ColorToken::Bright)),
            ("● audit-subagent", TextStyle::plain(ColorToken::Running)),
            ("  scanning retry races", TextStyle::plain(ColorToken::Muted)),
            ("", TextStyle::plain(ColorToken::Dim)),
            ("Done", TextStyle::bold(ColorToken::Bright)),
            ("✓ coverage-subagent", TextStyle::plain(ColorToken::Ok)),
            ("  1 open follow-up", TextStyle::plain(ColorToken::Muted)),
            ("", TextStyle::plain(ColorToken::Dim)),
            ("Enter expands findings", TextStyle::plain(ColorToken::Dim)),
        ],
    )?;
    finish(canvas)
}

fn sidebar_mcp() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_sidebar_shell(
        &mut canvas,
        62,
        "MCP peek",
        &[
            ("Attention", TextStyle::bold(ColorToken::Bright)),
            ("● Docs research", TextStyle::plain(ColorToken::Warn)),
            ("  needs sign-in", TextStyle::plain(ColorToken::Muted)),
            ("", TextStyle::plain(ColorToken::Dim)),
            ("Ready", TextStyle::bold(ColorToken::Bright)),
            ("○ Local tools", TextStyle::plain(ColorToken::Ok)),
            ("○ Memory notes", TextStyle::plain(ColorToken::Ok)),
            ("", TextStyle::plain(ColorToken::Dim)),
            ("Open full /mcp view", TextStyle::plain(ColorToken::Dim)),
        ],
    )?;
    finish(canvas)
}

fn sidebar_queue() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_sidebar_shell(
        &mut canvas,
        62,
        "Queue",
        &[
            ("3 waiting", TextStyle::bold(ColorToken::Bright)),
            ("› 1 floor integration", TextStyle::bold(ColorToken::Accent)),
            ("  2 document jitter", TextStyle::plain(ColorToken::Text)),
            ("  3 rate-limit note", TextStyle::plain(ColorToken::Text)),
            ("", TextStyle::plain(ColorToken::Dim)),
            ("Enter · send now", TextStyle::plain(ColorToken::Muted)),
            ("d remove · j/k reorder", TextStyle::plain(ColorToken::Dim)),
        ],
    )?;
    finish(canvas)
}

fn finish(canvas: Canvas) -> Result<Canvas, String> {
    reject_copper_residual(&canvas.to_text())?;
    Ok(canvas)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chrome::components::{paint_default_prompt, paint_quiet_origin};

    #[test]
    fn views_batch_pure_cortex() {
        for id in [
            "status",
            "timeline",
            "mcp",
            "diff-viewer",
            "diff-viewer-staging",
            "help",
            "review",
            "theme-gallery-dark",
            "theme-gallery-light",
            "sidebar-context",
            "sidebar-files",
            "sidebar-lsp",
            "sidebar-todo",
            "sidebar-subagents",
            "sidebar-mcp",
            "sidebar-queue",
        ] {
            let text = render(id)
                .unwrap_or_else(|e| panic!("{id}: {e}"))
                .to_text();
            assert!(!text.contains("Claude Opus"), "{id}");
            assert!(!text.contains("How Code should work"), "{id}");
            assert!(!text.contains("Cortex Code"), "{id}");
            assert!(!text.contains("Work mode"), "{id}");
            assert!(
                text.contains("Cortex")
                    || text.contains("CortexNight")
                    || text.contains("Shift+Tab"),
                "{id}"
            );
        }
        let light = render("theme-gallery-light").unwrap();
        // Day theme still paints Cortex labels, never copper.
        assert!(
            light.to_text().contains("CortexDay")
                || light.to_text().contains("#7d4bc6")
                || light.to_text().contains("light")
        );
    }

    #[test]
    fn quiet_origin_helper_compiles_path() {
        let mut c = Canvas::new(Theme::CortexNight);
        paint_quiet_origin(&mut c, "hi", "partial").unwrap();
        paint_default_prompt(&mut c, 33, "").unwrap();
        assert!(c.to_text().contains("Cortex"));
    }
}
