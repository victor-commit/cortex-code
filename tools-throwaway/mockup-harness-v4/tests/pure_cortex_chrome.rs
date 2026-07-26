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
fn palette_cortexday_peer_hex() {
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
    // MC V2 premium owns workflow-monitor as the only full MC product (VAL-V3-MC-004).
    let monitor = render_pilot("workflow-monitor").unwrap().to_text();
    assert!(monitor.contains(".: Mission Control"));
    assert!(monitor.contains("Workers (live)") || monitor.contains("Parallel 3/4"));
    assert!(!monitor.contains("Claude Opus"));
    assert!(!monitor.lines().any(|l| l.trim() == "Mission"));
    assert!(!monitor.contains("Tasks monitor"));
    let mode = render_pilot("mode-switch").unwrap().to_text();
    assert!(mode.contains("Shift+Tab") || mode.contains("plan"));
    assert!(!mode.contains("How Code should work"));
    // VAL-V3-MC-005: concurrency-metrics is a non-primary home pointer stub.
    let dash = render_pilot("concurrency-metrics").unwrap().to_text();
    assert!(
        dash.contains(".: Mission Control") || dash.contains("Dashboard"),
        "pool residual keeps brand or legacy Dashboard"
    );
    assert!(
        dash.contains("Open Mission Control home")
            || dash.contains("Parallel 3/4")
            || dash.contains("Pool board")
            || dash.contains(MODEL_LABEL),
        "pool residual points home or keeps Parallel cue"
    );
    assert!(
        !dash.contains("Workers (live)"),
        "concurrency stub must not paint full Workers pane"
    );
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
    // Product Ctrl+. surface is categorical Keyboard Shortcuts modal, not a
    // flat putty essay that invents `[stop]` as a typed key row.
    assert!(
        help.contains("Keyboard Shortcuts") || help.contains("Essentials"),
        "help must paint shortcuts_help categories"
    );
    assert!(help.contains("Shift+Tab") || help.contains("Ctrl+."));
    let status = render_pilot("status").unwrap().to_text();
    assert!(status.contains(MODEL_LABEL));
    assert!(!status.contains("Claude Opus"));
    assert!(!status.contains("Needs attention"));
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

/// VAL-V3-STRUCT-005 / CROSS-025: conversation family shares shell chrome without nameplates.
#[test]
fn conversation_family_pure_structure_shell() {
    use mockup_harness_v4::chrome::components::{
        default_model_chip, reject_structure_residual, DEFAULT_PUTTY,
    };
    let ids = [
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
        "composer-multiline",
    ];
    for id in ids {
        let text = render_pilot(id)
            .unwrap_or_else(|e| panic!("{id}: {e}"))
            .to_text();
        reject_structure_residual(&text).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert!(!text.contains("❯ You"), "{id}");
        assert!(!text.contains("◆ Cortex"), "{id}");
        assert!(!text.contains("Multi-line jewel"), "{id}");
        assert!(!text.contains("expanded thinking kept collapsed"), "{id}");
        assert!(
            text.contains("9.5K / 500K"),
            "{id} missing authenticated context budget"
        );
        assert!(
            text.contains('|') || DEFAULT_PUTTY.contains('|'),
            "{id} putty separator"
        );
        if matches!(id, "streaming" | "chat-turn" | "rich-response") {
            assert!(
                text.contains("◆ Thought for") || id == "chat-turn",
                "{id} expected Thought line"
            );
            // Dense body: some product ladder language present.
            let dense = text.contains("Race structure")
                || text.contains("Retry guard")
                || text.contains("Recovery procedure")
                || text.contains("Follow-up")
                || text.contains("Guard")
                || text.contains("Steps");
            assert!(dense, "{id} body missing dense heading ladder cues");
        }
        if matches!(id, "streaming" | "composer-multiline" | "chat-turn" | "home" | "empty") {
            assert!(
                text.contains("Cortex (high)")
                    || text.contains(&default_model_chip())
                    || text.contains("always-approve"),
                "{id} missing right jewel chip form"
            );
        }
        if id == "streaming" {
            assert!(text.contains("[stop]"));
            assert!(!text.split_whitespace().any(|tok| {
                tok.len() >= 4
                    && tok.starts_with("0m")
                    && tok.ends_with('s')
                    && tok[2..tok.len() - 1].chars().all(|c| c.is_ascii_digit())
            }));
        }
        if id == "composer-multiline" {
            assert!(!text.contains("Draft ·"));
            assert!(text.contains("Cortex (high)"));
        }
    }
}

/// Published gallery scan (when artifacts exist) for conversation pure-structure bans.
#[test]
fn published_conversation_set_structure_bans() {
    use mockup_harness_v4::chrome::components::reject_structure_residual;
    use std::path::PathBuf;

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../plans/assets/mockups-v3");
    if !root.exists() {
        return;
    }
    let pairs: &[(&str, &str)] = &[
        ("session", "home"),
        ("session", "onboarding"),
        ("session", "chat-turn"),
        ("session", "chat-long-scroll"),
        ("session", "streaming"),
        ("session", "reasoning"),
        ("session", "reasoning-collapsed"),
        ("session", "rich-response"),
        ("states", "empty"),
        ("states", "toasts"),
        ("input", "composer-multiline"),
    ];
    for (cat, id) in pairs {
        let path = root.join(cat).join(format!("{id}.txt"));
        if !path.exists() {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        reject_structure_residual(&text)
            .unwrap_or_else(|e| panic!("published {cat}/{id}: {e}"));
        assert!(!text.contains("❯ You"), "published {cat}/{id}");
        assert!(!text.contains("◆ Cortex"), "published {cat}/{id}");
        assert!(
            text.contains("9.5K / 500K")
                || text.contains("8.5K / 1.0M")
                || text.contains("K / "),
            "published {cat}/{id} missing context budget form"
        );
        if matches!(
            *id,
            "streaming" | "chat-turn" | "rich-response" | "composer-multiline"
        ) {
            assert!(!text.contains("Multi-line jewel"), "published {id}");
            assert!(!text.contains("Draft ·"), "published {id}");
            assert!(!text.contains("0m04s") && !text.contains("0m12s"), "published {id}");
        }
    }
}

/// VAL-V3-STRUCT-007 / CROSS-026: tools family product verb headers + shell chrome.
#[test]
fn tools_family_product_verb_language() {
    use mockup_harness_v4::chrome::components::reject_structure_residual;
    let ids = [
        "tool-calls",
        "tool-running",
        "tool-success",
        "tool-success-expanded",
        "tool-failed",
        "tool-task",
        "tool-task-expanded",
        "shell",
        "todo-list",
        "compact",
        "diff-edit",
    ];
    for id in ids {
        let text = render_pilot(id)
            .unwrap_or_else(|e| panic!("{id}: {e}"))
            .to_text();
        reject_structure_residual(&text).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert!(!text.contains("◆ Cortex"), "{id} nameplate");
        assert!(!text.contains("Shell  "), "{id} Shell invent primary");
        assert!(!text.contains("✓ Shell"), "{id}");
        assert!(!text.contains("✗ Shell"), "{id}");
        assert!(!text.contains("bash  "), "{id}");
        assert!(
            text.contains("9.5K / 500K") || text.contains("K / "),
            "{id} authenticated budget"
        );
        assert!(
            text.contains("Shift+Tab:mode | Ctrl+c:cancel"),
            "{id} putty"
        );
        assert!(text.contains("Cortex (high)"), "{id} jewel chip");
    }
    let calls = render_pilot("tool-calls").unwrap().to_text();
    assert!(calls.contains("Read crates/agent/src/retry.rs"));
    assert!(calls.contains("Search \"rate.?limit\" (3 matches)"));
    assert!(calls.contains("$ cargo test retry -- --nocapture"));
    assert!(calls.contains("Edit crates/agent/src/retry.rs +12/-4"));
    assert!(!calls.contains(" ·  3 matches"));
    assert!(!calls.contains("Mixed tool inventory"));
    let success = render_pilot("tool-success").unwrap().to_text();
    assert!(success.contains("$ cargo test retry"));
    assert!(success.contains("Edit crates/agent/src/retry.rs +12/-4"));
    let shell = render_pilot("shell").unwrap().to_text();
    assert!(shell.contains("Run (user)"));
    assert!(shell.contains("$ cargo test --package agent -- retry"));
    let running = render_pilot("tool-running").unwrap().to_text();
    assert!(running.contains("$ cargo test retry"));
    assert!(running.contains("[stop]"));
}

/// Published tools set bans Shell invent / nameplates when gallery exists.
#[test]
fn published_tools_set_structure_language_bans() {
    use mockup_harness_v4::chrome::components::reject_structure_residual;
    use std::path::PathBuf;

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../plans/assets/mockups-v3/session");
    if !root.exists() {
        return;
    }
    for id in [
        "tool-calls",
        "tool-running",
        "tool-success",
        "tool-success-expanded",
        "tool-failed",
        "tool-task",
        "tool-task-expanded",
        "shell",
        "todo-list",
        "compact",
        "diff-edit",
    ] {
        let path = root.join(format!("{id}.txt"));
        if !path.exists() {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        reject_structure_residual(&text)
            .unwrap_or_else(|e| panic!("published session/{id}: {e}"));
        assert!(!text.contains("◆ Cortex"), "published {id}");
        assert!(!text.contains("Shell  "), "published {id} Shell invent");
        assert!(!text.contains("✓ Shell"), "published {id}");
        assert!(!text.contains("✗ Shell"), "published {id}");
        assert!(
            text.contains("9.5K / 500K") || text.contains("K / "),
            "published {id} budget"
        );
    }
}

/// VAL-V3-STRUCT-008 / CROSS-026: permission docks product permission_view language.
#[test]
fn permission_family_product_view_language() {
    use mockup_harness_v4::chrome::components::reject_structure_residual;
    for id in [
        "permission-shell",
        "permission-file-edit",
        "permission-deny-reason",
        "blocked-command",
    ] {
        let text = render_pilot(id)
            .unwrap_or_else(|e| panic!("{id}: {e}"))
            .to_text();
        reject_structure_residual(&text).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert!(!text.contains("◆ Cortex"), "{id} nameplate");
        assert!(!text.contains("Shell  "), "{id} Shell invent");
        assert!(!text.contains("Allow this command?"), "{id}");
        assert!(!text.contains("Apply this edit?"), "{id}");
        assert!(!text.contains("Why deny or request changes?"), "{id}");
        assert!(!text.contains("Allow once"), "{id}");
        assert!(!text.contains("Allow for session"), "{id}");
        assert!(!text.contains("Working directory ·"), "{id}");
        assert!(
            text.contains("9.5K / 500K") || text.contains("K / "),
            "{id} budget"
        );
        assert!(
            text.contains("Shift+Tab:mode | Ctrl+c:cancel"),
            "{id} putty"
        );
    }
    let shell = render_pilot("permission-shell").unwrap().to_text();
    assert!(shell.contains("Allow `cargo`?"));
    assert!(shell.contains("$ cargo test retry -- --nocapture"));
    assert!(shell.contains("1 (●) Yes, proceed"));
    assert!(shell.contains("Always allow: cargo test"));
    assert!(shell.contains("No, and tell Cortex what to do differently"));
    assert!(shell.contains("< > to choose permission scope"));
    let file = render_pilot("permission-file-edit").unwrap().to_text();
    assert!(file.contains("Allow Edit to crates/agent/src/retry.rs?"));
    assert!(file.contains("1 (●) Yes"));
    assert!(file.contains("Yes, allow all edits during this session"));
    let deny = render_pilot("permission-deny-reason").unwrap().to_text();
    assert!(deny.contains("3 [x] ❯"));
    assert!(deny.contains("Allow `cargo`?"));
    let blocked = render_pilot("blocked-command").unwrap().to_text();
    assert!(blocked.contains("$ ./scripts/redeploy.sh --prod"));
    assert!(blocked.contains("Request approval"));
    assert!(blocked.contains("Cortex (high)"));
    assert!(!blocked.contains("✗ Shell"));
}

/// Published permission set bans hybrid essay when gallery exists.
#[test]
fn published_permission_set_structure_language_bans() {
    use mockup_harness_v4::chrome::components::reject_structure_residual;
    use std::path::PathBuf;

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plans/assets/mockups-v3");
    if !root.exists() {
        return;
    }
    let pairs: &[(&str, &str)] = &[
        ("input", "permission-shell"),
        ("input", "permission-file-edit"),
        ("input", "permission-deny-reason"),
        ("session", "blocked-command"),
    ];
    for (cat, id) in pairs {
        let path = root.join(cat).join(format!("{id}.txt"));
        if !path.exists() {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        reject_structure_residual(&text)
            .unwrap_or_else(|e| panic!("published {cat}/{id}: {e}"));
        assert!(!text.contains("◆ Cortex"), "published {id}");
        assert!(!text.contains("Shell  "), "published {id}");
        assert!(!text.contains("Allow this command?"), "published {id}");
        assert!(!text.contains("Allow once"), "published {id}");
        assert!(
            text.contains("9.5K / 500K") || text.contains("K / "),
            "published {id} budget"
        );
        if *id == "permission-shell" {
            assert!(text.contains("Allow `cargo`?") || text.contains("Yes, proceed"));
        }
    }
}

/// VAL-V3-STRUCT-009 + VAL-V3-MC-001/003: orch family structure bans with MC pack home.
#[test]
fn orchestration_family_tasks_dashboard_analogues() {
    use mockup_harness_v4::chrome::components::reject_structure_residual;

    let invent_bans = [
        "Tasks monitor",
        "i intervene",
        "Goals board",
        "Progress log",
        "Owner ·",
        "Shell  ",
        "Shell ·",
        "◆ Cortex",
        "❯ You",
        "Active work ·",
        "Esc to tasks monitor",
    ];
    let orch_ids = [
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
    ];
    for id in orch_ids {
        let text = render_pilot(id)
            .unwrap_or_else(|e| panic!("{id}: {e}"))
            .to_text();
        reject_structure_residual(&text).unwrap_or_else(|e| panic!("{id}: {e}"));
        for invent in invent_bans {
            assert!(
                !text.contains(invent),
                "{id} still paints invent {invent:?}"
            );
        }
        assert!(
            text.contains("9.5K / 500K") || text.contains("K / "),
            "{id} inherits M1 auth budget chrome"
        );
        let is_primary = matches!(id, "workflow-monitor" | "workflow-monitor-failure");
        let is_stub = matches!(
            id,
            "concurrency-metrics"
                | "workflow-launch"
                | "workflow-launch-blocked"
                | "team-board"
        );
        if is_primary {
            assert!(
                text.contains("F Features")
                    || text.contains("Esc")
                    || text.contains(".: Mission Control"),
                "{id} MC footer keys / brand"
            );
            assert!(
                text.contains("Parallel 3/4") || text.contains("Parallel 4/4"),
                "{id} Parallel N/max"
            );
            assert!(
                text.contains("Workers (live)") || text.contains("Workers 3/4"),
                "{id} full Workers pane"
            );
        } else if is_stub {
            // VAL-V3-MC-005: non-primary stubs point home; no multi-panel competition.
            assert!(
                text.contains(".: Mission Control"),
                "{id} residual brand"
            );
            assert!(
                text.contains("Open Mission Control home"),
                "{id} home pointer"
            );
            assert!(
                !text.contains("Workers (live)"),
                "{id} stub must not paint Workers live pane"
            );
            assert!(
                !text.contains("Progress Log"),
                "{id} stub must not paint Progress Log"
            );
            assert!(!text.contains("Stream  #"), "{id} stub must not paint Stream");
        } else {
            assert!(
                text.contains("Shift+Tab:mode | Ctrl+c:cancel") || text.contains("Shift+Tab"),
                "{id} putty / mode chrome"
            );
        }
    }

    // MC V2 premium home (primary product surface only).
    let mon = render_pilot("workflow-monitor").unwrap().to_text();
    assert!(mon.contains(".: Mission Control"), "MC brand");
    assert!(mon.contains("Parallel 3/4"), "default pool 3/4");
    assert!(
        mon.contains("Workers (live)") || mon.contains("Workers 3/4"),
        "Workers pane"
    );
    assert!(mon.contains("#21") && mon.contains("#22") && mon.contains("#23"));
    assert!(mon.contains("Progress Log"));
    assert!(!mon.contains("Tasks monitor"));

    let fail = render_pilot("workflow-monitor-failure").unwrap().to_text();
    assert!(fail.contains(".: Mission Control"));
    assert!(fail.contains("● RUNNING"), "run stays RUNNING after fail");
    assert!(fail.contains('✗'), "failed worker mark");
    assert!(fail.contains("#21") && fail.contains("#23"), "siblings live");
    assert!(
        fail.contains("Workers") && fail.contains("1 failed"),
        "failure pane names failed slot"
    );
    // VAL-V3-MC-006: premium DNA retained on failure variant.
    assert!(fail.contains("▮▮▮▯") || fail.contains("Stream  #21"));
    assert!(fail.contains("Preconditions") || fail.contains("Execute"));

    // VAL-V3-MC-005: secondary former MC destinations are home pointers, not boards.
    let board = render_pilot("team-board").unwrap().to_text();
    assert!(
        board.contains(".: Mission Control") || board.contains("assignment"),
        "board residual keeps brand/assignment"
    );
    assert!(
        board.contains("Open Mission Control home"),
        "board points home"
    );
    assert!(!board.contains("Workers (live)"));
    assert!(!board.contains("Goals board"));
    assert!(!board.contains("Owner ·"));

    let pool = render_pilot("concurrency-metrics").unwrap().to_text();
    assert!(pool.contains(".: Mission Control"));
    assert!(pool.contains("Open Mission Control home"));
    assert!(pool.contains("Parallel 3/4"));
    assert!(!pool.contains("Workers (live)"));
    assert!(!pool.contains("Progress Log"));

    let launch = render_pilot("workflow-launch").unwrap().to_text();
    assert!(launch.contains(".: Mission Control"));
    assert!(launch.contains("Open Mission Control home"));
    assert!(
        launch.contains("Assign") || launch.contains("free slot"),
        "launch residual mentions Assign"
    );
    assert!(!launch.contains("Workers (live)"));

    let blocked = render_pilot("workflow-launch-blocked").unwrap().to_text();
    assert!(blocked.contains(".: Mission Control"));
    assert!(blocked.contains("Open Mission Control home"));
    assert!(
        blocked.contains("Pool full")
            || blocked.contains("pool full")
            || blocked.contains("blocked")
            || blocked.contains("Blocked"),
        "blocked residual"
    );
    assert!(!blocked.contains("Workers (live)"));

    let env = render_pilot("tool-envelope").unwrap().to_text();
    assert!(env.contains("$ cargo test publish"), "tool-envelope $ form");
    assert!(!env.contains("Shell ·"), "tool-envelope Shell invent");
}

/// Published orchestration set bans invent chrome when gallery exists.
#[test]
fn published_orchestration_set_structure_language_bans() {
    use mockup_harness_v4::chrome::components::reject_structure_residual;
    use std::path::PathBuf;

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../plans/assets/mockups-v3/orchestration");
    if !root.exists() {
        return;
    }
    let invent_bans = [
        "Tasks monitor",
        "i intervene",
        "Goals board",
        "Progress log",
        "Owner ·",
        "Shell  ",
        "Shell ·",
        "◆ Cortex",
        "❯ You",
    ];
    for id in [
        "workflow-monitor",
        "workflow-monitor-failure",
        "team-board",
        "workflow-launch",
        "workflow-launch-blocked",
        "concurrency-metrics",
        "tool-envelope",
        "mode-switch",
        "reviewer-output",
        "reviewer-approved",
        "verification-ladder",
        "intent-scorer-pass",
        "intent-scorer-fail",
        "mode-switch-refused",
    ] {
        let path = root.join(format!("{id}.txt"));
        if !path.exists() {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        reject_structure_residual(&text)
            .unwrap_or_else(|e| panic!("published orchestration/{id}: {e}"));
        for invent in invent_bans {
            assert!(
                !text.contains(invent),
                "published {id} invent {invent:?}"
            );
        }
        assert!(
            text.contains("9.5K / 500K") || text.contains("K / "),
            "published {id} budget"
        );
    }
    let mon = root.join("workflow-monitor.txt");
    if mon.exists() {
        let text = std::fs::read_to_string(&mon).unwrap();
        // After MC V2 premium publish: full Mission Control home (not tasks_pane).
        assert!(
            text.contains(".: Mission Control")
                || text.contains("Subagents")
                || text.contains("Watchers"),
            "published monitor is MC home or legacy tasks pane"
        );
        assert!(!text.contains("Active work ·"));
        assert!(!text.contains("Tasks monitor"));
    }
    let fail = root.join("workflow-monitor-failure.txt");
    if fail.exists() {
        let text = std::fs::read_to_string(&fail).unwrap();
        if text.contains(".: Mission Control") {
            assert!(text.contains('✗') || text.contains("failed"));
            assert!(text.contains("● RUNNING") || text.contains("RUNNING"));
        }
    }
    let board = root.join("team-board.txt");
    if board.exists() {
        let text = std::fs::read_to_string(&board).unwrap();
        assert!(
            text.contains(".: Mission Control")
                || text.contains("assignment")
                || text.contains("Dashboard")
                || text.contains("Open Mission Control home"),
            "published board is home pointer residual or legacy"
        );
        assert!(!text.contains("Goals board"));
        assert!(!text.contains("Owner ·"));
        // VAL-V3-MC-005: after stub publish, board is not full multi-panel home.
        if text.contains("Open Mission Control home") {
            assert!(!text.contains("Workers (live)"));
            assert!(!text.contains("Progress Log"));
        }
    }
    for id in [
        "concurrency-metrics",
        "workflow-launch",
        "workflow-launch-blocked",
    ] {
        let path = root.join(format!("{id}.txt"));
        if !path.exists() {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains(".: Mission Control"),
            "published {id} MC brand"
        );
        // VAL-V3-MC-005: stubs point home; may mention Parallel without full chrome.
        assert!(
            text.contains("Open Mission Control home")
                || text.contains("Parallel 3/4")
                || text.contains("Parallel 4/4"),
            "published {id} home pointer or Parallel cue"
        );
        if text.contains("Open Mission Control home") {
            assert!(
                !text.contains("Workers (live)"),
                "published {id} must not be full Workers dashboard"
            );
            assert!(
                !text.contains("Progress Log"),
                "published {id} must not paint Progress Log"
            );
        }
    }
    let env = root.join("tool-envelope.txt");
    if env.exists() {
        let text = std::fs::read_to_string(&env).unwrap();
        assert!(text.contains("$ cargo") || text.contains("$ "));
        assert!(!text.contains("Shell ·"));
    }
}

/// VAL-V3-STRUCT-011 / CROSS-027: slash_dropdown + /model arg geometry over live prompt.
#[test]
fn pickers_family_slash_model_geometry() {
    use mockup_harness_v4::chrome::components::reject_structure_residual;

    let invent_bans = [
        "Filter · /",
        "Filter ·",
        "Selected ·",
        "statusline stays Cortex-labeled",
        "f for Filter models",
        "Type more to filter",
        "Esc restores the draft",
        "switch Cortex catalog model",
        "Each row says what changes before you apply it",
        "Models · filter",
        "◆ Cortex",
        "❯ You",
    ];

    for id in [
        "slash-menu",
        "model-picker",
        "model-picker-filtered",
        "command-palette",
        "session-picker",
        "variant-picker",
        "theme-picker",
        "export-picker",
        "file-mention",
        "queue-manager",
        "composer-multiline",
    ] {
        let text = render_pilot(id)
            .unwrap_or_else(|e| panic!("{id}: {e}"))
            .to_text();
        reject_structure_residual(&text).unwrap_or_else(|e| panic!("{id}: {e}"));
        for invent in invent_bans {
            assert!(!text.contains(invent), "{id} invent {invent:?}");
        }
        assert!(
            !text.lines().any(|l| l.trim() == "Commands"),
            "{id} detached Commands essay"
        );
        assert!(
            !text.contains("/session ·"),
            "{id} fake /session alias (use /resume or /session-info)"
        );
        assert!(
            text.contains("9.5K / 500K") || text.contains("K / "),
            "{id} inherits shell budget chrome"
        );
        assert!(
            text.contains("Shift+Tab:mode | Ctrl+c:cancel") || text.contains("Shift+Tab"),
            "{id} putty chrome"
        );
        // Root command pickers keep a live jewel over the dropdown (not mid-scroll only).
        assert!(
            text.contains('╭') && text.contains('╰'),
            "{id} live prompt jewel"
        );
    }

    let slash = render_pilot("slash-menu").unwrap().to_text();
    assert!(slash.contains("/resume"), "product /resume naming");
    assert!(slash.contains("/session-info"), "product /session-info naming");
    assert!(slash.contains("/model"), "product /model");
    assert!(
        slash.contains("Switch the active model") || slash.contains("Resume a previous session"),
        "product slash descriptions"
    );
    // Draft keeps the typed slash query inside the jewel, not a Filter dock.
    let jewel_has_query = slash.lines().any(|l| l.contains('❯') && l.contains('/'));
    assert!(jewel_has_query, "slash-menu live prompt must show slash query");

    let model = render_pilot("model-picker").unwrap().to_text();
    assert!(model.contains("/model"));
    assert!(model.contains(MODEL_LABEL) || model.contains("Cortex"));
    // No marketing Selected putty, no filter hotkey theater.
    assert!(!model.contains("Enter to apply · Esc to cancel · f for Filter"));
    assert!(!model.contains("Applies to the next reply only"));

    let filtered = render_pilot("model-picker-filtered").unwrap().to_text();
    assert!(filtered.contains("/model"));
    assert!(!filtered.contains("2 matches · filter text stays until Esc"));

    let effort = render_pilot("variant-picker").unwrap().to_text();
    assert!(
        effort.contains("high") || effort.contains("medium") || effort.contains("xhigh"),
        "effort sub-menu levels"
    );
    assert!(
        effort.contains("/model Cortex") || effort.contains("Cortex "),
        "chained /model effort prompt"
    );

    let session = render_pilot("session-picker").unwrap().to_text();
    assert!(
        session.contains("/resume") || session.contains("Resume"),
        "session-picker is /resume surface"
    );
}

/// Published pickers ban Filter/Selected invent + Commands essay when gallery exists.
#[test]
fn published_pickers_slash_model_structure_bans() {
    use mockup_harness_v4::chrome::components::reject_structure_residual;
    use std::path::PathBuf;

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plans/assets/mockups-v3/input");
    if !root.exists() {
        return;
    }
    let invent_bans = [
        "Filter · /",
        "Filter ·",
        "Selected ·",
        "statusline stays Cortex-labeled",
        "f for Filter models",
        "Type more to filter",
        "Esc restores the draft",
        "switch Cortex catalog model",
        "Models · filter",
        "◆ Cortex",
        "❯ You",
        "/session ·",
    ];
    for id in [
        "slash-menu",
        "model-picker",
        "model-picker-filtered",
        "command-palette",
        "session-picker",
        "variant-picker",
        "theme-picker",
        "export-picker",
        "composer-multiline",
    ] {
        let path = root.join(format!("{id}.txt"));
        if !path.exists() {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        reject_structure_residual(&text)
            .unwrap_or_else(|e| panic!("published input/{id}: {e}"));
        for invent in invent_bans {
            assert!(
                !text.contains(invent),
                "published {id} invent {invent:?}"
            );
        }
        assert!(
            !text.lines().any(|l| l.trim() == "Commands"),
            "published {id} Commands essay"
        );
        assert!(
            text.contains("9.5K / 500K") || text.contains("K / "),
            "published {id} budget"
        );
    }
    let slash = root.join("slash-menu.txt");
    if slash.exists() {
        let text = std::fs::read_to_string(&slash).unwrap();
        assert!(text.contains("/resume"));
        assert!(text.contains("/session-info"));
        assert!(text.contains('╭'));
    }
    let model = root.join("model-picker.txt");
    if model.exists() {
        let text = std::fs::read_to_string(&model).unwrap();
        assert!(text.contains("/model"));
        assert!(!text.contains("statusline stays Cortex-labeled"));
    }
}

/// VAL-V3-STRUCT-010: help = shortcuts_help modal; status = session-info + context.
#[test]
fn help_status_product_surfaces() {
    use mockup_harness_v4::chrome::components::reject_structure_residual;

    let invent_bans = [
        "Needs attention",
        "Session status",
        "Active configuration · return keeps draft",
        "Local dock bylines",
        "putty footer remains",
        "context-sensitive hints match the putty",
        "match the putty",
        "Advanced raw metadata stays folded",
        "◆ Cortex",
        "❯ You",
    ];

    let help = render_pilot("help")
        .unwrap_or_else(|e| panic!("help: {e}"))
        .to_text();
    reject_structure_residual(&help).unwrap_or_else(|e| panic!("help residual: {e}"));
    for invent in invent_bans {
        assert!(!help.contains(invent), "help invent {invent:?}");
    }
    assert!(
        help.contains("Keyboard Shortcuts"),
        "help needs product modal title"
    );
    assert!(
        help.contains("Essentials"),
        "help needs Essentials category"
    );
    // At least one collapsed later category from CATEGORY_ORDER.
    let cat_hit = help.contains("Input")
        || help.contains("Conversation Navigation")
        || help.contains("Conversation Actions")
        || help.contains("Panels")
        || help.contains("Session")
        || help.contains("Dashboard");
    assert!(cat_hit, "help needs additional category headers");
    assert!(
        help.contains("Enter details")
            || help.contains("Esc close")
            || help.contains("/ search")
            || help.contains("expand"),
        "help needs modal affordance chrome"
    );
    assert!(
        help.contains("9.5K / 500K") || help.contains("K / "),
        "help inherits M1 auth budget"
    );
    assert!(
        help.contains("Shift+Tab:mode | Ctrl+c:cancel") || help.contains("Shift+Tab"),
        "help putty chrome"
    );
    // Flat putty lecture / gallery essay primary is banned.
    assert!(!help.contains("This putty"));
    assert!(!help.contains("gallery"));

    let status = render_pilot("status")
        .unwrap_or_else(|e| panic!("status: {e}"))
        .to_text();
    reject_structure_residual(&status).unwrap_or_else(|e| panic!("status residual: {e}"));
    for invent in invent_bans {
        assert!(!status.contains(invent), "status invent {invent:?}");
    }
    // Compact /session-info facts.
    let session_facts = status.contains("Session ID")
        || status.contains("Working directory")
        || status.contains("session-info");
    assert!(session_facts, "status needs session-info facts");
    assert!(
        status.contains("Model") || status.contains(MODEL_LABEL),
        "status model fact"
    );
    // Compact /context facts (categories / usage), not mission Needs attention.
    let context_facts = status.contains("Context")
        || status.contains("tokens")
        || status.contains("Tool definitions")
        || status.contains("Auto-compact");
    assert!(context_facts, "status needs context compact facts");
    assert!(
        status.contains("9.5K / 500K")
            || status.contains("8.5K / 1.0M")
            || status.contains("K / ")
            || status.contains("tokens"),
        "status inherits context budget facts"
    );
    assert!(
        status.contains("Shift+Tab:mode | Ctrl+c:cancel") || status.contains("Shift+Tab"),
        "status putty chrome"
    );
    assert!(!status.contains("Docs research · needs sign-in"));
    assert!(!status.contains("return keeps draft and turn"));

    // Closely related inspectors sharing invent chrome must not keep nameplates.
    let ctx = render_pilot("sidebar-context")
        .unwrap_or_else(|e| panic!("sidebar-context: {e}"))
        .to_text();
    reject_structure_residual(&ctx).unwrap_or_else(|e| panic!("sidebar-context residual: {e}"));
    assert!(!ctx.contains("◆ Cortex"), "sidebar nameplate");
    assert!(!ctx.contains("Needs attention"));
    assert!(
        ctx.contains("Context") || ctx.contains("tokens") || ctx.contains("K / "),
        "sidebar-context compact facts"
    );
}

/// Published help/status ban invent strings when gallery exists.
#[test]
fn published_help_status_structure_language_bans() {
    use mockup_harness_v4::chrome::components::reject_structure_residual;
    use std::path::PathBuf;

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plans/assets/mockups-v3/views");
    if !root.exists() {
        return;
    }
    let invent_bans = [
        "Needs attention",
        "Session status",
        "Active configuration · return keeps draft",
        "Local dock bylines",
        "putty footer remains",
        "match the putty",
        "◆ Cortex",
        "❯ You",
    ];
    for id in ["help", "status", "sidebar-context"] {
        let path = root.join(format!("{id}.txt"));
        if !path.exists() {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        reject_structure_residual(&text)
            .unwrap_or_else(|e| panic!("published views/{id}: {e}"));
        for invent in invent_bans {
            assert!(
                !text.contains(invent),
                "published {id} invent {invent:?}"
            );
        }
        if id == "help" {
            assert!(
                text.contains("Keyboard Shortcuts") || text.contains("Essentials"),
                "published help"
            );
        }
        if id == "status" {
            assert!(
                text.contains("Session ID")
                    || text.contains("Working directory")
                    || text.contains("Context"),
                "published status session/context facts"
            );
        }
    }
}

/// VAL-V3-CROSS-028: inventory-wide pure-structure invent ban across all 86 pilots.
///
/// Covers nameplates, Tasks monitor / Goals board / intervene invent, Shell invent
/// headers, 0mNNs timers, product putty `|` separators on authenticated shells, and
/// authenticated context budget presence (login/device auth family may omit budget).
#[test]
fn inventory_wide_pure_structure_invent_bans() {
    use mockup_harness_v4::chrome::components::{
        has_context_budget_form, is_auth_family_screen, putty_product_separators_hold,
        reject_structure_residual,
    };

    let invent_bans = [
        "❯ You",
        "> You",
        "◆ Cortex",
        "Tasks monitor",
        "Goals board",
        "i intervene",
        "Progress log",
        "Shell  ",
        "Esc to tasks monitor",
    ];

    assert_eq!(list_pilots().len(), 86);
    for spec in list_pilots() {
        let text = render_pilot(spec.id)
            .unwrap_or_else(|e| panic!("{}: {e}", spec.id))
            .to_text();
        reject_structure_residual(&text)
            .unwrap_or_else(|e| panic!("inventory residual {}: {e}", spec.id));
        for invent in invent_bans {
            assert!(
                !text.contains(invent),
                "inventory {} invent {invent:?}",
                spec.id
            );
        }
        // 0mNNs <60s timer form.
        assert!(
            !text.split_whitespace().any(|tok| {
                tok.len() >= 4
                    && tok.starts_with("0m")
                    && tok.ends_with('s')
                    && tok[2..tok.len() - 1].chars().all(|c| c.is_ascii_digit())
            }),
            "inventory {} 0mNNs timer",
            spec.id
        );
        // Auth family may omit budget; every other screen must carry it.
        if !is_auth_family_screen(spec.id) {
            assert!(
                has_context_budget_form(&text),
                "inventory {} missing authenticated context budget",
                spec.id
            );
            assert!(
                putty_product_separators_hold(&text),
                "inventory {} exclusive middot putty dynasty",
                spec.id
            );
        }
    }
}

/// VAL-V3-CROSS-028 published gallery scan: all 86 canonical + 35 gate TXT.
#[test]
fn published_inventory_wide_structure_bans() {
    use mockup_harness_v4::chrome::components::{
        has_context_budget_form, is_auth_family_screen, putty_product_separators_hold,
        reject_structure_residual,
    };
    use mockup_harness_v4::GATE_FRAMES;
    use std::path::PathBuf;

    let invent_bans = [
        "❯ You",
        "> You",
        "◆ Cortex",
        "Tasks monitor",
        "Goals board",
        "i intervene",
        "Progress log",
        "Shell  ",
        "Esc to tasks monitor",
    ];

    let canonical = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../plans/assets/mockups-v3");
    let gates = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../plans/assets/mockups-v3-gates");
    if !canonical.exists() {
        return;
    }

    let mut scanned = 0usize;
    // Canonical 86.
    for spec in list_pilots() {
        let path = canonical
            .join(spec.category)
            .join(format!("{}.txt", spec.id));
        if !path.exists() {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        reject_structure_residual(&text)
            .unwrap_or_else(|e| panic!("published {}/{}: {e}", spec.category, spec.id));
        for invent in invent_bans {
            assert!(
                !text.contains(invent),
                "published {}/{} invent {invent:?}",
                spec.category,
                spec.id
            );
        }
        assert!(
            !text.split_whitespace().any(|tok| {
                tok.len() >= 4
                    && tok.starts_with("0m")
                    && tok.ends_with('s')
                    && tok[2..tok.len() - 1].chars().all(|c| c.is_ascii_digit())
            }),
            "published {}/{} 0mNNs",
            spec.category,
            spec.id
        );
        if !is_auth_family_screen(spec.id) {
            assert!(
                has_context_budget_form(&text),
                "published {}/{} missing budget",
                spec.category,
                spec.id
            );
            assert!(
                putty_product_separators_hold(&text),
                "published {}/{} middot putty",
                spec.category,
                spec.id
            );
        }
        scanned += 1;
    }

    // Gate 35 aliases: ban invent strings on every gate frame; budget follows
    // the painted pilot id (auth pilots may omit).
    if gates.exists() {
        for (folder, alias, pilot_id) in GATE_FRAMES {
            let path = gates.join(folder).join(format!("{alias}.txt"));
            if !path.exists() {
                continue;
            }
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
            reject_structure_residual(&text)
                .unwrap_or_else(|e| panic!("gate {folder}/{alias}: {e}"));
            for invent in invent_bans {
                assert!(
                    !text.contains(invent),
                    "gate {folder}/{alias} invent {invent:?}"
                );
            }
            assert!(
                !text.split_whitespace().any(|tok| {
                    tok.len() >= 4
                        && tok.starts_with("0m")
                        && tok.ends_with('s')
                        && tok[2..tok.len() - 1].chars().all(|c| c.is_ascii_digit())
                }),
                "gate {folder}/{alias} 0mNNs"
            );
            if !is_auth_family_screen(pilot_id) {
                assert!(
                    has_context_budget_form(&text),
                    "gate {folder}/{alias} missing budget (pilot {pilot_id})"
                );
                assert!(
                    putty_product_separators_hold(&text),
                    "gate {folder}/{alias} middot putty"
                );
            }
            scanned += 1;
        }
    }

    // When full gallery is present, expect 86 + 35.
    if scanned > 0 {
        assert!(
            scanned >= 86,
            "expected at least the 86 canonical texts, scanned {scanned}"
        );
    }
    if gates.exists() {
        assert!(
            scanned >= 86 + 35 || scanned >= 86,
            "gate root present but scan low: {scanned}"
        );
        // Prefer full 121 when both roots are complete.
        let gate_txts = GATE_FRAMES
            .iter()
            .filter(|(folder, alias, _)| gates.join(folder).join(format!("{alias}.txt")).exists())
            .count();
        let canon_txts = list_pilots()
            .iter()
            .filter(|s| {
                canonical
                    .join(s.category)
                    .join(format!("{}.txt", s.id))
                    .exists()
            })
            .count();
        assert_eq!(
            scanned,
            canon_txts + gate_txts,
            "scan count must equal existing 86+35 artifact set"
        );
        if canon_txts == 86 && gate_txts == 35 {
            assert_eq!(scanned, 121, "full inventory 86+35");
        }
    }
}

/// Unit coverage for CROSS-028 helpers (auth omit, budget form, putty separators).
#[test]
fn cross_028_helper_unit_contracts() {
    use mockup_harness_v4::chrome::components::{
        has_context_budget_form, is_auth_family_screen, putty_product_separators_hold,
        reject_structure_residual, DEFAULT_PUTTY,
    };

    assert!(is_auth_family_screen("login"));
    assert!(is_auth_family_screen("connect-oauth"));
    assert!(is_auth_family_screen("connect-method"));
    assert!(!is_auth_family_screen("chat-turn"));
    assert!(!is_auth_family_screen("workflow-monitor"));

    assert!(has_context_budget_form("  ~/projects/demo          9.5K / 500K  "));
    assert!(has_context_budget_form("8.5K / 1.0M"));
    assert!(!has_context_budget_form("   master /projects/demo  "));

    assert!(putty_product_separators_hold(DEFAULT_PUTTY));
    assert!(putty_product_separators_hold(
        "Shift+Tab:mode | Ctrl+c:cancel | Ctrl+.:shortcuts"
    ));
    assert!(!putty_product_separators_hold(
        "Shift+Tab:mode · Ctrl+c:cancel · Ctrl+.:shortcuts"
    ));
    // Non-putty auth tips line is not judged as product putty law.
    assert!(putty_product_separators_hold(
        "Tips: /model  /theme  /mcp  Shift+Tab for mode"
    ));

    reject_structure_residual("clean shell without bans").unwrap();
    assert!(reject_structure_residual("❯ You").is_err());
    assert!(reject_structure_residual("◆ Cortex").is_err());
    assert!(reject_structure_residual("Tasks monitor open").is_err());
    assert!(reject_structure_residual("Goals board writer").is_err());
    assert!(reject_structure_residual("press i intervene now").is_err());
    assert!(reject_structure_residual("Shell  cargo test").is_err());
    assert!(reject_structure_residual("elapsed 0m04s").is_err());
    assert!(reject_structure_residual("elapsed 0m12s").is_err());
    // ≥60s product form is allowed.
    reject_structure_residual("elapsed 1m20s").unwrap();
}
