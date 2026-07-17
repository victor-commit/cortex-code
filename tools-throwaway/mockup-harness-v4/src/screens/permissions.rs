//! Pure permissions & recovery (M2 pure-structure permission docks).
//!
//! Source law: `/tmp/grok-build-ref/.../views/permission_view.rs`,
//! `.../permission/prompter.rs`, `.../acp_handler/permissions.rs`,
//! mission `library/pure-structure-law.md` + `library/fidelity-audit-auth-tools.md`.
//!
//! Permission takeover vocabulary (product):
//! - Title: `Allow \`bin\`?` / `Allow Edit to {path}?` (or tool description)
//! - Command display of the raw shell/edit target under the title
//! - Numbered radio options: `1 (●) Yes, proceed` / Always-allow scope /
//!   `No, and tell Cortex what to do differently` (RejectOnce)
//! - Freeform RejectOnce preview: `N [x] ❯ <reason>`
//! - No `◆ Cortex` nameplates, no `Shell  cmd`, no Cortex dock essays
//! - Authenticated shell chrome: cwd + budget, putty pipes (M1 inherit)

use crate::chrome::canvas::Canvas;
use crate::chrome::components::{
    paint_assistant, paint_default_prompt, paint_default_putty, paint_demo_top_bar, paint_dock_frame,
    paint_edit_header, paint_execute_shell_header, paint_quiet_origin, paint_tool_body,
    paint_user_turn, reject_copper_residual, reject_structure_residual, MODEL_LABEL,
};
use crate::chrome::tokens::{ColorToken, TextStyle, Theme};

/// Stable product RejectOnce label (prompter `REJECT_ONCE_LABEL`).
const REJECT_ONCE: &str = "No, and tell Cortex what to do differently";

/// Stable always-approve enable option (prompter `ENABLE_ALWAYS_APPROVE_LABEL`).
const ENABLE_ALWAYS: &str = "Yes, and don't ask again for anything (always-approve mode)";

/// Edit-session Always allow (prompter ALLOW_EDITS_SESSION label).
const ALLOW_EDITS_SESSION: &str = "Yes, allow all edits during this session";

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

/// Shell permission takeover — permission_view geometry + product labels.
fn permission_shell() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    // Authenticated origin shell: cwd + budget + bare user ❯ (no You / ◆ Cortex).
    paint_quiet_origin(
        &mut canvas,
        "Tighten the shell path before the next suite run.",
        "",
    )?;
    // Pending execute surface in transcript (product `$` form, not Shell invent).
    paint_execute_shell_header(
        &mut canvas,
        7,
        "cargo test retry -- --nocapture",
        TextStyle::plain(ColorToken::Running),
    )?;
    paint_tool_body(
        &mut canvas,
        8,
        "awaiting approval",
        TextStyle::plain(ColorToken::Warn),
    )?;

    // permission_view takeover dock replaces the gem / prompt area.
    paint_dock_frame(&mut canvas, 24, 13, "", "")?;
    // Title: product Allow `bin`? form (description empty → Allow `{first token}`?).
    canvas.write(
        25,
        5,
        "Allow `cargo`?",
        TextStyle::bold(ColorToken::Bright),
    )?;
    // Bash command display under the title (permission_view chrome).
    canvas.write(
        27,
        5,
        "cargo test retry -- --nocapture",
        TextStyle::plain(ColorToken::Text),
    )?;
    // Scope hint (product uses ← →; pure-glyph allowlist has → only, use ASCII pair).
    canvas.write(
        28,
        5,
        "< > to choose permission scope",
        TextStyle::plain(ColorToken::Dim),
    )?;
    // Numbered radio options (permission_view: 1 (●) / 2 (○) / …).
    // Primary always-approve enable + Yes proceed + RejectOnce.
    paint_permission_option(
        &mut canvas,
        30,
        1,
        true,
        "Yes, proceed",
    )?;
    paint_permission_option(
        &mut canvas,
        31,
        2,
        false,
        "Always allow: cargo test",
    )?;
    paint_permission_option(&mut canvas, 32, 3, false, ENABLE_ALWAYS)?;
    paint_permission_option(&mut canvas, 33, 4, false, REJECT_ONCE)?;
    canvas.write(
        35,
        5,
        "1-4 select · Enter confirms · Esc cancels",
        TextStyle::plain(ColorToken::Dim),
    )?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

/// File-edit permission takeover with mini-diff disclosure.
fn permission_file_edit() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_quiet_origin(
        &mut canvas,
        "Apply the retry guard before we merge.",
        "",
    )?;
    paint_edit_header(
        &mut canvas,
        7,
        "crates/agent/src/retry.rs",
        Some("+12/-4"),
        TextStyle::plain(ColorToken::Warn),
    )?;
    // Mini unified preview above the dock (focused disclosure).
    canvas.write(
        9,
        4,
        "  rng.retry_after_ms = floor.max(250);",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        10,
        4,
        "+ if floor_ms < 250 { return Err(floor); }",
        TextStyle::on(ColorToken::Ok, ColorToken::AddBackground),
    )?;
    canvas.write(
        11,
        4,
        "- // trust caller floor",
        TextStyle::on(ColorToken::Bad, ColorToken::DeleteBackground),
    )?;

    paint_dock_frame(&mut canvas, 24, 13, "", "")?;
    // Title: product `Allow Edit to {path}?`.
    canvas.write(
        25,
        5,
        "Allow Edit to crates/agent/src/retry.rs?",
        TextStyle::bold(ColorToken::Bright),
    )?;
    canvas.write(
        27,
        5,
        "hunk 1 of 1 · +12 / -4",
        TextStyle::plain(ColorToken::Muted),
    )?;
    // Product edit options order: Yes primary, allow-all-edits session, RejectOnce.
    paint_permission_option(&mut canvas, 29, 1, true, "Yes")?;
    paint_permission_option(&mut canvas, 30, 2, false, ALLOW_EDITS_SESSION)?;
    paint_permission_option(&mut canvas, 31, 3, false, REJECT_ONCE)?;
    canvas.write(
        33,
        5,
        "1-3 select · Enter confirms · Esc cancels · d open full diff",
        TextStyle::plain(ColorToken::Dim),
    )?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

/// RejectOnce freeform follow-up (permission_view FollowupInput mode).
fn permission_deny_reason() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_quiet_origin(
        &mut canvas,
        "Tighten the shell path before the next suite run.",
        "",
    )?;
    // Still show the denied target so the follow-up stays bound.
    paint_execute_shell_header(
        &mut canvas,
        7,
        "cargo test retry -- --nocapture",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_tool_body(
        &mut canvas,
        8,
        "denied · tell Cortex what to do differently",
        TextStyle::plain(ColorToken::Bad),
    )?;

    paint_dock_frame(&mut canvas, 24, 13, "", "")?;
    canvas.write(
        25,
        5,
        "Allow `cargo`?",
        TextStyle::bold(ColorToken::Bright),
    )?;
    canvas.write(
        27,
        5,
        "cargo test retry -- --nocapture",
        TextStyle::plain(ColorToken::Text),
    )?;
    // Static options remain selectable; RejectOnce row is freeform input preview.
    paint_permission_option(&mut canvas, 29, 1, false, "Yes, proceed")?;
    paint_permission_option(
        &mut canvas,
        30,
        2,
        false,
        "Always allow: cargo test",
    )?;
    // Focused RejectOnce freeform preview: `3 [x] ❯ <reason draft>`.
    canvas.write(
        31,
        5,
        "3 [x] ❯ Keep production config read-only; rewrite the plan step",
        TextStyle::bold(ColorToken::Accent),
    )?;
    canvas.write(
        33,
        5,
        "Enter sends follow-up · Esc cancels without running",
        TextStyle::plain(ColorToken::Dim),
    )?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

/// Blocked execute result after sandbox / protected path denial.
fn blocked_command() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_demo_top_bar(&mut canvas)?;
    paint_user_turn(
        &mut canvas,
        3,
        &["Run the redeploy script after tests pass."],
    )?;
    // Product execute header with bad status — never `✗ Shell`.
    paint_execute_shell_header(
        &mut canvas,
        7,
        "./scripts/redeploy.sh --prod",
        TextStyle::plain(ColorToken::Bad),
    )?;
    paint_tool_body(
        &mut canvas,
        8,
        "blocked · protected path",
        TextStyle::plain(ColorToken::Bad),
    )?;
    canvas.write(
        11,
        2,
        "Protected path · production redeploy is not allowed in always-approve.",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        12,
        2,
        "Request approval for this command, or switch mode and re-run safer steps.",
        TextStyle::plain(ColorToken::Secondary),
    )?;
    // Concrete recovery: Request approval → permission_view path.
    canvas.write(
        15,
        2,
        "1 (●) Request approval",
        TextStyle::bold(ColorToken::Accent),
    )?;
    canvas.write(
        16,
        2,
        "2 (○) Keep blocked and continue",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        18,
        2,
        "Enter requests approval · Esc keeps the block visible",
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
    canvas.write(
        8,
        2,
        "Two writers can skip the watched counter when the",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        9,
        2,
        "snapshot freezes mid-batch…",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        12,
        2,
        "Turn failed · provider closed the stream early",
        TextStyle::bold(ColorToken::Bad),
    )?;
    canvas.write(
        13,
        2,
        "Partial answer kept above · draft stays in the jewel",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        16,
        2,
        "1 (●) Retry turn",
        TextStyle::bold(ColorToken::Accent),
    )?;
    canvas.write(17, 2, "2 (○) Keep partial and continue", TextStyle::plain(ColorToken::Text))?;
    canvas.write(
        18,
        2,
        "3 (○) Inspect network evidence",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        20,
        2,
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
    canvas.write(
        10,
        2,
        "Network error · could not reach xAI",
        TextStyle::bold(ColorToken::Bad),
    )?;
    canvas.write(
        11,
        2,
        "Check the link, then retry once the path is clear.",
        TextStyle::plain(ColorToken::Secondary),
    )?;
    canvas.write(
        13,
        2,
        "Evidence · timeout after 12s · DNS ok · TLS handshake fail",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        16,
        2,
        "1 (●) Retry connection",
        TextStyle::bold(ColorToken::Accent),
    )?;
    canvas.write(17, 2, "2 (○) Work offline with local tools", TextStyle::plain(ColorToken::Text))?;
    canvas.write(
        19,
        2,
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
    canvas.write(
        9,
        2,
        "Context  ████████████████████░░  948K / 1.0M",
        TextStyle::plain(ColorToken::Warn),
    )?;
    canvas.write(
        11,
        2,
        "Context is nearly full · compact before the next send",
        TextStyle::bold(ColorToken::Warn),
    )?;
    canvas.write(
        12,
        2,
        "Older tool bodies can fold without losing your draft.",
        TextStyle::plain(ColorToken::Secondary),
    )?;
    canvas.write(
        15,
        2,
        "1 (●) Compact now",
        TextStyle::bold(ColorToken::Accent),
    )?;
    canvas.write(16, 2, "2 (○) Send anyway (may truncate)", TextStyle::plain(ColorToken::Text))?;
    canvas.write(17, 2, "3 (○) Open context sidebar", TextStyle::plain(ColorToken::Text))?;
    canvas.write(
        19,
        2,
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
        2,
        "Rate limit · credit floor reached",
        TextStyle::bold(ColorToken::Bad),
    )?;
    // Product timer-safe language (≥60s uses minute form when relevant).
    canvas.write(
        10,
        2,
        "Next slot opens in 42s · draft and queue stay saved",
        TextStyle::plain(ColorToken::Secondary),
    )?;
    canvas.write(
        12,
        2,
        "Credits  ████████░░░░░░░░░░░░  40% recovered",
        TextStyle::plain(ColorToken::Warn),
    )?;
    canvas.write(
        15,
        2,
        "1 (●) Wait and retry automatically",
        TextStyle::bold(ColorToken::Accent),
    )?;
    canvas.write(16, 2, "2 (○) Switch to xAI model Fast", TextStyle::plain(ColorToken::Text))?;
    canvas.write(17, 2, "3 (○) Work offline until reset", TextStyle::plain(ColorToken::Text))?;
    canvas.write(
        19,
        2,
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
    canvas.write(
        8,
        2,
        "Two writers can skip the watched counter when the",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        9,
        2,
        "snapshot freezes…",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        12,
        2,
        "Interrupted · stream stopped with [stop]",
        TextStyle::bold(ColorToken::Warn),
    )?;
    canvas.write(
        13,
        2,
        "Draft stays in the jewel · partial answer kept above",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_default_prompt(
        &mut canvas,
        33,
        "Continue from the partial note…",
    )?;
    canvas.write(
        31,
        2,
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
        2,
        "Rewind available · choose how far to jump",
        TextStyle::bold(ColorToken::Bright),
    )?;
    paint_picker_like(
        &mut canvas,
        13,
        "Checkpoint · before edit",
        "retry.rs hash a1b2… · 18s ago",
        true,
    )?;
    paint_picker_like(
        &mut canvas,
        15,
        "Checkpoint · after tests green",
        "suite pass · 1m2s ago",
        false,
    )?;
    canvas.write(
        18,
        2,
        "1 (●) Restore selected checkpoint",
        TextStyle::bold(ColorToken::Accent),
    )?;
    canvas.write(19, 2, "2 (○) Keep current workspace", TextStyle::plain(ColorToken::Text))?;
    canvas.write(
        21,
        2,
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
    paint_permission_option(&mut canvas, 33, 1, true, "Create share link")?;
    paint_permission_option(&mut canvas, 34, 2, false, "Export file only")?;
    paint_default_putty(&mut canvas)?;
    let _ = MODEL_LABEL;
    finish(canvas)
}

/// Paint one permission_view radio option: `N (●|○) label`.
fn paint_permission_option(
    canvas: &mut Canvas,
    row: u16,
    n: u8,
    selected: bool,
    label: &str,
) -> Result<(), String> {
    let marker = if selected { "●" } else { "○" };
    let line = format!("{n} ({marker}) {label}");
    let style = if selected {
        TextStyle::bold(ColorToken::Accent)
    } else {
        TextStyle::plain(ColorToken::Text)
    };
    canvas.write(row, 5, &line, style)?;
    Ok(())
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
            2,
            &format!("› {title}"),
            TextStyle::bold(ColorToken::Accent),
        )?;
        canvas.write(row + 1, 4, detail, TextStyle::plain(ColorToken::Muted))?;
    } else {
        canvas.write(
            row,
            2,
            &format!("  {title}"),
            TextStyle::plain(ColorToken::Text),
        )?;
        canvas.write(row + 1, 4, detail, TextStyle::plain(ColorToken::Dim))?;
    }
    Ok(())
}

fn finish(canvas: Canvas) -> Result<Canvas, String> {
    let text = canvas.to_text();
    reject_copper_residual(&text)?;
    reject_structure_residual(&text)?;
    reject_permission_language_residual(&text)?;
    Ok(canvas)
}

/// VAL-V3-STRUCT-008 residual bans for permission / blocked family.
pub fn reject_permission_language_residual(text: &str) -> Result<(), String> {
    if text.contains("◆ Cortex") {
        return Err("permission residual refused: Cortex nameplate".to_owned());
    }
    for needle in ["Shell  ", "✓ Shell", "✗ Shell", "bash  "] {
        if text.contains(needle) {
            return Err(format!(
                "permission residual refused: Shell invent {needle:?}"
            ));
        }
    }
    // Cortex / hybrid gallery dock essay fingerprints.
    for essay in [
        "Allow this command?",
        "Apply this edit?",
        "Why deny or request changes?",
        "Allow once",
        "Allow for session",
        "Allow for this project",
        "Working directory ·",
        "needs permission before running on this host",
        "Cortex cannot run production redeploy from this shell",
        "Open safer command variants",
        "Keep blocked and continue chatting",
        "Request changes",
        "Deny without comment",
        "Allow with caution",
        "Enter allows once",
    ] {
        if text.contains(essay) {
            return Err(format!(
                "permission residual refused: Cortex essay/hybrid {essay:?}"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chrome::components::paint_turn_status_busy;

    const PERMISSION_CORE: &[&str] = &[
        "permission-shell",
        "permission-file-edit",
        "permission-deny-reason",
        "blocked-command",
    ];

    const PERMISSION_BATCH: &[&str] = &[
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
    ];

    #[test]
    fn permission_core_product_language_and_shell_chrome() {
        for id in PERMISSION_CORE {
            let text = render(id)
                .unwrap_or_else(|e| panic!("{id}: {e}"))
                .to_text();
            reject_copper_residual(&text).unwrap_or_else(|e| panic!("{id}: {e}"));
            reject_structure_residual(&text).unwrap_or_else(|e| panic!("{id}: {e}"));
            reject_permission_language_residual(&text)
                .unwrap_or_else(|e| panic!("{id}: {e}"));
            assert_eq!(text.lines().count(), 40, "{id}");
            assert!(!text.contains("◆ Cortex"), "{id} nameplate ban");
            assert!(!text.contains("Shell  "), "{id} Shell invent");
            assert!(!text.contains("Allow this command?"), "{id}");
            assert!(!text.contains("Allow once"), "{id}");
            assert!(
                text.contains("9.5K / 500K") || text.contains("K / "),
                "{id} context budget"
            );
            assert!(
                text.contains("Shift+Tab:mode | Ctrl+c:cancel"),
                "{id} putty pipes"
            );
            // Bare user wash origin (not ❯ You).
            assert!(
                text.contains("❯ Tighten")
                    || text.contains("❯ Apply")
                    || text.contains("❯ Run the redeploy"),
                "{id} bare user ❯ origin"
            );
            assert!(!text.contains("❯ You"), "{id}");
        }
        let shell = render("permission-shell").unwrap().to_text();
        assert!(shell.contains("Allow `cargo`?"), "product title");
        assert!(shell.contains("cargo test retry -- --nocapture"));
        assert!(shell.contains("$ cargo test retry -- --nocapture"));
        assert!(shell.contains("1 (●) Yes, proceed"));
        assert!(shell.contains("Always allow: cargo test"));
        assert!(shell.contains(REJECT_ONCE));
        assert!(shell.contains("< > to choose permission scope"));
        assert!(!shell.contains("Allow for session"));
        assert!(!shell.contains("Allow this command?"));

        let file = render("permission-file-edit").unwrap().to_text();
        assert!(file.contains("Allow Edit to crates/agent/src/retry.rs?"));
        assert!(file.contains("Edit crates/agent/src/retry.rs"));
        assert!(file.contains("1 (●) Yes"));
        assert!(file.contains(ALLOW_EDITS_SESSION));
        assert!(file.contains(REJECT_ONCE));
        assert!(!file.contains("Apply this edit?"));
        assert!(!file.contains("Apply edit"));

        let deny = render("permission-deny-reason").unwrap().to_text();
        assert!(deny.contains("3 [x] ❯"));
        assert!(deny.contains("Keep production config read-only"));
        assert!(deny.contains("Allow `cargo`?"));
        assert!(!deny.contains("Why deny or request changes?"));
        assert!(!deny.contains("Request changes"));

        let blocked = render("blocked-command").unwrap().to_text();
        assert!(blocked.contains("$ ./scripts/redeploy.sh --prod"));
        assert!(blocked.contains("blocked · protected path"));
        assert!(blocked.contains("Request approval"));
        assert!(blocked.contains("Cortex (high)"));
        assert!(!blocked.contains("✗ Shell"));
        assert!(!blocked.contains("Open safer command variants"));
    }

    #[test]
    fn permissions_batch_pure_cortex() {
        for id in PERMISSION_BATCH {
            let text = render(id)
                .unwrap_or_else(|e| panic!("{id}: {e}"))
                .to_text();
            assert!(!text.contains("Claude Opus"), "{id}");
            assert!(!text.contains("Cortex Code"), "{id}");
            assert!(!text.contains("Work mode"), "{id}");
            assert!(!text.contains("◆ Cortex"), "{id}");
            assert!(
                text.contains("Shift+Tab:mode") || text.contains("Esc"),
                "{id}"
            );
            assert!(
                text.contains("9.5K / 500K") || text.contains("K / "),
                "{id} budget"
            );
            reject_permission_language_residual(&text)
                .unwrap_or_else(|e| panic!("{id}: {e}"));
        }
        let interrupt = render("interrupt").unwrap().to_text();
        assert!(interrupt.contains("[stop]"));
        assert!(interrupt.contains("Cortex"));
        let mut c = Canvas::new(Theme::CortexNight);
        paint_turn_status_busy(&mut c, 30, "Responding", "1.0s", "1k").unwrap();
        assert!(c.to_text().contains("[stop]"));
    }
}
