//! Human-gate approval protocol tests for VAL-V3-GATE-014 and VAL-V3-GATE-015.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, Once, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use mockup_harness_v3::gate_approval::{
    identify_affected_pilot_features, load_decision_if_present, load_request,
    prove_changed_digest_requires_new_directory, prove_worker_decision_is_rejected,
    revalidate_request_against_live_bundle, validate_approval_integrity,
    worker_write_decision_is_forbidden, write_human_gate_request, ApprovalIntegrity,
    APPROVAL_FEATURE_ID, APPROVAL_SCHEMA_VERSION, DECISION_FILE_NAME, HUMAN_GATE_ROOT,
    REQUEST_FILE_NAME,
};
use mockup_harness_v3::gate_programmatic_publication::locate_gate_ready;
use mockup_harness_v3::gate_vision_review::validate_gate_vision_review;

const REPOSITORY: &str = "/projects/cortex-code";
const MISSION_DIRECTORY: &str = "/root/.factory/missions/7060a79b-ccfb-445c-9e82-ee4504b320dc";

static PREFLIGHT: Once = Once::new();
static EVIDENCE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn evidence_lock() -> std::sync::MutexGuard<'static, ()> {
    EVIDENCE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("human-gate evidence lock")
}

fn ensure_programmatic_and_vision_ready() {
    PREFLIGHT.call_once(|| {
        locate_gate_ready(true, None).expect("programmatic gate ready");
        assert!(
            Path::new(MISSION_DIRECTORY)
                .join("evidence/v3-gate-programmatic-publication/publication-ledger.md")
                .is_file(),
            "programmatic publication ledger is required"
        );
        assert!(
            Path::new(MISSION_DIRECTORY)
                .join("evidence/v3-gate-vision-review/render-to-review-barrier.json")
                .is_file(),
            "vision barrier is required"
        );
        validate_gate_vision_review(REPOSITORY, MISSION_DIRECTORY)
            .expect("vision product must already be complete for human-gate approval");
    });
}

#[test]
fn approval_request_schema_binds_full_current_bundle() {
    let _guard = evidence_lock();
    ensure_programmatic_and_vision_ready();
    let request_time = 1_720_000_000_000u64;
    let request = write_human_gate_request(REPOSITORY, MISSION_DIRECTORY, Some(request_time))
        .expect("immutable awaiting_user request must write or reuse existing digest history");
    assert_eq!(request.schema_version, APPROVAL_SCHEMA_VERSION);
    assert_eq!(request.feature_id, APPROVAL_FEATURE_ID);
    assert_eq!(request.status, "awaiting_user");
    // Immutable history keeps the original request_time when the bundle digest already existed.
    assert!(request.request_time_unix_ms > 0);
    assert_eq!(request.path_hash_tuples.len(), 70);
    assert_eq!(request.review_sheet_digests.len(), 6);
    assert_eq!(request.bridge_digests.len(), 30);
    assert_eq!(request.request_digest.len(), 64);
    assert_eq!(request.review_bundle_sha256.len(), 64);
    assert_eq!(request.gate_manifest_sha256.len(), 64);
    assert!(request.pre_bulk_tree.entry_count > 0);
    assert_eq!(request.pre_bulk_tree.tree_sha256.len(), 64);

    let path = Path::new(MISSION_DIRECTORY).join(&request.relative_path);
    assert!(path.is_file(), "request.json must exist at {}", path.display());
    let body = fs::read_to_string(&path).expect("read request");
    for required in [
        "\"status\": \"awaiting_user\"",
        "\"feature_id\": \"v3-human-gate-approval\"",
        "\"path_hash_tuples\"",
        "\"review_sheet_digests\"",
        "\"bridge_digests\"",
        "\"pre_bulk_tree\"",
        "\"request_digest\"",
        "\"review_bundle_sha256\"",
        "\"gate_manifest_sha256\"",
        "\"request_time_unix_ms\"",
    ] {
        assert!(body.contains(required), "request missing {required}");
    }

    // Worker path must never author a decision. An orchestrator decision may already exist
    // for the live approved bundle; when present, it must be role-sealed to orchestrator.
    let decision = load_decision_if_present(MISSION_DIRECTORY, &request).expect("decision load");
    match decision {
        None => {}
        Some(decision) => {
            assert_eq!(
                decision.decision_actor_role, "orchestrator",
                "only the orchestrator may author decision.json"
            );
            assert_eq!(decision.request_digest, request.request_digest);
            assert_eq!(decision.reviewed_bundle_digest, request.review_bundle_sha256);
            assert!(decision.status == "approved" || decision.status == "rejected");
        }
    }
}

#[test]
fn approval_rejects_worker_authored_decision() {
    let sample = r#"{
  "schema_version": 1,
  "status": "approved",
  "user_actor": "gate-worker-session",
  "user_message_digest": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "decision_time_unix_ms": 1,
  "request_digest": "0000000000000000000000000000000000000000000000000000000000000000",
  "reviewed_bundle_digest": "1111111111111111111111111111111111111111111111111111111111111111",
  "decision_actor_role": "worker"
}"#;
    prove_worker_decision_is_rejected(sample).expect("worker decision must fail");
    let err = worker_write_decision_is_forbidden(
        MISSION_DIRECTORY,
        &"f".repeat(64),
        &mockup_harness_v3::gate_approval::HumanGateDecision {
            schema_version: APPROVAL_SCHEMA_VERSION,
            status: "approved".into(),
            user_actor: "user".into(),
            user_message_digest: "a".repeat(64),
            decision_time_unix_ms: 1,
            request_digest: "b".repeat(64),
            reviewed_bundle_digest: "c".repeat(64),
            decision_actor_role: "orchestrator".into(),
            relative_path: "x".into(),
        },
    )
    .expect_err("workers cannot write decisions at all");
    assert!(err.contains("never author") || err.contains("orchestrator"));
}

#[test]
fn approval_rejects_changed_gate_hash_with_new_directory_requirement() {
    let previous = "a".repeat(64);
    let next = "b".repeat(64);
    prove_changed_digest_requires_new_directory(&previous, &next).unwrap();
    let err = prove_changed_digest_requires_new_directory(&previous, &previous)
        .expect_err("identical digests must fail when a change was claimed");
    assert!(err.contains("new review_bundle_sha256") || err.contains("mutate"));
}

#[test]
fn awaiting_user_integrity_passes_and_require_approved_blocks() {
    let _guard = evidence_lock();
    ensure_programmatic_and_vision_ready();
    let request = write_human_gate_request(REPOSITORY, MISSION_DIRECTORY, Some(1_720_000_000_001))
        .expect("request write");
    let integrity = validate_approval_integrity(REPOSITORY, MISSION_DIRECTORY, false)
        .expect("integrity must accept the live request with or without orchestrator decision");
    match integrity {
        ApprovalIntegrity::AwaitingUser { request: loaded } => {
            assert_eq!(loaded.review_bundle_sha256, request.review_bundle_sha256);
            assert_eq!(loaded.status, "awaiting_user");
            // Bulk remains blocked until an orchestrator-approved decision exists.
            let blocked = validate_approval_integrity(REPOSITORY, MISSION_DIRECTORY, true)
                .expect_err("bulk production must remain blocked until orchestrator approval");
            assert!(
                blocked.contains("awaiting_user")
                    || blocked.contains("decision.json is missing")
                    || blocked.contains("blocked")
            );
            assert!(load_decision_if_present(MISSION_DIRECTORY, &request)
                .expect("decision load")
                .is_none());
        }
        ApprovalIntegrity::Approved {
            request: loaded,
            decision,
        } => {
            assert_eq!(loaded.review_bundle_sha256, request.review_bundle_sha256);
            assert_eq!(decision.status, "approved");
            assert_eq!(decision.decision_actor_role, "orchestrator");
            assert_eq!(decision.request_digest, request.request_digest);
            let required =
                validate_approval_integrity(REPOSITORY, MISSION_DIRECTORY, true)
                    .expect("require-approved must pass for live orchestrator approval");
            match required {
                ApprovalIntegrity::Approved { .. } => {}
                other => panic!("expected approved under require-approved, got {other:?}"),
            }
        }
        other => panic!("expected awaiting_user or approved live integrity, got {other:?}"),
    }
    revalidate_request_against_live_bundle(REPOSITORY, MISSION_DIRECTORY, &request)
        .expect("live revalidation must accept the just-written request");
    let reloaded = load_request(MISSION_DIRECTORY, &request.review_bundle_sha256).unwrap();
    assert_eq!(reloaded.request_digest, request.request_digest);
    let affected = identify_affected_pilot_features(&request, "ready_for_review");
    assert!(affected.iter().any(|item| item.contains("storyboard") || item.contains("gate")));
}

#[test]
fn immutable_request_refuses_content_mutation() {
    let _guard = evidence_lock();
    ensure_programmatic_and_vision_ready();
    let request = write_human_gate_request(REPOSITORY, MISSION_DIRECTORY, Some(1_720_000_000_002))
        .expect("first write");
    let path = Path::new(MISSION_DIRECTORY).join(&request.relative_path);
    // Tamper with on-disk content so load/repair attempts fail closed.
    let original = fs::read_to_string(&path).unwrap();
    let tampered = original.replacen(
        &request.request_digest,
        "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        1,
    );
    // Temporarily make writable then restore for the mutability probe.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o644);
        fs::set_permissions(&path, permissions).unwrap();
    }
    fs::write(&path, &tampered).unwrap();
    let err = write_human_gate_request(REPOSITORY, MISSION_DIRECTORY, Some(1_720_000_000_002))
        .expect_err("tampered request must not be silently rewritten");
    assert!(
        err.contains("immutable")
            || err.contains("mutate")
            || err.contains("new review_bundle")
            || err.contains("content changed")
            || err.contains("stale")
            || err.contains("forged")
            || err.contains("request_digest"),
        "unexpected error: {err}"
    );
    // Restore original immutable request so later tests stay green.
    fs::write(&path, original).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o444);
        fs::set_permissions(&path, permissions).unwrap();
    }
}

#[test]
fn check_gate_approval_shell_reports_awaiting_user() {
    let _guard = evidence_lock();
    ensure_programmatic_and_vision_ready();
    let request = write_human_gate_request(REPOSITORY, MISSION_DIRECTORY, Some(1_720_000_000_003))
        .expect("request must exist for shell checker");
    let harness = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let script = harness.join("check_gate_approval.sh");
    assert!(script.is_file(), "check_gate_approval.sh must exist");
    let root = Path::new(MISSION_DIRECTORY).join(HUMAN_GATE_ROOT);
    let output = Command::new(&script)
        .arg(root.as_os_str())
        .output()
        .expect("shell checker must run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "integrity should succeed without --require-approved\nstdout={stdout}\nstderr={stderr}"
    );
    let combined = format!("{stdout}{stderr}");
    let decision = load_decision_if_present(MISSION_DIRECTORY, &request).expect("decision load");
    match decision {
        None => {
            assert!(
                combined.contains("awaiting_user"),
                "checker should report awaiting_user when no decision exists\n{combined}"
            );
            let blocked = Command::new(&script)
                .arg("--require-approved")
                .arg(root.as_os_str())
                .output()
                .expect("require-approved checker must run");
            assert!(
                !blocked.status.success(),
                "require-approved must fail while decision is missing"
            );
        }
        Some(decision) => {
            assert_eq!(decision.decision_actor_role, "orchestrator");
            assert!(
                combined.contains("approved") || combined.contains("rejected"),
                "checker should report the orchestrator decision status\n{combined}"
            );
            let required = Command::new(&script)
                .arg("--require-approved")
                .arg(root.as_os_str())
                .output()
                .expect("require-approved checker must run for live decision");
            if decision.status == "approved" {
                assert!(
                    required.status.success(),
                    "require-approved must pass for live orchestrator approval\nstdout={}\nstderr={}",
                    String::from_utf8_lossy(&required.stdout),
                    String::from_utf8_lossy(&required.stderr)
                );
            } else {
                assert!(
                    !required.status.success(),
                    "require-approved must fail while decision is rejected"
                );
            }
        }
    }
}

#[test]
fn request_file_name_constants_match_architecture() {
    assert_eq!(REQUEST_FILE_NAME, "request.json");
    assert_eq!(DECISION_FILE_NAME, "decision.json");
    assert_eq!(HUMAN_GATE_ROOT, "evidence/v3-human-gate");
    let _ = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock");
}
