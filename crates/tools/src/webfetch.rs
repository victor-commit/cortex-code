//! The `webfetch` tool: real HTTP(S) retrieval with bounded markdown output.
//!
//! The tool opens a real network request to the requested URL, after consulting the shared
//! `CORTEX_NO_NET` outbound guard. HTML responses are converted into model-readable markdown with
//! script/style payloads omitted and links preserved. Large responses are capped and marked as
//! truncated, and binary or non-HTML responses report their content type without dumping bytes into
//! the transcript.

use std::time::Duration;

use futures_util::StreamExt;
use protocol::ToolResultPart;
use redaction::guard_outbound;
use reqwest::StatusCode;
use reqwest::header::{ACCEPT, CONTENT_TYPE, USER_AGENT};
use serde_json::{Value, json};
use url::Url;

use crate::output::{OutputLimits, bound_output};
use crate::tool::{Tool, ToolContext, ToolOutput};

const DESCRIPTION: &str = "Fetch a real HTTP(S) URL over the network and return bounded markdown. \
Use this when the user asks you to read, retrieve, fetch, summarize, or answer from a web page. \
HTML is converted to markdown with visible text, headings, lists, and links preserved while \
scripts/styles are omitted. Large and non-HTML responses are bounded and clearly labeled. Never \
guess page contents when a URL is available, call webfetch first.";

const OUTBOUND_OPERATION: &str = "webfetch HTTP request";
const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const MAX_TIMEOUT_MS: u64 = 120_000;
const MIN_TIMEOUT_MS: u64 = 50;
const MIN_TIMEOUT_SECS: u64 = 1;
const MAX_TIMEOUT_SECS: u64 = 120;
const DEFAULT_MAX_BYTES: usize = 5 * 1024 * 1024;
const MAX_MAX_BYTES: usize = 5 * 1024 * 1024;
const MIN_MAX_BYTES: usize = 1024;
const MAX_REDIRECTS: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResponseFormat {
    Markdown,
    Text,
    Html,
}

/// Fetch a web page and return markdown.
pub struct WebFetchTool;

impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "webfetch"
    }

    fn description(&self) -> &str {
        DESCRIPTION
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The absolute http:// or https:// URL to retrieve."
                },
                "format": {
                    "type": "string",
                    "enum": ["markdown", "text", "html"],
                    "description": "Optional output format. markdown converts HTML to markdown (default), text returns plain text, and html returns raw HTML when available."
                },
                "timeout": {
                    "type": "integer",
                    "description": "Optional timeout in seconds (compatibility alias), clamped between 1 and 120. Defaults to 30."
                },
                "timeout_ms": {
                    "type": "integer",
                    "description": "Optional total timeout in milliseconds, clamped between 50 and 120000. Defaults to 30000."
                },
                "max_bytes": {
                    "type": "integer",
                    "description": "Optional maximum response bytes to read before truncating, clamped between 1024 and 5242880. Defaults to 5242880."
                }
            },
            "required": ["url"],
            "additionalProperties": false
        })
    }

    fn execute<'a>(
        &'a self,
        arguments: Value,
        ctx: &'a ToolContext,
    ) -> futures_util::future::BoxFuture<'a, ToolOutput> {
        Box::pin(async move { run(arguments, ctx).await })
    }
}

async fn run(arguments: Value, ctx: &ToolContext) -> ToolOutput {
    let Some(raw_url) = string_arg(&arguments, "url").or_else(|| string_arg(&arguments, "uri"))
    else {
        return ToolOutput::error("webfetch: the 'url' argument is required");
    };
    let raw_url = raw_url.trim();
    if raw_url.is_empty() {
        return ToolOutput::error("webfetch: the 'url' argument must not be empty");
    }

    let url = match parse_http_url(raw_url) {
        Ok(url) => url,
        Err(message) => return ToolOutput::error(message),
    };

    if let Err(message) = ctx.ensure_permitted("webfetch", &safe_url(&url)) {
        return ToolOutput::error(redaction::redact(&message));
    }

    let options = match options_from(&arguments) {
        Ok(options) => options,
        Err(message) => return ToolOutput::error(message),
    };

    match fetch(&url, options, ctx.audit_sink().map(|sink| sink.as_ref())).await {
        Ok(output) => output,
        Err(message) => ToolOutput::error(message),
    }
}

#[derive(Clone, Copy)]
struct FetchOptions {
    timeout: Duration,
    max_bytes: usize,
    format: ResponseFormat,
}

impl Default for FetchOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_millis(DEFAULT_TIMEOUT_MS),
            max_bytes: DEFAULT_MAX_BYTES,
            format: ResponseFormat::Markdown,
        }
    }
}

fn options_from(arguments: &Value) -> Result<FetchOptions, String> {
    let mut options = FetchOptions::default();
    if let Some(value) = arguments.get("format") {
        let Some(raw_format) = value.as_str() else {
            return Err(
                "webfetch: 'format' must be one of `markdown`, `text`, or `html`".to_owned(),
            );
        };
        options.format = parse_format(raw_format)?;
    }
    if let Some(value) = arguments
        .get("timeout_ms")
        .or_else(|| arguments.get("timeoutMs"))
    {
        let Some(ms) = value.as_u64() else {
            return Err("webfetch: 'timeout_ms' must be an integer".to_owned());
        };
        options.timeout = Duration::from_millis(ms.clamp(MIN_TIMEOUT_MS, MAX_TIMEOUT_MS));
    } else if let Some(value) = arguments.get("timeout") {
        let Some(seconds) = value.as_u64() else {
            return Err("webfetch: 'timeout' must be an integer number of seconds".to_owned());
        };
        options.timeout = Duration::from_secs(seconds.clamp(MIN_TIMEOUT_SECS, MAX_TIMEOUT_SECS));
    }
    if let Some(value) = arguments
        .get("max_bytes")
        .or_else(|| arguments.get("maxBytes"))
    {
        let Some(max_bytes) = value.as_u64() else {
            return Err("webfetch: 'max_bytes' must be an integer".to_owned());
        };
        let max_bytes = (max_bytes as usize).clamp(MIN_MAX_BYTES, MAX_MAX_BYTES);
        options.max_bytes = max_bytes;
    }
    Ok(options)
}

fn parse_format(raw: &str) -> Result<ResponseFormat, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "markdown" | "md" => Ok(ResponseFormat::Markdown),
        "text" | "txt" => Ok(ResponseFormat::Text),
        "html" => Ok(ResponseFormat::Html),
        _ => Err("webfetch: 'format' must be one of `markdown`, `text`, or `html`".to_owned()),
    }
}

async fn fetch(
    url: &Url,
    options: FetchOptions,
    audit: Option<&audit::AuditSink>,
) -> Result<ToolOutput, String> {
    if let Err(diagnostic) = guard_outbound(OUTBOUND_OPERATION) {
        record_network(audit, url, None, "blocked");
        return Err(format!("webfetch: {diagnostic}"));
    }

    let client = reqwest::Client::builder()
        .use_rustls_tls()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if has_embedded_credentials(attempt.url()) {
                // Stop before reqwest can construct a request that derives Basic authentication
                // from the redirect target's userinfo.
                attempt.stop()
            } else if attempt.previous().len() > MAX_REDIRECTS {
                attempt.error("too many redirects")
            } else {
                attempt.follow()
            }
        }))
        .connect_timeout(options.timeout.min(Duration::from_secs(3)))
        .timeout(options.timeout)
        .user_agent(concat!(
            "cortex-code/",
            env!("CARGO_PKG_VERSION"),
            " webfetch"
        ))
        .build()
        .map_err(|err| redaction::redact(&format!("webfetch: transport setup failed: {err}")))?;

    let request = client
        .get(url.clone())
        .header(USER_AGENT, concat!("cortex-code/", env!("CARGO_PKG_VERSION"), " webfetch"))
        .header(
            ACCEPT,
            "text/html,application/xhtml+xml,text/plain,application/json,application/xml;q=0.9,*/*;q=0.1",
        );

    let response = match tokio::time::timeout(options.timeout, request.send()).await {
        Err(_) => {
            record_network(audit, url, None, "timeout");
            return Err(redaction::redact(&format!(
                "webfetch: timeout fetching {url} after {} ms",
                options.timeout.as_millis()
            )));
        }
        Ok(Err(err)) if err.is_timeout() => {
            record_network(audit, url, None, "timeout");
            return Err(redaction::redact(&format!(
                "webfetch: timeout fetching {url}: {err}"
            )));
        }
        Ok(Err(err)) => {
            record_network(audit, url, None, "error");
            return Err(redaction::redact(&format!(
                "webfetch: transport error fetching {url}: {err}"
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

    if status.is_redirection() {
        record_network(audit, &final_url, Some(status), "blocked");
        return Err(format!(
            "webfetch: refusing redirect with embedded credentials from {}",
            safe_url(&final_url)
        ));
    }

    let body = match collect_body(response, options).await {
        Ok(body) => body,
        Err(message) => {
            record_network(audit, &final_url, Some(status), "error");
            return Err(message);
        }
    };
    if !status.is_success() {
        record_network(audit, &final_url, Some(status), "error");
        return Err(render_status_error(
            &final_url,
            status,
            &content_type,
            &body,
        ));
    }

    record_network(audit, &final_url, Some(status), "success");
    Ok(render_response(
        &final_url,
        status,
        &content_type,
        &body,
        options.format,
    ))
}

fn record_network(
    audit: Option<&audit::AuditSink>,
    url: &Url,
    status: Option<StatusCode>,
    outcome: &str,
) {
    if let Some(audit) = audit {
        audit.record_network_call(
            "webfetch",
            None,
            None,
            "GET",
            url.host_str().unwrap_or(""),
            url.as_str(),
            status.map(|status| status.as_u16()),
            outcome,
        );
    }
}

struct FetchedBody {
    bytes: Vec<u8>,
    truncated: bool,
}

async fn collect_body(
    response: reqwest::Response,
    options: FetchOptions,
) -> Result<FetchedBody, String> {
    let mut bytes = Vec::new();
    let mut truncated = false;
    let mut stream = response.bytes_stream();

    while let Some(next) = match tokio::time::timeout(options.timeout, stream.next()).await {
        Ok(next) => next,
        Err(_) => {
            return Err(redaction::redact(&format!(
                "webfetch: timeout reading response body after {} ms",
                options.timeout.as_millis()
            )));
        }
    } {
        let chunk = next.map_err(|err| {
            if err.is_timeout() {
                redaction::redact(&format!("webfetch: timeout reading response body: {err}"))
            } else {
                redaction::redact(&format!(
                    "webfetch: transport error reading response body: {err}"
                ))
            }
        })?;
        if bytes.len() + chunk.len() > options.max_bytes {
            let remaining = options.max_bytes.saturating_sub(bytes.len());
            bytes.extend_from_slice(&chunk[..remaining]);
            truncated = true;
            break;
        }
        bytes.extend_from_slice(&chunk);
    }

    Ok(FetchedBody { bytes, truncated })
}

fn parse_http_url(raw_url: &str) -> Result<Url, String> {
    let url = Url::parse(raw_url)
        .map_err(|err| redaction::redact(&format!("webfetch: invalid URL: {err}")))?;
    match url.scheme() {
        "http" | "https" if has_embedded_credentials(&url) => {
            Err("webfetch: invalid URL: embedded credentials are not permitted".to_owned())
        }
        "http" | "https" => Ok(url),
        scheme => Err(format!(
            "webfetch: invalid URL: unsupported scheme `{scheme}` (expected http or https)"
        )),
    }
}

fn has_embedded_credentials(url: &Url) -> bool {
    !url.username().is_empty() || url.password().is_some()
}

fn string_arg<'a>(arguments: &'a Value, key: &str) -> Option<&'a str> {
    arguments.get(key).and_then(Value::as_str)
}

fn render_status_error(
    url: &Url,
    status: StatusCode,
    content_type: &str,
    body: &FetchedBody,
) -> String {
    let mut message = format!(
        "webfetch: HTTP {} while fetching {}\nContent-Type: {}",
        status_line(status),
        safe_url(url),
        content_type
    );
    if is_textual_content_type(content_type) && !is_probably_binary(&body.bytes) {
        let excerpt = decode_lossy(&body.bytes);
        let excerpt = excerpt.trim();
        if !excerpt.is_empty() {
            let mut excerpt = excerpt.to_owned();
            if excerpt.len() > 2048 {
                excerpt.truncate(2048);
                excerpt.push_str("\n[webfetch: error body excerpt truncated]");
            }
            message.push_str("\n\n");
            message.push_str(&excerpt);
        }
    }
    if body.truncated {
        message.push_str("\n[webfetch: response body truncated while reading error response]");
    }
    redaction::redact(&message)
}

fn render_response(
    url: &Url,
    status: StatusCode,
    content_type: &str,
    body: &FetchedBody,
    format: ResponseFormat,
) -> ToolOutput {
    if is_html_response(content_type, &body.bytes) {
        render_html_response(url, status, content_type, body, format)
    } else {
        render_non_html_response(url, status, content_type, body, format)
    }
}

fn render_html_response(
    url: &Url,
    status: StatusCode,
    content_type: &str,
    body: &FetchedBody,
    format: ResponseFormat,
) -> ToolOutput {
    let html = decode_lossy(&body.bytes);
    let markdown = html_to_markdown(&html, Some(url));
    let mut full = response_header(url, status, content_type);
    if body.truncated {
        full.push_str(&format!(
            "\n[webfetch: response truncated after {} bytes; content may be incomplete]",
            body.bytes.len()
        ));
    }
    match format {
        ResponseFormat::Markdown => {
            full.push_str("\n\n");
            full.push_str(&markdown);
        }
        ResponseFormat::Text => {
            let text = html_to_plain_text(&html);
            full.push_str("\n\n[webfetch: HTML converted to plain text]\n\n");
            full.push_str("~~~~text\n");
            full.push_str(&text);
            if !text.ends_with('\n') {
                full.push('\n');
            }
            full.push_str("~~~~");
        }
        ResponseFormat::Html => {
            full.push_str("\n\n[webfetch: raw HTML response]\n\n");
            full.push_str("~~~~html\n");
            full.push_str(&html);
            if !html.ends_with('\n') {
                full.push('\n');
            }
            full.push_str("~~~~");
        }
    }
    ToolOutput::text(bound_markdown(&redaction::redact(&full)))
}

fn render_non_html_response(
    url: &Url,
    status: StatusCode,
    content_type: &str,
    body: &FetchedBody,
    format: ResponseFormat,
) -> ToolOutput {
    let mut full = response_header(url, status, content_type);
    if is_binary_content_type(content_type) || is_probably_binary(&body.bytes) {
        full.push_str("\n\n[webfetch: binary response returned as attachment]");
        if body.truncated {
            full.push_str(&format!(
                "\n[webfetch: response truncated after {} bytes]",
                body.bytes.len()
            ));
        }
        let message = bound_markdown(&redaction::redact(&full));
        if let Some(attachment) = binary_attachment_part(content_type, safe_url(url)) {
            return ToolOutput::parts(vec![ToolResultPart::text(message), attachment]);
        }
        return ToolOutput::text(message);
    }

    let text = decode_lossy(&body.bytes);
    match format {
        ResponseFormat::Text => {
            full.push_str("\n\n");
            if body.truncated {
                full.push_str(&format!(
                    "[webfetch: response truncated after {} bytes; content may be incomplete]\n\n",
                    body.bytes.len()
                ));
            }
            full.push_str(&text);
        }
        ResponseFormat::Markdown | ResponseFormat::Html => {
            full.push_str("\n\n[webfetch: non-HTML text response]\n\n");
            if body.truncated {
                full.push_str(&format!(
                    "[webfetch: response truncated after {} bytes; content may be incomplete]\n\n",
                    body.bytes.len()
                ));
            }
            full.push_str("~~~~text\n");
            full.push_str(&text);
            if !text.ends_with('\n') {
                full.push('\n');
            }
            full.push_str("~~~~");
        }
    }
    ToolOutput::text(bound_markdown(&redaction::redact(&full)))
}

fn response_header(url: &Url, status: StatusCode, content_type: &str) -> String {
    format!(
        "Fetched URL: {}\nStatus: {}\nContent-Type: {}",
        safe_url(url),
        status_line(status),
        content_type
    )
}

fn status_line(status: StatusCode) -> String {
    match status.canonical_reason() {
        Some(reason) => format!("{} {reason}", status.as_u16()),
        None => status.as_u16().to_string(),
    }
}

fn safe_url(url: &Url) -> String {
    let mut sanitized = url.clone();
    let _ = sanitized.set_username("");
    let _ = sanitized.set_password(None);
    redaction::redact(sanitized.as_str())
}

fn bound_markdown(markdown: &str) -> String {
    bound_output(markdown, OutputLimits::default(), "webfetch").text
}

fn is_html_response(content_type: &str, bytes: &[u8]) -> bool {
    let media = media_type(content_type);
    media == "text/html" || media == "application/xhtml+xml" || body_looks_like_html(bytes)
}

fn body_looks_like_html(bytes: &[u8]) -> bool {
    let sample = decode_lossy(&bytes[..bytes.len().min(512)]).to_ascii_lowercase();
    sample.contains("<!doctype html") || sample.contains("<html") || sample.contains("<body")
}

fn is_textual_content_type(content_type: &str) -> bool {
    let media = media_type(content_type);
    media.starts_with("text/")
        || media.ends_with("+json")
        || media.ends_with("+xml")
        || matches!(
            media.as_str(),
            "application/json"
                | "application/xml"
                | "application/javascript"
                | "application/x-javascript"
                | "application/yaml"
                | "application/toml"
        )
}

fn is_binary_content_type(content_type: &str) -> bool {
    let media = media_type(content_type);
    media.starts_with("image/")
        || media.starts_with("audio/")
        || media.starts_with("video/")
        || media.starts_with("font/")
        || matches!(
            media.as_str(),
            "application/octet-stream"
                | "application/pdf"
                | "application/zip"
                | "application/gzip"
                | "application/x-tar"
                | "application/x-7z-compressed"
        )
}

fn media_type(content_type: &str) -> String {
    content_type
        .split(';')
        .next()
        .unwrap_or(content_type)
        .trim()
        .to_ascii_lowercase()
}

fn binary_attachment_part(content_type: &str, path: String) -> Option<ToolResultPart> {
    let media_type = media_type(content_type);
    if media_type.starts_with("image/") {
        Some(ToolResultPart::image(media_type, path))
    } else if media_type == "application/pdf" {
        Some(ToolResultPart::document(media_type, path))
    } else {
        None
    }
}

fn is_probably_binary(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }
    if bytes.contains(&0) {
        return true;
    }
    let control = bytes
        .iter()
        .filter(|byte| {
            let byte = **byte;
            byte < 0x20 && !matches!(byte, b'\n' | b'\r' | b'\t')
        })
        .count();
    control * 10 > bytes.len()
}

fn decode_lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn html_to_plain_text(html: &str) -> String {
    let stripped = strip_html_tags(html);
    let decoded = decode_entities(&stripped);
    let text = decoded.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.trim().is_empty() {
        "(no visible text found after HTML conversion)".to_owned()
    } else {
        text
    }
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

fn html_to_markdown(html: &str, base: Option<&Url>) -> String {
    let mut converter = HtmlMarkdown::new(base);
    converter.convert(html)
}

struct HtmlMarkdown<'a> {
    output: String,
    links: Vec<LinkCapture>,
    skip_tag: Option<(String, usize)>,
    base: Option<&'a Url>,
}

struct LinkCapture {
    href: String,
    text: String,
}

impl<'a> HtmlMarkdown<'a> {
    fn new(base: Option<&'a Url>) -> Self {
        Self {
            output: String::new(),
            links: Vec::new(),
            skip_tag: None,
            base,
        }
    }

    fn convert(&mut self, html: &str) -> String {
        let mut index = 0usize;
        while index < html.len() {
            let rest = &html[index..];
            if let Some(after_comment) = rest.strip_prefix("<!--")
                && let Some(end) = after_comment.find("-->")
            {
                index += "<!--".len() + end + "-->".len();
                continue;
            }

            if rest.starts_with('<')
                && let Some(end) = rest.find('>')
            {
                let raw = &rest[1..end];
                self.handle_tag(raw);
                index += end + 1;
                continue;
            }

            let next_tag = rest.find('<').unwrap_or(rest.len());
            self.emit_text(&rest[..next_tag]);
            index += next_tag;
        }

        while let Some(link) = self.links.pop() {
            self.emit_markdown(&link.text);
        }

        cleanup_markdown(&self.output)
    }

    fn handle_tag(&mut self, raw: &str) {
        let tag = parse_tag(raw);
        let Some(tag) = tag else {
            return;
        };

        if let Some((skip, depth)) = &mut self.skip_tag {
            if tag.name == *skip {
                if tag.end {
                    *depth = depth.saturating_sub(1);
                    if *depth == 0 {
                        self.skip_tag = None;
                    }
                } else if !tag.self_closing {
                    *depth += 1;
                }
            }
            return;
        }

        if !tag.end && is_skipped_tag(&tag.name) {
            if !tag.self_closing {
                self.skip_tag = Some((tag.name, 1));
            }
            return;
        }

        if tag.end {
            self.close_tag(&tag.name);
        } else {
            self.open_tag(&tag);
        }
    }

    fn open_tag(&mut self, tag: &ParsedTag) {
        match tag.name.as_str() {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let level = tag
                    .name
                    .strip_prefix('h')
                    .and_then(|n| n.parse::<usize>().ok())
                    .unwrap_or(1)
                    .clamp(1, 6);
                self.ensure_blank_line();
                self.emit_markdown(&format!("{} ", "#".repeat(level)));
            }
            "p" | "div" | "section" | "article" | "main" | "header" | "footer" | "nav"
            | "table" | "thead" | "tbody" | "tr" | "blockquote" => {
                self.ensure_blank_line();
                if tag.name == "blockquote" {
                    self.emit_markdown("> ");
                }
            }
            "ul" | "ol" => self.ensure_blank_line(),
            "li" => {
                self.ensure_newline();
                self.emit_markdown("- ");
            }
            "br" => self.ensure_newline(),
            "a" => {
                if let Some(href) = attr_value(tag.attrs, "href") {
                    self.links.push(LinkCapture {
                        href: self.resolve_link(&href),
                        text: String::new(),
                    });
                }
            }
            "img" => {
                if let Some(src) = attr_value(tag.attrs, "src") {
                    let alt = attr_value(tag.attrs, "alt").unwrap_or_else(|| "image".to_owned());
                    let src = self.resolve_link(&src);
                    self.emit_markdown(&format!("![{}]({src})", escape_markdown_text(&alt)));
                }
            }
            "td" | "th" => self.emit_markdown(" "),
            _ => {}
        }
    }

    fn close_tag(&mut self, name: &str) {
        match name {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "p" | "div" | "section" | "article"
            | "main" | "header" | "footer" | "nav" | "table" | "thead" | "tbody" | "tr"
            | "blockquote" | "ul" | "ol" => self.ensure_blank_line(),
            "li" => self.ensure_newline(),
            "a" => {
                if let Some(link) = self.links.pop() {
                    let text = cleanup_inline(&link.text);
                    if text.is_empty() {
                        return;
                    }
                    if is_safe_link(&link.href) {
                        self.emit_markdown(&format!(
                            "[{}]({})",
                            escape_markdown_text(&text),
                            link.href
                        ));
                    } else {
                        self.emit_markdown(&text);
                    }
                }
            }
            _ => {}
        }
    }

    fn resolve_link(&self, href: &str) -> String {
        let href = decode_entities(href.trim());
        if let Some(base) = self.base
            && let Ok(url) = base.join(&href)
        {
            return url.to_string();
        }
        href
    }

    fn emit_text(&mut self, text: &str) {
        if self.skip_tag.is_some() {
            return;
        }
        let decoded = decode_entities(text);
        self.emit_markdown(&decoded);
    }

    fn emit_markdown(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if let Some(link) = self.links.last_mut() {
            append_collapsed(&mut link.text, text);
        } else {
            append_collapsed(&mut self.output, text);
        }
    }

    fn ensure_newline(&mut self) {
        if let Some(link) = self.links.last_mut() {
            append_collapsed(&mut link.text, " ");
            return;
        }
        trim_trailing_spaces(&mut self.output);
        if !self.output.is_empty() && !self.output.ends_with('\n') {
            self.output.push('\n');
        }
    }

    fn ensure_blank_line(&mut self) {
        if let Some(link) = self.links.last_mut() {
            append_collapsed(&mut link.text, " ");
            return;
        }
        trim_trailing_spaces(&mut self.output);
        if self.output.is_empty() {
            return;
        }
        if self.output.ends_with("\n\n") {
            return;
        }
        if self.output.ends_with('\n') {
            self.output.push('\n');
        } else {
            self.output.push_str("\n\n");
        }
    }
}

struct ParsedTag<'a> {
    name: String,
    attrs: &'a str,
    end: bool,
    self_closing: bool,
}

fn parse_tag(raw: &str) -> Option<ParsedTag<'_>> {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.starts_with('!')
        || trimmed.starts_with('?')
        || trimmed.starts_with('/')
            && trimmed
                .chars()
                .nth(1)
                .is_some_and(|ch| matches!(ch, '!' | '?'))
    {
        return None;
    }

    let end = trimmed.starts_with('/');
    let inner = if end {
        trimmed[1..].trim_start()
    } else {
        trimmed
    };
    let self_closing = inner.trim_end().ends_with('/');
    let name_end = inner
        .char_indices()
        .find_map(|(idx, ch)| {
            (!(ch.is_ascii_alphanumeric() || matches!(ch, '-' | ':'))).then_some(idx)
        })
        .unwrap_or(inner.len());
    if name_end == 0 {
        return None;
    }
    let name = inner[..name_end].to_ascii_lowercase();
    let mut attrs = inner[name_end..].trim();
    if self_closing {
        attrs = attrs.trim_end_matches('/').trim_end();
    }
    Some(ParsedTag {
        name,
        attrs,
        end,
        self_closing,
    })
}

fn is_skipped_tag(name: &str) -> bool {
    matches!(
        name,
        "script" | "style" | "noscript" | "template" | "svg" | "head"
    )
}

fn attr_value(attrs: &str, target: &str) -> Option<String> {
    let mut index = 0usize;
    while index < attrs.len() {
        skip_attr_ws(attrs, &mut index);
        if index >= attrs.len() {
            break;
        }
        let name_start = index;
        while index < attrs.len() {
            let ch = attrs[index..].chars().next()?;
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | ':' | '.') {
                index += ch.len_utf8();
            } else {
                break;
            }
        }
        if name_start == index {
            index += attrs[index..].chars().next()?.len_utf8();
            continue;
        }
        let name = attrs[name_start..index].to_ascii_lowercase();
        skip_attr_ws(attrs, &mut index);
        if !attrs[index..].starts_with('=') {
            if name == target {
                return Some(String::new());
            }
            continue;
        }
        index += 1;
        skip_attr_ws(attrs, &mut index);
        let value = read_attr_value(attrs, &mut index);
        if name == target {
            return Some(decode_entities(&value));
        }
    }
    None
}

fn skip_attr_ws(attrs: &str, index: &mut usize) {
    while *index < attrs.len() {
        let Some(ch) = attrs[*index..].chars().next() else {
            return;
        };
        if ch.is_whitespace() {
            *index += ch.len_utf8();
        } else {
            return;
        }
    }
}

fn read_attr_value(attrs: &str, index: &mut usize) -> String {
    if *index >= attrs.len() {
        return String::new();
    }
    let Some(first) = attrs[*index..].chars().next() else {
        return String::new();
    };
    if matches!(first, '"' | '\'') {
        *index += first.len_utf8();
        let start = *index;
        while *index < attrs.len() {
            let Some(ch) = attrs[*index..].chars().next() else {
                break;
            };
            if ch == first {
                let value = attrs[start..*index].to_owned();
                *index += ch.len_utf8();
                return value;
            }
            *index += ch.len_utf8();
        }
        attrs[start..].to_owned()
    } else {
        let start = *index;
        while *index < attrs.len() {
            let Some(ch) = attrs[*index..].chars().next() else {
                break;
            };
            if ch.is_whitespace() {
                break;
            }
            *index += ch.len_utf8();
        }
        attrs[start..*index].to_owned()
    }
}

fn append_collapsed(target: &mut String, text: &str) {
    let mut pending_space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            pending_space = true;
            continue;
        }
        if pending_space && should_insert_space(target, ch) {
            target.push(' ');
        }
        pending_space = false;
        target.push(ch);
    }
    if pending_space
        && target
            .chars()
            .next_back()
            .is_some_and(|ch| !ch.is_whitespace())
    {
        target.push(' ');
    }
}

fn should_insert_space(target: &str, next: char) -> bool {
    let Some(prev) = target.chars().next_back() else {
        return false;
    };
    if prev.is_whitespace() {
        return false;
    }
    !matches!(next, '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']')
}

fn trim_trailing_spaces(text: &mut String) {
    while text.ends_with(' ') || text.ends_with('\t') {
        text.pop();
    }
}

fn cleanup_markdown(markdown: &str) -> String {
    let mut cleaned = String::new();
    let mut blank_count = 0usize;
    for line in markdown.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            blank_count += 1;
            if blank_count <= 1 && !cleaned.is_empty() {
                cleaned.push('\n');
            }
            continue;
        }
        blank_count = 0;
        if !cleaned.is_empty() && !cleaned.ends_with('\n') {
            cleaned.push('\n');
        }
        cleaned.push_str(line);
        cleaned.push('\n');
    }
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        "(no visible text found after HTML conversion)".to_owned()
    } else {
        cleaned.to_owned()
    }
}

fn cleanup_inline(text: &str) -> String {
    let mut cleaned = String::new();
    append_collapsed(&mut cleaned, text);
    cleaned.trim().to_owned()
}

fn escape_markdown_text(text: &str) -> String {
    text.replace('[', "\\[").replace(']', "\\]")
}

fn is_safe_link(href: &str) -> bool {
    let lower = href.trim_start().to_ascii_lowercase();
    !(lower.starts_with("javascript:") || lower.starts_with("data:"))
}

fn decode_entities(text: &str) -> String {
    let mut decoded = String::new();
    let mut index = 0usize;
    while index < text.len() {
        let rest = &text[index..];
        if let Some(after_amp) = rest.strip_prefix('&')
            && let Some(semi) = after_amp.find(';')
            && semi <= 32
        {
            let entity = &after_amp[..semi];
            if let Some(ch) = decode_entity(entity) {
                decoded.push_str(&ch);
                index += 1 + semi + 1;
                continue;
            }
        }
        let Some(ch) = rest.chars().next() else {
            break;
        };
        decoded.push(ch);
        index += ch.len_utf8();
    }
    decoded
}

fn decode_entity(entity: &str) -> Option<String> {
    match entity {
        "amp" => Some("&".to_owned()),
        "lt" => Some("<".to_owned()),
        "gt" => Some(">".to_owned()),
        "quot" => Some("\"".to_owned()),
        "apos" => Some("'".to_owned()),
        "nbsp" => Some(" ".to_owned()),
        _ if entity.starts_with("#x") || entity.starts_with("#X") => {
            let value = u32::from_str_radix(&entity[2..], 16).ok()?;
            char::from_u32(value).map(|ch| ch.to_string())
        }
        _ if entity.starts_with('#') => {
            let value = entity[1..].parse::<u32>().ok()?;
            char::from_u32(value).map(|ch| ch.to_string())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_log(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "cortex-webfetch-audit-{tag}-{}-{}.jsonl",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn html_converter_omits_script_style_and_keeps_link_markdown() {
        let base = Url::parse("https://example.test/base/page.html").unwrap();
        let markdown = html_to_markdown(
            r#"
            <html><head><style>.x{}</style><script>alert(1)</script></head>
            <body><h2>Title &amp; More</h2><p>See <a href="../next?q=1&amp;x=2">next page</a>.</p></body></html>
            "#,
            Some(&base),
        );

        assert!(markdown.contains("## Title & More"), "{markdown}");
        assert!(
            markdown.contains("[next page](https://example.test/next?q=1&x=2)"),
            "{markdown}"
        );
        assert!(!markdown.contains("alert"), "{markdown}");
        assert!(!markdown.contains(".x"), "{markdown}");
    }

    #[test]
    fn html_converter_skips_unsafe_links() {
        let markdown = html_to_markdown(
            r#"<p><a href="javascript:alert(1)">unsafe link</a></p>"#,
            None,
        );
        assert_eq!(markdown, "unsafe link");
    }

    #[test]
    fn network_audit_record_names_host_endpoint_and_method() {
        let path = temp_log("network");
        let sink = audit::AuditSink::new(&path, "ses_webfetch");
        let url = Url::parse("https://example.com/page?q=1").unwrap();

        record_network(Some(&sink), &url, Some(StatusCode::OK), "success");

        let records = audit::read_jsonl(&path).expect("audit records");
        assert!(records.iter().any(|record| {
            record["type"] == "network_call"
                && record["session"] == "ses_webfetch"
                && record["tool"] == "webfetch"
                && record["method"] == "GET"
                && record["host"] == "example.com"
                && record["endpoint"] == "https://example.com/page?q=1"
                && record["status"] == 200
        }));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn options_timeout_seconds_zero_clamps_to_one_second() {
        let options = options_from(&json!({ "timeout": 0 })).expect("options");
        assert_eq!(options.timeout, Duration::from_secs(1));
    }

    #[test]
    fn options_timeout_seconds_one_remains_one_second() {
        let options = options_from(&json!({ "timeout": 1 })).expect("options");
        assert_eq!(options.timeout, Duration::from_secs(1));
    }

    #[test]
    fn options_timeout_seconds_clamps_to_maximum() {
        let options = options_from(&json!({ "timeout": 999 })).expect("options");
        assert_eq!(options.timeout, Duration::from_secs(MAX_TIMEOUT_SECS));
    }

    #[test]
    fn options_timeout_ms_zero_keeps_millisecond_minimum() {
        let options = options_from(&json!({ "timeout_ms": 0 })).expect("options");
        assert_eq!(options.timeout, Duration::from_millis(MIN_TIMEOUT_MS));
    }

    #[test]
    fn options_timeout_ms_camel_case_alias_keeps_millisecond_minimum() {
        let options = options_from(&json!({ "timeoutMs": 0 })).expect("options");
        assert_eq!(options.timeout, Duration::from_millis(MIN_TIMEOUT_MS));
    }
}
