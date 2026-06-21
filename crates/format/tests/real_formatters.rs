use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

use format::{FormatOutcome, FormatterConfig, FormatterKind, FormatterRunner, binary_available};

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

#[cfg(unix)]
fn write_executable(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;

    fs::write(path, body).expect("write executable script");
    let mut permissions = fs::metadata(path).expect("script metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("chmod executable script");
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

#[cfg(unix)]
#[test]
fn custom_formatter_override_from_config_is_used_for_matching_extension() {
    let dir = temp_dir("custom-uppercase");
    let script = dir.join("upper.sh");
    write_executable(
        &script,
        "#!/usr/bin/env sh\nfile=\"$1\"\ntr '[:lower:]' '[:upper:]' < \"$file\" > \"$file.tmp\"\nmv \"$file.tmp\" \"$file\"\n",
    );
    let path = dir.join("demo.foo");
    fs::write(&path, "hello custom formatter\n").expect("write custom input");
    let config = FormatterConfig::from_table(
        toml::from_str::<toml::Value>(&format!(
            "[formatter.custom]\ncommand = [\"{}\", \"$FILE\"]\nextensions = [\".foo\"]\n",
            script.display()
        ))
        .unwrap()
        .as_table()
        .unwrap(),
    )
    .expect("parse formatter config");

    let outcome = FormatterRunner::with_config(&dir, config).format_path(&path);

    assert_eq!(outcome, FormatOutcome::Formatted(FormatterKind::Custom));
    assert_eq!(
        fs::read_to_string(&path).expect("read custom formatted file"),
        "HELLO CUSTOM FORMATTER\n"
    );
}

#[cfg(unix)]
#[test]
fn custom_formatter_override_takes_precedence_over_builtin_detection() {
    let dir = temp_dir("custom-over-rustfmt");
    let script = dir.join("upper.sh");
    write_executable(
        &script,
        "#!/usr/bin/env sh\nfile=\"$1\"\ntr '[:lower:]' '[:upper:]' < \"$file\" > \"$file.tmp\"\nmv \"$file.tmp\" \"$file\"\n",
    );
    let path = dir.join("main.rs");
    fs::write(&path, "fn custom() {}\n").expect("write rust input");
    let config = FormatterConfig::from_table(
        toml::from_str::<toml::Value>(&format!(
            "[formatter.custom]\ncommand = [\"{}\", \"$FILE\"]\nextensions = [\".rs\"]\n",
            script.display()
        ))
        .unwrap()
        .as_table()
        .unwrap(),
    )
    .expect("parse formatter config");

    let outcome = FormatterRunner::with_config(&dir, config).format_path(&path);

    assert_eq!(outcome, FormatOutcome::Formatted(FormatterKind::Custom));
    assert_eq!(
        fs::read_to_string(&path).expect("read custom formatted rust"),
        "FN CUSTOM() {}\n",
        "custom override must replace built-in rustfmt detection"
    );
}

#[cfg(unix)]
#[test]
fn formatter_failure_restores_the_raw_written_bytes() {
    let dir = temp_dir("custom-failure-restore");
    let script = dir.join("corrupt_then_fail.sh");
    write_executable(
        &script,
        "#!/usr/bin/env sh\nfile=\"$1\"\nprintf 'CORRUPTED\\n' > \"$file\"\nprintf 'custom formatter failed\\n' >&2\nexit 7\n",
    );
    let path = dir.join("demo.foo");
    let raw = "raw bytes that must survive\n";
    fs::write(&path, raw).expect("write custom input");
    let config = FormatterConfig::from_table(
        toml::from_str::<toml::Value>(&format!(
            "[formatter.custom]\ncommand = [\"{}\", \"$FILE\"]\nextensions = [\".foo\"]\n",
            script.display()
        ))
        .unwrap()
        .as_table()
        .unwrap(),
    )
    .expect("parse formatter config");

    let outcome = FormatterRunner::with_config(&dir, config).format_path(&path);

    assert!(
        matches!(
            outcome,
            FormatOutcome::Failed(format::FormatFailure {
                formatter: FormatterKind::Custom,
                status: Some(7),
                ..
            })
        ),
        "expected custom formatter failure, got {outcome:?}"
    );
    assert_eq!(
        fs::read_to_string(&path).expect("read restored file"),
        raw,
        "formatter failure must restore the raw post-write bytes"
    );
}
