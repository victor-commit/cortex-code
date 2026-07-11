use std::process::{Command, Output};

use mockup_harness_v3::negative_fixtures::{fixture_manifest, REQUIRED_FIXTURE_IDS};

fn harness() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mockup-harness-v3"))
}

fn run(arguments: &[&str]) -> Output {
    harness()
        .args(arguments)
        .output()
        .expect("the public negative-fixture CLI must run")
}

#[test]
fn public_negative_fixture_matrix_rejects_every_required_bypass() {
    let manifest = fixture_manifest();
    let static_manifest = include_str!("negative-fixtures.tsv")
        .lines()
        .filter(|line| !line.starts_with('#'))
        .map(|line| line.split('\t').collect::<Vec<_>>())
        .collect::<Vec<_>>();
    assert_eq!(static_manifest.len(), REQUIRED_FIXTURE_IDS.len());
    assert_eq!(
        manifest
            .iter()
            .map(|fixture| fixture.id)
            .collect::<Vec<_>>(),
        REQUIRED_FIXTURE_IDS
    );
    for (fixture, row) in manifest.iter().zip(static_manifest) {
        assert_eq!(
            row,
            vec![
                fixture.id,
                "mockup-harness-v3",
                fixture.command,
                "3",
                fixture.validator,
                fixture.screen_id,
                fixture.offending_identity,
                fixture.expected_diagnostic,
            ]
        );
    }

    for fixture in manifest {
        let output = run(&[fixture.command.split(' ').next().unwrap(), fixture.id]);
        assert_eq!(
            output.status.code(),
            Some(3),
            "{} must return the real public validation exit code",
            fixture.id
        );
        assert!(
            output.stdout.is_empty(),
            "{} must not mix rejection diagnostics into stdout",
            fixture.id
        );
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(
            diagnostic.contains(&format!("fixture {}", fixture.id)),
            "{} must name the fixture",
            fixture.id
        );
        assert!(
            diagnostic.contains(&format!("screen {}", fixture.screen_id)),
            "{} must name the screen or case",
            fixture.id
        );
        assert!(
            diagnostic.contains(fixture.offending_identity),
            "{} must name the offending identity",
            fixture.id
        );
        assert!(
            diagnostic.contains(fixture.expected_diagnostic),
            "{} must preserve its intended validator diagnostic",
            fixture.id
        );
    }
}

#[test]
fn public_negative_fixture_usage_failures_are_distinct_from_validation_failures() {
    for arguments in [
        vec!["negative-fixture"],
        vec!["negative-fixture", "unknown-fixture"],
        vec!["negative-fixture", "orphan-candidate", "extra"],
    ] {
        let output = run(&arguments);
        assert_eq!(
            output.status.code(),
            Some(2),
            "{arguments:?} must be a public usage error"
        );
        assert!(output.stdout.is_empty(), "{arguments:?}");
        assert!(
            !output.stderr.is_empty(),
            "{arguments:?} must emit its usage diagnostic on stderr"
        );
    }
}
