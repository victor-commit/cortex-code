//! Cortex foundation chrome: prompt jewel, turn-status band, outer pad,
//! and user-turn surface strip (VAL-V3-DS-031/032/034/035/037).

use mockup_harness_v3::chrome::{
    components::{
        contains_bracket_key_atom, contains_global_putty_footer, map_product_glyph,
        reject_composer_debug_meta, user_turn_surface_line, user_turn_surface_strip, Component,
        PromptJewel, PromptPrefix, TurnBusyKind, TurnStatusBand, PROMPT_PREFIX,
    },
    layout::{
        outer_content_width, outer_content_x, with_outer_hpad, OUTER_CONTENT_WIDTH, OUTER_HPAD,
        CANVAS_COLUMNS,
    },
    tokens::{validate_glyphs, ColorToken, TextStyle},
};

#[test]
fn outer_editorial_pad_targets_two_column_gutters_on_120_canvas() {
    assert_eq!(CANVAS_COLUMNS, 120);
    assert_eq!(OUTER_HPAD, 2);
    assert_eq!(OUTER_CONTENT_WIDTH, 116);
    assert_eq!(outer_content_x(), 2);
    assert_eq!(outer_content_width(), 116);

    let padded = with_outer_hpad("Ask Code about the rate limit")
        .expect("content must fit the editorial measure");
    assert_eq!(padded.chars().count(), 120);
    assert!(padded.starts_with("  "));
    assert!(padded.ends_with("  "));
    assert!(!padded.chars().next().unwrap().is_alphanumeric());
    assert!(!padded.chars().last().unwrap().is_alphanumeric());
    assert!(padded.contains("Ask Code about the rate limit"));

    assert!(
        with_outer_hpad(&"x".repeat(117)).is_err(),
        "content wider than outer measure must fail closed"
    );
}

#[test]
fn prompt_jewel_geometry_is_single_rounded_border_with_prefix_draft_and_info() {
    let jewel = PromptJewel::new(
        "Find where the server applies the rate limit",
        "Claude Opus 4.8",
        "Work mode",
    )
    .expect("valid jewel fields")
    .focused();

    let lines = jewel.lines();
    let snapshot = jewel.render_txt();
    validate_glyphs(&snapshot).expect("jewel uses closed glyphs only");
    reject_composer_debug_meta(&snapshot).expect("zero debug meta");
    assert!(!contains_bracket_key_atom(&snapshot));
    assert!(!contains_global_putty_footer(&snapshot));
    assert!(!snapshot.contains('❯'), "map Cortex arrow to closed ›");
    assert!(!snapshot.contains('◆'), "map diamonds to closed ●/○");

    // Rounded border geometry.
    assert!(
        lines.first().map(|l| l.text.starts_with('╭') && l.text.ends_with('╮'))
            == Some(true),
        "top border: {snapshot}"
    );
    assert!(
        lines.last().map(|l| l.text.starts_with('╰') && l.text.ends_with('╯')) == Some(true),
        "bottom border: {snapshot}"
    );
    assert!(
        lines.iter().any(|l| l.text.contains("│ › ")),
        "copper-accent prompt prefix › inside jewel: {snapshot}"
    );
    assert!(
        lines.iter().any(|l| l.text.contains("Find where the server applies the rate limit")),
        "draft lives inside jewel: {snapshot}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.text.contains("Claude Opus 4.8 · Work mode") && l.style == TextStyle::muted()),
        "muted human model · work mode info line: {snapshot}"
    );

    // Every interior row shares the outer jewel width; borders match content measure.
    let width = jewel.width();
    assert_eq!(width, OUTER_CONTENT_WIDTH);
    for line in &lines {
        assert_eq!(
            line.text.chars().count() as u16,
            width,
            "jewel row width mismatch: {:?}",
            line.text
        );
    }

    // Accent budget: prefix (and optional focused border weight) stays restrained.
    let anatomy = jewel.anatomy();
    assert!(
        anatomy.accent_uses <= 2,
        "jewel accent uses must stay within F-11 budget, got {}",
        anatomy.accent_uses
    );
    assert_eq!(anatomy.primary_action_count, 0);

    // Prefix glyph is the closed set only.
    assert_eq!(PROMPT_PREFIX, '›');
    assert_eq!(PromptPrefix::closed().as_char(), '›');
    assert_eq!(map_product_glyph('❯'), Some('›'));
    assert_eq!(map_product_glyph('◆'), Some('●'));
}

#[test]
fn prompt_jewel_rejects_debug_meta_and_forbidden_glyphs() {
    assert!(PromptJewel::new("cursor at character 12", "Claude Opus 4.8", "Work mode").is_err());
    assert!(PromptJewel::new("wraps visually", "Claude Opus 4.8", "Work mode").is_err());
    assert!(PromptJewel::new("hello", "claude-opus-4-8", "Work mode").is_err());
    assert!(PromptJewel::new("hello ❯ world", "Claude Opus 4.8", "Work mode").is_err());
    assert!(PromptJewel::new("", "Claude Opus 4.8", "Work mode").is_err());
}

#[test]
fn turn_status_band_paints_busy_and_collapses_when_idle() {
    let idle = TurnStatusBand::idle();
    assert!(idle.is_idle());
    assert_eq!(idle.height(), 0);
    assert!(idle.lines().is_empty(), "idle turn-status must collapse empty");
    assert_eq!(idle.render_txt(), "");

    let responding = TurnStatusBand::busy(TurnBusyKind::Responding, "15s")
        .expect("busy band")
        .with_stop("stop")
        .expect("stop prose");
    assert!(!responding.is_idle());
    assert_eq!(responding.height(), 1);
    let text = responding.render_txt();
    validate_glyphs(&text).expect("closed glyphs");
    assert!(text.contains('●') || text.contains("Responding"), "live activity: {text}");
    assert!(text.contains("Responding"), "{text}");
    assert!(text.contains("15s"), "{text}");
    assert!(
        text.contains("Esc to stop"),
        "stop must be prose, not [stop]: {text}"
    );
    assert!(!text.contains("[stop]"));
    assert!(!contains_bracket_key_atom(&text));
    assert!(!contains_global_putty_footer(&text));
    // Must not invent a filled footer strap token — no Surface/STATUS strip bg.
    for line in responding.lines() {
        assert_ne!(
            line.style.background,
            Some(ColorToken::Surface),
            "turn-status must not paint a filled strap"
        );
        assert_ne!(
            line.style.background,
            Some(ColorToken::Border),
            "turn-status must not paint a filled strap"
        );
    }

    let running = TurnStatusBand::busy(TurnBusyKind::Running, "4s")
        .expect("running")
        .with_detail("Search project code")
        .with_stop("stop")
        .expect("stop");
    let running_txt = running.render_txt();
    assert!(running_txt.contains("Running"), "{running_txt}");
    assert!(running_txt.contains("Search project code"), "{running_txt}");
    assert!(running_txt.contains("4s"), "{running_txt}");
    assert!(running_txt.contains("Esc to stop"), "{running_txt}");
    assert!(!running_txt.contains("[stop]"));

    // Anatomy is passive chrome, not a dock with primary CTA competition.
    assert_eq!(responding.anatomy().primary_action_count, 0);
}

#[test]
fn turn_status_band_never_emits_putty_or_stop_brackets() {
    let band = TurnStatusBand::busy(TurnBusyKind::Responding, "1m20s")
        .unwrap()
        .with_stop("stop")
        .unwrap();
    let text = band.render_txt();
    assert!(!text.contains("Shift+Tab"));
    assert!(!text.contains("Ctrl+c:"));
    assert!(!text.contains("[stop]"));
    assert!(!contains_global_putty_footer(&text));
}

#[test]
fn user_turn_surface_strip_uses_closed_prefix_and_surface_wash() {
    let strip = user_turn_surface_strip("Find the rate limit path")
        .expect("user strip");
    assert!(strip.text.contains('›'), "closed › once: {}", strip.text);
    assert!(!strip.text.contains('❯'));
    assert!(!strip.text.contains('✦'));
    assert_eq!(strip.style.background, Some(ColorToken::Surface));
    assert_eq!(strip.style.foreground, ColorToken::Bright);
    assert!(strip.text.contains("Find the rate limit path"));

    // Legacy helper remains available for plain wash without forcing a prefix.
    let plain = user_turn_surface_line("You · prior request").expect("legacy");
    assert_eq!(plain.style.background, Some(ColorToken::Surface));

    assert!(user_turn_surface_strip("").is_err());
    assert!(user_turn_surface_strip("Daily work · current conversation").is_err());
}

/// VAL-V3-DS-034: user-turn rows must rasterize a SURFACE tonal wash that is
/// pixel-distinguishable from pure CANVAS. Before the render_audit_png fix,
/// style.background was ignored and probes returned RGB(16,16,17) everywhere.
#[cfg(feature = "png")]
#[test]
fn published_user_turn_rows_paint_surface_wash_not_canvas() {
    use image::load_from_memory;
    use mockup_harness_v3::audit::audit_screen;
    use mockup_harness_v3::chrome::tokens::{
        palette, ColorToken, Theme, CELL_HEIGHT, CELL_WIDTH,
    };
    use mockup_harness_v3::scenario::render_audit_png;

    let surface = palette(Theme::Dark).color(ColorToken::Surface);
    let canvas_tok = palette(Theme::Dark).color(ColorToken::Canvas);
    assert_ne!(
        (surface.red, surface.green, surface.blue),
        (canvas_tok.red, canvas_tok.green, canvas_tok.blue),
        "Surface and Canvas must remain distinct tokens"
    );

    // Conversation shells that use paint_user_turn / user_turn_surface_strip.
    let fixtures = [
        ("chat-turn", "conversation-completed"),
        ("chat-long-scroll", "session.history.earlier"),
        ("streaming", "conversation-streaming"),
    ];
    for (screen_id, fixture_id) in fixtures {
        let audit = audit_screen(screen_id, fixture_id).expect("audit must exist");
        let user_cells: Vec<_> = audit
            .styled_cells
            .iter()
            .filter(|c| {
                c.style.background == Some(ColorToken::Surface)
                    && c.row >= 3
                    && c.row <= 12
            })
            .collect();
        assert!(
            !user_cells.is_empty(),
            "{screen_id}: expected SURFACE-bg cells on the user-turn strip"
        );
        // Absolute RGB is forbidden off theme-swatch paths; Surface comes from style.background.
        for cell in &user_cells {
            assert_eq!(
                cell.style.background,
                Some(ColorToken::Surface),
                "{screen_id}: glyph {:?} at ({},{}) must use SURFACE style.background",
                cell.glyph,
                cell.row,
                cell.column
            );
        }

        let bytes = render_audit_png(&audit).expect("png");
        let image = load_from_memory(&bytes).expect("valid png").to_rgb8();
        // Prefer a space cell so glyph raster does not tint the mid-cell sample.
        let sample = user_cells
            .iter()
            .find(|c| c.glyph == ' ')
            .or_else(|| user_cells.iter().find(|c| c.glyph == '›'))
            .unwrap_or(&user_cells[0]);
        let px = image.get_pixel(
            sample.column as u32 * CELL_WIDTH + CELL_WIDTH / 2,
            sample.row as u32 * CELL_HEIGHT + CELL_HEIGHT / 2,
        );
        assert_eq!(
            (px[0], px[1], px[2]),
            (surface.red, surface.green, surface.blue),
            "{screen_id}: user-row PNG pixel must be SURFACE #1A1A1C not CANVAS #101011"
        );
        let canvas_px = image.get_pixel(0, 0);
        assert_eq!(
            (canvas_px[0], canvas_px[1], canvas_px[2]),
            (canvas_tok.red, canvas_tok.green, canvas_tok.blue),
            "{screen_id}: empty canvas corner must stay CANVAS"
        );
        assert_ne!(
            (px[0], px[1], px[2]),
            (canvas_px[0], canvas_px[1], canvas_px[2]),
            "{screen_id}: DS-034 requires wash ≠ canvas"
        );
    }
}

#[test]
fn helpers_reject_global_putty_footer_patterns() {
    assert!(contains_global_putty_footer(
        "Shift+Tab: mode  │  Ctrl+c: cancel  │  Ctrl+.: shortcuts"
    ));
    assert!(contains_global_putty_footer("Shift+Tab: mode"));
    assert!(!contains_global_putty_footer(
        "Enter to send · Esc to cancel · ↑↓ to choose"
    ));
    assert!(!contains_global_putty_footer("● Responding · 15s · Esc to stop"));
}

#[test]
fn jewel_and_turn_status_txt_snapshot_contract() {
    let jewel = PromptJewel::new("explain the universe", "Claude Opus 4.8", "Plan mode")
        .unwrap()
        .focused();
    let jewel_snapshot = jewel.render_txt();
    insta_like_assert_contains_geometry(&jewel_snapshot);

    let busy = TurnStatusBand::busy(TurnBusyKind::Responding, "15s")
        .unwrap()
        .with_stop("stop")
        .unwrap();
    let busy_snapshot = busy.render_txt();
    assert_eq!(busy_snapshot.lines().count(), 1);

    let idle_snapshot = TurnStatusBand::idle().render_txt();
    assert!(idle_snapshot.is_empty(), "idle collapse snapshot must be empty");
}

/// Lightweight geometry snapshot without pulling an insta dependency.
fn insta_like_assert_contains_geometry(snapshot: &str) {
    let lines: Vec<&str> = snapshot.lines().collect();
    assert!(lines.len() >= 3, "jewel needs top, body, bottom: {snapshot}");
    assert!(lines[0].starts_with('╭') && lines[0].ends_with('╮'), "{snapshot}");
    assert!(
        lines[lines.len() - 1].starts_with('╰') && lines[lines.len() - 1].ends_with('╯'),
        "{snapshot}"
    );
    assert!(
        lines.iter().any(|l| l.contains("│ › ")),
        "draft prefix row missing: {snapshot}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.contains("Claude Opus 4.8 · Plan mode")),
        "info line missing: {snapshot}"
    );
    for line in &lines {
        assert_eq!(line.chars().count() as u16, OUTER_CONTENT_WIDTH);
    }
}
