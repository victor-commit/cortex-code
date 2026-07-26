//! Black-box proof that base-URL and credential-env resolution fall back to
//! the catalogue (`api` / `env`) when config and built-in profiles do not
//! supply them.
//!
//! Deterministic and offline — no network. Credential env is proven via a
//! child-process probe (workspace forbids in-process `set_var`).

use providers::{Catalogue, OPENROUTER_API_BASE, OPENROUTER_ID, ProviderConfigSet};

/// SambaNova is catalogue-only (no built-in profile, not in the W2 profile list),
/// so an empty config must resolve the base URL from `ProviderInfo.api`.
#[test]
fn base_url_falls_back_to_catalogue_api() {
    const SAMBANOVA_API: &str = "https://api.sambanova.ai/v1";

    let catalogue = Catalogue::from_json(
        r#"{
  "sambanova": {
    "id": "sambanova",
    "name": "SambaNova",
    "api": "https://api.sambanova.ai/v1",
    "env": ["SAMBANOVA_API_KEY"],
    "models": {}
  }
}"#,
    )
    .expect("minimal sambanova catalogue parses");

    assert!(
        catalogue.provider("sambanova").is_some(),
        "fixture must expose sambanova"
    );
    assert_eq!(
        catalogue.provider("sambanova").and_then(|p| p.api.as_deref()),
        Some(SAMBANOVA_API)
    );

    let config = ProviderConfigSet::default();
    let resolved = config
        .resolve_provider_base_url_with("sambanova", &catalogue)
        .expect("catalogue api is a valid base URL fallback");

    assert_eq!(resolved, SAMBANOVA_API);
}

/// SambaNova is catalogue-only (no built-in profile, not OpenRouter), so an
/// empty config must pick the credential env name from `ProviderInfo.env`.
///
/// `CredentialInputs` stores the resolved env *value*, not the name — so the
/// probe child sets `SAMBANOVA_API_KEY` at process start and asserts
/// `env_value` is populated only when the catalogue-aware path is used.
#[test]
fn credential_inputs_fall_back_to_catalogue_env() {
    const ENV_NAME: &str = "SAMBANOVA_API_KEY";
    const ENV_VALUE: &str = "sambanova-catalogue-fixture-key";
    const PROBE_ENV: &str = "CORTEX_CATALOGUE_CREDENTIAL_ENV_PROBE";
    const MARKER: &str = "CATALOGUE_CREDENTIAL_ENV=";

    if std::env::var_os(PROBE_ENV).is_some() {
        let catalogue = Catalogue::from_json(
            r#"{
  "sambanova": {
    "id": "sambanova",
    "name": "SambaNova",
    "api": "https://api.sambanova.ai/v1",
    "env": ["SAMBANOVA_API_KEY"],
    "models": {}
  }
}"#,
        )
        .expect("minimal sambanova catalogue parses");

        assert_eq!(
            catalogue.provider("sambanova").map(|p| p.env.clone()),
            Some(vec![ENV_NAME.to_owned()])
        );

        let config = ProviderConfigSet::default();

        // Without catalogue fallback the env name is unknown, so even a set
        // SAMBANOVA_API_KEY must not populate env_value.
        let without = config.credential_inputs("sambanova");
        assert!(
            without.env_value.is_none(),
            "empty config without catalogue must not invent catalogue env; got {:?}",
            without.env_value
        );

        let inputs = config.credential_inputs_with("sambanova", &catalogue);
        let ok = inputs.env_value.as_deref() == Some(ENV_VALUE);
        println!(
            "{MARKER}{}",
            if ok {
                "ok"
            } else {
                "fail"
            }
        );
        return;
    }

    let exe = std::env::current_exe().expect("path to this test binary");
    let output = std::process::Command::new(exe)
        .args([
            "--exact",
            "credential_inputs_fall_back_to_catalogue_env",
            "--nocapture",
        ])
        .env(PROBE_ENV, "1")
        .env(ENV_NAME, ENV_VALUE)
        .output()
        .expect("spawn child probe");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(&format!("{MARKER}ok")),
        "child with {ENV_NAME} set must populate CredentialInputs.env_value via catalogue env. stdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Regression: catalogue `api` fallback must not reorder OpenRouter's default
/// last, and must not invent a base URL for a genuinely unknown provider id.
#[test]
fn catalogue_only_provider_keeps_openrouter_default_last() {
    const SAMBANOVA_API: &str = "https://api.sambanova.ai/v1";
    const UNKNOWN_ID: &str = "totally-unknown-provider-xyz";

    let catalogue = Catalogue::from_json(
        r#"{
  "sambanova": {
    "id": "sambanova",
    "name": "SambaNova",
    "api": "https://api.sambanova.ai/v1",
    "env": ["SAMBANOVA_API_KEY"],
    "models": {}
  }
}"#,
    )
    .expect("minimal sambanova catalogue parses");

    let config = ProviderConfigSet::default();

    // Catalogue-only still resolves from ProviderInfo.api.
    assert_eq!(
        config
            .resolve_provider_base_url_with("sambanova", &catalogue)
            .expect("catalogue api is a valid base URL fallback"),
        SAMBANOVA_API
    );

    // OpenRouter with empty config still resolves to the canonical default
    // (no config override; catalogue fixture omits openrouter).
    assert_eq!(
        config
            .resolve_provider_base_url_with(OPENROUTER_ID, &catalogue)
            .expect("openrouter keeps its default base URL"),
        OPENROUTER_API_BASE
    );

    // Genuinely unknown id must not fall through to the OpenRouter default.
    let err = config
        .resolve_provider_base_url_with(UNKNOWN_ID, &catalogue)
        .expect_err("unknown provider must not invent a base URL");
    assert_eq!(err.key, format!("$.provider.{UNKNOWN_ID}.base_url"));
    assert!(
        err.message.contains("no `base_url`"),
        "expected missing_base_url message, got: {}",
        err.message
    );
}

/// W2 first-class remotes pin `default_base_url: None` (D11) so catalogue
/// `api` supplies the URL. A profile row must not regress into a hard-coded
/// base that shadows models.dev, and must not fall through to OpenRouter.
#[test]
fn profiles_do_not_regress_catalogue_fallback() {
    // anthropic is a W2 first-class profile with default_base_url: None.
    const PROVIDER_ID: &str = "anthropic";
    const CATALOGUE_API: &str = "https://api.anthropic.com/v1";

    let catalogue = Catalogue::from_json(
        r#"{
  "anthropic": {
    "id": "anthropic",
    "name": "Anthropic",
    "api": "https://api.anthropic.com/v1",
    "env": ["ANTHROPIC_API_KEY"],
    "models": {}
  }
}"#,
    )
    .expect("minimal anthropic catalogue parses");

    assert_eq!(
        catalogue.provider(PROVIDER_ID).and_then(|p| p.api.as_deref()),
        Some(CATALOGUE_API),
        "fixture must expose non-empty anthropic api"
    );

    let config = ProviderConfigSet::default();
    let resolved = config
        .resolve_provider_base_url_with(PROVIDER_ID, &catalogue)
        .expect("W2 profile with default_base_url None must resolve from catalogue api");

    assert_eq!(
        resolved, CATALOGUE_API,
        "catalogue api must win over missing profile default and OpenRouter last"
    );
    assert_ne!(
        resolved, OPENROUTER_API_BASE,
        "W2 remote must not fall through to OpenRouter default"
    );
}
