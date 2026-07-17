//! Pure Cortex auth journey screens (F1).
//!
//! Device-auth codes freeze to `DEVICE_CODE`. xAI-first: no multi-vendor copper matrix.

use crate::chrome::canvas::Canvas;
use crate::chrome::components::{
    paint_auth_option_list, paint_braille_logo, paint_default_prompt, paint_default_putty,
    paint_demo_top_bar, paint_login_quit, paint_top_branch_bar, reject_copper_residual,
    AUTH_PATH, DEMO_BRANCH, DEVICE_CODE, MODEL_LABEL,
};
use crate::chrome::tokens::{ColorToken, TextStyle, Theme};

use super::login;

pub fn render(id: &str) -> Result<Canvas, String> {
    match id {
        "login" => login::render(),
        "connect-provider" => connect_provider(),
        "connect-method" => connect_method(),
        "connect-oauth" => connect_oauth(),
        "connect-apikey" => connect_apikey(),
        "connect-connected" => connect_connected(),
        "connect-disconnect" => connect_disconnect(),
        other => Err(format!("unknown auth pure-Cortex id: {other}")),
    }
}

fn connect_provider() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_top_branch_bar(&mut canvas, DEMO_BRANCH, AUTH_PATH)?;
    paint_braille_logo(&mut canvas, 4)?;
    canvas.write_centered(
        13,
        "Sign in to use Cortex",
        TextStyle::bold(ColorToken::Bright),
    )?;
    canvas.write_centered(
        15,
        "xAI account powers Cortex for this workspace.",
        TextStyle::plain(ColorToken::Secondary),
    )?;
    paint_auth_option_list(
        &mut canvas,
        "Account",
        "One cloud account · no multi-vendor sprawl",
        &[("xAI Cortex (recommended)", true), ("Paste XAI_API_KEY later", false)],
        "continue",
        "quit",
    )?;
    paint_login_quit(&mut canvas)?;
    finish(canvas)
}

fn connect_method() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_top_branch_bar(&mut canvas, DEMO_BRANCH, AUTH_PATH)?;
    canvas.write(4, 4, "How do you want to sign in?", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(
        5,
        4,
        "Cortex uses your xAI account or an API key.",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_auth_option_list(
        &mut canvas,
        "Method",
        "",
        &[
            ("Browser device code (recommended)", true),
            ("Open browser OAuth tab", false),
            ("Paste XAI_API_KEY", false),
        ],
        "choose method",
        "back",
    )?;
    canvas.write(
        32,
        4,
        "Tip: device code works in headless SSH shells.",
        TextStyle::plain(ColorToken::Dim),
    )?;
    paint_login_quit(&mut canvas)?;
    finish(canvas)
}

fn connect_oauth() -> Result<Canvas, String> {
    // Runtime-proven device-code TUI (same freeze as login).
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_top_branch_bar(&mut canvas, DEMO_BRANCH, AUTH_PATH)?;
    paint_braille_logo(&mut canvas, 4)?;
    canvas.write_centered(
        13,
        "Device code sign-in",
        TextStyle::bold(ColorToken::Bright),
    )?;
    canvas.write_centered(
        15,
        "Approve in your browser to finish signing in.",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write_centered(17, DEVICE_CODE, TextStyle::bold(ColorToken::Bright))?;
    canvas.write_centered(
        19,
        "Make sure your browser shows this code.",
        TextStyle::plain(ColorToken::Secondary),
    )?;
    canvas.write_centered(
        21,
        "Waiting for approval...",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write_centered(
        24,
        "https://accounts.x.ai/device  (copy link)",
        TextStyle::plain(ColorToken::Info),
    )?;
    paint_login_quit(&mut canvas)?;
    finish(canvas)
}

fn connect_apikey() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_top_branch_bar(&mut canvas, DEMO_BRANCH, AUTH_PATH)?;
    canvas.write(4, 4, "Paste an XAI API key", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(
        5,
        4,
        "Never paint real secrets. Keys stay local.",
        TextStyle::plain(ColorToken::Muted),
    )?;
    // Masked key entry box.
    canvas.write(10, 4, "╭──────────────────────────────────────────────╮", TextStyle::plain(ColorToken::Border))?;
    canvas.write(
        11,
        4,
        "│  xai-••••••••••••••••••••••••••••  █         │",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(12, 4, "╰──────────────────────────────────────────────╯", TextStyle::plain(ColorToken::Border))?;
    canvas.write(
        14,
        4,
        "Or set XAI_API_KEY in your environment and reload.",
        TextStyle::plain(ColorToken::Secondary),
    )?;
    canvas.write(
        16,
        4,
        "Enter to save key · Esc to back",
        TextStyle::plain(ColorToken::Dim),
    )?;
    paint_login_quit(&mut canvas)?;
    finish(canvas)
}

fn connect_connected() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_demo_top_bar(&mut canvas)?;
    canvas.write(4, 2, "✓ Signed in to xAI", TextStyle::bold(ColorToken::Ok))?;
    canvas.write(
        5,
        4,
        "Cortex is ready for this workspace.",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        7,
        4,
        "Account   demo@x.ai",
        TextStyle::plain(ColorToken::Secondary),
    )?;
    let model_line = format!("Model     {MODEL_LABEL}");
    canvas.write(
        8,
        4,
        &model_line,
        TextStyle::plain(ColorToken::Secondary),
    )?;
    canvas.write(
        9,
        4,
        "Mode      always-approve",
        TextStyle::plain(ColorToken::Secondary),
    )?;
    canvas.write(
        12,
        4,
        "Enter to open agent shell · Esc stays here",
        TextStyle::plain(ColorToken::Dim),
    )?;
    paint_default_prompt(&mut canvas, 33, "Ask Cortex to start a task…")?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn connect_disconnect() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_demo_top_bar(&mut canvas)?;
    canvas.write(6, 4, "Sign out of Cortex?", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(
        7,
        4,
        "This clears the local xAI session on this machine.",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        10,
        4,
        "› Sign out now",
        TextStyle::bold(ColorToken::Bad),
    )?;
    canvas.write(
        11,
        4,
        "  Keep me signed in",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        14,
        4,
        "Enter to confirm · Esc to cancel",
        TextStyle::plain(ColorToken::Dim),
    )?;
    canvas.write(
        16,
        4,
        "Drafts and local transcripts stay on disk.",
        TextStyle::plain(ColorToken::Secondary),
    )?;
    paint_login_quit(&mut canvas)?;
    finish(canvas)
}

fn finish(canvas: Canvas) -> Result<Canvas, String> {
    reject_copper_residual(&canvas.to_text())?;
    Ok(canvas)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_batch_has_frozen_device_code_and_no_copper() {
        for id in [
            "login",
            "connect-provider",
            "connect-method",
            "connect-oauth",
            "connect-apikey",
            "connect-connected",
            "connect-disconnect",
        ] {
            let canvas = render(id).unwrap_or_else(|e| panic!("{id}: {e}"));
            let text = canvas.to_text();
            reject_copper_residual(&text).unwrap_or_else(|e| panic!("{id}: {e}"));
            assert_eq!(text.lines().count(), 40, "{id}");
            if matches!(id, "login" | "connect-oauth") {
                assert!(text.contains(DEVICE_CODE), "{id} missing frozen code");
            }
            assert!(!text.contains("Authorize OpenAI"), "{id}");
            assert!(!text.contains("Claude"), "{id}");
        }
    }
}
