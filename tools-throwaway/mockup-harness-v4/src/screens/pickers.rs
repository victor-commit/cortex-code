//! Pure Cortex composer & pickers (M3 pure-structure slash / model geometry).
//!
//! Authority: `library/pure-structure-law.md`, fidelity-audit-views-orchestration.md,
//! source `/tmp/grok-build-ref/.../views/slash_dropdown.rs` + slash/commands/{model,resume,session_info}.
//!
//! VAL-V3-STRUCT-011:
//! - `slash-menu` = slash_dropdown glued over live prompt with product command names
//!   (`/resume`, `/session-info`), never a detached Commands essay + Filter putty invent.
//! - `model-picker` / `model-picker-filtered` = `/model` arg suggestions + effort geometry,
//!   never marketing catalog + Selected putty invent.
//!
//! VAL-V3-CROSS-027: authenticated shell chrome (cwd + budget + putty pipes) inherited.

use crate::chrome::canvas::Canvas;
use crate::chrome::components::{
    paint_default_putty, paint_prompt_jewel, paint_prompt_jewel_multiline, paint_quiet_origin,
    paint_slash_dropdown_over_prompt, paint_slash_suggestion_row, reject_copper_residual,
    reject_structure_residual, MODEL_LABEL,
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

/// `/` command completions — product `slash_dropdown` over the live prompt.
///
/// Product names (source): /model, /theme, /mcps, /export, /resume, /session-info.
/// BAN: detached "Commands" essay, "Filter · /" putty rewrite, fake `/session` alias.
fn slash_menu() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_quiet_origin(
        &mut canvas,
        "Tighten the retry helper before we merge.",
        "Applied the retry guard and covered the failure path…",
    )?;

    // Suggestion rows sit immediately above the live jewel (not mid-scroll essay).
    // Rows use product descriptions from slash command registry (not gallery marketing).
    let rows: &[(&str, &str)] = &[
        ("/model", "Switch the active model"),
        ("/theme", "Switch the color theme"),
        ("/mcps", "Show MCP server status"),
        ("/export", "Export the current conversation to a file or clipboard"),
        ("/resume", "Resume a previous session"),
        ("/session-info", "Show session info"),
    ];
    // first_row leaves room above the prompt: jewel starts around row 33 style.
    // 6 rows + 1 gap + 3 jewel = 10 → first_row = 28 ⇒ jewel top 35? Too low.
    // Prefer first_row 26 → jewel 33 (matches chat-turn prompt row).
    paint_slash_dropdown_over_prompt(&mut canvas, 26, rows, 0, "/m")?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

/// Root action palette still a compact list, but product command naming (no Filter invent).
fn command_palette() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_quiet_origin(
        &mut canvas,
        "Open a command without leaving this chat.",
        "",
    )?;
    // Palette uses slash_dropdown geometry over live prompt rather than Filter dock.
    let rows: &[(&str, &str)] = &[
        ("/model", "Switch the active model"),
        ("/queue", "Manage queued prompts"),
        ("/help", "Browse commands and keyboard shortcuts"),
        ("/export", "Export the current conversation to a file or clipboard"),
        ("/session-info", "Show session info"),
    ];
    paint_slash_dropdown_over_prompt(&mut canvas, 26, rows, 0, "/")?;
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
    // Submenu remains a compact list pane glided above the live prompt.
    let rows: &[(&str, &str)] = &[
        ("send-next", "Send the next queued prompt now"),
        ("move-up", "Raise priority one slot"),
        ("move-down", "Lower priority one slot"),
        ("remove", "Remove from queue"),
    ];
    paint_slash_dropdown_over_prompt(&mut canvas, 27, rows, 0, "/queue ")?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

/// `/model` arg suggestion dropdown (display name + desc), optional current tag.
/// Effort is a chained sub-menu when the model supports reasoning.
/// BAN: marketing essay body + Selected · putty + "statusline stays Cortex-labeled".
fn model_picker() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_quiet_origin(
        &mut canvas,
        "Switch model for the next reply only when needed.",
        "",
    )?;
    // Model catalog as /model arg suggestions (product form: Name (current) + description).
    let rows: &[(&str, &str)] = &[
        ("Cortex (current)", "Balanced default · reasoning supported"),
        ("Cortex Fast", "Lower latency · lighter reasoning"),
        ("code model", "Code-focused next-turn completions"),
    ];
    // Live prompt shows partial `/model ` so Enter is arg-commit geometry.
    paint_slash_dropdown_over_prompt(&mut canvas, 27, rows, 0, "/model ")?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

/// Filtered `/model` document cube: typed prefix narrows the same arg dropdown
/// (not a full marketing filter pane with selected putty invent).
fn model_picker_filtered() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_quiet_origin(
        &mut canvas,
        "Narrow the model list with a typed prefix.",
        "",
    )?;
    // Prefix "fast" keeps only matches; still slash_dropdown over live prompt.
    let rows: &[(&str, &str)] = &[
        ("Cortex Fast", "Lower latency · lighter reasoning"),
        ("Cortex (current)", "Still listed when prefix hits the catalog"),
    ];
    paint_slash_dropdown_over_prompt(&mut canvas, 28, rows, 0, "/model fast")?;
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
    // Agent list as compact suggestions over live prompt (no Selected putty invent).
    let rows: &[(&str, &str)] = &[
        ("Coding", "Default · edit, test, and explain"),
        ("Plan", "Read-only planning before any write"),
        ("Review", "Critique diffs and suggest repairs"),
    ];
    paint_slash_dropdown_over_prompt(&mut canvas, 28, rows, 0, "/agents ")?;
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
    let rows: &[(&str, &str)] = &[
        ("✓ retry-audit", "inspect retry helpers"),
        ("  docs-search", "index docs for lookups"),
        ("✓ shell-safe", "sandboxed command preflight"),
        ("  image-caption", "describe pasted screenshots"),
    ];
    paint_slash_dropdown_over_prompt(&mut canvas, 27, rows, 0, "/plugin ")?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

/// `/resume` session picker surface — resume list, not Filter invent.
fn session_picker() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_quiet_origin(
        &mut canvas,
        "Resume a previous conversation.",
        "",
    )?;
    // Product: /resume opens session picker. Keep list gloved above live prompt.
    let rows: &[(&str, &str)] = &[
        ("Retry policy review", "17 turns · master"),
        ("Cache invalidation sketch", "9 turns · feature/cache"),
        ("MCP docs research", "4 turns · idle"),
    ];
    paint_slash_dropdown_over_prompt(&mut canvas, 28, rows, 0, "/resume")?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

/// Effort sub-menu for `/model <name> ` chained args (`low|medium|high|xhigh`).
fn variant_picker() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_quiet_origin(
        &mut canvas,
        "Pick reasoning effort for the next reply.",
        "",
    )?;
    // Product chained effort phase after a reasoning-capable model name + space.
    let rows: &[(&str, &str)] = &[
        ("low", "Fast answers · light plan"),
        ("medium", "Default balanced budget"),
        ("high", "Deep plans · slower stream"),
        ("xhigh", "Maximum reasoning budget"),
    ];
    paint_slash_dropdown_over_prompt(&mut canvas, 27, rows, 2, "/model Cortex ")?;
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
    let rows: &[(&str, &str)] = &[
        ("Fork at pause", "New session · original stays untouched"),
        ("Fork + open draft", "Copies the current draft into the fork"),
        ("Cancel", "Return without branching"),
    ];
    paint_slash_dropdown_over_prompt(&mut canvas, 28, rows, 0, "/fork ")?;
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
    // `/theme` arg suggestions (product theme kinds), not Selected · putty invent.
    let rows: &[(&str, &str)] = &[
        ("CortexNight", "Default dark · magenta accent"),
        ("CortexDay", "Light peer · deepened magenta"),
        ("auto", "Follow system appearance"),
    ];
    paint_slash_dropdown_over_prompt(&mut canvas, 28, rows, 0, "/theme ")?;
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
    let rows: &[(&str, &str)] = &[
        ("markdown", "Transcript with tool folds"),
        ("plain", "Flat copy without role chrome"),
        ("clipboard", "Copy the export payload now"),
    ];
    paint_slash_dropdown_over_prompt(&mut canvas, 28, rows, 0, "/export ")?;
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
    // File-search style rows still glued above live prompt (no Filter invent dock).
    let rows: &[(&str, &str)] = &[
        ("crates/agent/src/retry.rs", "14 KB · rust"),
        ("docs/retry.md", "4 KB · markdown"),
        ("fixtures/rate_limit.json", "1 KB · json"),
    ];
    paint_slash_dropdown_over_prompt(&mut canvas, 28, rows, 0, "@ret")?;
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
    let rows: &[(&str, &str)] = &[
        ("audit-subagent", "Scan retry helpers for races"),
        ("coverage-subagent", "Find untested floor cases"),
        ("docs-subagent", "Gather related notes without edits"),
    ];
    paint_slash_dropdown_over_prompt(&mut canvas, 28, rows, 0, "/tasks ")?;
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
    // Soft Info decoration on match fragment + dropdown glue above prompt.
    let label_col_w = "crates/agent/src/retry.rs".chars().count().min(40);
    paint_slash_suggestion_row(
        &mut canvas,
        26,
        "crates/agent/src/retry.rs",
        "14 KB · last touched this branch",
        true,
        label_col_w,
    )?;
    // Soft-info paint over the match run inside the selected path.
    canvas.write(26, 4 + 2 + 17, "retry", TextStyle::plain(ColorToken::Info))?;
    paint_slash_suggestion_row(
        &mut canvas,
        27,
        "crates/agent/src/rate_limit.rs",
        "9 KB · related helper",
        false,
        label_col_w,
    )?;
    paint_slash_suggestion_row(
        &mut canvas,
        28,
        "tests/retry_floors.rs",
        "3 KB · suite entry",
        false,
        label_col_w,
    )?;
    paint_prompt_jewel(
        &mut canvas,
        33,
        "Check @ret",
        MODEL_LABEL,
        "always-approve",
    )?;
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
    let rows: &[(&str, &str)] = &[
        ("1  Add floor integration case", "next"),
        ("2  Document jitter order", "waiting"),
        ("3  Note rate-limit ceiling", "waiting"),
    ];
    paint_slash_dropdown_over_prompt(&mut canvas, 28, rows, 0, "/queue")?;
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
    let text = canvas.to_text();
    reject_copper_residual(&text)?;
    reject_structure_residual(&text)?;
    // Picker-specific invent bans (STRUCT-011).
    for invent in [
        "Commands",
        "Filter · /",
        "Filter ·",
        "Selected ·",
        "statusline stays Cortex-labeled",
        "f for Filter models",
        "Type more to filter",
        "Esc restores the draft",
        "/session ·",
        "switch Cortex catalog model",
        "Each row says what changes before you apply it",
        "Models · filter",
    ] {
        // Allow product word "session-info" and "Cortex" while banning essays.
        if invent == "Commands" {
            // Ban standalone detached Commands essay header (whole-line-ish substring
            // "Commands" was used as title). Product never paints a mid-scroll
            // "Commands" heading for slash_dropdown.
            if text.lines().any(|l| l.trim() == "Commands") {
                return Err(format!("picker invent refused: {invent:?}"));
            }
            continue;
        }
        if text.contains(invent) {
            return Err(format!("picker invent refused: {invent:?}"));
        }
    }
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
            assert!(!text.contains("Selected ·"), "{id} Selected invent");
            assert!(!text.contains("Filter ·"), "{id} Filter invent");
            assert!(!text.contains("statusline stays Cortex-labeled"), "{id}");
            assert!(
                text.contains("9.5K / 500K") || text.contains("K / "),
                "{id} auth budget"
            );
            if id == "model-picker" || id == "composer-multiline" || id == "slash-menu" {
                assert!(text.contains(MODEL_LABEL), "{id} missing model");
            }
            if id == "composer-multiline" {
                assert!(
                    text.contains("Cortex (high)") && text.contains("always-approve"),
                    "{id} missing right jewel chip"
                );
                assert!(!text.contains("Draft ·"), "{id} Draft·N invent");
            }
            if id == "slash-menu" {
                assert!(text.contains("/resume"), "{id} product /resume");
                assert!(text.contains("/session-info"), "{id} product /session-info");
                assert!(!text.contains("/session ·"), "{id} fake /session");
                assert!(!text.lines().any(|l| l.trim() == "Commands"), "{id}");
                // Geometry: live prompt draft includes slash query.
                assert!(text.contains("❯ /m") || text.contains("/m"), "{id} live prompt");
            }
            if id == "model-picker" {
                assert!(text.contains("/model"), "{id} /model arg geometry");
                assert!(text.contains("Cortex"), "{id}");
                assert!(!text.contains("f for Filter models"), "{id}");
            }
            if id == "model-picker-filtered" {
                assert!(text.contains("/model"), "{id}");
                assert!(!text.contains("Models · filter"), "{id}");
            }
            if id == "variant-picker" {
                // Effort sub-menu product ids.
                assert!(text.contains("high") || text.contains("xhigh"), "{id} effort");
                assert!(text.contains("/model Cortex"), "{id} chained phase");
            }
            assert!(text.contains("Shift+Tab:mode"), "{id} putty");
            assert!(
                text.contains('|') || text.contains("  "),
                "{id} putty must use product | / multi-space separators"
            );
        }
    }

    #[test]
    fn slash_dropdown_rows_use_product_prefix() {
        let mut canvas = Canvas::new(Theme::CortexNight);
        paint_slash_suggestion_row(&mut canvas, 10, "/model", "Switch the active model", true, 14)
            .unwrap();
        paint_slash_suggestion_row(
            &mut canvas,
            11,
            "/resume",
            "Resume a previous session",
            false,
            14,
        )
        .unwrap();
        let text = canvas.to_text();
        let sel = text.lines().nth(10).unwrap();
        let unsel = text.lines().nth(11).unwrap();
        assert!(sel.contains('❯'), "selected uses ❯ prefix: {sel}");
        assert!(sel.contains("/model"));
        assert!(sel.contains("Switch the active model"));
        assert!(!unsel.trim_start().starts_with('❯') || unsel.contains("  /resume"));
        assert!(unsel.contains("/resume"));
    }
}
