//! Pure permissions & recovery (F5).
//!
//! Shell/file permissions, blocks, turn errors, network/context failures,
//! rate limits, interrupt after [stop], checkpoint undo, and share.

use crate::chrome::canvas::Canvas;
use crate::chrome::components::{
    paint_assistant, paint_default_prompt, paint_default_putty, paint_demo_top_bar, paint_dock_frame,
    paint_quiet_origin, paint_tool_elbow, paint_tool_header, paint_user_turn,
    reject_copper_residual, MODEL_LABEL,
};
use crate::chrome::tokens::{ColorToken, TextStyle, Theme};

pub fn render(id: &str) -> Result<Canvas, String> {
    match id {
        "permission-shell" => permission_shell(),
        "permission-file-edit" => permission_file_edit(),
        "permission-deny-reason" => permission_deny_reason(),
        "blocked-command" => blocked_command(),
        "turn-error" => turn_error(),
        "error-network" => error_network(),
        "error-context" => error_context(),
        "rate-limit" => rate_limit(),
        "interrupt" => interrupt(),
        "checkpoint-undo" => checkpoint_undo(),
        "share" => share(),
        other => Err(format!("unknown permissions pure-Cortex id: {other}")),
    }
}

fn permission_shell() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_quiet_origin(
        &mut canvas,
        "Tighten the shell path before the next suite run.",
        "",
    )?;
    canvas.write(10, 2, "◆ Cortex", TextStyle::accent())?;
    paint_tool_header(
        &mut canvas,
        12,
        "Shell",
        "cargo test retry -- --nocapture",
        TextStyle::plain(ColorToken::Running),
    )?;
    paint_tool_elbow(
        &mut canvas,
        13,
        "needs permission before running on this host",
        TextStyle::plain(ColorToken::Warn),
    )?;
    paint_dock_frame(
        &mut canvas,
        28,
        9,
        "Allow this command?",
        "Enter allows once · a always · Esc denies",
    )?;
    canvas.write(
        30,
        5,
        "cargo test retry -- --nocapture",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        31,
        5,
        "Working directory · /projects/demo",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        32,
        5,
        "› Allow once",
        TextStyle::bold(ColorToken::Accent),
    )?;
    canvas.write(33, 5, "  Allow for session", TextStyle::plain(ColorToken::Text))?;
    canvas.write(34, 5, "  Deny", TextStyle::plain(ColorToken::Text))?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn permission_file_edit() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_quiet_origin(
        &mut canvas,
        "Apply the retry guard before we merge.",
        "",
    )?;
    canvas.write(10, 2, "◆ Cortex", TextStyle::accent())?;
    paint_tool_header(
        &mut canvas,
        12,
        "Edit",
        "crates/agent/src/retry.rs  +12 / -4",
        TextStyle::plain(ColorToken::Warn),
    )?;
    // Mini-diff preview.
    canvas.write(
        14,
        4,
        "  rng.retry_after_ms = floor.max(250);",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        15,
        4,
        "+ if floor_ms < 250 { return Err(floor); }",
        TextStyle::on(ColorToken::Ok, ColorToken::AddBackground),
    )?;
    canvas.write(
        16,
        4,
        "- // trust caller floor",
        TextStyle::on(ColorToken::Bad, ColorToken::DeleteBackground),
    )?;
    paint_dock_frame(
        &mut canvas,
        28,
        9,
        "Apply this edit?",
        "Enter to apply · Esc to reject · d open full diff",
    )?;
    canvas.write(
        30,
        5,
        "crates/agent/src/retry.rs · hunk 1 of 1",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        32,
        5,
        "› Apply edit",
        TextStyle::bold(ColorToken::Accent),
    )?;
    canvas.write(33, 5, "  Reject edit", TextStyle::plain(ColorToken::Text))?;
    canvas.write(34, 5, "  Open full diff", TextStyle::plain(ColorToken::Text))?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn permission_deny_reason() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_quiet_origin(
        &mut canvas,
        "Request a safer rewrite of the plan step.",
        "",
    )?;
    canvas.write(10, 2, "◆ Cortex", TextStyle::accent())?;
    canvas.write(
        12,
        4,
        "Plan approval · changes requested",
        TextStyle::bold(ColorToken::Warn),
    )?;
    canvas.write(
        13,
        4,
        "The write step still touches production config.",
        TextStyle::plain(ColorToken::Text),
    )?;
    paint_dock_frame(
        &mut canvas,
        28,
        9,
        "Why deny or request changes?",
        "Enter sends comment · Esc keeps plan blocked",
    )?;
    canvas.write(
        30,
        5,
        "› Request changes",
        TextStyle::bold(ColorToken::Accent),
    )?;
    canvas.write(
        31,
        5,
        "  Keep production config read-only in plan",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(33, 5, "  Deny without comment", TextStyle::plain(ColorToken::Text))?;
    canvas.write(34, 5, "  Allow with caution", TextStyle::plain(ColorToken::Text))?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn blocked_command() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_demo_top_bar(&mut canvas)?;
    paint_user_turn(
        &mut canvas,
        3,
        &["Run the redeploy script after tests pass."],
    )?;
    canvas.write(7, 2, "◆ Cortex", TextStyle::accent())?;
    paint_tool_header(
        &mut canvas,
        9,
        "✗ Shell",
        "./scripts/redeploy.sh --prod",
        TextStyle::plain(ColorToken::Bad),
    )?;
    paint_tool_elbow(
        &mut canvas,
        10,
        "blocked · protected path · sandbox refusal",
        TextStyle::plain(ColorToken::Bad),
    )?;
    canvas.write(
        13,
        4,
        "Cortex cannot run production redeploy from this shell.",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        14,
        4,
        "Switch to a reviewed plan or run it outside Cortex.",
        TextStyle::plain(ColorToken::Secondary),
    )?;
    canvas.write(
        17,
        4,
        "› Open safer command variants",
        TextStyle::bold(ColorToken::Accent),
    )?;
    canvas.write(
        18,
        4,
        "  Keep blocked and continue chatting",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        20,
        4,
        "Enter chooses recovery · Esc returns with block visible",
        TextStyle::plain(ColorToken::Dim),
    )?;
    paint_default_prompt(&mut canvas, 33, "")?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn turn_error() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_demo_top_bar(&mut canvas)?;
    paint_user_turn(
        &mut canvas,
        3,
        &["Explain the flush root race in one short note."],
    )?;
    canvas.write(7, 2, "◆ Cortex", TextStyle::accent())?;
    canvas.write(
        8,
        4,
        "Two writers can skip the watched counter when the",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        9,
        4,
        "snapshot freezes mid-batch…",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        12,
        4,
        "Turn failed · provider closed the stream early",
        TextStyle::bold(ColorToken::Bad),
    )?;
    canvas.write(
        13,
        4,
        "Partial answer kept above · draft stays in the jewel",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        16,
        4,
        "› Retry turn",
        TextStyle::bold(ColorToken::Accent),
    )?;
    canvas.write(17, 4, "  Keep partial and continue", TextStyle::plain(ColorToken::Text))?;
    canvas.write(
        18,
        4,
        "  Inspect network evidence",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        20,
        4,
        "Enter retries · Esc leaves the partial answer visible",
        TextStyle::plain(ColorToken::Dim),
    )?;
    paint_default_prompt(&mut canvas, 33, "")?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn error_network() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_demo_top_bar(&mut canvas)?;
    paint_user_turn(
        &mut canvas,
        3,
        &["Continue after the offline blip."],
    )?;
    canvas.write(8, 2, "◆ Cortex", TextStyle::accent())?;
    canvas.write(
        10,
        4,
        "Network error · could not reach xAI",
        TextStyle::bold(ColorToken::Bad),
    )?;
    canvas.write(
        11,
        4,
        "Check the link, then retry once the path is clear.",
        TextStyle::plain(ColorToken::Secondary),
    )?;
    canvas.write(
        13,
        4,
        "Evidence · timeout after 12s · DNS ok · TLS handshake fail",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        16,
        4,
        "› Retry connection",
        TextStyle::bold(ColorToken::Accent),
    )?;
    canvas.write(17, 4, "  Work offline with local tools", TextStyle::plain(ColorToken::Text))?;
    canvas.write(
        19,
        4,
        "Enter retries · draft and origin stay unchanged",
        TextStyle::plain(ColorToken::Dim),
    )?;
    paint_default_prompt(&mut canvas, 33, "")?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn error_context() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_demo_top_bar(&mut canvas)?;
    paint_user_turn(
        &mut canvas,
        3,
        &["Keep summarizing while context is near the ceiling."],
    )?;
    // Context bar near MAX.
    canvas.write(
        9,
        4,
        "Context  ████████████████████░░  948K / 1.0M",
        TextStyle::plain(ColorToken::Warn),
    )?;
    canvas.write(
        11,
        4,
        "Context is nearly full · compact before the next send",
        TextStyle::bold(ColorToken::Warn),
    )?;
    canvas.write(
        12,
        4,
        "Older tool bodies can fold without losing your draft.",
        TextStyle::plain(ColorToken::Secondary),
    )?;
    canvas.write(
        15,
        4,
        "› Compact now",
        TextStyle::bold(ColorToken::Accent),
    )?;
    canvas.write(16, 4, "  Send anyway (may truncate)", TextStyle::plain(ColorToken::Text))?;
    canvas.write(17, 4, "  Open context sidebar", TextStyle::plain(ColorToken::Text))?;
    canvas.write(
        19,
        4,
        "Enter compacts · Esc returns with context bar visible",
        TextStyle::plain(ColorToken::Dim),
    )?;
    paint_default_prompt(&mut canvas, 33, "")?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn rate_limit() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_demo_top_bar(&mut canvas)?;
    paint_user_turn(
        &mut canvas,
        3,
        &["Continue the audit after credits recover."],
    )?;
    canvas.write(
        9,
        4,
        "Rate limit · credit floor reached",
        TextStyle::bold(ColorToken::Bad),
    )?;
    canvas.write(
        10,
        4,
        "Next slot opens in 0m42s · draft and queue stay saved",
        TextStyle::plain(ColorToken::Secondary),
    )?;
    // Credit bar.
    canvas.write(
        12,
        4,
        "Credits  ████████░░░░░░░░░░░░  40% recovered",
        TextStyle::plain(ColorToken::Warn),
    )?;
    canvas.write(
        15,
        4,
        "› Wait and retry automatically",
        TextStyle::bold(ColorToken::Accent),
    )?;
    canvas.write(16, 4, "  Switch to xAI model Fast", TextStyle::plain(ColorToken::Text))?;
    canvas.write(17, 4, "  Work offline until reset", TextStyle::plain(ColorToken::Text))?;
    canvas.write(
        19,
        4,
        "Enter waits · Esc keeps the paused queue visible",
        TextStyle::plain(ColorToken::Dim),
    )?;
    paint_default_prompt(&mut canvas, 33, "")?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn interrupt() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_demo_top_bar(&mut canvas)?;
    paint_user_turn(
        &mut canvas,
        3,
        &["Explain the race in the flush root briefly."],
    )?;
    canvas.write(7, 2, "◆ Cortex", TextStyle::accent())?;
    canvas.write(
        8,
        4,
        "Two writers can skip the watched counter when the",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        9,
        4,
        "snapshot freezes…",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        12,
        4,
        "Interrupted · stream stopped with [stop]",
        TextStyle::bold(ColorToken::Warn),
    )?;
    canvas.write(
        13,
        4,
        "Draft stays in the jewel · partial answer kept above",
        TextStyle::plain(ColorToken::Muted),
    )?;
    // Idle jewel shows preserved draft (not busy turn-status).
    paint_default_prompt(
        &mut canvas,
        33,
        "Continue from the partial note…",
    )?;
    // Still surface what [stop] did.
    canvas.write(
        31,
        4,
        "○ idle after cancel · Ctrl+c or [stop] both preserve draft",
        TextStyle::plain(ColorToken::Dim),
    )?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn checkpoint_undo() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_demo_top_bar(&mut canvas)?;
    paint_user_turn(
        &mut canvas,
        3,
        &["Undo the last edit if the suite went red."],
    )?;
    paint_assistant(
        &mut canvas,
        7,
        &["Last checkpoint restored the retry helper pre-edit."],
    )?;
    canvas.write(
        11,
        4,
        "Rewind available · choose how far to jump",
        TextStyle::bold(ColorToken::Bright),
    )?;
    paint_picker_like(
        &mut canvas,
        13,
        "Checkpoint · before edit",
        "retry.rs hash a1b2… · 0m18s ago",
        true,
    )?;
    paint_picker_like(
        &mut canvas,
        15,
        "Checkpoint · after tests green",
        "suite pass · 1m02s ago",
        false,
    )?;
    canvas.write(
        18,
        4,
        "› Restore selected checkpoint",
        TextStyle::bold(ColorToken::Accent),
    )?;
    canvas.write(19, 4, "  Keep current workspace", TextStyle::plain(ColorToken::Text))?;
    canvas.write(
        21,
        4,
        "Enter restores · Esc leaves files as-is",
        TextStyle::plain(ColorToken::Dim),
    )?;
    paint_default_prompt(&mut canvas, 33, "")?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn share() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_demo_top_bar(&mut canvas)?;
    paint_user_turn(
        &mut canvas,
        3,
        &["Share a redacted snapshot of this final answer."],
    )?;
    paint_assistant(
        &mut canvas,
        7,
        &[
            "Applied the retry guard and covered the failure path.",
            "Follow-up: floor integration case.",
        ],
    )?;
    paint_dock_frame(
        &mut canvas,
        28,
        9,
        "Share conversation?",
        "Enter creates redacted link · Esc cancels",
    )?;
    canvas.write(
        30,
        5,
        "Format · Markdown snapshot",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        31,
        5,
        "Secrets · API keys and tokens redacted",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        33,
        5,
        "› Create share link",
        TextStyle::bold(ColorToken::Accent),
    )?;
    canvas.write(34, 5, "  Export file only", TextStyle::plain(ColorToken::Text))?;
    paint_default_putty(&mut canvas)?;
    // silence unused if MODEL_LABEL only referenced in tests elsewhere
    let _ = MODEL_LABEL;
    finish(canvas)
}

fn paint_picker_like(
    canvas: &mut Canvas,
    row: u16,
    title: &str,
    detail: &str,
    selected: bool,
) -> Result<(), String> {
    if selected {
        canvas.write(
            row,
            4,
            &format!("› {title}"),
            TextStyle::bold(ColorToken::Accent),
        )?;
        canvas.write(row + 1, 6, detail, TextStyle::plain(ColorToken::Muted))?;
    } else {
        canvas.write(
            row,
            4,
            &format!("  {title}"),
            TextStyle::plain(ColorToken::Text),
        )?;
        canvas.write(row + 1, 6, detail, TextStyle::plain(ColorToken::Dim))?;
    }
    Ok(())
}

fn finish(canvas: Canvas) -> Result<Canvas, String> {
    reject_copper_residual(&canvas.to_text())?;
    Ok(canvas)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chrome::components::paint_turn_status_busy;

    #[test]
    fn permissions_batch_pure_cortex() {
        for id in [
            "permission-shell",
            "permission-file-edit",
            "permission-deny-reason",
            "blocked-command",
            "turn-error",
            "error-network",
            "error-context",
            "rate-limit",
            "interrupt",
            "checkpoint-undo",
            "share",
        ] {
            let text = render(id)
                .unwrap_or_else(|e| panic!("{id}: {e}"))
                .to_text();
            assert!(!text.contains("Claude Opus"), "{id}");
            assert!(!text.contains("Cortex Code"), "{id}");
            assert!(!text.contains("Work mode"), "{id}");
            assert!(text.contains("Shift+Tab:mode") || text.contains("Esc"), "{id}");
        }
        let interrupt = render("interrupt").unwrap().to_text();
        assert!(interrupt.contains("[stop]"));
        assert!(interrupt.contains("Cortex"));
        // busy paint still available as unit helper
        let mut c = Canvas::new(Theme::CortexNight);
        paint_turn_status_busy(&mut c, 30, "Responding", "1.0s", "1k").unwrap();
        assert!(c.to_text().contains("[stop]"));
    }
}
