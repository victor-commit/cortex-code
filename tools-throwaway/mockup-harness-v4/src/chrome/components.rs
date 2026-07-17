//! Pure Cortex shell primitives: top bar, braille logo, prompt jewel, putty footer, turn-status.
//!
//! NO copper brand helpers. No ASCII "Code" monument. Model chrome is Cortex.

use super::canvas::Canvas;
use super::tokens::{ColorToken, TextStyle};

/// Default model label paint for pure Cortex dump chrome.
pub const MODEL_LABEL: &str = "Cortex";

/// Frozen deterministic device code for login pilot (never live wall codes).
pub const DEVICE_CODE: &str = "BTS4-P4ZF";

/// Cortex prompt arrow (U+276F).
pub const PROMPT_ARROW: char = '❯';

/// Distinct braille pattern used as a logo mark (not an animation frame).
/// Each line is a short braille sequence for the welcome hero.
pub const BRAILLE_LOGO: [&str; 7] = [
    "⠀⠀⠀⠀⠀⠀⣀⣀⡀⠀⠀⠀⢀⠄",
    "⠀⠀⠀⣠⣾⠿⠛⠛⠛⠛⢀⡴⠁⠀",
    "⠀⠀⣼⡟⠁⠀⠀⠀⢀⡴⠻⣿⡀⠀",
    "⠀⠀⣿⡇⠀⠀⠀⠔⠁⠀⠀⣿⡇⠀",
    "⠀⠀⢹⣷⠀⠀⠀⠀⠀⢀⣴⡿⠀⠀",
    "⠀⢀⠞⠁⠠⢶⣶⣶⣶⠿⠋⠀⠀⠀",
    "⠐⠁⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀",
];

pub fn paint_top_branch_bar(
    canvas: &mut Canvas,
    branch: &str,
    path: &str,
) -> Result<(), String> {
    // ` branch path` — Cortex welcome top bar.
    let line = format!("   {branch} {path}");
    canvas.write(1, 0, &line, TextStyle::bold(ColorToken::Secondary))?;
    Ok(())
}

pub fn paint_braille_logo(canvas: &mut Canvas, start_row: u16) -> Result<(), String> {
    for (i, line) in BRAILLE_LOGO.iter().enumerate() {
        canvas.write_centered(
            start_row + i as u16,
            line,
            TextStyle::plain(ColorToken::Muted),
        )?;
    }
    Ok(())
}

pub fn paint_putty_footer(canvas: &mut Canvas, hints: &str) -> Result<(), String> {
    // Permanent product putty footer is allowed on pure dump shells.
    canvas.write(39, 1, hints, TextStyle::plain(ColorToken::Dim))?;
    Ok(())
}

pub fn paint_turn_status_busy(
    canvas: &mut Canvas,
    row: u16,
    activity: &str,
    timer: &str,
    tokens: &str,
) -> Result<(), String> {
    // `⠧ Run command 0.2s · N queued — Enter...     1m20s ⇣12k [stop]`
    let left = format!("  ⠧ {activity}");
    canvas.write(row, 0, &left, TextStyle::plain(ColorToken::Running))?;
    let right = format!("{timer} ⇣{tokens} [stop]");
    let col = CANVAS_RIGHT.saturating_sub(right.chars().count() as u16 + 2);
    canvas.write(row, col, &right, TextStyle::plain(ColorToken::Muted))?;
    // Stop control uses error color on the bracketed label region.
    let stop_col = col + right.find("[stop]").unwrap_or(0) as u16;
    canvas.write(row, stop_col, "[stop]", TextStyle::plain(ColorToken::Bad))?;
    Ok(())
}

const CANVAS_RIGHT: u16 = 120;

/// Idle prompt jewel with `❯ ` draft + model · mode chip.
pub fn paint_prompt_jewel(
    canvas: &mut Canvas,
    box_top: u16,
    draft: &str,
    model: &str,
    mode: &str,
) -> Result<(), String> {
    let box_bottom = box_top + 3;
    // Rounded box
    let left = 2u16;
    let right = 117u16;
    let width = (right - left + 1) as usize;
    let top = format!("╭{}╮", "─".repeat(width - 2));
    let bottom = format!("╰{}╯", "─".repeat(width - 2));
    canvas.write(box_top, left, &top, TextStyle::plain(ColorToken::Border))?;
    canvas.write(box_bottom, left, &bottom, TextStyle::plain(ColorToken::Border))?;
    // Middle content row
    let arrow_style = TextStyle::bold(ColorToken::Accent);
    canvas.put(box_top + 1, left, '│', TextStyle::plain(ColorToken::Border))?;
    canvas.put(box_top + 1, right, '│', TextStyle::plain(ColorToken::Border))?;
    canvas.put(box_top + 1, left + 2, PROMPT_ARROW, arrow_style)?;
    canvas.write(
        box_top + 1,
        left + 4,
        draft,
        TextStyle::plain(ColorToken::Text),
    )?;
    // Info row: model · mode
    canvas.put(box_top + 2, left, '│', TextStyle::plain(ColorToken::Border))?;
    canvas.put(box_top + 2, right, '│', TextStyle::plain(ColorToken::Border))?;
    let chip = format!("  {model} · {mode}");
    canvas.write(
        box_top + 2,
        left + 2,
        &chip,
        TextStyle::plain(ColorToken::Muted),
    )?;
    Ok(())
}

/// Login / device-auth quit affordance (localized, not global putty).
pub fn paint_login_quit(canvas: &mut Canvas) -> Result<(), String> {
    canvas.write_centered(33, "ctrl+d  quit", TextStyle::plain(ColorToken::Dim))?;
    Ok(())
}
