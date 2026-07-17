use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Once;

use mockup_harness_v3::{
    audit::audit_screen,
    flow_manifests,
    gate_remaining_integration::{
        remaining_gate_artifacts, remaining_gate_bridges, remaining_gate_publications,
        validate_remaining_gate_integration, validate_remaining_gate_publications,
        validate_remaining_transition_contracts, validate_six_flow_gate_product,
    },
    registry::canonical_inventory,
    scenario::render_audit_png,
};


fn pure_mode() -> bool {
    mockup_harness_v3::renderer::pure_cortex_gallery_active()
}

const REPOSITORY: &str = "/projects/cortex-code";
const MISSION_DIRECTORY: &str = "/root/.factory/missions/7060a79b-ccfb-445c-9e82-ee4504b320dc";
const GATE_ROOT: &str = "/projects/cortex-code/plans/assets/mockups-v3-gates";
static PUBLISHED: Once = Once::new();

fn ensure_published() {
    PUBLISHED.call_once(|| {
        let harness = env!("CARGO_BIN_EXE_mockup-harness-v3");
        for arguments in [
            vec!["--implemented"],
            vec!["--flow", "daily-work"],
            vec!["--flow", "input-and-approval"],
            vec!["--flow", "work-configuration"],
            vec!["--flow", "provider-and-mcp-connection"],
            vec!["--flow", "recovery"],
            vec!["--flow", "mission"],
        ] {
            let output = Command::new(harness)
                .args(arguments)
                .output()
                .expect("the public renderer must publish pilot evidence");
            assert!(
                output.status.success(),
                "pilot publication failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    });
}

#[test]
fn remaining_three_manifests_join_to_the_published_gate_contract() {
    ensure_published();
    let report = validate_remaining_gate_integration(REPOSITORY, MISSION_DIRECTORY)
        .expect("the remaining three gate manifests must join to valid evidence");
    assert_eq!(
        report.flow_names,
        vec![
            "provider-and-mcp-connection",
            "recovery",
            "mission"
        ]
    );
    assert_eq!(report.step_count, 17);
    assert_eq!(report.artifact_count, 34);
    assert_eq!(report.bridge_count, 18);
    assert!(report.programmatic_checks_before_vision);
}

#[test]
fn remaining_publication_records_bind_declared_fixtures_and_origins() {
    let publications = remaining_gate_publications();
    assert_eq!(publications.len(), 17);
    validate_remaining_gate_publications(&publications)
        .expect("the remaining-three publication map must validate");
    assert!(publications.iter().all(|publication| {
        !publication.fixture_id.starts_with("canonical.")
            && !publication.origin.trim().is_empty()
            && publication.gate_txt.ends_with(".txt")
            && publication.gate_png.ends_with(".png")
    }));

    let mut fallback = publications.clone();
    fallback[0].fixture_id = "canonical.login".to_owned();
    assert!(validate_remaining_gate_publications(&fallback)
        .expect_err("canonical fixture fallback must fail")
        .contains("fixture"));
}

#[test]
fn remaining_declared_actions_and_illustrative_bridges_truthfully_reconcile() {
    validate_remaining_transition_contracts()
        .expect("all remaining direct, automatic, summarized, and illustrative contracts must reconcile");
}

#[test]
fn remaining_gate_aliases_match_declared_fixture_renderings() {
    ensure_published();
    let mut seen = BTreeSet::new();
    for artifact in remaining_gate_artifacts() {
        assert!(seen.insert(artifact.gate_relative.clone()));
        let gate = Path::new(GATE_ROOT).join(&artifact.gate_relative);
        assert!(gate.is_file(), "missing gate artifact: {gate:?}");
        let expected = audit_screen(artifact.screen_id, artifact.fixture_id)
            .expect("the declared canonical renderer and fixture must resolve");
        let expected_bytes = if artifact.extension == "txt" {
            expected.render_txt().into_bytes()
        } else {
            render_audit_png(&expected).expect("the declared canonical fixture must rasterize")
        };
        assert_eq!(
            expected_bytes,
            fs::read(&gate).expect("gate artifact must be readable"),
            "gate alias must be fixture-bound: {gate:?}"
        );
    }
}

#[test]
fn remaining_storyboards_match_exact_gate_sequences() {
    let expected = [
        (
            "provider-and-mcp-connection",
            vec![
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
            vec![
                "turn-error",
                "error-network",
                "rate-limit",
                "chat-turn",
                "checkpoint-undo",
            ],
        ),
        (
            "mission",
            vec![
                "workflow-launch-blocked",
                "workflow-launch",
                "workflow-monitor",
                "workflow-monitor-failure",
                "reviewer-output",
                "reviewer-approved",
            ],
        ),
    ];
    for (name, screens) in expected {
        let flow = flow_manifests()
            .iter()
            .find(|flow| flow.name == name)
            .expect("flow must exist");
        assert_eq!(
            flow.steps
                .iter()
                .map(|step| step.screen_id)
                .collect::<Vec<_>>(),
            screens
        );
    }
}

#[test]
fn provider_success_never_adds_direct_mcp_edge_and_mcp_returns_to_conversation() {
    let connected = audit_screen(
        "connect-connected",
        "gate.provider-and-mcp.connect-connected",
    )
    .expect("connected");
    assert!(
        !connected
            .transitions
            .iter()
            .any(|transition| transition.to_screen_id == "mcp" && transition.kind == "direct")
    );
    let mcp = audit_screen("mcp", "gate.provider-and-mcp.mcp").expect("mcp");
    assert!(mcp.transitions.iter().any(|transition| {
        transition.action_id == "action.mcp.return" && transition.to_screen_id == "chat-turn"
    }));
}

#[test]
fn mission_and_mcp_receive_focused_transition_review() {
    ensure_published();
    let report = validate_remaining_gate_integration(REPOSITORY, MISSION_DIRECTORY)
        .expect("remaining integration");
    assert!(report.bridge_count >= 2);
    // MCP and mission bridges must be present and named truthfully.
    let mcp_bridge = remaining_gate_bridges()
        .iter()
        .find(|bridge| bridge.case_id == "provider-to-mcp-bridge")
        .expect("provider-to-mcp bridge");
    assert_eq!(mcp_bridge.expected_kind, "illustrative_detour");
    assert_eq!(mcp_bridge.expected_before, "connect-connected");
    assert_eq!(mcp_bridge.expected_after, "mcp");

    let mission_failure = remaining_gate_bridges()
        .iter()
        .find(|bridge| bridge.case_id == "mission-monitor-to-failure-bridge")
        .expect("mission failure detour");
    assert_eq!(mission_failure.expected_kind, "illustrative_detour");

    let mission_fix = remaining_gate_bridges()
        .iter()
        .find(|bridge| bridge.case_id == "mission-failure-to-review-bridge")
        .expect("mission fix summary");
    assert_eq!(mission_fix.expected_kind, "summarized");
}

#[test]
fn six_flow_product_reconciles_first_three_and_remaining_without_new_ids() {
    ensure_published();
    let report = validate_six_flow_gate_product(REPOSITORY, MISSION_DIRECTORY)
        .expect("six-flow product must reconcile");
    assert_eq!(
        report.flow_names,
        vec![
            "daily-work",
            "input-and-approval",
            "work-configuration",
            "provider-and-mcp-connection",
            "recovery",
            "mission",
        ]
    );
    assert_eq!(report.step_count, 35);
    assert_eq!(report.artifact_count, 70);
    assert_eq!(report.first_three_step_count, 18);
    assert_eq!(report.remaining_step_count, 17);
    assert!(report.no_new_canonical_ids);
    assert_eq!(canonical_inventory().len(), 86);
}

#[test]
fn remaining_gate_directories_have_only_ordered_txt_png_pairs() {
    ensure_published();
    for flow in flow_manifests()
        .iter()
        .filter(|flow| {
            matches!(
                flow.name,
                "provider-and-mcp-connection" | "recovery" | "mission"
            )
        })
    {
        let directory = PathBuf::from(GATE_ROOT).join(flow.name);
        let mut actual = fs::read_dir(&directory)
            .expect("published gate directory must exist")
            .map(|entry| {
                entry
                    .expect("gate directory entry must be readable")
                    .file_name()
            })
            .collect::<Vec<_>>();
        actual.sort();
        let expected = remaining_gate_artifacts()
            .iter()
            .filter(|artifact| {
                artifact
                    .gate_relative
                    .starts_with(&format!("{}/", flow.name))
            })
            .map(|artifact| {
                artifact
                    .gate_relative
                    .split_once('/')
                    .expect("flow prefix")
                    .1
                    .to_owned()
            })
            .map(std::ffi::OsString::from)
            .collect::<Vec<_>>();
        let mut expected = expected;
        expected.sort();
        assert_eq!(actual, expected, "gate artifacts must be ordered");
    }
}
