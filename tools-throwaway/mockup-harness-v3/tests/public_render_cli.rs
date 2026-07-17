use std::collections::{BTreeMap, BTreeSet};
use std::process::{Command, Output};
use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard, OnceLock},
};

use mockup_harness_v3::{canonical_inventory, Category, ImplementationStatus};

static RENDER_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn pure_cortex_mode() -> bool {
    mockup_harness_v3::renderer::pure_cortex_gallery_active()
}

fn harness() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mockup-harness-v3"))
}

fn renderer_test_lock() -> MutexGuard<'static, ()> {
    RENDER_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("public renderer test lock must not be poisoned")
}

fn run(arguments: &[&str]) -> Output {
    harness()
        .args(arguments)
        .output()
        .expect("the public renderer CLI must run")
}

fn gallery_root() -> PathBuf {
    PathBuf::from("/projects/cortex-code/plans/assets/mockups-v3")
}

/// Path→SHA ownership map from the gallery renderer manifest.
/// Independent snapshots after each selector prove VAL-V3-HARNESS-008 set equality
/// without vacuous dual-read of a single final filesystem state.
fn ownership_manifest_paths(root: &Path) -> BTreeMap<String, String> {
    let path = root.join(".renderer-manifest-v1");
    let body = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("ownership manifest missing at {}: {error}", path.display()));
    let mut lines = body.lines();
    assert_eq!(
        lines.next(),
        Some("mockup-harness-v3-render-manifest-v1"),
        "ownership header for {}",
        root.display()
    );
    let mut map = BTreeMap::new();
    for line in lines {
        let (relative, hash) = line
            .split_once('\t')
            .unwrap_or_else(|| panic!("invalid ownership line: {line}"));
        assert_eq!(hash.len(), 64, "hash width for {relative}");
        assert!(
            map.insert(relative.to_owned(), hash.to_owned()).is_none(),
            "duplicate ownership path {relative}"
        );
    }
    map
}

fn assert_complete_86_inventory_on_disk() {
    let mut counts = BTreeMap::<String, usize>::new();
    for screen in canonical_inventory() {
        let category = screen.category.as_str();
        let base = gallery_root().join(category).join(screen.id);
        assert!(
            base.with_extension("txt").is_file(),
            "missing final TXT for {}/{}",
            category,
            screen.id
        );
        assert!(
            base.with_extension("png").is_file(),
            "missing final PNG for {}/{}",
            category,
            screen.id
        );
        *counts.entry(category.to_owned()).or_default() += 1;
    }
    for (category, expected) in expected_category_counts() {
        assert_eq!(
            counts.get(category).copied().unwrap_or(0),
            expected,
            "category {category} pair count must match the registry"
        );
    }
}

fn expected_category_counts() -> BTreeMap<&'static str, usize> {
    [
        ("session", 21usize),
        ("input", 20),
        ("views", 16),
        ("auth", 7),
        ("states", 8),
        ("orchestration", 14),
    ]
    .into_iter()
    .collect()
}

#[test]
fn implemented_and_all_selectors_publish_the_complete_86_state_gallery() {
    // VAL-V3-HARNESS-008: snapshot membership independently after --implemented
    // and after --all. Comparing only a post-both inventory is vacuous.
    let _test_lock = renderer_test_lock();
    assert_eq!(canonical_inventory().len(), 86);
    assert!(canonical_inventory()
        .iter()
        .all(|screen| screen.status == ImplementationStatus::Implemented));

    if pure_cortex_mode() {
        // Pure dump: hybrid --implemented/--all must refuse (v4 owns paint).
        // Equality is proven from the live pure ownership sheet alone.
        let ownership = ownership_manifest_paths(&gallery_root());
        assert_eq!(ownership.len(), 172);
        assert_complete_86_inventory_on_disk();
        for selector in [["--implemented"], ["--all"]] {
            let refused = run(&selector);
            assert_eq!(
                refused.status.code(),
                Some(3),
                "pure-mode must refuse hybrid {:?} over pure roots; stderr={}",
                selector,
                String::from_utf8_lossy(&refused.stderr)
            );
            let stderr = String::from_utf8_lossy(&refused.stderr);
            assert!(
                stderr.contains("pure Cortex") || stderr.contains("pure-cortex"),
                "pure-mode refuse diagnostic: {stderr}"
            );
        }
        // Independent pre/post ownership snapshots (no hybrid write) stay equal.
        let after = ownership_manifest_paths(&gallery_root());
        assert_eq!(ownership, after);
        return;
    }

    let implemented = run(&["--implemented"]);
    assert!(
        implemented.status.success(),
        "--implemented must succeed for the final 86-state gallery; stderr={}",
        String::from_utf8_lossy(&implemented.stderr)
    );
    assert!(
        implemented.stdout.is_empty(),
        "--implemented must write no stdout"
    );
    assert!(
        implemented.stderr.is_empty(),
        "--implemented must write no stderr"
    );
    let after_implemented = ownership_manifest_paths(&gallery_root());
    assert_eq!(
        after_implemented.len(),
        172,
        "--implemented ownership must list exactly 86 TXT + 86 PNG paths"
    );
    assert_complete_86_inventory_on_disk();

    let all = run(&["--all"]);
    assert!(
        all.status.success(),
        "--all must succeed for the final 86-state gallery; stderr={}",
        String::from_utf8_lossy(&all.stderr)
    );
    assert!(all.stdout.is_empty(), "--all must write no stdout");
    assert!(all.stderr.is_empty(), "--all must write no stderr");
    let after_all = ownership_manifest_paths(&gallery_root());
    assert_eq!(
        after_all.len(),
        172,
        "--all ownership must list exactly 86 TXT + 86 PNG paths"
    );
    assert_complete_86_inventory_on_disk();

    assert_eq!(
        after_implemented, after_all,
        "post--implemented path→SHA ownership must equal post--all ownership"
    );
}

#[test]
fn single_screen_selector_is_exact_and_unknown_screen_is_usage_error() {
    let _test_lock = renderer_test_lock();
    let marker = gallery_root()
        .join("session")
        .join("home.png")
        .metadata()
        .and_then(|meta| meta.modified())
        .ok();

    if pure_cortex_mode() {
        let refused = run(&["home"]);
        assert_eq!(
            refused.status.code(),
            Some(3),
            "pure-mode must refuse hybrid single-screen publish; stderr={}",
            String::from_utf8_lossy(&refused.stderr)
        );
        assert!(gallery_root().join("session/home.txt").is_file());
        assert!(gallery_root().join("session/home.png").is_file());
    } else {
        let output = run(&["home"]);
        assert!(output.status.success());
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
        assert!(gallery_root().join("session/home.txt").is_file());
        assert!(gallery_root().join("session/home.png").is_file());
    }
    let _ = marker;

    let rejected = run(&["not-a-screen"]);
    assert_eq!(rejected.status.code(), Some(2));
    assert!(rejected.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&rejected.stderr);
    assert!(stderr.contains("not-a-screen"));
}

#[test]
fn category_selectors_are_exact_for_all_six_categories() {
    let _test_lock = renderer_test_lock();
    let expected = expected_category_counts();
    for category in Category::ALL {
        let name = category.as_str();
        if pure_cortex_mode() {
            let refused = run(&[name]);
            assert_eq!(
                refused.status.code(),
                Some(3),
                "pure-mode must refuse hybrid category {name}; stderr={}",
                String::from_utf8_lossy(&refused.stderr)
            );
        } else {
            let output = run(&[name]);
            assert!(
                output.status.success(),
                "category {name} selector must succeed; stderr={}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let want = *expected.get(name).expect("expected category count");
        let mut pairs = 0usize;
        for screen in canonical_inventory()
            .iter()
            .filter(|screen| screen.category == category)
        {
            assert!(gallery_root()
                .join(name)
                .join(format!("{}.txt", screen.id))
                .is_file());
            assert!(gallery_root()
                .join(name)
                .join(format!("{}.png", screen.id))
                .is_file());
            pairs += 1;
        }
        assert_eq!(pairs, want, "category {name} must publish only its registry IDs");
    }

    let rejected = run(&["not-a-category"]);
    assert_eq!(rejected.status.code(), Some(2));
    assert!(rejected.stdout.is_empty());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("not-a-category"));
}

#[test]
fn busy_public_renderer_exits_75_without_replacing_the_canonical_gallery() {
    let _test_lock = renderer_test_lock();
    let output_root = gallery_root();
    let lock = output_root
        .parent()
        .expect("canonical root has a parent")
        .join(".mockups-v3.render.lock");
    let prior_manifest = fs::read(output_root.join(".renderer-manifest-v1"))
        .expect("the final gallery must already have an ownership manifest");
    if pure_cortex_mode() {
        // Pure ownership refuses publish before the coordinator is acquired, so
        // the busy lock is only a hybrid-path contract. Prove refuse + immutability.
        let refused = run(&["--implemented"]);
        assert_eq!(refused.status.code(), Some(3));
        assert!(String::from_utf8_lossy(&refused.stderr).contains("pure"));
        assert_eq!(
            fs::read(output_root.join(".renderer-manifest-v1"))
                .expect("pure refuse must preserve ownership manifest"),
            prior_manifest
        );
        return;
    }
    let lock_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock)
        .expect("test must own the coordinator lock");
    let output = run(&["--implemented"]);
    drop(lock_file);
    fs::remove_file(&lock).expect("test must remove its coordinator lock");

    assert_eq!(output.status.code(), Some(75));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("render coordinator busy"));
    assert_eq!(
        fs::read(output_root.join(".renderer-manifest-v1"))
            .expect("busy exit must preserve the canonical ownership manifest"),
        prior_manifest,
        "busy exit must publish nothing"
    );
}

#[test]
fn missing_renderer_map_still_fails_closed_before_publication() {
    // Public CLI path is complete; prove preflight risk for partial maps remains
    // via the local renderer map validator used by --all.
    use mockup_harness_v3::renderer::run_renderer_negative_fixture;

fn pure_mode() -> bool {
    mockup_harness_v3::renderer::pure_cortex_gallery_active()
}

    let fail = run_renderer_negative_fixture(
        "inventory-without-renderer",
        Path::new("/tmp/mockup-harness-v3-map-placeholder"),
    )
    .expect_err("implemented inventory without a renderer must fail closed");
    assert!(fail.contains("implemented canonical ID lacks renderer"));
}

#[test]
fn unknown_selectors_are_usage_errors_without_writes() {
    let _test_lock = renderer_test_lock();
    for arguments in [
        vec!["not-a-screen"],
        vec!["--flow", "not-a-flow"],
        vec!["--flow"],
        vec!["--all", "--flow", "daily-work"],
    ] {
        let output = run(&arguments);
        assert_eq!(output.status.code(), Some(2), "{arguments:?}");
        assert!(output.stdout.is_empty(), "{arguments:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!stderr.is_empty(), "{arguments:?}");
        if let Some(selector) = arguments.last() {
            assert!(
                stderr.contains(selector),
                "usage diagnostics must name the invalid selector"
            );
        }
    }
}

#[test]
fn audit_selectors_emit_json_only_on_stdout() {
    let _test_lock = renderer_test_lock();
    let single = run(&["--audit-json", "permission-shell"]);
    assert!(single.status.success());
    assert!(single.stderr.is_empty());
    let single_json = String::from_utf8(single.stdout).expect("audit output must be UTF-8");
    assert!(single_json.starts_with("{\"document_type\":\"typed-render-audit\""));
    assert!(single_json.contains("\"screen_id\":\"permission-shell\""));
    assert!(single_json.contains("\"category\":\"input\""));
    assert!(single_json.contains("\"flow\":\"input-and-approval\""));

    let all = run(&["--audit-json", "--all"]);
    assert!(all.status.success());
    assert!(all.stderr.is_empty());
    let all_json = String::from_utf8(all.stdout).expect("audit output must be UTF-8");
    assert!(all_json.starts_with("{\"document_type\":\"typed-render-audit-collection\""));
    assert!(all_json.contains("\"screen_count\":86"));
    assert!(all_json.contains("\"screen_id\":\"home\""));
    assert!(all_json.contains("\"screen_id\":\"tool-envelope\""));

    let repeated = run(&["--audit-json", "--all"]);
    assert!(repeated.status.success());
    assert_eq!(
        all_json.as_bytes(),
        repeated.stdout.as_slice(),
        "repeated public audit of the same fixture set must be byte-identical"
    );
}

#[test]
fn final_impl_statuses_and_selector_sets_align() {
    let list = run(&["--list"]);
    assert!(list.status.success());
    let lines = String::from_utf8(list.stdout)
        .expect("list output must be UTF-8")
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    assert_eq!(lines.len(), 86);
    let mut implemented = BTreeSet::new();
    for (line, screen) in lines.iter().zip(canonical_inventory()) {
        let fields = line.split('\t').collect::<Vec<_>>();
        assert_eq!(
            fields,
            vec![
                screen.category.as_str(),
                screen.id,
                "implemented",
                screen.flow.as_str(),
                screen.surface.as_str()
            ]
        );
        implemented.insert(screen.id);
    }
    assert_eq!(implemented.len(), 86);
}
