use mockup_harness_v3::chrome::{
    components::{
        ActivityRow, ActivityState, Component, ConnectionRow, ConnectionState, DecisionDock,
        DiffBlock, DiffKind, DisclosureBlock, DisclosureState, Inspector, InspectorTab,
        MissionCard, RecoveryBlock, ReviewBlock, SelectionDock, SelectionRow,
    },
    layout::{ScreenComposition, StatusLine, SurfaceLayout, CANVAS_COLUMNS, CANVAS_ROWS},
    tokens::{
        allowed_glyphs, palette, validate_glyphs, ColorToken, Theme, ACCENT_USE_BUDGET, PNG_HEIGHT,
        PNG_WIDTH,
    },
};
use mockup_harness_v3::Surface;

#[test]
fn dark_and_light_palettes_match_the_authoritative_tokens() {
    let dark = palette(Theme::Dark);
    let light = palette(Theme::Light);

    assert_eq!(dark.hex(ColorToken::Canvas), "#101011");
    assert_eq!(dark.hex(ColorToken::Surface), "#1A1A1C");
    assert_eq!(dark.hex(ColorToken::Text), "#DDDAD4");
    assert_eq!(dark.hex(ColorToken::Bright), "#F6F3ED");
    assert_eq!(dark.hex(ColorToken::Muted), "#99958E");
    assert_eq!(dark.hex(ColorToken::Faint), "#5F5C57");
    assert_eq!(dark.hex(ColorToken::Border), "#393735");
    assert_eq!(dark.hex(ColorToken::Accent), "#D9825F");
    assert_eq!(dark.hex(ColorToken::Info), "#7D9BB8");
    assert_eq!(dark.hex(ColorToken::Ok), "#79A889");
    assert_eq!(dark.hex(ColorToken::Warn), "#C29A60");
    assert_eq!(dark.hex(ColorToken::Bad), "#C66F6F");
    assert_eq!(dark.hex(ColorToken::AddBackground), "#17241D");
    assert_eq!(dark.hex(ColorToken::DeleteBackground), "#28191B");

    assert_eq!(light.hex(ColorToken::Canvas), "#FAF7F2");
    assert_eq!(light.hex(ColorToken::Surface), "#E8E4DC");
    assert_eq!(light.hex(ColorToken::Text), "#33383F");
    assert_eq!(light.hex(ColorToken::Bright), "#14171C");
    assert_eq!(light.hex(ColorToken::Muted), "#6B7280");
    assert_eq!(light.hex(ColorToken::Faint), "#C3C8D1");
    assert_eq!(light.hex(ColorToken::Border), "#7C828C");
    assert_eq!(light.hex(ColorToken::Accent), "#C06B44");
    assert_eq!(light.hex(ColorToken::Info), "#3D6FA8");
    assert_eq!(light.hex(ColorToken::Ok), "#4F7942");
    assert_eq!(light.hex(ColorToken::Warn), "#A87A2E");
    assert_eq!(light.hex(ColorToken::Bad), "#B54A55");
    assert_eq!(light.hex(ColorToken::AddBackground), "#E4EFDA");
    assert_eq!(light.hex(ColorToken::DeleteBackground), "#F3E0DD");
}

#[test]
fn canvas_glyphs_and_embedded_font_geometry_are_fixed() {
    assert_eq!((CANVAS_COLUMNS, CANVAS_ROWS), (120, 40));
    assert_eq!((PNG_WIDTH, PNG_HEIGHT), (1200, 840));
    assert_eq!(ACCENT_USE_BUDGET, 3);
    assert_eq!(
        allowed_glyphs(),
        "● ○ ✓ ✗ › → ↑ ↓ ─ │ ╭ ╮ ╰ ╯ █ ░ · ⎿ ▸ ▾ …"
    );
    assert!(validate_glyphs("● Running › Review · 2 files →").is_ok());
    assert!(validate_glyphs("No decorative emoji ✦").is_err());
    assert!(validate_glyphs("No unsupported unicode λ").is_err());
}

#[cfg(feature = "png")]
#[test]
fn embedded_fonts_cover_the_closed_glyph_policy() {
    mockup_harness_v3::chrome::tokens::verify_embedded_font_coverage()
        .expect("the embedded DejaVu pair and approved deterministic fallback must cover every allowed glyph");
    assert_eq!(
        mockup_harness_v3::chrome::tokens::glyph_rendering('⎿'),
        Some(mockup_harness_v3::chrome::tokens::GlyphRendering::SyntheticElbow)
    );
}

#[test]
fn four_surface_layouts_are_bounded_and_have_one_arrangement() {
    for surface in [
        Surface::ConversationShell,
        Surface::ActionDock,
        Surface::ContextualInspector,
        Surface::TemporaryDetailView,
    ] {
        let layout = SurfaceLayout::for_surface(surface);
        layout
            .validate()
            .expect("each canonical arrangement must stay within 120x40");
        assert_eq!(layout.surface(), surface);
        assert_eq!(layout.canvas().width, CANVAS_COLUMNS);
        assert_eq!(layout.canvas().height, CANVAS_ROWS);
        assert_eq!(layout.statusline().height, 1);
        assert_eq!(layout.statusline().y, CANVAS_ROWS - 1);
        assert!(!layout.has_floating_modal());
    }

    let conversation = SurfaceLayout::for_surface(Surface::ConversationShell);
    assert!(conversation.composer().is_some());
    assert!(conversation.action_dock().is_none());
    assert!(conversation.inspector().is_none());

    let action_dock = SurfaceLayout::for_surface(Surface::ActionDock);
    assert!(action_dock.composer().is_none());
    assert!(action_dock.action_dock().is_some());
    assert!(action_dock.action_dock().unwrap().bottom() < CANVAS_ROWS - 1);

    let inspector = SurfaceLayout::for_surface(Surface::ContextualInspector);
    assert!(inspector.composer().is_some());
    assert!(inspector.inspector().is_some());

    let detail = SurfaceLayout::for_surface(Surface::TemporaryDetailView);
    assert!(detail.detail().is_some());
    assert!(detail.composer().is_none());
}

#[test]
fn statusline_and_surface_declaration_are_centralized() {
    let statusline = StatusLine::new("~/code", "main", "Claude Opus 4.8")
        .expect("a human-readable model name and local path are valid");
    assert_eq!(statusline.text(), "~/code · main · Claude Opus 4.8");
    assert!(statusline.text().contains('·'));
    assert!(!statusline.text().contains("ctx"));
    assert!(!statusline.text().contains('$'));
    assert!(!statusline.text().contains("idle"));
    assert_eq!(
        statusline.style().background,
        Some(ColorToken::Canvas),
        "the statusline must not create a separate filled strip"
    );

    assert!(StatusLine::new("~/code", "main", "anthropic/claude-opus-4-8").is_err());
    assert!(StatusLine::new("~/code", "main", "Claude Opus 4.8 · ctx 6%").is_err());

    let composition =
        ScreenComposition::new("home", Surface::ConversationShell, statusline.clone());
    composition
        .validate()
        .expect("a composition must declare one valid surface arrangement");
    assert_eq!(composition.surface(), Surface::ConversationShell);
    assert_eq!(composition.statusline(), &statusline);
}

#[test]
fn shared_components_encode_focus_local_keys_and_primary_actions() {
    let activity = ActivityRow::new(
        "Check changed files",
        ActivityState::Running,
        "2 of 4 files checked",
    )
    .with_elapsed("12s")
    .with_interrupt("Stop");
    assert_eq!(activity.anatomy().primary_action_count, 0);
    assert!(activity.anatomy().has_local_keys);
    assert!(activity
        .lines()
        .iter()
        .any(|line| line.text.contains("Stop")));

    let disclosure = DisclosureBlock::new("Why this changed", DisclosureState::Collapsed)
        .with_expanded_detail("A concise user-facing rationale.");
    assert!(disclosure
        .lines()
        .iter()
        .any(|line| line.text.contains("Expand")));

    let selection = SelectionDock::new(
        "Choose a model",
        vec![
            SelectionRow::new("Claude Opus 4.8", "Careful work on complex changes"),
            SelectionRow::new("GPT-5.2 Codex", "Fast help for focused edits"),
        ],
        1,
        "Changes the model for this conversation.",
        "Apply model",
    )
    .expect("a selected row and one primary action are required");
    assert_eq!(selection.anatomy().primary_action_count, 1);
    assert_eq!(selection.anatomy().accent_uses, 1);
    assert!(selection
        .lines()
        .iter()
        .any(|line| line.text.starts_with('›')));

    let decision = DecisionDock::new(
        "Allow this command?",
        "It will check the project files.",
        "Allow once",
    )
    .with_secondary("Deny")
    .with_secondary("Cancel")
    .validate()
    .expect("a decision supports at most two secondary actions");
    assert_eq!(decision.anatomy().primary_action_count, 1);
    assert!(decision.anatomy().has_local_keys);

    let connection = ConnectionRow::new("Project search", ConnectionState::NeedsSignIn, "Sign in");
    assert_eq!(connection.anatomy().primary_action_count, 1);

    let recovery = RecoveryBlock::new(
        "Connection paused",
        "Your draft and completed work are still here.",
        "Try again",
    )
    .with_evidence("Show connection details");
    assert_eq!(recovery.anatomy().primary_action_count, 1);

    let mission = MissionCard::new(
        "Prepare the release notes",
        "Reviewing changes",
        3,
        2,
        Some("One item needs attention"),
        "View progress",
    );
    assert_eq!(mission.anatomy().primary_action_count, 1);
}

#[test]
fn inspector_detail_diff_and_review_have_reversible_focused_anatomy() {
    let inspector = Inspector::new(
        "Files",
        "chat-turn",
        vec![InspectorTab::new("Files"), InspectorTab::new("Diagnostics")],
        0,
    )
    .expect("an inspector needs a selected contextual tab");
    assert_eq!(inspector.anatomy().primary_action_count, 1);
    assert!(inspector
        .lines()
        .iter()
        .any(|line| line.text.contains("Return to conversation")));

    let detail = mockup_harness_v3::chrome::components::DetailView::new(
        "Review changes",
        "chat-turn",
        vec!["One finding needs attention."],
    );
    assert_eq!(detail.anatomy().primary_action_count, 1);

    let diff = DiffBlock::new(
        "src/lib.rs",
        vec![
            (DiffKind::Added, "Add the shared surface layout."),
            (DiffKind::Removed, "Remove the old detached panel."),
        ],
    );
    assert_eq!(diff.anatomy().primary_action_count, 0);
    assert!(diff.lines().iter().any(|line| line.text.starts_with('+')));
    assert!(diff.lines().iter().any(|line| line.text.starts_with('-')));

    let review = ReviewBlock::new(
        "Ready for review",
        "The selected change is safe to inspect.",
        "Open evidence",
    );
    assert_eq!(review.anatomy().primary_action_count, 1);
    assert!(review.anatomy().accent_uses <= ACCENT_USE_BUDGET);
}
