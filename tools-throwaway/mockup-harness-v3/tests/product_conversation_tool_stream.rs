//! Cortex conversation + tool stream rewrite
//! (VAL-V3-DS-033 and related presence of jewel / turn-status / user strip).

use mockup_harness_v3::{
    audit::audit_screen,
    chrome::components::{
        contains_bracket_key_atom, contains_global_putty_footer, Component, ToolVerb,
        ActivityRow, ActivityState, tool_elbow_line, tool_fold_summary,
    },
};

const CONVERSATION_IDS: &[&str] = &[
    "home",
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
    "tool-task",
    "tool-task-expanded",
    "todo-list",
    "compact",
    "shell",
];

fn fixture_for(screen_id: &str) -> &'static str {
    match screen_id {
        "chat-turn" => "conversation-completed",
        "tool-running" => "tool-pilot-running",
        "tool-success" | "tool-success-expanded" => "public-audit",
        "chat-long-scroll" => "public-audit",
        "shell" => "public-audit",
        _ => "public-audit",
    }
}

#[test]
fn tool_verb_helpers_conjugate_present_and_past() {
    assert_eq!(ToolVerb::Search.phrase("project code", false), "Searching project code");
    assert_eq!(ToolVerb::Search.phrase("project code", true), "Searched project code");
    assert_eq!(ToolVerb::Read.phrase("session rules", true), "Read session rules");
    assert_eq!(ToolVerb::Shell.phrase("git status", false), "Running git status");
    assert_eq!(ToolVerb::Shell.phrase("git status", true), "Ran git status");

    let running = ActivityRow::tool(
        ToolVerb::Search,
        "project code",
        ActivityState::Running,
        "128 of 400 files",
    )
    .with_elapsed("4s")
    .compact();
    let running_txt = running.lines()[0].text.clone();
    assert!(
        running_txt.contains("Searching project code"),
        "present verb-first header: {running_txt}"
    );
    assert!(
        !running_txt.contains("· Running"),
        "verb-first must not restates Running noun: {running_txt}"
    );

    let done = ActivityRow::tool(
        ToolVerb::Search,
        "project code",
        ActivityState::Succeeded,
        "3 matches",
    )
    .with_elapsed("0.4s")
    .compact();
    let done_txt = done.lines()[0].text.clone();
    assert!(
        done_txt.contains("Searched project code"),
        "past verb-first header: {done_txt}"
    );
    assert!(
        !done_txt.contains("Complete"),
        "past verb replaces Complete noun: {done_txt}"
    );

    let elbow = tool_elbow_line("middleware/rate_limit.rs:42").expect("elbow");
    assert!(elbow.text.contains("⎿"));
    assert!(elbow.text.contains("middleware/rate_limit.rs:42"));

    let fold = tool_fold_summary(ToolVerb::Read, 3, "files").expect("fold");
    assert!(fold.text.contains("Read 3 files"), "{}", fold.text);
}

#[test]
fn busy_frames_paint_turn_status_idle_frames_do_not() {
    let streaming = audit_screen("streaming", "public-audit").expect("streaming");
    let streaming_txt = streaming.render_txt();
    assert!(
        streaming_txt.contains("Responding") || streaming_txt.contains("● Responding"),
        "streaming must paint turn-status band:\n{}",
        streaming_txt.lines().rev().take(12).collect::<Vec<_>>().join("\n")
    );
    assert!(
        streaming_txt.contains("Esc to stop") || streaming_txt.contains("Esc to stop reply"),
        "turn-status stop prose required:\n{streaming_txt}"
    );
    assert!(!streaming_txt.contains("[stop]"));
    assert!(!contains_global_putty_footer(&streaming_txt));

    let tool = audit_screen("tool-running", "tool-pilot-running").expect("tool-running");
    let tool_txt = tool.render_txt();
    assert!(
        tool_txt.contains("Running") && (tool_txt.contains("● Running") || tool_txt.contains("Searching")),
        "tool-running paints live band or verb:\n{}",
        tool_txt.lines().rev().take(14).collect::<Vec<_>>().join("\n")
    );
    assert!(
        tool_txt.contains("Searching project code") || tool_txt.contains("Search project code"),
        "present verb header required:\n{tool_txt}"
    );
    assert!(!tool_txt.contains("[stop]"));

    let idle_chat = audit_screen("chat-turn", "conversation-completed").expect("chat-turn");
    let idle_txt = idle_chat.render_txt();
    // Idle: no Responding / Running band above jewel.
    assert!(
        !idle_txt.lines().any(|line| {
            let t = line.trim();
            (t.starts_with('●') && t.contains("Responding"))
                || (t.starts_with('●') && t.contains("Running ·") && t.contains("Esc to stop"))
        }),
        "idle chat-turn must collapse turn-status band"
    );

    let success = audit_screen("tool-success", "public-audit").expect("tool-success");
    let success_txt = success.render_txt();
    assert!(
        success_txt.contains("Searched project code") || success_txt.contains("Search project code"),
        "past or legacy success header:\n{success_txt}"
    );
    // Idle success has no busy strip.
    assert!(!success_txt.contains("[stop]"));
    assert!(!contains_global_putty_footer(&success_txt));
}

#[test]
fn jewel_user_strip_and_copper_language_on_conversation_shells() {
    for screen_id in CONVERSATION_IDS {
        let fixture = fixture_for(screen_id);
        let audit = audit_screen(screen_id, fixture)
            .unwrap_or_else(|err| panic!("{screen_id}/{fixture}: {err}"));
        let text = audit.render_txt();
        assert!(
            !contains_bracket_key_atom(&text),
            "{screen_id} leaked bracket atoms"
        );
        assert!(
            !contains_global_putty_footer(&text),
            "{screen_id} painted permanent putty footer"
        );
        assert!(
            !text.contains('❯') && !text.contains('◆') && !text.contains('✦'),
            "{screen_id} used forbidden Cortex decorative glyphs"
        );
        // Outer editorial pad: subject chrome starts inside gutter, not col 0 fill of text.
        // Market jewel border markers appear on conversation shells with a composer.
        if !matches!(*screen_id, "todo-list" | "compact") {
            // most shells still carry jewel; allow room for dock-only failures elsewhere.
        }
        assert!(
            text.contains("╭") && text.contains("╰"),
            "{screen_id} must compose prompt jewel borders"
        );
        assert!(
            text.contains("Claude") || text.contains("Work mode") || text.contains("Enter to"),
            "{screen_id} must show model/work info or local byline"
        );
    }

    let home = audit_screen("home", "public-audit").expect("home");
    let home_txt = home.render_txt();
    assert!(home_txt.contains('█'), "home keeps Code brand");
    assert!(home_txt.contains("│ › ") || home_txt.contains("› "));
    assert!(home_txt.contains("Claude Opus 4.8") || home_txt.contains("Work mode"));

    let chat = audit_screen("chat-turn", "conversation-completed").expect("chat");
    let chat_txt = chat.render_txt();
    // User surface strip leads with closed › (may double if request already had You ·).
    assert!(
        chat_txt.contains('›'),
        "chat-turn paints closed prompt/user prefix"
    );
}

#[test]
fn tool_stream_uses_present_past_verbs_not_essay_walls() {
    let running = audit_screen("tool-running", "tool-pilot-running").expect("running");
    let rtxt = running.render_txt();
    assert!(
        rtxt.contains("Searching project code") || rtxt.contains("Search project code"),
        "{rtxt}"
    );
    assert!(
        rtxt.contains("Read session rules") || rtxt.contains("Reading session rules"),
        "completed sibling past/present: {rtxt}"
    );
    // No multi-paragraph reassurance wall about the tool API.
    assert!(
        !rtxt.contains("tool API") && !rtxt.contains("transport") && !rtxt.contains("worker_id"),
        "tool stream must not dump technical walls"
    );

    let calls = audit_screen("tool-calls", "public-audit").expect("calls");
    let ctxt = calls.render_txt();
    assert!(
        ctxt.contains("Read session rules") || ctxt.contains("Reading"),
        "{ctxt}"
    );
    assert!(
        ctxt.contains("Ran rate-limit") || ctxt.contains("Run rate-limit") || ctxt.contains("rate-limit tests"),
        "{ctxt}"
    );
    assert!(
        ctxt.contains("Searching project code") || ctxt.contains("Search project code"),
        "{ctxt}"
    );

    let expanded = audit_screen("tool-success-expanded", "public-audit").expect("expanded");
    let etxt = expanded.render_txt();
    assert!(
        etxt.contains("⎿") || etxt.contains("middleware/rate_limit"),
        "expanded detail must use elbows/paths under the tool: {etxt}"
    );
}
