use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use errors::Diagnostic;
use serde_json::{Map as JsonMap, Value as JsonValue};

use crate::credentials::{
    CredentialInputs, non_empty_env, non_empty_env_path, resolve_credential, store_credential,
};

const OAUTH_REFRESH_SKEW_SECS: u64 = 30;
const GOOGLE_ADC_PATH_ENV: &str = "GOOGLE_APPLICATION_CREDENTIALS";
const GOOGLE_ADC_DEFAULT_PATH: &str = ".config/gcloud/application_default_credentials.json";
const GOOGLE_OAUTH_TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
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

fn persist_refreshed_oauth(path: Option<&Path>, provider_id: &str, oauth: &OAuthCredential) {
    let Some(path) = path else {
        return;
    };
    let _ = store_credential(path, provider_id, &oauth.to_stored_string());
}

fn refresh_oauth_credential(oauth: &OAuthCredential) -> Result<OAuthCredential, Diagnostic> {
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
        token_type: optional_string(object, "token_type")
            .or_else(|| Some(oauth.token_type.clone()))
            .unwrap_or_else(|| "Bearer".to_owned()),
        expires_at,
        token_endpoint: oauth.token_endpoint.clone(),
        client_id: oauth.client_id.clone(),
        client_secret: oauth.client_secret.clone(),
    })
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
    use super::*;

    #[test]
    fn oauth_payload_parses_common_shapes() {
        let payload =
            r#"{"type":"oauth","access_token":"abc","refresh_token":"def","expires_at":123}"#;
        let parsed = OAuthCredential::from_stored(payload).expect("oauth payload");
        assert_eq!(parsed.access_token, "abc");
        assert_eq!(parsed.refresh_token.as_deref(), Some("def"));
        assert_eq!(parsed.expires_at, Some(123));
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
}
