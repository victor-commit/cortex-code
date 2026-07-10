use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static FIXTURE_COUNTER: AtomicUsize = AtomicUsize::new(0);

struct Fixture {
    root: PathBuf,
    harness: PathBuf,
    tracked: PathBuf,
    untracked: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        Self::with_dirty_index_and_deletion(false, false)
    }

    fn with_dirty_index(dirty_index: bool) -> Self {
        Self::with_dirty_index_and_deletion(dirty_index, false)
    }

    fn with_staged_deletion() -> Self {
        Self::with_dirty_index_and_deletion(false, true)
    }

    fn with_dirty_index_and_deletion(dirty_index: bool, staged_deletion: bool) -> Self {
        let sequence = FIXTURE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = env::temp_dir().join(format!(
            "mockup-harness-v3-forbidden-paths-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("fixture root must be created");

        let harness = root.join("tools-throwaway/mockup-harness-v3");
        fs::create_dir_all(harness.join("baselines")).expect("fixture harness must be created");
        for frozen_root in [
            "tools-throwaway/mockup-harness",
            "plans/assets/mockups-v2",
            "plans/assets/mockups",
        ] {
            let directory = root.join(frozen_root);
            fs::create_dir_all(&directory).expect("frozen reference directory must be created");
            fs::write(directory.join("reference.txt"), "frozen reference\n")
                .expect("frozen reference must be written");
        }

        let source_harness = Path::new(env!("CARGO_MANIFEST_DIR"));
        for script in [
            "record_baseline.sh",
            "check_forbidden_paths.sh",
            "forbidden_path_baseline.py",
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
        run(&root, ["git", "add", "."]);
        run(
            &root,
            ["git", "commit", "--quiet", "-m", "fixture baseline"],
        );

        if staged_deletion {
            run(&root, ["git", "rm", "--quiet", "protected.txt"]);
        } else if dirty_index {
            fs::write(&tracked, "pre-existing dirty index\n")
                .expect("dirty index fixture must be written");
            run(&root, ["git", "add", "protected.txt"]);
        }
        if !staged_deletion {
            fs::write(&tracked, "pre-existing dirty worktree\n")
                .expect("dirty tracked fixture must be written");
        }
        let untracked = root.join("user-owned.txt");
        fs::write(&untracked, "untracked content must remain opaque\n")
            .expect("untracked fixture must be written");

        let fixture = Self {
            root,
            harness,
            tracked,
            untracked,
        };
        fixture.record_baseline();
        fixture.check_passes();
        fixture
    }

    fn record_baseline(&self) {
        let output = Command::new("bash")
            .arg(self.harness.join("record_baseline.sh"))
            .current_dir(&self.harness)
            .output()
            .expect("baseline recorder must start");
        assert_success(output, "baseline recorder");
    }

    fn check_passes(&self) {
        let output = self.run_checker();
        assert_success(output, "forbidden-path checker");
    }

    fn check_fails(&self) {
        let output = self.run_checker();
        assert!(
            !output.status.success(),
            "forbidden-path checker unexpectedly accepted a changed baseline\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }

    fn run_checker(&self) -> Output {
        Command::new("bash")
            .arg(self.harness.join("check_forbidden_paths.sh"))
            .current_dir(&self.harness)
            .output()
            .expect("forbidden-path checker must start")
    }

    fn baseline(&self) -> String {
        fs::read_to_string(self.harness.join("baselines/forbidden-status.json"))
            .expect("forbidden status baseline must be readable")
    }

    fn porcelain(&self) -> String {
        let output = Command::new("git")
            .args(["status", "--porcelain=v1"])
            .current_dir(&self.root)
            .output()
            .expect("fixture status must start");
        assert!(
            output.status.success(),
            "fixture status failed with {}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr),
        );
        let status = String::from_utf8(output.stdout).expect("fixture status must be UTF-8");
        let entries = status
            .lines()
            .filter(|line| !line.contains("tools-throwaway/mockup-harness-v3/"))
            .collect::<Vec<_>>();
        if entries.is_empty() {
            String::new()
        } else {
            format!("{}\n", entries.join("\n"))
        }
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

fn assert_success(output: Output, purpose: &str) {
    assert!(
        output.status.success(),
        "{purpose} failed with {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn baseline_entry<'a>(baseline: &'a str, path: &str) -> &'a str {
    let marker = format!("\"path\": \"{path}\"");
    let end = baseline
        .find(&marker)
        .unwrap_or_else(|| panic!("baseline must include {path:?}"));
    let start = baseline[..end]
        .rfind("    {")
        .expect("entry must begin with an object");
    let remainder = &baseline[end..];
    let entry_end = remainder
        .find("\n    }")
        .expect("entry must end with an object");
    &baseline[start..end + entry_end + "\n    }".len()]
}

#[test]
fn dirty_tracked_content_changes_that_keep_porcelain_fail() {
    let fixture = Fixture::new();
    assert_eq!(fixture.porcelain(), " M protected.txt\n?? user-owned.txt\n");

    fs::write(&fixture.tracked, "same porcelain, different content\n")
        .expect("tracked fixture content must change");
    assert_eq!(fixture.porcelain(), " M protected.txt\n?? user-owned.txt\n");

    fixture.check_fails();
}

#[test]
fn dirty_tracked_mode_changes_that_keep_porcelain_fail() {
    let fixture = Fixture::new();
    assert_eq!(fixture.porcelain(), " M protected.txt\n?? user-owned.txt\n");
    let mut permissions = fs::metadata(&fixture.tracked)
        .expect("tracked fixture metadata must be readable")
        .permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        permissions.set_mode(0o755);
    }
    fs::set_permissions(&fixture.tracked, permissions).expect("tracked fixture mode must change");
    assert_eq!(fixture.porcelain(), " M protected.txt\n?? user-owned.txt\n");

    fixture.check_fails();
}

#[test]
fn dirty_tracked_index_changes_that_keep_porcelain_fail() {
    let index = Fixture::with_dirty_index(true);
    assert_eq!(index.porcelain(), "MM protected.txt\n?? user-owned.txt\n");
    fs::write(&index.tracked, "different staged bytes\n")
        .expect("index fixture content must change");
    run(&index.root, ["git", "add", "protected.txt"]);
    fs::write(&index.tracked, "pre-existing dirty worktree\n")
        .expect("worktree fixture must be restored");
    assert_eq!(index.porcelain(), "MM protected.txt\n?? user-owned.txt\n");
    index.check_fails();
}

#[test]
fn dirty_tracked_deletion_type_path_rename_and_status_changes_fail() {
    let deletion = Fixture::new();
    fs::remove_file(&deletion.tracked).expect("tracked fixture must be deleted");
    deletion.check_fails();

    let type_change = Fixture::new();
    fs::remove_file(&type_change.tracked).expect("tracked fixture must be replaced");
    #[cfg(unix)]
    std::os::unix::fs::symlink("other-target", &type_change.tracked)
        .expect("tracked fixture must become a symlink");
    type_change.check_fails();

    let path_change = Fixture::new();
    run(
        &path_change.root,
        ["git", "mv", "protected.txt", "renamed.txt"],
    );
    path_change.check_fails();

    let status_change = Fixture::new();
    run(
        &status_change.root,
        ["git", "checkout", "--", "protected.txt"],
    );
    status_change.check_fails();
}

#[test]
fn dirty_tracked_staged_deletion_is_fingerprinted_and_unchanged_baseline_passes() {
    let fixture = Fixture::with_staged_deletion();
    assert_eq!(fixture.porcelain(), "D  protected.txt\n?? user-owned.txt\n");
    let baseline = fixture.baseline();
    let tracked = baseline_entry(&baseline, "protected.txt");
    assert!(tracked.contains("\"worktree_sha256\""));
    assert!(tracked.contains("\"index_sha256\""));

    fixture.check_passes();
}

#[test]
fn only_dirty_tracked_paths_have_fingerprints_and_untracked_contents_are_opaque() {
    let fixture = Fixture::new();
    let baseline = fixture.baseline();

    assert!(baseline.contains("\"schema_version\": 2"));
    let tracked = baseline_entry(&baseline, "protected.txt");
    assert!(tracked.contains("\"worktree_sha256\""));
    assert!(tracked.contains("\"index_sha256\""));

    let untracked = baseline_entry(&baseline, "user-owned.txt");
    assert!(!untracked.contains("sha256"));
    assert!(!untracked.contains("fingerprints"));

    fs::write(
        &fixture.untracked,
        "different untracked content must not affect verification\n",
    )
    .expect("untracked fixture content must change");
    fixture.check_passes();
}

#[test]
fn verifier_rejects_a_fingerprint_declared_for_an_untracked_path() {
    let fixture = Fixture::new();
    let invalid = fixture.baseline().replace(
        r#"    {
      "path": "user-owned.txt",
      "status": "??"
    }"#,
        r#"    {
      "fingerprints": {
        "index_sha256": "0000000000000000000000000000000000000000000000000000000000000000",
        "worktree_sha256": "0000000000000000000000000000000000000000000000000000000000000000"
      },
      "path": "user-owned.txt",
      "status": "??"
    }"#,
    );
    fs::write(
        fixture.harness.join("baselines/forbidden-status.json"),
        invalid,
    )
    .expect("invalid fixture baseline must be written");

    fixture.check_fails();
}
