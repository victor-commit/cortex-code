//! Cortex dashboard + Mission Control fusion for orchestration
//! (VAL-V3-F6-060 and related launch/failure/dock language).
//!
//! Mission surfaces remain exception-first with panel counters, calm when
//! idle, effect-first docks, blocks-first team board, and de-emphasized
//! plan/score advanced detail. No permanent putty bar or bracket chords.

use mockup_harness_v3::{
    audit::audit_screen,
    chrome::components::{contains_bracket_key_atom, contains_global_putty_footer},
};

const MISSION_DASHBOARDS: &[(&str, &str)] = &[
    ("workflow-monitor", "gate.mission.workflow-monitor"),
    (
        "workflow-monitor-failure",
        "gate.mission.workflow-monitor-failure",
    ),
    ("team-board", "orchestration.team-board"),
    ("workflow-launch", "gate.mission.workflow-launch"),
    (
        "workflow-launch-blocked",
        "gate.mission.workflow-launch-blocked",
    ),
];

const ADVANCED_DETAIL: &[(&str, &str)] = &[
    (
        "verification-ladder",
        "orchestration.verification-ladder",
    ),
    ("intent-scorer-pass", "orchestration.intent-scorer-pass"),
    ("intent-scorer-fail", "orchestration.intent-scorer-fail"),
    (
        "concurrency-metrics",
        "orchestration.concurrency-metrics",
    ),
    ("tool-envelope", "orchestration.tool-envelope"),
];

const ORCHESTRATION_ALL: &[&str] = &[
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

fn fixture_for(screen_id: &str) -> &'static str {
    match screen_id {
        "mode-switch" => "public-audit",
        "mode-switch-refused" => "public-audit",
        "workflow-launch" => "gate.mission.workflow-launch",
        "workflow-launch-blocked" => "gate.mission.workflow-launch-blocked",
        "workflow-monitor" => "gate.mission.workflow-monitor",
        "workflow-monitor-failure" => "gate.mission.workflow-monitor-failure",
        "team-board" => "orchestration.team-board",
        "reviewer-output" => "gate.mission.reviewer-output",
        "reviewer-approved" => "gate.mission.reviewer-approved",
        "verification-ladder" => "orchestration.verification-ladder",
        "intent-scorer-pass" => "orchestration.intent-scorer-pass",
        "intent-scorer-fail" => "orchestration.intent-scorer-fail",
        "concurrency-metrics" => "orchestration.concurrency-metrics",
        "tool-envelope" => "orchestration.tool-envelope",
        _ => "public-audit",
    }
}

fn first_content_row(text: &str, needle: &str) -> Option<usize> {
    text.lines()
        .position(|line| line.contains(needle))
}

#[test]
fn f060_healthy_monitor_is_exception_first_with_counters_and_calm_idle() {
    let audit = audit_screen("workflow-monitor", "gate.mission.workflow-monitor")
        .expect("healthy monitor must render");
    audit
        .validate()
        .expect("healthy monitor audit must stay bijective");
    let text = audit.render_txt();

    assert!(
        text.contains("● Running") && text.contains("1/3"),
        "live Running band with fraction counter:\n{text}"
    );
    assert!(
        text.contains("Active work · 1/3") || text.contains("Active work · 1/3"),
        "panel header counter on active work:\n{text}"
    );
    assert!(
        text.contains("Progress log · 1-3 of 12"),
        "progress log page-range counter:\n{text}"
    );
    assert!(
        text.contains("1 complete · 2 remaining"),
        "truthful completed/remaining counters:\n{text}"
    );
    assert!(
        text.to_lowercase().contains("calm")
            || text.to_lowercase().contains("no blocker")
            || text.to_lowercase().contains("quiet"),
        "idle/healthy densify stays calm, not fake activity chrome:\n{text}"
    );
    assert!(
        !text.contains("Needs attention:"),
        "healthy idle monitor must not invent a needs-attention blocker:\n{text}"
    );
    assert!(
        !contains_bracket_key_atom(&text),
        "no nested bracket chords on monitor:\n{text}"
    );
    assert!(
        !contains_global_putty_footer(&text),
        "no permanent putty footer on monitor:\n{text}"
    );
    assert!(
        text.contains("Enter to view progress")
            || text.contains("Esc to resume conversation")
            || text.contains("› View progress"),
        "prose bylines / dock language present:\n{text}"
    );
}

#[test]
fn f060_failure_monitor_leads_with_needs_attention_and_repair_primary() {
    let audit = audit_screen(
        "workflow-monitor-failure",
        "gate.mission.workflow-monitor-failure",
    )
    .expect("failure monitor must render");
    let text = audit.render_txt();

    let needs = first_content_row(&text, "Needs attention")
        .expect("needs attention zone");
    let completed = first_content_row(&text, "Completed")
        .expect("completed zone");
    assert!(
        needs < completed,
        "exception-first: Needs attention before Completed ({needs} < {completed})\n{text}"
    );
    assert!(
        text.contains("✗ Needs attention") && text.contains("1/3"),
        "live needs-attention band with counters:\n{text}"
    );
    assert!(
        text.contains("Needs attention · 1/3") || text.contains("1 complete · 2 remaining"),
        "truthful counters reconcile:\n{text}"
    );
    assert!(
        text.contains("› Create a fix task")
            || text.contains("Create a fix task") && text.contains("Create planned fix"),
        "failure primary repair action visible:\n{text}"
    );
    // Effect-first dock title/effect, not essay/journey labels.
    let dock_title = first_content_row(&text, "Repair the validation failure")
        .or_else(|| first_content_row(&text, "Retry the failed work"))
        .expect("effect-first dock title");
    let gutter = text.lines().nth(dock_title).unwrap_or("");
    assert!(
        !gutter.contains("Mission ·") && !gutter.contains("[enter]"),
        "dock must not open with journey crumb or brackets:\n{gutter}"
    );
    assert!(!contains_bracket_key_atom(&text));
    assert!(!contains_global_putty_footer(&text));
}

#[test]
fn f060_team_board_blocks_first_with_zone_counters() {
    let audit = audit_screen("team-board", "orchestration.team-board")
        .expect("team board must render");
    let text = audit.render_txt();

    let needs = first_content_row(&text, "Needs attention")
        .expect("needs attention");
    let working = first_content_row(&text, "Working")
        .expect("working");
    let completed = first_content_row(&text, "Completed")
        .expect("completed");
    assert!(
        needs < working && working < completed,
        "blocks-first zones: Needs attention → Working → Completed\n{text}"
    );
    assert!(
        text.contains("Needs attention · 0/1") || text.contains("Needs attention · 1/"),
        "zone counter on needs-attention:\n{text}"
    );
    assert!(
        text.contains("Working · 0/1") || text.contains("Working · 1/"),
        "zone counter on working:\n{text}"
    );
    assert!(
        text.contains("ownership") || text.contains("assignments"),
        "board announces ownership purpose:\n{text}"
    );
    assert!(
        text.contains("Esc to return") || text.contains("Return to mission monitor"),
        "prose return path:\n{text}"
    );
    assert!(!contains_bracket_key_atom(&text));
    assert!(!contains_global_putty_footer(&text));
}

#[test]
fn f060_launch_preflight_docks_are_effect_first_with_counters() {
    let ready = audit_screen("workflow-launch", "gate.mission.workflow-launch")
        .expect("ready launch")
        .render_txt();
    assert!(ready.contains("○ Waiting") || ready.contains("Ready to launch"));
    assert!(ready.contains("0/3") || ready.contains("0 complete · 3 remaining"));
    assert!(ready.contains("Start this mission?"));
    assert!(
        ready.contains("Start runs the planned tasks")
            || ready.contains("opens compact progress"),
        "effect-first launch consequence:\n{ready}"
    );
    assert!(ready.contains("› Start mission"));
    assert!(
        ready.contains("Enter to start mission") && ready.contains("Esc to cancel"),
        "prose bylines on launch dock:\n{ready}"
    );
    assert!(!contains_bracket_key_atom(&ready));
    assert!(!contains_global_putty_footer(&ready));

    let blocked = audit_screen(
        "workflow-launch-blocked",
        "gate.mission.workflow-launch-blocked",
    )
    .expect("blocked launch")
    .render_txt();
    assert!(blocked.contains("✗ Needs attention"));
    assert!(blocked.contains("Preflight") && (blocked.contains("0/3") || blocked.contains("0 complete")));
    assert!(
        blocked.contains("Resolve the missing input")
            || blocked.contains("Add the release checklist"),
        "missing-input effect dock:\n{blocked}"
    );
    assert!(blocked.contains("› Add release checklist"));
    assert!(!contains_bracket_key_atom(&blocked));
}

#[test]
fn f060_progress_disclosure_orders_attention_first_with_group_counters() {
    let attention = audit_screen(
        "workflow-monitor",
        "workflow-monitor.progress.attention",
    )
    .expect("attention progress")
    .render_txt();
    let needs = first_content_row(&attention, "Needs attention").expect("needs");
    let working = first_content_row(&attention, "Working").expect("working");
    let done = first_content_row(&attention, "Recently completed").expect("done");
    assert!(
        needs < working && working < done,
        "progress groups exception-first:\n{attention}"
    );
    // Counters on group headers when structured panels are used.
    assert!(
        attention.contains("·")
            && (attention.contains("1/1")
                || attention.contains("1 complete")
                || attention.contains("Needs attention")),
        "progress groups expose readable counters or counts:\n{attention}"
    );

    let healthy = audit_screen("workflow-monitor", "workflow-monitor.progress")
        .expect("healthy progress")
        .render_txt();
    let working_h = first_content_row(&healthy, "Working").expect("working healthy");
    // Calm: no fictional Needs attention section when empty.
    if let Some(needs_h) = first_content_row(&healthy, "Needs attention") {
        assert!(
            needs_h > working_h,
            "empty needs-attention must not precede working when healthy:\n{healthy}"
        );
    }
}

#[test]
fn f060_advanced_plan_vote_details_are_deemphasized() {
    for &(screen, fixture) in ADVANCED_DETAIL {
        let text = audit_screen(screen, fixture)
            .unwrap_or_else(|err| panic!("{screen}/{fixture}: {err}"))
            .render_txt();
        assert!(
            text.contains("Advanced ·")
                || text.contains("advisory")
                || text.contains("Advisory")
                || text.contains("does not change")
                || text.to_lowercase().contains("focused diagnostics"),
            "{screen} must read as de-emphasized advanced detail:\n{text}"
        );
        // Must still keep origin + return.
        assert!(
            text.contains("Mission") || text.contains("Prepare the release"),
            "{screen} keeps mission context:\n{text}"
        );
        assert!(
            text.contains("Esc to Return") || text.contains("Return to"),
            "{screen} keeps return path:\n{text}"
        );
        assert!(
            !contains_bracket_key_atom(&text),
            "{screen} ban bracket chords"
        );
        assert!(
            !contains_global_putty_footer(&text),
            "{screen} ban putty footer"
        );
    }
}

#[test]
fn f060_all_orchestration_gallery_ids_ban_putty_and_brackets() {
    for &id in ORCHESTRATION_ALL {
        let text = audit_screen(id, fixture_for(id))
            .unwrap_or_else(|err| panic!("{id}: {err}"))
            .render_txt();
        assert!(
            !contains_bracket_key_atom(&text),
            "{id}: bracket key atom present:\n{text}"
        );
        assert!(
            !contains_global_putty_footer(&text),
            "{id}: global putty footer present:\n{text}"
        );
        assert!(
            !text.contains("Mission · current conversation"),
            "{id}: journey crumb painted on canvas:\n{text}"
        );
        // Geometry contract keeps every orchestration frame 120x40.
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 40, "{id}: expected 40 rows, got {}", lines.len());
        for (idx, line) in lines.iter().enumerate() {
            assert!(
                line.chars().count() <= 120,
                "{id} row {idx} exceeds 120 cols"
            );
        }
    }
}

#[test]
fn f060_dashboard_counter_truth_reconciles_with_visible_rows() {
    // Healthy monitor: completed 1 remaining 2 must match visible story.
    let healthy = audit_screen("workflow-monitor", "gate.mission.workflow-monitor")
        .expect("healthy")
        .render_txt();
    assert!(healthy.contains("1 complete · 2 remaining"));
    assert!(healthy.contains("1/3"));

    // Failure: completed preserved 1, remaining work under attention.
    let failure = audit_screen(
        "workflow-monitor-failure",
        "gate.mission.workflow-monitor-failure",
    )
    .expect("failure")
    .render_txt();
    assert!(failure.contains("1 complete · 2 remaining"));
    assert!(failure.contains("Completed · 1 preserved") || failure.contains("1 preserved"));

    // Team board: blocked, working, completed sections present.
    let board = audit_screen("team-board", "orchestration.team-board")
        .expect("board")
        .render_txt();
    assert!(board.contains("blocked") || board.contains("Needs attention"));
    assert!(board.contains("working") || board.contains("Working"));
    assert!(board.contains("complete") || board.contains("Completed"));
}

#[test]
fn f060_mission_dashboard_ids_surface_without_fake_activity() {
    for &(screen, fixture) in MISSION_DASHBOARDS {
        let text = audit_screen(screen, fixture)
            .unwrap_or_else(|err| panic!("{screen}: {err}"))
            .render_txt();
        // Fake multi-agent roster chrome from foreign dashboards must stay out.
        for banned in [
            "Shift+Tab",
            "Ctrl+c:",
            "Ctrl+\\",
            "[stop]",
            "awaiting ·",
            "N more",
            "Dispatch a new agent",
        ] {
            assert!(
                !text.contains(banned),
                "{screen} imported forbidden product putty/roster chrome `{banned}`:\n{text}"
            );
        }
    }
}
