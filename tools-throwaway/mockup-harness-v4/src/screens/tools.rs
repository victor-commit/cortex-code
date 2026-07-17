//! Pure tools & tasks in transcript (F3).

use crate::chrome::canvas::Canvas;
use crate::chrome::components::{
    paint_assistant, paint_default_prompt, paint_default_putty, paint_demo_top_bar,
    paint_tool_elbow, paint_tool_header, paint_turn_status_busy, paint_user_turn,
    reject_copper_residual,
};
use crate::chrome::tokens::{ColorToken, TextStyle, Theme};

pub fn render(id: &str) -> Result<Canvas, String> {
    match id {
        "tool-calls" => tool_calls(),
        "tool-running" => tool_running(),
        "tool-success" => tool_success(),
        "tool-success-expanded" => tool_success_expanded(),
        "tool-failed" => tool_failed(),
        "tool-task" => tool_task(),
        "tool-task-expanded" => tool_task_expanded(),
        "shell" => shell(),
        "todo-list" => todo_list(),
        "compact" => compact(),
        "diff-edit" => diff_edit(),
        other => Err(format!("unknown tools pure-Cortex id: {other}")),
    }
}

fn common_user(canvas: &mut Canvas) -> Result<(), String> {
    paint_demo_top_bar(canvas)?;
    paint_user_turn(
        canvas,
        3,
        &["Inspect the retry helper and tighten the shell path."],
    )
}

fn tool_calls() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    common_user(&mut canvas)?;
    canvas.write(7, 2, "◆ Cortex", TextStyle::accent())?;
    paint_tool_header(
        &mut canvas,
        9,
        "Read",
        "crates/agent/src/retry.rs",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_tool_header(
        &mut canvas,
        10,
        "Search",
        "rate.?limit  ·  3 matches",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_tool_header(
        &mut canvas,
        11,
        "Shell",
        "cargo test retry -- --nocapture",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_tool_header(
        &mut canvas,
        12,
        "Edit",
        "crates/agent/src/retry.rs +12/-4",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        14,
        4,
        "Mixed tool inventory · expand any row for details.",
        TextStyle::plain(ColorToken::Secondary),
    )?;
    paint_default_prompt(&mut canvas, 33, "")?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn tool_running() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    common_user(&mut canvas)?;
    canvas.write(7, 2, "◆ Cortex", TextStyle::accent())?;
    paint_tool_header(
        &mut canvas,
        9,
        "Shell",
        "cargo test retry -- --nocapture",
        TextStyle::plain(ColorToken::Running),
    )?;
    paint_tool_elbow(
        &mut canvas,
        10,
        "running · 0.2s · waiting on subshell",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_turn_status_busy(
        &mut canvas,
        31,
        "Run command 0.2s · 0 queued",
        "0m12s",
        "4.8k",
    )?;
    paint_default_prompt(&mut canvas, 33, "")?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn tool_success() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    common_user(&mut canvas)?;
    canvas.write(7, 2, "◆ Cortex", TextStyle::accent())?;
    paint_tool_header(
        &mut canvas,
        9,
        "✓ Shell",
        "cargo test retry  ·  1.8s",
        TextStyle::plain(ColorToken::Ok),
    )?;
    paint_tool_elbow(
        &mut canvas,
        10,
        "exit 0 · 24 tests passed",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_tool_header(
        &mut canvas,
        12,
        "✓ Edit",
        "crates/agent/src/retry.rs",
        TextStyle::plain(ColorToken::Ok),
    )?;
    paint_tool_elbow(
        &mut canvas,
        13,
        "+12 / -4  ·  Enter to expand output",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_assistant(
        &mut canvas,
        16,
        &["Retry helper is covered. Ready for the rate-limit floor case."],
    )?;
    paint_default_prompt(&mut canvas, 33, "")?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn tool_success_expanded() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    common_user(&mut canvas)?;
    canvas.write(7, 2, "◆ Cortex", TextStyle::accent())?;
    paint_tool_header(
        &mut canvas,
        9,
        "✓ Shell",
        "cargo test retry  ·  1.8s",
        TextStyle::plain(ColorToken::Ok),
    )?;
    let out = TextStyle::on(ColorToken::Text, ColorToken::CodeBg);
    canvas.fill_row_bg(10, 4, 90, out)?;
    canvas.fill_row_bg(11, 4, 90, out)?;
    canvas.fill_row_bg(12, 4, 90, out)?;
    canvas.fill_row_bg(13, 4, 90, out)?;
    canvas.write(10, 6, "running 24 tests", out)?;
    canvas.write(11, 6, "test retry_ceiling ... ok", out)?;
    canvas.write(12, 6, "test retry_jitter ... ok", out)?;
    canvas.write(13, 6, "test result: ok. 24 passed; 0 failed", out)?;
    canvas.write(
        15,
        4,
        "Esc to collapse expanded tool output",
        TextStyle::plain(ColorToken::Dim),
    )?;
    paint_default_prompt(&mut canvas, 33, "")?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn tool_failed() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    common_user(&mut canvas)?;
    canvas.write(7, 2, "◆ Cortex", TextStyle::accent())?;
    paint_tool_header(
        &mut canvas,
        9,
        "✗ Shell",
        "cargo test rate_limit_floors",
        TextStyle::plain(ColorToken::Bad),
    )?;
    paint_tool_elbow(
        &mut canvas,
        10,
        "exit 101 · assertion failed: floor_ms >= 250",
        TextStyle::plain(ColorToken::Bad),
    )?;
    canvas.write(
        13,
        4,
        "› Retry command",
        TextStyle::bold(ColorToken::Accent),
    )?;
    canvas.write(
        14,
        4,
        "  Edit and rerun",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        15,
        4,
        "  Skip and continue",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        17,
        4,
        "Enter to retry · Esc keeps failure visible",
        TextStyle::plain(ColorToken::Dim),
    )?;
    paint_default_prompt(&mut canvas, 33, "")?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn tool_task() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    common_user(&mut canvas)?;
    canvas.write(7, 2, "◆ Cortex", TextStyle::accent())?;
    paint_tool_header(
        &mut canvas,
        9,
        "Task",
        "audit-subagent · influences_retry",
        TextStyle::plain(ColorToken::Info),
    )?;
    paint_tool_elbow(
        &mut canvas,
        10,
        "background · 4 findings · Expand for details",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_tool_header(
        &mut canvas,
        12,
        "Task",
        "coverage-subagent · floors",
        TextStyle::plain(ColorToken::Info),
    )?;
    paint_tool_elbow(
        &mut canvas,
        13,
        "done · 1 open follow-up",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_default_prompt(&mut canvas, 33, "")?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn tool_task_expanded() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    common_user(&mut canvas)?;
    canvas.write(7, 2, "◆ Cortex", TextStyle::accent())?;
    paint_tool_header(
        &mut canvas,
        9,
        "Task",
        "audit-subagent · influences_retry  ·  expanded",
        TextStyle::plain(ColorToken::Info),
    )?;
    canvas.write(
        11,
        4,
        "Findings",
        TextStyle::bold(ColorToken::Bright),
    )?;
    canvas.write(
        12,
        4,
        "• jitter helper loses order under concurrent floors",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        13,
        4,
        "• ceiling is not reset when workspace switches",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        14,
        4,
        "• path chip missed absolute /tmp fixtures",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        15,
        4,
        "• subagent log redacts API-shaped substrings",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        17,
        4,
        "Esc to collapse · Enter to open first finding",
        TextStyle::plain(ColorToken::Dim),
    )?;
    paint_default_prompt(&mut canvas, 33, "")?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn shell() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_demo_top_bar(&mut canvas)?;
    paint_user_turn(
        &mut canvas,
        3,
        &["! cargo test --package agent -- retry"],
    )?;
    canvas.write(7, 2, "◆ Cortex", TextStyle::accent())?;
    paint_tool_header(
        &mut canvas,
        9,
        "bash",
        "cargo test --package agent -- retry",
        TextStyle::plain(ColorToken::Warn),
    )?;
    let out = TextStyle::on(ColorToken::Text, ColorToken::CodeBg);
    canvas.fill_row_bg(10, 4, 92, out)?;
    canvas.fill_row_bg(11, 4, 92, out)?;
    canvas.fill_row_bg(12, 4, 92, out)?;
    canvas.write(10, 6, "$ cargo test --package agent -- retry", out)?;
    canvas.write(11, 6, "running 8 tests", out)?;
    canvas.write(12, 6, "test result: ok. 8 passed; 0 failed", out)?;
    paint_default_prompt(&mut canvas, 33, "")?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn todo_list() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_demo_top_bar(&mut canvas)?;
    paint_user_turn(
        &mut canvas,
        3,
        &["Track the remaining recovery steps as todos."],
    )?;
    canvas.write(7, 2, "◆ Cortex", TextStyle::accent())?;
    canvas.write(9, 4, "Todos", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(
        11,
        4,
        "✓ Pin fixture ids in the flaky suite",
        TextStyle::plain(ColorToken::Ok),
    )?;
    canvas.write(
        12,
        4,
        "› Rebuild group counter before re-run",
        TextStyle::bold(ColorToken::Accent),
    )?;
    canvas.write(
        13,
        4,
        "○ Add rate-limit floor integration case",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        14,
        4,
        "○ Document recovery in docs/recovery.md",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        16,
        4,
        "2/4 complete · Enter toggles selected row",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_default_prompt(&mut canvas, 33, "")?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn compact() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_demo_top_bar(&mut canvas)?;
    canvas.write(
        4,
        2,
        "Compact recap · auto-compact after long tool stream",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        6,
        4,
        "Kept: retry helper + rate-limit plan + failing floors.",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        7,
        4,
        "Dropped: intermediate search spills and partial shells.",
        TextStyle::plain(ColorToken::Secondary),
    )?;
    canvas.write(
        9,
        4,
        "Context  184k / 1.0M  after compact",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_user_turn(
        &mut canvas,
        12,
        &["Continue from the recap: finish the floor case."],
    )?;
    paint_assistant(
        &mut canvas,
        16,
        &["Picking up from the recap. Opening floors next."],
    )?;
    paint_default_prompt(&mut canvas, 33, "")?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn diff_edit() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    common_user(&mut canvas)?;
    canvas.write(7, 2, "◆ Cortex", TextStyle::accent())?;
    paint_tool_header(
        &mut canvas,
        9,
        "Edit",
        "crates/agent/src/retry.rs  ·  pending approval",
        TextStyle::plain(ColorToken::Warn),
    )?;
    // Mini-diff rows with add/delete backgrounds.
    let del = TextStyle::on(ColorToken::Text, ColorToken::DeleteBackground);
    let add = TextStyle::on(ColorToken::Text, ColorToken::AddBackground);
    canvas.write(11, 4, "  @@ -40,6 +40,10 @@ fn ceiling_ms(...)", TextStyle::plain(ColorToken::Muted))?;
    canvas.fill_row_bg(12, 4, 90, del)?;
    canvas.write(12, 4, "-    floor.max(50)", del)?;
    canvas.fill_row_bg(13, 4, 90, add)?;
    canvas.write(13, 4, "+    floor.max(250)", add)?;
    canvas.fill_row_bg(14, 4, 90, add)?;
    canvas.write(14, 4, "+    // rate-limit floors never go under 250ms", add)?;
    canvas.write(
        17,
        4,
        "› Apply edit",
        TextStyle::bold(ColorToken::Accent),
    )?;
    canvas.write(18, 4, "  Reject", TextStyle::plain(ColorToken::Text))?;
    canvas.write(
        20,
        4,
        "Enter to apply · Esc to reject and keep draft",
        TextStyle::plain(ColorToken::Dim),
    )?;
    paint_default_prompt(&mut canvas, 33, "")?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn finish(canvas: Canvas) -> Result<Canvas, String> {
    reject_copper_residual(&canvas.to_text())?;
    Ok(canvas)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_batch_pure_cortex_and_stop_on_running() {
        for id in [
            "tool-calls",
            "tool-running",
            "tool-success",
            "tool-success-expanded",
            "tool-failed",
            "tool-task",
            "tool-task-expanded",
            "shell",
            "todo-list",
            "compact",
            "diff-edit",
        ] {
            let canvas = render(id).unwrap_or_else(|e| panic!("{id}: {e}"));
            let text = canvas.to_text();
            reject_copper_residual(&text).unwrap_or_else(|e| panic!("{id}: {e}"));
            assert_eq!(text.lines().count(), 40, "{id}");
            assert!(!text.contains("Cortex"), "{id}");
            assert!(!text.contains("Claude Opus"), "{id}");
            if id == "tool-running" {
                assert!(text.contains("[stop]"), "busy frame needs [stop]");
                assert!(text.contains('⠧'));
            }
            if id == "diff-edit" {
                assert!(text.contains("Apply edit"));
                assert!(text.contains("+    floor.max(250)"));
            }
        }
    }
}
