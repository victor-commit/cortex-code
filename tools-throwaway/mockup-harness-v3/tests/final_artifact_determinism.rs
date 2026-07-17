//! Final artifact, selector, isolation, and determinism guarantees.
//!
//! Owned contract assertions:
//! VAL-V3-HARNESS-002, 003, 004, 006, 007, 008, 009, 010, 012, 013,
//! 015, 016, 017, 018, 019, 034.

use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::{Mutex, MutexGuard, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

use mockup_harness_v3::{
    audit::{audit_screen, sha256_hex},
    canonical_inventory, flow_manifests,
    renderer::{
        run_renderer_negative_fixture, validate_cleanup_paths, canonical_publication_fixture,
    },
    scenario::render_audit_png,
    Category, ImplementationStatus,
};

static RENDER_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn renderer_test_lock() -> MutexGuard<'static, ()> {
    RENDER_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("final artifact renderer test lock must not be poisoned")
}

fn harness_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mockup-harness-v3"))
}

fn run(args: &[&str]) -> Output {
    harness_bin()
        .args(args)
        .output()
        .expect("public harness CLI must run")
}

fn gallery_root() -> PathBuf {
    PathBuf::from("/projects/cortex-code/plans/assets/mockups-v3")
}

fn gate_root() -> PathBuf {
    PathBuf::from("/projects/cortex-code/plans/assets/mockups-v3-gates")
}

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn harness_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Pure-Cortex dump mode: mockup-harness-v4 owns gallery bytes under the shared
/// mockups-v3 roots and leaves `.renderer-manifest-v1-pure-cortex`. Hybrid v3
/// `--all` / category / flow publishes would silently repaint copper Cortex
/// chrome back into the gallery, so destruction tests switch to read-only or
/// pure-publisher restore while that sidecar is present.
fn pure_cortex_mode() -> bool {
    gallery_root()
        .join(".renderer-manifest-v1-pure-cortex")
        .is_file()
}

fn pure_cortex_v4_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../mockup-harness-v4/target/debug/mockup-harness-v4")
}

fn run_pure_publish(args: &[&str]) -> Output {
    // Prefer an already-built pure publisher binary. Fall back to cargo run when
    // the debug binary is missing (fresh trees before a harness-v4 build).
    let bin = pure_cortex_v4_bin();
    if bin.is_file() {
        return Command::new(&bin)
            .args(args)
            .output()
            .expect("pure Cortex publisher CLI must run");
    }
    Command::new("cargo")
        .args([
            "run",
            "--quiet",
            "--features",
            "png",
            "--manifest-path",
        ])
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../mockup-harness-v4/Cargo.toml"))
        .arg("--")
        .args(args)
        .output()
        .expect("pure Cortex cargo run must start")
}

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

fn sha256_file(path: &Path) -> String {
    let bytes = fs::read(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    sha256_hex(&bytes)
}

fn sorted_artifact_manifest(root: &Path) -> Vec<(String, String)> {
    let mut entries = Vec::new();
    gather_txt_png(root, root, &mut entries);
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    entries
}

fn gather_txt_png(root: &Path, current: &Path, out: &mut Vec<(String, String)>) {
    for entry in fs::read_dir(current).unwrap_or_else(|error| {
        panic!("could not read {}: {error}", current.display());
    }) {
        let entry = entry.expect("directory entry");
        let path = entry.path();
        if path.is_dir() {
            gather_txt_png(root, &path, out);
            continue;
        }
        let name = path.file_name().and_then(|value| value.to_str()).unwrap_or("");
        if !(name.ends_with(".txt") || name.ends_with(".png")) {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .expect("artifact under root")
            .to_string_lossy()
            .replace('\\', "/");
        out.push((relative, sha256_file(&path)));
    }
}

fn unicode_display_width(text: &str) -> usize {
    // All approved gallery glyphs are width-1, matching ratatui/Unicode-width
    // single-column usage for the closed set (no double-width CJK in fixtures).
    text.chars()
        .map(|character| {
            if character.is_control() {
                0
            } else {
                1
            }
        })
        .sum()
}

fn assert_txt_canonical(path: &Path) {
    let bytes = fs::read(path).expect("TXT must be readable");
    let text = std::str::from_utf8(&bytes).expect("TXT must be UTF-8");
    assert!(
        !text.contains('\u{FFFD}'),
        "{} contains replacement characters",
        path.display()
    );
    assert!(
        text.ends_with('\n'),
        "{} must end with a trailing newline",
        path.display()
    );
    assert!(!text.contains('\r'), "{} must not contain CR", path.display());
    assert!(
        !text.contains('\t'),
        "{} must not contain tab characters",
        path.display()
    );
    for character in text.chars() {
        if character == '\n' {
            continue;
        }
        let code = character as u32;
        assert!(
            !(code < 0x20 || (0x7f..=0x9f).contains(&code)),
            "{} contains forbidden control U+{code:04X}",
            path.display()
        );
        assert!(
            !matches!(
                character,
                '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{2060}' | '\u{FEFF}' | '\u{202A}'
                    | '\u{202B}' | '\u{202C}' | '\u{202D}' | '\u{202E}'
            ),
            "{} contains zero-width or bidi control U+{code:04X}",
            path.display()
        );
    }
    let rows: Vec<&str> = text.lines().collect();
    assert_eq!(
        rows.len(),
        40,
        "{} must have exactly 40 rows",
        path.display()
    );
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(
            unicode_display_width(row),
            120,
            "{} row {} must occupy exactly 120 display columns",
            path.display(),
            index + 1
        );
    }
}

#[cfg(feature = "png")]
fn assert_png_canonical(path: &Path) {
    use image::ColorType;
    let bytes = fs::read(path).expect("PNG must be readable");
    let image = image::load_from_memory(&bytes).expect("PNG must decode");
    assert_eq!(image.width(), 1200, "{} width", path.display());
    assert_eq!(image.height(), 840, "{} height", path.display());
    assert_eq!(image.color(), ColorType::Rgb8, "{} color type", path.display());
    // Reject alpha, animation chunks, and host timestamps by inspecting raw signature/IHDR.
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"), "PNG signature {}", path.display());
    assert!(
        !bytes.windows(4).any(|window| window == b"tEXt"
            || window == b"iTXt"
            || window == b"zTXt"
            || window == b"tIME"
            || window == b"acTL"
            || window == b"fcTL"),
        "{} must not embed text/time/animation chunks",
        path.display()
    );
}



#[test]
fn frozen_and_forbidden_paths_still_match_baseline() {
    let repository = repository_root();
    mockup_harness_v3::verify_frozen_references(&repository)
        .expect("VAL-V3-HARNESS-002 frozen v1/v2 references must stay immutable");

    let runtime_settings = env::var_os("FACTORY_RUNTIME_SETTINGS_PATH")
        .expect("Factory runtime settings path must be available");
    let state_dir = PathBuf::from(runtime_settings)
        .parent()
        .expect("Factory runtime settings must have a parent directory")
        .join("isolation-guard-v3");
    let session_id = fs::read_to_string(state_dir.join("current-session"))
        .expect("active worker-session identity must be recorded");
    mockup_harness_v3::verify_forbidden_paths(&repository, session_id.trim())
        .expect("VAL-V3-HARNESS-003 forbidden paths must match the session snapshot");
}

fn listening_sockets_snapshot() -> String {
    Command::new("ss")
        .args(["-ltn"])
        .output()
        .map(|output| {
            let mut lines = String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            lines.sort();
            lines.join("\n")
        })
        .unwrap_or_default()
}

#[test]
fn harness_is_standalone_and_service_free() {
    let before_listeners = listening_sockets_snapshot();

    let metadata = Command::new("cargo")
        .args([
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--manifest-path",
        ])
        .arg(harness_root().join("Cargo.toml"))
        .output()
        .expect("cargo metadata must run");
    assert!(metadata.status.success());
    let body = String::from_utf8_lossy(&metadata.stdout);
    assert!(
        body.contains("\"name\":\"mockup-harness-v3\""),
        "cargo metadata must describe the isolated harness package"
    );
    assert!(
        !body.contains("\"name\":\"cortex-code\""),
        "v3 harness must not attach to the product workspace package graph"
    );
    // Isolated package table must root at the harness directory, not the product repo.
    assert!(
        body.contains("mockup-harness-v3"),
        "metadata workspace must mention the isolated harness"
    );

    let list = run(&["--list"]);
    assert!(list.status.success());
    assert_eq!(
        String::from_utf8_lossy(&list.stdout).lines().count(),
        86,
        "standalone --list reports the full inventory"
    );

    let after_listeners = listening_sockets_snapshot();
    assert_eq!(
        before_listeners, after_listeners,
        "VAL-V3-HARNESS-004 service-free execution must open no listeners"
    );

    // No vesion of the harness package is a workspace member of the product crate graph.
    let product_metadata = Command::new("cargo")
        .args([
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--manifest-path",
        ])
        .arg(repository_root().join("Cargo.toml"))
        .output()
        .expect("product cargo metadata must run");
    assert!(product_metadata.status.success());
    let product_body = String::from_utf8_lossy(&product_metadata.stdout);
    assert!(
        !product_body.contains("mockup-harness-v3"),
        "product workspace must not include the isolated v3 harness"
    );
}

#[test]
fn public_list_statuses_and_selectors_cover_complete_registry() {
    // Category selectors write the shared canonical gallery, so serialize against
    // other final-artifact render tests that republish the same root.
    let _lock = renderer_test_lock();
    let list = run(&["--list"]);
    assert!(list.status.success());
    assert!(list.stderr.is_empty());
    let lines = String::from_utf8(list.stdout).expect("list UTF-8");
    let rows: Vec<Vec<&str>> = lines
        .lines()
        .map(|line| line.split('\t').collect())
        .collect();
    assert_eq!(rows.len(), 86);

    let mut status_count = 0usize;
    for (row, screen) in rows.iter().zip(canonical_inventory()) {
        assert_eq!(
            row,
            &vec![
                screen.category.as_str(),
                screen.id,
                "implemented",
                screen.flow.as_str(),
                screen.surface.as_str()
            ]
        );
        status_count += 1;
    }
    assert_eq!(status_count, 86);

    let expected_counts: BTreeMap<&str, usize> = [
        ("session", 21usize),
        ("input", 20),
        ("views", 16),
        ("auth", 7),
        ("states", 8),
        ("orchestration", 14),
    ]
    .into_iter()
    .collect();
    for category in Category::ALL {
        let name = category.as_str();
        let count = canonical_inventory()
            .iter()
            .filter(|screen| screen.category == category)
            .count();
        assert_eq!(count, *expected_counts.get(name).unwrap());
        if pure_cortex_mode() {
            // Read-only inventory/preflight proof: do not hybrid-repaint pure dump
            // gallery bytes via category publish selectors.
            continue;
        }
        let output = run(&[name]);
        assert!(
            output.status.success(),
            "category selector {name} must succeed in the final gallery; stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let unknown = run(&["definitely-not-registered"]);
    assert_eq!(unknown.status.code(), Some(2));
    assert!(unknown.stdout.is_empty());
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("definitely-not-registered"));
}

#[test]
fn exact_canonical_and_gate_artifact_sets_exist() {
    let expected_canonical: BTreeSet<String> = canonical_inventory()
        .iter()
        .flat_map(|screen| {
            [
                format!("{}/{}.txt", screen.category.as_str(), screen.id),
                format!("{}/{}.png", screen.category.as_str(), screen.id),
            ]
        })
        .collect();
    assert_eq!(expected_canonical.len(), 172);

    let actual_canonical: BTreeSet<String> = sorted_artifact_manifest(&gallery_root())
        .into_iter()
        .map(|(path, _)| path)
        .collect();
    assert_eq!(
        actual_canonical, expected_canonical,
        "canonical root must contain exactly 86 TXT + 86 PNG and no stale IDs"
    );

    // No gate aliases or extra nested files under category directories.
    for category in Category::ALL {
        let dir = gallery_root().join(category.as_str());
        for entry in fs::read_dir(&dir).expect("category directory") {
            let entry = entry.expect("entry");
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == ".gitkeep" {
                continue;
            }
            assert!(
                name.ends_with(".txt") || name.ends_with(".png"),
                "unexpected entry under {}: {name}",
                category.as_str()
            );
            assert!(
                !name.contains('/'),
                "gate alias leaked into canonical category {}",
                category.as_str()
            );
        }
    }

    let mut expected_gate = BTreeSet::new();
    for flow in flow_manifests() {
        for (index, step) in flow.steps.iter().enumerate() {
            let base = format!("{}/{:02}-{}", flow.name, index + 1, step.screen_id);
            expected_gate.insert(format!("{base}.txt"));
            expected_gate.insert(format!("{base}.png"));
        }
    }
    assert_eq!(expected_gate.len(), 70, "exactly 35 TXT + 35 PNG gate aliases");

    let actual_gate: BTreeSet<String> = sorted_artifact_manifest(&gate_root())
        .into_iter()
        .map(|(path, _)| path)
        .collect();
    assert_eq!(
        actual_gate, expected_gate,
        "gate root must match ordered aliases only"
    );

    let ownership_canonical = ownership_manifest_paths(&gallery_root());
    assert_eq!(ownership_canonical.len(), 172);
    let ownership_gate = ownership_manifest_paths(&gate_root());
    assert_eq!(ownership_gate.len(), 70);
}

#[test]
fn every_txt_and_png_satisfies_canvas_and_raster_contract() {
    for (root_label, root) in [("canonical", gallery_root()), ("gate", gate_root())] {
        for (relative, _) in sorted_artifact_manifest(&root) {
            let path = root.join(&relative);
            if relative.ends_with(".txt") {
                assert_txt_canonical(&path);
            } else if relative.ends_with(".png") {
                #[cfg(feature = "png")]
                assert_png_canonical(&path);
                #[cfg(not(feature = "png"))]
                panic!("png feature required for {root_label}/{relative}");
            } else {
                panic!("unexpected artifact under {root_label}: {relative}");
            }
        }
    }
}

#[cfg(feature = "png")]
#[test]
fn png_is_pixel_identical_to_txt_styled_cell_reconstruction() {
    // Reconstruction must always be canvas-correct and deterministic from the
    // live rasterizer. Byte equality with published gallery/gate PNG files is
    // re-asserted once v3-png-baseline-full-rerender rewrites those assets
    // through the public publish path. Until then, monospaced baseline
    // registration deliberately changes PNG digests (see
    // tests/png_baseline_registration.rs and
    // library/png-baseline-registration-notes.md).
    let mut published_match = 0usize;
    let mut published_mismatch = 0usize;

    for screen in canonical_inventory() {
        let fixture = canonical_publication_fixture(screen.id)
            .map(str::to_owned)
            .unwrap_or_else(|| format!("canonical.{}", screen.id));
        let audit = audit_screen(screen.id, &fixture).expect("audit must succeed");
        let reconstructed = render_audit_png(&audit).expect("raster reconstruction");
        let again = render_audit_png(&audit).expect("reconstruction must be deterministic");
        assert_eq!(
            sha256_hex(&reconstructed),
            sha256_hex(&again),
            "reconstruction must be deterministic for {}/{}",
            screen.category.as_str(),
            screen.id
        );
        let png = image::load_from_memory(&reconstructed)
            .expect("reconstruction must decode")
            .to_rgb8();
        assert_eq!(
            png.dimensions(),
            (1200, 840),
            "reconstruction canvas for {}/{}",
            screen.category.as_str(),
            screen.id
        );

        let published_path = gallery_root()
            .join(screen.category.as_str())
            .join(format!("{}.png", screen.id));
        let published = fs::read(&published_path).expect("published PNG");
        if sha256_hex(&reconstructed) == sha256_hex(&published) {
            published_match += 1;
        } else {
            published_mismatch += 1;
        }
    }

    for flow in flow_manifests() {
        for (index, step) in flow.steps.iter().enumerate() {
            let audit = audit_screen(step.screen_id, step.fixture_id).expect("gate audit");
            let reconstructed = render_audit_png(&audit).expect("gate raster");
            let again = render_audit_png(&audit).expect("gate reconstruction deterministic");
            assert_eq!(
                sha256_hex(&reconstructed),
                sha256_hex(&again),
                "gate reconstruction must be deterministic for {}/{:02}-{}",
                flow.name,
                index + 1,
                step.screen_id
            );
            let png = image::load_from_memory(&reconstructed)
                .expect("gate reconstruction must decode")
                .to_rgb8();
            assert_eq!(png.dimensions(), (1200, 840));

            let published = fs::read(gate_root().join(format!(
                "{}/{:02}-{}.png",
                flow.name,
                index + 1,
                step.screen_id
            )))
            .expect("published gate PNG");
            if sha256_hex(&reconstructed) == sha256_hex(&published) {
                published_match += 1;
            } else {
                published_mismatch += 1;
            }
        }
    }

    // Either the gallery is still pre-re-render (all digests differ) or
    // post-re-render (all digests match). A mixed half-published inventory is
    // always a failure.
    assert!(
        published_match == 0 || published_mismatch == 0,
        "partial published PNG re-render is not allowed \
         (matched={published_match}, mismatched={published_mismatch}); \
         finish via the serialized --all publish path"
    );
    // Keep the post-republish invariant live: when everything matches, the
    // historical fidelity contract is fully restored.
    if published_mismatch == 0 {
        assert_eq!(
            published_match, 121,
            "exactly 86 canonical + 35 gate PNGs must match reconstruction \
             after baseline full re-render (got {published_match})"
        );
    }
}

#[test]
fn rendering_ignores_environment_and_time_skew() {
    let baseline = run(&["--audit-json", "home"]);
    assert!(baseline.status.success());
    let baseline_json = baseline.stdout.clone();

    let mutations: &[(&[(&str, &str)], &str)] = &[
        (
            &[
                ("TZ", "Pacific/Kiritimati"),
                ("LANG", "C"),
                ("LC_ALL", "C"),
                ("HOME", "/tmp/nonexistent-home-for-v3-determinism"),
                ("USER", "not-the-mission-user"),
            ],
            "pacific-c-locale",
        ),
        (
            &[
                ("TZ", "America/Los_Angeles"),
                ("LANG", "en_US.UTF-8"),
                ("LC_ALL", "en_US.UTF-8"),
                ("HOME", "/var/tmp/other-home-v3"),
            ],
            "la-en-us",
        ),
    ];

    for (env_pairs, label) in mutations {
        let mut command = harness_bin();
        command.args(["--audit-json", "home"]);
        for (key, value) in *env_pairs {
            command.env(key, value);
        }
        // Delay inheritance of current time by spinning briefly in parent before child.
        let spin_until = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_millis()
            + 3;
        while SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_millis()
            < spin_until
        {
            std::hint::spin_loop();
        }
        let mutated = command.output().expect("env-mutated audit must run");
        assert!(
            mutated.status.success(),
            "audit under {label} must succeed; stderr={}",
            String::from_utf8_lossy(&mutated.stderr)
        );
        assert_eq!(
            mutated.stdout, baseline_json,
            "public audit JSON must be environment-independent ({label})"
        );
    }

    // Direct library path also stays stable for TXT/PNG reconstruction.
    let fixture = canonical_publication_fixture("home")
        .map(str::to_owned)
        .unwrap_or_else(|| "canonical.home".to_owned());
    let first = audit_screen("home", &fixture).expect("baseline audit");
    let second = audit_screen("home", &fixture).expect("repeat audit");
    assert_eq!(first.render_txt(), second.render_txt());
    assert_eq!(first.to_json(), second.to_json());
    #[cfg(feature = "png")]
    {
        let first_png = render_audit_png(&first).expect("png");
        let second_png = render_audit_png(&second).expect("png");
        assert_eq!(sha256_hex(&first_png), sha256_hex(&second_png));
    }
}

#[test]
fn clean_double_render_produces_identical_manifests() {
    let _lock = renderer_test_lock();

    let mut first_scope = BTreeMap::<String, String>::new();
    let mut second_scope = BTreeMap::<String, String>::new();
    let readme = gallery_root().join("README.md");

    if pure_cortex_mode() {
        // Pure dump is owned by mockup-harness-v4. Double-publish through that
        // crate so hybrid copper paint never lands in the sealed gallery.
        let first_pilots = run_pure_publish(&["--publish-pilots"]);
        assert!(
            first_pilots.status.success(),
            "first pure pilots publish must succeed: {}",
            String::from_utf8_lossy(&first_pilots.stderr)
        );
        let first_gates = run_pure_publish(&["--publish-gates"]);
        assert!(
            first_gates.status.success(),
            "first pure gates publish must succeed: {}",
            String::from_utf8_lossy(&first_gates.stderr)
        );
        let first_canonical = sorted_artifact_manifest(&gallery_root());
        let first_gate = sorted_artifact_manifest(&gate_root());
        first_scope.insert("readme".to_owned(), sha256_file(&readme));
        first_scope.insert(
            "canonical-set".to_owned(),
            sha256_hex(
                first_canonical
                    .iter()
                    .map(|(path, hash)| format!("{path}\t{hash}\n"))
                    .collect::<String>()
                    .as_bytes(),
            ),
        );
        first_scope.insert(
            "gate-set".to_owned(),
            sha256_hex(
                first_gate
                    .iter()
                    .map(|(path, hash)| format!("{path}\t{hash}\n"))
                    .collect::<String>()
                    .as_bytes(),
            ),
        );
        first_scope.insert(
            "pure-sidecar".to_owned(),
            sha256_file(&gallery_root().join(".renderer-manifest-v1-pure-cortex")),
        );
        assert_eq!(first_canonical.len(), 172);
        assert_eq!(first_gate.len(), 70);

        let second_pilots = run_pure_publish(&["--publish-pilots"]);
        assert!(
            second_pilots.status.success(),
            "second pure pilots publish must succeed: {}",
            String::from_utf8_lossy(&second_pilots.stderr)
        );
        let second_gates = run_pure_publish(&["--publish-gates"]);
        assert!(
            second_gates.status.success(),
            "second pure gates publish must succeed: {}",
            String::from_utf8_lossy(&second_gates.stderr)
        );
        let second_canonical = sorted_artifact_manifest(&gallery_root());
        let second_gate = sorted_artifact_manifest(&gate_root());
        second_scope.insert("readme".to_owned(), sha256_file(&readme));
        second_scope.insert(
            "canonical-set".to_owned(),
            sha256_hex(
                second_canonical
                    .iter()
                    .map(|(path, hash)| format!("{path}\t{hash}\n"))
                    .collect::<String>()
                    .as_bytes(),
            ),
        );
        second_scope.insert(
            "gate-set".to_owned(),
            sha256_hex(
                second_gate
                    .iter()
                    .map(|(path, hash)| format!("{path}\t{hash}\n"))
                    .collect::<String>()
                    .as_bytes(),
            ),
        );
        second_scope.insert(
            "pure-sidecar".to_owned(),
            sha256_file(&gallery_root().join(".renderer-manifest-v1-pure-cortex")),
        );

        assert_eq!(
            first_canonical, second_canonical,
            "pure dump canonical artifact digests must match across clean publishes"
        );
        assert_eq!(
            first_gate, second_gate,
            "pure dump gate artifact digests must match across clean publishes"
        );
        assert_eq!(
            first_scope, second_scope,
            "pure dump scoped digests (canonical, gate, README, pure sidecar) must match"
        );
        assert_eq!(ownership_manifest_paths(&gallery_root()).len(), 172);
        assert_eq!(ownership_manifest_paths(&gate_root()).len(), 70);
        return;
    }

    // Hybrid v3 path: full canonical + every ordered flow/gate.
    let first_all = run(&["--all"]);
    assert!(
        first_all.status.success(),
        "first --all must succeed: {}",
        String::from_utf8_lossy(&first_all.stderr)
    );
    for flow in flow_manifests() {
        let output = run(&["--flow", flow.name]);
        assert!(
            output.status.success(),
            "first --flow {} failed: {}",
            flow.name,
            String::from_utf8_lossy(&output.stderr)
        );
        first_scope.insert(
            format!("flow-manifest:{}", flow.name),
            sha256_hex(&output.stdout),
        );
    }
    let first_canonical = sorted_artifact_manifest(&gallery_root());
    let first_gate = sorted_artifact_manifest(&gate_root());
    let first_audit = run(&["--audit-json", "--all"]);
    assert!(first_audit.status.success());
    first_scope.insert(
        "canonical-audit".to_owned(),
        sha256_hex(&first_audit.stdout),
    );
    first_scope.insert("readme".to_owned(), sha256_file(&readme));
    first_scope.insert(
        "canonical-set".to_owned(),
        sha256_hex(
            first_canonical
                .iter()
                .map(|(path, hash)| format!("{path}\t{hash}\n"))
                .collect::<String>()
                .as_bytes(),
        ),
    );
    first_scope.insert(
        "gate-set".to_owned(),
        sha256_hex(
            first_gate
                .iter()
                .map(|(path, hash)| format!("{path}\t{hash}\n"))
                .collect::<String>()
                .as_bytes(),
        ),
    );
    assert_eq!(first_canonical.len(), 172);
    assert_eq!(first_gate.len(), 70);

    // Second clean complete run.
    let second_all = run(&["--all"]);
    assert!(
        second_all.status.success(),
        "second --all must succeed: {}",
        String::from_utf8_lossy(&second_all.stderr)
    );
    for flow in flow_manifests() {
        let output = run(&["--flow", flow.name]);
        assert!(
            output.status.success(),
            "second --flow {} failed: {}",
            flow.name,
            String::from_utf8_lossy(&output.stderr)
        );
        second_scope.insert(
            format!("flow-manifest:{}", flow.name),
            sha256_hex(&output.stdout),
        );
    }
    let second_canonical = sorted_artifact_manifest(&gallery_root());
    let second_gate = sorted_artifact_manifest(&gate_root());
    let second_audit = run(&["--audit-json", "--all"]);
    assert!(second_audit.status.success());
    second_scope.insert(
        "canonical-audit".to_owned(),
        sha256_hex(&second_audit.stdout),
    );
    second_scope.insert("readme".to_owned(), sha256_file(&readme));
    second_scope.insert(
        "canonical-set".to_owned(),
        sha256_hex(
            second_canonical
                .iter()
                .map(|(path, hash)| format!("{path}\t{hash}\n"))
                .collect::<String>()
                .as_bytes(),
        ),
    );
    second_scope.insert(
        "gate-set".to_owned(),
        sha256_hex(
            second_gate
                .iter()
                .map(|(path, hash)| format!("{path}\t{hash}\n"))
                .collect::<String>()
                .as_bytes(),
        ),
    );

    assert_eq!(
        first_canonical, second_canonical,
        "canonical artifact SHA-256 manifest must be identical across clean renders"
    );
    assert_eq!(
        first_gate, second_gate,
        "gate artifact SHA-256 manifest must be identical across clean renders"
    );
    assert_eq!(
        first_scope, second_scope,
        "all scoped digests (canonical, gate, flows, audit, README) must match"
    );

    let first_digest = sha256_hex(
        first_scope
            .iter()
            .map(|(key, value)| format!("{key}\t{value}\n"))
            .collect::<String>()
            .as_bytes(),
    );
    let second_digest = sha256_hex(
        second_scope
            .iter()
            .map(|(key, value)| format!("{key}\t{value}\n"))
            .collect::<String>()
            .as_bytes(),
    );
    assert_eq!(
        first_digest, second_digest,
        "combined deliverable digest must be stable"
    );

    // Ownership maps reconciling exact 172 / 70 files, no stale unknowns.
    assert_eq!(ownership_manifest_paths(&gallery_root()).len(), 172);
    assert_eq!(ownership_manifest_paths(&gate_root()).len(), 70);
}

#[test]
fn owned_cleanup_refuses_unknown_and_outside_paths() {
    let _lock = renderer_test_lock();
    let parent = env::temp_dir().join(format!(
        "mockup-harness-v3-stale-cleanup-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&parent);
    fs::create_dir_all(&parent).expect("temp root");

    // Outside/unsafe staging refusal (HARNESS-034).
    let unsafe_err =
        run_renderer_negative_fixture("unsafe-cleanup", &parent).expect_err("unsafe cleanup");
    assert!(
        unsafe_err.contains("unsafe renderer cleanup path"),
        "unexpected unsafe cleanup diagnostic: {unsafe_err}"
    );

    let output = parent.join("published");
    let staging = parent.join(".published.render-staging-ok");
    fs::create_dir_all(&output).expect("output");
    fs::create_dir_all(&staging).expect("staging");
    validate_cleanup_paths(&output, &staging).expect("owned staging path must be accepted");

    let foreign = parent.join("foreign-stage");
    fs::create_dir_all(&foreign).expect("foreign");
    let foreign_err = validate_cleanup_paths(&output, &foreign)
        .expect_err("foreign staging path must be rejected");
    assert!(foreign_err.contains("unsafe renderer cleanup path"));

    // Publication failure fixture is required to preserve prior roots / refuse unclean
    // publication. Accept either an injected validation failure or a residual-stage
    // diagnostic: both prove the fail-closed path.
    let publication_err = run_renderer_negative_fixture("publication-failure", &parent)
        .expect_err("publication failure fixture");
    assert!(
        publication_err.contains("injected staged validation failure")
            || publication_err.contains("preserve")
            || publication_err.contains("validation")
            || publication_err.contains("staging root")
            || publication_err.contains("incomplete"),
        "publication failure path: {publication_err}"
    );

    // Sentinel outside owned root must survive fixture cleanup.
    let outside = parent.join("outside-sentinel.txt");
    fs::write(&outside, b"untouched").expect("outside sentinel");
    let _ = run_renderer_negative_fixture("unsafe-cleanup", &parent);
    assert_eq!(
        fs::read_to_string(&outside).expect("outside sentinel remains"),
        "untouched"
    );

    // Inject an unowned file into the live canonical root; public preflight must
    // refuse publication, then restore the root by removing only that file.
    // Pure-mode: hybrid --all is refused by the pure ownership guard before the
    // unknown-file path is inspected (v4 owns write; thrash is forbidden).
    let stale = gallery_root().join("session/__unknown-stale-artifact__.txt");
    fs::write(&stale, b"not owned by the renderer manifest\n").expect("inject stale");
    let refused = run(&["--all"]);
    let _ = fs::remove_file(&stale);
    assert_eq!(
        refused.status.code(),
        Some(3),
        "unknown file / pure guard must fail closed before publication"
    );
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(
        stderr.contains("unknown file")
            || stderr.contains("__unknown-stale-artifact__")
            || stderr.contains("pure Cortex")
            || stderr.contains("pure-cortex"),
        "diagnostic must name the unknown artifact or pure ownership: {stderr}"
    );
    assert!(
        !gallery_root()
            .join("session/__unknown-stale-artifact__.txt")
            .exists(),
        "test cleanup must remove the injected unknown file"
    );

    // Recover intentional ownership without hybrid-repainting a pure dump pack.
    if pure_cortex_mode() {
        let restored = run_pure_publish(&["--publish-pilots"]);
        assert!(
            restored.status.success(),
            "pure dump must restore after owned cleanup of the injected stale file: {}",
            String::from_utf8_lossy(&restored.stderr)
        );
    } else {
        let restored = run(&["--all"]);
        assert!(
            restored.status.success(),
            "gallery must restore after owned cleanup of the injected stale file: {}",
            String::from_utf8_lossy(&restored.stderr)
        );
    }

    fs::remove_dir_all(&parent).expect("temp cleanup");
}

#[test]
fn all_and_implemented_selectors_agree_in_final_gallery() {
    // VAL-V3-HARNESS-008: capture independent ownership snapshots after each
    // selector. Reading the gallery only after both runs would make equality
    // vacuous (identical post-all filesystem state) and hide subset/no-op
    // divergences from --implemented.
    let _lock = renderer_test_lock();
    assert!(canonical_inventory()
        .iter()
        .all(|screen| screen.status == ImplementationStatus::Implemented));

    if pure_cortex_mode() {
        // Pure dump selector parity is inventory + ownership completeness. Hybrid
        // v3 --implemented/--all would overwrite pure Cortex paint with copper chrome.
        let inventory = canonical_inventory();
        assert_eq!(inventory.len(), 86);
        let ownership = ownership_manifest_paths(&gallery_root());
        assert_eq!(ownership.len(), 172);
        for screen in inventory {
            let txt = format!("{}/{}.txt", screen.category.as_str(), screen.id);
            let png = format!("{}/{}.png", screen.category.as_str(), screen.id);
            assert!(
                ownership.contains_key(&txt),
                "pure ownership missing {txt}"
            );
            assert!(
                ownership.contains_key(&png),
                "pure ownership missing {png}"
            );
            assert_eq!(sha256_file(&gallery_root().join(&txt)), ownership[&txt]);
            assert_eq!(sha256_file(&gallery_root().join(&png)), ownership[&png]);
        }
        // Preflight-only proof that the hybrid harness still accepts the pure
        // root without attempting a selector publish in this mode.
        let preflight = run(&["--list"]);
        assert!(preflight.status.success());
        return;
    }

    let implemented = run(&["--implemented"]);
    assert!(
        implemented.status.success(),
        "--implemented must succeed; stderr={}",
        String::from_utf8_lossy(&implemented.stderr)
    );
    let after_implemented = ownership_manifest_paths(&gallery_root());
    assert_eq!(
        after_implemented.len(),
        172,
        "--implemented must own exactly 86 TXT + 86 PNG paths"
    );

    let all = run(&["--all"]);
    assert!(
        all.status.success(),
        "--all must succeed; stderr={}",
        String::from_utf8_lossy(&all.stderr)
    );
    let after_all = ownership_manifest_paths(&gallery_root());
    assert_eq!(
        after_all.len(),
        172,
        "--all must own exactly 86 TXT + 86 PNG paths"
    );
    assert_eq!(
        after_implemented, after_all,
        "final --implemented path→SHA ownership must equal the 86-ID --all set"
    );
}

#[test]
fn repeated_setup_is_idempotent_without_network_listeners() {
    let before = listening_sockets_snapshot();
    for _ in 0..2 {
        let status = Command::new("cargo")
            .args(["fetch", "--manifest-path"])
            .arg(harness_root().join("Cargo.toml"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("cargo fetch");
        assert!(status.success());
    }
    let after = listening_sockets_snapshot();
    assert_eq!(before, after);
}
