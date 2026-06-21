use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

use format::{FormatOutcome, FormatterKind, FormatterRunner, binary_available};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn temp_dir(tag: &str) -> PathBuf {
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "cortex-format-real-{tag}-{}-{unique}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn run_manual(command: &mut Command) {
    let output = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn manual formatter");
    assert!(
        output.status.success(),
        "manual formatter failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn rustfmt_expected(raw: &str) -> String {
    let dir = temp_dir("rustfmt-expected");
    let path = dir.join("expected.rs");
    fs::write(&path, raw).expect("write rust input");
    run_manual(
        Command::new("rustfmt")
            .arg("--edition")
            .arg("2024")
            .arg(&path),
    );
    fs::read_to_string(path).expect("read rustfmt output")
}

fn prettier_expected(raw: &str, ext: &str) -> String {
    let dir = temp_dir("prettier-expected");
    fs::write(dir.join(".prettierrc"), "{}").expect("write prettier marker");
    let path = dir.join(format!("expected.{ext}"));
    fs::write(&path, raw).expect("write prettier input");
    run_manual(
        Command::new("prettier")
            .current_dir(&dir)
            .arg("--write")
            .arg(&path),
    );
    fs::read_to_string(path).expect("read prettier output")
}

fn ruff_expected(raw: &str) -> String {
    let dir = temp_dir("ruff-expected");
    fs::write(dir.join("pyproject.toml"), "[tool.ruff]\n").expect("write ruff marker");
    let path = dir.join("expected.py");
    fs::write(&path, raw).expect("write ruff input");
    run_manual(
        Command::new("ruff")
            .current_dir(&dir)
            .arg("format")
            .arg(&path),
    );
    fs::read_to_string(path).expect("read ruff output")
}

fn assert_unchanged(path: &Path, expected: &str) {
    assert_eq!(
        fs::read_to_string(path).expect("read unformatted file"),
        expected
    );
}

#[test]
fn rustfmt_formats_rust_files_with_edition_2024() {
    if !binary_available("rustfmt") {
        eprintln!("skipping real formatter test: rustfmt is not on PATH");
        return;
    }

    let dir = temp_dir("rustfmt");
    let path = dir.join("main.rs");
    let raw = "fn  main( ){let x=1;println!(\"{}\",x);}\n";
    fs::write(&path, raw).expect("write rust file");

    let outcome = FormatterRunner::new(&dir).format_path(&path);

    assert_eq!(outcome, FormatOutcome::Formatted(FormatterKind::Rustfmt));
    assert_ne!(fs::read_to_string(&path).expect("read rust file"), raw);
    assert_eq!(
        fs::read_to_string(&path).expect("read rust file"),
        rustfmt_expected(raw)
    );
}

#[test]
fn prettier_formats_supported_files_only_when_a_marker_is_present() {
    let no_marker_dir = temp_dir("prettier-no-marker");
    let no_marker_path = no_marker_dir.join("demo.ts");
    let raw = "const value={foo:\"bar\",answer:42};\n";
    fs::write(&no_marker_path, raw).expect("write ts file");

    let no_marker_outcome = FormatterRunner::new(&no_marker_dir).format_path(&no_marker_path);
    assert!(
        !matches!(
            no_marker_outcome,
            FormatOutcome::Formatted(FormatterKind::Prettier)
        ),
        "prettier must not run without a marker: {no_marker_outcome:?}"
    );
    assert_unchanged(&no_marker_path, raw);

    if !binary_available("prettier") {
        eprintln!("skipping configured prettier proof: prettier is not on PATH");
        return;
    }

    let dir = temp_dir("prettier-marker");
    fs::write(dir.join(".prettierrc"), "{}").expect("write prettier marker");
    let path = dir.join("demo.ts");
    fs::write(&path, raw).expect("write ts file");

    let outcome = FormatterRunner::new(&dir).format_path(&path);

    assert_eq!(outcome, FormatOutcome::Formatted(FormatterKind::Prettier));
    assert_ne!(fs::read_to_string(&path).expect("read ts file"), raw);
    assert_eq!(
        fs::read_to_string(&path).expect("read ts file"),
        prettier_expected(raw, "ts")
    );
}

#[test]
fn ruff_formats_python_only_when_project_config_indicates_ruff() {
    let no_marker_dir = temp_dir("ruff-no-marker");
    let no_marker_path = no_marker_dir.join("demo.py");
    let raw = "def demo():\n    value= {\"answer\":42}\n    return value\n";
    fs::write(&no_marker_path, raw).expect("write python file");

    let no_marker_outcome = FormatterRunner::new(&no_marker_dir).format_path(&no_marker_path);
    assert!(
        !matches!(
            no_marker_outcome,
            FormatOutcome::Formatted(FormatterKind::Ruff)
        ),
        "ruff must not run without a marker: {no_marker_outcome:?}"
    );
    assert_unchanged(&no_marker_path, raw);

    if !binary_available("ruff") {
        eprintln!("skipping configured ruff proof: ruff is not on PATH");
        return;
    }

    let dir = temp_dir("ruff-marker");
    fs::write(dir.join("pyproject.toml"), "[tool.ruff]\n").expect("write ruff marker");
    let path = dir.join("demo.py");
    fs::write(&path, raw).expect("write python file");

    let outcome = FormatterRunner::new(&dir).format_path(&path);

    assert_eq!(outcome, FormatOutcome::Formatted(FormatterKind::Ruff));
    assert_ne!(fs::read_to_string(&path).expect("read python file"), raw);
    assert_eq!(
        fs::read_to_string(&path).expect("read python file"),
        ruff_expected(raw)
    );
}
