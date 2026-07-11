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

fn gofmt_expected(raw: &str) -> String {
    let dir = temp_dir("gofmt-expected");
    let path = dir.join("expected.go");
    fs::write(&path, raw).expect("write gofmt input");
    run_manual(Command::new("gofmt").arg("-w").arg(&path));
    fs::read_to_string(path).expect("read gofmt output")
}

fn biome_expected(raw: &str, ext: &str) -> String {
    let dir = temp_dir("biome-expected");
    fs::write(dir.join("biome.json"), "{}").expect("write biome marker");
    let path = dir.join(format!("expected.{ext}"));
    fs::write(&path, raw).expect("write biome input");
    run_manual(
        Command::new("biome")
            .current_dir(&dir)
            .arg("format")
            .arg("--write")
            .arg(&path),
    );
    fs::read_to_string(path).expect("read biome output")
}

fn assert_unchanged(path: &Path, expected: &str) {
    assert_eq!(
        fs::read_to_string(path).expect("read unformatted file"),
        expected
    );
}

fn synthetic_openrouter_key() -> String {
    let body = "deadbeef".repeat(8);
    format!("sk-{}-{}", "or-v1", body)
}

#[cfg(unix)]
fn write_executable(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;

    fs::write(path, body).expect("write executable script");
    let mut permissions = fs::metadata(path).expect("script metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("chmod executable script");
}

#[cfg(unix)]
fn formatter_override_config(name: &str, script: &Path) -> FormatterConfig {
    let table = toml::from_str::<toml::Value>(&format!(
        "[formatter.{name}]\ncommand = [\"{}\", \"$FILE\"]\n",
        script.display()
    ))
    .expect("valid formatter config");
    FormatterConfig::from_table(table.as_table().expect("table root"))
        .expect("parse formatter config")
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

#[cfg(unix)]
#[test]
fn rustfmt_works_with_an_isolated_home_when_its_rustup_proxy_is_on_path() {
    if !binary_available("rustfmt") {
        eprintln!("skipping isolated-home rustfmt test: rustfmt is not on PATH");
        return;
    }

    let isolated_home = temp_dir("rustfmt-isolated-home");
    let mut command = Command::new(std::env::current_exe().expect("current test binary"));
    command
        .arg("--exact")
        .arg("rustfmt_works_with_an_isolated_home_child")
        .arg("--nocapture")
        .env("CORTEX_FORMAT_RUSTFMT_ISOLATED_HOME_CHILD", "1")
        .env("HOME", isolated_home)
        .env_remove("RUSTUP_HOME")
        .env_remove("RUSTUP_TOOLCHAIN");
    let output = command.output().expect("spawn isolated-home rustfmt child");
    assert!(
        output.status.success(),
        "isolated-home rustfmt child failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(unix)]
#[test]
fn rustfmt_works_with_an_isolated_home_child() {
    if std::env::var("CORTEX_FORMAT_RUSTFMT_ISOLATED_HOME_CHILD").as_deref() != Ok("1") {
        return;
    }
    assert!(
        std::env::var_os("RUSTUP_HOME").is_none() && std::env::var_os("RUSTUP_TOOLCHAIN").is_none(),
        "the child must simulate a formatter subprocess without inherited rustup settings"
    );

    let dir = temp_dir("rustfmt-isolated-home-child");
    let path = dir.join("main.rs");
    fs::write(&path, "fn  main( ){let x=1;}\n").expect("write Rust input");

    let outcome = FormatterRunner::new(&dir).format_path(&path);

    assert_eq!(outcome, FormatOutcome::Formatted(FormatterKind::Rustfmt));
    assert_eq!(
        fs::read_to_string(path).expect("read formatted Rust"),
        "fn main() {\n    let x = 1;\n}\n"
    );
}

#[cfg(unix)]
#[test]
fn rustup_proxy_resolution_obeys_rustup_toolchain_precedence() {
    for scenario in ["env", "toml", "legacy", "default"] {
        let root = temp_dir(&format!("rustup-precedence-{scenario}"));
        write_rustup_proxy_fixture(&root);
        match scenario {
            "toml" => {
                fs::write(
                    root.join("rust-toolchain.toml"),
                    "[toolchain]\nchannel = \"toml\"\n",
                )
                .expect("write TOML project toolchain");
                fs::write(root.join("rust-toolchain"), "legacy\n")
                    .expect("write lower-priority legacy project toolchain");
            }
            "legacy" => {
                fs::write(root.join("rust-toolchain"), "legacy\n")
                    .expect("write legacy project toolchain");
            }
            "env" | "default" => {}
            _ => unreachable!("all scenarios are enumerated above"),
        }

        let mut command = Command::new(std::env::current_exe().expect("current test binary"));
        command
            .arg("--exact")
            .arg("rustup_proxy_resolution_obeys_rustup_toolchain_precedence_child")
            .arg("--nocapture")
            .env("CORTEX_FORMAT_RUSTUP_PRECEDENCE_CHILD", scenario)
            .env("CORTEX_FORMAT_RUSTUP_PRECEDENCE_ROOT", &root)
            .env("HOME", root.join("isolated-home"))
            .env_remove("RUSTUP_HOME")
            .env_remove("RUSTUP_TOOLCHAIN")
            .env("OPENROUTER_API_KEY", synthetic_openrouter_key())
            .env("FORMATTER_VISIBLE_VALUE", synthetic_openrouter_key());
        if scenario == "env" {
            command.env("RUSTUP_TOOLCHAIN", "env");
        }
        prepend_fixture_rustup_proxy_to_path(&mut command, &root);

        let output = command
            .output()
            .expect("spawn rustup precedence child test");
        assert!(
            output.status.success(),
            "rustup precedence child ({scenario}) failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[cfg(unix)]
#[test]
fn rustup_proxy_resolution_obeys_rustup_toolchain_precedence_child() {
    let Ok(scenario) = std::env::var("CORTEX_FORMAT_RUSTUP_PRECEDENCE_CHILD") else {
        return;
    };
    let root = PathBuf::from(
        std::env::var("CORTEX_FORMAT_RUSTUP_PRECEDENCE_ROOT")
            .expect("rustup precedence fixture root is set"),
    );
    let path = root.join("main.rs");
    fs::write(&path, "fn  main( ){}\n").expect("write Rust input");

    let outcome = FormatterRunner::new(&root).format_path(&path);

    assert_eq!(outcome, FormatOutcome::Formatted(FormatterKind::Rustfmt));
    assert_eq!(
        fs::read_to_string(path).expect("read formatted Rust"),
        format!("fn {scenario}() {{}}\n"),
        "the selected concrete formatter must match rustup's {scenario} precedence"
    );
}

#[cfg(unix)]
#[test]
fn rustup_proxy_resolution_honors_safe_project_toolchain_path() {
    for scenario in ["path", "env"] {
        let root = temp_dir(&format!("rustup-path-{scenario}"));
        write_rustup_proxy_fixture(&root);
        install_project_path_toolchain(&root);
        fs::write(
            root.join("rust-toolchain.toml"),
            "[toolchain]\npath = \"local-toolchain\"\n",
        )
        .expect("write TOML project path toolchain");
        fs::write(root.join("rust-toolchain"), "legacy\n")
            .expect("write lower-priority legacy project toolchain");

        let mut command = Command::new(std::env::current_exe().expect("current test binary"));
        command
            .arg("--exact")
            .arg("rustup_proxy_resolution_honors_safe_project_toolchain_path_child")
            .arg("--nocapture")
            .env("CORTEX_FORMAT_RUSTUP_PATH_CHILD", scenario)
            .env("CORTEX_FORMAT_RUSTUP_PATH_ROOT", &root)
            .env("HOME", root.join("isolated-home"))
            .env_remove("RUSTUP_HOME")
            .env_remove("RUSTUP_TOOLCHAIN")
            .env("OPENROUTER_API_KEY", synthetic_openrouter_key())
            .env("FORMATTER_VISIBLE_VALUE", synthetic_openrouter_key());
        if scenario == "env" {
            command.env("RUSTUP_TOOLCHAIN", "env");
        }
        prepend_fixture_rustup_proxy_to_path(&mut command, &root);

        let output = command
            .output()
            .expect("spawn rustup path-resolution child");
        assert!(
            output.status.success(),
            "rustup path-resolution child ({scenario}) failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[cfg(unix)]
#[test]
fn rustup_proxy_resolution_honors_safe_project_toolchain_path_child() {
    let Ok(scenario) = std::env::var("CORTEX_FORMAT_RUSTUP_PATH_CHILD") else {
        return;
    };
    let root = PathBuf::from(
        std::env::var("CORTEX_FORMAT_RUSTUP_PATH_ROOT")
            .expect("rustup path-resolution fixture root is set"),
    );
    let path = root.join("main.rs");
    fs::write(&path, "fn  main( ){}\n").expect("write Rust input");

    let outcome = FormatterRunner::new(&root).format_path(&path);

    assert_eq!(outcome, FormatOutcome::Formatted(FormatterKind::Rustfmt));

    let manual_rustfmt = match scenario.as_str() {
        "path" => root.join("local-toolchain").join("bin").join("rustfmt"),
        "env" => root
            .join(".rustup")
            .join("toolchains")
            .join("env")
            .join("bin")
            .join("rustfmt"),
        _ => unreachable!("safe-path scenarios are enumerated above"),
    };
    let manual_path = root.join("manual.rs");
    fs::write(&manual_path, "fn  manual( ){}\n").expect("write manual Rust input");
    run_manual(
        Command::new(manual_rustfmt)
            .current_dir(&root)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .arg(&manual_path),
    );

    assert_eq!(
        fs::read_to_string(path).expect("read project-formatted Rust"),
        fs::read_to_string(manual_path).expect("read manually formatted Rust"),
        "formatter output must match the selected toolchain under a sanitized environment"
    );
}

#[cfg(unix)]
#[test]
fn rustup_proxy_resolution_rejects_unsafe_project_toolchain_paths() {
    use std::os::unix::fs::symlink;

    for scenario in [
        "absolute",
        "parent",
        "symlink-escape",
        "formatter-symlink-escape",
        "missing",
        "non-executable",
        "empty",
    ] {
        let root = temp_dir(&format!("rustup-unsafe-path-{scenario}"));
        let outside = temp_dir(&format!("rustup-unsafe-path-outside-{scenario}"));
        write_rustup_proxy_fixture(&root);
        let outside_rustfmt = outside.join("bin").join("rustfmt");
        fs::create_dir_all(outside_rustfmt.parent().expect("outside formatter parent"))
            .expect("create outside formatter parent");
        write_executable(
            &outside_rustfmt,
            "#!/usr/bin/env sh\nfor arg do file=\"$arg\"; done\nprintf 'fn outside() {}\\n' > \"$file\"\n",
        );
        let non_executable = root
            .join("non-executable-toolchain")
            .join("bin")
            .join("rustfmt");
        fs::create_dir_all(
            non_executable
                .parent()
                .expect("non-executable formatter parent"),
        )
        .expect("create non-executable formatter parent");
        fs::write(&non_executable, "#!/usr/bin/env sh\nexit 0\n")
            .expect("write non-executable formatter");
        symlink(&outside, root.join("escape-link")).expect("create escaping toolchain symlink");
        let formatter_symlink = root
            .join("formatter-symlink-toolchain")
            .join("bin")
            .join("rustfmt");
        fs::create_dir_all(
            formatter_symlink
                .parent()
                .expect("formatter symlink parent"),
        )
        .expect("create formatter symlink parent");
        symlink(&outside_rustfmt, &formatter_symlink).expect("create escaping formatter symlink");

        let configured_path = match scenario {
            "absolute" => outside.display().to_string(),
            "parent" => format!(
                "../{}",
                outside.file_name().expect("outside name").display()
            ),
            "symlink-escape" => "escape-link".to_owned(),
            "formatter-symlink-escape" => "formatter-symlink-toolchain".to_owned(),
            "missing" => "missing-toolchain".to_owned(),
            "non-executable" => "non-executable-toolchain".to_owned(),
            "empty" => String::new(),
            _ => unreachable!("unsafe-path scenarios are enumerated above"),
        };
        fs::write(
            root.join("rust-toolchain.toml"),
            format!("[toolchain]\npath = {configured_path:?}\n"),
        )
        .expect("write unsafe TOML project path toolchain");
        fs::write(root.join("rust-toolchain"), "legacy\n")
            .expect("write lower-priority legacy project toolchain");

        let mut command = Command::new(std::env::current_exe().expect("current test binary"));
        command
            .arg("--exact")
            .arg("rustup_proxy_resolution_rejects_unsafe_project_toolchain_paths_child")
            .arg("--nocapture")
            .env("CORTEX_FORMAT_RUSTUP_UNSAFE_PATH_CHILD", scenario)
            .env("CORTEX_FORMAT_RUSTUP_UNSAFE_PATH_ROOT", &root)
            .env("HOME", root.join("isolated-home"))
            .env_remove("RUSTUP_HOME")
            .env_remove("RUSTUP_TOOLCHAIN")
            .env("OPENROUTER_API_KEY", synthetic_openrouter_key())
            .env("FORMATTER_VISIBLE_VALUE", synthetic_openrouter_key());
        prepend_fixture_rustup_proxy_to_path(&mut command, &root);

        let output = command.output().expect("spawn rustup unsafe-path child");
        assert!(
            output.status.success(),
            "rustup unsafe-path child ({scenario}) failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[cfg(unix)]
#[test]
fn rustup_proxy_resolution_rejects_unsafe_project_toolchain_paths_child() {
    if std::env::var("CORTEX_FORMAT_RUSTUP_UNSAFE_PATH_CHILD").is_err() {
        return;
    }
    let root = PathBuf::from(
        std::env::var("CORTEX_FORMAT_RUSTUP_UNSAFE_PATH_ROOT")
            .expect("rustup unsafe-path fixture root is set"),
    );
    let path = root.join("main.rs");
    let raw = "fn  main( ){}\n";
    fs::write(&path, raw).expect("write Rust input");

    let outcome = FormatterRunner::new(&root).format_path(&path);

    assert!(
        matches!(
            outcome,
            FormatOutcome::Skipped(format::SkipReason::MissingBinary { binary: "rustfmt" })
        ),
        "unsafe project toolchain path must fail closed instead of running another rustfmt: {outcome:?}"
    );
    assert_eq!(
        fs::read_to_string(path).expect("read unformatted Rust"),
        raw,
        "unsafe project toolchain path must leave the file unchanged"
    );
}

#[cfg(unix)]
fn install_project_path_toolchain(root: &Path) {
    let source = root
        .join(".rustup")
        .join("toolchains")
        .join("path")
        .join("bin")
        .join("rustfmt");
    let target = root.join("local-toolchain").join("bin").join("rustfmt");
    fs::create_dir_all(target.parent().expect("project toolchain formatter parent"))
        .expect("create project toolchain formatter parent");
    fs::hard_link(source, target).expect("link project toolchain formatter");
}

#[cfg(unix)]
fn write_rustup_proxy_fixture(root: &Path) {
    use std::os::unix::fs::symlink;

    let bin = root.join(".cargo").join("bin");
    fs::create_dir_all(&bin).expect("create rustup proxy bin");
    let rustup = bin.join("rustup");
    write_executable(
        &rustup,
        "#!/usr/bin/env sh\nprintf 'rustup proxy should not execute\\n' >&2\nexit 99\n",
    );
    symlink(&rustup, bin.join("rustfmt")).expect("link rustfmt proxy to rustup");

    for toolchain in ["env", "toml", "legacy", "default", "path"] {
        let formatter = root
            .join(".rustup")
            .join("toolchains")
            .join(toolchain)
            .join("bin")
            .join("rustfmt");
        fs::create_dir_all(formatter.parent().expect("formatter parent"))
            .expect("create concrete formatter parent");
        write_executable(
            &formatter,
            &format!(
                "#!/usr/bin/env sh\n\
if [ \"${{OPENROUTER_API_KEY+x}}\" = x ]; then printf 'OPENROUTER_API_KEY leaked\\n' >&2; exit 42; fi\n\
if [ \"${{FORMATTER_VISIBLE_VALUE+x}}\" = x ]; then printf 'key-shaped value leaked\\n' >&2; exit 43; fi\n\
for arg do file=\"$arg\"; done\n\
printf 'fn {toolchain}() {{}}\\n' > \"$file\"\n"
            ),
        );
    }
    fs::write(
        root.join(".rustup").join("settings.toml"),
        "default_toolchain = \"default\"\n",
    )
    .expect("write rustup settings");
}

#[cfg(unix)]
fn prepend_fixture_rustup_proxy_to_path(command: &mut Command, root: &Path) {
    let inherited_path = std::env::var_os("PATH").unwrap_or_default();
    let mut paths = vec![root.join(".cargo").join("bin")];
    paths.extend(std::env::split_paths(&inherited_path));
    command.env(
        "PATH",
        std::env::join_paths(paths).expect("build fixture PATH"),
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

#[test]
fn gofmt_formats_go_files_when_available() {
    if !binary_available("gofmt") {
        eprintln!("skipping real formatter test: gofmt is not on PATH");
        return;
    }

    let dir = temp_dir("gofmt");
    let path = dir.join("main.go");
    let raw = "package main\nfunc main(){println(\"hi\")}\n";
    fs::write(&path, raw).expect("write go file");

    let outcome = FormatterRunner::new(&dir).format_path(&path);

    assert_eq!(outcome, FormatOutcome::Formatted(FormatterKind::Gofmt));
    assert_ne!(fs::read_to_string(&path).expect("read go file"), raw);
    assert_eq!(
        fs::read_to_string(&path).expect("read go file"),
        gofmt_expected(raw)
    );
}

#[test]
fn biome_formats_supported_files_only_when_marker_and_binary_are_present() {
    let no_marker_dir = temp_dir("biome-no-marker");
    let no_marker_path = no_marker_dir.join("demo.ts");
    let raw = "const value={foo:\"bar\",answer:42};\n";
    fs::write(&no_marker_path, raw).expect("write ts file");

    let no_marker_outcome = FormatterRunner::new(&no_marker_dir).format_path(&no_marker_path);
    assert!(
        !matches!(
            no_marker_outcome,
            FormatOutcome::Formatted(FormatterKind::Biome)
        ),
        "biome must not run without a marker: {no_marker_outcome:?}"
    );
    assert_unchanged(&no_marker_path, raw);

    if !binary_available("biome") {
        eprintln!("skipping configured biome proof: biome is not on PATH");
        return;
    }

    let dir = temp_dir("biome-marker");
    fs::write(dir.join("biome.json"), "{}").expect("write biome marker");
    let path = dir.join("demo.ts");
    fs::write(&path, raw).expect("write ts file");

    let outcome = FormatterRunner::new(&dir).format_path(&path);

    assert_eq!(outcome, FormatOutcome::Formatted(FormatterKind::Biome));
    assert_ne!(fs::read_to_string(&path).expect("read ts file"), raw);
    assert_eq!(
        fs::read_to_string(&path).expect("read ts file"),
        biome_expected(raw, "ts")
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
fn custom_formatter_array_runs_all_matching_entries_in_order() {
    let dir = temp_dir("custom-chain-order");
    let first = dir.join("first.sh");
    let second = dir.join("second.sh");
    write_executable(
        &first,
        "#!/usr/bin/env sh\nfile=\"$1\"\nprintf 'first\\n' >> \"$file\"\n",
    );
    write_executable(
        &second,
        "#!/usr/bin/env sh\nfile=\"$1\"\nprintf 'second\\n' >> \"$file\"\n",
    );

    let path = dir.join("demo.foo");
    fs::write(&path, "start\n").expect("write initial content");
    let config = FormatterConfig::from_table(
        toml::from_str::<toml::Value>(&format!(
            "[[formatter.custom]]\ncommand = [\"{}\", \"$FILE\"]\nextensions = [\".foo\"]\n\
[[formatter.custom]]\ncommand = [\"{}\", \"$FILE\"]\nextensions = [\".foo\"]\n",
            first.display(),
            second.display()
        ))
        .unwrap()
        .as_table()
        .unwrap(),
    )
    .expect("parse formatter config");

    let outcome = FormatterRunner::with_config(&dir, config).format_path(&path);

    assert_eq!(outcome, FormatOutcome::Formatted(FormatterKind::Custom));
    assert_eq!(
        fs::read_to_string(&path).expect("read chained output"),
        "start\nfirst\nsecond\n",
        "both custom formatters should run in configured order"
    );
}

#[cfg(unix)]
#[test]
fn formatter_chain_failure_restores_pre_format_bytes() {
    let dir = temp_dir("custom-chain-rollback");
    let first = dir.join("first.sh");
    let second = dir.join("second.sh");
    write_executable(
        &first,
        "#!/usr/bin/env sh\nfile=\"$1\"\nprintf 'intermediate\\n' > \"$file\"\n",
    );
    write_executable(
        &second,
        "#!/usr/bin/env sh\nfile=\"$1\"\nprintf 'corrupted\\n' > \"$file\"\nprintf 'second formatter failed\\n' >&2\nexit 9\n",
    );

    let path = dir.join("demo.foo");
    let raw = "raw bytes that must survive\n";
    fs::write(&path, raw).expect("write initial content");
    let config = FormatterConfig::from_table(
        toml::from_str::<toml::Value>(&format!(
            "[[formatter.custom]]\ncommand = [\"{}\", \"$FILE\"]\nextensions = [\".foo\"]\n\
[[formatter.custom]]\ncommand = [\"{}\", \"$FILE\"]\nextensions = [\".foo\"]\n",
            first.display(),
            second.display()
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
                status: Some(9),
                ..
            })
        ),
        "expected second formatter to fail chain, got {outcome:?}"
    );
    assert_eq!(
        fs::read_to_string(&path).expect("read restored bytes"),
        raw,
        "chain failures must restore pre-format bytes"
    );
}

#[cfg(unix)]
#[test]
fn custom_formatter_entries_honor_disabled_and_env_per_entry() {
    let dir = temp_dir("custom-disabled-env");
    let marker = dir.join("env-marker.txt");
    let script = dir.join("env-aware.sh");
    write_executable(
        &script,
        "#!/usr/bin/env sh\nfile=\"$1\"\nprintf '%s' \"${CUSTOM_MARKER_VALUE:-missing}\" > \"${CUSTOM_MARKER_PATH}\"\nprintf 'env-run\\n' >> \"$file\"\n",
    );

    let path = dir.join("demo.foo");
    fs::write(&path, "start\n").expect("write initial content");
    let config = FormatterConfig::from_table(
        toml::from_str::<toml::Value>(&format!(
            "[[formatter.custom]]\ncommand = [\"{}\", \"$FILE\"]\nextensions = [\".foo\"]\ndisabled = true\n\
[[formatter.custom]]\ncommand = [\"{}\", \"$FILE\"]\nextensions = [\".foo\"]\nenv = {{ CUSTOM_MARKER_PATH = \"{}\", CUSTOM_MARKER_VALUE = \"entry-two\" }}\n",
            script.display(),
            script.display(),
            marker.display()
        ))
        .unwrap()
        .as_table()
        .unwrap(),
    )
    .expect("parse formatter config");

    let outcome = FormatterRunner::with_config(&dir, config).format_path(&path);

    assert_eq!(outcome, FormatOutcome::Formatted(FormatterKind::Custom));
    assert_eq!(
        fs::read_to_string(&path).expect("read formatted content"),
        "start\nenv-run\n",
        "only enabled custom entry should run"
    );
    assert_eq!(
        fs::read_to_string(marker).expect("read env marker"),
        "entry-two",
        "enabled custom entry should receive its configured env values"
    );
}

#[cfg(unix)]
#[test]
fn builtin_extensions_override_applies_formatter_to_custom_extension() {
    let dir = temp_dir("builtin-extension-override");
    let script = dir.join("fake-rustfmt.sh");
    write_executable(
        &script,
        "#!/usr/bin/env sh\nfile=\"$1\"\nprintf 'formatted by rustfmt override\\n' > \"$file\"\n",
    );

    let path = dir.join("demo.foo");
    fs::write(&path, "raw\n").expect("write initial content");
    let config = FormatterConfig::from_table(
        toml::from_str::<toml::Value>(&format!(
            "[formatter.rustfmt]\ncommand = [\"{}\", \"$FILE\"]\nextensions = [\".foo\"]\n",
            script.display()
        ))
        .unwrap()
        .as_table()
        .unwrap(),
    )
    .expect("parse formatter config");

    let outcome = FormatterRunner::with_config(&dir, config).format_path(&path);

    assert_eq!(outcome, FormatOutcome::Formatted(FormatterKind::Rustfmt));
    assert_eq!(
        fs::read_to_string(&path).expect("read formatted content"),
        "formatted by rustfmt override\n"
    );
}

#[cfg(unix)]
#[test]
fn extended_formatter_matrix_uses_declared_shipped_paths() {
    struct Case {
        config_key: &'static str,
        kind: FormatterKind,
        file_name: &'static str,
        marker: Option<(&'static str, &'static str)>,
    }

    let cases = vec![
        Case {
            config_key: "gofmt",
            kind: FormatterKind::Gofmt,
            file_name: "demo.go",
            marker: None,
        },
        Case {
            config_key: "biome",
            kind: FormatterKind::Biome,
            file_name: "demo.ts",
            marker: Some(("biome.json", "{}\n")),
        },
        Case {
            config_key: "mix",
            kind: FormatterKind::Mix,
            file_name: "demo.ex",
            marker: None,
        },
        Case {
            config_key: "clang_format",
            kind: FormatterKind::ClangFormat,
            file_name: "demo.c",
            marker: Some((".clang-format", "BasedOnStyle: LLVM\n")),
        },
        Case {
            config_key: "zig_fmt",
            kind: FormatterKind::ZigFmt,
            file_name: "demo.zig",
            marker: None,
        },
        Case {
            config_key: "ktlint",
            kind: FormatterKind::Ktlint,
            file_name: "demo.kt",
            marker: None,
        },
        Case {
            config_key: "dart",
            kind: FormatterKind::DartFormat,
            file_name: "demo.dart",
            marker: None,
        },
        Case {
            config_key: "terraform",
            kind: FormatterKind::TerraformFmt,
            file_name: "demo.tf",
            marker: None,
        },
        Case {
            config_key: "shfmt",
            kind: FormatterKind::Shfmt,
            file_name: "demo.sh",
            marker: None,
        },
        Case {
            config_key: "nixfmt",
            kind: FormatterKind::Nixfmt,
            file_name: "demo.nix",
            marker: None,
        },
        Case {
            config_key: "ocamlformat",
            kind: FormatterKind::Ocamlformat,
            file_name: "demo.ml",
            marker: Some((".ocamlformat", "version = 0.26.1\n")),
        },
        Case {
            config_key: "rubocop",
            kind: FormatterKind::Rubocop,
            file_name: "demo.rb",
            marker: None,
        },
        Case {
            config_key: "standardrb",
            kind: FormatterKind::Standardrb,
            file_name: "demo.rb",
            marker: None,
        },
        Case {
            config_key: "erb",
            kind: FormatterKind::Erb,
            file_name: "demo.erb",
            marker: None,
        },
        Case {
            config_key: "gleam",
            kind: FormatterKind::Gleam,
            file_name: "demo.gleam",
            marker: None,
        },
        Case {
            config_key: "cljfmt",
            kind: FormatterKind::Cljfmt,
            file_name: "demo.clj",
            marker: None,
        },
        Case {
            config_key: "ormolu",
            kind: FormatterKind::Ormolu,
            file_name: "demo.hs",
            marker: None,
        },
        Case {
            config_key: "latexindent",
            kind: FormatterKind::Latexindent,
            file_name: "demo.tex",
            marker: None,
        },
        Case {
            config_key: "air",
            kind: FormatterKind::Air,
            file_name: "demo.R",
            marker: None,
        },
    ];

    for case in cases {
        let dir = temp_dir(case.config_key);
        let script = dir.join(format!("fake-{}.sh", case.config_key));
        write_executable(
            &script,
            "#!/usr/bin/env sh\nfile=\"$1\"\nprintf 'formatted by registry\\n' > \"$file\"\n",
        );
        if let Some((marker, contents)) = case.marker {
            fs::write(dir.join(marker), contents).expect("write marker");
        }
        let path = dir.join(case.file_name);
        fs::write(&path, "raw\n").expect("write raw input");
        let config = formatter_override_config(case.config_key, &script);

        let outcome = FormatterRunner::with_config(&dir, config).format_path(&path);

        assert_eq!(
            outcome,
            FormatOutcome::Formatted(case.kind),
            "expected formatter {:?} for {}",
            case.kind,
            case.file_name
        );
        assert_eq!(
            fs::read_to_string(&path).expect("read formatted output"),
            "formatted by registry\n"
        );
    }
}

#[cfg(unix)]
#[test]
fn missing_formatter_binary_is_reported_cleanly_without_mutating_bytes() {
    let mut command = Command::new(std::env::current_exe().expect("current test binary"));
    command
        .arg("--exact")
        .arg("missing_formatter_binary_child")
        .arg("--nocapture")
        .env("CORTEX_FORMAT_MISSING_BINARY_CHILD", "1")
        .env("PATH", "");
    let output = command
        .output()
        .expect("spawn missing-binary child formatter test");
    assert!(
        output.status.success(),
        "missing-binary child failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(unix)]
#[test]
fn gofmt_detection_does_not_require_a_version_flag() {
    let mut command = Command::new(std::env::current_exe().expect("current test binary"));
    command
        .arg("--exact")
        .arg("gofmt_detection_without_version_flag_child")
        .arg("--nocapture")
        .env("CORTEX_FORMAT_GOFMT_NO_VERSION_CHILD", "1")
        .env("PATH", "");
    let output = command
        .output()
        .expect("spawn gofmt no-version child formatter test");
    assert!(
        output.status.success(),
        "gofmt no-version child failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(unix)]
#[test]
fn gofmt_detection_without_version_flag_child() {
    if std::env::var("CORTEX_FORMAT_GOFMT_NO_VERSION_CHILD").as_deref() != Ok("1") {
        return;
    }

    let dir = temp_dir("gofmt-no-version");
    let bin_dir = dir.join(".cortex").join("formatter-toolchain").join("bin");
    fs::create_dir_all(&bin_dir).expect("create local formatter bin directory");
    let gofmt = bin_dir.join("gofmt");
    write_executable(
        &gofmt,
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then printf 'unsupported\\n' >&2; exit 2; fi\nif [ \"$1\" = \"-w\" ]; then file=\"$2\"; printf 'package main\\n\\nfunc main() {}\\n' > \"$file\"; exit 0; fi\nprintf 'unexpected args\\n' >&2\nexit 9\n",
    );

    let path = dir.join("main.go");
    fs::write(&path, "package main\nfunc main(){println(\"hi\")}\n").expect("write go fixture");
    let outcome = FormatterRunner::new(&dir).format_path(&path);

    assert_eq!(outcome, FormatOutcome::Formatted(FormatterKind::Gofmt));
    assert_eq!(
        fs::read_to_string(&path).expect("read formatted go"),
        "package main\n\nfunc main() {}\n"
    );
}

#[cfg(unix)]
#[test]
fn missing_formatter_binary_child() {
    if std::env::var("CORTEX_FORMAT_MISSING_BINARY_CHILD").as_deref() != Ok("1") {
        return;
    }

    let dir = temp_dir("missing-binary-child");
    let path = dir.join("demo.sh");
    let raw = "#!/usr/bin/env sh\necho hi\n";
    fs::write(&path, raw).expect("write shell fixture");

    let outcome = FormatterRunner::new(&dir).format_path(&path);

    assert!(
        matches!(
            outcome,
            FormatOutcome::Skipped(format::SkipReason::MissingBinary { binary: "shfmt" })
        ),
        "expected clean missing-binary status, got {outcome:?}"
    );
    assert_eq!(
        fs::read_to_string(&path).expect("read untouched file"),
        raw,
        "missing formatter should leave bytes unchanged"
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

#[cfg(unix)]
#[test]
fn formatter_failure_note_redacts_secret_stderr_and_preserves_bytes() {
    let dir = temp_dir("custom-failure-redaction");
    let secret = synthetic_openrouter_key();
    let script = dir.join("corrupt_then_echo_secret.sh");
    write_executable(
        &script,
        &format!(
            "#!/usr/bin/env sh\nfile=\"$1\"\nprintf 'CORRUPTED\\n' > \"$file\"\nprintf 'formatter saw {}\\n' >&2\nexit 7\n",
            secret
        ),
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

    let FormatOutcome::Failed(failure) = outcome else {
        panic!("expected formatter failure, got {outcome:?}");
    };
    let note = format::format_failure_note(&failure);
    assert!(
        note.contains(redaction::PLACEHOLDER),
        "formatter failure note must use canonical redaction: {note}"
    );
    assert!(
        !note.contains(&secret),
        "raw formatter stderr secret must not reach model-visible output: {note}"
    );
    assert_eq!(
        fs::read_to_string(&path).expect("read restored file"),
        raw,
        "formatter failure must restore the raw post-write bytes"
    );
}

#[cfg(unix)]
#[test]
fn custom_formatter_subprocess_receives_secret_stripped_env() {
    run_secret_env_probe_child("custom_formatter_secret_env_probe_child", "custom", &[]);
}

#[cfg(unix)]
#[test]
fn custom_formatter_secret_env_probe_child() {
    if std::env::var("CORTEX_FORMAT_ENV_PROBE_CHILD").as_deref() != Ok("custom") {
        return;
    }

    let dir = temp_dir("custom-env-child");
    let script = dir.join("env_probe_formatter.sh");
    write_executable(
        &script,
        "#!/usr/bin/env sh\nif [ \"${OPENROUTER_API_KEY+x}\" = x ]; then printf 'OPENROUTER_API_KEY leaked\\n' >&2; exit 42; fi\nif [ \"${FORMATTER_VISIBLE_VALUE+x}\" = x ]; then printf 'key-shaped value leaked\\n' >&2; exit 43; fi\nfile=\"$1\"\nprintf 'custom formatted\\n' > \"$file\"\n",
    );
    let path = dir.join("demo.foo");
    fs::write(&path, "custom raw\n").expect("write custom input");
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
        "custom formatted\n"
    );
}

#[cfg(unix)]
#[test]
fn builtin_formatter_subprocess_receives_secret_stripped_env() {
    let dir = temp_dir("builtin-env-parent");
    let shim = dir.join("rustfmt");
    write_executable(
        &shim,
        "#!/usr/bin/env sh\nif [ \"${OPENROUTER_API_KEY+x}\" = x ]; then printf 'OPENROUTER_API_KEY leaked\\n' >&2; exit 42; fi\nif [ \"${FORMATTER_VISIBLE_VALUE+x}\" = x ]; then printf 'key-shaped value leaked\\n' >&2; exit 43; fi\nif [ \"$1\" = \"--version\" ]; then printf 'rustfmt 1.0\\n'; exit 0; fi\nfor arg do file=\"$arg\"; done\nprintf 'fn main() {}\\n' > \"$file\"\n",
    );
    let existing_path = std::env::var_os("PATH").unwrap_or_default();
    let path = format!("{}:{}", dir.display(), existing_path.to_string_lossy());
    run_secret_env_probe_child(
        "builtin_formatter_secret_env_probe_child",
        "builtin",
        &[
            ("PATH", path),
            ("CORTEX_FORMAT_SHIM_DIR", dir.display().to_string()),
        ],
    );
}

#[cfg(unix)]
#[test]
fn builtin_formatter_secret_env_probe_child() {
    if std::env::var("CORTEX_FORMAT_ENV_PROBE_CHILD").as_deref() != Ok("builtin") {
        return;
    }

    let dir = PathBuf::from(std::env::var("CORTEX_FORMAT_SHIM_DIR").expect("shim dir env is set"));
    let path = dir.join("main.rs");
    fs::write(&path, "fn  main( ){}\n").expect("write rust input");

    let outcome = FormatterRunner::new(&dir).format_path(&path);

    assert_eq!(outcome, FormatOutcome::Formatted(FormatterKind::Rustfmt));
    assert_eq!(
        fs::read_to_string(&path).expect("read shim formatted file"),
        "fn main() {}\n"
    );
}

#[cfg(unix)]
fn run_secret_env_probe_child(test_name: &str, marker: &str, extra_env: &[(&str, String)]) {
    let mut command = Command::new(std::env::current_exe().expect("current test binary"));
    command
        .arg("--exact")
        .arg(test_name)
        .arg("--nocapture")
        .env("CORTEX_FORMAT_ENV_PROBE_CHILD", marker)
        .env("OPENROUTER_API_KEY", synthetic_openrouter_key())
        .env("FORMATTER_VISIBLE_VALUE", synthetic_openrouter_key());
    for (key, value) in extra_env {
        command.env(key, value);
    }
    let output = command.output().expect("spawn env probe child test");
    assert!(
        output.status.success(),
        "env probe child failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
