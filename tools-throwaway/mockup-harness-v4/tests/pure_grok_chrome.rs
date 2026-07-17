//! Pure product chrome + F1-F6 paint unit gates.

use mockup_harness_v4::chrome::canvas::Canvas;
use mockup_harness_v4::chrome::components::{
    paint_braille_logo, paint_prompt_jewel, paint_top_branch_bar, paint_turn_status_busy,
    reject_copper_residual, DEVICE_CODE, MODEL_LABEL, PROMPT_ARROW, BRAILLE_LOGO,
};
use mockup_harness_v4::chrome::tokens::{
    glyph_allowed, glyph_rendering, palette, validate_glyphs, ColorToken, FORBIDDEN_COPPER_ACCENT,
    GlyphRendering, TextStyle, Theme, ALLOWED_GLYPHS,
};
use mockup_harness_v4::screens::{list_pilots, render_pilot};

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
        assert!(glyph_allowed(c), "glyph should be allowed: {c}");
    }
    for c in ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'] {
        assert!(glyph_allowed(c), "spinner should be allowed: {c}");
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
    paint_top_branch_bar(&mut canvas, "master", "/projects/demo").unwrap();
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
fn f1_f7_inventory_is_eighty_six() {
    assert_eq!(list_pilots().len(), 86);
    let ids: Vec<&str> = list_pilots().iter().map(|p| p.id).collect();
    for expected in [
        "login",
        "connect-oauth",
        "chat-turn",
        "onboarding",
        "streaming",
        "empty",
        "toasts",
        "tool-running",
        "diff-edit",
        "shell",
        "todo-list",
        "compact",
        // F4
        "slash-menu",
        "model-picker",
        "queue-manager",
        "composer-multiline",
        "file-mention",
        // F5
        "permission-shell",
        "permission-file-edit",
        "blocked-command",
        "turn-error",
        "error-network",
        "rate-limit",
        "interrupt",
        "share",
        // F6
        "status",
        "mcp",
        "diff-viewer",
        "help",
        "theme-gallery-dark",
        "theme-gallery-light",
        "sidebar-queue",
        // F7 orchestration
        "mode-switch",
        "mode-switch-refused",
        "workflow-launch",
        "workflow-launch-blocked",
        "workflow-monitor",
        "workflow-monitor-failure",
        "team-board",
        "reviewer-output",
        "reviewer-approved",
        "verification-ladder",
        "intent-scorer-pass",
        "intent-scorer-fail",
        "concurrency-metrics",
        "tool-envelope",
    ] {
        assert!(ids.contains(&expected), "missing id {expected}");
    }
    assert_eq!(mockup_harness_v4::GATE_FRAMES.len(), 35);
}

#[test]
fn all_f1_f7_screens_reject_copper_brand() {
    for spec in list_pilots() {
        let canvas = render_pilot(spec.id).unwrap_or_else(|e| panic!("{}: {e}", spec.id));
        let text = canvas.to_text();
        reject_copper_residual(&text).unwrap_or_else(|e| panic!("{}: {e}", spec.id));
        assert!(!text.contains("Claude Opus"), "claude residual: {}", spec.id);
        assert!(
            !text.contains("Authorize OpenAI"),
            "openai residual: {}",
            spec.id
        );
        assert!(!text.contains("Code ·"), "code brand residual: {}", spec.id);
        assert!(
            !text.contains("Connect a model provider"),
            "provider residual: {}",
            spec.id
        );
        assert!(!text.contains("Ask Code"), "ask code residual: {}", spec.id);
        assert!(!text.contains("Work mode"), "work mode residual: {}", spec.id);
        assert!(
            !text.contains("How Code should work"),
            "how-code residual: {}",
            spec.id
        );
        assert!(
            !text.contains("Factory Mission"),
            "factory residual: {}",
            spec.id
        );
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 40, "row count for {}", spec.id);
        let bad_width = lines.iter().any(|l| l.chars().count() != 120);
        assert!(!bad_width, "row width for {}", spec.id);
    }
}

#[test]
fn orchestration_uses_tasks_dashboard_plan_language() {
    let monitor = render_pilot("workflow-monitor").unwrap().to_text();
    assert!(monitor.contains("Tasks"));
    assert!(monitor.contains("Running"));
    assert!(!monitor.contains("Claude Opus"));
    assert!(!monitor.lines().any(|l| l.trim() == "Mission"));
    let mode = render_pilot("mode-switch").unwrap().to_text();
    assert!(mode.contains("Shift+Tab") || mode.contains("plan"));
    assert!(!mode.contains("How Code should work"));
    let dash = render_pilot("concurrency-metrics").unwrap().to_text();
    assert!(dash.contains("Dashboard") || dash.contains("tiles"));
    assert!(dash.contains(MODEL_LABEL));
}

#[test]
fn f4_f6_markers_putty_and_stop() {
    let model = render_pilot("model-picker").unwrap().to_text();
    assert!(model.contains(MODEL_LABEL));
    assert!(model.contains("Shift+Tab:mode"));
    assert!(!model.contains("Claude"));
    let interrupt = render_pilot("interrupt").unwrap().to_text();
    assert!(interrupt.contains("[stop]"));
    assert!(interrupt.contains(MODEL_LABEL));
    let help = render_pilot("help").unwrap().to_text();
    assert!(help.contains("[stop]"));
    assert!(help.contains("Shift+Tab"));
    let status = render_pilot("status").unwrap().to_text();
    assert!(status.contains(MODEL_LABEL));
    assert!(!status.contains("Claude Opus"));
}

#[test]
fn device_auth_freeze_deterministic_codes() {
    for id in ["login", "connect-oauth"] {
        let text = render_pilot(id).unwrap().to_text();
        assert!(
            text.contains(DEVICE_CODE),
            "{id} must freeze device code {DEVICE_CODE}"
        );
        // No live wall-clock theatre.
        assert!(!text.contains("Elapsed"), "{id} has live elapsed timer");
    }
}

#[test]
fn busy_frames_expose_stop_and_putty() {
    let running = render_pilot("tool-running").unwrap().to_text();
    assert!(running.contains("[stop]"));
    assert!(
        !running.contains("[s[stop]"),
        "turn-status stop column must be char-indexed, got: {}",
        running
            .lines()
            .find(|l| l.contains("[stop]"))
            .unwrap_or("")
    );
    assert!(running.contains("Shift+Tab:mode"));
    let streaming = render_pilot("streaming").unwrap().to_text();
    assert!(streaming.contains("[stop]"));
    assert!(!streaming.contains("[s[stop]"));
    assert!(streaming.contains(MODEL_LABEL));
}

#[test]
fn turn_status_right_edge_is_clean_stop() {
    let mut canvas = Canvas::new(Theme::CortexNight);
    // Product form under 60s is "12s", never "0m12s".
    paint_turn_status_busy(&mut canvas, 30, "Run command 0.2s · 0 queued", "12s", "4.8k")
        .unwrap();
    let line = canvas
        .to_text()
        .lines()
        .nth(30)
        .expect("row 30")
        .to_string();
    assert!(
        line.contains("12s ⇣4.8k [stop]"),
        "expected clean timer + tokens + stop, got: {line:?}"
    );
    assert!(!line.contains("0m12s"));
    assert!(!line.contains("[s[stop]"));
    // Column math is Unicode scalar count, not UTF-8 bytes (braille + ⇣ are multi-byte).
    let stop_col = line
        .char_indices()
        .find(|(i, _)| line[*i..].starts_with("[stop]"))
        .map(|(i, _)| line[..i].chars().count())
        .expect("stop present");
    assert!(
        stop_col + "[stop]".chars().count() <= 118,
        "stop must not overflow canvas: col {stop_col} in {line:?}"
    );
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
    use mockup_harness_v4::chrome::components::{
        default_model_chip, format_turn_timer, paint_default_putty, paint_demo_top_bar,
        paint_user_turn, reject_structure_residual, DEFAULT_PUTTY,
    };
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_demo_top_bar(&mut canvas).unwrap();
    paint_user_turn(&mut canvas, 4, &["Check timeout paths."]).unwrap();
    paint_prompt_jewel(
        &mut canvas,
        33,
        "draft here",
        MODEL_LABEL,
        "always-approve",
    )
    .unwrap();
    paint_default_putty(&mut canvas).unwrap();
    let text = canvas.to_text();
    reject_structure_residual(&text).unwrap();
    assert!(text.contains(PROMPT_ARROW));
    assert!(text.contains(MODEL_LABEL));
    assert!(text.contains("Cortex (high) · always-approve") || text.contains(&default_model_chip()));
    assert!(text.contains("9.5K / 500K"));
    assert!(text.contains("❯ Check timeout paths."));
    assert!(!text.contains("❯ You"));
    assert!(!text.contains("◆ Cortex"));
    assert!(DEFAULT_PUTTY.contains('|'));
    assert!(text.contains("Shift+Tab:mode | Ctrl+c:cancel"));
    assert!(text.contains('╭'));
    assert!(text.contains('╰'));
    // Draft-row chip placement: model on same line as draft.
    let draft_row = text.lines().nth(34).unwrap();
    assert!(draft_row.contains("draft here"));
    assert!(draft_row.contains("always-approve"));
    assert_eq!(format_turn_timer(4.0), "4.0s");
    assert_eq!(format_turn_timer(15.0), "15s");
    assert_eq!(format_turn_timer(80.0), "1m20s");
}

#[cfg(feature = "png")]
#[test]
fn png_glyph_coverage_sample_batch() {
    use mockup_harness_v4::chrome::canvas::render_png;
    for id in [
        "login",
        "chat-turn",
        "streaming",
        "tool-running",
        "diff-edit",
        "empty",
        "connect-oauth",
        "model-picker",
        "permission-shell",
        "interrupt",
        "status",
        "sidebar-mcp",
        "theme-gallery-light",
        "workflow-monitor",
        "workflow-launch",
        "mode-switch",
        "team-board",
        "onboarding",
    ] {
        let canvas = render_pilot(id).expect(id);
        let bytes = render_png(&canvas).expect("png");
        assert!(
            bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
            "{id} missing png magic"
        );
        assert!(bytes.len() > 1_000, "{id} png too small");
        assert_eq!(&bytes[12..16], b"IHDR", "{id} missing IHDR");
        let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
        let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
        assert_eq!((width, height), (1200, 840), "{id} png size");
    }
}
