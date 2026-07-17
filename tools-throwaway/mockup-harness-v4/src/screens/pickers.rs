//! Pure Cortex composer & pickers (F4).
//!
//! Slash dropdowns, model/agent/session catalogs, queue, attachment, export,
//! and multi-line composer. Magenta `›` focus, putty footer allowed, no copper.

use crate::chrome::canvas::Canvas;
use crate::chrome::components::{
    paint_default_putty, paint_dock_frame, paint_picker_row, paint_picker_row_compact,
    paint_prompt_jewel_multiline, paint_quiet_origin, reject_copper_residual, MODEL_LABEL,
};
use crate::chrome::tokens::{ColorToken, TextStyle, Theme};

pub fn render(id: &str) -> Result<Canvas, String> {
    match id {
        "slash-menu" => slash_menu(),
        "command-palette" => command_palette(),
        "command-palette-submenu" => command_palette_submenu(),
        "model-picker" => model_picker(),
        "model-picker-filtered" => model_picker_filtered(),
        "agent-picker" => agent_picker(),
        "skills-picker" => skills_picker(),
        "session-picker" => session_picker(),
        "variant-picker" => variant_picker(),
        "fork-picker" => fork_picker(),
        "theme-picker" => theme_picker(),
        "export-picker" => export_picker(),
        "attachment-picker" => attachment_picker(),
        "subagent-picker" => subagent_picker(),
        "file-mention" => file_mention(),
        "queue-manager" => queue_manager(),
        "composer-multiline" => composer_multiline(),
        other => Err(format!("unknown pickers pure-Cortex id: {other}")),
    }
}

fn slash_menu() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_quiet_origin(
        &mut canvas,
        "Tighten the retry helper before we merge.",
        "Applying the retry guard and covering the failure path…",
    )?;
    canvas.write(11, 4, "Commands", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(
        12,
        4,
        "Type more to filter · Esc restores the draft",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_picker_row_compact(
        &mut canvas,
        14,
        "/model",
        "switch Cortex catalog model",
        true,
    )?;
    paint_picker_row_compact(
        &mut canvas,
        15,
        "/theme",
        "preview CortexNight or CortexDay",
        false,
    )?;
    paint_picker_row_compact(&mut canvas, 16, "/mcp", "manage MCP connections", false)?;
    paint_picker_row_compact(
        &mut canvas,
        17,
        "/export",
        "share this conversation",
        false,
    )?;
    paint_picker_row_compact(
        &mut canvas,
        18,
        "/session",
        "resume an earlier turn",
        false,
    )?;
    paint_dock_frame(
        &mut canvas,
        30,
        7,
        "Filter · /",
        "Enter to run · Esc to cancel · ↑↓ to choose",
    )?;
    canvas.write(32, 5, "❯ /m", TextStyle::plain(ColorToken::Text))?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn command_palette() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_quiet_origin(
        &mut canvas,
        "Open a command without leaving this chat.",
        "",
    )?;
    canvas.write(11, 4, "Actions", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(
        12,
        4,
        "Shortcuts that stay bound to this origin",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_picker_row(
        &mut canvas,
        14,
        "Switch model",
        "Open the Cortex catalog for this turn",
        true,
    )?;
    paint_picker_row(
        &mut canvas,
        16,
        "Queued prompts",
        "Reorder, drop, or send the next prompt now",
        false,
    )?;
    paint_picker_row(
        &mut canvas,
        18,
        "All shortcuts",
        "Ctrl+. cheatsheet with context-sensitive hints",
        false,
    )?;
    paint_picker_row(
        &mut canvas,
        20,
        "Export conversation",
        "Markdown or plain text snapshot",
        false,
    )?;
    paint_dock_frame(
        &mut canvas,
        30,
        7,
        "Filter · empty shows recent actions",
        "Enter to open · Esc to cancel · ↑↓ to choose",
    )?;
    canvas.write(32, 5, "❯ ", TextStyle::plain(ColorToken::Text))?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn command_palette_submenu() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_quiet_origin(
        &mut canvas,
        "Manage queued prompts for this turn.",
        "",
    )?;
    canvas.write(
        11,
        4,
        "Queued prompts · submenu",
        TextStyle::bold(ColorToken::Bright),
    )?;
    canvas.write(
        12,
        4,
        "Scoped under the palette · Esc returns to Actions",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_picker_row_compact(
        &mut canvas,
        14,
        "Send next now",
        "interrupt current stream safely",
        true,
    )?;
    paint_picker_row_compact(
        &mut canvas,
        15,
        "Move up",
        "raise priority one slot",
        false,
    )?;
    paint_picker_row_compact(
        &mut canvas,
        16,
        "Move down",
        "lower priority one slot",
        false,
    )?;
    paint_picker_row_compact(
        &mut canvas,
        17,
        "Remove from queue",
        "keeps draft unsent",
        false,
    )?;
    paint_dock_frame(
        &mut canvas,
        30,
        7,
        "Esc exits submenu only · parent stays open",
        "Enter to apply · Esc to back · ↑↓ to choose",
    )?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn model_picker() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_quiet_origin(
        &mut canvas,
        "Choose the model for the next reply.",
        "",
    )?;
    canvas.write(11, 4, "Models", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(
        12,
        4,
        "Each row says what changes before you apply it.",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_picker_row(
        &mut canvas,
        14,
        "Cortex",
        "Current · balanced depth for everyday coding",
        true,
    )?;
    paint_picker_row(
        &mut canvas,
        16,
        "Cortex (high)",
        "More reasoning budget · slower, deeper plans",
        false,
    )?;
    paint_picker_row(
        &mut canvas,
        18,
        "xAI model Fast",
        "Quick edits and triage · lighter reasoning",
        false,
    )?;
    paint_dock_frame(
        &mut canvas,
        30,
        7,
        &format!("Selected · {MODEL_LABEL}"),
        "Enter to apply · Esc to cancel · f for Filter models",
    )?;
    canvas.write(
        32,
        5,
        "Applies to the next reply only · statusline stays Cortex-labeled",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn model_picker_filtered() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_quiet_origin(
        &mut canvas,
        "Filter the catalog for a faster match.",
        "",
    )?;
    canvas.write(
        11,
        4,
        "Models · filter \"fast\"",
        TextStyle::bold(ColorToken::Bright),
    )?;
    canvas.write(
        12,
        4,
        "2 matches · filter text stays until Esc",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_picker_row(
        &mut canvas,
        14,
        "xAI model Fast",
        "Quick edits and triage · lighter reasoning",
        true,
    )?;
    paint_picker_row(
        &mut canvas,
        16,
        "Cortex (high)",
        "Still listed when the filter touches effort tags",
        false,
    )?;
    paint_dock_frame(
        &mut canvas,
        30,
        7,
        "Filter · fast",
        "Enter to apply · Esc clears filter then cancels",
    )?;
    canvas.write(32, 5, "❯ fast", TextStyle::plain(ColorToken::Text))?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn agent_picker() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_quiet_origin(
        &mut canvas,
        "Pick how Cortex should approach this task.",
        "",
    )?;
    canvas.write(11, 4, "Agents", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(
        12,
        4,
        "Personas and agents · not a Cortex work-style matrix",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_picker_row(
        &mut canvas,
        14,
        "Coding",
        "Default · edit, test, and explain in one loop",
        true,
    )?;
    paint_picker_row(
        &mut canvas,
        16,
        "Plan",
        "Read-only planning before any write",
        false,
    )?;
    paint_picker_row(
        &mut canvas,
        18,
        "Review",
        "Critique diffs and suggest repairs only",
        false,
    )?;
    paint_dock_frame(
        &mut canvas,
        30,
        7,
        "Selected · Coding",
        "Enter to apply · Esc to cancel · ↑↓ to choose",
    )?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn skills_picker() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_quiet_origin(
        &mut canvas,
        "Enable skills for this session.",
        "",
    )?;
    canvas.write(11, 4, "Skills", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(
        12,
        4,
        "Session-scoped plugins and extensions",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_picker_row_compact(
        &mut canvas,
        14,
        "✓ retry-audit",
        "inspect retry helpers",
        true,
    )?;
    paint_picker_row_compact(
        &mut canvas,
        15,
        "  docs-search",
        "index docs for lookups",
        false,
    )?;
    paint_picker_row_compact(
        &mut canvas,
        16,
        "✓ shell-safe",
        "sandboxed command preflight",
        false,
    )?;
    paint_picker_row_compact(
        &mut canvas,
        17,
        "  image-caption",
        "describe pasted screenshots",
        false,
    )?;
    paint_dock_frame(
        &mut canvas,
        30,
        7,
        "Space toggles · only checked skills load",
        "Enter to save · Esc to cancel · ↑↓ to move",
    )?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn session_picker() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_quiet_origin(
        &mut canvas,
        "Resume an earlier conversation in this workspace.",
        "",
    )?;
    canvas.write(11, 4, "Sessions", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(
        12,
        4,
        "Local resume list · drafts restore with the transcript",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_picker_row(
        &mut canvas,
        14,
        "Retry policy review",
        "17 turns · master · open draft kept",
        true,
    )?;
    paint_picker_row(
        &mut canvas,
        16,
        "Cache invalidation sketch",
        "9 turns · feature/cache · idle",
        false,
    )?;
    paint_picker_row(
        &mut canvas,
        18,
        "MCP docs research",
        "4 turns · connect work still open",
        false,
    )?;
    paint_dock_frame(
        &mut canvas,
        30,
        7,
        "Selected · Retry policy review",
        "Enter to resume · Esc to cancel · ↑↓ to choose",
    )?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn variant_picker() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_quiet_origin(
        &mut canvas,
        "Pick effort for this reply only.",
        "",
    )?;
    canvas.write(
        11,
        4,
        "Effort · /effort",
        TextStyle::bold(ColorToken::Bright),
    )?;
    canvas.write(
        12,
        4,
        "Reasoning budget for the next assistant turn",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_picker_row_compact(
        &mut canvas,
        14,
        "Low",
        "fast answers · light plan",
        false,
    )?;
    paint_picker_row_compact(
        &mut canvas,
        15,
        "Medium",
        "default balanced budget",
        true,
    )?;
    paint_picker_row_compact(
        &mut canvas,
        16,
        "High",
        "deep plans · slower stream",
        false,
    )?;
    paint_dock_frame(
        &mut canvas,
        30,
        7,
        "Selected · Medium",
        "Enter to apply · Esc keeps previous effort",
    )?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn fork_picker() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_quiet_origin(
        &mut canvas,
        "Fork from this turn into a new branch of the chat.",
        "",
    )?;
    canvas.write(11, 4, "Fork conversation", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(
        12,
        4,
        "Branches keep history up to the selected turn",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_picker_row(
        &mut canvas,
        14,
        "Fork at pause",
        "New session · original stays untouched",
        true,
    )?;
    paint_picker_row(
        &mut canvas,
        16,
        "Fork + open draft",
        "Copies the current draft into the fork",
        false,
    )?;
    paint_picker_row(
        &mut canvas,
        18,
        "Cancel",
        "Return without branching",
        false,
    )?;
    paint_dock_frame(
        &mut canvas,
        30,
        7,
        "Confirm fork destination",
        "Enter to fork · Esc to cancel",
    )?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn theme_picker() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_quiet_origin(
        &mut canvas,
        "Preview a theme without leaving this origin.",
        "",
    )?;
    canvas.write(11, 4, "Themes", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(
        12,
        4,
        "Live preview · Apply writes ui.theme only",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_picker_row(
        &mut canvas,
        14,
        "CortexNight",
        "Default dark · magenta accent #bb9af7",
        true,
    )?;
    paint_picker_row(
        &mut canvas,
        16,
        "CortexDay",
        "Light peer · deepened magenta #7d4bc6",
        false,
    )?;
    paint_dock_frame(
        &mut canvas,
        30,
        7,
        "Preview active · CortexNight",
        "Enter to apply · Esc to cancel · ↑↓ previews",
    )?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn export_picker() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_quiet_origin(
        &mut canvas,
        "Export or share this conversation.",
        "",
    )?;
    canvas.write(11, 4, "Export", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(
        12,
        4,
        "Choose a format · secrets stay redacted",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_picker_row_compact(
        &mut canvas,
        14,
        "Markdown",
        "transcript with tool folds",
        true,
    )?;
    paint_picker_row_compact(
        &mut canvas,
        15,
        "Plain text",
        "flat copy without role chrome",
        false,
    )?;
    paint_picker_row_compact(
        &mut canvas,
        16,
        "Share link",
        "redacted snapshot URL when enabled",
        false,
    )?;
    paint_dock_frame(
        &mut canvas,
        30,
        7,
        "Selected · Markdown",
        "Enter to export · Esc to cancel",
    )?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn attachment_picker() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_quiet_origin(
        &mut canvas,
        "Attach a project file to the next send.",
        "",
    )?;
    canvas.write(11, 4, "Attach file", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(
        12,
        4,
        "Workspace files only · images use [Image #N] chips",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_picker_row_compact(
        &mut canvas,
        14,
        "crates/agent/src/retry.rs",
        "14 KB · rust",
        true,
    )?;
    paint_picker_row_compact(
        &mut canvas,
        15,
        "docs/retry.md",
        "4 KB · markdown",
        false,
    )?;
    paint_picker_row_compact(
        &mut canvas,
        16,
        "fixtures/rate_limit.json",
        "1 KB · json",
        false,
    )?;
    paint_dock_frame(
        &mut canvas,
        30,
        7,
        "Selected · crates/agent/src/retry.rs",
        "Enter to attach · Esc to cancel · type to filter",
    )?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn subagent_picker() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_quiet_origin(
        &mut canvas,
        "Spawn a task-local helper for this turn.",
        "",
    )?;
    canvas.write(11, 4, "Subagents", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(
        12,
        4,
        "Short-lived helpers · results fold back into chat",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_picker_row(
        &mut canvas,
        14,
        "audit-subagent",
        "Scan retry helpers for races",
        true,
    )?;
    paint_picker_row(
        &mut canvas,
        16,
        "coverage-subagent",
        "Find untested floor cases",
        false,
    )?;
    paint_picker_row(
        &mut canvas,
        18,
        "docs-subagent",
        "Gather related notes without edits",
        false,
    )?;
    paint_dock_frame(
        &mut canvas,
        30,
        7,
        "Selected · audit-subagent",
        "Enter to spawn · Esc to cancel",
    )?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn file_mention() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_quiet_origin(
        &mut canvas,
        "Mention a path with @ before sending.",
        "",
    )?;
    canvas.write(11, 4, "Files · @", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(
        12,
        4,
        "Fuzzy path match · soft highlight on hits",
        TextStyle::plain(ColorToken::Muted),
    )?;
    // Soft Info decoration on match fragment.
    canvas.write(14, 4, "› crates/agent/src/", TextStyle::bold(ColorToken::Accent))?;
    canvas.write(14, 24, "retry", TextStyle::plain(ColorToken::Info))?;
    canvas.write(14, 29, ".rs", TextStyle::bold(ColorToken::Accent))?;
    canvas.write(
        15,
        6,
        "14 KB · last touched this branch",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        17,
        4,
        "  crates/agent/src/rate_limit.rs",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        18,
        6,
        "9 KB · related helper",
        TextStyle::plain(ColorToken::Dim),
    )?;
    canvas.write(
        20,
        4,
        "  tests/retry_floors.rs",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        21,
        6,
        "3 KB · suite entry",
        TextStyle::plain(ColorToken::Dim),
    )?;
    paint_dock_frame(
        &mut canvas,
        30,
        7,
        "Filter · @ret",
        "Enter to insert · Esc to cancel · ↑↓ to choose",
    )?;
    canvas.write(32, 5, "❯ Check @ret", TextStyle::plain(ColorToken::Text))?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn queue_manager() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_quiet_origin(
        &mut canvas,
        "Three prompts are waiting behind the live stream.",
        "Still writing the retry summary…",
    )?;
    canvas.write(11, 4, "Queue", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(
        12,
        4,
        "Reorder · remove · Enter to send now",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_picker_row_compact(
        &mut canvas,
        14,
        "1  Add floor integration case",
        "next",
        true,
    )?;
    paint_picker_row_compact(
        &mut canvas,
        15,
        "2  Document jitter order",
        "waiting",
        false,
    )?;
    paint_picker_row_compact(
        &mut canvas,
        16,
        "3  Note rate-limit ceiling",
        "waiting",
        false,
    )?;
    paint_dock_frame(
        &mut canvas,
        30,
        7,
        "Selected · 1 of 3 · send interrupts safely",
        "Enter to send now · d remove · j/k reorder · Esc dismiss",
    )?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn composer_multiline() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_quiet_origin(
        &mut canvas,
        "Draft a longer request without losing the mode chip.",
        "",
    )?;
    paint_prompt_jewel_multiline(
        &mut canvas,
        28,
        &[
            "Review the retry helper,",
            "list the remaining risks,",
            "and propose one focused test.",
        ],
        MODEL_LABEL,
        "always-approve",
    )?;
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
    use crate::chrome::components::MODEL_LABEL;

    #[test]
    fn pickers_batch_pure_cortex() {
        for id in [
            "slash-menu",
            "command-palette",
            "command-palette-submenu",
            "model-picker",
            "model-picker-filtered",
            "agent-picker",
            "skills-picker",
            "session-picker",
            "variant-picker",
            "fork-picker",
            "theme-picker",
            "export-picker",
            "attachment-picker",
            "subagent-picker",
            "file-mention",
            "queue-manager",
            "composer-multiline",
        ] {
            let canvas = render(id).unwrap_or_else(|e| panic!("{id}: {e}"));
            let text = canvas.to_text();
            assert!(!text.contains("Claude Opus"), "{id}");
            assert!(!text.contains("Work mode"), "{id}");
            assert!(!text.contains("How Code should work"), "{id}");
            if id == "model-picker" || id == "composer-multiline" {
                assert!(text.contains(MODEL_LABEL), "{id} missing model");
            }
            if id == "composer-multiline" {
                // Right-aligned effort chip on draft row — no invented Draft·N meta.
                assert!(
                    text.contains("Cortex (high)") && text.contains("always-approve"),
                    "{id} missing right jewel chip"
                );
                assert!(!text.contains("Draft ·"), "{id} Draft·N invent");
            }
            assert!(text.contains("Shift+Tab:mode"), "{id} putty");
            assert!(
                text.contains('|') || text.contains("  "),
                "{id} putty must use product | / multi-space separators"
            );
        }
    }
}
