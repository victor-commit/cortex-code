use mockup_harness_v3::{
    audit::audit_screen, flow_manifests, scenario::replay_case, FlowManifest, TransitionKind,
};

fn provider_and_mcp() -> &'static FlowManifest {
    flow_manifests()
        .iter()
        .find(|flow| flow.name == "provider-and-mcp-connection")
        .expect("the provider-and-mcp storyboard must be public")
}

#[test]
fn login_offers_one_provider_path_without_mcp_adjacency() {
    let audit = audit_screen("login", "gate.provider-and-mcp.login")
        .expect("login must render typed evidence");
    audit.validate().expect("login audit must stay bijective");

    let text = audit.render_txt();
    assert!(text.contains("Connect a model provider"));
    assert!(text.contains("Code needs a provider before it can use the models in this project."));
    assert!(text.contains("› [enter] Choose a provider"));
    assert!(text.contains("[esc] Cancel"));
    assert!(
        !text.to_lowercase().contains("mcp"),
        "login is the provider entry and must not present MCP as a neighbor"
    );
    assert!(!text.contains("transport"));
    assert!(!text.contains("endpoint"));
    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "primary-action")
            .count(),
        1
    );
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.login.choose-provider"
            && transition.to_screen_id == "connect-provider"
            && transition.kind == "direct"
    }));
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.login.cancel"
            && transition.to_screen_id == "model-picker"
            && transition.kind == "cancel"
    }));
}

#[test]
fn connect_provider_lists_human_named_states_and_preserves_origin() {
    let audit = audit_screen(
        "connect-provider",
        "gate.provider-and-mcp.connect-provider",
    )
    .expect("provider catalogue must render");
    audit
        .validate()
        .expect("provider catalogue audit must remain bijective");

    let text = audit.render_txt();
    assert!(text.contains("Provider connection · blocked model action"));
    assert!(text.contains("GPT-5.2 Codex is waiting for a provider connection."));
    assert!(text.contains("OpenRouter"));
    assert!(text.contains("Anthropic"));
    assert!(text.contains("OpenAI"));
    assert!(text.contains("Google"));
    assert!(text.contains("Connected · in use"));
    assert!(text.contains("Connected"));
    assert!(text.contains("Not connected"));
    assert!(text.contains("› OpenAI"));
    assert!(text.contains("› [enter] Connect OpenAI"));
    assert!(text.contains("[esc] Back"));
    assert!(!text.contains("openai-provider-id"));
    assert!(!text.contains("oauth2"));
    assert!(!text.to_lowercase().contains("mcp server"));
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.connect-provider.connect"
            && transition.to_screen_id == "connect-method"
            && transition.kind == "direct"
    }));
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.connect-provider.back"
            && transition.to_screen_id == "login"
            && transition.kind == "cancel"
    }));
}

#[test]
fn connect_method_shows_only_applicable_sign_in_choices_for_openai() {
    let audit = audit_screen("connect-method", "gate.provider-and-mcp.connect-method")
        .expect("method choice must render");
    audit
        .validate()
        .expect("method choice audit must remain bijective");

    let text = audit.render_txt();
    assert!(text.contains("Sign in to OpenAI"));
    assert!(text.contains("› Browser sign-in"));
    assert!(text.contains("Use an API key"));
    assert!(text.contains("authorize in your browser"));
    assert!(text.contains("paste a key stored only on this machine"));
    assert!(text.contains("› [enter] Continue with browser sign-in"));
    assert!(text.contains("[esc] Back"));
    assert!(!text.contains("device_code"));
    assert!(!text.contains("PKCE"));
    assert!(!text.contains("transport"));
    assert_eq!(text.matches('›').count(), 2); // selected row + dock primary
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.connect-method.oauth"
            && transition.to_screen_id == "connect-oauth"
            && transition.kind == "direct"
    }));
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.connect-method.back"
            && transition.to_screen_id == "connect-provider"
            && transition.kind == "cancel"
    }));
}

#[test]
fn connect_oauth_is_live_and_cancelable_without_connecting() {
    let audit = audit_screen("connect-oauth", "gate.provider-and-mcp.connect-oauth")
        .expect("oauth wait must render");
    audit
        .validate()
        .expect("oauth audit must remain bijective");

    let text = audit.render_txt();
    assert!(text.contains("Authorize OpenAI"));
    assert!(text.contains("Open this sign-in page in your browser"));
    assert!(text.contains("Enter this one-time code"));
    assert!(text.contains("WDJB-MJHT"));
    assert!(text.contains("● Waiting for authorization"));
    // Live wait has zero primary; secondary reopen + Cancel only.
    assert!(!text.contains("› [enter] I've authorized"));
    assert!(
        text.contains("Reopen browser") || text.contains("secondary"),
        "reopen stays secondary during wait"
    );
    assert!(text.contains("[esc] Cancel sign-in"));
    assert!(!text.contains("device_code"));
    assert!(!text.contains("client_secret"));
    assert!(
        !audit
            .events
            .iter()
            .any(|event| event.identity.family == "primary-action"),
        "waiting has zero primary-action events"
    );
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.connect-oauth.finish"
            && transition.to_screen_id == "connect-connected"
            && (transition.kind == "direct" || transition.kind == "automatic")
    }));
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.connect-oauth.cancel"
            && transition.to_screen_id == "connect-method"
            && transition.kind == "cancel"
    }));
}

#[test]
fn connect_connected_returns_to_blocked_model_origin_not_mcp() {
    let audit = audit_screen(
        "connect-connected",
        "gate.provider-and-mcp.connect-connected",
    )
    .expect("connected confirmation must render");
    audit
        .validate()
        .expect("connected audit must remain bijective");

    let text = audit.render_txt();
    assert!(text.contains("Provider connection · blocked model action"));
    assert!(text.contains("OpenAI is connected"));
    assert!(text.contains("✓ Connected"));
    assert!(text.contains("GPT-5.2 Codex can use this provider now."));
    assert!(text.contains("GPT-5.2 Codex"));
    assert!(
        text.contains("› [enter] Continue") || text.contains("› [enter] Return to model choices"),
        "primary must be Continue or the historic return label"
    );
    assert!(
        text.contains("Return to model choices") || text.contains("blocked model action"),
        "blocked-model destination must remain explicit"
    );
    assert!(
        text.contains("ordered capability branch")
            || text.contains("separate ordered branch")
            || text.contains("own ordered path"),
        "connected must name the capability branch as a separate ordered path, not invent it as this return"
    );
    assert!(
        !text.contains("next step is MCP")
            && !text.contains("Continue to MCP")
            && !text.contains("Open MCP now"),
        "provider success must not present MCP as the live primary next step"
    );
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.connect-connected.return"
            && transition.to_screen_id == "model-picker"
            && transition.kind == "direct"
    }));
    assert!(
        !audit
            .transitions
            .iter()
            .any(|transition| transition.to_screen_id == "mcp" && transition.kind == "direct"),
        "provider success must never own a direct edge into MCP"
    );
    // Named illustrative adjacency for the ordered storyboard (audit-visible, not a live direct edge).
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.storyboard.open-mcp-after-provider"
            && transition.to_screen_id == "mcp"
            && (transition.kind == "illustrative" || transition.kind == "illustrative_detour")
    }));
    assert_eq!(
        audit
            .conversation
            .as_ref()
            .map(|conversation| conversation.visible_origin.as_str()),
        Some("Provider connection · blocked model action")
    );
    assert_eq!(
        audit
            .conversation
            .as_ref()
            .map(|conversation| conversation.user_task.as_str()),
        Some("Connect the model needed for this conversation.")
    );
    assert_eq!(
        audit
            .conversation
            .as_ref()
            .and_then(|conversation| conversation.paused_activity_id.as_deref()),
        Some("pending.model.gpt-5-2-codex")
    );
}

#[test]
fn mcp_is_capability_led_attention_first_and_separate_from_providers() {
    let audit = audit_screen("mcp", "gate.provider-and-mcp.mcp").expect("mcp must render");
    audit.validate().expect("mcp audit must remain bijective");

    let text = audit.render_txt();
    assert!(text.contains("MCP · conversation capabilities"));
    assert!(text.contains("Illustrative detour"));
    assert!(text.contains("separate capability branch"));
    assert!(text.contains("Fix the capabilities this conversation needs."));
    assert!(text.contains("MCP capabilities"));
    assert!(text.contains("Browser automation"));
    assert!(text.contains("Needs setup"));
    assert!(text.contains("Project docs"));
    assert!(text.contains("Sign in needed"));
    assert!(text.contains("Code search"));
    assert!(text.contains("Ready"));
    assert!(text.contains("1 ready connection collapsed"));
    assert!(
        text.contains("› [enter] Fix setup")
            || text.contains("› Fix setup")
            || text.contains("Enter to fix setup")
            || text.contains("Fix setup"),
        "primary Fix setup affordance must remain readable"
    );
    assert!(
        text.contains("Esc to return to conversation")
            || text.contains("[esc] Return to conversation")
            || text.contains("Return to conversation"),
        "return-to-conversation guidance must remain readable"
    );
    assert!(
        !text.contains("Provider connection · blocked model action"),
        "MCP detour must not pretend the blocked-model provider origin still owns this frame"
    );
    assert!(
        !text.contains("Connect the model needed for this conversation."),
        "MCP must not silently keep the provider user task as same-turn continuity"
    );
    assert!(!text.contains("stdio"));
    assert!(!text.contains("http"));
    assert!(!text.contains("transport"));
    assert!(!text.contains("endpoint"));
    assert!(!text.contains("timeout"));
    assert!(!text.contains("OpenRouter"));
    assert!(!text.contains("Anthropic"));
    assert!(!text.contains("provider catalogue"));
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.mcp.return"
            && transition.to_screen_id == "chat-turn"
            && transition.kind == "direct"
    }));
    assert!(
        !audit
            .transitions
            .iter()
            .any(|transition| transition.from_screen_id == "connect-connected"),
        "MCP must not inherit the provider success edge"
    );
    assert_eq!(
        audit
            .conversation
            .as_ref()
            .map(|conversation| conversation.visible_origin.as_str()),
        Some("MCP · conversation capabilities")
    );
    assert_eq!(
        audit
            .conversation
            .as_ref()
            .map(|conversation| conversation.user_task.as_str()),
        Some("Fix the capabilities this conversation needs.")
    );
    assert_eq!(
        audit
            .conversation
            .as_ref()
            .map(|conversation| conversation.session_id.as_str()),
        Some("session.mcp-capabilities.1")
    );
}

#[test]
fn flow4_ordered_mcp_detour_rewrites_identity_openly() {
    let flow = provider_and_mcp();
    let connected = flow
        .steps
        .iter()
        .find(|step| step.screen_id == "connect-connected")
        .expect("connected step");
    let mcp = flow
        .steps
        .iter()
        .find(|step| step.screen_id == "mcp")
        .expect("mcp step");

    assert_eq!(connected.transition_kind, TransitionKind::IllustrativeDetour);
    assert_eq!(connected.expected_next_frame, "mcp");
    assert_eq!(
        connected.visible_origin,
        "Provider connection · blocked model action"
    );
    assert_eq!(
        connected.user_task,
        "Connect the model needed for this conversation."
    );
    assert_eq!(mcp.visible_origin, "MCP · conversation capabilities");
    assert_eq!(
        mcp.user_task,
        "Fix the capabilities this conversation needs."
    );
    assert_ne!(connected.conversation_id, mcp.conversation_id);
    assert_ne!(connected.session_id, mcp.session_id);
    assert_ne!(connected.user_task, mcp.user_task);
    assert_ne!(connected.visible_origin, mcp.visible_origin);

    let connected_audit = audit_screen(connected.screen_id, connected.fixture_id)
        .expect("connected must render");
    let mcp_audit = audit_screen(mcp.screen_id, mcp.fixture_id).expect("mcp must render");
    let connected_text = connected_audit.render_txt();
    let mcp_text = mcp_audit.render_txt();

    assert!(connected_text.contains("Provider connection · blocked model action"));
    assert!(connected_text.contains("GPT-5.2 Codex"));
    assert!(mcp_text.contains("Illustrative detour"));
    assert!(mcp_text.contains("MCP · conversation capabilities"));
    assert!(mcp_text.contains("Fix the capabilities this conversation needs."));

    let bridge = replay_case("provider-to-mcp-bridge", 1).expect("ordered detour bridge must replay");
    assert_eq!(bridge.before.screen_id, "connect-connected");
    assert_eq!(bridge.after.screen_id, "mcp");
    assert_eq!(
        bridge.actions[0].action_id,
        "action.storyboard.open-mcp-after-provider"
    );
    assert!(
        bridge
            .transitions
            .iter()
            .any(|transition| transition.kind == "illustrative_detour"
                && transition.to_screen_id == "mcp"),
        "ordered detour bridge must expose illustrative_detour into mcp"
    );
    assert_eq!(
        bridge.before.visible_origin,
        "Provider connection · blocked model action"
    );
    assert_eq!(
        bridge.before.user_task,
        "Connect the model needed for this conversation."
    );
    assert_eq!(
        bridge.after.visible_origin,
        "MCP · conversation capabilities"
    );
    assert_eq!(
        bridge.after.user_task,
        "Fix the capabilities this conversation needs."
    );
    assert_eq!(bridge.after.session_id, "session.mcp-capabilities.1");
    assert_eq!(
        bridge.after.conversation_id,
        "conversation.mcp-capabilities"
    );
}

#[test]
fn flow4_storyboard_declares_provider_and_mcp_as_separate_branches() {
    let flow = provider_and_mcp();
    let expected = [
        ("login", "connect-provider", TransitionKind::Direct),
        ("connect-provider", "connect-method", TransitionKind::Direct),
        ("connect-method", "connect-oauth", TransitionKind::Direct),
        ("connect-oauth", "connect-connected", TransitionKind::Direct),
        (
            "connect-connected",
            "mcp",
            TransitionKind::IllustrativeDetour,
        ),
        ("mcp", "chat-turn", TransitionKind::Direct),
    ];

    assert_eq!(flow.steps.len(), expected.len());
    for (step, (screen_id, next_frame, kind)) in flow.steps.iter().zip(expected) {
        assert_eq!(step.screen_id, screen_id);
        assert_eq!(step.expected_next_frame, next_frame);
        assert_eq!(step.transition_kind, kind);
        assert_eq!(
            step.fixture_id,
            format!("gate.provider-and-mcp.{screen_id}")
        );
        assert_eq!(
            step.visible_origin,
            if screen_id == "mcp" {
                "MCP · conversation capabilities"
            } else {
                "Provider connection · blocked model action"
            }
        );
        assert!(step.preservation_fields.contains(&"visible_origin"));
        assert!(step.preservation_fields.contains(&"return_policy"));

        let audit = audit_screen(screen_id, step.fixture_id)
            .unwrap_or_else(|error| panic!("{screen_id} must render: {error}"));
        audit
            .validate()
            .unwrap_or_else(|error| panic!("{screen_id} must validate: {error}"));
        assert!(
            !audit.transitions.is_empty(),
            "{screen_id} must expose transitions"
        );
        assert!(
            audit
                .transitions
                .iter()
                .all(|transition| transition.from_screen_id == screen_id),
            "{screen_id} transitions must preserve their origin"
        );
        if kind == TransitionKind::Direct {
            assert!(
                audit.transitions.iter().any(|transition| {
                    transition.to_screen_id == next_frame && transition.kind == "direct"
                }),
                "{screen_id} must expose the declared direct edge to {next_frame}"
            );
        } else {
            assert!(
                audit.transitions.iter().any(|transition| {
                    transition.kind == "cancel" || transition.to_screen_id == step.success_return
                }),
                "{screen_id} illustrative detour must expose its return path, not invent MCP adjacency"
            );
            assert!(
                !audit.transitions.iter().any(|transition| {
                    transition.to_screen_id == "mcp" && transition.kind == "direct"
                }),
                "connect-connected must not claim a direct provider-to-MCP edge"
            );
        }
    }
}

#[test]
fn provider_and_mcp_scenarios_preserve_origin_and_stay_separate() {
    let login_cancel = replay_case("login-cancel", 1).expect("login cancel must replay");
    assert_eq!(login_cancel.before.screen_id, "login");
    assert_eq!(login_cancel.after.screen_id, "model-picker");
    assert_eq!(login_cancel.actions[0].action_id, "action.login.cancel");
    for field in ["draft", "selection", "transcript"] {
        let hashes = login_cancel
            .preserved_field_hashes
            .get(field)
            .unwrap_or_else(|| panic!("{field} must be preserved"));
        assert_eq!(hashes.before, hashes.after, "{field} must survive cancel");
    }

    let provider_select =
        replay_case("connect-provider-select", 1).expect("provider select must replay");
    assert_eq!(provider_select.before.screen_id, "connect-provider");
    assert_eq!(provider_select.after.screen_id, "connect-method");
    assert_eq!(
        provider_select.actions[0].action_id,
        "action.connect-provider.connect"
    );

    let method_back = replay_case("connect-method-back", 1).expect("method back must replay");
    assert_eq!(method_back.before.screen_id, "connect-method");
    assert_eq!(method_back.after.screen_id, "connect-provider");
    assert_eq!(method_back.actions[0].action_id, "action.connect-method.back");

    let connected_return =
        replay_case("connect-connected-return", 1).expect("connected return must replay");
    assert_eq!(connected_return.before.screen_id, "connect-connected");
    assert_eq!(connected_return.after.screen_id, "model-picker");
    assert_eq!(
        connected_return.actions[0].action_id,
        "action.connect-connected.return"
    );
    assert_ne!(
        connected_return.after.screen_id, "mcp",
        "provider success returns to the blocked model origin, never MCP"
    );

    let mcp_return = replay_case("mcp-return", 1).expect("mcp return must replay");
    assert_eq!(mcp_return.before.screen_id, "mcp");
    assert_eq!(mcp_return.after.screen_id, "chat-turn");
    assert_eq!(mcp_return.actions[0].action_id, "action.mcp.return");
    for field in ["draft", "transcript"] {
        let hashes = mcp_return
            .preserved_field_hashes
            .get(field)
            .unwrap_or_else(|| panic!("{field} must be preserved on MCP return"));
        assert_eq!(hashes.before, hashes.after);
    }
}
