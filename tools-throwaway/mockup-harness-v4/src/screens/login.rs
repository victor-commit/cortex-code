//! Pure Cortex device-code login shell (runtime capture fidelity).
//! Matches `evidence/cortex-runtime-captures/welcome-120x40-reconstructed.txt`.

use crate::chrome::canvas::Canvas;
use crate::chrome::components::{
    paint_braille_logo, paint_login_quit, paint_top_branch_bar, reject_copper_residual,
    AUTH_PATH, DEMO_BRANCH, DEVICE_CODE,
};
use crate::chrome::tokens::{ColorToken, TextStyle, Theme};

pub fn render() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_top_branch_bar(&mut canvas, DEMO_BRANCH, AUTH_PATH)?;
    paint_braille_logo(&mut canvas, 5)?;

    canvas.write_centered(
        14,
        "Approve in your browser to finish signing in.",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write_centered(16, DEVICE_CODE, TextStyle::bold(ColorToken::Bright))?;
    canvas.write_centered(
        18,
        "Make sure your browser shows this code.",
        TextStyle::plain(ColorToken::Secondary),
    )?;
    canvas.write_centered(
        20,
        "If it doesn't open, click here to copy.",
        TextStyle::plain(ColorToken::Secondary),
    )?;
    canvas.write_centered(
        24,
        "Copying not working? Click here to show full URL.",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write_centered(
        26,
        "Waiting for approval...",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_login_quit(&mut canvas)?;
    reject_copper_residual(&canvas.to_text())?;
    Ok(canvas)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chrome::components::DEVICE_CODE;

    #[test]
    fn login_smoke_has_device_code_and_braille_no_code_brand() {
        let canvas = render().expect("login paints");
        let text = canvas.to_text();
        assert!(text.contains(DEVICE_CODE));
        assert!(text.contains("Approve in your browser"));
        assert!(text.contains("Waiting for approval"));
        assert!(text.contains("ctrl+d  quit"));
        assert!(text.contains(''));
        assert!(text.contains("/projects/demo"));
        assert!(!text.contains("cortex-code"));
        assert!(text.chars().any(|c| (0x2800..=0x28FF).contains(&(c as u32))));
        assert!(!text.contains("Connect a model provider"));
        assert!(!text.lines().any(|l| l.trim() == "Code"));
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 40);
        assert!(lines.iter().all(|l| l.chars().count() == 120));
    }
}
