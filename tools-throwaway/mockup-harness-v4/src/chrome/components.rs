//! Pure Cortex shell primitives: authenticated top bar, bare user wash, assistant
//! markdown stream (no speaker nameplates), prompt jewel, putty footer, turn-status.
//!
//! Structure law: mission `library/pure-structure-law.md`.
//! NO copper brand helpers. No ASCII "Code" monument. Model chrome is Cortex.

use super::canvas::Canvas;
use super::tokens::{ColorToken, TextStyle};

/// Default model label paint for pure Cortex dump chrome.
pub const MODEL_LABEL: &str = "Cortex";

/// Default effort tier shown in the prompt jewel chip (busy/idle product form).
pub const MODEL_EFFORT: &str = "high";

/// Default permission mode flag for the prompt jewel chip.
pub const MODEL_MODE_DEFAULT: &str = "always-approve";

/// Frozen deterministic device code for login / oauth fixtures (never live wall codes).
pub const DEVICE_CODE: &str = "BTS4-P4ZF";

/// Cortex prompt arrow (U+276F).
pub const PROMPT_ARROW: char = '❯';

/// Default putty footer hints for authenticated shells (product `|` separators).
pub const DEFAULT_PUTTY: &str = "Shift+Tab:mode | Ctrl+c:cancel | Ctrl+.:shortcuts";

/// Default workspace identifiers for deterministic pure dump fixtures.
pub const DEMO_BRANCH: &str = "master";
pub const DEMO_PATH: &str = "/projects/demo";
/// Authenticated shell cwd (muted left chrome). Home-style absolute form from product shots.
pub const DEMO_CWD: &str = "~/projects/demo";
/// Neutral workspace path for pure-dump auth/login chrome (no product-name residual).
pub const AUTH_PATH: &str = "/projects/demo";
/// Deterministic context budget for authenticated demo shells (`fmt_tokens` form).
pub const DEMO_CTX_USED: &str = "9.5K";
pub const DEMO_CTX_TOTAL: &str = "500K";

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

const CANVAS_RIGHT: u16 = 120;

/// Build the default right-aligned model chip (`Cortex (high) · always-approve`).
pub fn default_model_chip() -> String {
    format!("{MODEL_LABEL} ({MODEL_EFFORT}) · {MODEL_MODE_DEFAULT}")
}

/// Build a model chip with an explicit effort tier and permission mode.
pub fn model_chip(model: &str, effort: Option<&str>, mode: &str) -> String {
    match effort {
        Some(e) if !e.is_empty() => format!("{model} ({e}) · {mode}"),
        _ => format!("{model} · {mode}"),
    }
}

/// Product turn timer (source `format_duration` / `format_turn_timer`):
/// - `<10s` → one decimal (`4.0s`, `1.4s`, `0.5s`)
/// - `10–59s` → whole seconds (`15s`, `59s`)
/// - `≥60s` → `1m20s` / `1h2m` (never `0mNNs` for under a minute)
pub fn format_turn_timer(total_secs_f64: f64) -> String {
    let total_secs = total_secs_f64.max(0.0).floor() as u64;
    if total_secs < 10 {
        return format!("{:.1}s", total_secs_f64.max(0.0));
    }
    if total_secs < 60 {
        return format!("{total_secs}s");
    }
    let mins = total_secs / 60;
    let secs = total_secs % 60;
    if mins < 60 {
        return format!("{mins}m{secs}s");
    }
    let hours = mins / 60;
    let remaining_mins = mins % 60;
    format!("{hours}h{remaining_mins}m")
}

/// Compact context budget string (`9.5K / 500K`).
pub fn format_context_budget(used: &str, total: &str) -> String {
    format!("{used} / {total}")
}

/// Pre-auth / login-allowed branch + path top bar.
pub fn paint_top_branch_bar(
    canvas: &mut Canvas,
    branch: &str,
    path: &str,
) -> Result<(), String> {
    let line = format!("   {branch} {path}");
    canvas.write(1, 0, &line, TextStyle::bold(ColorToken::Secondary))?;
    Ok(())
}

/// Authenticated top chrome: muted cwd left + context budget right.
///
/// Product form: `~/projects/demo` .............. `9.5K / 500K`
/// Branch-only chrome is NOT sufficient for post-login shells; use
/// [`paint_top_branch_bar`] only for pre-auth login/welcome surfaces.
pub fn paint_auth_top_bar(
    canvas: &mut Canvas,
    cwd: &str,
    used: &str,
    total: &str,
) -> Result<(), String> {
    let left = format!("  {cwd}");
    canvas.write(0, 0, &left, TextStyle::plain(ColorToken::Muted))?;
    let budget = format_context_budget(used, total);
    let col = CANVAS_RIGHT.saturating_sub(budget.chars().count() as u16 + 2);
    canvas.write(0, col, &budget, TextStyle::plain(ColorToken::Muted))?;
    Ok(())
}

/// Authenticated demo top bar (`~/projects/demo` + `9.5K / 500K`).
pub fn paint_demo_top_bar(canvas: &mut Canvas) -> Result<(), String> {
    paint_auth_top_bar(canvas, DEMO_CWD, DEMO_CTX_USED, DEMO_CTX_TOTAL)
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

/// Idle prompt jewel: draft left + right-aligned model chip **on the draft row**.
///
/// Product form:
/// ```text
/// ╭──────────────────────────────────────────────────────────╮
/// │ ❯ <draft>                    Cortex (high) · always-approve │
/// ╰──────────────────────────────────────────────────────────╯
/// ```
/// No under-draft-only chip stack. No invented `Draft · N lines` meta.
pub fn paint_prompt_jewel(
    canvas: &mut Canvas,
    box_top: u16,
    draft: &str,
    model: &str,
    mode: &str,
) -> Result<(), String> {
    paint_prompt_jewel_with_effort(canvas, box_top, draft, model, Some(MODEL_EFFORT), mode)
}

/// Prompt jewel with optional explicit effort tier (None omits `(high)`-style label).
pub fn paint_prompt_jewel_with_effort(
    canvas: &mut Canvas,
    box_top: u16,
    draft: &str,
    model: &str,
    effort: Option<&str>,
    mode: &str,
) -> Result<(), String> {
    let box_bottom = box_top + 2;
    let left = 2u16;
    let right = 117u16;
    let width = (right - left + 1) as usize;
    let top = format!("╭{}╮", "─".repeat(width - 2));
    let bottom = format!("╰{}╯", "─".repeat(width - 2));
    canvas.write(box_top, left, &top, TextStyle::plain(ColorToken::Border))?;
    canvas.write(box_bottom, left, &bottom, TextStyle::plain(ColorToken::Border))?;

    let draft_row = box_top + 1;
    canvas.put(draft_row, left, '│', TextStyle::plain(ColorToken::Border))?;
    canvas.put(draft_row, right, '│', TextStyle::plain(ColorToken::Border))?;

    let arrow_style = TextStyle::bold(ColorToken::Accent);
    canvas.put(draft_row, left + 2, PROMPT_ARROW, arrow_style)?;
    canvas.write(
        draft_row,
        left + 4,
        draft,
        TextStyle::plain(ColorToken::Text),
    )?;

    let chip = model_chip(model, effort, mode);
    // Two-cell inset from the right border, right-aligned on the draft row.
    let chip_cols = chip.chars().count() as u16;
    let chip_col = right.saturating_sub(2 + chip_cols);
    let draft_end = left + 4 + draft.chars().count() as u16;
    if chip_col <= draft_end + 1 {
        return Err(format!(
            "prompt jewel chip overflows draft: draft ends at {draft_end}, chip at {chip_col}"
        ));
    }
    canvas.write(
        draft_row,
        chip_col,
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
    paint_prompt_jewel(
        canvas,
        box_top,
        draft,
        MODEL_LABEL,
        MODEL_MODE_DEFAULT,
    )
}

/// Login / device-auth quit affordance (localized, not global putty).
pub fn paint_login_quit(canvas: &mut Canvas) -> Result<(), String> {
    canvas.write_centered(33, "ctrl+d  quit", TextStyle::plain(ColorToken::Dim))?;
    Ok(())
}

/// Full-width user wash with inline `❯ <prompt text>` (+ optional right timestamp).
///
/// BAN: `❯ You` / `> You` speaker nameplates. The wash **is** the user turn;
/// the first body line is the prompt text following the arrow.
pub fn paint_user_turn(
    canvas: &mut Canvas,
    start_row: u16,
    body: &[&str],
) -> Result<(), String> {
    paint_user_turn_with_timestamp(canvas, start_row, body, None)
}

/// User turn with optional right-aligned timestamp (e.g. `8:19 AM`).
pub fn paint_user_turn_with_timestamp(
    canvas: &mut Canvas,
    start_row: u16,
    body: &[&str],
    timestamp: Option<&str>,
) -> Result<(), String> {
    let user_style = TextStyle::user_turn();
    let wash_end = 118u16;
    // Primary prompt line: full-width wash + `❯ <text>`.
    canvas.fill_row_bg(start_row, 0, wash_end, user_style)?;
    let prompt = body.first().copied().unwrap_or("");
    let head = format!("{PROMPT_ARROW} {prompt}");
    canvas.write(start_row, 2, &head, user_style)?;
    if let Some(ts) = timestamp {
        let col = wash_end.saturating_sub(ts.chars().count() as u16 + 1);
        // Avoid clobbering the prompt text when space is tight.
        let head_end = 2 + head.chars().count() as u16;
        if col > head_end + 1 {
            canvas.write(start_row, col, ts, TextStyle::plain(ColorToken::Muted))?;
            // Reapply wash background under the timestamp glyph cells.
            if let Some(cell) = canvas
                .cells
                .iter_mut()
                .find(|c| c.row == start_row && c.column == col)
            {
                cell.style.background = Some(ColorToken::Surface);
            }
        }
    }
    // Continuation lines stay on the wash without a nameplate.
    for (i, line) in body.iter().skip(1).enumerate() {
        let r = start_row + 1 + i as u16;
        canvas.fill_row_bg(r, 0, wash_end, user_style)?;
        canvas.write(r, 4, line, user_style)?;
    }
    Ok(())
}

/// Thought / group diamond header: `◆ Thought for Xs` (dim magenta).
///
/// Diamonds belong on Thought/group lines only — never as a `◆ Cortex` nameplate.
pub fn paint_thought_line(
    canvas: &mut Canvas,
    row: u16,
    time_label: &str,
) -> Result<(), String> {
    let line = format!("◆ Thought for {time_label}");
    canvas.write(row, 2, &line, TextStyle::plain(ColorToken::Accent))?;
    Ok(())
}

/// Green primary markdown header (product heading ladder).
pub fn paint_md_primary_header(
    canvas: &mut Canvas,
    row: u16,
    text: &str,
) -> Result<(), String> {
    canvas.write(row, 2, text, TextStyle::bold(ColorToken::Ok))?;
    Ok(())
}

/// Blue secondary markdown header (product heading ladder).
pub fn paint_md_secondary_header(
    canvas: &mut Canvas,
    row: u16,
    text: &str,
) -> Result<(), String> {
    canvas.write(row, 2, text, TextStyle::bold(ColorToken::Info))?;
    Ok(())
}

/// Assistant body stream — **no** `◆ Cortex` nameplate.
///
/// Body lines are plain text starting at `start_row`. Prefer
/// [`paint_thought_line`] for thinking, and the green/blue header helpers for
/// markdown hierarchy, before body content.
pub fn paint_assistant(
    canvas: &mut Canvas,
    start_row: u16,
    body: &[&str],
) -> Result<(), String> {
    for (i, line) in body.iter().enumerate() {
        canvas.write(
            start_row + i as u16,
            2,
            line,
            TextStyle::plain(ColorToken::Text),
        )?;
    }
    Ok(())
}

/// Convenience: Thought line + green primary + body (common dense answer stack).
pub fn paint_assistant_with_thought(
    canvas: &mut Canvas,
    start_row: u16,
    thought_time: Option<&str>,
    primary_header: Option<&str>,
    body: &[&str],
) -> Result<(), String> {
    let mut row = start_row;
    if let Some(t) = thought_time {
        paint_thought_line(canvas, row, t)?;
        row += 1;
    }
    if let Some(h) = primary_header {
        paint_md_primary_header(canvas, row, h)?;
        row += 1;
    }
    paint_assistant(canvas, row, body)?;
    Ok(())
}

/// Compact tool activity row: product verb + detail with **single** space.
///
/// Prefer Cortex product forms via helpers below (`$ cmd`, `Run …`, `Read path`,
/// `Search "pat" (N matches)`, `Edit path`). Do **not** invent `Shell  cmd`.
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
        format!("  {verb} {detail}")
    };
    canvas.write(row, 0, &line, style)?;
    Ok(())
}

/// Preformatted product tool activity line (indent + exact header string).
///
/// Use for full product headers:
/// - `$ cargo test …`
/// - `Run cargo test …`
/// - `Search "rate.?limit" (3 matches)`
/// - `Read crates/agent/src/retry.rs`
/// - `Edit crates/agent/src/retry.rs +12/-4`
pub fn paint_tool_activity(
    canvas: &mut Canvas,
    row: u16,
    header: &str,
    style: TextStyle,
) -> Result<(), String> {
    canvas.write(row, 0, &format!("  {header}"), style)?;
    Ok(())
}

/// Execute / shell-style primary header: `$ {command}`.
pub fn paint_execute_shell_header(
    canvas: &mut Canvas,
    row: u16,
    command: &str,
    style: TextStyle,
) -> Result<(), String> {
    paint_tool_activity(canvas, row, &format!("$ {command}"), style)
}

/// Label-style execute header: `Run {description_or_command}`.
pub fn paint_execute_run_header(
    canvas: &mut Canvas,
    row: u16,
    title: &str,
    style: TextStyle,
) -> Result<(), String> {
    paint_tool_activity(canvas, row, &format!("Run {title}"), style)
}

/// Search tool product form: `Search "{pattern}" (N matches[…])`.
pub fn paint_search_header(
    canvas: &mut Canvas,
    row: u16,
    pattern: &str,
    summary: &str,
    style: TextStyle,
) -> Result<(), String> {
    // Summary is parenthetical, e.g. `(3 matches)` / `(3 matches in 2 files)`.
    let header = if summary.is_empty() {
        format!("Search \"{pattern}\"")
    } else {
        format!("Search \"{pattern}\" {summary}")
    };
    paint_tool_activity(canvas, row, &header, style)
}

/// Read tool product form: `Read {path}` (+ optional range suffix).
pub fn paint_read_header(
    canvas: &mut Canvas,
    row: u16,
    path: &str,
    range_suffix: Option<&str>,
    style: TextStyle,
) -> Result<(), String> {
    let header = match range_suffix {
        Some(r) if !r.is_empty() => format!("Read {path} {r}"),
        _ => format!("Read {path}"),
    };
    paint_tool_activity(canvas, row, &header, style)
}

/// Edit tool product form: `Edit {path}` (+ optional diffstat / edits suffix).
pub fn paint_edit_header(
    canvas: &mut Canvas,
    row: u16,
    path: &str,
    suffix: Option<&str>,
    style: TextStyle,
) -> Result<(), String> {
    let header = match suffix {
        Some(s) if !s.is_empty() => format!("Edit {path} {s}"),
        _ => format!("Edit {path}"),
    };
    paint_tool_activity(canvas, row, &header, style)
}

/// Nested tool meta / body line under a header (dim cell hang, no Claude `⎿`).
pub fn paint_tool_body(
    canvas: &mut Canvas,
    row: u16,
    text: &str,
    style: TextStyle,
) -> Result<(), String> {
    canvas.write(row, 4, text, style)?;
    Ok(())
}

/// Legacy elbow detail under tool headers. Prefer [`paint_tool_body`] for pure product paint.
pub fn paint_tool_elbow(
    canvas: &mut Canvas,
    row: u16,
    text: &str,
    style: TextStyle,
) -> Result<(), String> {
    // Keep `⎿` available for residual Claude-family gates, but tools pure-structure
    // frames should not invent elbows as the primary meta channel.
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

/// Title · tradeoff picker row. Focus uses `›` + accent (never filled SELBG).
pub fn paint_picker_row(
    canvas: &mut Canvas,
    row: u16,
    title: &str,
    tradeoff: &str,
    selected: bool,
) -> Result<(), String> {
    if selected {
        canvas.write(
            row,
            4,
            &format!("› {title}"),
            TextStyle::bold(ColorToken::Accent),
        )?;
        if !tradeoff.is_empty() {
            canvas.write(
                row + 1,
                6,
                tradeoff,
                TextStyle::plain(ColorToken::Muted),
            )?;
        }
    } else {
        canvas.write(
            row,
            4,
            &format!("  {title}"),
            TextStyle::plain(ColorToken::Text),
        )?;
        if !tradeoff.is_empty() {
            canvas.write(
                row + 1,
                6,
                tradeoff,
                TextStyle::plain(ColorToken::Dim),
            )?;
        }
    }
    Ok(())
}

/// Compact single-line picker rows packing (title · tradeoff).
pub fn paint_picker_row_compact(
    canvas: &mut Canvas,
    row: u16,
    title: &str,
    tradeoff: &str,
    selected: bool,
) -> Result<(), String> {
    let label = if tradeoff.is_empty() {
        title.to_owned()
    } else {
        format!("{title} · {tradeoff}")
    };
    if selected {
        canvas.write(
            row,
            4,
            &format!("› {label}"),
            TextStyle::bold(ColorToken::Accent),
        )?;
    } else {
        canvas.write(
            row,
            4,
            &format!("  {label}"),
            TextStyle::plain(ColorToken::Text),
        )?;
    }
    Ok(())
}

/// Product slash_dropdown suggestion row: `❯ label` + aligned description.
///
/// Source: `/tmp/grok-build-ref/.../views/slash_dropdown.rs` — PREFIX_W=2 (`❯ `/`  `),
/// label column + gap + description. Selection uses Accent `❯` (canvas has no bg_visual fill).
/// Glue these immediately above the live prompt jewel; do not invent a detached
/// Commands essay or Filter putty rewrite.
pub fn paint_slash_suggestion_row(
    canvas: &mut Canvas,
    row: u16,
    label: &str,
    description: &str,
    selected: bool,
    label_col_w: usize,
) -> Result<(), String> {
    let left = 4u16;
    let prefix = if selected { "❯ " } else { "  " };
    let prefix_style = if selected {
        TextStyle::bold(ColorToken::Accent)
    } else {
        TextStyle::plain(ColorToken::Text)
    };
    let label_style = if selected {
        TextStyle::bold(ColorToken::Bright)
    } else {
        TextStyle::plain(ColorToken::Text)
    };
    canvas.write(row, left, prefix, prefix_style)?;
    canvas.write(row, left + 2, label, label_style)?;
    if !description.is_empty() {
        let desc_col = left + 2 + label_col_w as u16 + 2;
        // Keep description inside the inner frame (col ~115).
        if desc_col < 115 {
            canvas.write(
                row,
                desc_col,
                description,
                TextStyle::plain(ColorToken::Muted),
            )?;
        }
    }
    Ok(())
}

/// Paint slash/model arg suggestions glued above a live prompt jewel.
///
/// `prompt_draft` is the in-progress slash query (e.g. `/m`, `/model `, `/model Cortex `).
/// The jewel remains the anchored composer, not a Filter · / Selected · invent dock.
pub fn paint_slash_dropdown_over_prompt(
    canvas: &mut Canvas,
    first_row: u16,
    rows: &[(&str, &str)],
    selected: usize,
    prompt_draft: &str,
) -> Result<(), String> {
    let label_col_w = rows
        .iter()
        .map(|(label, _)| label.chars().count())
        .max()
        .unwrap_or(8)
        .min(40);
    for (i, (label, desc)) in rows.iter().enumerate() {
        paint_slash_suggestion_row(
            canvas,
            first_row + i as u16,
            label,
            desc,
            i == selected,
            label_col_w,
        )?;
    }
    // One blank separator row, then the live prompt jewel (3 rows: border+draft+border).
    let jewel_top = first_row + rows.len() as u16 + 1;
    // Draft keeps model chip right-aligned when the prompt is not mid-slash; slash
    // queries keep the chip so authenticated shell chrome remains product-true.
    paint_prompt_jewel(
        canvas,
        jewel_top,
        prompt_draft,
        MODEL_LABEL,
        MODEL_MODE_DEFAULT,
    )?;
    Ok(())
}

/// Bottom action dock frame replacing the idle composer for pickers/permissions.
pub fn paint_dock_frame(
    canvas: &mut Canvas,
    box_top: u16,
    height: u16,
    title: &str,
    byline: &str,
) -> Result<(), String> {
    let box_bottom = box_top + height.saturating_sub(1);
    let left = 2u16;
    let right = 117u16;
    let width = (right - left + 1) as usize;
    let top = format!("╭{}╮", "─".repeat(width - 2));
    let bottom = format!("╰{}╯", "─".repeat(width - 2));
    canvas.write(box_top, left, &top, TextStyle::plain(ColorToken::Border))?;
    canvas.write(box_bottom, left, &bottom, TextStyle::plain(ColorToken::Border))?;
    for r in (box_top + 1)..box_bottom {
        canvas.put(r, left, '│', TextStyle::plain(ColorToken::Border))?;
        canvas.put(r, right, '│', TextStyle::plain(ColorToken::Border))?;
    }
    if !title.is_empty() {
        canvas.write(
            box_top + 1,
            left + 2,
            title,
            TextStyle::bold(ColorToken::Bright),
        )?;
    }
    if !byline.is_empty() {
        canvas.write(
            box_bottom.saturating_sub(1),
            left + 2,
            byline,
            TextStyle::plain(ColorToken::Dim),
        )?;
    }
    Ok(())
}

/// Multi-line prompt jewel for composer drafts.
///
/// Chip is right-aligned on the first draft row. Drop invented `Draft · N` lines.
pub fn paint_prompt_jewel_multiline(
    canvas: &mut Canvas,
    box_top: u16,
    lines: &[&str],
    model: &str,
    mode: &str,
) -> Result<(), String> {
    let body_lines = lines.len().max(1) as u16;
    let box_bottom = box_top + body_lines + 1; // top border + body + bottom border
    let left = 2u16;
    let right = 117u16;
    let width = (right - left + 1) as usize;
    let top = format!("╭{}╮", "─".repeat(width - 2));
    let bottom = format!("╰{}╯", "─".repeat(width - 2));
    canvas.write(box_top, left, &top, TextStyle::plain(ColorToken::Border))?;
    canvas.write(box_bottom, left, &bottom, TextStyle::plain(ColorToken::Border))?;
    for r in (box_top + 1)..box_bottom {
        canvas.put(r, left, '│', TextStyle::plain(ColorToken::Border))?;
        canvas.put(r, right, '│', TextStyle::plain(ColorToken::Border))?;
    }
    let arrow_style = TextStyle::bold(ColorToken::Accent);
    let chip = model_chip(model, Some(MODEL_EFFORT), mode);
    let chip_cols = chip.chars().count() as u16;
    let chip_col = right.saturating_sub(2 + chip_cols);
    for (i, line) in lines.iter().enumerate() {
        let r = box_top + 1 + i as u16;
        if i == 0 {
            canvas.put(r, left + 2, PROMPT_ARROW, arrow_style)?;
            canvas.write(r, left + 4, line, TextStyle::plain(ColorToken::Text))?;
            // Right-align model chip on the first (draft) row inside the border.
            let draft_end = left + 4 + line.chars().count() as u16;
            if chip_col > draft_end + 1 {
                canvas.write(r, chip_col, &chip, TextStyle::plain(ColorToken::Muted))?;
            } else {
                return Err(format!(
                    "multiline jewel chip overflows draft: draft ends at {draft_end}, chip at {chip_col}"
                ));
            }
        } else {
            canvas.write(r, left + 4, line, TextStyle::plain(ColorToken::Text))?;
        }
    }
    if lines.is_empty() {
        let r = box_top + 1;
        canvas.put(r, left + 2, PROMPT_ARROW, arrow_style)?;
        canvas.write(r, chip_col, &chip, TextStyle::plain(ColorToken::Muted))?;
    }
    Ok(())
}

/// Quiet conversation backdrop reused by pickers/permissions (origin continuity).
pub fn paint_quiet_origin(
    canvas: &mut Canvas,
    user: &str,
    partial: &str,
) -> Result<(), String> {
    paint_demo_top_bar(canvas)?;
    paint_user_turn(canvas, 3, &[user])?;
    if !partial.is_empty() {
        // No ◆ Cortex nameplate — dim body stream only.
        canvas.write(7, 2, partial, TextStyle::plain(ColorToken::Dim))?;
    }
    Ok(())
}

/// Detail-view header for full-screen views.
pub fn paint_detail_header(
    canvas: &mut Canvas,
    title: &str,
    subtitle: &str,
) -> Result<(), String> {
    paint_demo_top_bar(canvas)?;
    canvas.write(3, 4, title, TextStyle::bold(ColorToken::Bright))?;
    if !subtitle.is_empty() {
        canvas.write(4, 4, subtitle, TextStyle::plain(ColorToken::Muted))?;
    }
    Ok(())
}

/// Return / putty footer pair for temporary detail views.
pub fn paint_detail_footer(
    canvas: &mut Canvas,
    return_byline: &str,
) -> Result<(), String> {
    canvas.write(
        37,
        4,
        return_byline,
        TextStyle::plain(ColorToken::Dim),
    )?;
    paint_default_putty(canvas)
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
        "Claude Sonnet",
        "Ask Code",
        "Work mode",
        "How Code should work",
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

/// Pure-structure residual scan for shared shell anti-patterns (nameplates, etc.).
///
/// Covers VAL-V3-STRUCT-001..004 nameplate/timer inventory bans plus Mission
/// Control invent strings from pure-structure-law hard ban #8 so inventory-wide
/// CROSS-028 scans fail closed on residual frames.
pub fn reject_structure_residual(text: &str) -> Result<(), String> {
    if text.contains("❯ You") || text.contains("> You") {
        return Err("pure-structure residual refused: You nameplate".to_owned());
    }
    if text.contains("◆ Cortex") {
        return Err("pure-structure residual refused: Cortex nameplate".to_owned());
    }
    if text.contains("Draft ·") {
        return Err("pure-structure residual refused: Draft·N invent".to_owned());
    }
    // Gallery/meta monologue invents (pure-structure-law hard ban #7).
    for monologue in [
        "Multi-line jewel",
        "expanded thinking kept collapsed",
        "statusline promises",
        "putty lectures",
        "jump with timeline rail",
    ] {
        if text.contains(monologue) {
            return Err(format!(
                "pure-structure residual refused: gallery monologue {monologue:?}"
            ));
        }
    }
    // Mission Control invent under Cortex labels (pure-structure-law hard ban #8).
    // MC parallel pack (2026-07-21) may paint Factory "Progress Log" (capital L)
    // and `.: Mission Control` brand; those are NOT invent residuals.
    // Still banned: lowercase "Progress log", "Tasks monitor", Owner kanban, etc.
    for invent in [
        "Tasks monitor",
        "Goals board",
        "i intervene",
        "Progress log",
        "Esc to tasks monitor",
    ] {
        if text.contains(invent) {
            return Err(format!(
                "pure-structure residual refused: mission invent {invent:?}"
            ));
        }
    }
    // Bare double-space Shell execute header invent (product uses `$ cmd` / `Run …`).
    if text.contains("Shell  ") {
        return Err("pure-structure residual refused: Shell invent header".to_owned());
    }
    // Zero-padded minute form for sub-minute timers (e.g. 0m04s).
    if text.split_whitespace().any(|tok| {
        tok.len() >= 4
            && tok.starts_with("0m")
            && tok.ends_with('s')
            && tok[2..tok.len() - 1].chars().all(|c| c.is_ascii_digit())
    }) {
        return Err("pure-structure residual refused: 0mNNs timer form".to_owned());
    }
    Ok(())
}

/// IDs that may omit authenticated context budget (login / device / provider auth family).
/// CROSS-028: authenticated shells must carry budget form; auth surfaces may omit it.
pub fn is_auth_family_screen(id: &str) -> bool {
    matches!(
        id,
        "login"
            | "connect-provider"
            | "connect-method"
            | "connect-oauth"
            | "connect-apikey"
            | "connect-connected"
            | "connect-disconnect"
    ) || id.starts_with("connect-")
        || id.contains("login")
}

/// True when text carries product context budget form (`9.5K / 500K`, `8.5K / 1.0M`, …).
pub fn has_context_budget_form(text: &str) -> bool {
    // Preferred compact product digests.
    if text.contains("K / ") || text.contains("M / ") {
        return true;
    }
    // Fallback line scan: used/total with KM units around a slash.
    text.lines().any(|line| {
        line.contains('/')
            && (line.contains('K') || line.contains('M'))
            && line.chars().any(|c| c.is_ascii_digit())
    })
}

/// Product putty footer separators for authenticated shells must not be
/// exclusive middot-only dynasty (`Shift+Tab:mode · Ctrl+c…` without `|`).
///
/// True when every product putty law line uses `|` / `│` separators, or when no
/// product putty law line is claimed at all (pre-auth / modal-only may omit it).
pub fn putty_product_separators_hold(text: &str) -> bool {
    for line in text.lines() {
        let trimmed = line.trim();
        // Product putty law lines always name both mode cycle and cancel.
        let is_product_putty = trimmed.contains("Shift+Tab:mode")
            && (trimmed.contains("Ctrl+c") || trimmed.contains("Ctrl+c:cancel"));
        if !is_product_putty {
            continue;
        }
        if trimmed.contains('|') || trimmed.contains('│') {
            continue;
        }
        // Exclusive middot dynasty on a product putty law line fails CROSS-028.
        if trimmed.contains('·') {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod pure_structure_tests {
    use super::*;
    use crate::chrome::tokens::Theme;

    #[test]
    fn format_turn_timer_product_rule() {
        assert_eq!(format_turn_timer(0.5), "0.5s");
        assert_eq!(format_turn_timer(4.0), "4.0s");
        assert_eq!(format_turn_timer(1.4), "1.4s");
        assert_eq!(format_turn_timer(9.9), "9.9s");
        assert_eq!(format_turn_timer(10.0), "10s");
        assert_eq!(format_turn_timer(15.0), "15s");
        assert_eq!(format_turn_timer(59.0), "59s");
        assert_eq!(format_turn_timer(60.0), "1m0s");
        assert_eq!(format_turn_timer(80.0), "1m20s");
        assert_eq!(format_turn_timer(600.0), "10m0s");
        // Never emit 0mNNs for under 60s.
        for secs in [0.0_f64, 1.0, 4.0, 12.0, 32.0, 59.0] {
            let s = format_turn_timer(secs);
            assert!(
                !s.starts_with("0m"),
                "under-60s must not use 0m form, got {s} for {secs}"
            );
        }
    }

    #[test]
    fn auth_top_bar_emits_cwd_and_context_budget() {
        let mut canvas = Canvas::new(Theme::CortexNight);
        paint_demo_top_bar(&mut canvas).unwrap();
        let text = canvas.to_text();
        let line0 = text.lines().next().unwrap();
        assert!(
            line0.contains(DEMO_CWD),
            "missing cwd: {line0:?}"
        );
        assert!(
            line0.contains("9.5K / 500K"),
            "missing context budget: {line0:?}"
        );
        // Branch-only form is not the authenticated primary bar.
        assert!(!line0.contains(''), "auth top must not be branch-only: {line0:?}");
        // Budget sits right of the left-side cwd (column probe).
        let cwd_pos = line0.find(DEMO_CWD).unwrap();
        let budget_pos = line0.find("9.5K / 500K").unwrap();
        assert!(budget_pos > cwd_pos, "budget must be right of cwd");
    }

    #[test]
    fn pre_auth_branch_bar_still_available() {
        let mut canvas = Canvas::new(Theme::CortexNight);
        paint_top_branch_bar(&mut canvas, DEMO_BRANCH, DEMO_PATH).unwrap();
        let text = canvas.to_text();
        assert!(text.contains(''));
        assert!(text.contains(DEMO_BRANCH));
        assert!(text.contains(DEMO_PATH));
    }

    #[test]
    fn user_turn_has_no_you_nameplate() {
        let mut canvas = Canvas::new(Theme::CortexNight);
        paint_user_turn(
            &mut canvas,
            4,
            &["Explain the race briefly.", "Keep it short."],
        )
        .unwrap();
        let text = canvas.to_text();
        assert!(!text.contains("❯ You"));
        assert!(!text.contains("> You"));
        assert!(text.contains("❯ Explain the race briefly."));
        assert!(text.contains("Keep it short."));
        reject_structure_residual(&text).unwrap();
    }

    #[test]
    fn user_turn_optional_timestamp_right() {
        let mut canvas = Canvas::new(Theme::CortexNight);
        paint_user_turn_with_timestamp(
            &mut canvas,
            5,
            &["Ship the retry guard."],
            Some("8:19 AM"),
        )
        .unwrap();
        let line = canvas.to_text().lines().nth(5).unwrap().to_string();
        assert!(line.contains("❯ Ship the retry guard."));
        assert!(line.contains("8:19 AM"));
        let prompt_pos = line.find("Ship the retry guard.").unwrap();
        let ts_pos = line.find("8:19 AM").unwrap();
        assert!(ts_pos > prompt_pos);
    }

    #[test]
    fn assistant_has_no_assistant_nameplate() {
        let mut canvas = Canvas::new(Theme::CortexNight);
        paint_assistant(
            &mut canvas,
            8,
            &["Two writers can skip the watched counter."],
        )
        .unwrap();
        let text = canvas.to_text();
        assert!(!text.contains("◆ Cortex"));
        assert!(text.contains("Two writers can skip the watched counter."));
        reject_structure_residual(&text).unwrap();
    }

    #[test]
    fn thought_line_uses_diamond_not_nameplate() {
        let mut canvas = Canvas::new(Theme::CortexNight);
        paint_thought_line(&mut canvas, 7, "4.0s").unwrap();
        let text = canvas.to_text();
        assert!(text.contains("◆ Thought for 4.0s"));
        assert!(!text.contains("◆ Cortex"));
    }

    #[test]
    fn assistant_header_styles_green_and_blue() {
        let mut canvas = Canvas::new(Theme::CortexNight);
        paint_md_primary_header(&mut canvas, 9, "Recovery procedure").unwrap();
        paint_md_secondary_header(&mut canvas, 10, "Why it failed").unwrap();
        paint_assistant(&mut canvas, 11, &["Clear temp keys, then re-run."]).unwrap();
        let text = canvas.to_text();
        assert!(text.contains("Recovery procedure"));
        assert!(text.contains("Why it failed"));
        // Style probes via cell styles.
        let primary = canvas
            .cells
            .iter()
            .find(|c| c.row == 9 && c.glyph == 'R')
            .expect("primary header cell");
        assert_eq!(primary.style.foreground, ColorToken::Ok);
        assert!(primary.style.bold);
        let secondary = canvas
            .cells
            .iter()
            .find(|c| c.row == 10 && c.glyph == 'W')
            .expect("secondary header cell");
        assert_eq!(secondary.style.foreground, ColorToken::Info);
    }

    #[test]
    fn prompt_jewel_right_aligns_model_chip_on_draft_row() {
        let mut canvas = Canvas::new(Theme::CortexNight);
        paint_default_prompt(&mut canvas, 33, "Ask Cortex to start a task…").unwrap();
        let text = canvas.to_text();
        let draft_row = text.lines().nth(34).expect("draft row");
        assert!(
            draft_row.contains(PROMPT_ARROW),
            "missing arrow: {draft_row:?}"
        );
        assert!(
            draft_row.contains("Ask Cortex to start a task…"),
            "missing draft: {draft_row:?}"
        );
        let chip = default_model_chip();
        assert!(
            draft_row.contains(&chip),
            "chip must sit on draft row, got: {draft_row:?}"
        );
        // Right-aligned: chip starts later than draft text.
        let draft_pos = draft_row.find("Ask Cortex to start a task…").unwrap();
        let chip_pos = draft_row.find(&chip).unwrap();
        assert!(chip_pos > draft_pos);
        // No second meta row invent under draft.
        let meta_row = text.lines().nth(35).unwrap();
        assert!(
            !meta_row.contains(MODEL_LABEL),
            "model must not stack under draft only: {meta_row:?}"
        );
        assert!(!text.contains("Draft ·"));
        reject_structure_residual(&text).unwrap();
    }

    #[test]
    fn multiline_jewel_drops_draft_n_invent() {
        let mut canvas = Canvas::new(Theme::CortexNight);
        paint_prompt_jewel_multiline(
            &mut canvas,
            30,
            &["first line of draft", "second continues"],
            MODEL_LABEL,
            MODEL_MODE_DEFAULT,
        )
        .unwrap();
        let text = canvas.to_text();
        assert!(!text.contains("Draft ·"));
        assert!(text.contains(&default_model_chip()) || text.contains("Cortex (high)"));
        let row = text.lines().nth(31).unwrap();
        assert!(row.contains("first line of draft"));
        assert!(row.contains("always-approve"));
    }

    #[test]
    fn default_putty_uses_product_pipe_separators() {
        assert!(DEFAULT_PUTTY.contains(" | "));
        assert!(!DEFAULT_PUTTY.contains(" · "), "middot-only dynasty banned as primary");
        let mut canvas = Canvas::new(Theme::CortexNight);
        paint_default_putty(&mut canvas).unwrap();
        let text = canvas.to_text();
        let footer = text.lines().nth(39).unwrap();
        assert!(footer.contains("Shift+Tab:mode | Ctrl+c:cancel | Ctrl+.:shortcuts"));
        assert!(footer.contains('|'));
    }

    #[test]
    fn busy_turn_status_keeps_stop_and_product_timer() {
        let mut canvas = Canvas::new(Theme::CortexNight);
        let timer = format_turn_timer(15.0);
        assert_eq!(timer, "15s");
        paint_turn_status_busy(
            &mut canvas,
            31,
            &format!("Responding… {}", format_turn_timer(1.4)),
            &timer,
            "9.45k",
        )
        .unwrap();
        let text = canvas.to_text();
        let line = text.lines().nth(31).unwrap();
        assert!(line.contains("[stop]"));
        assert!(line.contains("15s"));
        assert!(line.contains("Responding… 1.4s"));
        assert!(!line.contains("0m"));
        reject_structure_residual(&text).unwrap();
    }

    #[test]
    fn format_context_budget_shape() {
        assert_eq!(format_context_budget("9.5K", "500K"), "9.5K / 500K");
        assert_eq!(format_context_budget("8.5K", "1.0M"), "8.5K / 1.0M");
        let re = regex_is_budget("9.5K / 500K");
        assert!(re);
        assert!(regex_is_budget("8.5K / 1.0M"));
    }

    fn regex_is_budget(s: &str) -> bool {
        // \d+(\.\d+)?[KM] / \d+(\.\d+)?[KM]
        let parts: Vec<&str> = s.split(" / ").collect();
        if parts.len() != 2 {
            return false;
        }
        fn tok(t: &str) -> bool {
            let bytes = t.as_bytes();
            if bytes.is_empty() {
                return false;
            }
            let unit = *bytes.last().unwrap() as char;
            if unit != 'K' && unit != 'M' {
                return false;
            }
            let num = &t[..t.len() - 1];
            !num.is_empty()
                && num
                    .chars()
                    .all(|c| c.is_ascii_digit() || c == '.')
                && num.chars().any(|c| c.is_ascii_digit())
        }
        tok(parts[0]) && tok(parts[1])
    }
}
