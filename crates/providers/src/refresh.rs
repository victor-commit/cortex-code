//! `cortex models --refresh`: a real models.dev catalog fetch behind the fail-closed network
//! guard, plus the on-disk cache the offline listing prefers.
//!
//! This is the one network path in the `providers` crate. It consults the shared `CORTEX_NO_NET`
//! outbound guard FIRST (before any socket), fetches the live models.dev dataset over the
//! rustls-only transport, and curates a bounded multi-provider snapshot:
//! - `openrouter` stays stable by refreshing only the recognizable ids the embedded snapshot tracks;
//! - selected secondary providers are refreshed from models.dev and normalized into
//!   provider-qualified model ids (`provider/model`).
//!
//! Curating against a bounded set keeps listings stable and avoids reintroducing retired ids
//! (for example Claude 3.5 under OpenRouter) while expanding beyond the OpenRouter-only snapshot.
//!
//! On success the curated snapshot is written to a per-user cache (alongside the auth store) so a
//! later offline `cortex models` reflects the refresh. The write is atomic and only happens after
//! a fully successful fetch, so a failed refresh never touches the cache and the catalogue is
//! never corrupted; a malformed cache is ignored in favor of the embedded snapshot.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use errors::Diagnostic;
use redaction::{guard_outbound, redact};
use reqwest::header::ACCEPT;
use serde_json::Value;

use crate::catalogue::{Catalogue, ModelInfo, ProviderInfo};
use crate::credentials::{ENV_DATA_DIR, non_empty_env, non_empty_env_path};
use crate::openrouter::OPENROUTER_ID;
use crate::provider_profiles::{self, ProviderProfile};

/// The canonical live models.dev catalog endpoint.
pub const MODELS_DEV_URL: &str = "https://models.dev/api.json";

/// Optional override for the models.dev endpoint (a self-hosted mirror, or a deliberately
/// unreachable/local endpoint in tests). Mirrors the `CORTEX_OPENROUTER_BASE_URL` seam.
pub const ENV_MODELS_DEV_URL: &str = "CORTEX_MODELS_DEV_URL";

/// The cache file name placed in the per-user data directory next to the auth store.
pub const MODELS_CACHE_FILE: &str = "models.json";

/// The data subdirectory under the user's data home (mirrors the auth/persistence layout).
const DATA_SUBDIR: &str = "cortex";

/// The outbound operation named in a `net-disabled` refusal raised by the refresh.
const OUTBOUND_OPERATION: &str = "models.dev catalog refresh";

/// Bounded connect/total timeouts so a failed refresh degrades within a bounded time (never hangs).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(20);
/// Keep the secondary-provider refresh bounded and deterministic.
const MAX_SECONDARY_MODELS_PER_PROVIDER: usize = 6;

/// The resolved models.dev endpoint: the `CORTEX_MODELS_DEV_URL` override when set and non-empty,
/// else the canonical live endpoint.
#[must_use]
pub fn models_dev_url() -> String {
    non_empty_env(ENV_MODELS_DEV_URL).unwrap_or_else(|| MODELS_DEV_URL.to_owned())
}

/// Fetch and curate the live models.dev catalog into a refreshed [`Catalogue`].
///
/// The `CORTEX_NO_NET` outbound guard is consulted FIRST, so under the kill-switch this returns a
/// `net-disabled` [`Diagnostic`] without opening any socket. A transport/parse failure becomes a
/// `transport-unavailable` (or `upstream`) [`Diagnostic`] so the caller can degrade cleanly.
///
/// # Errors
/// Returns a [`Diagnostic`] for a net-disabled refusal, a transport/connection failure, a non-2xx
/// response, or a response that does not contain any recognized models.
pub async fn refresh_openrouter_catalogue(url: &str) -> Result<Catalogue, Diagnostic> {
    guard_outbound(OUTBOUND_OPERATION)?;
    let bytes = fetch(url).await?;
    parse_and_curate(&bytes)
}

/// Open the bounded rustls-only request and read the response body.
async fn fetch(url: &str) -> Result<Vec<u8>, Diagnostic> {
    let client = reqwest::Client::builder()
        .use_rustls_tls()
        .no_proxy()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(TOTAL_TIMEOUT)
        .user_agent(concat!(
            "cortex-code/",
            env!("CARGO_PKG_VERSION"),
            " models-refresh"
        ))
        .build()
        .map_err(|err| {
            Diagnostic::transport_unavailable(redact(&format!(
                "failed to build HTTP client: {err}"
            )))
        })?;

    let response = client
        .get(url)
        .header(ACCEPT, "application/json")
        .send()
        .await
        .map_err(|err| Diagnostic::transport_unavailable(redact(&err.to_string())))?;

    let status = response.status();
    if !status.is_success() {
        return Err(Diagnostic::upstream(format!(
            "models.dev returned HTTP {}",
            status.as_u16()
        )));
    }

    response
        .bytes()
        .await
        .map(|bytes| bytes.to_vec())
        .map_err(|err| Diagnostic::transport_unavailable(redact(&err.to_string())))
}

/// Parse a models.dev response and curate a bounded multi-provider snapshot.
///
/// OpenRouter remains intentionally stable by refreshing only the recognizable embedded ids.
/// Secondary providers are selected from built-in profiles and normalized to provider-qualified
/// model ids (`provider/model`) so model resolution remains explicit and deterministic.
fn parse_and_curate(bytes: &[u8]) -> Result<Catalogue, Diagnostic> {
    let root: Value = serde_json::from_slice(bytes).map_err(|err| {
        Diagnostic::transport_unavailable(format!("models.dev response was not valid JSON: {err}"))
    })?;
    let openrouter = root.get(OPENROUTER_ID).ok_or_else(|| {
        Diagnostic::transport_unavailable(
            "models.dev response did not include the openrouter provider".to_owned(),
        )
    })?;
    let root_obj = root.as_object().ok_or_else(|| {
        Diagnostic::transport_unavailable("models.dev response root must be a JSON object")
    })?;

    let embedded = Catalogue::embedded()
        .provider(OPENROUTER_ID)
        .expect("embedded snapshot always carries the openrouter provider");

    let mut models = BTreeMap::new();
    if let Some(live_models) = openrouter.get("models").and_then(Value::as_object) {
        for id in embedded.models.keys() {
            if let Some(entry) = live_models.get(id)
                && let Ok(info) = serde_json::from_value::<ModelInfo>(entry.clone())
            {
                models.insert(id.clone(), info);
            }
        }
    }

    if models.is_empty() {
        return Err(Diagnostic::transport_unavailable(
            "models.dev response did not contain any of the recognized models".to_owned(),
        ));
    }

    let provider = ProviderInfo {
        id: OPENROUTER_ID.to_owned(),
        name: string_field(openrouter, "name").unwrap_or_else(|| embedded.name.clone()),
        api: string_field(openrouter, "api").or_else(|| embedded.api.clone()),
        env: string_vec_field(openrouter, "env").unwrap_or_else(|| embedded.env.clone()),
        models,
    };

    let mut providers = BTreeMap::new();
    providers.insert(OPENROUTER_ID.to_owned(), provider);
    for profile in provider_profiles::refresh_profiles() {
        if profile.provider_id == OPENROUTER_ID {
            continue;
        }
        if let Some(provider) = curate_secondary_provider(root_obj, profile) {
            providers.insert(provider.id.clone(), provider);
        }
    }
    Ok(Catalogue::from_providers(providers))
}

fn curate_secondary_provider(
    root_obj: &serde_json::Map<String, Value>,
    profile: &ProviderProfile,
) -> Option<ProviderInfo> {
    let models_dev_id = profile.models_dev_id?;
    let live_provider = root_obj.get(models_dev_id)?;
    let live_models = live_provider.get("models")?.as_object()?;
    let mut model_keys: Vec<&str> = live_models.keys().map(String::as_str).collect();
    model_keys.sort_unstable();

    let mut models = BTreeMap::new();
    for key in model_keys {
        let Some(entry) = live_models.get(key) else {
            continue;
        };
        let Ok(mut info) = serde_json::from_value::<ModelInfo>(entry.clone()) else {
            continue;
        };
        let model_id = canonical_model_id(profile.provider_id, key);
        info.id = model_id.clone();
        models.insert(model_id, info);
        if models.len() >= MAX_SECONDARY_MODELS_PER_PROVIDER {
            break;
        }
    }
    if models.is_empty() {
        return None;
    }

    let env = string_vec_field(live_provider, "env")
        .or_else(|| {
            profile
                .default_api_key_env
                .map(|name| vec![name.to_owned()])
        })
        .unwrap_or_default();
    Some(ProviderInfo {
        id: profile.provider_id.to_owned(),
        name: string_field(live_provider, "name")
            .unwrap_or_else(|| profile.display_name.to_owned()),
        api: string_field(live_provider, "api")
            .or_else(|| profile.default_base_url.map(ToOwned::to_owned)),
        env,
        models,
    })
}

fn canonical_model_id(provider_id: &str, live_model_id: &str) -> String {
    if provider_id == OPENROUTER_ID || live_model_id.starts_with(&format!("{provider_id}/")) {
        return live_model_id.to_owned();
    }
    format!("{provider_id}/{live_model_id}")
}

/// Read a trimmed, non-empty string field from a JSON object.
fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

/// Read a non-empty array-of-strings field from a JSON object.
fn string_vec_field(value: &Value, key: &str) -> Option<Vec<String>> {
    let items: Vec<String> = value
        .get(key)?
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    (!items.is_empty()).then_some(items)
}

/// The on-disk cache path per the documented data-dir precedence (mirrors the auth store):
/// `CORTEX_DATA_DIR/models.json`, else `${XDG_DATA_HOME:-$HOME/.local/share}/cortex/models.json`.
#[must_use]
pub fn models_cache_path() -> Option<PathBuf> {
    resolve_cache_path(
        non_empty_env_path(ENV_DATA_DIR),
        non_empty_env_path("XDG_DATA_HOME"),
        non_empty_env_path("HOME"),
    )
}

/// The pure cache-path decision, separated from the environment read so the precedence is
/// exhaustively unit-testable without mutating the process environment.
fn resolve_cache_path(
    data_dir: Option<PathBuf>,
    xdg_data_home: Option<PathBuf>,
    home: Option<PathBuf>,
) -> Option<PathBuf> {
    if let Some(data_dir) = data_dir {
        return Some(data_dir.join(MODELS_CACHE_FILE));
    }
    let data_home = match xdg_data_home {
        Some(xdg) => xdg,
        None => home?.join(".local").join("share"),
    };
    Some(data_home.join(DATA_SUBDIR).join(MODELS_CACHE_FILE))
}

/// Load the cached catalogue if a readable, well-formed cache file exists.
///
/// Any failure (no cache path, unreadable, or malformed JSON) returns `None` so the caller falls
/// back to the embedded snapshot — a corrupt cache never breaks the offline listing.
#[must_use]
pub fn load_cached_catalogue() -> Option<Catalogue> {
    load_cache_from(&models_cache_path()?)
}

/// Load a catalogue from an explicit cache path, returning `None` on any read/parse failure.
fn load_cache_from(path: &Path) -> Option<Catalogue> {
    let json = std::fs::read_to_string(path).ok()?;
    Catalogue::from_json(&json).ok()
}

/// Atomically write the refreshed catalogue to the on-disk cache.
///
/// The snapshot is written to a temporary sibling and renamed into place, so a partial write can
/// never corrupt an existing cache. A missing data directory (no cache path resolvable) is a
/// no-op. The caller treats any error as non-fatal: the refresh has already produced its listing.
///
/// # Errors
/// Returns the underlying [`std::io::Error`] if serialization, the write, or the rename fails.
pub fn write_models_cache(catalogue: &Catalogue) -> std::io::Result<()> {
    match models_cache_path() {
        Some(path) => write_cache_to(&path, catalogue),
        None => Ok(()),
    }
}

/// Atomically write a catalogue to an explicit cache path (temp sibling + rename).
fn write_cache_to(path: &Path, catalogue: &Catalogue) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = catalogue
        .to_snapshot_json()
        .map_err(std::io::Error::other)?;
    let tmp = path.with_file_name(format!("{MODELS_CACHE_FILE}.tmp.{}", std::process::id()));
    std::fs::write(&tmp, json.as_bytes())?;
    if let Err(err) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(err);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A models.dev-shaped fixture: the openrouter provider with a handful of recognizable ids,
    /// an unrelated extra model, a (retired) Claude 3.5 id, and one malformed recognizable entry
    /// missing its required `name`. Other providers are present to mirror the real response shape.
    fn fixture() -> String {
        r#"{
          "anthropic": { "id": "anthropic", "name": "Anthropic", "models": {} },
          "azure": {
            "id": "azure",
            "name": "Azure OpenAI",
            "api": "https://azure.example.test/openai/v1",
            "env": ["AZURE_OPENAI_API_KEY"],
            "models": {
              "gpt-4.1-mini": { "id": "gpt-4.1-mini", "name": "GPT-4.1 mini", "tool_call": true, "cost": { "input": 0.4, "output": 1.6 }, "limit": { "context": 1047576, "output": 32768 } }
            }
          },
          "xai": {
            "id": "xai",
            "name": "xAI",
            "api": "https://api.x.ai/v1",
            "env": ["XAI_API_KEY"],
            "models": {
              "grok-4.20-0309-non-reasoning": { "id": "grok-4.20-0309-non-reasoning", "name": "xAI model", "tool_call": true, "cost": { "input": 3.0, "output": 15.0 }, "limit": { "context": 262144, "output": 32768 } }
            }
          },
          "openrouter": {
            "id": "openrouter",
            "name": "OpenRouter",
            "api": "https://openrouter.ai/api/v1",
            "env": ["OPENROUTER_API_KEY"],
            "models": {
              "google/gemini-2.5-flash": { "id": "google/gemini-2.5-flash", "name": "Gemini 2.5 Flash", "tool_call": true, "cost": { "input": 0.3, "output": 2.5 }, "limit": { "context": 1048576, "output": 65535 } },
              "anthropic/claude-sonnet-4": { "id": "anthropic/claude-sonnet-4", "name": "Claude Sonnet 4", "tool_call": true, "cost": { "input": 3, "output": 15 }, "limit": { "context": 1000000, "output": 64000 } },
              "openai/o3": { "id": "openai/o3", "cost": { "input": 2, "output": 8 }, "limit": { "context": 200000, "output": 100000 } },
              "anthropic/claude-3.5-sonnet": { "id": "anthropic/claude-3.5-sonnet", "name": "Claude 3.5 Sonnet", "cost": { "input": 3, "output": 15 }, "limit": { "context": 200000, "output": 8192 } },
              "some/unlisted-model-v9": { "id": "some/unlisted-model-v9", "name": "Unlisted", "cost": { "input": 1, "output": 1 }, "limit": { "context": 1000, "output": 100 } }
            }
          }
        }"#
        .to_owned()
    }

    fn curate(json: &str) -> Result<Catalogue, Diagnostic> {
        parse_and_curate(json.as_bytes())
    }

    #[test]
    fn curate_keeps_recognizable_models_and_excludes_others() {
        let catalogue = curate(&fixture()).expect("curates");
        let openrouter = catalogue
            .provider(OPENROUTER_ID)
            .expect("openrouter present");

        // Recognizable, well-formed models are kept.
        assert!(openrouter.models.contains_key("google/gemini-2.5-flash"));
        assert!(openrouter.models.contains_key("anthropic/claude-sonnet-4"));

        // A retired Claude 3.5 id present upstream is NEVER reintroduced (it is not in the
        // embedded recognizable set), and an unrelated/unlisted id is not added.
        assert!(
            !openrouter
                .models
                .contains_key("anthropic/claude-3.5-sonnet")
        );
        assert!(!openrouter.models.contains_key("some/unlisted-model-v9"));

        // Provider metadata comes from the live response.
        assert_eq!(openrouter.name, "OpenRouter");
        assert_eq!(
            openrouter.api.as_deref(),
            Some("https://openrouter.ai/api/v1")
        );
        assert_eq!(openrouter.env, vec!["OPENROUTER_API_KEY".to_owned()]);
    }

    #[test]
    fn curate_adds_secondary_profiles_with_provider_qualified_model_ids() {
        let catalogue = curate(&fixture()).expect("curates");
        let azure = catalogue.provider("azure").expect("azure provider present");
        assert_eq!(azure.name, "Azure OpenAI");
        assert_eq!(
            azure.api.as_deref(),
            Some("https://azure.example.test/openai/v1")
        );
        assert_eq!(azure.env, vec!["AZURE_OPENAI_API_KEY".to_owned()]);
        assert!(
            azure.models.contains_key("azure/gpt-4.1-mini"),
            "secondary provider model ids must be provider-qualified"
        );

        let xai = catalogue.provider("xai").expect("xai provider present");
        assert!(xai.models.contains_key("xai/grok-4.20-0309-non-reasoning"));
    }

    #[test]
    fn curate_drops_a_single_malformed_entry_but_keeps_the_rest() {
        let catalogue = curate(&fixture()).expect("curates");
        let openrouter = catalogue
            .provider(OPENROUTER_ID)
            .expect("openrouter present");
        // `openai/o3` is malformed in the fixture (missing the required `name`) so it is dropped,
        // while the well-formed recognizable models survive.
        assert!(!openrouter.models.contains_key("openai/o3"));
        assert!(openrouter.models.len() >= 2);
    }

    #[test]
    fn curate_refreshes_live_pricing_for_a_recognized_model() {
        let catalogue = curate(&fixture()).expect("curates");
        let flash = catalogue
            .model(OPENROUTER_ID, "google/gemini-2.5-flash")
            .expect("flash present");
        assert_eq!(flash.cost.input, 0.3);
        assert_eq!(flash.cost.output, 2.5);
        assert_eq!(flash.limit.context, 1_048_576);
    }

    #[test]
    fn curate_errors_when_openrouter_is_absent() {
        let err = curate(r#"{ "anthropic": { "id": "anthropic", "name": "A", "models": {} } }"#)
            .expect_err("missing openrouter is an error");
        assert_eq!(err.kind(), "transport-unavailable");
    }

    #[test]
    fn curate_errors_when_no_recognized_models_are_present() {
        let err = curate(
            r#"{ "openrouter": { "id": "openrouter", "name": "OpenRouter", "models": { "x/y": { "id": "x/y", "name": "XY" } } } }"#,
        )
        .expect_err("no recognized models is an error");
        assert_eq!(err.kind(), "transport-unavailable");
    }

    #[test]
    fn curate_errors_on_invalid_json_without_panicking() {
        let err = curate("{ not json").expect_err("invalid json is an error");
        assert_eq!(err.kind(), "transport-unavailable");
    }

    #[test]
    fn cache_path_precedence_matches_the_auth_store() {
        // CORTEX_DATA_DIR wins outright.
        assert_eq!(
            resolve_cache_path(
                Some(PathBuf::from("/data")),
                Some(PathBuf::from("/xdg")),
                Some(PathBuf::from("/home/u"))
            ),
            Some(PathBuf::from("/data/models.json"))
        );
        // Then XDG_DATA_HOME/cortex.
        assert_eq!(
            resolve_cache_path(
                None,
                Some(PathBuf::from("/xdg")),
                Some(PathBuf::from("/home/u"))
            ),
            Some(PathBuf::from("/xdg/cortex/models.json"))
        );
        // Then $HOME/.local/share/cortex.
        assert_eq!(
            resolve_cache_path(None, None, Some(PathBuf::from("/home/u"))),
            Some(PathBuf::from("/home/u/.local/share/cortex/models.json"))
        );
        // Nothing resolvable.
        assert_eq!(resolve_cache_path(None, None, None), None);
    }

    #[test]
    fn cache_round_trips_through_disk() {
        let catalogue = curate(&fixture()).expect("curates");
        let dir = std::env::temp_dir().join(format!(
            "cortex-models-cache-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = dir.join("models.json");

        write_cache_to(&path, &catalogue).expect("writes cache");
        let loaded = load_cache_from(&path).expect("loads cache");
        assert_eq!(loaded, catalogue, "the cache round-trips losslessly");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_cache_file_loads_as_none() {
        let dir = std::env::temp_dir().join(format!(
            "cortex-models-cache-corrupt-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("create dir");
        let path = dir.join("models.json");
        std::fs::write(&path, b"{ not valid json").expect("write corrupt");

        assert!(
            load_cache_from(&path).is_none(),
            "a corrupt cache is ignored rather than crashing"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
