//! Pure tools & tasks in transcript (M2 pure-structure tools language).
//!
//! Source law: `scrollback/blocks/tool/*` + mission `library/pure-structure-law.md`
//! + `library/fidelity-audit-auth-tools.md`.
//!
//! Product headers:
//! - Execute: `$ {command}` / `Run …` (never bare `Shell  cmd`)
//! - Search: `Search "pattern" (N matches…)` parenthetical
//! - Read/Edit: `Read {path}` / `Edit {path}` (+ optional range / diffstat)
//! - No `◆ Cortex` nameplates; no gallery monologue on tool frames.

use crate::chrome::canvas::Canvas;
use crate::chrome::components::{
    paint_assistant, paint_default_prompt, paint_default_putty, paint_demo_top_bar,
    paint_edit_header, paint_execute_run_header, paint_execute_shell_header, paint_read_header,
    paint_search_header, paint_tool_activity, paint_tool_body, paint_turn_status_busy,
    paint_user_turn, reject_copper_residual, reject_structure_residual,
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
    // Product tool inventory under the user turn — no agent nameplate.
    paint_read_header(
        &mut canvas,
        7,
        "crates/agent/src/retry.rs",
        None,
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_search_header(
        &mut canvas,
        8,
        "rate.?limit",
        "(3 matches)",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_execute_shell_header(
        &mut canvas,
        9,
        "cargo test retry -- --nocapture",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_edit_header(
        &mut canvas,
        10,
        "crates/agent/src/retry.rs",
        Some("+12/-4"),
        TextStyle::plain(ColorToken::Muted),
    )?;
    // Compact success copy for the edit line is enough; no gallery essay.
    paint_tool_body(
        &mut canvas,
        12,
        "floor.max raised to 250ms · tests green",
        TextStyle::plain(ColorToken::Secondary),
    )?;
    paint_default_prompt(&mut canvas, 33, "")?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn tool_running() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    common_user(&mut canvas)?;
    paint_execute_shell_header(
        &mut canvas,
        7,
        "cargo test retry -- --nocapture",
        TextStyle::plain(ColorToken::Running),
    )?;
    paint_tool_body(
        &mut canvas,
        8,
        "running 0.2s · waiting on subshell",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_turn_status_busy(
        &mut canvas,
        31,
        "Run command 0.2s · 0 queued",
        "12s",
        "4.8k",
    )?;
    paint_default_prompt(&mut canvas, 33, "")?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn tool_success() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    common_user(&mut canvas)?;
    // Product success: `$ command` obtained titles + Edit path with diffstat.
    // No `✓ Shell` invent; duration soft under-body when useful.
    paint_execute_shell_header(
        &mut canvas,
        7,
        "cargo test retry",
        TextStyle::plain(ColorToken::Ok),
    )?;
    paint_tool_body(
        &mut canvas,
        8,
        "exit 0 · 24 tests passed · 1.8s",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_edit_header(
        &mut canvas,
        10,
        "crates/agent/src/retry.rs",
        Some("+12/-4"),
        TextStyle::plain(ColorToken::Ok),
    )?;
    paint_assistant(
        &mut canvas,
        13,
        &["Retry helper is covered. Ready for the rate-limit floor case."],
    )?;
    paint_default_prompt(&mut canvas, 33, "")?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn tool_success_expanded() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    common_user(&mut canvas)?;
    paint_execute_shell_header(
        &mut canvas,
        7,
        "cargo test retry",
        TextStyle::plain(ColorToken::Ok),
    )?;
    let out = TextStyle::on(ColorToken::Text, ColorToken::CodeBg);
    canvas.fill_row_bg(9, 4, 90, out)?;
    canvas.fill_row_bg(10, 4, 90, out)?;
    canvas.fill_row_bg(11, 4, 90, out)?;
    canvas.fill_row_bg(12, 4, 90, out)?;
    canvas.write(9, 6, "running 24 tests", out)?;
    canvas.write(10, 6, "test retry_ceiling ... ok", out)?;
    canvas.write(11, 6, "test retry_jitter ... ok", out)?;
    canvas.write(12, 6, "test result: ok. 24 passed; 0 failed", out)?;
    paint_tool_body(
        &mut canvas,
        14,
        "Esc collapses expanded tool output",
        TextStyle::plain(ColorToken::Dim),
    )?;
    paint_default_prompt(&mut canvas, 33, "")?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn tool_failed() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    common_user(&mut canvas)?;
    // Failed execute keeps the `$` header; error hangs as body (source failedExecute).
    paint_execute_shell_header(
        &mut canvas,
        7,
        "cargo test rate_limit_floors",
        TextStyle::plain(ColorToken::Bad),
    )?;
    paint_tool_body(
        &mut canvas,
        8,
        "exit 101 · assertion failed: floor_ms >= 250",
        TextStyle::plain(ColorToken::Bad),
    )?;
    canvas.write(
        11,
        4,
        "› Retry command",
        TextStyle::bold(ColorToken::Accent),
    )?;
    canvas.write(
        12,
        4,
        "  Edit and rerun",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        13,
        4,
        "  Skip and continue",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        15,
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
    // Task blocks stay as product Task rows (subagent dispatch, not Mission invent).
    paint_tool_activity(
        &mut canvas,
        7,
        "Task audit-subagent influences_retry",
        TextStyle::plain(ColorToken::Info),
    )?;
    paint_tool_body(
        &mut canvas,
        8,
        "background · 4 findings",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_tool_activity(
        &mut canvas,
        10,
        "Task coverage-subagent floors",
        TextStyle::plain(ColorToken::Info),
    )?;
    paint_tool_body(
        &mut canvas,
        11,
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
    paint_tool_activity(
        &mut canvas,
        7,
        "Task audit-subagent influences_retry",
        TextStyle::plain(ColorToken::Info),
    )?;
    canvas.write(9, 4, "Findings", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(
        10,
        4,
        "• jitter helper loses order under concurrent floors",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        11,
        4,
        "• ceiling is not reset when workspace switches",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        12,
        4,
        "• path chip missed absolute /tmp fixtures",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        13,
        4,
        "• subagent log redacts API-shaped substrings",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        15,
        4,
        "Esc to collapse · Enter opens first finding",
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
    // Bash user mode: product Label header `Run (user) …` + `$ command` body.
    paint_execute_run_header(
        &mut canvas,
        7,
        "(user) cargo test --package agent -- retry",
        TextStyle::plain(ColorToken::Warn),
    )?;
    let out = TextStyle::on(ColorToken::Text, ColorToken::CodeBg);
    canvas.fill_row_bg(9, 4, 92, out)?;
    canvas.fill_row_bg(10, 4, 92, out)?;
    canvas.fill_row_bg(11, 4, 92, out)?;
    canvas.write(9, 6, "$ cargo test --package agent -- retry", out)?;
    canvas.write(10, 6, "running 8 tests", out)?;
    canvas.write(11, 6, "test result: ok. 8 passed; 0 failed", out)?;
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
    // Product todo list rows under authenticated shell (no nameplate).
    canvas.write(7, 4, "Todos", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(
        9,
        4,
        "✓ Pin fixture ids in the flaky suite",
        TextStyle::plain(ColorToken::Ok),
    )?;
    canvas.write(
        10,
        4,
        "› Rebuild group counter before re-run",
        TextStyle::bold(ColorToken::Accent),
    )?;
    canvas.write(
        11,
        4,
        "○ Add rate-limit floor integration case",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        12,
        4,
        "○ Document recovery in docs/recovery.md",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        14,
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
    // Compact is session recap chrome — product-ish brevity, no essay monologue.
    canvas.write(
        4,
        2,
        "Compact · kept retry path + rate-limit plan",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        6,
        4,
        "Kept: retry helper, rate-limit plan, failing floors.",
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
        "Context 184k / 1.0M after compact",
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
    paint_edit_header(
        &mut canvas,
        7,
        "crates/agent/src/retry.rs",
        Some("+1/-1"),
        TextStyle::plain(ColorToken::Warn),
    )?;
    // Mini-diff rows with add/delete backgrounds (product edit expansion).
    let del = TextStyle::on(ColorToken::Text, ColorToken::DeleteBackground);
    let add = TextStyle::on(ColorToken::Text, ColorToken::AddBackground);
    canvas.write(
        9,
        4,
        "  @@ -40,6 +40,10 @@ fn ceiling_ms(...)",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.fill_row_bg(10, 4, 90, del)?;
    canvas.write(10, 4, "-    floor.max(50)", del)?;
    canvas.fill_row_bg(11, 4, 90, add)?;
    canvas.write(11, 4, "+    floor.max(250)", add)?;
    canvas.fill_row_bg(12, 4, 90, add)?;
    canvas.write(12, 4, "+    // rate-limit floors never go under 250ms", add)?;
    canvas.write(
        15,
        4,
        "› Apply edit",
        TextStyle::bold(ColorToken::Accent),
    )?;
    canvas.write(16, 4, "  Reject", TextStyle::plain(ColorToken::Text))?;
    canvas.write(
        18,
        4,
        "Enter to apply · Esc to reject and keep draft",
        TextStyle::plain(ColorToken::Dim),
    )?;
    paint_default_prompt(&mut canvas, 33, "")?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn finish(canvas: Canvas) -> Result<Canvas, String> {
    let text = canvas.to_text();
    reject_copper_residual(&text)?;
    reject_structure_residual(&text)?;
    reject_tool_language_residual(&text)?;
    Ok(canvas)
}

/// VAL-V3-STRUCT-007 tool verb language bans (Shell invent / nameplates / gallery essay).
pub fn reject_tool_language_residual(text: &str) -> Result<(), String> {
    // Bare Shell invent primary header (gallery fingerprint). Allow word Shell in
    // prose like "shell path" but ban `Shell  ` / `✓ Shell` / `✗ Shell` tool titles.
    for needle in ["Shell  ", "✓ Shell", "✗ Shell", "bash  "] {
        if text.contains(needle) {
            return Err(format!(
                "tool language residual refused: Shell invent {needle:?}"
            ));
        }
    }
    // Middot match count galley form, not product parenthetical summary.
    if text.contains(" ·  3 matches")
        || text.contains(" · 3 matches")
        || text.contains("· 3 matches")
    {
        return Err(
            "tool language residual refused: Search middot matches invent".to_owned(),
        );
    }
    if text.contains("Mixed tool inventory")
        || text.contains("expand any row for details")
        || text.contains("Enter to expand output")
    {
        return Err("tool language residual refused: gallery monologue".to_owned());
    }
    if text.contains("◆ Cortex") {
        return Err("tool language residual refused: Cortex nameplate".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOOL_IDS: &[&str] = &[
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
    ];

    #[test]
    fn tools_batch_product_language_and_shell_chrome() {
        for id in TOOL_IDS {
            let canvas = render(id).unwrap_or_else(|e| panic!("{id}: {e}"));
            let text = canvas.to_text();
            reject_copper_residual(&text).unwrap_or_else(|e| panic!("{id}: {e}"));
            reject_structure_residual(&text).unwrap_or_else(|e| panic!("{id}: {e}"));
            reject_tool_language_residual(&text).unwrap_or_else(|e| panic!("{id}: {e}"));
            assert_eq!(text.lines().count(), 40, "{id}");
            assert!(!text.contains("Cortex"), "{id}");
            assert!(!text.contains("Claude Opus"), "{id}");
            assert!(!text.contains("◆ Cortex"), "{id} nameplate ban");
            assert!(!text.contains("Shell  "), "{id} Shell invent ban");
            assert!(!text.contains("✓ Shell"), "{id}");
            assert!(!text.contains("✗ Shell"), "{id}");
            // Authenticated shell chrome from M1 (VAL-V3-CROSS-026).
            assert!(
                text.contains("9.5K / 500K") || text.contains("K / "),
                "{id} missing context budget"
            );
            assert!(
                text.contains("Shift+Tab:mode | Ctrl+c:cancel"),
                "{id} putty pipes"
            );
            assert!(text.contains("Cortex (high)"), "{id} right jewel chip");
            if *id == "tool-running" {
                assert!(text.contains("[stop]"), "busy frame needs [stop]");
                assert!(text.contains('⠧'));
                assert!(text.contains("$ cargo test retry"));
                assert!(text.contains("Run command 0.2s"));
            }
            if *id == "tool-calls" {
                assert!(text.contains("Read crates/agent/src/retry.rs"));
                assert!(text.contains("Search \"rate.?limit\" (3 matches)"));
                assert!(text.contains("$ cargo test retry -- --nocapture"));
                assert!(text.contains("Edit crates/agent/src/retry.rs +12/-4"));
                assert!(!text.contains("Mixed tool inventory"));
            }
            if *id == "tool-success" {
                assert!(text.contains("$ cargo test retry"));
                assert!(text.contains("24 tests passed"));
                assert!(text.contains("Edit crates/agent/src/retry.rs +12/-4"));
                assert!(!text.contains("✓ Shell"));
            }
            if *id == "shell" {
                assert!(text.contains("Run (user)"));
                assert!(text.contains("$ cargo test --package agent -- retry"));
                assert!(!text.contains("bash  "));
            }
            if *id == "diff-edit" {
                assert!(text.contains("Apply edit"));
                assert!(text.contains("Edit crates/agent/src/retry.rs"));
                assert!(text.contains("+    floor.max(250)"));
            }
            if *id == "tool-failed" {
                assert!(text.contains("$ cargo test rate_limit_floors"));
                assert!(text.contains("exit 101"));
                assert!(!text.contains("✗ Shell"));
            }
        }
    }

    #[test]
    fn reject_tool_language_bans_shell_invent() {
        assert!(reject_tool_language_residual("Shell  cargo test").is_err());
        assert!(reject_tool_language_residual("✓ Shell cargo test").is_err());
        assert!(reject_tool_language_residual("Search foo · 3 matches").is_err());
        assert!(reject_tool_language_residual("◆ Cortex\n$ cargo test").is_err());
        assert!(reject_tool_language_residual("$ cargo test\nSearch \"x\" (3 matches)").is_ok());
    }
}
