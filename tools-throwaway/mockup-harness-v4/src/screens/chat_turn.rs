//! Pure Cortex idle chat-turn shell: completed You/assistant pair + prompt jewel + putty.

use crate::chrome::canvas::Canvas;
use crate::chrome::components::{
    paint_prompt_jewel, paint_putty_footer, MODEL_LABEL, PROMPT_ARROW,
};
use crate::chrome::tokens::{ColorToken, TextStyle, Theme};

pub fn render() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);

    // Quiet top context (Cortex-style; not Cortex statusline NPR style).
    canvas.write(
        1,
        2,
        " master  /projects/demo",
        TextStyle::plain(ColorToken::Dim),
    )?;

    // User turn with soft surface wash.
    let user_style = TextStyle::user_turn();
    canvas.fill_row_bg(4, 2, 100, user_style)?;
    canvas.write(4, 2, "❯ You", TextStyle::user_turn())?;
    canvas.write(
        5,
        4,
        "Check the retry path before we merge.",
        TextStyle::plain(ColorToken::Secondary),
    )?;
    canvas.write(
        6,
        4,
        "Keep the summary short and list one follow-up test.",
        TextStyle::plain(ColorToken::Secondary),
    )?;

    // Assistant (magenta accent identity — pure Cortex, never copper Code).
    canvas.write(9, 2, "◆ Cortex", TextStyle::accent())?;
    canvas.write(
        10,
        4,
        "Applied the retry guard and covered the failure path.",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        11,
        4,
        "Follow-up: add an integration case for rate-limit floors.",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        13,
        4,
        "Thought for 1s",
        TextStyle::plain(ColorToken::Accent),
    )?;
    canvas.write(
        14,
        4,
        "◇ expanded thinking kept collapsed in this idle frame",
        TextStyle::plain(ColorToken::Muted),
    )?;

    // Prompt jewel near bottom.
    paint_prompt_jewel(
        &mut canvas,
        33,
        "Add a note about the remaining test hunk…",
        MODEL_LABEL,
        "always-approve",
    )?;

    // Permanent putty footer is pure Cortex, allowed here.
    paint_putty_footer(
        &mut canvas,
        "Shift+Tab:mode · Ctrl+c:cancel · Ctrl+.:shortcuts",
    )?;

    let text = canvas.to_text();
    if text.contains("Claude") || text.contains("Cortex") {
        return Err("chat-turn still shows non-Cortex product brands".to_owned());
    }
    // Banish copper Identity banners; tolerate substring only if XAI code words.
    if text.contains("Code ·") || text.contains("Cortex Code") {
        return Err("copper Code brand residual".to_owned());
    }
    if !text.contains(PROMPT_ARROW) {
        return Err("missing Cortex prompt arrow".to_owned());
    }
    if !text.contains(MODEL_LABEL) {
        return Err("missing Cortex model chrome".to_owned());
    }
    Ok(canvas)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chrome::components::{MODEL_LABEL, PROMPT_ARROW};

    #[test]
    fn chat_turn_smoke_pure_cortex_shell() {
        let canvas = render().expect("chat-turn paints");
        let text = canvas.to_text();
        assert!(text.contains(PROMPT_ARROW));
        assert!(text.contains(MODEL_LABEL));
        assert!(text.contains("always-approve"));
        assert!(text.contains("Shift+Tab:mode"));
        assert!(text.contains('◆'));
        assert!(text.contains("Cortex"));
        assert!(!text.contains("Claude Opus"));
        assert!(!text.contains("Code ·"));
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 40);
        assert!(lines.iter().all(|l| l.chars().count() == 120));
    }
}
