//! F0 pure product chrome unit gates: palette hex, glyph coverage, logo/login smoke.

use mockup_harness_v4::chrome::canvas::Canvas;
use mockup_harness_v4::chrome::components::{
    paint_braille_logo, paint_prompt_jewel, paint_putty_footer, paint_top_branch_bar,
    paint_turn_status_busy, DEVICE_CODE, MODEL_LABEL, PROMPT_ARROW, BRAILLE_LOGO,
};
use mockup_harness_v4::chrome::tokens::{
    glyph_allowed, glyph_rendering, palette, validate_glyphs, ColorToken, FORBIDDEN_COPPER_ACCENT,
    GlyphRendering, TextStyle, Theme, ALLOWED_GLYPHS,
};
use mockup_harness_v4::screens::{render_pilot, list_pilots};

#[test]
fn palette_cortexnight_identity_hex() {
    let p = palette(Theme::CortexNight);
    assert_eq!(p.hex(ColorToken::Canvas), "#141414");
    assert_eq!(p.hex(ColorToken::Text), "#e1e1e1");
    assert_eq!(p.hex(ColorToken::Accent), "#bb9af7");
    assert_eq!(p.hex(ColorToken::Secondary), "#c8c8c8");
    assert_ne!(
        p.hex(ColorToken::Accent).to_lowercase(),
        FORBIDDEN_COPPER_ACCENT
    );
}

#[test]
fn palette_grokday_peer_hex() {
    let p = palette(Theme::CortexDay);
    assert_eq!(p.hex(ColorToken::Canvas), "#eeeeee");
    assert_eq!(p.hex(ColorToken::Text), "#262626");
    assert_eq!(p.hex(ColorToken::Accent), "#7d4bc6");
}

#[test]
fn glyph_coverage_prompt_diamonds_braille_stop_putty() {
    for c in [
        '❯', '◆', '◇', '◈', '◉', '◎', '•', '⧉', '✓', '✗', '⇣', '', '▸', '▾', '·',
    ] {
        assert!(glyph_allowed(c), "glyph {c} should be allowed");
    }
    for c in ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'] {
        assert!(glyph_allowed(c), "spinner {c}");
    }
    assert!(glyph_allowed('\u{2801}'));
    assert!(glyph_allowed('\u{28FF}'));
    assert!(validate_glyphs("[stop]").is_ok());
    assert!(validate_glyphs("Shift+Tab:mode · Ctrl+c:cancel").is_ok());
    assert!(validate_glyphs("✦").is_err());
    assert!(ALLOWED_GLYPHS.contains(PROMPT_ARROW));
    assert_eq!(
        glyph_rendering('❯'),
        Some(GlyphRendering::SyntheticPromptArrow)
    );
    assert_eq!(
        glyph_rendering('◆'),
        Some(GlyphRendering::SyntheticDiamond)
    );
}

#[test]
fn logo_login_smoke_cells() {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_top_branch_bar(&mut canvas, "master", "/projects/cortex-code").unwrap();
    paint_braille_logo(&mut canvas, 5).unwrap();
    canvas
        .write_centered(16, DEVICE_CODE, TextStyle::bold(ColorToken::Bright))
        .unwrap();
    let text = canvas.to_text();
    assert!(text.contains(DEVICE_CODE));
    assert!(text.contains(''));
    for line in BRAILLE_LOGO {
        assert!(
            text.contains(line.trim_end()),
            "missing braille line {line:?}"
        );
    }
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 40);
    assert!(lines.iter().all(|l| l.chars().count() == 120));
}

#[test]
fn pilot_login_and_chat_turn_no_copper_brand() {
    assert_eq!(list_pilots().len(), 2);
    let login = render_pilot("login").expect("login");
    let login_text = login.to_text();
    assert!(login_text.contains(DEVICE_CODE));
    assert!(login_text.contains("Waiting for approval"));
    assert!(!login_text.contains("Connect a model provider"));
    assert!(!login_text.contains("Code needs a provider"));

    let chat = render_pilot("chat-turn").expect("chat-turn");
    let chat_text = chat.to_text();
    assert!(chat_text.contains(PROMPT_ARROW));
    assert!(chat_text.contains(MODEL_LABEL));
    assert!(chat_text.contains("always-approve"));
    assert!(chat_text.contains("Shift+Tab:mode"));
    assert!(!chat_text.contains("Claude Opus"));
    assert!(!chat_text.contains("Code ·"));
}

#[test]
fn turn_status_exposes_stop_control() {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_turn_status_busy(&mut canvas, 30, "Run command 0.2s", "1m20s", "12k").unwrap();
    let text = canvas.to_text();
    assert!(text.contains("[stop]"));
    assert!(text.contains('⠧'));
}

#[test]
fn prompt_jewel_and_putty_present() {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_prompt_jewel(
        &mut canvas,
        33,
        "draft here",
        MODEL_LABEL,
        "always-approve",
    )
    .unwrap();
    paint_putty_footer(&mut canvas, "Shift+Tab:mode · Ctrl+c:cancel · Ctrl+.:shortcuts")
        .unwrap();
    let text = canvas.to_text();
    assert!(text.contains(PROMPT_ARROW));
    assert!(text.contains(MODEL_LABEL));
    assert!(text.contains("Shift+Tab:mode"));
    assert!(text.contains('╭'));
    assert!(text.contains('╰'));
}

#[cfg(feature = "png")]
#[test]
fn png_glyph_coverage_login_and_chat() {
    use mockup_harness_v4::chrome::canvas::render_png;
    for id in ["login", "chat-turn"] {
        let canvas = render_pilot(id).expect(id);
        let bytes = render_png(&canvas).expect("png");
        assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"), "{id} png magic");
        assert!(bytes.len() > 1_000, "{id} png too small");
        // PNG IHDR chunk: after 8-byte signature, 4 len + 4 "IHDR" then width/height.
        assert_eq!(&bytes[12..16], b"IHDR", "{id} missing IHDR");
        let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
        let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
        assert_eq!((width, height), (1200, 840), "{id}");
    }
}
