//! Black-box assertions over the embedded models.dev snapshot: the offline catalogue must list a
//! meaningful number of real, recognizable models with real per-token pricing and varying context
//! windows, and must include the default model used by `cortex run`. These back the
//! `cortex models` offline-listing contract (VAL-FINISH-001..004) at the data layer.

use providers::{Catalogue, DEFAULT_MODEL, DEFAULT_PROVIDER};

fn embedded_models() -> Vec<providers::ModelInfo> {
    Catalogue::embedded()
        .providers()
        .flat_map(|provider| provider.models.values().cloned())
        .collect()
}

#[test]
fn snapshot_lists_at_least_ten_recognizable_models() {
    let models = embedded_models();
    assert!(
        models.len() >= 10,
        "embedded snapshot must list >=10 models, found {}",
        models.len()
    );

    for vendor in ["google/", "anthropic/", "openai/"] {
        assert!(
            models.iter().any(|model| model.id.starts_with(vendor)),
            "snapshot must include at least one {vendor} model"
        );
    }
}

#[test]
fn snapshot_includes_the_default_model_under_the_default_provider() {
    let catalogue = Catalogue::embedded();
    let model = catalogue
        .model(DEFAULT_PROVIDER, DEFAULT_MODEL)
        .expect("default model present under the default provider");
    assert_eq!(model.id, DEFAULT_MODEL);
    assert!(
        model.limit.context >= 8000,
        "default model has a real context window"
    );
}

#[test]
fn every_model_surfaces_nonnegative_pricing_and_some_model_is_paid() {
    let models = embedded_models();
    for model in &models {
        assert!(
            model.cost.input >= 0.0 && model.cost.output >= 0.0,
            "{} pricing must be non-negative",
            model.id
        );
    }
    assert!(
        models
            .iter()
            .any(|model| model.cost.input > 0.0 && model.cost.output > 0.0),
        "at least one model must show real (>0) per-token pricing"
    );
}

#[test]
fn context_windows_are_positive_and_vary_across_models() {
    let models = embedded_models();
    for model in &models {
        assert!(
            model.limit.context > 0,
            "{} must expose a positive context window",
            model.id
        );
    }
    let distinct: std::collections::BTreeSet<u64> =
        models.iter().map(|model| model.limit.context).collect();
    assert!(
        distinct.len() >= 2,
        "context windows must vary across models, not be a single hard-coded constant"
    );
}
