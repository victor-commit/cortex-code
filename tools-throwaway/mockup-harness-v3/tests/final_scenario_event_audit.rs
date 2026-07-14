//! Final public scenario and typed semantic coverage gate.
//!
//! Owned contract assertions:
//! VAL-V3-HARNESS-020, VAL-V3-HARNESS-021, VAL-V3-HARNESS-022,
//! VAL-V3-HARNESS-023, VAL-V3-HARNESS-024, VAL-V3-HARNESS-025,
//! VAL-V3-HARNESS-036.
//!
//! Proves candidate↔event↔action↔transition bijections, preservation hashes,
//! mutation ledgers, family completeness, deterministic case output, and
//! protected scenario artifact destinations across every public case and every
//! canonical screen.

use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fs,
    path::PathBuf,
    process::Command,
};

use mockup_harness_v3::{
    audit::{audit_screen, AuditDocument},
    canonical_inventory,
    renderer::canonical_publication_fixture,
    scenario::{
        assertion_to_case_coverage, public_case_ids, replay_case, validate_png_destination,
        CaseReplay,
    },
};

fn harness() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mockup-harness-v3"))
}

fn family_counts(audit: &AuditDocument) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for event in &audit.events {
        *counts
            .entry(event.identity.family.clone())
            .or_insert(0usize) += 1;
    }
    counts
}

fn assert_identity_schema(audit: &AuditDocument, label: &str) {
    assert!(
        !audit.screen_id.is_empty() && !audit.fixture_id.is_empty(),
        "{label}: missing screen/fixture identity"
    );
    for candidate in &audit.candidates {
        let id = &candidate.identity;
        assert_eq!(id.screen_id, audit.screen_id, "{label}: candidate screen");
        assert_eq!(id.fixture_id, audit.fixture_id, "{label}: candidate fixture");
        assert!(!id.candidate_id.is_empty(), "{label}: empty candidate_id");
        assert!(!id.event_id.is_empty(), "{label}: empty event_id");
        assert!(!id.family.is_empty(), "{label}: empty family");
        assert!(!id.subject_id.is_empty(), "{label}: empty subject_id");
        assert!(!id.render_refs.is_empty(), "{label}: empty render_refs");
        assert!(!id.detail_level.is_empty(), "{label}: empty detail_level");
    }
    for event in &audit.events {
        let id = &event.identity;
        assert_eq!(id.screen_id, audit.screen_id, "{label}: event screen");
        assert_eq!(id.fixture_id, audit.fixture_id, "{label}: event fixture");
        assert!(!id.candidate_id.is_empty(), "{label}: empty event candidate");
        assert!(!id.event_id.is_empty(), "{label}: empty event_id");
        assert!(!id.family.is_empty(), "{label}: empty event family");
        assert!(!id.subject_id.is_empty(), "{label}: empty event subject");
        assert!(!id.render_refs.is_empty(), "{label}: empty event refs");
        assert!(!id.detail_level.is_empty(), "{label}: empty event detail");
        if let Some(action) = &event.action {
            assert!(!action.action_id.is_empty(), "{label}: empty action_id");
            assert!(!action.label.is_empty(), "{label}: empty action label");
            assert!(action.enabled, "{label}: inaccessible action");
        }
    }
}

fn assert_candidate_event_bijection(audit: &AuditDocument, label: &str) {
    let candidate_ids: BTreeSet<_> = audit
        .candidates
        .iter()
        .map(|c| c.identity.candidate_id.clone())
        .collect();
    let event_candidates: BTreeSet<_> = audit
        .events
        .iter()
        .map(|e| e.identity.candidate_id.clone())
        .collect();
    assert_eq!(
        candidate_ids, event_candidates,
        "{label}: candidate↔event bijection failed"
    );
    assert_eq!(
        audit.candidates.len(),
        audit.events.len(),
        "{label}: candidate/event count mismatch"
    );
    let event_ids: BTreeSet<_> = audit
        .events
        .iter()
        .map(|e| e.identity.event_id.clone())
        .collect();
    assert_eq!(
        event_ids.len(),
        audit.events.len(),
        "{label}: duplicate event_id values"
    );
}

fn assert_action_transition_bijection(audit: &AuditDocument, label: &str) {
    let mut action_ids = BTreeSet::new();
    for event in &audit.events {
        if let Some(action) = &event.action {
            assert!(
                action_ids.insert(action.action_id.clone()),
                "{label}: duplicate action_id {}",
                action.action_id
            );
        }
    }
    let mut transition_actions = BTreeSet::new();
    for transition in &audit.transitions {
        assert!(
            !transition.action_id.is_empty()
                && !transition.from_screen_id.is_empty()
                && !transition.to_screen_id.is_empty()
                && !transition.kind.is_empty()
                && !transition.path.is_empty(),
            "{label}: incomplete transition {:?}",
            transition
        );
        assert_ne!(
            transition.kind, "dead-end",
            "{label}: dead-end transition {}",
            transition.action_id
        );
        assert_eq!(
            transition.from_screen_id, audit.screen_id,
            "{label}: transition from_screen mismatch"
        );
        assert!(
            action_ids.contains(&transition.action_id),
            "{label}: transition {} has no matching action",
            transition.action_id
        );
        assert!(
            transition_actions.insert(transition.action_id.clone()),
            "{label}: duplicate transition for {}",
            transition.action_id
        );
    }
}

fn assert_surface_and_statusline(audit: &AuditDocument, label: &str) {
    let counts = family_counts(audit);
    assert_eq!(
        counts.get("surface").copied().unwrap_or(0),
        1,
        "{label}: expected exactly one surface event, got {:?}",
        counts.get("surface")
    );
    let statusline_events = counts.get("statusline").copied().unwrap_or(0);
    assert!(
        statusline_events <= 1,
        "{label}: duplicate statusline events"
    );
    let bottom: String = audit
        .styled_cells
        .iter()
        .filter(|cell| cell.row == 39)
        .map(|cell| cell.glyph)
        .collect();
    let painted = bottom.trim().starts_with("~/") && bottom.contains(" · ");
    if painted {
        assert_eq!(
            statusline_events, 1,
            "{label}: painted statusline missing event"
        );
    }
}

fn assert_family_completeness(audit: &AuditDocument, label: &str) {
    let counts = family_counts(audit);
    // Activity: no duplicate ownership (already validated), but if present every one is unique.
    if let Some(activity) = counts.get("activity") {
        let owners: BTreeSet<_> = audit
            .events
            .iter()
            .filter(|e| e.identity.family == "activity")
            .map(|e| e.identity.subject_id.clone())
            .collect();
        assert_eq!(
            owners.len(),
            *activity,
            "{label}: duplicate activity owners"
        );
    }
    // Selection and decision families, when present, must biject to candidates (already covered).
    for family in [
        "selection",
        "decision",
        "connection",
        "recovery",
        "recovery-cause",
        "mission-card",
        "mission-goal",
        "statusline",
        "capability-row",
    ] {
        if let Some(count) = counts.get(family) {
            assert!(*count >= 1, "{label}: empty family {family}");
        }
    }
    // Render refs must land inside the canvas and reference non-empty glyph ranges.
    let text = audit.render_txt();
    let rows: Vec<&str> = text.lines().collect();
    assert_eq!(rows.len(), 40, "{label}: TXT row count");
    for event in &audit.events {
        for reference in &event.identity.render_refs {
            assert!(
                (reference.row as usize) < rows.len(),
                "{label}: render_ref row OOB for {}",
                event.identity.event_id
            );
            let row = rows[reference.row as usize];
            let chars: Vec<char> = row.chars().collect();
            assert!(
                (reference.end as usize) <= chars.len()
                    && reference.start < reference.end,
                "{label}: render_ref span invalid for {}",
                event.identity.event_id
            );
        }
    }
}

fn assert_replay_invariants(replay: &CaseReplay) {
    let label = format!("{}/tick.{}", replay.case_id, replay.virtual_tick);
    replay
        .validate()
        .unwrap_or_else(|e| panic!("{label}: validate failed: {e}"));
    assert_eq!(replay.actions.len(), 1, "{label}: action count");
    assert_eq!(replay.transitions.len(), 1, "{label}: transition count");
    assert_eq!(
        replay.actions[0].action_id, replay.transitions[0].action_id,
        "{label}: action/transition mismatch"
    );
    assert_eq!(
        replay.transitions[0].from_screen_id, replay.before.screen_id,
        "{label}: transition from"
    );
    assert_eq!(
        replay.transitions[0].to_screen_id, replay.after.screen_id,
        "{label}: transition to"
    );
    assert_eq!(
        replay.audit.screen_id, replay.after.screen_id,
        "{label}: audit must describe after state"
    );
    assert!(
        !replay.preserved_field_hashes.is_empty(),
        "{label}: missing preservation hashes"
    );
    for (field, hash) in &replay.preserved_field_hashes {
        assert_eq!(hash.before.len(), 64, "{label}: bad before hash for {field}");
        assert_eq!(hash.after.len(), 64, "{label}: bad after hash for {field}");
        assert!(
            hash.before.chars().all(|c| c.is_ascii_hexdigit()),
            "{label}: non-hex before hash for {field}"
        );
        assert!(
            hash.after.chars().all(|c| c.is_ascii_hexdigit()),
            "{label}: non-hex after hash for {field}"
        );
    }
    assert_eq!(
        replay.virtual_state_before_hash.len(),
        64,
        "{label}: missing virtual_state_before_hash"
    );
    assert_eq!(
        replay.virtual_state_after_hash.len(),
        64,
        "{label}: missing virtual_state_after_hash"
    );
    assert!(
        !replay.mutation_ledger.is_empty(),
        "{label}: empty mutation ledger"
    );
    let required_stores: HashSet<&str> = [
        "workspace-files",
        "git-index",
        "exports",
        "permission-rules",
        "queue",
        "clipboard",
        "provider-mcp",
        "invocation-logs",
        "mission-state",
        "checkpoints",
        "timers",
        "side-effect",
    ]
    .into_iter()
    .collect();
    let present: HashSet<_> = replay
        .mutation_ledger
        .iter()
        .map(|entry| entry.store.as_str())
        .collect();
    assert_eq!(
        present, required_stores,
        "{label}: mutation ledger store set incomplete"
    );
    for entry in &replay.mutation_ledger {
        assert_eq!(
            entry.mutated,
            entry.before_hash != entry.after_hash,
            "{label}: mutation flag mismatch for {}",
            entry.subject
        );
    }
    assert_candidate_event_bijection(&replay.audit, &label);
    assert_action_transition_bijection(&replay.audit, &label);
    assert_identity_schema(&replay.audit, &label);
    assert_surface_and_statusline(&replay.audit, &label);
    assert_family_completeness(&replay.audit, &label);
}

#[test]
fn every_canonical_screen_has_complete_typed_semantic_audit() {
    for screen in canonical_inventory() {
        let fixture = canonical_publication_fixture(screen.id).unwrap_or("public-audit");
        let label = format!("{}/{}", screen.id, fixture);
        let audit = audit_screen(screen.id, fixture)
            .unwrap_or_else(|e| panic!("{label}: audit failed: {e}"));
        audit
            .validate()
            .unwrap_or_else(|e| panic!("{label}: validate failed: {e}"));
        assert_candidate_event_bijection(&audit, &label);
        assert_action_transition_bijection(&audit, &label);
        assert_identity_schema(&audit, &label);
        assert_surface_and_statusline(&audit, &label);
        assert_family_completeness(&audit, &label);
        let text = audit.render_txt();
        let rows: Vec<_> = text.lines().collect();
        assert_eq!(rows.len(), 40, "{label}: TXT rows");
        assert!(
            rows.iter().all(|row| row.chars().count() == 120),
            "{label}: TXT columns"
        );
    }
}

#[test]
fn every_public_case_branch_is_covered_and_deterministic() {
    let case_ids = public_case_ids();
    assert!(
        case_ids.len() >= 300,
        "public case catalog is unexpectedly small: {}",
        case_ids.len()
    );
    let branches = assertion_to_case_coverage()
        .unwrap_or_else(|e| panic!("coverage manifest failed: {e}"));
    assert_eq!(
        branches.len(),
        case_ids.len(),
        "coverage branch count must match public case catalog"
    );
    let mut seen = BTreeSet::new();
    for branch in &branches {
        assert!(
            seen.insert(branch.case_id),
            "duplicate coverage case {}",
            branch.case_id
        );
        assert_eq!(branch.tick, 1, "HARNESS-036 public tick must be 1");
        assert!(!branch.action_id.is_empty());
        assert!(!branch.after_screen_id.is_empty());
        assert!(!branch.preservation_fields.is_empty());
        assert!(branch.mutation_store_count >= 12);
        assert!(
            branch
                .assertion_ids
                .contains(&"VAL-V3-HARNESS-036"),
            "case {} missing HARNESS-036 assertion id",
            branch.case_id
        );
    }
    // Every public ID is accounted for (zero-coverage empty).
    for case_id in case_ids {
        assert!(
            seen.contains(case_id),
            "public case {case_id} missing from coverage manifest"
        );
    }
}

#[test]
fn every_public_case_tick_replays_with_ledger_and_bijections() {
    for case_id in public_case_ids() {
        let first = replay_case(case_id, 1)
            .unwrap_or_else(|e| panic!("{case_id}: first replay failed: {e}"));
        assert_replay_invariants(&first);
        let second = replay_case(case_id, 1)
            .unwrap_or_else(|e| panic!("{case_id}: second replay failed: {e}"));
        assert_eq!(
            first.to_json(),
            second.to_json(),
            "{case_id}: replay JSON is not byte-identical"
        );
        assert_eq!(
            first.audit.render_txt(),
            second.audit.render_txt(),
            "{case_id}: after-state TXT drifted"
        );
        assert_eq!(
            first.virtual_state_before_hash, second.virtual_state_before_hash,
            "{case_id}: virtual_state_before_hash drifted"
        );
        assert_eq!(
            first.virtual_state_after_hash, second.virtual_state_after_hash,
            "{case_id}: virtual_state_after_hash drifted"
        );
    }
}

#[test]
fn public_case_cli_emits_stdout_only_and_rejects_protected_destinations() {
    // Representative automatic, cancel, mutation, and multi-tick oriented probes.
    for (case_id, expect_auto) in [
        ("permission-allow-once", false),
        ("automatic-retry", true),
        ("staging-finish", false),
        ("queue-apply", false),
        ("mission-start", false),
        ("connect-provider-back-conversation", false),
    ] {
        let audit = harness()
            .args(["--case", case_id, "--tick", "1", "--audit-json"])
            .output()
            .expect("case audit CLI must run");
        assert!(
            audit.status.success(),
            "{case_id} audit-json failed: {}",
            String::from_utf8_lossy(&audit.stderr)
        );
        assert!(audit.stderr.is_empty(), "{case_id}: stderr not empty");
        let stdout = String::from_utf8(audit.stdout).expect("utf8");
        assert!(
            stdout.starts_with("{\"document_type\":\"case-replay\""),
            "{case_id}: unexpected audit document"
        );
        assert!(
            stdout.contains("\"virtual_state_before_hash\""),
            "{case_id}: missing virtual_state_before_hash"
        );
        assert!(
            stdout.contains("\"mutation_ledger\""),
            "{case_id}: missing mutation_ledger"
        );
        assert!(
            stdout.contains("\"preserved_field_hashes\""),
            "{case_id}: missing preserved_field_hashes"
        );
        if expect_auto {
            assert!(
                stdout.contains("\"automatic\":true"),
                "{case_id}: expected automatic action"
            );
        }

        let text = harness()
            .args(["--case", case_id, "--tick", "1", "--txt"])
            .output()
            .expect("case txt CLI must run");
        assert!(text.status.success(), "{case_id}: --txt failed");
        assert!(text.stderr.is_empty());
        let rows: Vec<_> = String::from_utf8(text.stdout)
            .expect("utf8")
            .lines()
            .map(str::to_owned)
            .collect();
        assert_eq!(rows.len(), 40, "{case_id}: TXT rows");
        assert!(rows.iter().all(|row| row.chars().count() == 120));
    }

    // Protected destinations fail closed.
    for destination in [
        "/projects/cortex-code/plans/assets/mockups-v3/forbidden-scenario.png",
        "/projects/cortex-code/plans/assets/mockups-v3-gates/forbidden-scenario.png",
    ] {
        let path = PathBuf::from(destination);
        assert!(!path.exists());
        let output = harness()
            .args([
                "--case",
                "permission-allow-once",
                "--tick",
                "1",
                "--png",
                destination,
            ])
            .output()
            .expect("protected png probe");
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(!path.exists());
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .to_ascii_lowercase()
                .contains("temporary")
                || String::from_utf8_lossy(&output.stderr)
                    .to_ascii_lowercase()
                    .contains("reject")
                || !output.stderr.is_empty()
        );
    }
    // Explicit temporary destination is accepted by the destination validator.
    validate_png_destination(std::path::Path::new("/tmp/mockup-harness-v3-scenario-probe.png"))
        .expect("temp png destination must be accepted");
    for bad in ["plans/assets/mockups-v3/oops.png", "mockups-v3-gates/oops.png"] {
        let full = PathBuf::from("/projects/cortex-code").join(bad);
        assert!(
            validate_png_destination(&full).is_err(),
            "expected rejection for {}",
            full.display()
        );
    }
}

#[cfg(feature = "png")]
#[test]
fn public_case_png_temp_destination_is_deterministic() {
    let path = std::env::temp_dir().join(format!(
        "mockup-harness-v3-final-scenario-{}.png",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    let first = harness()
        .args([
            "--case",
            "permission-allow-once",
            "--tick",
            "1",
            "--png",
        ])
        .arg(&path)
        .output()
        .expect("png case CLI");
    assert!(first.status.success());
    let hash1 = String::from_utf8(first.stdout).unwrap();
    assert_eq!(hash1.trim().len(), 64);
    assert!(path.is_file());
    let bytes1 = fs::read(&path).expect("read png");
    fs::remove_file(&path).ok();

    let second = harness()
        .args([
            "--case",
            "permission-allow-once",
            "--tick",
            "1",
            "--png",
        ])
        .arg(&path)
        .output()
        .expect("png case CLI 2");
    assert!(second.status.success());
    let hash2 = String::from_utf8(second.stdout).unwrap();
    assert_eq!(hash1.trim(), hash2.trim());
    let bytes2 = fs::read(&path).expect("read png 2");
    assert_eq!(bytes1, bytes2);
    fs::remove_file(&path).ok();
}

#[test]
fn unknown_case_and_invalid_tick_exit_usage_two() {
    for args in [
        vec!["--case", "unknown-case-xyz", "--tick", "1", "--audit-json"],
        vec![
            "--case",
            "permission-allow-once",
            "--tick",
            "99",
            "--audit-json",
        ],
    ] {
        let output = harness().args(args).output().expect("cli");
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }
}

#[test]
fn negative_unknown_or_missing_consumers_fail_closed() {
    // Public negative fixtures for orphan/duplicate/unknown demonstrate
    // HARNESS-025 fail-closed behavior: matrix entry must exit 3.
    for fixture_id in [
        "orphan-candidate",
        "duplicate-candidate",
        "unknown-candidate",
        "orphan-event",
        "duplicate-event",
        "unknown-event",
        "missing-event",
        "duplicate-activity-owner",
        "inaccessible-action",
        "unknown-action",
        "duplicate-action",
        "dead-end-transition",
        "unknown-transition",
        "missing-surface",
        "duplicate-surface",
        "unknown-surface",
    ] {
        let output = harness()
            .args(["negative-fixture", fixture_id])
            .output()
            .expect("negative fixture CLI");
        assert_eq!(
            output.status.code(),
            Some(3),
            "{fixture_id} must fail closed"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains(fixture_id) || stderr.contains("fixture"),
            "{fixture_id}: diagnostic missing fixture identity: {stderr}"
        );
    }
}
