use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

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
            "isolation_session_state.sh",
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

    fn audit_origin_path(&self) -> PathBuf {
        self.state.join("mission-audit-origin.json")
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
    fixture.check_fails();

    fixture.start_next_session();
    fixture.check_passes();
}

#[test]
fn untracked_user_owned_contents_are_never_read_or_hashed() {
    let fixture = Fixture::new();
    let snapshot = fs::read_to_string(fixture.snapshot_path())
        .expect("active snapshot must be readable");
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
fn allowed_echobt_commit_passes_and_audit_ignores_interleaved_user_commit() {
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

    fs::write(&fixture.untracked, "user-owned interleaved bytes\n")
        .expect("user-owned fixture must change");
    run(&fixture.root, ["git", "add", "user-owned.txt"]);
    run_with_identity(
        &fixture.root,
        "user",
        "user@example.invalid",
        "interleaved user commit",
    );

    assert_success(fixture.run_audit(), "mission commit audit");
    fixture.check_fails();
}

#[test]
fn out_of_scope_echobt_commit_fails_the_commit_audit() {
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
        fixture
            .harness
            .join("copied-from-protected.txt"),
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

    let audit = fixture.run_audit();
    assert!(
        !audit.status.success(),
        "a clean forbidden source copied into an allowed root must fail audit"
    );
    assert!(String::from_utf8_lossy(&audit.stderr).contains("protected.txt"));
}

#[test]
fn persistent_audit_origin_detects_mission_commit_after_session_refresh() {
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
    let audit = fixture.run_audit();
    assert!(
        !audit.status.success(),
        "a refreshed worker snapshot must not exclude earlier mission commits"
    );
    assert!(String::from_utf8_lossy(&audit.stderr).contains("protected.txt"));
}

#[test]
fn missing_persistent_audit_origin_cannot_hide_a_mission_commit() {
    let mut fixture = Fixture::new();
    fs::write(&fixture.tracked, "mission must not write here\n")
        .expect("tracked fixture content must change");
    run(&fixture.root, ["git", "add", "protected.txt"]);
    run_with_identity(
        &fixture.root,
        "echobt",
        "154886644+echobt@users.noreply.github.com",
        "out of scope mission change before origin refresh",
    );

    fs::remove_file(fixture.audit_origin_path())
        .expect("persistent origin must be removable in the fixture");
    fixture.session_id.push_str("-next");
    let refresh = fixture.run_snapshot();
    assert!(
        !refresh.status.success(),
        "an existing mission must fail closed when its persistent audit origin is missing"
    );
    assert!(
        String::from_utf8_lossy(&refresh.stderr).contains("missing persistent mission audit origin"),
        "the failure must identify the missing persistent audit origin"
    );
}

#[test]
fn deleted_mission_state_cannot_be_reinitialized() {
    let mut fixture = Fixture::new();
    fs::remove_dir_all(&fixture.state)
        .expect("mission state must be removable in the fixture");
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
    fs::remove_file(fixture.snapshot_path())
        .expect("active snapshot must be removed");
    assert!(!fixture.run_checker().status.success());
    assert!(!fixture.run_audit().status.success());

    fixture.start_next_session();
    fs::write(
        fixture.snapshot_path(),
        "{\"schema_version\": 999}\n",
    )
    .expect("malformed active snapshot must be written");
    assert!(!fixture.run_checker().status.success());
    assert!(!fixture.run_audit().status.success());
}
