use mockup_harness_v3::negative_fixtures::{
    fixture_manifest, run_negative_fixtures, REQUIRED_FIXTURE_IDS,
};

#[test]
fn isolated_negative_fixture_manifest_rejects_every_required_bypass() {
    let manifest = fixture_manifest();
    let static_manifest = include_str!("negative-fixtures.manifest")
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
                fixture.command,
                fixture.validator,
                fixture.screen_id,
                fixture.offending_identity,
                fixture.expected_diagnostic,
            ]
        );
    }

    let report = run_negative_fixtures()
        .expect("every negative fixture must reject its intended invalid input safely");
    assert_eq!(report.outcomes.len(), REQUIRED_FIXTURE_IDS.len());
    assert_eq!(report.protected_before, report.protected_after);
    assert!(!report.fixture_root_exists);

    for outcome in &report.outcomes {
        assert_eq!(
            outcome.exit_code, 3,
            "{} must use the public validation exit code",
            outcome.id
        );
        assert!(
            outcome
                .diagnostic
                .contains(&format!("fixture {}", outcome.id)),
            "{} must name the fixture",
            outcome.id
        );
        assert!(
            outcome
                .diagnostic
                .contains(&format!("screen {}", outcome.screen_id)),
            "{} must name the screen or case",
            outcome.id
        );
        assert!(
            outcome.diagnostic.contains(outcome.offending_identity),
            "{} must name the offending identity",
            outcome.id
        );
        assert!(
            outcome.diagnostic.contains(outcome.expected_diagnostic),
            "{} must preserve its intended validator diagnostic",
            outcome.id
        );
    }
}
