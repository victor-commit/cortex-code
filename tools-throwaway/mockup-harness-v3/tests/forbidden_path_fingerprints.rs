use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};


fn pure_mode() -> bool {
    mockup_harness_v3::renderer::pure_cortex_gallery_active()
}

static FIXTURE_COUNTER: AtomicUsize = AtomicUsize::new(0);

struct Fixture {
    root: PathBuf,
    harness: PathBuf,
    state: PathBuf,
    runtime_settings: PathBuf,
    tracked: PathBuf,
    untracked: PathBuf,
    session_id: String,
}

impl Fixture {
    fn new() -> Self {
        let sequence = FIXTURE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = env::temp_dir().join(format!(
            "mockup-harness-v3-session-snapshot-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("tools-throwaway/mockup-harness-v3"))
            .expect("fixture root must be created");
        fs::create_dir_all(root.join("factory-runtime"))
            .expect("fixture runtime directory must be created");

        let harness = root.join("tools-throwaway/mockup-harness-v3");
        let source_harness = Path::new(env!("CARGO_MANIFEST_DIR"));
        for script in [
            "start_session_snapshot.sh",
            "check_forbidden_paths.sh",
            "isolation_snapshot.py",
            "audit_mission_commits.sh",
            "audit_mission_commits.py",
            "check_mission_handoff.sh",
            "commit_mission_change.sh",
            "initialize_mission_ledger.sh",
            "isolation_session_state.sh",
            "mission_commits.seed",
            "record_mission_commit.sh",
        ] {
            fs::copy(source_harness.join(script), harness.join(script))
                .expect("fixture script must be copied");
        }

        run(&root, ["git", "init", "--quiet"]);
        run(&root, ["git", "config", "user.name", "fixture"]);
        run(
            &root,
            ["git", "config", "user.email", "fixture@example.invalid"],
        );

        let tracked = root.join("protected.txt");
        fs::write(&tracked, "committed\n").expect("tracked fixture must be written");
        let untracked = root.join("user-owned.txt");
        fs::write(&untracked, "opaque user content\n").expect("untracked fixture must be written");
        fs::create_dir_all(root.join("tools-throwaway/mockup-harness-v3"))
            .expect("allowed root must exist");
        run(
            &root,
            [
                "git",
                "add",
                "protected.txt",
                "tools-throwaway/mockup-harness-v3",
            ],
        );
        run(
            &root,
            ["git", "commit", "--quiet", "-m", "fixture baseline"],
        );
        fs::write(harness.join("mission_commits.seed"), "")
            .expect("fixture ledger seed must be cleared");
        fs::write(&untracked, "opaque user content after commit\n")
            .expect("untracked fixture must be restored");

        let fixture = Self {
            state: root.join("factory-runtime/isolation-guard-v3"),
            runtime_settings: root.join("factory-runtime/runtime-settings.json"),
            root,
            harness,
            tracked,
            untracked,
            session_id: format!("fixture-session-{sequence}"),
        };
        fixture.start_snapshot();
        fixture.check_passes();
        fixture
    }

    fn start_snapshot(&self) {
        assert_success(self.run_snapshot(), "session snapshot");
    }

    fn run_snapshot(&self) -> Output {
        Command::new("bash")
            .arg(self.harness.join("start_session_snapshot.sh"))
            .env("V3_ISOLATION_SESSION_ID", &self.session_id)
            .env("FACTORY_RUNTIME_SETTINGS_PATH", &self.runtime_settings)
            .current_dir(&self.harness)
            .output()
            .expect("session snapshot script must start")
    }

    fn start_next_session(&mut self) {
        self.session_id.push_str("-next");
        self.start_snapshot();
    }

    fn snapshot_path(&self) -> PathBuf {
        self.state
            .join("snapshots")
            .join(format!("{}.json", self.session_id))
    }

    fn ledger_path(&self) -> PathBuf {
        self.harness
            .join(".isolation_state")
            .join("mission_commits.ledger")
    }

    fn state_marker_path(&self) -> PathBuf {
        self.state
            .parent()
            .expect("state path must have a parent directory")
            .join("isolation-guard-v3.initialized")
    }

    fn check_passes(&self) {
        assert_success(self.run_checker(), "forbidden-path checker");
    }

    fn check_fails(&self) -> Output {
        let output = self.run_checker();
        assert!(
            !output.status.success(),
            "forbidden-path checker unexpectedly accepted a changed session\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        output
    }

    fn run_checker(&self) -> Output {
        Command::new("bash")
            .arg(self.harness.join("check_forbidden_paths.sh"))
            .env("V3_ISOLATION_SESSION_ID", &self.session_id)
            .env("FACTORY_RUNTIME_SETTINGS_PATH", &self.runtime_settings)
            .current_dir(&self.harness)
            .output()
            .expect("forbidden-path checker must start")
    }

    fn run_audit(&self) -> Output {
        Command::new("bash")
            .arg(self.harness.join("audit_mission_commits.sh"))
            .env("V3_ISOLATION_SESSION_ID", &self.session_id)
            .env("FACTORY_RUNTIME_SETTINGS_PATH", &self.runtime_settings)
            .current_dir(&self.harness)
            .output()
            .expect("mission commit audit must start")
    }

    fn run_handoff_audit(&self, commit: &str) -> Output {
        Command::new("bash")
            .arg(self.harness.join("check_mission_handoff.sh"))
            .arg(commit)
            .env("V3_ISOLATION_SESSION_ID", &self.session_id)
            .env("FACTORY_RUNTIME_SETTINGS_PATH", &self.runtime_settings)
            .current_dir(&self.harness)
            .output()
            .expect("reported mission commit audit must start")
    }

    fn run_ledger_append(&self, commit: &str) -> Output {
        Command::new("bash")
            .arg(self.harness.join("record_mission_commit.sh"))
            .arg(commit)
            .current_dir(&self.harness)
            .output()
            .expect("mission ledger append must start")
    }

    fn head_commit(&self) -> String {
        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&self.root)
            .output()
            .expect("head lookup must start");
        assert_success(output.clone(), "head lookup");
        String::from_utf8(output.stdout)
            .expect("head output must be UTF-8")
            .trim()
            .to_owned()
    }

    fn head_parent_commit(&self) -> String {
        let output = Command::new("git")
            .args(["rev-parse", "HEAD^"])
            .current_dir(&self.root)
            .output()
            .expect("parent lookup must start");
        assert_success(output.clone(), "parent lookup");
        String::from_utf8(output.stdout)
            .expect("parent output must be UTF-8")
            .trim()
            .to_owned()
    }

    fn record_head_commit(&self) {
        let head = self.head_commit();
        assert_success(self.run_ledger_append(&head), "mission ledger append");
    }

    fn run_mission_commit(&self, message: &str) -> Output {
        Command::new("bash")
            .arg(self.harness.join("commit_mission_change.sh"))
            .args(["-m", message])
            .current_dir(&self.harness)
            .output()
            .expect("mission commit helper must start")
    }

    fn run_mission_commit_with_after_commit_hook(&self, message: &str, hook: &Path) -> Output {
        Command::new("bash")
            .arg(self.harness.join("commit_mission_change.sh"))
            .args(["-m", message])
            .env("COMMIT_MISSION_CHANGE_TEST_AFTER_COMMIT", hook)
            .env("FIXTURE_ROOT", &self.root)
            .current_dir(&self.harness)
            .output()
            .expect("mission commit helper with hook must start")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).expect("fixture root must be removed");
    }
}

fn run<const N: usize>(root: &Path, arguments: [&str; N]) {
    let (program, arguments) = arguments
        .split_first()
        .expect("fixture command must include a program");
    let output = Command::new(program)
        .args(arguments)
        .current_dir(root)
        .output()
        .expect("fixture command must start");
    assert_success(output, "fixture command");
}

fn run_with_identity(root: &Path, name: &str, email: &str, message: &str) {
    let output = Command::new("git")
        .args(["commit", "--quiet", "-m", message])
        .env("GIT_AUTHOR_NAME", name)
        .env("GIT_AUTHOR_EMAIL", email)
        .env("GIT_COMMITTER_NAME", name)
        .env("GIT_COMMITTER_EMAIL", email)
        .current_dir(root)
        .output()
        .expect("identity commit must start");
    assert_success(output, "identity commit");
}

fn assert_success(output: Output, purpose: &str) {
    assert!(
        output.status.success(),
        "{purpose} failed with {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn in_session_tracked_forbidden_edit_fails() {
    let fixture = Fixture::new();
    fs::write(&fixture.tracked, "changed during worker session\n")
        .expect("tracked fixture content must change");
    fixture.check_fails();
}

#[test]
fn user_changes_between_sessions_are_absorbed_by_refresh() {
    let mut fixture = Fixture::new();
    fs::write(&fixture.tracked, "user commit between worker sessions\n")
        .expect("user fixture content must change");
    run(&fixture.root, ["git", "add", "protected.txt"]);
    run_with_identity(
        &fixture.root,
        "user",
        "user@example.invalid",
        "user change between sessions",
    );
    fixture.check_passes();

    fixture.start_next_session();
    fixture.check_passes();
}

#[test]
fn untracked_user_owned_contents_are_never_read_or_hashed() {
    let fixture = Fixture::new();
    let snapshot =
        fs::read_to_string(fixture.snapshot_path()).expect("active snapshot must be readable");
    assert!(!snapshot.contains("opaque user content"));
    let start = snapshot
        .find("\"path\": \"user-owned.txt\"")
        .expect("untracked entry metadata must remain visible");
    let entry = &snapshot[start..start + snapshot[start..].find("\n    }").unwrap()];
    assert!(!entry.contains("fingerprints"));

    fs::write(
        &fixture.untracked,
        "changed opaque bytes must remain unread and unhashed\n",
    )
    .expect("untracked fixture content must change");
    fixture.check_passes();
}

#[test]
fn forbidden_to_allowed_rename_fails() {
    let fixture = Fixture::new();
    run(
        &fixture.root,
        [
            "git",
            "mv",
            "protected.txt",
            "tools-throwaway/mockup-harness-v3/allowed.txt",
        ],
    );
    let output = fixture.check_fails();
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("protected.txt"),
        "rename failure should identify the forbidden source"
    );
}

#[test]
fn same_session_snapshot_cannot_rebase_a_forbidden_edit() {
    let fixture = Fixture::new();
    fs::write(&fixture.tracked, "forbidden edit during worker session\n")
        .expect("tracked fixture content must change");

    let refresh = fixture.run_snapshot();
    assert!(
        !refresh.status.success(),
        "the same worker session must not replace its active snapshot\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&refresh.stdout),
        String::from_utf8_lossy(&refresh.stderr),
    );
    assert!(
        String::from_utf8_lossy(&refresh.stderr).contains("already bound"),
        "same-session rejection must explain that the snapshot is immutable"
    );

    let check = fixture.check_fails();
    assert!(
        String::from_utf8_lossy(&check.stderr).contains("protected.txt"),
        "the original snapshot must continue to expose the forbidden edit"
    );
}

#[test]
fn ledgered_mission_commit_passes_and_unledgered_echobt_user_commit_is_ignored() {
    let fixture = Fixture::new();
    let allowed = fixture
        .root
        .join("tools-throwaway/mockup-harness-v3/fixture.txt");
    fs::write(&allowed, "mission-owned\n").expect("allowed fixture must be written");
    run(
        &fixture.root,
        [
            "git",
            "add",
            "tools-throwaway/mockup-harness-v3/fixture.txt",
        ],
    );
    run_with_identity(
        &fixture.root,
        "echobt",
        "154886644+echobt@users.noreply.github.com",
        "mission allowed change",
    );
    fixture.record_head_commit();

    fs::write(&fixture.tracked, "echobt user-owned interleaved bytes\n")
        .expect("user-owned fixture must change");
    run(&fixture.root, ["git", "add", "protected.txt"]);
    run_with_identity(
        &fixture.root,
        "echobt",
        "154886644+echobt@users.noreply.github.com",
        "interleaved echobt user commit",
    );

    assert_success(fixture.run_audit(), "mission commit audit");
    fixture.check_passes();
}

#[test]
fn reported_mission_commit_missing_from_ledger_fails_audit() {
    let fixture = Fixture::new();
    fs::write(
        &fixture.tracked,
        "reported mission commit without ledger entry\n",
    )
    .expect("tracked fixture content must change");
    run(&fixture.root, ["git", "add", "protected.txt"]);
    run_with_identity(
        &fixture.root,
        "echobt",
        "154886644+echobt@users.noreply.github.com",
        "reported mission commit without ledger entry",
    );

    let audit = fixture.run_handoff_audit(&fixture.head_commit());
    assert!(
        !audit.status.success(),
        "a reported mission commit absent from the ledger must fail audit"
    );
    assert!(
        String::from_utf8_lossy(&audit.stderr).contains("absent from mission commit ledger"),
        "the audit should explain that the reported commit is not ledgered"
    );
}

#[test]
fn commit_helper_appends_the_exact_new_commit_sha() {
    let fixture = Fixture::new();
    let allowed = fixture
        .root
        .join("tools-throwaway/mockup-harness-v3/ledgered-by-helper.txt");
    fs::write(&allowed, "commit helper fixture\n").expect("allowed fixture must be written");
    run(
        &fixture.root,
        [
            "git",
            "add",
            "tools-throwaway/mockup-harness-v3/ledgered-by-helper.txt",
        ],
    );

    assert_success(
        fixture.run_mission_commit("commit through mission ledger helper"),
        "mission commit helper",
    );
    let head = fixture.head_commit();
    let ledger = fs::read_to_string(fixture.ledger_path()).expect("ledger must be readable");
    assert_eq!(ledger, format!("{head}\n"));
    assert_success(
        fixture.run_handoff_audit(&head),
        "ledgered mission handoff audit",
    );
}

#[test]
fn commit_helper_ledger_preserves_mission_sha_across_interleaved_user_commit() {
    let fixture = Fixture::new();
    let allowed = fixture
        .root
        .join("tools-throwaway/mockup-harness-v3/ledgered-before-user.txt");
    fs::write(&allowed, "mission commit before user commit\n")
        .expect("allowed fixture must be written");
    run(
        &fixture.root,
        [
            "git",
            "add",
            "tools-throwaway/mockup-harness-v3/ledgered-before-user.txt",
        ],
    );

    let hook = fixture.root.join("concurrent-user-commit.sh");
    fs::write(
        &hook,
        "#!/usr/bin/env bash\n\
         set -euo pipefail\n\
         printf '%s\\n' 'user commit landed after mission commit' > \"$FIXTURE_ROOT/protected.txt\"\n\
         git -C \"$FIXTURE_ROOT\" commit --quiet --only protected.txt -m 'interleaved user commit'\n",
    )
    .expect("concurrent user hook must be written");
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o700))
        .expect("concurrent user hook must be executable");

    assert_success(
        fixture.run_mission_commit_with_after_commit_hook(
            "mission commit with interleaved user commit",
            &hook,
        ),
        "mission commit helper with interleaved user commit",
    );

    let user_commit = fixture.head_commit();
    let mission_commit = fixture.head_parent_commit();
    assert_ne!(
        mission_commit, user_commit,
        "the hook must create a distinct user commit after the mission commit"
    );
    assert_eq!(
        fs::read_to_string(fixture.ledger_path()).expect("ledger must be readable"),
        format!("{mission_commit}\n"),
        "the ledger must contain the mission commit, not the later user commit"
    );
    assert_success(
        fixture.run_handoff_audit(&mission_commit),
        "interleaved mission commit handoff audit",
    );
    assert_success(fixture.run_audit(), "interleaved mission commit audit");
}

#[test]
fn commit_ledger_append_fails_closed_without_a_ledger() {
    let fixture = Fixture::new();
    fs::remove_file(fixture.ledger_path()).expect("ledger must be removable in the fixture");
    let missing_ledger = fixture.run_ledger_append(&fixture.head_commit());
    assert!(
        !missing_ledger.status.success(),
        "ledger append must fail when its required ledger is missing"
    );
    assert!(
        String::from_utf8_lossy(&missing_ledger.stderr).contains("missing mission commit ledger"),
        "ledger append must identify the missing ledger"
    );
}

#[test]
fn commit_helper_refuses_to_commit_without_a_ledger() {
    let fixture = Fixture::new();
    let allowed = fixture
        .root
        .join("tools-throwaway/mockup-harness-v3/unledgered-helper-change.txt");
    fs::write(&allowed, "helper must fail before committing\n")
        .expect("allowed fixture must be written");
    run(
        &fixture.root,
        [
            "git",
            "add",
            "tools-throwaway/mockup-harness-v3/unledgered-helper-change.txt",
        ],
    );
    let before = fixture.head_commit();
    fs::remove_file(fixture.ledger_path()).expect("ledger must be removable in the fixture");

    let output = fixture.run_mission_commit("must not commit without a ledger");
    assert!(
        !output.status.success(),
        "commit helper must refuse to commit without a ledger"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("missing mission commit ledger"),
        "commit helper must identify the missing ledger"
    );
    assert_eq!(
        fixture.head_commit(),
        before,
        "commit helper must not create an unledgered commit"
    );
}

#[test]
fn ledgered_out_of_scope_commit_fails_the_commit_audit() {
    let fixture = Fixture::new();
    fs::write(&fixture.tracked, "mission must not write here\n")
        .expect("tracked fixture content must change");
    run(&fixture.root, ["git", "add", "protected.txt"]);
    let output = Command::new("git")
        .args(["commit", "--quiet", "-m", "out of scope mission change"])
        .env("GIT_AUTHOR_NAME", "echobt")
        .env(
            "GIT_AUTHOR_EMAIL",
            "154886644+echobt@users.noreply.github.com",
        )
        .env("GIT_COMMITTER_NAME", "echobt")
        .env(
            "GIT_COMMITTER_EMAIL",
            "154886644+echobt@users.noreply.github.com",
        )
        .current_dir(&fixture.root)
        .output()
        .expect("out-of-scope commit must start");
    assert_success(output, "out-of-scope commit");
    fixture.record_head_commit();

    let audit = fixture.run_audit();
    assert!(!audit.status.success(), "out-of-scope audit must fail");
    assert!(String::from_utf8_lossy(&audit.stderr).contains("protected.txt"));
}

#[test]
fn clean_forbidden_source_renamed_into_allowed_root_is_audited() {
    let fixture = Fixture::new();
    run(
        &fixture.root,
        [
            "git",
            "mv",
            "protected.txt",
            "tools-throwaway/mockup-harness-v3/renamed.txt",
        ],
    );
    run_with_identity(
        &fixture.root,
        "echobt",
        "154886644+echobt@users.noreply.github.com",
        "move clean forbidden source into v3 root",
    );
    fixture.record_head_commit();

    let audit = fixture.run_audit();
    assert!(
        !audit.status.success(),
        "a clean forbidden source renamed into an allowed root must fail audit"
    );
    assert!(String::from_utf8_lossy(&audit.stderr).contains("protected.txt"));
}

#[test]
fn clean_forbidden_source_copied_into_allowed_root_is_audited() {
    let fixture = Fixture::new();
    fs::copy(
        &fixture.tracked,
        fixture.harness.join("copied-from-protected.txt"),
    )
    .expect("clean protected source must be copied");
    run(
        &fixture.root,
        [
            "git",
            "add",
            "tools-throwaway/mockup-harness-v3/copied-from-protected.txt",
        ],
    );
    run_with_identity(
        &fixture.root,
        "echobt",
        "154886644+echobt@users.noreply.github.com",
        "copy clean forbidden source into v3 root",
    );
    fixture.record_head_commit();

    let audit = fixture.run_audit();
    assert!(
        !audit.status.success(),
        "a clean forbidden source copied into an allowed root must fail audit"
    );
    assert!(String::from_utf8_lossy(&audit.stderr).contains("protected.txt"));
}

#[test]
fn unledgered_echobt_commit_remains_user_owned_after_session_refresh() {
    let mut fixture = Fixture::new();
    fs::write(&fixture.tracked, "mission must not write here\n")
        .expect("tracked fixture content must change");
    run(&fixture.root, ["git", "add", "protected.txt"]);
    run_with_identity(
        &fixture.root,
        "echobt",
        "154886644+echobt@users.noreply.github.com",
        "out of scope mission change before next session",
    );

    fixture.start_next_session();
    assert_success(
        fixture.run_audit(),
        "unledgered echobt user commit audit after session refresh",
    );
}

#[test]
fn missing_ledger_fails_closed() {
    let fixture = Fixture::new();
    fs::remove_file(fixture.ledger_path()).expect("ledger must be removable in the fixture");
    let audit = fixture.run_audit();
    assert!(
        !audit.status.success(),
        "the commit audit must fail closed when the ledger is missing"
    );
    assert!(
        String::from_utf8_lossy(&audit.stderr).contains("missing mission commit ledger"),
        "the audit must identify the missing ledger"
    );
}

#[test]
fn trusted_migration_seed_is_ledgered_and_audited() {
    let fixture = Fixture::new();
    let allowed = fixture
        .root
        .join("tools-throwaway/mockup-harness-v3/pre-ledger-mission.txt");
    fs::write(&allowed, "trusted pre-ledger mission work\n")
        .expect("allowed fixture must be written");
    run(
        &fixture.root,
        [
            "git",
            "add",
            "tools-throwaway/mockup-harness-v3/pre-ledger-mission.txt",
        ],
    );
    run_with_identity(
        &fixture.root,
        "echobt",
        "154886644+echobt@users.noreply.github.com",
        "trusted pre-ledger mission work",
    );
    let commit = fixture.head_commit();
    fs::write(
        fixture.harness.join("mission_commits.seed"),
        format!("{commit}\n"),
    )
    .expect("trusted migration seed must be writable");

    let initialized = Command::new("bash")
        .arg(fixture.harness.join("initialize_mission_ledger.sh"))
        .current_dir(&fixture.harness)
        .output()
        .expect("trusted migration ledger initializer must start");
    assert_success(initialized, "trusted migration ledger initializer");
    assert_eq!(
        fs::read_to_string(fixture.ledger_path()).expect("ledger must be readable"),
        format!("{commit}\n")
    );
    assert_success(fixture.run_audit(), "trusted migration ledger audit");
}

#[test]
fn trusted_bootstrap_copy_source_is_narrowly_authorized() {
    let fixture = Fixture::new();
    let bootstrap = "c9410a0f5417d303945b9a2120c73a7a505c8a88";
    let output = Command::new("python3")
        .args([
            "-c",
            "from audit_mission_commits import allowed_for_commit; \
assert allowed_for_commit('c9410a0f5417d303945b9a2120c73a7a505c8a88', 'tools-throwaway/mockup-harness/Cargo.lock'); \
assert not allowed_for_commit('c9410a0f5417d303945b9a2120c73a7a505c8a88', 'protected.txt'); \
assert not allowed_for_commit('0000000000000000000000000000000000000000', 'tools-throwaway/mockup-harness/Cargo.lock')",
        ])
        .current_dir(&fixture.harness)
        .output()
        .expect("bootstrap authorization probe must start");
    assert_success(output, "bootstrap authorization probe");
    assert_eq!(bootstrap.len(), 40, "bootstrap SHA must remain full length");
}

#[test]
fn deterministic_foundation_font_copy_source_is_narrowly_authorized() {
    let fixture = Fixture::new();
    let foundation = "023e2019d6cfc362a075ec6b29c89a93f51e9259";
    let output = Command::new("python3")
        .args([
            "-c",
            "from audit_mission_commits import allowed_for_commit; \
assert allowed_for_commit('023e2019d6cfc362a075ec6b29c89a93f51e9259', 'tools-throwaway/mockup-harness/assets/DejaVuSansMono.ttf'); \
assert allowed_for_commit('023e2019d6cfc362a075ec6b29c89a93f51e9259', 'tools-throwaway/mockup-harness/assets/DejaVuSansMono-Bold.ttf'); \
assert not allowed_for_commit('023e2019d6cfc362a075ec6b29c89a93f51e9259', 'tools-throwaway/mockup-harness/Cargo.lock'); \
assert not allowed_for_commit('0000000000000000000000000000000000000000', 'tools-throwaway/mockup-harness/assets/DejaVuSansMono.ttf')",
        ])
        .current_dir(&fixture.harness)
        .output()
        .expect("foundation font authorization probe must start");
    assert_success(output, "foundation font authorization probe");
    assert_eq!(
        foundation.len(),
        40,
        "foundation SHA must remain full length"
    );
}

#[test]
fn unreachable_ledgered_commit_fails_closed() {
    let fixture = Fixture::new();
    fs::write(
        fixture.ledger_path(),
        "0000000000000000000000000000000000000000\n",
    )
    .expect("fixture ledger must be writable");
    let audit = fixture.run_audit();
    assert!(
        !audit.status.success(),
        "an unreachable ledgered commit must fail audit"
    );
    assert!(
        String::from_utf8_lossy(&audit.stderr).contains("unreachable"),
        "the audit must identify the unreachable ledger entry"
    );
}

#[test]
fn uncommitted_forbidden_change_fails_regardless_of_ledger_state() {
    let fixture = Fixture::new();
    let allowed = fixture
        .root
        .join("tools-throwaway/mockup-harness-v3/ledgered-allowed.txt");
    fs::write(&allowed, "ledgered allowed change\n").expect("allowed fixture must be written");
    run(
        &fixture.root,
        [
            "git",
            "add",
            "tools-throwaway/mockup-harness-v3/ledgered-allowed.txt",
        ],
    );
    run_with_identity(
        &fixture.root,
        "echobt",
        "154886644+echobt@users.noreply.github.com",
        "ledgered allowed mission change",
    );
    fixture.record_head_commit();
    assert_success(fixture.run_audit(), "ledgered allowed mission audit");

    fs::write(&fixture.tracked, "forbidden uncommitted change\n")
        .expect("tracked fixture content must change");
    fixture.check_fails();
}

#[test]
fn deleted_mission_state_cannot_be_reinitialized() {
    let mut fixture = Fixture::new();
    fs::remove_dir_all(&fixture.state).expect("mission state must be removable in the fixture");
    fs::create_dir(&fixture.state)
        .expect("deleted mission state must be recreatable in the fixture");
    assert!(fixture.state_marker_path().is_file());

    fixture.session_id.push_str("-next");
    let refresh = fixture.run_snapshot();
    assert!(
        !refresh.status.success(),
        "a previously initialized mission must fail closed when state is deleted"
    );
    assert!(
        String::from_utf8_lossy(&refresh.stderr).contains("persistent mission state identity"),
        "the failure must identify the missing state identity"
    );
}

#[test]
fn deleted_noncurrent_session_snapshot_cannot_be_rebound() {
    let mut fixture = Fixture::new();
    let original_session = fixture.session_id.clone();
    fs::remove_file(fixture.snapshot_path())
        .expect("session convenience snapshot must be removable in the fixture");

    fixture.start_next_session();
    fixture.session_id = original_session;
    let restart = fixture.run_snapshot();
    assert!(
        !restart.status.success(),
        "a deleted non-current convenience snapshot must remain bound"
    );
    assert!(
        String::from_utf8_lossy(&restart.stderr).contains("already bound"),
        "the original worker session must reject rebinding"
    );
}

#[test]
fn missing_or_malformed_active_snapshot_fails_closed() {
    let mut fixture = Fixture::new();
    fs::remove_file(fixture.snapshot_path()).expect("active snapshot must be removed");
    assert!(!fixture.run_checker().status.success());
    assert!(!fixture.run_audit().status.success());

    fixture.start_next_session();
    fs::write(fixture.snapshot_path(), "{\"schema_version\": 999}\n")
        .expect("malformed active snapshot must be written");
    assert!(!fixture.run_checker().status.success());
    assert!(!fixture.run_audit().status.success());
}
