//! Pure conversation + shell states (F2).

use crate::chrome::canvas::Canvas;
use crate::chrome::components::{
    paint_assistant, paint_braille_logo, paint_default_prompt, paint_default_putty,
    paint_demo_top_bar, paint_dock_frame, paint_turn_status_busy, paint_user_turn,
    reject_copper_residual, MODEL_LABEL, PROMPT_ARROW,
};
use crate::chrome::tokens::{ColorToken, TextStyle, Theme};

pub fn render(id: &str) -> Result<Canvas, String> {
    match id {
        "home" => home(),
        "onboarding" => onboarding(),
        "chat-turn" => chat_turn(),
        "chat-long-scroll" => chat_long_scroll(),
        "streaming" => streaming(),
        "reasoning" => reasoning(),
        "reasoning-collapsed" => reasoning_collapsed(),
        "rich-response" => rich_response(),
        "empty" => empty(),
        "toasts" => toasts(),
        other => Err(format!("unknown conversation pure-Cortex id: {other}")),
    }
}

fn onboarding() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_demo_top_bar(&mut canvas)?;
    paint_braille_logo(&mut canvas, 4)?;
    canvas.write_centered(
        13,
        "Welcome to Cortex",
        TextStyle::bold(ColorToken::Bright),
    )?;
    canvas.write_centered(
        15,
        "Set up your first conversation",
        TextStyle::plain(ColorToken::Secondary),
    )?;
    canvas.write(
        18,
        4,
        "Choose the model Cortex will use when you begin.",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        19,
        4,
        &format!("Recommended: {MODEL_LABEL} · balanced everyday coding."),
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        21,
        4,
        "You can change it later. Sign-in can wait until you need cloud tools.",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        23,
        4,
        "Permission mode: always-approve · Shift+Tab cycles later.",
        TextStyle::plain(ColorToken::Secondary),
    )?;
    paint_dock_frame(
        &mut canvas,
        28,
        9,
        &format!("{MODEL_LABEL} · recommended starting model"),
        "Enter continues · s skips · m models · Esc back",
    )?;
    canvas.write(
        30,
        5,
        "› Continue",
        TextStyle::bold(ColorToken::Accent),
    )?;
    canvas.write(
        31,
        5,
        "  Skip with this safe default",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        32,
        5,
        "  Review other models",
        TextStyle::plain(ColorToken::Text),
    )?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn home() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_demo_top_bar(&mut canvas)?;
    paint_braille_logo(&mut canvas, 6)?;
    canvas.write_centered(
        15,
        "Cortex",
        TextStyle::bold(ColorToken::Bright),
    )?;
    canvas.write_centered(
        17,
        &format!("{MODEL_LABEL} · ready in this workspace"),
        TextStyle::plain(ColorToken::Secondary),
    )?;
    canvas.write_centered(
        20,
        "Tips: /model  /theme  /mcp  Shift+Tab for mode",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write_centered(
        22,
        "Type a goal or paste a stack trace to begin.",
        TextStyle::plain(ColorToken::Dim),
    )?;
    paint_default_prompt(&mut canvas, 33, "Ask Cortex to start a task…")?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn chat_turn() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_demo_top_bar(&mut canvas)?;
    paint_user_turn(
        &mut canvas,
        4,
        &[
            "Check the retry path before we merge.",
            "Keep the summary short and list one follow-up test.",
        ],
    )?;
    paint_assistant(
        &mut canvas,
        9,
        &[
            "Applied the retry guard and covered the failure path.",
            "Follow-up: add an integration case for rate-limit floors.",
        ],
    )?;
    canvas.write(13, 4, "Thought for 1s", TextStyle::plain(ColorToken::Accent))?;
    canvas.write(
        14,
        4,
        "◇ expanded thinking kept collapsed in this idle frame",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_default_prompt(&mut canvas, 33, "Add a note about the remaining test hunk…")?;
    paint_default_putty(&mut canvas)?;
    let text = canvas.to_text();
    if !text.contains(PROMPT_ARROW) {
        return Err("missing Cortex prompt arrow".to_owned());
    }
    if !text.contains(MODEL_LABEL) {
        return Err("missing Cortex model chrome".to_owned());
    }
    finish(canvas)
}

fn chat_long_scroll() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_demo_top_bar(&mut canvas)?;
    // Earlier turns compressed at top.
    canvas.write(3, 2, "❯ You", TextStyle::plain(ColorToken::Dim))?;
    canvas.write(4, 4, "Earlier: sketch the cache invalidation plan.", TextStyle::plain(ColorToken::Dim))?;
    canvas.write(5, 2, "◆ Cortex", TextStyle::plain(ColorToken::Dim))?;
    canvas.write(6, 4, "Outlined a two-phase write-through plan.", TextStyle::plain(ColorToken::Dim))?;
    canvas.write(8, 2, "❯ You", TextStyle::plain(ColorToken::Dim))?;
    canvas.write(9, 4, "Then tight-loop the HTTP client retries.", TextStyle::plain(ColorToken::Dim))?;
    canvas.write(10, 2, "◆ Cortex", TextStyle::plain(ColorToken::Dim))?;
    canvas.write(
        11,
        4,
        "Added jittered backoff and a ceiling helper.",
        TextStyle::plain(ColorToken::Dim),
    )?;
    // Recent turns full wash near composer.
    paint_user_turn(
        &mut canvas,
        14,
        &["Ship it. Summarize what landed and what's left."],
    )?;
    paint_assistant(
        &mut canvas,
        18,
        &[
            "Retry path is guarded; flush root is deterministic.",
            "Left: integration case for 429 floors + docs note.",
        ],
    )?;
    canvas.write(
        23,
        4,
        "↑ 4 earlier turns · jump with timeline rail",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_default_prompt(&mut canvas, 33, "")?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn streaming() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_demo_top_bar(&mut canvas)?;
    paint_user_turn(
        &mut canvas,
        4,
        &["Explain the race in the flush root briefly."],
    )?;
    canvas.write(8, 2, "◆ Cortex", TextStyle::accent())?;
    canvas.write(
        9,
        4,
        "Two writers can skip the watched counter when the",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        10,
        4,
        "snapshot freezes mid-batch. Guard with a sequence…",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(11, 4, "█", TextStyle::bold(ColorToken::Accent))?;
    paint_turn_status_busy(&mut canvas, 31, "Responding… 1.4s", "4.0s", "2.1k")?;
    paint_default_prompt(&mut canvas, 33, "")?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn reasoning() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_demo_top_bar(&mut canvas)?;
    paint_user_turn(
        &mut canvas,
        4,
        &["Why did the flaky suite pass only after shred?"],
    )?;
    canvas.write(8, 2, "◆ Cortex", TextStyle::accent())?;
    canvas.write(
        9,
        4,
        "Thought for 12s",
        TextStyle::bold(ColorToken::Accent),
    )?;
    canvas.write(
        10,
        4,
        "◇ The failed worker reused a stale group counter.",
        TextStyle::plain(ColorToken::Accent),
    )?;
    canvas.write(
        11,
        4,
        "◇ Force-clearing temp keys resets membership.",
        TextStyle::plain(ColorToken::Accent),
    )?;
    canvas.write(
        12,
        4,
        "◇ Plan: pin fixture ids and assert counter rebuild.",
        TextStyle::plain(ColorToken::Accent),
    )?;
    canvas.write(
        14,
        4,
        "The suite flattened once the group counter rebuilt",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        15,
        4,
        "from empty membership. Pin that as an explicit assert.",
        TextStyle::plain(ColorToken::Text),
    )?;
    paint_default_prompt(&mut canvas, 33, "")?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn reasoning_collapsed() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_demo_top_bar(&mut canvas)?;
    paint_user_turn(
        &mut canvas,
        4,
        &["Compress the long thinking from the last answer."],
    )?;
    canvas.write(8, 2, "◆ Cortex", TextStyle::accent())?;
    canvas.write(
        9,
        4,
        "◇ Thought for 12s · Enter to expand",
        TextStyle::plain(ColorToken::Accent),
    )?;
    canvas.write(
        11,
        4,
        "Pinned fixture ids and rebuilt the group counter",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        12,
        4,
        "before re-running the flaky suite.",
        TextStyle::plain(ColorToken::Text),
    )?;
    paint_default_prompt(&mut canvas, 33, "")?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn rich_response() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_demo_top_bar(&mut canvas)?;
    paint_user_turn(
        &mut canvas,
        3,
        &["Document the recovery procedure with steps and a sample."],
    )?;
    canvas.write(6, 2, "◆ Cortex", TextStyle::accent())?;
    canvas.write(7, 4, "Recovery procedure", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(
        9,
        4,
        "1. Stop the worker with [stop] or Ctrl+c.",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        10,
        4,
        "2. Clear temp keys that hold stale membership.",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        11,
        4,
        "3. Re-run the suite and confirm the counter rebuilds.",
        TextStyle::plain(ColorToken::Text),
    )?;
    // Code block bg surface.
    let code_style = TextStyle::on(ColorToken::Text, ColorToken::CodeBg);
    canvas.fill_row_bg(13, 4, 78, code_style)?;
    canvas.fill_row_bg(14, 4, 78, code_style)?;
    canvas.fill_row_bg(15, 4, 78, code_style)?;
    canvas.write(13, 6, "cargo test --test flush_root_retry", code_style)?;
    canvas.write(14, 6, "# expect: membership rebuilt · 0 flaky", code_style)?;
    canvas.write(15, 6, "cargo test --test rate_limit_floors", code_style)?;
    canvas.write(
        17,
        4,
        "See also: docs/recovery.md for the long form.",
        TextStyle::plain(ColorToken::Info),
    )?;
    paint_default_prompt(&mut canvas, 33, "")?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn empty() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_demo_top_bar(&mut canvas)?;
    canvas.write(
        12,
        4,
        "New agent shell",
        TextStyle::bold(ColorToken::Bright),
    )?;
    canvas.write(
        14,
        4,
        "No turns yet. Describe a goal or paste a stack trace.",
        TextStyle::plain(ColorToken::Secondary),
    )?;
    canvas.write(
        16,
        4,
        &format!("Model ready · {MODEL_LABEL} · always-approve"),
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_default_prompt(&mut canvas, 33, "What should Cortex work on?")?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn toasts() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_demo_top_bar(&mut canvas)?;
    paint_user_turn(
        &mut canvas,
        4,
        &["Save the export and maybe fire a toast."],
    )?;
    paint_assistant(
        &mut canvas,
        8,
        &["Export packed. Showing confirmations above the prompt."],
    )?;
    // Toast stack floating above prompt jewel.
    canvas.write(
        22,
        66,
        "╭────────────────────────────╮",
        TextStyle::plain(ColorToken::Border),
    )?;
    canvas.write(
        23,
        66,
        "│ ✓ Export saved             │",
        TextStyle::plain(ColorToken::Ok),
    )?;
    canvas.write(
        24,
        66,
        "│ · session-a1b2c3.md        │",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        25,
        66,
        "╰────────────────────────────╯",
        TextStyle::plain(ColorToken::Border),
    )?;
    canvas.write(
        27,
        66,
        "╭────────────────────────────╮",
        TextStyle::plain(ColorToken::Border),
    )?;
    canvas.write(
        28,
        66,
        "│ ✗ Clipboard blocked        │",
        TextStyle::plain(ColorToken::Bad),
    )?;
    canvas.write(
        29,
        66,
        "│ · grant terminal paste     │",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        30,
        66,
        "╰────────────────────────────╯",
        TextStyle::plain(ColorToken::Border),
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
    use crate::chrome::components::{MODEL_LABEL, PROMPT_ARROW};

    #[test]
    fn conversation_batch_pure_cortex() {
        for id in [
            "home",
            "onboarding",
            "chat-turn",
            "chat-long-scroll",
            "streaming",
            "reasoning",
            "reasoning-collapsed",
            "rich-response",
            "empty",
            "toasts",
        ] {
            let canvas = render(id).unwrap_or_else(|e| panic!("{id}: {e}"));
            let text = canvas.to_text();
            reject_copper_residual(&text).unwrap_or_else(|e| panic!("{id}: {e}"));
            assert_eq!(text.lines().count(), 40, "{id}");
            assert!(!text.contains("Claude Opus"), "{id}");
            assert!(!text.contains("Ask Code"), "{id}");
            if id == "chat-turn" {
                assert!(text.contains(PROMPT_ARROW));
                assert!(text.contains(MODEL_LABEL));
                assert!(text.contains("always-approve"));
            }
            if id == "streaming" {
                assert!(text.contains("[stop]"));
            }
        }
    }
}
