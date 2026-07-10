use mockup_harness_v3::{
    canonical_inventory, flow_manifests, validate_flow_manifests, FlowManifest, TransitionKind,
};

#[test]
fn six_named_manifests_have_the_exact_stable_sequences() {
    let expected: &[(&str, &[&str])] = &[
        (
            "daily-work",
            &[
                "home",
                "chat-turn",
                "streaming",
                "tool-running",
                "tool-success",
                "rich-response",
            ],
        ),
        (
            "input-and-approval",
            &[
                "composer-multiline",
                "file-mention",
                "permission-shell",
                "diff-edit",
                "diff-viewer-staging",
                "chat-turn",
            ],
        ),
        (
            "work-configuration",
            &[
                "onboarding",
                "model-picker",
                "agent-picker",
                "mode-switch",
                "session-picker",
                "timeline",
            ],
        ),
        (
            "provider-and-mcp-connection",
            &[
                "login",
                "connect-provider",
                "connect-method",
                "connect-oauth",
                "connect-connected",
                "mcp",
            ],
        ),
        (
            "recovery",
            &[
                "turn-error",
                "error-network",
                "rate-limit",
                "chat-turn",
                "checkpoint-undo",
            ],
        ),
        (
            "mission",
            &[
                "workflow-launch-blocked",
                "workflow-launch",
                "workflow-monitor",
                "workflow-monitor-failure",
                "reviewer-output",
                "reviewer-approved",
            ],
        ),
    ];

    let flows = flow_manifests();
    assert_eq!(flows.len(), 6);
    assert_eq!(
        flows.iter().map(|flow| flow.name).collect::<Vec<_>>(),
        [
            "daily-work",
            "input-and-approval",
            "work-configuration",
            "provider-and-mcp-connection",
            "recovery",
            "mission",
        ]
    );
    assert_eq!(flows.iter().map(|flow| flow.steps.len()).sum::<usize>(), 35);

    for ((name, expected_ids), flow) in expected.iter().zip(flows) {
        assert_eq!(flow.name, *name);
        assert_eq!(
            flow.steps
                .iter()
                .map(|step| step.screen_id)
                .collect::<Vec<_>>(),
            *expected_ids
        );
        assert!(!flow.steps.is_empty());
        for step in &flow.steps {
            assert!(!step.user_action.is_empty());
            assert!(matches!(
                step.transition_kind,
                TransitionKind::Direct
                    | TransitionKind::Automatic
                    | TransitionKind::Summarized
                    | TransitionKind::IllustrativeDetour
            ));
            assert!(!step.transition_path.is_empty());
            assert!(!step.expected_next_frame.is_empty());
            assert!(!step.origin.is_empty());
            assert!(!step.success_return.is_empty());
            assert!(!step.cancel_return.is_empty());
            assert!(!step.acceptance_checks.is_empty());
        }
    }
}

#[test]
fn manifests_are_fail_closed_for_missing_duplicate_and_unknown_definitions() {
    let inventory = canonical_inventory();

    let missing = flow_manifests()[..5].to_vec();
    assert!(validate_flow_manifests(&missing, inventory)
        .expect_err("a missing flow manifest must fail")
        .contains("missing flow manifest: mission"));

    let mut duplicate = flow_manifests().to_vec();
    duplicate.push(FlowManifest {
        name: "daily-work",
        steps: duplicate[0].steps.clone(),
    });
    assert!(validate_flow_manifests(&duplicate, inventory)
        .expect_err("a duplicate flow manifest must fail")
        .contains("duplicate flow manifest: daily-work"));

    let mut reordered = flow_manifests().to_vec();
    reordered.swap(0, 1);
    assert!(validate_flow_manifests(&reordered, inventory)
        .expect_err("reordered manifests must fail")
        .contains("flow manifest order mismatch"));

    let mut unknown_screen = flow_manifests().to_vec();
    unknown_screen[0].steps[0].screen_id = "not-a-canonical-state";
    assert!(validate_flow_manifests(&unknown_screen, inventory)
        .expect_err("an unresolved screen reference must fail")
        .contains("unknown canonical screen: not-a-canonical-state"));
}
