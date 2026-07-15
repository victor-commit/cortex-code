use mockup_harness_v3::chrome::{
    components::{
        composer_header_meta, contains_bracket_key_atom, has_enter_control, has_esc_control,
        has_primary_action_label, has_secondary_key_clause, is_journey_breadcrumb_title,
        reject_composer_debug_meta, reject_journey_breadcrumb_title, validate_component,
        ActivityRow, ActivityState, Component, ConnectionRow, ConnectionState, DecisionDock,
        DiffBlock, DiffKind, DisclosureBlock, DisclosureState, Inspector, InspectorTab,
        MissionCard, RecoveryBlock, ReviewBlock, SelectionDock, SelectionRow, ShortcutByline,
        ShortcutHint,
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
fn statusline_rejects_control_characters_and_over_width_rows() {
    for (path, branch, model) in [
        ("~/code\nhidden", "main", "Claude Opus 4.8"),
        ("~/code", "main\tbranch", "Claude Opus 4.8"),
        ("~/code", "main", "\u{1b}[31mClaude Opus 4.8"),
    ] {
        assert_eq!(
            StatusLine::new(path, branch, model).expect_err("control characters must fail closed"),
            "statusline fields must not contain control characters"
        );
    }

    let exact_boundary = StatusLine::new("~/", "m", &"M".repeat(111))
        .expect("exactly one 120-column statusline row must remain valid");
    assert_eq!(exact_boundary.text().chars().count(), 120);
    assert_eq!(
        StatusLine::new("~/", "m", &"M".repeat(112))
            .expect_err("over-width statusline rows must fail closed"),
        "statusline must fit one 120-column canvas row"
    );
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
        .any(|line| line.text.contains("Esc to stop")));

    let disclosure = DisclosureBlock::new("Why this changed", DisclosureState::Collapsed)
        .with_expanded_detail("A concise user-facing rationale.");
    assert!(disclosure
        .lines()
        .iter()
        .any(|line| line.text.contains("Enter to expand")));

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

    assert_eq!(
        DecisionDock::new(
            Surface::ActionDock,
            "Allow this command?",
            "It will check the project files.",
            "",
        )
        .expect_err("an empty primary action must fail at construction"),
        "decision dock requires one primary action"
    );
    assert_eq!(
        DecisionDock::new(
            Surface::ConversationShell,
            "Allow this command?",
            "It will check the project files.",
            "Allow once",
        )
        .expect_err("a decision dock outside the bottom dock must fail at construction"),
        "decision dock must render on the action dock surface"
    );

    let decision = DecisionDock::new(
        Surface::ActionDock,
        "Allow this command?",
        "It will check the project files.",
        "Allow once",
    )
    .expect("a valid decision must require the bottom action dock")
    .with_secondary("Deny")
    .expect("one secondary action must be valid")
    .with_secondary("Cancel")
    .expect("two secondary actions must be valid");
    assert_eq!(decision.anatomy().primary_action_count, 1);
    assert!(decision.anatomy().has_local_keys);
    assert!(decision
        .render()
        .expect("a valid dock must render after mandatory validation")
        .iter()
        .any(|line| line.text.contains("Allow once")));

    assert_eq!(
        DecisionDock::new(
            Surface::ActionDock,
            "Allow this command?",
            "It will check the project files.",
            "Allow once",
        )
        .expect("a valid decision dock must construct")
        .with_secondary("Deny")
        .expect("one secondary action must be valid")
        .with_secondary("Cancel")
        .expect("two secondary actions must be valid")
        .with_secondary("Ask later")
        .expect_err("a third secondary action must fail immediately"),
        "decision dock allows at most two secondary actions"
    );

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
        .any(|line| line.text.contains("Esc to return to conversation")));

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

#[test]
fn shortcut_byline_renders_claude_style_prose_without_brackets() {
    let byline = ShortcutByline::enter_esc_choose("Apply model", "cancel")
        .expect("common dock byline must construct");
    assert_eq!(
        byline.render(),
        "Enter to apply model · Esc to cancel · ↑↓ to choose"
    );
    assert!(!contains_bracket_key_atom(&byline.render()));

    assert!(
        ShortcutHint::new("[enter]", "send").is_err(),
        "bracket key atoms must fail closed at construction"
    );
    assert!(
        ShortcutByline::new(vec![]).is_err(),
        "empty bylines must fail closed"
    );
}

#[test]
fn dual_branch_prose_or_bracket_helpers_accept_shortcut_byline_form() {
    let prose = "› Apply model · Esc to Cancel\nEnter to apply model · Esc to cancel";
    assert!(has_enter_control(prose));
    assert!(has_esc_control(prose));
    assert!(has_primary_action_label(prose, "Apply model"));
    assert!(has_secondary_key_clause(
        " c for Continue from partial reply",
        "c",
        "Continue from partial reply"
    ));

    let residual = "› [enter] Retry reply\n[c] Continue from partial reply\n[esc] Cancel";
    assert!(has_enter_control(residual));
    assert!(has_esc_control(residual));
    assert!(has_primary_action_label(residual, "Retry reply"));
    assert!(has_secondary_key_clause(
        residual,
        "c",
        "Continue from partial reply"
    ));
}

#[test]
fn shared_chrome_components_never_emit_bracket_key_atoms() {
    let activity = ActivityRow::new(
        "Check changed files",
        ActivityState::Running,
        "2 of 4 files checked",
    )
    .with_elapsed("12s")
    .with_interrupt("Stop");
    let disclosure = DisclosureBlock::new("Why this changed", DisclosureState::Collapsed)
        .with_expanded_detail("A concise user-facing rationale.");
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
    .expect("selection dock constructs");
    let decision = DecisionDock::new(
        Surface::ActionDock,
        "Allow this command?",
        "It will check the project files.",
        "Allow once",
    )
    .expect("decision dock constructs")
    .with_secondary("Deny")
    .expect("secondary constructs")
    .with_secondary("Cancel")
    .expect("cancel constructs");
    let connection = ConnectionRow::new("Project search", ConnectionState::NeedsSignIn, "Sign in");
    let recovery = RecoveryBlock::new(
        "Connection paused",
        "Your draft and completed work are still here.",
        "Try again",
    )
    .with_evidence("Show connection details");
    let mission = MissionCard::new(
        "Prepare the release notes",
        "Reviewing changes",
        3,
        2,
        Some("One item needs attention"),
        "View progress",
    );
    let inspector = Inspector::new(
        "Files",
        "chat-turn",
        vec![InspectorTab::new("Files"), InspectorTab::new("Diagnostics")],
        0,
    )
    .expect("inspector constructs");
    let detail = mockup_harness_v3::chrome::components::DetailView::new(
        "Review changes",
        "chat-turn",
        vec!["One finding needs attention."],
    );
    let review = ReviewBlock::new(
        "Ready for review",
        "The selected change is safe to inspect.",
        "Open evidence",
    )
    .with_secondary("Show details");

    let mut rendered = Vec::new();
    for component in [
        activity.lines(),
        disclosure.lines(),
        selection.lines(),
        decision
            .render()
            .expect("decision dock must render when valid"),
        connection.lines(),
        recovery.lines(),
        mission.lines(),
        inspector.lines(),
        detail.lines(),
        review.lines(),
    ] {
        rendered.extend(component);
    }

    for line in &rendered {
        assert!(
            !contains_bracket_key_atom(&line.text),
            "chrome line leaked bracket key atom: {}",
            line.text
        );
    }
    assert!(rendered.iter().any(|line| line.text.contains("Enter to")));
    assert!(rendered.iter().any(|line| line.text.contains("Esc to")));

    validate_component(&selection).expect("selection dock must remain valid anatomy");
    validate_component(&decision).expect("decision dock must remain valid anatomy");
    validate_component(&recovery).expect("recovery block must remain valid anatomy");
}

#[test]
fn journey_breadcrumb_titles_and_composer_debug_meta_are_banned() {
    for title in [
        "Daily work · current conversation",
        "Mission · current conversation",
        "Input and approval · paused conversation",
        "Work configuration · onboarding",
        "Provider and MCP · connect",
        "Recovery · paused conversation",
    ] {
        assert!(
            is_journey_breadcrumb_title(title),
            "expected breadcrumb recognition for {title}"
        );
        assert!(reject_journey_breadcrumb_title(title).is_err());
    }
    assert!(reject_journey_breadcrumb_title("Prepare the release safely").is_ok());
    assert!(reject_journey_breadcrumb_title("Current conversation").is_ok());

    assert!(reject_composer_debug_meta("cursor at character 30").is_err());
    assert!(reject_composer_debug_meta("Draft · wraps visually").is_err());
    assert!(reject_composer_debug_meta("Draft · [@] Mention").is_err());
    assert_eq!(
        composer_header_meta(3).expect("line-count meta is allowed"),
        "Draft · 3 lines"
    );
    assert_eq!(
        composer_header_meta(1).expect("singular line-count meta is allowed"),
        "Draft · 1 line"
    );
}

/// Fail-closed chrome unit fixtures for the redesign language kill-switch.
/// Full published gallery still has screen-owned brackets until phase-A
/// rewrites land; this scan proves shared chrome cannot emit them.
#[test]
fn chrome_unit_fixtures_scan_has_zero_bracket_key_atoms() {
    let fixtures = vec![
        ActivityRow::new(
            "Search project",
            ActivityState::Running,
            "128 of 400 files",
        )
        .with_elapsed("4s")
        .with_interrupt("Stop search")
        .compact()
        .lines(),
        SelectionDock::new(
            "Choose how Code should work",
            vec![
                SelectionRow::new("Careful edits", "Review before writes"),
                SelectionRow::new("Quick implementation", "Move faster on focused tasks"),
            ],
            0,
            "Applies to the next reply only.",
            "Apply work style",
        )
        .expect("fixture")
        .lines(),
        DecisionDock::new(
            Surface::ActionDock,
            "Allow this shell command?",
            "It will run once, then resume the paused edit.",
            "Allow once",
        )
        .expect("fixture")
        .with_secondary("Deny")
        .expect("fixture")
        .render()
        .expect("fixture"),
        RecoveryBlock::new(
            "Could not reach the provider",
            "Your draft and completed work are still here.",
            "Try again",
        )
        .lines(),
        MissionCard::new(
            "Prepare the release safely",
            "Running",
            1,
            2,
            None,
            "View progress",
        )
        .lines(),
        Inspector::new(
            "Files",
            "current conversation",
            vec![InspectorTab::new("Files")],
            0,
        )
        .expect("fixture")
        .lines(),
        mockup_harness_v3::chrome::components::DetailView::new(
            "Help",
            "current conversation",
            vec!["Enter · send the draft"],
        )
        .lines(),
        ReviewBlock::new("Ready for review", "One finding needs attention.", "Open evidence")
            .lines(),
    ];

    let mut violators = Vec::new();
    for lines in fixtures {
        for line in lines {
            if contains_bracket_key_atom(&line.text) {
                violators.push(line.text.clone());
            }
            assert!(
                !is_journey_breadcrumb_title(&line.text),
                "chrome fixture painted journey breadcrumb title: {}",
                line.text
            );
        }
    }
    assert!(
        violators.is_empty(),
        "chrome unit fixtures must not emit bracket key atoms: {violators:?}"
    );
}
