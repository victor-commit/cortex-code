//! The `websearch` tool: config-enabled, real web search provider requests.
//!
//! The tool is intentionally not registered by default. A surface must explicitly provide a
//! [`WebSearchConfig`] before `websearch` appears in the model's per-turn tool set. When it is
//! configured, every query goes to the selected live provider after the shared `CORTEX_NO_NET`
//! guard has allowed outbound network access. Provider errors are bounded and surfaced as tool
//! errors, never fabricated into search results.

use std::collections::HashSet;
use std::time::Duration;

use futures_util::StreamExt;
use redaction::guard_outbound;
use reqwest::StatusCode;
use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderValue, USER_AGENT};
use serde_json::{Value, json};
use url::Url;

use crate::output::{OutputLimits, bound_output};
use crate::tool::{Tool, ToolContext, ToolOutput};

const DESCRIPTION: &str = "Search the public web through a configured real search provider and \
return bounded result entries with titles, URLs, and snippets. Use this when the user asks you to \
search, look up current information, find sources, or ground an answer in web results. The tool is \
available only when a search provider is explicitly enabled in config; never invent search results.";

const OUTBOUND_OPERATION: &str = "websearch provider request";
const DEFAULT_TIMEOUT_MS: u64 = 8_000;
const MAX_TIMEOUT_MS: u64 = 30_000;
const MIN_TIMEOUT_MS: u64 = 50;
const DEFAULT_MAX_RESULTS: usize = 5;
const MAX_RESULTS: usize = 10;
const MAX_RESPONSE_BYTES: usize = 256 * 1024;
const BRAVE_DEFAULT_ENDPOINT: &str = "https://api.search.brave.com/res/v1/web/search";
const DUCKDUCKGO_DEFAULT_ENDPOINT: &str = "https://api.duckduckgo.com/";

/// Supported real search providers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WebSearchProvider {
    /// Brave Search API (`api.search.brave.com`), requires an API key.
    Brave,
    /// DuckDuckGo Instant Answer API, no API key.
    DuckDuckGo,
}

impl WebSearchProvider {
    /// Parse a provider name from config.
    ///
    /// # Errors
    /// Returns a provider-naming message when the name is unsupported.
    pub fn from_name(name: &str) -> Result<Self, String> {
        match normalize_provider(name).as_str() {
            "brave" | "bravesearch" | "brave_search" => Ok(Self::Brave),
            "duckduckgo" | "duck_duck_go" | "duckduckgo_instant" | "ddg" => Ok(Self::DuckDuckGo),
            other => Err(format!(
                "unsupported websearch provider `{other}` (expected `duckduckgo` or `brave`)"
            )),
        }
    }

    /// The canonical provider label used in tool output.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Brave => "brave",
            Self::DuckDuckGo => "duckduckgo",
        }
    }

    fn default_endpoint(self) -> &'static str {
        match self {
            Self::Brave => BRAVE_DEFAULT_ENDPOINT,
            Self::DuckDuckGo => DUCKDUCKGO_DEFAULT_ENDPOINT,
        }
    }
}

fn normalize_provider(name: &str) -> String {
    name.trim().to_ascii_lowercase().replace(['-', ' '], "_")
}

/// Configuration required to materialize a real `websearch` tool.
#[derive(Clone)]
pub struct WebSearchConfig {
    provider: WebSearchProvider,
    api_key: Option<String>,
    endpoint: Option<String>,
    max_results: usize,
    timeout: Duration,
}

impl WebSearchConfig {
    /// Build config for a provider with default endpoint, result count, and timeout.
    #[must_use]
    pub fn new(provider: WebSearchProvider) -> Self {
        Self {
            provider,
            api_key: None,
            endpoint: None,
            max_results: DEFAULT_MAX_RESULTS,
            timeout: Duration::from_millis(DEFAULT_TIMEOUT_MS),
        }
    }

    /// Convenience constructor for the keyless DuckDuckGo provider.
    #[must_use]
    pub fn duckduckgo() -> Self {
        Self::new(WebSearchProvider::DuckDuckGo)
    }

    /// Attach an optional provider API key. Empty strings are treated as absent.
    #[must_use]
    pub fn with_api_key(mut self, api_key: Option<String>) -> Self {
        self.api_key = api_key.and_then(|key| {
            let trimmed = key.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        });
        self
    }

    /// Override the provider endpoint.
    ///
    /// # Errors
    /// Returns an actionable message when the endpoint is not an absolute HTTP(S) URL.
    pub fn with_endpoint(mut self, endpoint: Option<String>) -> Result<Self, String> {
        if let Some(endpoint) = endpoint.and_then(|value| {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        }) {
            validate_http_endpoint(&endpoint)?;
            self.endpoint = Some(endpoint);
        }
        Ok(self)
    }

    /// Override the per-query maximum number of rendered results.
    ///
    /// # Errors
    /// Returns a range message when the count is zero.
    pub fn with_max_results(mut self, max_results: Option<usize>) -> Result<Self, String> {
        if let Some(max_results) = max_results {
            if max_results == 0 {
                return Err("websearch max_results must be at least 1".to_owned());
            }
            self.max_results = max_results.min(MAX_RESULTS);
        }
        Ok(self)
    }

    /// Override the provider request timeout.
    #[must_use]
    pub fn with_timeout(mut self, timeout_ms: Option<u64>) -> Self {
        if let Some(timeout_ms) = timeout_ms {
            self.timeout = Duration::from_millis(timeout_ms.clamp(MIN_TIMEOUT_MS, MAX_TIMEOUT_MS));
        }
        self
    }

    /// The configured provider.
    #[must_use]
    pub fn provider(&self) -> WebSearchProvider {
        self.provider
    }

    /// Whether the config carries a non-empty API key.
    #[must_use]
    pub fn has_api_key(&self) -> bool {
        self.api_key.is_some()
    }

    /// The configured endpoint, or the provider default.
    #[must_use]
    pub fn endpoint(&self) -> &str {
        self.endpoint
            .as_deref()
            .unwrap_or_else(|| self.provider.default_endpoint())
    }

    /// The configured result cap.
    #[must_use]
    pub fn max_results(&self) -> usize {
        self.max_results
    }
}

impl std::fmt::Debug for WebSearchConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WebSearchConfig")
            .field("provider", &self.provider)
            .field(
                "api_key",
                &self.api_key.as_ref().map(|_| redaction::PLACEHOLDER),
            )
            .field("endpoint", &self.endpoint)
            .field("max_results", &self.max_results)
            .field("timeout", &self.timeout)
            .finish()
    }
}

fn validate_http_endpoint(endpoint: &str) -> Result<(), String> {
    let url = Url::parse(endpoint)
        .map_err(|err| format!("websearch endpoint `{endpoint}` is invalid: {err}"))?;
    match url.scheme() {
        "http" | "https" => Ok(()),
        scheme => Err(format!(
            "websearch endpoint `{endpoint}` uses unsupported scheme `{scheme}` (expected http or https)"
        )),
    }
}

/// Search the web and return bounded result entries.
pub struct WebSearchTool {
    config: WebSearchConfig,
}

impl WebSearchTool {
    /// Build a configured `websearch` tool.
    #[must_use]
    pub fn new(config: WebSearchConfig) -> Self {
        Self { config }
    }
}

impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "websearch"
    }

    fn description(&self) -> &str {
        DESCRIPTION
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The web search query to send to the configured provider."
                },
                "count": {
                    "type": "integer",
                    "description": "Optional number of results to return, 1-10 and capped by config. Defaults to the configured max_results."
                },
                "timeout_ms": {
                    "type": "integer",
                    "description": "Optional total timeout in milliseconds, clamped between 50 and 30000. Defaults to the configured timeout."
                }
            },
            "required": ["query"],
            "additionalProperties": false
        })
    }

    fn execute<'a>(
        &'a self,
        arguments: Value,
        ctx: &'a ToolContext,
    ) -> futures_util::future::BoxFuture<'a, ToolOutput> {
        Box::pin(async move { run(arguments, ctx, &self.config).await })
    }
}

async fn run(arguments: Value, ctx: &ToolContext, config: &WebSearchConfig) -> ToolOutput {
    let Some(raw_query) = string_arg(&arguments, "query").or_else(|| string_arg(&arguments, "q"))
    else {
        return ToolOutput::error("websearch: the 'query' argument is required");
    };
    let query = raw_query.trim();
    if query.is_empty() {
        return ToolOutput::error("websearch: the 'query' argument must not be empty");
    }

    if let Err(message) = ctx.ensure_permitted("websearch", query) {
        return ToolOutput::error(redaction::redact(&message));
    }

    let options = match options_from(&arguments, config) {
        Ok(options) => options,
        Err(message) => return ToolOutput::error(message),
    };

    match search(
        config,
        query,
        options,
        ctx.audit_sink().map(|sink| sink.as_ref()),
    )
    .await
    {
        Ok(results) => ToolOutput::text(results),
        Err(message) => ToolOutput::error(message),
    }
}

#[derive(Clone, Copy)]
struct SearchOptions {
    timeout: Duration,
    count: usize,
}

fn options_from(arguments: &Value, config: &WebSearchConfig) -> Result<SearchOptions, String> {
    let mut timeout = config.timeout;
    if let Some(value) = arguments.get("timeout_ms") {
        let Some(ms) = value.as_u64() else {
            return Err("websearch: 'timeout_ms' must be an integer".to_owned());
        };
        timeout = Duration::from_millis(ms.clamp(MIN_TIMEOUT_MS, MAX_TIMEOUT_MS));
    }

    let mut count = config.max_results;
    if let Some(value) = arguments.get("count").or_else(|| arguments.get("limit")) {
        let Some(raw_count) = value.as_u64() else {
            return Err("websearch: 'count' must be an integer".to_owned());
        };
        if raw_count == 0 {
            return Err("websearch: 'count' must be at least 1".to_owned());
        }
        count = (raw_count as usize)
            .min(MAX_RESULTS)
            .min(config.max_results);
    }

    Ok(SearchOptions { timeout, count })
}

async fn search(
    config: &WebSearchConfig,
    query: &str,
    options: SearchOptions,
    audit: Option<&audit::AuditSink>,
) -> Result<String, String> {
    if let Err(diagnostic) = guard_outbound(OUTBOUND_OPERATION) {
        record_network(audit, config.endpoint(), None, "blocked");
        return Err(format!("websearch: {diagnostic}"));
    }

    let client = reqwest::Client::builder()
        .use_rustls_tls()
        .no_proxy()
        .connect_timeout(options.timeout.min(Duration::from_secs(3)))
        .timeout(options.timeout)
        .user_agent(concat!(
            "cortex-code/",
            env!("CARGO_PKG_VERSION"),
            " websearch"
        ))
        .build()
        .map_err(|err| redaction::redact(&format!("websearch: transport setup failed: {err}")))?;

    let results = match config.provider {
        WebSearchProvider::DuckDuckGo => {
            duckduckgo_search(&client, config, query, options, audit).await
        }
        WebSearchProvider::Brave => brave_search(&client, config, query, options, audit).await,
    }?;

    if results.is_empty() {
        return Err(redaction::redact(&format!(
            "websearch: provider {} returned no results for `{query}`",
            config.provider.as_str()
        )));
    }

    Ok(render_results(config.provider, query, &results))
}

async fn duckduckgo_search(
    client: &reqwest::Client,
    config: &WebSearchConfig,
    query: &str,
    options: SearchOptions,
    audit: Option<&audit::AuditSink>,
) -> Result<Vec<SearchResult>, String> {
    let mut url = endpoint_url(config)?;
    url.query_pairs_mut()
        .append_pair("q", query)
        .append_pair("format", "json")
        .append_pair("no_html", "1")
        .append_pair("skip_disambig", "1");
    let endpoint = url.clone();

    let json = send_json(
        client
            .get(url)
            .header(ACCEPT, "application/json,text/javascript,*/*;q=0.1")
            .header(
                USER_AGENT,
                concat!("cortex-code/", env!("CARGO_PKG_VERSION"), " websearch"),
            ),
        config.provider,
        options.timeout,
        audit,
        &endpoint,
    )
    .await?;

    Ok(parse_duckduckgo_results(&json, options.count))
}

async fn brave_search(
    client: &reqwest::Client,
    config: &WebSearchConfig,
    query: &str,
    options: SearchOptions,
    audit: Option<&audit::AuditSink>,
) -> Result<Vec<SearchResult>, String> {
    let Some(api_key) = config.api_key.as_deref() else {
        return Err(
            "websearch: provider brave requires an API key; set [websearch].api_key, \
             CORTEX_WEBSEARCH_API_KEY, or [provider.brave].api_key"
                .to_owned(),
        );
    };

    let mut url = endpoint_url(config)?;
    url.query_pairs_mut()
        .append_pair("q", query)
        .append_pair("count", &options.count.to_string())
        .append_pair("text_decorations", "false");
    let endpoint = url.clone();

    let mut headers = HeaderMap::new();
    headers.insert(
        "X-Subscription-Token",
        HeaderValue::from_str(api_key).map_err(|err| {
            redaction::redact(&format!(
                "websearch: invalid brave API key header value: {err}"
            ))
        })?,
    );

    let json = send_json(
        client
            .get(url)
            .headers(headers)
            .header(ACCEPT, "application/json")
            .header(
                USER_AGENT,
                concat!("cortex-code/", env!("CARGO_PKG_VERSION"), " websearch"),
            ),
        config.provider,
        options.timeout,
        audit,
        &endpoint,
    )
    .await?;

    Ok(parse_brave_results(&json, options.count))
}

fn endpoint_url(config: &WebSearchConfig) -> Result<Url, String> {
    Url::parse(config.endpoint()).map_err(|err| {
        redaction::redact(&format!(
            "websearch: invalid provider endpoint `{}`: {err}",
            config.endpoint()
        ))
    })
}

async fn send_json(
    request: reqwest::RequestBuilder,
    provider: WebSearchProvider,
    timeout: Duration,
    audit: Option<&audit::AuditSink>,
    endpoint: &Url,
) -> Result<Value, String> {
    let response = match tokio::time::timeout(timeout, request.send()).await {
        Err(_) => {
            record_network(audit, endpoint.as_str(), None, "timeout");
            return Err(format!(
                "websearch: timeout contacting provider {} after {} ms",
                provider.as_str(),
                timeout.as_millis()
            ));
        }
        Ok(Err(err)) if err.is_timeout() => {
            record_network(audit, endpoint.as_str(), None, "timeout");
            return Err(redaction::redact(&format!(
                "websearch: timeout contacting provider {}: {err}",
                provider.as_str()
            )));
        }
        Ok(Err(err)) => {
            record_network(audit, endpoint.as_str(), None, "error");
            return Err(redaction::redact(&format!(
                "websearch: transport error contacting provider {}: {err}",
                provider.as_str()
            )));
        }
        Ok(Ok(response)) => response,
    };

    let final_url = response.url().clone();
    let status = response.status();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("unknown")
        .to_owned();
    let body = match collect_body(response, timeout).await {
        Ok(body) => body,
        Err(message) => {
            record_network(audit, final_url.as_str(), Some(status), "error");
            return Err(message);
        }
    };
    let text = String::from_utf8_lossy(&body.bytes).into_owned();

    if !status.is_success() {
        record_network(audit, final_url.as_str(), Some(status), "error");
        return Err(render_status_error(
            provider,
            &final_url,
            status,
            &content_type,
            &text,
            body.truncated,
        ));
    }

    match serde_json::from_str(&text) {
        Ok(value) => {
            record_network(audit, final_url.as_str(), Some(status), "success");
            Ok(value)
        }
        Err(err) => {
            record_network(audit, final_url.as_str(), Some(status), "error");
            Err(redaction::redact(&format!(
                "websearch: provider {} returned invalid JSON from {final_url}: {err}",
                provider.as_str()
            )))
        }
    }
}

fn record_network(
    audit: Option<&audit::AuditSink>,
    endpoint: &str,
    status: Option<StatusCode>,
    outcome: &str,
) {
    if let Some(audit) = audit {
        let host = audit::host_from_endpoint(endpoint).unwrap_or_default();
        audit.record_network_call(
            "websearch",
            None,
            None,
            "GET",
            &host,
            endpoint,
            status.map(|status| status.as_u16()),
            outcome,
        );
    }
}

struct ProviderBody {
    bytes: Vec<u8>,
    truncated: bool,
}

async fn collect_body(
    response: reqwest::Response,
    timeout: Duration,
) -> Result<ProviderBody, String> {
    let mut bytes = Vec::new();
    let mut truncated = false;
    let mut stream = response.bytes_stream();

    while let Some(next) = match tokio::time::timeout(timeout, stream.next()).await {
        Ok(next) => next,
        Err(_) => {
            return Err(format!(
                "websearch: timeout reading provider response after {} ms",
                timeout.as_millis()
            ));
        }
    } {
        let chunk = next.map_err(|err| {
            if err.is_timeout() {
                redaction::redact(&format!(
                    "websearch: timeout reading provider response: {err}"
                ))
            } else {
                redaction::redact(&format!(
                    "websearch: transport error reading provider response: {err}"
                ))
            }
        })?;
        if bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
            let remaining = MAX_RESPONSE_BYTES.saturating_sub(bytes.len());
            bytes.extend_from_slice(&chunk[..remaining]);
            truncated = true;
            break;
        }
        bytes.extend_from_slice(&chunk);
    }

    Ok(ProviderBody { bytes, truncated })
}

fn render_status_error(
    provider: WebSearchProvider,
    final_url: &Url,
    status: StatusCode,
    content_type: &str,
    body: &str,
    truncated: bool,
) -> String {
    let auth_hint = match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            " authentication failed; check the configured websearch API key"
        }
        _ => "",
    };
    let snippet = body_snippet(body, truncated);
    redaction::redact(&format!(
        "websearch: provider {} returned HTTP {status}{auth_hint} for {final_url} \
         (content-type: {content_type}){snippet}",
        provider.as_str(),
    ))
}

fn body_snippet(body: &str, truncated: bool) -> String {
    let mut snippet = body.trim().replace(['\r', '\n', '\t'], " ");
    if snippet.len() > 400 {
        let mut end = 400;
        while end > 0 && !snippet.is_char_boundary(end) {
            end -= 1;
        }
        snippet.truncate(end);
        snippet.push('…');
    }
    if snippet.is_empty() {
        String::new()
    } else if truncated {
        format!("; body: {snippet} [truncated]")
    } else {
        format!("; body: {snippet}")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SearchResult {
    title: String,
    url: String,
    snippet: Option<String>,
}

fn parse_brave_results(json: &Value, count: usize) -> Vec<SearchResult> {
    let Some(items) = json
        .pointer("/web/results")
        .and_then(Value::as_array)
        .or_else(|| json.get("results").and_then(Value::as_array))
    else {
        return Vec::new();
    };

    let mut seen = HashSet::new();
    let mut results = Vec::new();
    for item in items {
        let Some(title) = json_string(item, "title") else {
            continue;
        };
        let Some(url) = json_string(item, "url") else {
            continue;
        };
        push_result(
            &mut results,
            &mut seen,
            title,
            url,
            json_string(item, "description").or_else(|| json_string(item, "snippet")),
            count,
        );
        if results.len() >= count {
            break;
        }
    }
    results
}

fn parse_duckduckgo_results(json: &Value, count: usize) -> Vec<SearchResult> {
    let mut seen = HashSet::new();
    let mut results = Vec::new();

    if let Some(url) = json_string(json, "AbstractURL") {
        let title = json_string(json, "Heading")
            .or_else(|| json_string(json, "AbstractSource"))
            .unwrap_or_else(|| "DuckDuckGo result".to_owned());
        let snippet = json_string(json, "Abstract").or_else(|| json_string(json, "Definition"));
        push_result(&mut results, &mut seen, title, url, snippet, count);
    }

    if let Some(items) = json.get("Results").and_then(Value::as_array) {
        collect_duckduckgo_topics(items, &mut results, &mut seen, count);
    }
    if let Some(items) = json.get("RelatedTopics").and_then(Value::as_array) {
        collect_duckduckgo_topics(items, &mut results, &mut seen, count);
    }

    results
}

fn collect_duckduckgo_topics(
    items: &[Value],
    results: &mut Vec<SearchResult>,
    seen: &mut HashSet<String>,
    count: usize,
) {
    for item in items {
        if results.len() >= count {
            return;
        }
        if let Some(children) = item.get("Topics").and_then(Value::as_array) {
            collect_duckduckgo_topics(children, results, seen, count);
            continue;
        }
        let Some(url) = json_string(item, "FirstURL") else {
            continue;
        };
        let Some(text) = json_string(item, "Text") else {
            continue;
        };
        let (title, snippet) = split_title_snippet(&text);
        push_result(results, seen, title, url, snippet, count);
    }
}

fn split_title_snippet(text: &str) -> (String, Option<String>) {
    let text = clean_text(text);
    if let Some((title, snippet)) = text.split_once(" - ") {
        (
            title.trim().to_owned(),
            non_empty(snippet).map(str::to_owned),
        )
    } else {
        (text, None)
    }
}

fn push_result(
    results: &mut Vec<SearchResult>,
    seen: &mut HashSet<String>,
    title: String,
    url: String,
    snippet: Option<String>,
    count: usize,
) {
    if results.len() >= count {
        return;
    }
    let title = clean_text(&title);
    let url = url.trim().to_owned();
    if title.is_empty() || !is_http_url(&url) || !seen.insert(url.clone()) {
        return;
    }
    let snippet = snippet
        .map(|value| clean_text(&value))
        .filter(|value| !value.is_empty());
    results.push(SearchResult {
        title,
        url,
        snippet,
    });
}

fn is_http_url(raw: &str) -> bool {
    Url::parse(raw)
        .map(|url| matches!(url.scheme(), "http" | "https"))
        .unwrap_or(false)
}

fn render_results(provider: WebSearchProvider, query: &str, results: &[SearchResult]) -> String {
    let mut rendered = format!(
        "websearch results for \"{}\" via {} ({} result{}):",
        clean_text(query),
        provider.as_str(),
        results.len(),
        if results.len() == 1 { "" } else { "s" }
    );
    for (index, result) in results.iter().enumerate() {
        rendered.push_str(&format!(
            "\n{}. {}\n   URL: {}",
            index + 1,
            result.title,
            result.url
        ));
        if let Some(snippet) = &result.snippet {
            rendered.push_str(&format!("\n   Snippet: {snippet}"));
        }
    }
    bound_output(&rendered, OutputLimits::default(), "websearch").text
}

fn json_string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .and_then(non_empty)
        .map(clean_text)
}

fn string_arg<'a>(arguments: &'a Value, key: &str) -> Option<&'a str> {
    arguments.get(key).and_then(Value::as_str)
}

fn non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn clean_text(value: &str) -> String {
    unescape_basic_entities(&strip_html_tags(value))
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn strip_html_tags(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut in_tag = false;
    for ch in value.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => output.push(ch),
            _ => {}
        }
    }
    output
}

fn unescape_basic_entities(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_log(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "cortex-websearch-audit-{tag}-{}-{}.jsonl",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn provider_names_are_normalized() {
        assert_eq!(
            WebSearchProvider::from_name("Duck Duck Go").expect("provider"),
            WebSearchProvider::DuckDuckGo
        );
        assert_eq!(
            WebSearchProvider::from_name("brave-search").expect("provider"),
            WebSearchProvider::Brave
        );
        assert!(WebSearchProvider::from_name("nope").is_err());
    }

    #[test]
    fn parses_brave_results() {
        let json = json!({
            "web": {
                "results": [
                    {
                        "title": "Rust Programming Language",
                        "url": "https://www.rust-lang.org/",
                        "description": "A language empowering everyone to build reliable software."
                    },
                    {
                        "title": "Duplicate",
                        "url": "https://www.rust-lang.org/",
                        "description": "duplicate"
                    },
                    {
                        "title": "Ignored FTP",
                        "url": "ftp://example.com/file",
                        "description": "bad scheme"
                    }
                ]
            }
        });
        let results = parse_brave_results(&json, 5);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Rust Programming Language");
        assert_eq!(results[0].url, "https://www.rust-lang.org/");
        assert_eq!(
            results[0].snippet.as_deref(),
            Some("A language empowering everyone to build reliable software.")
        );
    }

    #[test]
    fn parses_duckduckgo_results() {
        let json = json!({
            "Heading": "Rust (programming language)",
            "AbstractURL": "https://en.wikipedia.org/wiki/Rust_(programming_language)",
            "Abstract": "Rust is a programming language.",
            "RelatedTopics": [
                {
                    "FirstURL": "https://duckduckgo.com/c/Rust_(programming_language)",
                    "Text": "Rust (programming language) - Category"
                },
                {
                    "Topics": [
                        {
                            "FirstURL": "https://example.com/history",
                            "Text": "History of programming languages - Background"
                        }
                    ]
                }
            ]
        });
        let results = parse_duckduckgo_results(&json, 3);
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].title, "Rust (programming language)");
        assert_eq!(results[1].title, "Rust (programming language)");
        assert_eq!(
            results[2].snippet.as_deref(),
            Some("Background"),
            "{results:?}"
        );
    }

    #[test]
    fn render_is_bounded_and_includes_required_fields() {
        let results = vec![SearchResult {
            title: "Rust Programming Language".to_owned(),
            url: "https://www.rust-lang.org/".to_owned(),
            snippet: Some("Official site".to_owned()),
        }];
        let rendered = render_results(WebSearchProvider::DuckDuckGo, "Rust", &results);
        assert!(rendered.contains("websearch results for \"Rust\""));
        assert!(rendered.contains("1. Rust Programming Language"));
        assert!(rendered.contains("URL: https://www.rust-lang.org/"));
        assert!(rendered.contains("Snippet: Official site"));
    }

    #[test]
    fn network_audit_record_names_endpoint_host_and_method() {
        let path = temp_log("network");
        let sink = audit::AuditSink::new(&path, "ses_websearch");

        record_network(
            Some(&sink),
            "https://api.duckduckgo.com/?q=Rust",
            Some(StatusCode::OK),
            "success",
        );

        let records = audit::read_jsonl(&path).expect("audit records");
        assert!(records.iter().any(|record| {
            record["type"] == "network_call"
                && record["session"] == "ses_websearch"
                && record["tool"] == "websearch"
                && record["method"] == "GET"
                && record["host"] == "api.duckduckgo.com"
                && record["status"] == 200
        }));
        let _ = std::fs::remove_file(path);
    }
}
