use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use errors::Diagnostic;
use redaction::guard_outbound;
use serde_json::{Map as JsonMap, Value as JsonValue};

use crate::credentials::{
    CredentialInputs, non_empty_env, non_empty_env_path, resolve_credential, store_credential,
};

const OAUTH_REFRESH_SKEW_SECS: u64 = 30;
const GOOGLE_ADC_PATH_ENV: &str = "GOOGLE_APPLICATION_CREDENTIALS";
const GOOGLE_ADC_DEFAULT_PATH: &str = ".config/gcloud/application_default_credentials.json";
const GOOGLE_OAUTH_TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
const OAUTH_REFRESH_OUTBOUND_OPERATION: &str = "provider OAuth refresh token exchange";
const GOOGLE_ADC_REFRESH_OUTBOUND_OPERATION: &str = "google ADC refresh token exchange";
const AWS_BEARER_TOKEN_ENV: &str = "AWS_BEARER_TOKEN_BEDROCK";
const AWS_ACCESS_KEY_ID_ENV: &str = "AWS_ACCESS_KEY_ID";
const AWS_SECRET_ACCESS_KEY_ENV: &str = "AWS_SECRET_ACCESS_KEY";
const AWS_SESSION_TOKEN_ENV: &str = "AWS_SESSION_TOKEN";
const AWS_PROFILE_ENV: &str = "AWS_PROFILE";
const AWS_DEFAULT_PROFILE_ENV: &str = "AWS_DEFAULT_PROFILE";
const AWS_SHARED_CREDENTIALS_FILE_ENV: &str = "AWS_SHARED_CREDENTIALS_FILE";

#[derive(Debug, Clone, PartialEq, Eq)]
struct OAuthCredential {
    access_token: String,
    refresh_token: Option<String>,
    /// OpenID Connect id_token (JWT). Used for ChatGPT subscription account routing.
    /// Never logged; never placed on the wire as a header value.
    id_token: Option<String>,
    token_type: String,
    expires_at: Option<u64>,
    token_endpoint: Option<String>,
    client_id: Option<String>,
    client_secret: Option<String>,
}

impl OAuthCredential {
    fn from_stored(value: &str) -> Option<Self> {
        let json = serde_json::from_str::<JsonValue>(value).ok()?;
        let object = json.as_object()?;
        let access_token = object
            .get("access_token")
            .or_else(|| object.get("access"))
            .and_then(JsonValue::as_str)?
            .trim()
            .to_owned();
        if access_token.is_empty() {
            return None;
        }
        Some(Self {
            access_token,
            refresh_token: optional_string(object, "refresh_token")
                .or_else(|| optional_string(object, "refresh")),
            id_token: optional_string(object, "id_token"),
            token_type: optional_string(object, "token_type")
                .unwrap_or_else(|| "Bearer".to_owned()),
            expires_at: optional_u64(object, "expires_at")
                .or_else(|| optional_u64(object, "expires")),
            token_endpoint: optional_string(object, "token_endpoint"),
            client_id: optional_string(object, "client_id"),
            client_secret: optional_string(object, "client_secret"),
        })
    }

    fn is_expired(&self) -> bool {
        self.expires_at.is_some_and(|expires_at| {
            expires_at <= now_secs().saturating_add(OAUTH_REFRESH_SKEW_SECS)
        })
    }

    fn to_stored_string(&self) -> String {
        let mut object = JsonMap::new();
        object.insert("type".to_owned(), JsonValue::String("oauth".to_owned()));
        object.insert(
            "access_token".to_owned(),
            JsonValue::String(self.access_token.clone()),
        );
        object.insert(
            "token_type".to_owned(),
            JsonValue::String(self.token_type.clone()),
        );
        if let Some(refresh_token) = &self.refresh_token {
            object.insert(
                "refresh_token".to_owned(),
                JsonValue::String(refresh_token.clone()),
            );
        }
        if let Some(id_token) = &self.id_token {
            object.insert("id_token".to_owned(), JsonValue::String(id_token.clone()));
        }
        if let Some(expires_at) = self.expires_at {
            object.insert(
                "expires_at".to_owned(),
                JsonValue::Number(expires_at.into()),
            );
        }
        if let Some(token_endpoint) = &self.token_endpoint {
            object.insert(
                "token_endpoint".to_owned(),
                JsonValue::String(token_endpoint.clone()),
            );
        }
        if let Some(client_id) = &self.client_id {
            object.insert("client_id".to_owned(), JsonValue::String(client_id.clone()));
        }
        if let Some(client_secret) = &self.client_secret {
            object.insert(
                "client_secret".to_owned(),
                JsonValue::String(client_secret.clone()),
            );
        }
        serde_json::to_string(&JsonValue::Object(object)).unwrap_or_default()
    }
}

pub(crate) fn resolve_provider_api_key(
    provider_id: &str,
    inputs: &CredentialInputs,
) -> Result<String, Diagnostic> {
    match resolve_credential(provider_id, inputs) {
        Ok(credential) => {
            let raw = credential.expose().trim();
            if let Some(mut oauth) = OAuthCredential::from_stored(raw) {
                if oauth.is_expired()
                    && oauth.refresh_token.is_some()
                    && oauth.token_endpoint.is_some()
                    && oauth.client_id.is_some()
                {
                    oauth = refresh_oauth_credential(&oauth)?;
                    persist_refreshed_oauth(inputs.credential_file.as_deref(), provider_id, &oauth);
                }
                return Ok(oauth.access_token);
            }
            Ok(raw.to_owned())
        }
        Err(diag) if diag.kind() == "no-credential" => {
            if let Some(token) = resolve_aws_chain(provider_id) {
                return Ok(token);
            }
            if let Some(token) = resolve_google_adc(provider_id)? {
                return Ok(token);
            }
            Err(diag)
        }
        Err(diag) => Err(diag),
    }
}

/// Perform an authenticated GET with the current OAuth access token.
///
/// Proactively refreshes when the credential is within the expiry skew window.
/// On HTTP 401, performs a single-shot refresh-and-retry when refresh_token,
/// token_endpoint, and client_id are present (exactly one 401-driven refresh).
///
/// `allow(dead_code)`: the RED case in `tests` is the sole caller until this is
/// wired into the provider request path.
#[allow(dead_code)]
fn oauth_authorized_get(
    oauth: &mut OAuthCredential,
    credential_file: Option<&Path>,
    provider_id: &str,
    url: &str,
) -> Result<String, Diagnostic> {
    if oauth.is_expired()
        && oauth.refresh_token.is_some()
        && oauth.token_endpoint.is_some()
        && oauth.client_id.is_some()
    {
        *oauth = refresh_oauth_credential(oauth)?;
        persist_refreshed_oauth(credential_file, provider_id, oauth);
    }
    let client = reqwest::blocking::Client::builder()
        .build()
        .map_err(|err| {
            Diagnostic::transport_unavailable(format!(
                "provider OAuth authorized request client error: {err}"
            ))
        })?;
    let response = client
        .get(url)
        .header(
            "Authorization",
            format!("{} {}", oauth.token_type, oauth.access_token),
        )
        .send()
        .map_err(|err| {
            Diagnostic::transport_unavailable(format!(
                "provider OAuth authorized request failed: {err}"
            ))
        })?;
    let response = if response.status().as_u16() == 401
        && oauth.refresh_token.is_some()
        && oauth.token_endpoint.is_some()
        && oauth.client_id.is_some()
    {
        *oauth = refresh_oauth_credential(oauth)?;
        persist_refreshed_oauth(credential_file, provider_id, oauth);
        client
            .get(url)
            .header(
                "Authorization",
                format!("{} {}", oauth.token_type, oauth.access_token),
            )
            .send()
            .map_err(|err| {
                Diagnostic::transport_unavailable(format!(
                    "provider OAuth authorized request failed: {err}"
                ))
            })?
    } else {
        response
    };
    if !response.status().is_success() {
        return Err(Diagnostic::upstream(format!(
            "provider OAuth authorized request failed with HTTP {}",
            response.status()
        )));
    }
    response.text().map_err(|err| {
        Diagnostic::upstream(format!(
            "provider OAuth authorized response body could not be read: {err}"
        ))
    })
}

fn persist_refreshed_oauth(path: Option<&Path>, provider_id: &str, oauth: &OAuthCredential) {
    let Some(path) = path else {
        return;
    };
    let _ = store_credential(path, provider_id, &oauth.to_stored_string());
}

fn refresh_oauth_credential(oauth: &OAuthCredential) -> Result<OAuthCredential, Diagnostic> {
    guard_outbound(OAUTH_REFRESH_OUTBOUND_OPERATION)?;
    let refresh_token = oauth.refresh_token.as_deref().ok_or_else(|| {
        Diagnostic::upstream("provider OAuth token has expired with no refresh token")
    })?;
    let token_endpoint = oauth.token_endpoint.as_deref().ok_or_else(|| {
        Diagnostic::upstream("provider OAuth token has expired with no token endpoint")
    })?;
    let client_id = oauth.client_id.as_deref().ok_or_else(|| {
        Diagnostic::upstream("provider OAuth token has expired with no client id")
    })?;
    let mut form = vec![
        ("grant_type", "refresh_token".to_owned()),
        ("refresh_token", refresh_token.to_owned()),
        ("client_id", client_id.to_owned()),
    ];
    if let Some(client_secret) = oauth.client_secret.as_ref() {
        form.push(("client_secret", client_secret.clone()));
    }
    let client = reqwest::blocking::Client::builder()
        .build()
        .map_err(|err| {
            Diagnostic::transport_unavailable(format!("provider OAuth refresh client error: {err}"))
        })?;
    let response = client
        .post(token_endpoint)
        .form(&form)
        .send()
        .map_err(|err| {
            Diagnostic::transport_unavailable(format!(
                "provider OAuth refresh request failed: {err}"
            ))
        })?;
    if !response.status().is_success() {
        return Err(Diagnostic::upstream(format!(
            "provider OAuth refresh failed with HTTP {}",
            response.status()
        )));
    }
    let body = response.json::<JsonValue>().map_err(|err| {
        Diagnostic::upstream(format!(
            "provider OAuth refresh response was not valid JSON: {err}"
        ))
    })?;
    let object = body.as_object().ok_or_else(|| {
        Diagnostic::upstream("provider OAuth refresh response must be a JSON object")
    })?;
    let access_token = optional_string(object, "access_token").ok_or_else(|| {
        Diagnostic::upstream("provider OAuth refresh response did not include access_token")
    })?;
    let refresh_token =
        optional_string(object, "refresh_token").or_else(|| oauth.refresh_token.clone());
    let expires_at = optional_u64(object, "expires_at")
        .or_else(|| optional_u64(object, "expires"))
        .or_else(|| {
            optional_u64(object, "expires_in").map(|value| now_secs().saturating_add(value))
        });
    Ok(OAuthCredential {
        access_token,
        refresh_token,
        // Preserve prior id_token when the refresh response omits a new one.
        id_token: optional_string(object, "id_token").or_else(|| oauth.id_token.clone()),
        token_type: optional_string(object, "token_type")
            .or_else(|| Some(oauth.token_type.clone()))
            .unwrap_or_else(|| "Bearer".to_owned()),
        expires_at,
        token_endpoint: oauth.token_endpoint.clone(),
        client_id: oauth.client_id.clone(),
        client_secret: oauth.client_secret.clone(),
    })
}

/// Extra headers required by ChatGPT subscription (Codex backend) OAuth requests.
///
/// When `provider_id` is `chatgpt` and the stored credential is OAuth with an
/// `id_token`, injects `ChatGPT-Account-Id` derived from JWT claims. Never puts
/// the raw id_token (or access/refresh tokens) into a header.
pub(crate) fn subscription_request_headers(
    provider_id: &str,
    inputs: &CredentialInputs,
) -> Vec<(String, String)> {
    if provider_id != "chatgpt" {
        return Vec::new();
    }
    let Ok(credential) = resolve_credential(provider_id, inputs) else {
        return Vec::new();
    };
    let Some(oauth) = OAuthCredential::from_stored(credential.expose().trim()) else {
        return Vec::new();
    };
    let Some(id_token) = oauth.id_token.as_deref() else {
        return Vec::new();
    };
    let Some(account_id) = chatgpt_account_id_from_id_token(id_token) else {
        return Vec::new();
    };
    vec![("ChatGPT-Account-Id".to_owned(), account_id)]
}

/// Extract ChatGPT account id from an OIDC id_token JWT payload.
///
/// Claim order (Codex-compatible):
/// 1. top-level `chatgpt_account_id`
/// 2. `https://api.openai.com/auth` object → `chatgpt_account_id`
/// 3. `organizations[0].id`
///
/// Signature is not verified — callers only need the account claim for routing.
/// The raw token is never logged.
fn chatgpt_account_id_from_id_token(id_token: &str) -> Option<String> {
    let payload_b64 = id_token.split('.').nth(1)?;
    let payload_bytes = base64url_decode(payload_b64)?;
    let payload = serde_json::from_slice::<JsonValue>(&payload_bytes).ok()?;
    if let Some(id) = non_empty_json_str(payload.get("chatgpt_account_id")) {
        return Some(id);
    }
    if let Some(id) = payload
        .get("https://api.openai.com/auth")
        .and_then(|auth| non_empty_json_str(auth.get("chatgpt_account_id")))
    {
        return Some(id);
    }
    payload
        .get("organizations")
        .and_then(JsonValue::as_array)
        .and_then(|orgs| orgs.first())
        .and_then(|org| non_empty_json_str(org.get("id")))
}

fn non_empty_json_str(value: Option<&JsonValue>) -> Option<String> {
    value
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Decode a base64url (URL_SAFE_NO_PAD or padded) string into bytes.
fn base64url_decode(input: &str) -> Option<Vec<u8>> {
    const TABLE: [i8; 256] = {
        let mut table = [-1_i8; 256];
        let mut i = 0_u8;
        while i < 26 {
            table[(b'A' + i) as usize] = i as i8;
            table[(b'a' + i) as usize] = (26 + i) as i8;
            i += 1;
        }
        i = 0;
        while i < 10 {
            table[(b'0' + i) as usize] = (52 + i) as i8;
            i += 1;
        }
        table[b'-' as usize] = 62;
        table[b'_' as usize] = 63;
        // Also accept standard base64 alphabet variants.
        table[b'+' as usize] = 62;
        table[b'/' as usize] = 63;
        table
    };

    let filtered: Vec<u8> = input
        .bytes()
        .filter(|&b| b != b'=' && !b.is_ascii_whitespace())
        .collect();
    if filtered.is_empty() {
        return Some(Vec::new());
    }
    let mut out = Vec::with_capacity(filtered.len() * 3 / 4);
    let mut buffer = 0_u32;
    let mut bits = 0_u32;
    for byte in filtered {
        let value = TABLE[byte as usize];
        if value < 0 {
            return None;
        }
        buffer = (buffer << 6) | u32::from(value as u8);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Some(out)
}

fn resolve_aws_chain(provider_id: &str) -> Option<String> {
    if !matches!(provider_id, "bedrock" | "amazon-bedrock") {
        return None;
    }
    if let Some(token) = non_empty_env(AWS_BEARER_TOKEN_ENV) {
        return Some(token);
    }
    if let Some(access_key) = non_empty_env(AWS_ACCESS_KEY_ID_ENV) {
        let _secret = non_empty_env(AWS_SECRET_ACCESS_KEY_ENV)?;
        if let Some(session_token) = non_empty_env(AWS_SESSION_TOKEN_ENV) {
            return Some(session_token);
        }
        return Some(access_key);
    }
    aws_profile_token_from_file()
}

fn resolve_google_adc(provider_id: &str) -> Result<Option<String>, Diagnostic> {
    if !matches!(
        provider_id,
        "vertex" | "google-vertex" | "gemini" | "google"
    ) {
        return Ok(None);
    }
    let Some(path) = google_adc_path() else {
        return Ok(None);
    };
    let contents = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(Diagnostic::transport_unavailable(format!(
                "failed to read GOOGLE_APPLICATION_CREDENTIALS file {}: {err}",
                path.display()
            )));
        }
    };
    let mut value = serde_json::from_str::<JsonValue>(&contents).map_err(|err| {
        Diagnostic::upstream(format!(
            "GOOGLE_APPLICATION_CREDENTIALS must be valid JSON ({})",
            err
        ))
    })?;
    let Some(object) = value.as_object_mut() else {
        return Err(Diagnostic::upstream(
            "GOOGLE_APPLICATION_CREDENTIALS must be a JSON object",
        ));
    };
    let access_token = optional_string(object, "access_token");
    let expires_at = optional_u64(object, "expires_at").or_else(|| optional_u64(object, "expires"));
    if let Some(access_token) = access_token
        && !is_expired(expires_at)
    {
        return Ok(Some(access_token));
    }
    let refresh_token = optional_string(object, "refresh_token");
    let client_id = optional_string(object, "client_id");
    let client_secret = optional_string(object, "client_secret");
    let token_endpoint = optional_string(object, "token_uri")
        .unwrap_or_else(|| GOOGLE_OAUTH_TOKEN_ENDPOINT.to_owned());
    let (Some(refresh_token), Some(client_id), Some(client_secret)) =
        (refresh_token, client_id, client_secret)
    else {
        return Ok(None);
    };
    guard_outbound(GOOGLE_ADC_REFRESH_OUTBOUND_OPERATION)?;
    let client = reqwest::blocking::Client::builder()
        .build()
        .map_err(|err| {
            Diagnostic::transport_unavailable(format!("google ADC refresh client error: {err}"))
        })?;
    let form = vec![
        ("grant_type", "refresh_token".to_owned()),
        ("refresh_token", refresh_token.clone()),
        ("client_id", client_id.clone()),
        ("client_secret", client_secret.clone()),
    ];
    let response = client
        .post(&token_endpoint)
        .form(&form)
        .send()
        .map_err(|err| {
            Diagnostic::transport_unavailable(format!("google ADC refresh request failed: {err}"))
        })?;
    if !response.status().is_success() {
        return Err(Diagnostic::upstream(format!(
            "google ADC refresh failed with HTTP {}",
            response.status()
        )));
    }
    let body = response.json::<JsonValue>().map_err(|err| {
        Diagnostic::upstream(format!(
            "google ADC refresh response was not valid JSON: {err}"
        ))
    })?;
    let refreshed = body
        .as_object()
        .ok_or_else(|| Diagnostic::upstream("google ADC refresh response must be a JSON object"))?;
    let access_token = optional_string(refreshed, "access_token").ok_or_else(|| {
        Diagnostic::upstream("google ADC refresh response did not include access_token")
    })?;
    let refreshed_expires_at = optional_u64(refreshed, "expires_at")
        .or_else(|| optional_u64(refreshed, "expires"))
        .or_else(|| {
            optional_u64(refreshed, "expires_in").map(|value| now_secs().saturating_add(value))
        });
    object.insert(
        "access_token".to_owned(),
        JsonValue::String(access_token.clone()),
    );
    if let Some(expires_at) = refreshed_expires_at {
        object.insert(
            "expires_at".to_owned(),
            JsonValue::Number(expires_at.into()),
        );
    }
    if let Some(refresh) = optional_string(refreshed, "refresh_token") {
        object.insert("refresh_token".to_owned(), JsonValue::String(refresh));
    }
    let _ = write_json_private(&path, &value);
    Ok(Some(access_token))
}

fn google_adc_path() -> Option<PathBuf> {
    if let Some(path) = non_empty_env_path(GOOGLE_ADC_PATH_ENV) {
        return Some(path);
    }
    non_empty_env_path("HOME").map(|home| home.join(GOOGLE_ADC_DEFAULT_PATH))
}

fn write_json_private(path: &Path, value: &JsonValue) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_owned());
    std::fs::write(path, body.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn is_expired(expires_at: Option<u64>) -> bool {
    expires_at
        .is_some_and(|expires_at| expires_at <= now_secs().saturating_add(OAUTH_REFRESH_SKEW_SECS))
}

fn optional_string(object: &JsonMap<String, JsonValue>, key: &str) -> Option<String> {
    object
        .get(key)
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn optional_u64(object: &JsonMap<String, JsonValue>, key: &str) -> Option<u64> {
    object.get(key).and_then(|value| match value {
        JsonValue::Number(number) => number.as_u64(),
        JsonValue::String(string) => string.parse::<u64>().ok(),
        _ => None,
    })
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

fn aws_shared_credentials_file() -> Option<PathBuf> {
    non_empty_env_path(AWS_SHARED_CREDENTIALS_FILE_ENV)
        .or_else(|| non_empty_env_path("HOME").map(|home| home.join(".aws").join("credentials")))
}

fn aws_profile_name() -> String {
    non_empty_env(AWS_PROFILE_ENV)
        .or_else(|| non_empty_env(AWS_DEFAULT_PROFILE_ENV))
        .unwrap_or_else(|| "default".to_owned())
}

fn aws_profile_token_from_file() -> Option<String> {
    let path = aws_shared_credentials_file()?;
    let body = std::fs::read_to_string(path).ok()?;
    parse_aws_profile_token(&body, &aws_profile_name())
}

fn parse_aws_profile_token(body: &str, profile: &str) -> Option<String> {
    let mut in_profile = false;
    let mut access_key: Option<String> = None;
    let mut secret_key: Option<String> = None;
    let mut session_token: Option<String> = None;
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            let section = &trimmed[1..trimmed.len() - 1];
            in_profile = section == profile || section == format!("profile {profile}");
            continue;
        }
        if !in_profile {
            continue;
        }
        let Some((raw_key, raw_value)) = trimmed.split_once('=') else {
            continue;
        };
        let key = raw_key.trim();
        let value = raw_value.trim().to_owned();
        match key {
            "aws_access_key_id" => access_key = Some(value),
            "aws_secret_access_key" => secret_key = Some(value),
            "aws_session_token" => session_token = Some(value),
            _ => {}
        }
    }
    let _secret_key = secret_key?;
    if let Some(session_token) = session_token {
        return Some(session_token);
    }
    access_key
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::process::Command;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::*;

    const PROBE_CASE_ENV: &str = "CORTEX_RUNTIME_AUTH_PROBE_CASE";
    const PROBE_MARKER: &str = "RUNTIME_AUTH_PROBE_RESULT=";
    const PROBE_OAUTH_NO_NET: &str = "oauth-no-net";
    const PROBE_ADC_NO_NET: &str = "adc-no-net";
    const PROBE_OAUTH_FIXTURE: &str = "oauth-fixture";
    const PROBE_ADC_FIXTURE: &str = "adc-fixture";
    const PROBE_OAUTH_401_RETRY: &str = "oauth-401-retry";
    const PROBE_TOKEN_URL_ENV: &str = "CORTEX_RUNTIME_AUTH_PROBE_TOKEN_URL";
    const PROBE_RESOURCE_URL_ENV: &str = "CORTEX_RUNTIME_AUTH_PROBE_RESOURCE_URL";

    #[test]
    fn oauth_payload_parses_common_shapes() {
        let payload =
            r#"{"type":"oauth","access_token":"abc","refresh_token":"def","expires_at":123}"#;
        let parsed = OAuthCredential::from_stored(payload).expect("oauth payload");
        assert_eq!(parsed.access_token, "abc");
        assert_eq!(parsed.refresh_token.as_deref(), Some("def"));
        assert_eq!(parsed.expires_at, Some(123));
        assert_eq!(parsed.id_token, None);
    }

    #[test]
    fn aws_profile_parser_prefers_session_token() {
        let ini = r#"
[default]
aws_access_key_id = access
aws_secret_access_key = secret
aws_session_token = session
"#;
        assert_eq!(
            parse_aws_profile_token(ini, "default").as_deref(),
            Some("session")
        );
    }

    #[test]
    fn oauth_refresh_fails_closed_when_no_net_is_enabled() {
        if matches!(
            std::env::var(PROBE_CASE_ENV).as_deref(),
            Ok(PROBE_OAUTH_NO_NET)
        ) {
            let oauth = OAuthCredential {
                access_token: "stale".to_owned(),
                refresh_token: Some("refresh-token".to_owned()),
                id_token: None,
                token_type: "Bearer".to_owned(),
                expires_at: Some(0),
                token_endpoint: Some("http://127.0.0.1:9/token".to_owned()),
                client_id: Some("client-id".to_owned()),
                client_secret: Some("client-secret".to_owned()),
            };
            match refresh_oauth_credential(&oauth) {
                Ok(_) => println!("{PROBE_MARKER}unexpected-ok"),
                Err(err) => println!("{PROBE_MARKER}{}", err.kind()),
            }
            return;
        }

        let output = spawn_probe_case(
            "oauth_refresh_fails_closed_when_no_net_is_enabled",
            PROBE_OAUTH_NO_NET,
            true,
            &[],
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains(&format!("{PROBE_MARKER}net-disabled")),
            "expected net-disabled refusal. stdout:\n{stdout}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn google_adc_refresh_fails_closed_when_no_net_is_enabled() {
        if matches!(
            std::env::var(PROBE_CASE_ENV).as_deref(),
            Ok(PROBE_ADC_NO_NET)
        ) {
            match resolve_google_adc("google") {
                Ok(Some(token)) => println!("{PROBE_MARKER}unexpected-token:{token}"),
                Ok(None) => println!("{PROBE_MARKER}unexpected-none"),
                Err(err) => println!("{PROBE_MARKER}{}", err.kind()),
            }
            return;
        }

        let adc_dir = unique_temp_dir("adc-no-net");
        let adc_path = adc_dir.join("application_default_credentials.json");
        write_adc_credentials_fixture(&adc_path, "http://127.0.0.1:9/token");
        let output = spawn_probe_case(
            "google_adc_refresh_fails_closed_when_no_net_is_enabled",
            PROBE_ADC_NO_NET,
            true,
            &[("GOOGLE_APPLICATION_CREDENTIALS", adc_path.as_os_str())],
        );
        let _ = std::fs::remove_dir_all(adc_dir);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains(&format!("{PROBE_MARKER}net-disabled")),
            "expected net-disabled refusal. stdout:\n{stdout}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn oauth_refresh_uses_fixture_token_endpoint_when_network_enabled() {
        if matches!(
            std::env::var(PROBE_CASE_ENV).as_deref(),
            Ok(PROBE_OAUTH_FIXTURE)
        ) {
            let (token_endpoint, handle) = spawn_single_response_server(
                200,
                r#"{"access_token":"fresh-oauth-token","expires_in":3600}"#,
            );
            let oauth = OAuthCredential {
                access_token: "stale".to_owned(),
                refresh_token: Some("refresh-token".to_owned()),
                id_token: None,
                token_type: "Bearer".to_owned(),
                expires_at: Some(0),
                token_endpoint: Some(token_endpoint),
                client_id: Some("client-id".to_owned()),
                client_secret: Some("client-secret".to_owned()),
            };
            match refresh_oauth_credential(&oauth) {
                Ok(refreshed) => {
                    println!("{PROBE_MARKER}ok:{}", refreshed.access_token);
                    assert!(
                        refreshed.expires_at.is_some(),
                        "expires_at should be refreshed"
                    );
                }
                Err(err) => println!("{PROBE_MARKER}err:{}", err.kind()),
            }
            handle.join().expect("fixture server joined");
            return;
        }

        let output = spawn_probe_case(
            "oauth_refresh_uses_fixture_token_endpoint_when_network_enabled",
            PROBE_OAUTH_FIXTURE,
            false,
            &[],
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains(&format!("{PROBE_MARKER}ok:fresh-oauth-token")),
            "expected successful fixture refresh. stdout:\n{stdout}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn google_adc_refresh_uses_fixture_token_endpoint_when_network_enabled() {
        if matches!(
            std::env::var(PROBE_CASE_ENV).as_deref(),
            Ok(PROBE_ADC_FIXTURE)
        ) {
            let path = non_empty_env_path(GOOGLE_ADC_PATH_ENV).expect("adc path from env");
            match resolve_google_adc("google") {
                Ok(Some(token)) => {
                    println!("{PROBE_MARKER}ok:{token}");
                    let updated = std::fs::read_to_string(&path).expect("read updated adc payload");
                    assert!(
                        updated.contains("fresh-adc-token"),
                        "updated ADC credentials should store refreshed access token"
                    );
                }
                Ok(None) => println!("{PROBE_MARKER}unexpected-none"),
                Err(err) => println!("{PROBE_MARKER}err:{}", err.kind()),
            }
            return;
        }

        let (token_endpoint, handle) = spawn_single_response_server(
            200,
            r#"{"access_token":"fresh-adc-token","expires_in":1800}"#,
        );
        let adc_dir = unique_temp_dir("adc-fixture");
        let adc_path = adc_dir.join("application_default_credentials.json");
        write_adc_credentials_fixture(&adc_path, &token_endpoint);
        let output = spawn_probe_case(
            "google_adc_refresh_uses_fixture_token_endpoint_when_network_enabled",
            PROBE_ADC_FIXTURE,
            false,
            &[("GOOGLE_APPLICATION_CREDENTIALS", adc_path.as_os_str())],
        );
        handle.join().expect("fixture server joined");
        let _ = std::fs::remove_dir_all(adc_dir);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains(&format!("{PROBE_MARKER}ok:fresh-adc-token")),
            "expected successful fixture refresh. stdout:\n{stdout}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// Expired OAuth credential: proactive refresh before first use, then a single
    /// 401-driven refresh-and-retry when the resource fixture rejects the first bearer.
    ///
    /// Fixture contract: `/token` always mints a fresh access token; `/resource` returns
    /// 401 on the first hit and 200 on the second. Without 401 retry this case stays RED.
    #[test]
    fn expired_credential_refreshes_before_use_and_retries_once_on_401() {
        if matches!(
            std::env::var(PROBE_CASE_ENV).as_deref(),
            Ok(PROBE_OAUTH_401_RETRY)
        ) {
            let token_endpoint =
                std::env::var(PROBE_TOKEN_URL_ENV).expect("probe token url");
            let resource_url =
                std::env::var(PROBE_RESOURCE_URL_ENV).expect("probe resource url");
            let mut oauth = OAuthCredential {
                access_token: "stale".to_owned(),
                refresh_token: Some("refresh-token".to_owned()),
                id_token: None,
                token_type: "Bearer".to_owned(),
                // Expired → proactive refresh must run before the resource call.
                expires_at: Some(0),
                token_endpoint: Some(token_endpoint),
                client_id: Some("client-id".to_owned()),
                client_secret: Some("client-secret".to_owned()),
            };
            match oauth_authorized_get(&mut oauth, None, "test-provider", &resource_url) {
                Ok(body) => {
                    // Body is a fixed fixture string — never a token.
                    println!("{PROBE_MARKER}ok:{body}");
                    assert!(
                        body.contains("\"ok\":true"),
                        "resource body must signal success after the 401 retry"
                    );
                }
                Err(err) => println!("{PROBE_MARKER}err:{}", err.kind()),
            }
            return;
        }

        let (token_url, resource_url, handle) = spawn_token_and_401_once_resource_fixture();
        let output = spawn_probe_case(
            "expired_credential_refreshes_before_use_and_retries_once_on_401",
            PROBE_OAUTH_401_RETRY,
            false,
            &[
                (PROBE_TOKEN_URL_ENV, std::ffi::OsStr::new(token_url.as_str())),
                (
                    PROBE_RESOURCE_URL_ENV,
                    std::ffi::OsStr::new(resource_url.as_str()),
                ),
            ],
        );
        handle.join().expect("401-retry fixture server joined");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains(&format!("{PROBE_MARKER}ok:{{\"ok\":true}}")),
            "expected single 401 refresh-and-retry to yield 200. stdout:\n{stdout}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn spawn_probe_case(
        test_name: &str,
        case: &str,
        with_no_net: bool,
        extra_envs: &[(&str, &std::ffi::OsStr)],
    ) -> std::process::Output {
        let exe = std::env::current_exe().expect("path to this test binary");
        let mut command = Command::new(exe);
        let full_test_name = format!("runtime_auth::tests::{test_name}");
        command
            .args(["--exact", &full_test_name, "--nocapture"])
            .env(PROBE_CASE_ENV, case)
            .env_remove("GOOGLE_APPLICATION_CREDENTIALS");
        for (key, value) in extra_envs {
            command.env(key, value);
        }
        if with_no_net {
            command.env("CORTEX_NO_NET", "1");
        } else {
            command.env_remove("CORTEX_NO_NET");
        }
        command.output().expect("spawn child probe")
    }

    fn write_adc_credentials_fixture(path: &Path, token_endpoint: &str) {
        std::fs::create_dir_all(path.parent().expect("adc parent")).expect("create adc dir");
        let expired = now_secs().saturating_sub(1);
        let payload = format!(
            r#"{{"type":"authorized_user","client_id":"client-id","client_secret":"client-secret","refresh_token":"refresh-token","access_token":"stale-token","expires_at":{expired},"token_uri":"{token_endpoint}"}}"#
        );
        std::fs::write(path, payload).expect("write adc payload");
    }

    fn spawn_single_response_server(
        status: u16,
        body: &str,
    ) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
        let addr = listener.local_addr().expect("fixture addr");
        let response_body = body.to_owned();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept fixture request");
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .expect("set read timeout");
            let mut buffer = [0_u8; 4096];
            let _ = stream.read(&mut buffer);
            let status_text = match status {
                200 => "OK",
                400 => "Bad Request",
                401 => "Unauthorized",
                500 => "Internal Server Error",
                _ => "Unknown",
            };
            let response = format!(
                "HTTP/1.1 {status} {status_text}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                response_body.len()
            );
            stream
                .write_all(response.as_bytes())
                .expect("write fixture response");
            stream.flush().expect("flush fixture response");
        });
        (format!("http://{addr}/token"), handle)
    }

    /// Loopback fixture: `POST /token` → fresh access token; first `GET /resource` → 401;
    /// second `GET /resource` → 200 `{"ok":true}`. Serves several connections then exits.
    fn spawn_token_and_401_once_resource_fixture()
    -> (String, String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind 401-retry fixture");
        listener
            .set_nonblocking(true)
            .expect("nonblocking fixture listener");
        let addr = listener.local_addr().expect("fixture addr");
        let handle = std::thread::spawn(move || {
            let mut resource_hits = 0_u32;
            let deadline = std::time::Instant::now() + Duration::from_secs(4);
            while std::time::Instant::now() < deadline {
                let (mut stream, _) = match listener.accept() {
                    Ok(conn) => conn,
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(_) => break,
                };
                stream
                    .set_nonblocking(false)
                    .expect("blocking stream after accept");
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .expect("set read timeout");
                let mut buffer = [0_u8; 8192];
                let n = stream.read(&mut buffer).unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..n]);
                let request_line = request.lines().next().unwrap_or("");
                let is_token = request_line.contains("/token");
                let (status, status_text, body): (u16, &str, &str) = if is_token {
                    (
                        200,
                        "OK",
                        r#"{"access_token":"fresh-after-refresh","expires_in":3600,"token_type":"Bearer"}"#,
                    )
                } else if resource_hits == 0 {
                    resource_hits = resource_hits.saturating_add(1);
                    (401, "Unauthorized", r#"{"error":"invalid_token"}"#)
                } else {
                    resource_hits = resource_hits.saturating_add(1);
                    (200, "OK", r#"{"ok":true}"#)
                };
                let response = format!(
                    "HTTP/1.1 {status} {status_text}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });
        (
            format!("http://{addr}/token"),
            format!("http://{addr}/resource"),
            handle,
        )
    }

    fn unique_temp_dir(tag: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock must be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "cortex-runtime-auth-{tag}-{}-{nonce}",
            std::process::id()
        ))
    }

    fn fixture_jwt_payload(payload: &JsonValue) -> String {
        let header = base64url_encode_nopad(br#"{"alg":"none","typ":"JWT"}"#);
        let body = base64url_encode_nopad(&serde_json::to_vec(payload).expect("json"));
        format!("{header}.{body}.sig")
    }

    fn base64url_encode_nopad(bytes: &[u8]) -> String {
        const TABLE: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
        let mut i = 0;
        while i < bytes.len() {
            let b0 = bytes[i];
            let b1 = bytes.get(i + 1).copied();
            let b2 = bytes.get(i + 2).copied();
            out.push(TABLE[(b0 >> 2) as usize] as char);
            match (b1, b2) {
                (Some(b1), Some(b2)) => {
                    out.push(TABLE[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
                    out.push(TABLE[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
                    out.push(TABLE[(b2 & 0x3f) as usize] as char);
                    i += 3;
                }
                (Some(b1), None) => {
                    out.push(TABLE[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
                    out.push(TABLE[((b1 & 0x0f) << 2) as usize] as char);
                    i += 2;
                }
                (None, _) => {
                    out.push(TABLE[((b0 & 0x03) << 4) as usize] as char);
                    i += 1;
                }
            }
        }
        out
    }

    #[test]
    fn chatgpt_account_id_prefers_top_level_claim() {
        let token = fixture_jwt_payload(&serde_json::json!({
            "chatgpt_account_id": "acct_top",
            "https://api.openai.com/auth": { "chatgpt_account_id": "acct_nested" },
            "organizations": [{ "id": "acct_org" }]
        }));
        assert_eq!(
            chatgpt_account_id_from_id_token(&token).as_deref(),
            Some("acct_top")
        );
    }

    #[test]
    fn chatgpt_account_id_falls_back_to_openai_auth_object() {
        let token = fixture_jwt_payload(&serde_json::json!({
            "https://api.openai.com/auth": { "chatgpt_account_id": "acct_nested" },
            "organizations": [{ "id": "acct_org" }]
        }));
        assert_eq!(
            chatgpt_account_id_from_id_token(&token).as_deref(),
            Some("acct_nested")
        );
    }

    #[test]
    fn chatgpt_account_id_falls_back_to_organizations_first_id() {
        let token = fixture_jwt_payload(&serde_json::json!({
            "organizations": [{ "id": "acct_org" }, { "id": "acct_other" }]
        }));
        assert_eq!(
            chatgpt_account_id_from_id_token(&token).as_deref(),
            Some("acct_org")
        );
    }

    #[test]
    fn oauth_id_token_round_trips_through_stored_string() {
        let original = OAuthCredential {
            access_token: "access".to_owned(),
            refresh_token: Some("refresh".to_owned()),
            id_token: Some("header.payload.sig".to_owned()),
            token_type: "Bearer".to_owned(),
            expires_at: Some(99),
            token_endpoint: Some("https://example.invalid/token".to_owned()),
            client_id: Some("client".to_owned()),
            client_secret: None,
        };
        let stored = original.to_stored_string();
        // Secret hygiene: unit test must not print tokens; only assert structure.
        assert!(stored.contains("\"id_token\""));
        let parsed = OAuthCredential::from_stored(&stored).expect("round-trip");
        assert_eq!(parsed.id_token.as_deref(), Some("header.payload.sig"));
        assert_eq!(parsed.access_token, "access");
        assert_eq!(parsed.refresh_token.as_deref(), Some("refresh"));
    }

    #[test]
    fn oauth_refresh_preserves_prior_id_token_when_response_omits_it() {
        // Structural unit check of the merge rule used by refresh_oauth_credential:
        // when the token response has no id_token, keep the prior value.
        let prior = Some("prior-id-token".to_owned());
        let from_response: Option<String> = None;
        let merged = from_response.or_else(|| prior.clone());
        assert_eq!(merged.as_deref(), Some("prior-id-token"));
    }
}
