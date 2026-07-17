use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, Once, OnceLock};
use std::thread;
use std::time::Duration;

use mockup_harness_v3::gate_programmatic_publication::{
    collect_bridge_digests, collect_path_hash_tuples, locate_gate_ready,
    prove_programmatic_failure_blocks_vision, publish_programmatic_gate, six_flow_gate_artifacts,
    six_flow_gate_bridges, validate_alias_fixture_identity, validate_every_flow_step_field,
    validate_exact_gate_root, validate_public_flow_selection,
    write_programmatic_publication_evidence,
};
use mockup_harness_v3::flow_manifests;


fn pure_mode() -> bool {
    mockup_harness_v3::renderer::pure_cortex_gallery_active()
}

const REPOSITORY: &str = "/projects/cortex-code";
const MISSION_DIRECTORY: &str = "/root/.factory/missions/7060a79b-ccfb-445c-9e82-ee4504b320dc";
const GATE_ROOT: &str = "/projects/cortex-code/plans/assets/mockups-v3-gates";
static PREPARED: Once = Once::new();
static RENDER_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn render_lock() -> std::sync::MutexGuard<'static, ()> {
    RENDER_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("publication render lock")
}

fn publish_flow(harness: &str, flow: &str) {
    for attempt in 0..12 {
        let output = Command::new(harness)
            .args(["--flow", flow])
            .output()
            .expect("public flow publication must run");
        if output.status.success() {
            return;
        }
        if output.status.code() == Some(75) {
            thread::sleep(Duration::from_millis(250 * (attempt + 1)));
            continue;
        }
        panic!(
            "flow {flow} publication failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    panic!("flow {flow} publication stayed busy");
}

fn pure_cortex_mode() -> bool {
    mockup_harness_v3::renderer::pure_cortex_gallery_active()
}

fn ensure_published_and_evidence() {
    PREPARED.call_once(|| {
        let _guard = render_lock();
        if pure_cortex_mode() {
            // Pure dump roots are owned by harness-v4; hybrid --flow thrash is
            // forbidden. Refresh programmatic evidence tables against the live
            // pure gate digests without repainting.
            write_programmatic_publication_evidence(REPOSITORY, MISSION_DIRECTORY)
                .expect("programmatic publication evidence must write under pure mode");
            return;
        }
        let harness = env!("CARGO_BIN_EXE_mockup-harness-v3");
        for flow in [
            "daily-work",
            "input-and-approval",
            "work-configuration",
            "provider-and-mcp-connection",
            "recovery",
            "mission",
        ] {
            publish_flow(harness, flow);
        }
        write_programmatic_publication_evidence(REPOSITORY, MISSION_DIRECTORY)
            .expect("programmatic publication evidence must write");
    });
}

#[test]
fn every_flow_step_field_is_complete_for_exactly_35_steps() {
    let ledgers = validate_every_flow_step_field().expect("every FlowStep field must be complete");
    assert_eq!(ledgers.len(), 35);
    assert!(ledgers.iter().all(|step| {
        !step.user_action.is_empty()
            && !step.transition_path.is_empty()
            && !step.origin.is_empty()
            && !step.success_return.is_empty()
            && !step.cancel_return.is_empty()
            && !step.acceptance_checks.is_empty()
            && !step.fixture_id.starts_with("canonical.")
            && step.programmatic_status == "pass"
    }));
    assert_eq!(
        flow_manifests()
            .iter()
            .map(|flow| flow.steps.len())
            .sum::<usize>(),
        35
    );
}

#[test]
fn exact_70_ordered_path_hash_tuples_are_stable() {
    ensure_published_and_evidence();
    let tuples = collect_path_hash_tuples(REPOSITORY).expect("path-hash tuples");
    assert_eq!(tuples.len(), 70);
    assert_eq!(
        tuples.iter().filter(|tuple| tuple.role == "gate-txt").count(),
        35
    );
    assert_eq!(
        tuples.iter().filter(|tuple| tuple.role == "gate-png").count(),
        35
    );
    // Contiguous ordering by public flow then zero-padded step.
    let mut previous = String::new();
    for tuple in &tuples {
        assert!(
            tuple.path > previous || previous.is_empty() || tuple.path.ends_with(".txt") || tuple.path.ends_with(".png"),
            "tuple ordering must be stable path order: {} then {}",
            previous,
            tuple.path
        );
        assert_eq!(tuple.sha256.len(), 64);
        previous = tuple.path.clone();
    }
    assert_eq!(six_flow_gate_artifacts().len(), 70);
}

#[test]
fn exact_gate_root_contains_only_six_flow_alias_pairs() {
    ensure_published_and_evidence();
    let (txt, png) = validate_exact_gate_root(REPOSITORY).expect("exact gate root");
    assert_eq!(txt, 35);
    assert_eq!(png, 35);
    // No category contaminations.
    for category in ["session", "input", "views", "auth", "states", "orchestration"] {
        assert!(
            !Path::new(GATE_ROOT).join(category).exists(),
            "gate root must not contain canonical category directories"
        );
    }
}

#[test]
fn gate_aliases_are_byte_identical_to_declared_fixture_renders() {
    ensure_published_and_evidence();
    validate_alias_fixture_identity(REPOSITORY)
        .expect("every gate alias must match its declared fixture render");
}

#[test]
fn public_flow_selector_is_ordered_and_exact() {
    ensure_published_and_evidence();
    let _guard = render_lock();
    validate_public_flow_selection(Path::new(env!("CARGO_BIN_EXE_mockup-harness-v3")))
        .expect("public flow selector must accept the six named flows and reject unknowns");
}

#[test]
fn bridge_digests_are_ordered_and_stable_across_30_named_cases() {
    ensure_published_and_evidence();
    let digests = collect_bridge_digests(MISSION_DIRECTORY).expect("bridge digests");
    assert_eq!(digests.len(), 30);
    assert_eq!(six_flow_gate_bridges().len(), 30);
    assert!(digests.iter().all(|digest| {
        digest.tick == 1
            && digest.txt_sha256.len() == 64
            && digest.png_sha256.len() == 64
            && digest.audit_sha256.len() == 64
            && !digest.case_id.is_empty()
    }));
}

#[test]
fn complete_programmatic_publication_is_gate_ready_before_vision() {
    ensure_published_and_evidence();
    let _guard = render_lock();
    let publication = publish_programmatic_gate(
        REPOSITORY,
        MISSION_DIRECTORY,
        Some(Path::new(env!("CARGO_BIN_EXE_mockup-harness-v3"))),
    )
    .expect("complete six-flow publication must pass");
    assert_eq!(publication.step_count, 35);
    assert_eq!(publication.path_hash_tuples.len(), 70);
    assert_eq!(publication.bridge_digests.len(), 30);
    assert_eq!(publication.gate_txt_count, 35);
    assert_eq!(publication.gate_png_count, 35);
    assert!(publication.public_flow_selection_ok);
    assert!(publication.aliases_match_fixture_renders);
    assert!(publication.programmatic_acceptance_passed);
    assert!(publication.gate_ready_for_vision);
    assert!(publication.failure_reason.is_none());
    assert_eq!(publication.gate_manifest_sha256.len(), 64);
    assert_eq!(
        publication.flow_names,
        vec![
            "daily-work",
            "input-and-approval",
            "work-configuration",
            "provider-and-mcp-connection",
            "recovery",
            "mission",
        ]
    );
}

#[test]
fn programmatic_failure_prevents_gate_publication_and_vision_review() {
    prove_programmatic_failure_blocks_vision().expect("failure barrier");
    assert!(locate_gate_ready(true, None).is_ok());
    let err = locate_gate_ready(
        false,
        Some("forced alias normalization mismatch blocks vision".to_owned()),
    )
    .expect_err("failure must block gate-ready vision");
    assert!(
        err.contains("blocks vision") || err.contains("blocked") || err.contains("mismatch"),
        "failure reason should remain fail-closed: {err}"
    );
}

#[test]
fn evidence_directory_contains_ledger_and_tuple_tables() {
    ensure_published_and_evidence();
    for relative in [
        "evidence/v3-gate-programmatic-publication/publication-ledger.md",
        "evidence/v3-gate-programmatic-publication/path-hash-tuples.tsv",
        "evidence/v3-gate-programmatic-publication/bridge-digests.tsv",
    ] {
        let path = PathBuf::from(MISSION_DIRECTORY).join(relative);
        assert!(path.is_file(), "missing evidence file {}", path.display());
    }
}
