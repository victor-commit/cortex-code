//! Pure Cortex shell primitives: top bar, braille logo, prompt jewel, putty footer, turn-status.
//!
//! NO copper brand helpers. No ASCII "Code" monument. Model chrome is Cortex.

use super::canvas::Canvas;
use super::tokens::{ColorToken, TextStyle};

/// Default model label paint for pure Cortex dump chrome.
pub const MODEL_LABEL: &str = "Cortex";

/// Frozen deterministic device code for login / oauth fixtures (never live wall codes).
pub const DEVICE_CODE: &str = "BTS4-P4ZF";

/// Cortex prompt arrow (U+276F).
pub const PROMPT_ARROW: char = '❯';

/// Default putty footer hints for authenticated shells.
pub const DEFAULT_PUTTY: &str = "Shift+Tab:mode · Ctrl+c:cancel · Ctrl+.:shortcuts";

/// Default workspace identifiers for deterministic pure dump fixtures.
pub const DEMO_BRANCH: &str = "master";
pub const DEMO_PATH: &str = "/projects/demo";
/// Neutral workspace path for pure-dump auth/login chrome (no product-name residual).
pub const AUTH_PATH: &str = "/projects/demo";

/// Distinct braille pattern used as a logo mark (not an animation frame).
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
    let line = format!("   {branch} {path}");
    canvas.write(1, 0, &line, TextStyle::bold(ColorToken::Secondary))?;
    Ok(())
}

pub fn paint_demo_top_bar(canvas: &mut Canvas) -> Result<(), String> {
    paint_top_branch_bar(canvas, DEMO_BRANCH, DEMO_PATH)
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
    canvas.write(39, 1, hints, TextStyle::plain(ColorToken::Dim))?;
    Ok(())
}

pub fn paint_default_putty(canvas: &mut Canvas) -> Result<(), String> {
    paint_putty_footer(canvas, DEFAULT_PUTTY)
}

const CANVAS_RIGHT: u16 = 120;

pub fn paint_turn_status_busy(
    canvas: &mut Canvas,
    row: u16,
    activity: &str,
    timer: &str,
    tokens: &str,
) -> Result<(), String> {
    let left = format!("  ⠧ {activity}");
    canvas.write(row, 0, &left, TextStyle::plain(ColorToken::Running))?;
    // Suffix is a known ASCII segment; paint muted prefix then red stop so UTF-8
    // multi-byte glyphs such as ⇣ never desync byte vs column indices.
    let prefix = format!("{timer} ⇣{tokens} ");
    let stop = "[stop]";
    let right_width = prefix.chars().count() + stop.chars().count();
    let col = CANVAS_RIGHT.saturating_sub(right_width as u16 + 2);
    canvas.write(row, col, &prefix, TextStyle::plain(ColorToken::Muted))?;
    let stop_col = col + prefix.chars().count() as u16;
    canvas.write(row, stop_col, stop, TextStyle::plain(ColorToken::Bad))?;
    Ok(())
}

/// Idle prompt jewel with `❯ ` draft + model · mode chip.
pub fn paint_prompt_jewel(
    canvas: &mut Canvas,
    box_top: u16,
    draft: &str,
    model: &str,
    mode: &str,
) -> Result<(), String> {
    let box_bottom = box_top + 3;
    let left = 2u16;
    let right = 117u16;
    let width = (right - left + 1) as usize;
    let top = format!("╭{}╮", "─".repeat(width - 2));
    let bottom = format!("╰{}╯", "─".repeat(width - 2));
    canvas.write(box_top, left, &top, TextStyle::plain(ColorToken::Border))?;
    canvas.write(box_bottom, left, &bottom, TextStyle::plain(ColorToken::Border))?;
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

pub fn paint_default_prompt(
    canvas: &mut Canvas,
    box_top: u16,
    draft: &str,
) -> Result<(), String> {
    paint_prompt_jewel(canvas, box_top, draft, MODEL_LABEL, "always-approve")
}

/// Login / device-auth quit affordance (localized, not global putty).
pub fn paint_login_quit(canvas: &mut Canvas) -> Result<(), String> {
    canvas.write_centered(33, "ctrl+d  quit", TextStyle::plain(ColorToken::Dim))?;
    Ok(())
}

/// User turn wash + `❯ You` header + body lines.
pub fn paint_user_turn(
    canvas: &mut Canvas,
    start_row: u16,
    body: &[&str],
) -> Result<(), String> {
    let user_style = TextStyle::user_turn();
    canvas.fill_row_bg(start_row, 2, 100, user_style)?;
    canvas.write(start_row, 2, "❯ You", user_style)?;
    for (i, line) in body.iter().enumerate() {
        canvas.write(
            start_row + 1 + i as u16,
            4,
            line,
            TextStyle::plain(ColorToken::Secondary),
        )?;
    }
    Ok(())
}

/// Magenta to assistant header + body lines.
pub fn paint_assistant(
    canvas: &mut Canvas,
    start_row: u16,
    body: &[&str],
) -> Result<(), String> {
    canvas.write(start_row, 2, "◆ Cortex", TextStyle::accent())?;
    for (i, line) in body.iter().enumerate() {
        canvas.write(
            start_row + 1 + i as u16,
            4,
            line,
            TextStyle::plain(ColorToken::Text),
        )?;
    }
    Ok(())
}

/// Compact tool activity row: header + optional elbow body lines.
pub fn paint_tool_header(
    canvas: &mut Canvas,
    row: u16,
    verb: &str,
    detail: &str,
    style: TextStyle,
) -> Result<(), String> {
    let line = if detail.is_empty() {
        format!("  {verb}")
    } else {
        format!("  {verb}  {detail}")
    };
    canvas.write(row, 0, &line, style)?;
    Ok(())
}

pub fn paint_tool_elbow(
    canvas: &mut Canvas,
    row: u16,
    text: &str,
    style: TextStyle,
) -> Result<(), String> {
    canvas.write(row, 4, &format!("⎿ {text}"), style)?;
    Ok(())
}

/// Shared auth decision list dock (bottom, not floating).
pub fn paint_auth_option_list(
    canvas: &mut Canvas,
    title: &str,
    subtitle: &str,
    options: &[(&str, bool)],
    primary: &str,
    secondary: &str,
) -> Result<(), String> {
    let mut row = 18u16;
    canvas.write(row, 4, title, TextStyle::bold(ColorToken::Bright))?;
    row += 1;
    if !subtitle.is_empty() {
        canvas.write(row, 4, subtitle, TextStyle::plain(ColorToken::Muted))?;
        row += 2;
    } else {
        row += 1;
    }
    for (label, selected) in options {
        if *selected {
            canvas.write(
                row,
                4,
                &format!("› {label}"),
                TextStyle::bold(ColorToken::Accent),
            )?;
        } else {
            canvas.write(row, 4, &format!("  {label}"), TextStyle::plain(ColorToken::Text))?;
        }
        row += 1;
    }
    row += 1;
    canvas.write(
        row,
        4,
        &format!("Enter to {primary} · Esc to {secondary}"),
        TextStyle::plain(ColorToken::Dim),
    )?;
    Ok(())
}

/// Fail-closed pure-dump residual scan for copper/Cortex identity leftovers.
pub fn reject_copper_residual(text: &str) -> Result<(), String> {
    let banned = [
        "Cortex Code",
        "cortex-code",
        "Connect a model provider",
        "Code needs a provider",
        "Code ·",
        "Claude Opus",
        "Ask Code",
        "Work mode",
        "Authorize OpenAI",
        "Connect a provider",
        "copper",
    ];
    for needle in banned {
        if text.contains(needle) {
            return Err(format!("pure Cortex residual refused: found {needle:?}"));
        }
    }
    // Standalone ASCII brand monument line "Code" alone is forbidden.
    for line in text.lines() {
        if line.trim() == "Code" {
            return Err("pure Cortex residual refused: ASCII Code monument".to_owned());
        }
    }
    Ok(())
}
