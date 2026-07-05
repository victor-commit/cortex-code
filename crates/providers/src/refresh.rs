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

use crate::catalogue::{Catalogue, ModelInfo, ProviderInfo, is_excluded_zen};
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
/// Keep the per-provider refresh bounded and deterministic; the newest models are kept.
const MAX_MODELS_PER_SECONDARY_PROVIDER: usize = 6;

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
pub async fn refresh_models_dev_catalogue(url: &str) -> Result<Catalogue, Diagnostic> {
    guard_outbound(OUTBOUND_OPERATION)?;
    let bytes = fetch(url).await?;
    parse_and_curate(&bytes)
}

/// Resolve the multi-provider catalogue the picker and CLI draw from, gated by the outbound guard.
///
/// - When outbound network is permitted: attempt a live models.dev refresh; on success the curated
///   snapshot is cached (best-effort) and returned; on failure it degrades to the offline sources.
/// - Under `CORTEX_NO_NET` (or any refusal): NO socket is opened — the catalogue resolves from the
///   on-disk cache, then the embedded snapshot, with no error.
///
/// The result always omits the excluded Zen provider (every catalogue built in this crate does).
#[must_use]
pub fn resolve_catalogue() -> Catalogue {
    resolve_catalogue_core(
        guard_outbound(OUTBOUND_OPERATION).is_err(),
        load_cached_catalogue(),
        || refresh_blocking(&models_dev_url()),
        |catalogue| {
            let _ = write_models_cache(catalogue);
        },
    )
}

/// The offline catalogue: the on-disk cache written by a prior refresh when present and valid,
/// otherwise the embedded snapshot. No network, no error; the excluded Zen provider is omitted
/// either way.
#[must_use]
pub fn offline_catalogue() -> Catalogue {
    choose_offline(load_cached_catalogue())
}

/// The offline catalogue paired with the source tag the CLI and server report: `"cached"` when a
/// prior `--refresh` wrote the on-disk cache, else `"builtin"` for the embedded snapshot.
///
/// This is the single source of truth for the cache-then-embedded offline resolution that
/// `cortex models` and the server provider listing draw from, so neither re-implements the
/// fallback. No network, no error; the excluded Zen provider is omitted either way.
#[must_use]
pub fn offline_catalogue_with_source() -> (Catalogue, &'static str) {
    choose_offline_with_source(load_cached_catalogue())
}

/// The pure resolve decision, separated from the environment/socket/disk reads so both the online
/// and offline branches are unit-testable without touching the network or the process env.
fn resolve_catalogue_core<Fetch, Write>(
    network_disabled: bool,
    cached: Option<Catalogue>,
    fetch: Fetch,
    write: Write,
) -> Catalogue
where
    Fetch: FnOnce() -> Result<Catalogue, Diagnostic>,
    Write: FnOnce(&Catalogue),
{
    if network_disabled {
        return choose_offline(cached);
    }
    match fetch() {
        Ok(catalogue) => {
            write(&catalogue);
            catalogue
        }
        Err(_) => choose_offline(cached),
    }
}

/// Prefer the cache, then fall back to the embedded snapshot.
fn choose_offline(cached: Option<Catalogue>) -> Catalogue {
    choose_offline_with_source(cached).0
}

/// The cache-then-embedded decision paired with the `"cached"`/`"builtin"` source tag the
/// CLI/server surface as each entry's `source`.
fn choose_offline_with_source(cached: Option<Catalogue>) -> (Catalogue, &'static str) {
    match cached {
        Some(cached) => (cached, "cached"),
        None => (Catalogue::embedded().clone(), "builtin"),
    }
}

/// Run the async refresh to completion on a dedicated thread with its own current-thread runtime.
///
/// Running the fetch on a fresh thread keeps [`resolve_catalogue`] safe to call from a synchronous
/// context (the TUI event loop) or from within an existing async runtime without nesting one.
fn refresh_blocking(url: &str) -> Result<Catalogue, Diagnostic> {
    let url = url.to_owned();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|err| {
                Diagnostic::transport_unavailable(redact(&format!(
                    "failed to start the models.dev refresh runtime: {err}"
                )))
            })?;
        runtime.block_on(refresh_models_dev_catalogue(&url))
    })
    .join()
    .unwrap_or_else(|_| {
        Err(Diagnostic::transport_unavailable(
            "the models.dev refresh worker thread panicked".to_owned(),
        ))
    })
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
/// OpenRouter remains intentionally stable by refreshing only the recognizable embedded ids (so a
/// retired id is never reintroduced). Every OTHER top-level models.dev provider is surfaced too
/// (except the excluded Zen provider, which is omitted everywhere), normalized to provider-qualified model ids
/// (`provider/model`) and capped to its most recent models — so the merged catalogue draws from
/// the dynamic source across many providers, not just openrouter.
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

    // Surface every OTHER top-level models.dev provider (minus the excluded Zen provider), so the merged
    // catalogue draws from the dynamic source across many providers. A built-in profile, when one
    // exists, supplies runtime-default fallbacks (env/base_url/display name).
    for (models_dev_id, live_provider) in root_obj {
        if models_dev_id == OPENROUTER_ID || is_excluded_zen(models_dev_id) {
            continue;
        }
        let profile = provider_profiles::profile_by_models_dev_id(models_dev_id);
        let provider_id = profile.map_or(models_dev_id.as_str(), |profile| profile.provider_id);
        if is_excluded_zen(provider_id) {
            continue;
        }
        if let Some(provider) = curate_provider(provider_id, models_dev_id, live_provider, profile)
        {
            providers.insert(provider.id.clone(), provider);
        }
    }
    Ok(Catalogue::from_providers(providers))
}

/// Curate one non-openrouter provider from its models.dev entry: parse every well-formed model,
/// qualify each id to `provider/model`, and keep only the most-recent [`MAX_MODELS_PER_SECONDARY_PROVIDER`]
/// (recency = the models.dev `release_date`, then the id) so the cached slice is bounded and
/// newest-first. Provider metadata comes from the live response, with profile defaults as fallback.
fn curate_provider(
    provider_id: &str,
    models_dev_id: &str,
    live_provider: &Value,
    profile: Option<&ProviderProfile>,
) -> Option<ProviderInfo> {
    let live_models = live_provider.get("models")?.as_object()?;

    let mut parsed: Vec<ModelInfo> = live_models
        .iter()
        .filter_map(|(key, entry)| {
            let mut info = serde_json::from_value::<ModelInfo>(entry.clone()).ok()?;
            info.id = canonical_model_id(provider_id, key);
            Some(info)
        })
        .collect();
    if parsed.is_empty() {
        return None;
    }
    parsed.sort_by(|a, b| {
        b.release_date
            .cmp(&a.release_date)
            .then_with(|| b.id.cmp(&a.id))
    });
    parsed.truncate(MAX_MODELS_PER_SECONDARY_PROVIDER);

    let models: BTreeMap<String, ModelInfo> = parsed
        .into_iter()
        .map(|info| (info.id.clone(), info))
        .collect();

    let env = string_vec_field(live_provider, "env")
        .or_else(|| {
            profile
                .and_then(|profile| profile.default_api_key_env)
                .map(|name| vec![name.to_owned()])
        })
        .unwrap_or_default();
    let name = string_field(live_provider, "name")
        .or_else(|| profile.map(|profile| profile.display_name.to_owned()))
        .unwrap_or_else(|| models_dev_id.to_owned());
    let api = string_field(live_provider, "api").or_else(|| {
        profile
            .and_then(|profile| profile.default_base_url)
            .map(ToOwned::to_owned)
    });
    Some(ProviderInfo {
        id: provider_id.to_owned(),
        name,
        api,
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
    /// missing its required `name`. Other providers are present to mirror the real response shape,
    /// including one WITHOUT a built-in profile (`openai`, with dated models) and the excluded Zen
    /// provider (its models.dev id read from the brand-safe `EXCLUDED_ZEN_PROVIDER_IDS`), which
    /// must be stripped from the curated catalogue entirely.
    fn fixture() -> String {
        let zen = crate::EXCLUDED_ZEN_PROVIDER_IDS[0].to_owned();
        let zen_env = format!("{}_API_KEY", zen.to_ascii_uppercase());
        let zen_entry = format!(
            r#""{zen}": {{
            "id": "{zen}",
            "name": "{zen} Zen",
            "api": "https://{zen}.ai/zen/v1",
            "env": ["{zen_env}"],
            "models": {{
              "grok-code": {{ "id": "grok-code", "name": "code model", "cost": {{ "input": 0.0, "output": 0.0 }}, "limit": {{ "context": 256000, "output": 8192 }} }}
            }}
          }}"#
        );
        let head = r#"{
          "anthropic": { "id": "anthropic", "name": "Anthropic", "models": {} },
          "#;
        let tail = r#",
          "openai": {
            "id": "openai",
            "name": "OpenAI",
            "api": "https://api.openai.com/v1",
            "env": ["OPENAI_API_KEY"],
            "models": {
              "gpt-5": { "id": "gpt-5", "name": "GPT-5", "tool_call": true, "release_date": "2025-08-01", "cost": { "input": 1.25, "output": 10.0 }, "limit": { "context": 400000, "output": 128000 } },
              "gpt-4o": { "id": "gpt-4o", "name": "GPT-4o", "tool_call": true, "release_date": "2024-05-13", "cost": { "input": 2.5, "output": 10.0 }, "limit": { "context": 128000, "output": 16384 } }
            }
          },
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
        }"#;
        format!("{head}{zen_entry}{tail}")
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
    fn curate_surfaces_providers_without_a_builtin_profile() {
        // `openai` has no built-in profile, yet it is surfaced from the models.dev response with
        // its live metadata and provider-qualified model ids.
        let catalogue = curate(&fixture()).expect("curates");
        let openai = catalogue.provider("openai").expect("openai present");
        assert_eq!(openai.name, "OpenAI");
        assert_eq!(openai.api.as_deref(), Some("https://api.openai.com/v1"));
        assert_eq!(openai.env, vec!["OPENAI_API_KEY".to_owned()]);
        assert!(openai.models.contains_key("openai/gpt-5"));
        assert!(openai.models.contains_key("openai/gpt-4o"));
    }

    #[test]
    fn curate_draws_from_many_providers_not_just_openrouter() {
        let catalogue = curate(&fixture()).expect("curates");
        let provider_ids: Vec<&str> = catalogue.providers().map(|p| p.id.as_str()).collect();
        for expected in ["openrouter", "openai", "azure", "xai"] {
            assert!(
                provider_ids.contains(&expected),
                "merged catalogue must include {expected}: {provider_ids:?}"
            );
        }
        assert!(
            provider_ids.len() >= 4,
            "the catalogue must draw from many providers, found {provider_ids:?}"
        );
    }

    #[test]
    fn curate_excludes_the_zen_provider_entirely() {
        let zen = crate::EXCLUDED_ZEN_PROVIDER_IDS[0].to_owned();
        let zen_model = format!("{zen}/grok-code");
        let catalogue = curate(&fixture()).expect("curates");
        assert!(
            catalogue.provider(&zen).is_none(),
            "the excluded Zen provider must be stripped from the catalogue"
        );
        assert!(catalogue.provider("zen").is_none());
        assert!(
            catalogue.providers().all(|p| p.id != zen && p.id != "zen"),
            "no provider group may be the excluded Zen provider"
        );
        assert!(
            catalogue.find_model(&zen_model).is_none(),
            "no excluded-Zen model may leak into the catalogue"
        );
    }

    #[test]
    fn curate_orders_provider_models_recent_first_not_alphabetical() {
        let catalogue = curate(&fixture()).expect("curates");
        let openai = catalogue.provider("openai").expect("openai present");
        let ordered: Vec<&str> = openai
            .models_recent_first()
            .iter()
            .map(|model| model.id.as_str())
            .collect();
        // The newer (2025) model sorts above the older (2024) one; alphabetical order would place
        // `openai/gpt-4o` first, so recency ordering is observable.
        assert_eq!(ordered.first().copied(), Some("openai/gpt-5"));
        assert_eq!(ordered.last().copied(), Some("openai/gpt-4o"));
    }

    #[test]
    fn curate_caps_secondary_providers_to_their_most_recent_models() {
        let mut entries = String::new();
        for index in 0..8 {
            if index > 0 {
                entries.push(',');
            }
            // release year climbs with the index, so index 7 is the newest model.
            entries.push_str(&format!(
                "\"m{index}\": {{ \"id\": \"m{index}\", \"name\": \"M{index}\", \"release_date\": \"20{:02}-01-01\", \"cost\": {{ \"input\": 1, \"output\": 1 }}, \"limit\": {{ \"context\": 1000, \"output\": 100 }} }}",
                10 + index
            ));
        }
        let json = format!(
            "{{ \"openrouter\": {{ \"id\": \"openrouter\", \"name\": \"OpenRouter\", \"models\": {{ \"google/gemini-2.5-flash\": {{ \"id\": \"google/gemini-2.5-flash\", \"name\": \"Gemini 2.5 Flash\", \"cost\": {{ \"input\": 0.3, \"output\": 2.5 }}, \"limit\": {{ \"context\": 1048576, \"output\": 65535 }} }} }} }}, \"acme\": {{ \"id\": \"acme\", \"name\": \"Acme\", \"models\": {{ {entries} }} }} }}"
        );
        let catalogue = curate(&json).expect("curates");
        let acme = catalogue.provider("acme").expect("acme present");
        assert_eq!(acme.models.len(), MAX_MODELS_PER_SECONDARY_PROVIDER);
        // The newest models are kept; the two oldest (m0, m1) are dropped by the recency cap.
        assert!(acme.models.contains_key("acme/m7"), "newest kept");
        assert!(!acme.models.contains_key("acme/m0"), "oldest dropped");
        assert!(!acme.models.contains_key("acme/m1"), "oldest dropped");
    }

    #[test]
    fn resolve_catalogue_offline_never_fetches_and_prefers_the_cache() {
        let cached = curate(&fixture()).expect("curates");
        let resolved = resolve_catalogue_core(
            true,
            Some(cached.clone()),
            || panic!("no fetch may be attempted when the network is disabled"),
            |_| panic!("no cache write may happen when offline"),
        );
        assert_eq!(resolved, cached, "offline resolves from the on-disk cache");
    }

    #[test]
    fn resolve_catalogue_offline_without_cache_falls_back_to_embedded() {
        let resolved = resolve_catalogue_core(
            true,
            None,
            || panic!("no fetch when offline"),
            |_| panic!("no write when offline"),
        );
        assert!(
            resolved.provider(OPENROUTER_ID).is_some(),
            "offline with no cache lists the embedded snapshot with no error"
        );
    }

    #[test]
    fn resolve_catalogue_online_refreshes_and_writes_the_cache() {
        let refreshed = curate(&fixture()).expect("curates");
        let wrote = std::cell::Cell::new(false);
        let resolved =
            resolve_catalogue_core(false, None, || Ok(refreshed.clone()), |_| wrote.set(true));
        assert!(
            wrote.get(),
            "a successful online refresh writes the on-disk cache"
        );
        assert_eq!(resolved, refreshed);
        assert!(
            resolved.provider("openai").is_some(),
            "the refreshed catalogue is multi-provider"
        );
    }

    #[test]
    fn resolve_catalogue_online_failure_degrades_to_the_cache() {
        let cached = curate(&fixture()).expect("curates");
        let resolved = resolve_catalogue_core(
            false,
            Some(cached.clone()),
            || Err(Diagnostic::transport_unavailable("boom".to_owned())),
            |_| panic!("a failed refresh must not touch the cache"),
        );
        assert_eq!(resolved, cached, "a failed refresh degrades to the cache");
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
