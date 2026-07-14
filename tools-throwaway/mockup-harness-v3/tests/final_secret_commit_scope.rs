//! Final secrets, commit identity, and path-scope audit.
//!
//! Owned contract assertions:
//! - VAL-V3-CROSS-019 Secrets never enter deliverables
//! - VAL-V3-HARNESS-035 Commit scope and identity are enforced

use std::env;
use std::path::PathBuf;
use std::process::Command;

use mockup_harness_v3::final_secret_commit_scope::{
    path_is_allowed, scan_text_for_secrets, validate_final_secret_commit_scope,
    APPROVED_USER_CODES, REQUIRED_AUTHOR_EMAIL, REQUIRED_AUTHOR_NAME, SECRET_SENTINELS,
};

const REPOSITORY: &str = "/projects/cortex-code";
const MISSION_DIRECTORY: &str = "/root/.factory/missions/7060a79b-ccfb-445c-9e82-ee4504b320dc";

fn session_id() -> String {
    if let Ok(id) = env::var("V3_ISOLATION_SESSION_ID") {
        let id = id.trim().to_owned();
        if !id.is_empty() {
            return id;
        }
    }
    // Mission-owned active session from start_session_snapshot.sh, via the
    // runtime settings directory (FACTORY_RUNTIME_SETTINGS_PATH).
    let candidates = [
        env::var("FACTORY_RUNTIME_SETTINGS_PATH")
            .ok()
            .map(|settings| {
                PathBuf::from(settings)
                    .parent()
                    .map(|parent| parent.join("isolation-guard-v3/current-session"))
                    .unwrap_or_else(|| PathBuf::from("isolation-guard-v3/current-session"))
            }),
        Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".isolation_state/current-session")),
        Some(
            PathBuf::from("/root/.factory/missions/7060a79b-ccfb-445c-9e82-ee4504b320dc")
                .join("isolation-guard-v3/current-session"),
        ),
    ];
    for candidate in candidates.into_iter().flatten() {
        if let Ok(raw) = std::fs::read_to_string(&candidate) {
            let id = raw.trim().to_owned();
            if !id.is_empty() {
                return id;
            }
        }
    }
    panic!("V3_ISOLATION_SESSION_ID or isolation-guard-v3/current-session must exist");
}

#[test]
fn secret_scanner_unit_table_covers_required_classes() {
    // Credentials, tokens, private keys, private traces, and fixture sentinels.
    for required in [
        "fixture-secret-value",
        "sk-live-secret-token",
        "device_code=",
        "Authorization: Bearer",
        "-----BEGIN PRIVATE KEY-----",
        "HARMFUL_PRIVATE_TRACE",
        "chain-of-thought",
        "hidden prompt",
    ] {
        assert!(
            SECRET_SENTINELS.contains(&required),
            "ban list missing {required}"
        );
        let findings = scan_text_for_secrets("unit", &format!("prefix {required} suffix"));
        assert!(
            findings.iter().any(|f| f.sentinel == required),
            "scanner must report {required}"
        );
    }
    assert!(APPROVED_USER_CODES.contains(&"WDJB-MJHT"));
    assert!(scan_text_for_secrets("unit", "Enter code WDJB-MJHT to authorize").is_empty());
    assert_eq!(REQUIRED_AUTHOR_NAME, "echobt");
    assert_eq!(
        REQUIRED_AUTHOR_EMAIL,
        "154886644+echobt@users.noreply.github.com"
    );
}

#[test]
fn path_scope_allows_only_v3_and_authorized_history() {
    assert!(path_is_allowed("tools-throwaway/mockup-harness-v3/src/lib.rs"));
    assert!(path_is_allowed("plans/assets/mockups-v3/README.md"));
    assert!(path_is_allowed(
        "plans/assets/mockups-v3-gates/daily-work/01-home.txt"
    ));
    assert!(path_is_allowed(
        "tools-throwaway/mockup-harness/assets/DejaVuSansMono.ttf"
    ));
    assert!(!path_is_allowed("crates/skills/src/lib.rs"));
    assert!(!path_is_allowed("tools-throwaway/mockup-harness/src/main.rs"));
    assert!(!path_is_allowed("README.md"));
    assert!(!path_is_allowed("research/05-gap-analysis.md"));
}

#[test]
fn secrets_identity_and_paths_are_clean_across_deliverables() {
    let session = session_id();
    let report = validate_final_secret_commit_scope(REPOSITORY, MISSION_DIRECTORY, &session)
        .expect("final secret/identity/path audit must pass with zero unresolved findings");

    assert!(
        report.scanned_file_count >= 172,
        "must scan full gallery text+png inventory; got {}",
        report.scanned_file_count
    );
    assert_eq!(
        report.scanned_audit_count, 86,
        "every canonical screen receives an audit JSON scan"
    );
    assert!(
        report.scanned_commit_count >= 1,
        "mission ledger must be non-empty"
    );
    assert!(
        report.secret_findings.is_empty(),
        "secret findings: {:?}",
        report.secret_findings
    );
    assert!(
        report.identity_mismatches.is_empty(),
        "identity mismatches: {:?}",
        report.identity_mismatches
    );
    assert!(
        report.path_violations.is_empty(),
        "path violations: {:?}",
        report.path_violations
    );
    assert!(
        report.user_owned_status_preserved,
        "user-owned working tree drifted: {:?}",
        report.baseline_user_owned_entries
    );
    // Unrelated baseline dirt still present and untouched.
    let status = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(REPOSITORY)
        .output()
        .expect("git status");
    let porcelain = String::from_utf8_lossy(&status.stdout);
    assert!(
        porcelain.contains("crates/skills/src/lib.rs")
            || porcelain.contains(".gitignore")
            || porcelain.lines().any(|line| {
                let path = line.get(3..).unwrap_or("").trim();
                !path.starts_with("tools-throwaway/mockup-harness-v3/")
                    && !path.starts_with("plans/assets/mockups-v3")
            }),
        "baseline user-owned dirt must remain visible and untouched: {porcelain}"
    );
}

#[test]
fn leaked_secret_negative_fixture_still_fail_closed() {
    let output = Command::new(env!("CARGO_BIN_EXE_mockup-harness-v3"))
        .args(["negative-fixture", "leaked-secret"])
        .output()
        .expect("harness must run");
    // Some harness builds exit 3 for validation; accept known fail-closed codes.
    let code = output.status.code().unwrap_or(0);
    assert!(
        code == 3 || code == 1,
        "leaked-secret fixture must fail closed, got {code}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("leaked secret") || combined.contains("fixture-secret-value"),
        "diagnostic must name the extract: {combined}"
    );
}
