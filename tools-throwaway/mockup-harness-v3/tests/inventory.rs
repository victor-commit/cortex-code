use std::process::Command;

use mockup_harness_v3::{
    canonical_inventory, validate_inventory, Category, FlowOwner, ImplementationStatus, ScreenSpec,
    Surface,
};

fn ids_in(category: Category) -> Vec<&'static str> {
    canonical_inventory()
        .iter()
        .filter(|screen| screen.category == category)
        .map(|screen| screen.id)
        .collect()
}

#[test]
fn canonical_inventory_has_the_exact_stable_category_membership() {
    let expected: &[(Category, &[&str])] = &[
        (
            Category::Session,
            &[
                "home",
                "onboarding",
                "chat-turn",
                "chat-long-scroll",
                "streaming",
                "reasoning",
                "reasoning-collapsed",
                "rich-response",
                "tool-calls",
                "tool-running",
                "tool-success",
                "tool-success-expanded",
                "tool-failed",
                "tool-task",
                "tool-task-expanded",
                "diff-edit",
                "shell",
                "blocked-command",
                "turn-error",
                "todo-list",
                "compact",
            ],
        ),
        (
            Category::Input,
            &[
                "slash-menu",
                "agent-picker",
                "command-palette",
                "command-palette-submenu",
                "model-picker",
                "attachment-picker",
                "model-picker-filtered",
                "file-mention",
                "export-picker",
                "fork-picker",
                "queue-manager",
                "composer-multiline",
                "skills-picker",
                "session-picker",
                "subagent-picker",
                "variant-picker",
                "theme-picker",
                "permission-shell",
                "permission-file-edit",
                "permission-deny-reason",
            ],
        ),
        (
            Category::Views,
            &[
                "status",
                "timeline",
                "mcp",
                "diff-viewer",
                "diff-viewer-staging",
                "help",
                "review",
                "theme-gallery-dark",
                "theme-gallery-light",
                "sidebar-context",
                "sidebar-files",
                "sidebar-lsp",
                "sidebar-todo",
                "sidebar-subagents",
                "sidebar-mcp",
                "sidebar-queue",
            ],
        ),
        (
            Category::Auth,
            &[
                "login",
                "connect-provider",
                "connect-method",
                "connect-apikey",
                "connect-oauth",
                "connect-connected",
                "connect-disconnect",
            ],
        ),
        (
            Category::States,
            &[
                "empty",
                "error-network",
                "error-context",
                "rate-limit",
                "interrupt",
                "toasts",
                "checkpoint-undo",
                "share",
            ],
        ),
        (
            Category::Orchestration,
            &[
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
            ],
        ),
    ];

    assert_eq!(canonical_inventory().len(), 86);
    for (category, expected_ids) in expected {
        assert_eq!(ids_in(*category), *expected_ids);
    }
    for screen in canonical_inventory() {
        let expected_status = if [
            "home",
            "onboarding",
            "chat-turn",
            "chat-long-scroll",
            "streaming",
            "reasoning",
            "reasoning-collapsed",
            "tool-running",
            "tool-success",
            "tool-success-expanded",
            "tool-calls",
            "tool-task",
            "tool-task-expanded",
            "rich-response",
            "shell",
            "todo-list",
            "compact",
            "file-mention",
            "composer-multiline",
            "slash-menu",
            "command-palette",
            "command-palette-submenu",
            "attachment-picker",
            "export-picker",
            "queue-manager",
            "model-picker",
            "model-picker-filtered",
            "permission-shell",
            "permission-file-edit",
            "permission-deny-reason",
            "diff-edit",
            "diff-viewer-staging",
            "agent-picker",
            "skills-picker",
            "subagent-picker",
            "variant-picker",
            "theme-picker",
            "mode-switch",
            "mode-switch-refused",
            "session-picker",
            "timeline",
            "fork-picker",
            "login",
            "connect-provider",
            "connect-method",
            "connect-apikey",
            "connect-oauth",
            "connect-connected",
            "connect-disconnect",
            "mcp",
            "turn-error",
            "error-network",
            "rate-limit",
            "checkpoint-undo",
            "tool-failed",
            "blocked-command",
            "workflow-launch",
            "workflow-launch-blocked",
            "workflow-monitor",
            "workflow-monitor-failure",
            "reviewer-output",
            "reviewer-approved",
            "sidebar-context",
            "sidebar-files",
            "sidebar-lsp",
            "sidebar-todo",
            "sidebar-subagents",
            "sidebar-mcp",
            "sidebar-queue",
            "help",
            "review",
            "diff-viewer",
            "share",
            "status",
            "theme-gallery-dark",
            "theme-gallery-light",
            "empty",
        ]
        .contains(&screen.id)
        {
            ImplementationStatus::Implemented
        } else {
            ImplementationStatus::Unimplemented
        };
        assert_eq!(
            screen.status, expected_status,
            "{} must report its milestone implementation status",
            screen.id
        );
    }
    assert!(canonical_inventory().iter().all(|screen| matches!(
        screen.surface,
        Surface::ConversationShell
            | Surface::ActionDock
            | Surface::ContextualInspector
            | Surface::TemporaryDetailView
    )));
    assert_eq!(
        canonical_inventory()
            .iter()
            .filter(|screen| screen.flow == FlowOwner::DailyWork)
            .count(),
        24
    );
    assert_eq!(
        canonical_inventory()
            .iter()
            .filter(|screen| screen.flow == FlowOwner::InputAndApproval)
            .count(),
        15
    );
    assert_eq!(
        canonical_inventory()
            .iter()
            .filter(|screen| screen.flow == FlowOwner::WorkConfiguration)
            .count(),
        17
    );
    assert_eq!(
        canonical_inventory()
            .iter()
            .filter(|screen| screen.flow == FlowOwner::ProviderAndMcpConnection)
            .count(),
        9
    );
    assert_eq!(
        canonical_inventory()
            .iter()
            .filter(|screen| screen.flow == FlowOwner::Recovery)
            .count(),
        9
    );
    assert_eq!(
        canonical_inventory()
            .iter()
            .filter(|screen| screen.flow == FlowOwner::Mission)
            .count(),
        12
    );
}

#[test]
fn list_reports_one_machine_parseable_unimplemented_status_per_canonical_id() {
    let output = Command::new(env!("CARGO_BIN_EXE_mockup-harness-v3"))
        .arg("--list")
        .output()
        .expect("the inventory CLI must run");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());

    let stdout = String::from_utf8(output.stdout).expect("list output must be UTF-8");
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 86);

    for (line, screen) in lines.iter().zip(canonical_inventory()) {
        assert_eq!(
            line.split('\t').collect::<Vec<_>>(),
            vec![
                screen.category.as_str(),
                screen.id,
                screen.status.as_str(),
                screen.flow.as_str(),
                screen.surface.as_str(),
            ]
        );
    }
}

#[test]
fn duplicate_missing_extra_and_multiply_owned_ids_fail_validation() {
    let mut duplicate = canonical_inventory().to_vec();
    duplicate[2].id = "home";
    assert!(validate_inventory(&duplicate)
        .expect_err("a duplicate canonical ID must fail")
        .contains("duplicate canonical ID: home"));

    let missing = canonical_inventory()[..85].to_vec();
    assert!(validate_inventory(&missing)
        .expect_err("a missing canonical ID must fail")
        .contains("missing canonical ID"));

    let mut extra = canonical_inventory().to_vec();
    extra.push(ScreenSpec {
        id: "outside-the-contract",
        category: Category::Session,
        flow: FlowOwner::DailyWork,
        surface: Surface::ConversationShell,
        status: ImplementationStatus::Unimplemented,
    });
    assert!(validate_inventory(&extra)
        .expect_err("an extra canonical ID must fail")
        .contains("extra canonical ID: outside-the-contract"));

    let mut multiply_owned = canonical_inventory().to_vec();
    multiply_owned[1].id = "home";
    assert!(validate_inventory(&multiply_owned)
        .expect_err("a state with two flow owners must fail")
        .contains("multiple flow owners: home"));

    let mut wrong_flow = canonical_inventory().to_vec();
    wrong_flow[0].flow = FlowOwner::Recovery;
    assert!(validate_inventory(&wrong_flow)
        .expect_err("an ID assigned to the wrong journey must fail")
        .contains("flow ownership mapping mismatch for home"));

    let mut wrong_surface = canonical_inventory().to_vec();
    wrong_surface[0].surface = Surface::ActionDock;
    assert!(validate_inventory(&wrong_surface)
        .expect_err("an ID assigned to the wrong surface must fail")
        .contains("surface mapping mismatch for home"));
}
