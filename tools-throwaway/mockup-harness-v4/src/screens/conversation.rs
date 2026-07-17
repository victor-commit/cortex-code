//! Pure conversation + shell states (F2).
//!
//! Pure-structure M1: shared chrome primitives + densified bodies matching the
//! official busy-chat DNA (`library/pure-structure-law.md`).

use crate::chrome::canvas::Canvas;
use crate::chrome::components::{
    default_model_chip, format_turn_timer, paint_assistant, paint_braille_logo,
    paint_default_prompt, paint_default_putty, paint_demo_top_bar, paint_dock_frame,
    paint_md_primary_header, paint_md_secondary_header, paint_thought_line,
    paint_turn_status_busy, paint_user_turn, paint_user_turn_with_timestamp,
    reject_copper_residual, reject_structure_residual, MODEL_LABEL, PROMPT_ARROW,
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
    // Pre-auth / first-run: authenticated demo top is still fine for post-login
    // empty shells; onboarding keeps demo cwd+budget for product continuity.
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

/// Dense idle completed turn: bare user wash, Thought, green/blue ladder body.
fn chat_turn() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_demo_top_bar(&mut canvas)?;
    paint_user_turn_with_timestamp(
        &mut canvas,
        3,
        &[
            "Check the retry path before we merge.",
            "Keep the summary short and list one follow-up test.",
        ],
        Some("8:19 AM"),
    )?;
    paint_thought_line(&mut canvas, 6, "1.2s")?;
    paint_md_primary_header(&mut canvas, 8, "Retry guard")?;
    paint_assistant(
        &mut canvas,
        9,
        &[
            "Applied the retry guard and covered the failure path in the flush root.",
            "The watcher now freezes the related sequence before the mid-batch skip.",
        ],
    )?;
    paint_md_secondary_header(&mut canvas, 12, "Follow-up")?;
    paint_assistant(
        &mut canvas,
        13,
        &[
            "- add an integration case for rate-limit floors",
            "- keep the suite assertion on membership rebuild",
        ],
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
    if !text.contains(&default_model_chip()) && !text.contains("Cortex (high)") {
        return Err("missing right-aligned effort chip".to_owned());
    }
    finish(canvas)
}

fn chat_long_scroll() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_demo_top_bar(&mut canvas)?;
    // Dim earlier turns use the same bare-user + assistant body form (no nameplates).
    canvas.write(
        2,
        2,
        "❯ Earlier: sketch the cache invalidation plan.",
        TextStyle::plain(ColorToken::Dim),
    )?;
    canvas.write(
        3,
        2,
        "Outlined a two-phase write-through plan with a flush root.",
        TextStyle::plain(ColorToken::Dim),
    )?;
    canvas.write(
        5,
        2,
        "❯ Then tight-loop the HTTP client retries.",
        TextStyle::plain(ColorToken::Dim),
    )?;
    canvas.write(
        6,
        2,
        "Added jittered backoff and a ceiling helper on 429 floors.",
        TextStyle::plain(ColorToken::Dim),
    )?;
    // Recent turns full wash near composer.
    paint_user_turn(
        &mut canvas,
        9,
        &["Ship it. Summarize what landed and what's left."],
    )?;
    paint_thought_line(&mut canvas, 11, "2.4s")?;
    paint_md_primary_header(&mut canvas, 13, "What landed")?;
    paint_assistant(
        &mut canvas,
        14,
        &[
            "Retry path is guarded; flush root is deterministic under concurrent writers.",
            "Jittered backoff covers 429 floors; write-through plan keeps membership cold.",
        ],
    )?;
    paint_md_secondary_header(&mut canvas, 17, "Still open")?;
    paint_assistant(
        &mut canvas,
        18,
        &[
            "- integration case for 429 floors",
            "- short docs note on write-through phases",
        ],
    )?;
    canvas.write(
        21,
        2,
        "────────────────────────",
        TextStyle::plain(ColorToken::Dim),
    )?;
    paint_assistant(
        &mut canvas,
        22,
        &["4 earlier turns above · ↑ to scroll"],
    )?;
    paint_default_prompt(&mut canvas, 33, "")?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

/// Busy streaming DNA: top budget, bare user, Thought, dense ladder, turn-status, jewel.
fn streaming() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_demo_top_bar(&mut canvas)?;
    paint_user_turn_with_timestamp(
        &mut canvas,
        2,
        &["Explain the race in the flush root briefly."],
        Some("8:19 AM"),
    )?;
    paint_thought_line(&mut canvas, 4, "1.4s")?;
    paint_md_primary_header(&mut canvas, 6, "Race structure")?;
    paint_assistant(
        &mut canvas,
        7,
        &[
            "Two writers can skip the watched counter when the snapshot freezes mid-batch.",
            "A sequence gate re-checks membership before the flush root publishes.",
        ],
    )?;
    paint_md_secondary_header(&mut canvas, 10, "Guard")?;
    paint_assistant(
        &mut canvas,
        11,
        &[
            "- freeze the related sequence before the write",
            "- recompute the counter only after both writers quiesce",
            "- surface a short elapsed timer while the stream is live",
        ],
    )?;
    canvas.write(
        15,
        2,
        "────────────────────────",
        TextStyle::plain(ColorToken::Dim),
    )?;
    paint_assistant(
        &mut canvas,
        16,
        &[
            "The partial answer stream keeps the same heading ladder as the final turn.",
            "█",
        ],
    )?;
    // Re-style the caret so it still reads as live stream caret.
    canvas.write(17, 2, "█", TextStyle::bold(ColorToken::Accent))?;
    let phase = format!("Responding… {}", format_turn_timer(1.4));
    paint_turn_status_busy(
        &mut canvas,
        31,
        &phase,
        &format_turn_timer(4.0),
        "2.1k",
    )?;
    paint_default_prompt(&mut canvas, 33, "")?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn reasoning() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_demo_top_bar(&mut canvas)?;
    paint_user_turn(
        &mut canvas,
        3,
        &["Why did the flaky suite pass only after shred?"],
    )?;
    paint_thought_line(&mut canvas, 5, "12s")?;
    // Expanded thinking body (diamonds on thought steps, not as Cortex nameplate).
    canvas.write(
        6,
        4,
        "◇ The failed worker reused a stale group counter.",
        TextStyle::plain(ColorToken::Accent),
    )?;
    canvas.write(
        7,
        4,
        "◇ Force-clearing temp keys resets membership.",
        TextStyle::plain(ColorToken::Accent),
    )?;
    canvas.write(
        8,
        4,
        "◇ Plan: pin fixture ids and assert counter rebuild.",
        TextStyle::plain(ColorToken::Accent),
    )?;
    paint_md_primary_header(&mut canvas, 10, "Root cause")?;
    paint_assistant(
        &mut canvas,
        11,
        &[
            "The suite flattened once the group counter rebuilt from empty membership.",
            "Pin that rebuild as an explicit assert before re-running the flaky suite.",
        ],
    )?;
    paint_md_secondary_header(&mut canvas, 14, "Fix")?;
    paint_assistant(
        &mut canvas,
        15,
        &[
            "- shred temp keys that own stale membership",
            "- pin fixture ids so counters cannot drift across workers",
        ],
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
        3,
        &["Compress the long thinking from the last answer."],
    )?;
    // Collapsed Thought only — no nameplate, no monologue about "kept collapsed".
    paint_thought_line(&mut canvas, 5, "12s")?;
    canvas.write(
        6,
        4,
        "◇ Enter to expand",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_md_primary_header(&mut canvas, 8, "Summary")?;
    paint_assistant(
        &mut canvas,
        9,
        &[
            "Pinned fixture ids and rebuilt the group counter before re-running the suite.",
            "Stale membership no longer survives across workers after the temp key clear.",
        ],
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
        2,
        &["Document the recovery procedure with steps and a sample."],
    )?;
    paint_thought_line(&mut canvas, 4, "3.1s")?;
    paint_md_primary_header(&mut canvas, 6, "Recovery procedure")?;
    paint_assistant(
        &mut canvas,
        7,
        &["Use the ordered steps below. Stop live work first; only then clear stale keys."],
    )?;
    paint_md_secondary_header(&mut canvas, 9, "Steps")?;
    paint_assistant(
        &mut canvas,
        10,
        &[
            "1. Stop the worker with [stop] or Ctrl+c.",
            "2. Clear temp keys that hold stale membership.",
            "3. Re-run the suite and confirm the counter rebuilds.",
            "4. Record the restored budget before the next turn.",
        ],
    )?;
    canvas.write(
        15,
        2,
        "────────────────────────",
        TextStyle::plain(ColorToken::Dim),
    )?;
    paint_md_secondary_header(&mut canvas, 16, "Sample commands")?;
    // Code block bg surface.
    let code_style = TextStyle::on(ColorToken::Text, ColorToken::CodeBg);
    canvas.fill_row_bg(17, 2, 80, code_style)?;
    canvas.fill_row_bg(18, 2, 80, code_style)?;
    canvas.fill_row_bg(19, 2, 80, code_style)?;
    canvas.write(17, 4, "cargo test --test flush_root_retry", code_style)?;
    canvas.write(18, 4, "# expect: membership rebuilt · 0 flaky", code_style)?;
    canvas.write(19, 4, "cargo test --test rate_limit_floors", code_style)?;
    paint_assistant(
        &mut canvas,
        21,
        &["See also: docs/recovery.md for the long form."],
    )?;
    // Tint the cross-ref as Info-blue secondary trail.
    canvas.write(
        21,
        2,
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
        3,
        &["Save the export and maybe fire a toast."],
    )?;
    paint_thought_line(&mut canvas, 5, "0.8s")?;
    paint_md_primary_header(&mut canvas, 7, "Export")?;
    paint_assistant(
        &mut canvas,
        8,
        &["Export packed. Confirmations stack above the prompt jewel."],
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
    let text = canvas.to_text();
    reject_copper_residual(&text)?;
    reject_structure_residual(&text)?;
    Ok(canvas)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chrome::components::{default_model_chip, MODEL_LABEL, PROMPT_ARROW};

    const CONVERSATION_IDS: &[&str] = &[
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
    ];

    #[test]
    fn conversation_batch_pure_structure() {
        for id in CONVERSATION_IDS {
            let canvas = render(id).unwrap_or_else(|e| panic!("{id}: {e}"));
            let text = canvas.to_text();
            reject_copper_residual(&text).unwrap_or_else(|e| panic!("{id}: {e}"));
            reject_structure_residual(&text).unwrap_or_else(|e| panic!("{id}: {e}"));
            assert_eq!(text.lines().count(), 40, "{id}");
            assert!(!text.contains("Claude Opus"), "{id}");
            assert!(!text.contains("Ask Code"), "{id}");
            assert!(!text.contains("❯ You"), "{id} nameplate");
            assert!(!text.contains("◆ Cortex"), "{id} nameplate");
            assert!(!text.contains("expanded thinking kept collapsed"), "{id}");
            // Authenticated top chrome carries context budget form.
            assert!(
                text.contains("9.5K / 500K") || text.contains("8.5K / 1.0M"),
                "{id} missing context budget"
            );
            if *id == "chat-turn" {
                assert!(text.contains(PROMPT_ARROW));
                assert!(text.contains(MODEL_LABEL));
                assert!(text.contains("always-approve"));
                assert!(
                    text.contains("Cortex (high)") || text.contains(&default_model_chip())
                );
                assert!(text.contains("◆ Thought for"));
                assert!(text.contains("Retry guard") || text.contains("Follow-up"));
            }
            if *id == "streaming" {
                assert!(text.contains("[stop]"));
                assert!(text.contains("◆ Thought for"));
                assert!(text.contains("Race structure") || text.contains("Guard"));
                assert!(!text.split_whitespace().any(|tok| {
                    tok.len() >= 4
                        && tok.starts_with("0m")
                        && tok.ends_with('s')
                        && tok[2..tok.len() - 1].chars().all(|c| c.is_ascii_digit())
                }));
                assert!(text.contains("Shift+Tab:mode | Ctrl+c:cancel"));
            }
            if *id == "rich-response" {
                assert!(text.contains("Recovery procedure"));
                assert!(text.contains("Steps") || text.contains("Sample"));
                assert!(text.contains("◆ Thought for"));
            }
            if matches!(
                *id,
                "chat-turn"
                    | "streaming"
                    | "chat-long-scroll"
                    | "rich-response"
                    | "reasoning"
                    | "reasoning-collapsed"
                    | "toasts"
                    | "home"
                    | "empty"
            ) {
                // Idle/authenticated jewel still present where a prompt is drawn.
                if text.contains('╭') {
                    assert!(
                        text.contains("always-approve") || text.contains(MODEL_LABEL),
                        "{id} jewel missing model"
                    );
                }
            }
        }
    }

    #[test]
    fn streaming_dna_checklist_product_form() {
        let text = render("streaming").unwrap().to_text();
        assert!(text.contains("~/projects/demo") || text.contains("projects/demo"));
        assert!(text.contains("9.5K / 500K"));
        assert!(text.contains("❯ Explain the race in the flush root briefly."));
        assert!(!text.contains("❯ You"));
        assert!(!text.contains("◆ Cortex"));
        assert!(text.contains("◆ Thought for"));
        assert!(text.contains("[stop]"));
        assert!(
            text.contains("Cortex (high)") || text.contains(&default_model_chip())
        );
        assert!(text.contains('|'), "putty must use | separators");
        // Density: non-sparse body — green/blue ladder copy present.
        let body_lines = text
            .lines()
            .filter(|l| {
                let t = l.trim();
                !t.is_empty()
                    && !t.starts_with('╭')
                    && !t.starts_with('╰')
                    && !t.starts_with('│')
                    && !t.contains("Shift+Tab")
            })
            .count();
        assert!(
            body_lines >= 12,
            "streaming body too sparse: {body_lines} non-empty content-ish rows"
        );
    }
}
