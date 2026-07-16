//! Phase A redesign checks for denser conversation + composer language
//! (VAL-V3-CROSS-020, N-11..N-13 / F-8..F-10).

use mockup_harness_v3::audit::audit_screen;
use mockup_harness_v3::chrome::components::{
    contains_bracket_key_atom, is_journey_breadcrumb_title, subject_title_for_origin,
};

const PHASE_A_IDS: &[&str] = &[
    "home",
    "onboarding",
    "chat-turn",
    "chat-long-scroll",
    "streaming",
    "reasoning",
    "reasoning-collapsed",
    "rich-response",
    "tool-running",
    "tool-calls",
    "tool-success",
    "tool-success-expanded",
    "tool-failed",
    "tool-task",
    "tool-task-expanded",
    "todo-list",
    "compact",
    "shell",
    "blocked-command",
    "diff-edit",
    "composer-multiline",
    "file-mention",
    "slash-menu",
    "permission-shell",
    "permission-file-edit",
    "permission-deny-reason",
    "model-picker",
    "agent-picker",
    "session-picker",
];

fn fixture_for(screen_id: &str) -> &'static str {
    match screen_id {
        "onboarding" => "flow3-onboarding",
        "chat-turn" => "conversation-completed",
        "permission-shell" => "flow2-permission-shell-short",
        "permission-file-edit" => "flow2.input-and-approval.stable",
        "permission-deny-reason" => "flow2.input-and-approval.stable",
        "diff-edit" => "flow2.input-and-approval.stable",
        "composer-multiline" | "file-mention" | "slash-menu" => "flow2.input-and-approval.stable",
        "blocked-command" | "tool-failed" => "public-audit",
        "model-picker" => "flow3-model-picker",
        "agent-picker" => "flow3-agent-picker",
        "session-picker" => "flow3-session-picker",
        _ => "public-audit",
    }
}

#[test]
fn subject_title_mapping_strips_journey_crumbs() {
    assert_eq!(
        subject_title_for_origin("Daily work · current conversation"),
        "Current conversation"
    );
    assert_eq!(
        subject_title_for_origin("Input and approval · paused conversation"),
        "Paused conversation"
    );
    assert_eq!(
        subject_title_for_origin("Mission · current conversation"),
        "Mission"
    );
    assert!(!is_journey_breadcrumb_title(subject_title_for_origin(
        "Daily work · current conversation"
    )));
}

#[test]
fn phase_a_screens_paint_subject_titles_not_journey_crumbs() {
    for screen_id in PHASE_A_IDS {
        let fixture = fixture_for(screen_id);
        let audit = audit_screen(screen_id, fixture)
            .unwrap_or_else(|err| panic!("{screen_id}/{fixture} must render: {err}"));
        let text = audit.render_txt();
        for line in text.lines().take(6) {
            assert!(
                !is_journey_breadcrumb_title(line.trim()),
                "{screen_id} paints journey crumb title: {line}"
            );
        }
        assert!(
            !text.contains("Daily work ·"),
            "{screen_id} still paints Daily work journey crumb"
        );
        assert!(
            !text.contains("Input and approval ·"),
            "{screen_id} still paints Input and approval journey crumb"
        );
        assert!(
            !text.contains("Mission · current conversation"),
            "{screen_id} still paints Mission journey crumb"
        );
        assert!(
            !text.contains("Work configuration ·"),
            "{screen_id} still paints Work configuration journey crumb"
        );
        assert!(
            !text.contains("Recovery ·"),
            "{screen_id} still paints Recovery journey crumb"
        );
    }
}

#[test]
fn phase_a_screens_have_zero_bracket_key_atoms() {
    for screen_id in PHASE_A_IDS {
        let fixture = fixture_for(screen_id);
        let audit = audit_screen(screen_id, fixture)
            .unwrap_or_else(|err| panic!("{screen_id}/{fixture} must render: {err}"));
        let text = audit.render_txt();
        assert!(
            !contains_bracket_key_atom(&text),
            "{screen_id} still emits bracket key atoms:\n{}",
            text.lines()
                .filter(|line| contains_bracket_key_atom(line))
                .take(8)
                .collect::<Vec<_>>()
                .join("\n")
        );
        assert!(
            !text.to_ascii_lowercase().contains("cursor at character"),
            "{screen_id} leaks cursor debug meta"
        );
        assert!(
            !text.to_ascii_lowercase().contains("wraps visually"),
            "{screen_id} leaks wrap debug meta"
        );
    }
}

#[test]
fn home_shows_code_brand_and_clean_composer_jewel() {
    let audit = audit_screen("home", "public-audit").expect("home");
    let text = audit.render_txt();
    assert!(text.contains('█'), "home keeps Code ASCII brand");
    assert!(text.contains("Recent work"));
    assert!(text.contains("Enter to resume") || text.contains("Ask Code"));
    assert!(!text.contains("[enter]"), "home must not paint bracket chords");
    assert!(!text.contains("Daily work ·"));
    assert!(text.contains("╭") && text.contains("╰"), "composer jewel border");
}

#[test]
fn chat_turn_and_composer_multiline_use_dense_language() {
    let chat = audit_screen("chat-turn", "conversation-completed").expect("chat-turn");
    let chat_text = chat.render_txt();
    assert!(chat_text.contains("Current conversation") || chat_text.contains("Claude"));
    assert!(!chat_text.contains("Daily work ·"));
    assert!(!contains_bracket_key_atom(&chat_text));
    assert!(
        chat_text.contains("Enter to") || chat_text.contains("› "),
        "chat-turn should keep prose action language"
    );

    let multi = audit_screen("composer-multiline", "flow2.input-and-approval.stable")
        .expect("composer-multiline");
    let multi_text = multi.render_txt();
    assert!(multi_text.contains("Draft · 3 lines"));
    assert!(!multi_text.to_ascii_lowercase().contains("cursor at character"));
    assert!(!multi_text.to_ascii_lowercase().contains("wraps visually"));
    assert!(!contains_bracket_key_atom(&multi_text));
    assert!(!multi_text.contains("Input and approval ·"));
    assert!(multi_text.contains("Paused conversation") || multi_text.contains("Check the retry"));
}

#[test]
fn permission_shell_dock_uses_prose_bylines() {
    let audit = audit_screen("permission-shell", "flow2-permission-shell-short")
        .expect("permission-shell");
    let text = audit.render_txt();
    assert!(text.contains("› Allow once") || text.contains("Allow once"));
    assert!(!text.contains("› [enter]"));
    assert!(!contains_bracket_key_atom(&text));
    assert!(
        text.contains("Esc to") || text.contains("Enter to"),
        "permission dock needs a prose byline"
    );
}

#[test]
fn chat_long_scroll_uses_prose_ctrl_u_without_brackets() {
    for fixture in ["public-audit", "session.history.earlier", "session.history.latest"] {
        let audit = audit_screen("chat-long-scroll", fixture)
            .unwrap_or_else(|err| panic!("chat-long-scroll/{fixture} must render: {err}"));
        let text = audit.render_txt();
        assert!(
            !text.contains("[ctrl+u]"),
            "chat-long-scroll/{fixture} still paints [ctrl+u] brackets"
        );
        assert!(
            !contains_bracket_key_atom(&text),
            "chat-long-scroll/{fixture} emits bracket key atoms"
        );
        assert!(
            text.contains("ctrl+u to jump earlier")
                || text.contains("ctrl+u to jump to earlier")
                || text.contains("Ctrl+U to jump earlier"),
            "chat-long-scroll/{fixture} must paint prose ctrl+u byline (VAL-V3-DS-008 lowercase); got:\n{}",
            text.lines().take(8).collect::<Vec<_>>().join("\n")
        );
        assert!(
            !text.contains("Ctrl+U") && !text.contains("Ctrl+"),
            "chat-long-scroll/{fixture} must use lowercase chord form (VAL-V3-DS-008)"
        );
        assert!(
            !text.contains("Daily work ·"),
            "chat-long-scroll/{fixture} must keep subject title, not journey crumb"
        );
    }
}

#[test]
fn public_permission_screens_paint_subject_titles() {
    // Gallery uses public-audit / canonical.<id> for permission IDs not bound
    // to a first-three flow step fixture. Subject chrome must still paint.
    for (screen_id, fixture) in [
        ("permission-file-edit", "public-audit"),
        ("permission-file-edit", "canonical.permission-file-edit"),
        ("permission-deny-reason", "public-audit"),
        ("permission-deny-reason", "canonical.permission-deny-reason"),
        ("permission-shell", "public-audit"),
    ] {
        let audit = audit_screen(screen_id, fixture)
            .unwrap_or_else(|err| panic!("{screen_id}/{fixture} must render: {err}"));
        let text = audit.render_txt();
        let top = text
            .lines()
            .take(6)
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>();
        assert!(
            top.contains(&"Paused conversation"),
            "{screen_id}/{fixture} must paint subject title 'Paused conversation'; top lines: {top:?}"
        );
        assert!(
            !is_journey_breadcrumb_title(top.first().copied().unwrap_or("")),
            "{screen_id}/{fixture} paints journey crumb as title: {top:?}"
        );
        assert!(
            !contains_bracket_key_atom(&text),
            "{screen_id}/{fixture} emits bracket key atoms"
        );
    }
}
