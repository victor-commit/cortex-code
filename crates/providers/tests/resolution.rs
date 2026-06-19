//! Black-box integration tests over the public `providers` API: credential precedence and
//! model-id resolution as a downstream consumer (the runner) exercises them. Deterministic and
//! offline — no network, no ambient environment.

use std::path::PathBuf;

use providers::{
    Catalogue, CredentialInputs, CredentialSource, DEFAULT_MODEL, OPENROUTER_ID,
    resolve_credential, resolve_model, store_credential,
};

#[test]
fn full_precedence_chain_is_observable_env_then_file_then_config() {
    let dir = std::env::temp_dir().join(format!(
        "cortex-providers-precedence-{}-{:?}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let auth = dir.join("auth.json");
    store_credential(&auth, OPENROUTER_ID, "file-layer-key").expect("seed file layer");

    // All three layers present -> env wins.
    let all = CredentialInputs {
        env_value: Some("env-layer-key".to_owned()),
        credential_file: Some(auth.clone()),
        config_value: Some("config-layer-key".to_owned()),
    };
    let resolved = resolve_credential(OPENROUTER_ID, &all).expect("resolves");
    assert_eq!(resolved.source(), CredentialSource::Env);
    assert_eq!(resolved.expose(), "env-layer-key");

    // Drop env -> file wins.
    let no_env = CredentialInputs {
        env_value: None,
        ..all.clone()
    };
    let resolved = resolve_credential(OPENROUTER_ID, &no_env).expect("resolves");
    assert_eq!(resolved.source(), CredentialSource::File);
    assert_eq!(resolved.expose(), "file-layer-key");

    // Drop env + file -> config wins.
    let config_only = CredentialInputs {
        env_value: None,
        credential_file: None,
        config_value: Some("config-layer-key".to_owned()),
    };
    let resolved = resolve_credential(OPENROUTER_ID, &config_only).expect("resolves");
    assert_eq!(resolved.source(), CredentialSource::Config);
    assert_eq!(resolved.expose(), "config-layer-key");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn no_credential_anywhere_is_the_canonical_no_credential_error() {
    let inputs = CredentialInputs {
        env_value: None,
        credential_file: Some(PathBuf::from("/nonexistent/path/auth.json")),
        config_value: None,
    };
    let err = resolve_credential(OPENROUTER_ID, &inputs).expect_err("must fail");
    assert_eq!(err.kind(), "no-credential");
}

#[test]
fn model_resolution_covers_default_known_and_passthrough() {
    // Default.
    assert_eq!(resolve_model(None).model_id, DEFAULT_MODEL);
    assert_eq!(resolve_model(None).provider_id, OPENROUTER_ID);

    // Known.
    let known = resolve_model(Some(DEFAULT_MODEL));
    assert_eq!(known.provider_id, OPENROUTER_ID);
    assert!(known.info.is_some());

    // Passthrough (unknown id still resolves to OpenRouter, no metadata).
    let passthrough = resolve_model(Some("vendor/brand-new-model"));
    assert_eq!(passthrough.provider_id, OPENROUTER_ID);
    assert_eq!(passthrough.model_id, "vendor/brand-new-model");
    assert!(passthrough.info.is_none());
}

#[test]
fn catalogue_exposes_openrouter_and_its_default_model() {
    let catalogue = Catalogue::embedded();
    let openrouter = catalogue
        .provider(OPENROUTER_ID)
        .expect("openrouter present");
    assert_eq!(
        openrouter.api.as_deref(),
        Some("https://openrouter.ai/api/v1")
    );
    assert!(catalogue.model(OPENROUTER_ID, DEFAULT_MODEL).is_some());
}
