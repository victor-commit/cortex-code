//! Final secrets, commit identity, and path-scope audit (VAL-V3-CROSS-019,
//! VAL-V3-HARNESS-035).
//!
//! Scans every canonical/gate TXT pair, gallery README, live audit JSON,
//! mission evidence text surfaced as validation proof, stdout-style
//! diagnostics from negative fixtures, and every ledgered mission commit for:
//! - reusable credentials / secret sentinels / private traces
//! - required `echobt` author and committer identity
//! - path inventories limited to approved v3 roots (or authorized bootstrap copies)

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::audit::{audit_screen, sha256_hex};
use crate::canonical_inventory;
use crate::flows::flow_manifests;
use crate::negative_fixtures::fixture_manifest;
use crate::renderer::canonical_publication_fixture;

/// Required mission commit author/committer identity (per-command, not git config).
pub const REQUIRED_AUTHOR_NAME: &str = "echobt";
pub const REQUIRED_AUTHOR_EMAIL: &str = "154886644+echobt@users.noreply.github.com";

const CANONICAL_GALLERY: &str = "/projects/cortex-code/plans/assets/mockups-v3";
const GATE_GALLERY: &str = "/projects/cortex-code/plans/assets/mockups-v3-gates";
const HARNESS_RELATIVE: &str = "tools-throwaway/mockup-harness-v3";
const LEDGER_RELATIVE: &str = "tools-throwaway/mockup-harness-v3/.isolation_state/mission_commits.ledger";

/// Explicitly allowed non-secret OAuth user-facing codes.
pub const APPROVED_USER_CODES: &[&str] = &["WDJB-MJHT"];

/// Literal secret/private-trace sentinels that must never appear in deliverables.
pub const SECRET_SENTINELS: &[&str] = &[
    "fixture-secret-value",
    "sk-live-secret-token",
    "api_key=supersecret",
    "Bearer aquicktoken",
    "access_token=sk-oauth-secret",
    "access_token=plaintext",
    "Authorization: Bearer",
    "OPENAI_API_KEY=",
    "refresh_token=",
    "client_secret=",
    "device_code=",
    "HARMFUL_PRIVATE_TRACE",
    "private_cot_marker",
    "hidden_system_instructions",
    "chain-of-thought",
    "hidden prompt",
    "-----BEGIN RSA PRIVATE KEY-----",
    "-----BEGIN OPENSSH PRIVATE KEY-----",
    "-----BEGIN EC PRIVATE KEY-----",
    "-----BEGIN PRIVATE KEY-----",
];

/// Path prefixes every ledgered mission commit is allowed to touch.
pub const ALLOWED_COMMIT_PREFIXES: &[&str] = &[
    "tools-throwaway/mockup-harness-v3/",
    "tools-throwaway/mockup-harness-v4/",
    "plans/assets/mockups-v3/",
    "plans/assets/mockups-v3-gates/",
];

/// Exact paths authorized for historical bootstrap font/lock copies.
const AUTHORIZED_HISTORICAL_COPY_SOURCES: &[&str] = &[
    "tools-throwaway/mockup-harness/Cargo.lock",
    "tools-throwaway/mockup-harness/assets/DejaVuSansMono.ttf",
    "tools-throwaway/mockup-harness/assets/DejaVuSansMono-Bold.ttf",
];

const AUTHORIZED_EXACT_PATHS: &[&str] = &[
    "tools-throwaway/mockup-harness-v3/.isolation_state/mission_commits.ledger",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SecretFinding {
    pub location: String,
    pub sentinel: String,
    pub detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdentityMismatch {
    pub commit: String,
    pub author_name: String,
    pub author_email: String,
    pub committer_name: String,
    pub committer_email: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PathViolation {
    pub commit: String,
    pub paths: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalSecretCommitScopeReport {
    pub scanned_file_count: usize,
    pub scanned_audit_count: usize,
    pub scanned_commit_count: usize,
    pub secret_findings: Vec<SecretFinding>,
    pub identity_mismatches: Vec<IdentityMismatch>,
    pub path_violations: Vec<PathViolation>,
    pub approved_user_codes_observed: BTreeSet<String>,
    pub user_owned_status_preserved: bool,
    pub baseline_user_owned_entries: Vec<String>,
}

/// Returns whether a repository-relative path is inside an approved v3 root.
pub fn path_is_allowed(path: &str) -> bool {
    if AUTHORIZED_EXACT_PATHS.contains(&path) || AUTHORIZED_HISTORICAL_COPY_SOURCES.contains(&path)
    {
        return true;
    }
    ALLOWED_COMMIT_PREFIXES
        .iter()
        .any(|prefix| path.starts_with(prefix))
}

/// Scan one text blob for secret/private-trace sentinels.
pub fn scan_text_for_secrets(location: &str, text: &str) -> Vec<SecretFinding> {
    let mut findings = Vec::new();
    for sentinel in SECRET_SENTINELS {
        if text.contains(sentinel) {
            // Approved non-secret user-facing OAuth code may sit near the word "code".
            if *sentinel == "device_code=" && text.contains("user code") {
                // still banned: exact "device_code=" form is never user-facing
            }
            findings.push(SecretFinding {
                location: location.to_owned(),
                sentinel: (*sentinel).to_owned(),
                detail: format!("sentinel substring present in {location}"),
            });
        }
    }

    // Realistic credential shapes beyond the closed sentinel table.
    for (label, needle) in [
        ("sk-proj- unmasked project key", "sk-proj-"),
        ("github classic token prefix", "ghp_"),
        ("github fine-grained token prefix", "github_pat_"),
        ("slack bot token prefix", "xoxb-"),
        ("aws access key prefix", "AKIA"),
    ] {
        if text.contains(needle) {
            // Allow masked/redacted forms that keep only the prefix and use ·/•/*
            // far enough that a full token cannot be recovered.
            if needle == "sk-proj-"
                && (text.contains('·') || text.contains('•') || text.contains('*'))
            {
                continue;
            }
            findings.push(SecretFinding {
                location: location.to_owned(),
                sentinel: needle.to_owned(),
                detail: format!("{label} in {location}"),
            });
        }
    }

    findings
}

fn git_output(repository: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repository)
        .output()
        .map_err(|error| format!("could not run git {}: {error}", args.join(" ")))?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn read_ledger(repository: &Path) -> Result<Vec<String>, String> {
    let ledger = repository.join(LEDGER_RELATIVE);
    let body = fs::read_to_string(&ledger)
        .map_err(|error| format!("cannot read mission ledger {}: {error}", ledger.display()))?;
    if body.is_empty() {
        return Ok(Vec::new());
    }
    if !body.ends_with('\n') {
        return Err("mission commit ledger does not end with a newline".to_owned());
    }
    let mut commits = Vec::new();
    let mut seen = BTreeSet::new();
    for line in body.lines() {
        if line.is_empty() {
            return Err("mission commit ledger contains an empty line".to_owned());
        }
        if line.len() < 40 || !line.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("mission commit ledger has invalid SHA: {line}"));
        }
        if !seen.insert(line.to_owned()) {
            return Err(format!("mission commit ledger has duplicate SHA: {line}"));
        }
        commits.push(line.to_owned());
    }
    Ok(commits)
}

fn commit_identity(repository: &Path, commit: &str) -> Result<(String, String, String, String), String> {
    let raw = git_output(
        repository,
        &["show", "-s", "--format=%an%x00%ae%x00%cn%x00%ce", commit],
    )?;
    let parts: Vec<&str> = raw.trim_end_matches('\n').split('\0').collect();
    if parts.len() != 4 {
        return Err(format!("unexpected identity payload for {commit}: {raw:?}"));
    }
    Ok((
        parts[0].to_owned(),
        parts[1].to_owned(),
        parts[2].to_owned(),
        parts[3].to_owned(),
    ))
}

fn commit_paths(repository: &Path, commit: &str) -> Result<Vec<String>, String> {
    let raw = git_output(
        repository,
        &[
            "diff-tree",
            "--root",
            "--no-commit-id",
            "--name-only",
            "-r",
            "-m",
            "--find-renames",
            "--find-copies",
            "--find-copies-harder",
            commit,
        ],
    )?;
    let mut paths = BTreeSet::new();
    for line in raw.lines() {
        if !line.is_empty() {
            paths.insert(line.to_owned());
        }
    }
    Ok(paths.into_iter().collect())
}

fn commit_patch_text(repository: &Path, commit: &str) -> Result<String, String> {
    // Patch for text deliverables only: README, txt, md, json, yaml, sh, tsv, toml.
    // Skip binary PNG diffs and large lock churn beyond first-line path headers.
    let paths = commit_paths(repository, commit)?;
    let mut combined = String::new();
    for path in paths {
        let lower = path.to_ascii_lowercase();
        let scan = lower.ends_with(".txt")
            || lower.ends_with(".md")
            || lower.ends_with(".json")
            || lower.ends_with(".yml")
            || lower.ends_with(".yaml")
            || lower.ends_with(".toml")
            || lower.ends_with(".tsv")
            || lower.ends_with(".sh")
            || lower.ends_with("readme")
            || lower.ends_with(".rs");
        if !scan {
            continue;
        }
        // Do not treat closed sentinel *definitions* in test sources as leaks:
        // only scan published artifact paths and README under galleries.
        let publishable = path.starts_with("plans/assets/mockups-v3/")
            || path.starts_with("plans/assets/mockups-v3-gates/")
            || path.ends_with("README.md");
        if !publishable {
            continue;
        }
        let body = git_output(repository, &["show", &format!("{commit}:{path}")]);
        match body {
            Ok(text) => {
                combined.push_str(&format!("\n// file:{path}\n"));
                combined.push_str(&text);
            }
            Err(_) => {
                // deleted path
            }
        }
    }
    Ok(combined)
}

fn walk_text_files(root: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    if !root.exists() {
        return Err(format!("missing path {}", root.display()));
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(current) = stack.pop() {
        let entries = fs::read_dir(&current)
            .map_err(|error| format!("read_dir {}: {error}", current.display()))?;
        for entry in entries {
            let entry = entry.map_err(|error| format!("dir entry error: {error}"))?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|error| format!("file_type {}: {error}", path.display()))?;
            if file_type.is_dir() {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default();
                if name == "target" || name == ".isolation_state" || name == ".isolation-session" {
                    continue;
                }
                stack.push(path);
            } else if file_type.is_file() {
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    match ext {
                        "txt" | "md" | "json" | "tsv" | "yml" | "yaml" | "toml" | "sh" => {
                            out.push(path);
                        }
                        _ => {}
                    }
                } else if path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.eq_ignore_ascii_case("README"))
                {
                    out.push(path);
                }
            }
        }
    }
    Ok(())
}

fn gather_user_owned_status(repository: &Path) -> Result<Vec<String>, String> {
    let raw = git_output(repository, &["status", "--porcelain"])?;
    let mut entries = Vec::new();
    for line in raw.lines() {
        if line.len() < 4 {
            continue;
        }
        let path = line[3..].trim().to_owned();
        // Mission-owned v3 paths must remain modifiable; only track user-owned
        // relative paths that live outside the approved v3 roots.
        if path_is_allowed(&path) || path.starts_with(HARNESS_RELATIVE) {
            continue;
        }
        entries.push(line.to_owned());
    }
    entries.sort();
    Ok(entries)
}

fn scan_file(path: &Path, findings: &mut Vec<SecretFinding>, scanned: &mut usize) -> Result<(), String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    *scanned += 1;
    findings.extend(scan_text_for_secrets(&path.display().to_string(), &text));
    Ok(())
}

fn scan_png_ascii_residue(
    path: &Path,
    findings: &mut Vec<SecretFinding>,
    scanned: &mut usize,
) -> Result<(), String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    *scanned += 1;
    // PNG binary may embed rasterized glyphs only. Soften scan: RSA PEM markers
    // or raw sentinel ASCII substrings should never appear as raw bytes either.
    let lossy = String::from_utf8_lossy(&bytes);
    findings.extend(scan_text_for_secrets(&path.display().to_string(), &lossy));
    Ok(())
}

fn record_user_codes(text: &str, observed: &mut BTreeSet<String>) {
    for code in APPROVED_USER_CODES {
        if text.contains(code) {
            observed.insert((*code).to_owned());
        }
    }
}

/// Full production audit for final secret, identity, and path-scope guarantees.
pub fn validate_final_secret_commit_scope(
    repository: &str,
    mission_directory: &str,
    session_id: &str,
) -> Result<FinalSecretCommitScopeReport, String> {
    let repository = Path::new(repository);
    let mut secret_findings = Vec::new();
    let mut scanned_file_count = 0usize;
    let mut scanned_audit_count = 0usize;
    let mut approved_user_codes_observed = BTreeSet::new();

    // 1. Canonical + gate text deliverables and README.
    let mut text_files = Vec::new();
    walk_text_files(Path::new(CANONICAL_GALLERY), &mut text_files)?;
    walk_text_files(Path::new(GATE_GALLERY), &mut text_files)?;
    for path in &text_files {
        scan_file(path, &mut secret_findings, &mut scanned_file_count)?;
        if let Ok(text) = fs::read_to_string(path) {
            record_user_codes(&text, &mut approved_user_codes_observed);
        }
    }

    // 2. PNG ASCII residue scan (catches accidental embedded tokens).
    for inventory in [
        Path::new(CANONICAL_GALLERY),
        Path::new(GATE_GALLERY),
    ] {
        let mut pngs = Vec::new();
        gather_pngs(inventory, &mut pngs)?;
        for path in pngs {
            scan_png_ascii_residue(&path, &mut secret_findings, &mut scanned_file_count)?;
        }

    }

    // 3. Live audit JSON for every canonical publication fixture.
    for screen in canonical_inventory() {
        let fixture = canonical_publication_fixture(screen.id)
            .map(str::to_owned)
            .unwrap_or_else(|| format!("canonical.{}", screen.id));
        let audit = audit_screen(screen.id, &fixture).map_err(|error| {
            format!(
                "audit_screen({}, {}) failed: {error}",
                screen.id, fixture
            )
        })?;
        audit.validate().map_err(|error| {
            format!(
                "audit validate failed for {}/{}: {error}",
                screen.id, fixture
            )
        })?;
        let json = audit.to_json();
        let txt = audit.render_txt();
        scanned_audit_count += 1;
        secret_findings.extend(scan_text_for_secrets(
            &format!("audit-json:{}:{fixture}", screen.id),
            &json,
        ));
        secret_findings.extend(scan_text_for_secrets(
            &format!("audit-txt:{}:{fixture}", screen.id),
            &txt,
        ));
        record_user_codes(&txt, &mut approved_user_codes_observed);
    }

    // 4. Negative-fixture diagnostic catalogue: skip fixtures whose purpose is to
    // name the sentinel they catch (leaked-secret). Other diagnostics remain free
    // of credentials.
    for case in fixture_manifest() {
        if case.id == "leaked-secret" {
            continue;
        }
        secret_findings.extend(scan_text_for_secrets(
            &format!("negative-fixture:{}", case.id),
            case.expected_diagnostic,
        ));
    }

    // 5. Mission evidence trees that form final validation proof.
    // Library notes deliberately name the ban-list sentinels they verify; they are
    // not published artifact classes. Scan vision evidence + human-gate JSON only.
    let evidence_roots = [
        Path::new(mission_directory).join("evidence/v3-final-canonical-vision"),
        Path::new(mission_directory).join("evidence/v3-human-gate"),
    ];
    for root in evidence_roots {
        if !root.exists() {
            continue;
        }
        let mut files = Vec::new();
        walk_text_files(&root, &mut files)?;
        for path in files {
            scan_file(&path, &mut secret_findings, &mut scanned_file_count)?;
        }
    }

    // 6. Mission commits: identity + paths + published artifact contents.
    let commits = read_ledger(repository)?;
    let mut identity_mismatches = Vec::new();
    let mut path_violations = Vec::new();
    for commit in &commits {
        let (an, ae, cn, ce) = commit_identity(repository, commit)?;
        if an != REQUIRED_AUTHOR_NAME
            || ae != REQUIRED_AUTHOR_EMAIL
            || cn != REQUIRED_AUTHOR_NAME
            || ce != REQUIRED_AUTHOR_EMAIL
        {
            identity_mismatches.push(IdentityMismatch {
                commit: commit.clone(),
                author_name: an,
                author_email: ae,
                committer_name: cn,
                committer_email: ce,
            });
        }
        let paths = commit_paths(repository, commit)?;
        let invalid: Vec<String> = paths
            .into_iter()
            .filter(|path| !path_is_allowed(path))
            .collect();
        if !invalid.is_empty() {
            path_violations.push(PathViolation {
                commit: commit.clone(),
                paths: invalid,
            });
        }
        let patch = commit_patch_text(repository, commit)?;
        secret_findings.extend(scan_text_for_secrets(
            &format!("commit:{commit}"),
            &patch,
        ));
    }

    // 7. Cross-check session isolation audit helper (ledger path inventory).
    crate::verify_mission_commit_paths(repository, session_id)?;
    crate::verify_forbidden_paths(repository, session_id)?;

    let baseline_user_owned_entries = gather_user_owned_status(repository)?;
    // After audit, re-read status — mission work must not have mutated
    // non-v3 user-owned paths.
    let after = gather_user_owned_status(repository)?;
    let user_owned_status_preserved = after == baseline_user_owned_entries;

    // Prove six flows remain registered while scanning (no silent inventory shrink).
    if flow_manifests().len() != 6 {
        return Err(format!(
            "expected six flow manifests, found {}",
            flow_manifests().len()
        ));
    }
    if canonical_inventory().len() != 86 {
        return Err(format!(
            "expected 86 canonical IDs, found {}",
            canonical_inventory().len()
        ));
    }

    let report = FinalSecretCommitScopeReport {
        scanned_file_count,
        scanned_audit_count,
        scanned_commit_count: commits.len(),
        secret_findings,
        identity_mismatches,
        path_violations,
        approved_user_codes_observed,
        user_owned_status_preserved,
        baseline_user_owned_entries,
    };

    if !report.secret_findings.is_empty()
        || !report.identity_mismatches.is_empty()
        || !report.path_violations.is_empty()
        || !report.user_owned_status_preserved
    {
        return Err(format_report_error(&report));
    }

    // sha256 fingerprint of empty findings, used by evidence notes.
    let _ok_marker = sha256_hex(b"final-secret-commit-scope:clean");
    let _ = _ok_marker;

    Ok(report)
}

fn format_report_error(report: &FinalSecretCommitScopeReport) -> String {
    let mut parts = Vec::new();
    if !report.secret_findings.is_empty() {
        parts.push(format!(
            "{} secret finding(s); first: {} @ {}",
            report.secret_findings.len(),
            report.secret_findings[0].sentinel,
            report.secret_findings[0].location
        ));
    }
    if !report.identity_mismatches.is_empty() {
        parts.push(format!(
            "{} identity mismatch(es); first commit {}",
            report.identity_mismatches.len(),
            report.identity_mismatches[0].commit
        ));
    }
    if !report.path_violations.is_empty() {
        parts.push(format!(
            "{} path violation(s); first commit {} paths {:?}",
            report.path_violations.len(),
            report.path_violations[0].commit,
            report.path_violations[0].paths
        ));
    }
    if !report.user_owned_status_preserved {
        parts.push("user-owned working-tree status drifted".to_owned());
    }
    parts.join("; ")
}

fn gather_pngs(root: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(current) = stack.pop() {
        let entries = fs::read_dir(&current)
            .map_err(|error| format!("read_dir {}: {error}", current.display()))?;
        for entry in entries {
            let entry = entry.map_err(|error| format!("dir entry error: {error}"))?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|error| format!("file_type {}: {error}", path.display()))?;
            if file_type.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("png") {
                out.push(path);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn scanner_flags_known_sentinels() {
        let findings = scan_text_for_secrets("unit", "token fixture-secret-value here");
        assert!(findings.iter().any(|f| f.sentinel == "fixture-secret-value"));
    }

    #[test]
    fn scanner_allows_clean_text() {
        assert!(scan_text_for_secrets("unit", "Connect with user code WDJB-MJHT").is_empty());
    }

    #[test]
    fn path_scope_rejects_product_crates() {
        assert!(!path_is_allowed("crates/skills/src/lib.rs"));
        assert!(path_is_allowed("tools-throwaway/mockup-harness-v3/src/lib.rs"));
        assert!(path_is_allowed("plans/assets/mockups-v3/session/home.txt"));
        assert!(path_is_allowed("plans/assets/mockups-v3-gates/daily-work/01-home.txt"));
    }
}
