use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tools::{Tool, ToolContext, ToolRegistry, WebSearchConfig, WebSearchProvider, WebSearchTool};

static WEBSEARCH_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn bind_loopback() -> (TcpListener, u16) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind ephemeral loopback listener");
    listener
        .set_nonblocking(false)
        .expect("set listener blocking");
    let port = listener.local_addr().expect("local addr").port();
    (listener, port)
}

fn serve_once(
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
) -> (String, thread::JoinHandle<String>) {
    let (listener, port) = bind_loopback();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept request");
        let request = read_request(&mut stream);
        let response = format!(
            "HTTP/1.1 {status} test\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            body.len()
        );
        stream
            .write_all(response.as_bytes())
            .expect("write response head");
        stream.write_all(&body).expect("write response body");
        request
    });
    (format!("http://127.0.0.1:{port}/search"), handle)
}

fn read_request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("set request timeout");
    let mut request = Vec::new();
    let mut buf = [0u8; 512];
    loop {
        let read = stream.read(&mut buf).expect("read request");
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buf[..read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    String::from_utf8_lossy(&request).into_owned()
}

fn unique_session() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    format!("websearch-test-{}-{nanos}", std::process::id())
}

#[tokio::test]
async fn registries_omit_websearch_until_configured() {
    for registry in [ToolRegistry::builtin(), ToolRegistry::gpt()] {
        assert!(registry.get("websearch").is_none());
        assert!(
            !registry
                .definitions()
                .iter()
                .any(|definition| definition.name == "websearch"),
            "websearch must be default-off"
        );
    }

    let mut registry = ToolRegistry::builtin();
    registry.register_websearch(WebSearchConfig::duckduckgo());
    assert!(registry.get("websearch").is_some());
}

#[tokio::test]
async fn duckduckgo_provider_returns_bounded_real_result_entries() {
    let _guard = WEBSEARCH_TEST_LOCK.lock().await;
    let body = serde_json::json!({
        "Heading": "Rust (programming language)",
        "AbstractURL": "https://www.rust-lang.org/",
        "Abstract": "Rust is a language empowering everyone to build reliable and efficient software.",
        "RelatedTopics": [
            {
                "FirstURL": "https://doc.rust-lang.org/book/",
                "Text": "The Rust Programming Language - The official Rust book"
            },
            {
                "FirstURL": "https://crates.io/",
                "Text": "crates.io - The Rust community crate registry"
            }
        ]
    })
    .to_string()
    .into_bytes();
    let (endpoint, server) = serve_once(200, "application/json", body);
    let config = WebSearchConfig::duckduckgo()
        .with_endpoint(Some(endpoint))
        .expect("endpoint config")
        .with_max_results(Some(3))
        .expect("max results config");
    let ctx = ToolContext::new(std::env::temp_dir()).with_session(unique_session());

    let output = WebSearchTool::new(config)
        .execute(
            serde_json::json!({
                "query": "Rust programming language",
                "count": 3
            }),
            &ctx,
        )
        .await;

    let request = server.join().expect("server thread");
    assert!(!output.is_error, "got error: {}", output.model_content());
    let rendered = output.model_content();
    assert!(rendered.contains("websearch results"), "{rendered}");
    assert!(
        rendered.contains("Rust (programming language)"),
        "{rendered}"
    );
    assert!(
        rendered.contains("https://www.rust-lang.org/"),
        "{rendered}"
    );
    assert!(
        rendered.contains("https://doc.rust-lang.org/book/"),
        "{rendered}"
    );
    assert!(
        !request.to_ascii_lowercase().contains("authorization:"),
        "keyless provider must not send Authorization headers:\n{request}"
    );
    assert!(
        request.contains("q=Rust+programming+language")
            || request.contains("q=Rust%20programming%20language"),
        "request should carry the real query:\n{request}"
    );
}

#[tokio::test]
async fn provider_secret_shaped_title_url_snippet_never_reach_model_content() {
    let _guard = WEBSEARCH_TEST_LOCK.lock().await;
    // Runtime-assembled so no contiguous key-shaped literal sits in source (Droid-Shield).
    let secret = format!("sk-{}-{}", "or-v1", "deadbeef".repeat(8));
    let body = serde_json::json!({
        "Heading": format!("Title with {secret}"),
        "AbstractURL": format!("https://example.test/result?token={secret}"),
        "Abstract": format!("Snippet embeds {secret} twice {secret}"),
        "RelatedTopics": [
            {
                "FirstURL": format!("https://docs.example.test/path?auth={secret}"),
                "Text": format!("Related title - related body with {secret}")
            }
        ]
    })
    .to_string()
    .into_bytes();
    let (endpoint, server) = serve_once(200, "application/json", body);
    let config = WebSearchConfig::duckduckgo()
        .with_endpoint(Some(endpoint))
        .expect("endpoint config");
    let ctx = ToolContext::new(std::env::temp_dir()).with_session(unique_session());

    let output = WebSearchTool::new(config)
        .execute(
            serde_json::json!({
                "query": "secret probe",
                "livecrawl": true
            }),
            &ctx,
        )
        .await;

    let _ = server.join().expect("server thread");
    assert!(!output.is_error, "got error: {}", output.model_content());
    let rendered = output.model_content();
    assert!(
        !rendered.contains(&secret),
        "raw secret must not appear in model-facing ToolOutput: {rendered}"
    );
    assert!(
        !output
            .parts
            .iter()
            .any(|part| part.as_text().is_some_and(|text| text.contains(&secret))),
        "raw secret must not appear in ToolOutput parts: {:?}",
        output.parts
    );
    assert!(
        rendered.contains(redaction::PLACEHOLDER),
        "secret-shaped provider fields must be replaced with the placeholder: {rendered}"
    );
    // Clean structure and surrounding text must still render.
    assert!(rendered.contains("websearch results"), "{rendered}");
    assert!(rendered.contains("Title with"), "{rendered}");
    assert!(rendered.contains("example.test"), "{rendered}");
    assert!(rendered.contains("Snippet embeds"), "{rendered}");
}

#[tokio::test]
async fn websearch_accepts_num_results_alias_and_caps_context() {
    let _guard = WEBSEARCH_TEST_LOCK.lock().await;
    let body = serde_json::json!({
        "RelatedTopics": [
            {
                "FirstURL": "https://example.com/one",
                "Text": "One - abcdefghijklmnopqrstuvwxyz"
            },
            {
                "FirstURL": "https://example.com/two",
                "Text": "Two - 0123456789abcdefghijklmnopqrstuvwxyz"
            },
            {
                "FirstURL": "https://example.com/three",
                "Text": "Three - should-not-appear"
            }
        ]
    })
    .to_string()
    .into_bytes();
    let (endpoint, server) = serve_once(200, "application/json", body);
    let config = WebSearchConfig::duckduckgo()
        .with_endpoint(Some(endpoint))
        .expect("endpoint config")
        .with_max_results(Some(10))
        .expect("max results config");
    let ctx = ToolContext::new(std::env::temp_dir()).with_session(unique_session());

    let output = WebSearchTool::new(config)
        .execute(
            serde_json::json!({
                "query": "Rust",
                "numResults": 2,
                "livecrawl": true,
                "contextMaxCharacters": 12
            }),
            &ctx,
        )
        .await;

    let request = server.join().expect("server thread");
    assert!(!output.is_error, "got error: {}", output.model_content());
    let rendered = output.model_content();
    assert!(rendered.contains("1. One"), "{rendered}");
    assert!(rendered.contains("2. Two"), "{rendered}");
    assert!(!rendered.contains("3. Three"), "{rendered}");
    assert!(rendered.contains("Snippet: abcdefghijkl"), "{rendered}");
    assert!(!rendered.contains("Snippet: abcdefghijklm"), "{rendered}");
    assert!(
        request.contains("q=Rust"),
        "request should carry the real query:\n{request}"
    );
}

#[tokio::test]
async fn websearch_livecrawl_false_omits_hydrated_snippets() {
    let _guard = WEBSEARCH_TEST_LOCK.lock().await;
    let body = serde_json::json!({
        "RelatedTopics": [
            {
                "FirstURL": "https://example.com/one",
                "Text": "One - hydrated context should be hidden"
            }
        ]
    })
    .to_string()
    .into_bytes();
    let (endpoint, server) = serve_once(200, "application/json", body);
    let config = WebSearchConfig::duckduckgo()
        .with_endpoint(Some(endpoint))
        .expect("endpoint config");
    let ctx = ToolContext::new(std::env::temp_dir()).with_session(unique_session());

    let output = WebSearchTool::new(config)
        .execute(
            serde_json::json!({
                "query": "Rust",
                "livecrawl": false
            }),
            &ctx,
        )
        .await;

    let _ = server.join().expect("server thread");
    assert!(!output.is_error, "got error: {}", output.model_content());
    let rendered = output.model_content();
    assert!(rendered.contains("1. One"), "{rendered}");
    assert!(!rendered.contains("Snippet:"), "{rendered}");
}

#[tokio::test]
async fn websearch_type_parameter_shapes_provider_request() {
    let _guard = WEBSEARCH_TEST_LOCK.lock().await;
    let body = serde_json::json!({
        "RelatedTopics": [
            {
                "FirstURL": "https://example.com/news",
                "Text": "News - Result"
            }
        ]
    })
    .to_string()
    .into_bytes();
    let (endpoint, server) = serve_once(200, "application/json", body);
    let config = WebSearchConfig::duckduckgo()
        .with_endpoint(Some(endpoint))
        .expect("endpoint config");
    let ctx = ToolContext::new(std::env::temp_dir()).with_session(unique_session());

    let output = WebSearchTool::new(config)
        .execute(
            serde_json::json!({
                "query": "Rust",
                "type": "news"
            }),
            &ctx,
        )
        .await;

    let request = server.join().expect("server thread");
    assert!(!output.is_error, "got error: {}", output.model_content());
    assert!(
        request.contains("ia=news"),
        "request should carry the selected type:\n{request}"
    );
}

#[tokio::test]
async fn brave_missing_key_is_clear_and_actionable() {
    let _guard = WEBSEARCH_TEST_LOCK.lock().await;
    let config = WebSearchConfig::new(WebSearchProvider::Brave);
    let ctx = ToolContext::new(std::env::temp_dir()).with_session(unique_session());

    let output = WebSearchTool::new(config)
        .execute(
            serde_json::json!({ "query": "Rust programming language" }),
            &ctx,
        )
        .await;

    assert!(output.is_error);
    let rendered = output.model_content();
    assert!(
        rendered.contains("provider brave requires an API key"),
        "{rendered}"
    );
    assert!(rendered.contains("websearch"), "{rendered}");
}

#[tokio::test]
async fn provider_auth_failure_is_bounded_and_not_fabricated() {
    let _guard = WEBSEARCH_TEST_LOCK.lock().await;
    let body = br#"{"error":"invalid subscription token"}"#.to_vec();
    let (endpoint, server) = serve_once(401, "application/json", body);
    let config = WebSearchConfig::new(WebSearchProvider::Brave)
        .with_api_key(Some("invalid-test-token".to_owned()))
        .with_endpoint(Some(endpoint))
        .expect("endpoint config");
    let ctx = ToolContext::new(std::env::temp_dir()).with_session(unique_session());

    let output = WebSearchTool::new(config)
        .execute(
            serde_json::json!({ "query": "Rust programming language" }),
            &ctx,
        )
        .await;

    let request = server.join().expect("server thread");
    assert!(output.is_error);
    let rendered = output.model_content();
    assert!(rendered.contains("HTTP 401"), "{rendered}");
    assert!(rendered.contains("authentication failed"), "{rendered}");
    assert!(
        !rendered.contains("Rust Programming Language"),
        "an auth error must not fabricate search results: {rendered}"
    );
    assert!(
        request
            .to_ascii_lowercase()
            .contains("x-subscription-token: invalid-test-token"),
        "brave request should include the configured provider key header:\n{request}"
    );
}

#[tokio::test]
async fn websearch_reports_a_bounded_transport_error_for_a_closed_host() {
    let _guard = WEBSEARCH_TEST_LOCK.lock().await;
    let (listener, port) = bind_loopback();
    drop(listener);
    let config = WebSearchConfig::duckduckgo()
        .with_endpoint(Some(format!("http://127.0.0.1:{port}/unreachable")))
        .expect("endpoint config");
    let ctx = ToolContext::new(std::env::temp_dir()).with_session(unique_session());
    let started = std::time::Instant::now();

    let output = WebSearchTool::new(config)
        .execute(
            serde_json::json!({
                "query": "Rust programming language",
                "timeout_ms": 200
            }),
            &ctx,
        )
        .await;

    assert!(output.is_error);
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "closed-host websearch must return within its bounded timeout"
    );
    let rendered = output.model_content().to_ascii_lowercase();
    assert!(
        rendered.contains("transport") || rendered.contains("timeout"),
        "expected transport/timeout error, got: {}",
        output.model_content()
    );
    assert!(
        !rendered.contains("websearch results"),
        "an unreachable provider must not fabricate results: {}",
        output.model_content()
    );
}

#[tokio::test]
async fn websearch_timeout_on_stalling_provider_is_bounded() {
    let _guard = WEBSEARCH_TEST_LOCK.lock().await;
    let (listener, port) = bind_loopback();
    let (accepted_tx, accepted_rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let (_stream, _) = listener.accept().expect("accept stalling request");
        accepted_tx.send(()).expect("send accepted");
        thread::sleep(Duration::from_millis(900));
    });
    let config = WebSearchConfig::duckduckgo()
        .with_endpoint(Some(format!("http://127.0.0.1:{port}/hang")))
        .expect("endpoint config");
    let ctx = ToolContext::new(std::env::temp_dir()).with_session(unique_session());

    let output = WebSearchTool::new(config)
        .execute(
            serde_json::json!({
                "query": "Rust programming language",
                "timeout_ms": 200
            }),
            &ctx,
        )
        .await;

    accepted_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("server accepted the request");
    handle.join().expect("stalling server thread");
    assert!(output.is_error, "stalling provider should fail");
    let rendered = output.model_content().to_ascii_lowercase();
    assert!(
        rendered.contains("timeout") || rendered.contains("transport"),
        "expected timeout/transport error, got: {}",
        output.model_content()
    );
    assert!(
        !rendered.contains("websearch results"),
        "a stalled provider must not fabricate results: {}",
        output.model_content()
    );
}

#[tokio::test]
async fn websearch_rejects_embedded_endpoint_credentials_before_connecting() {
    let _guard = WEBSEARCH_TEST_LOCK.lock().await;
    let (listener, port) = bind_loopback();
    listener
        .set_nonblocking(true)
        .expect("set listener nonblocking");
    let (accepted_tx, accepted_rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let start = SystemTime::now();
        loop {
            match listener.accept() {
                Ok((_stream, _)) => {
                    accepted_tx.send(true).expect("send accepted");
                    return;
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    if start.elapsed().expect("elapsed") > Duration::from_millis(500) {
                        accepted_tx.send(false).expect("send no accept");
                        return;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(err) => panic!("accept failed: {err}"),
            }
        }
    });
    let password = "websearch-fixture-password";
    let result = WebSearchConfig::duckduckgo().with_endpoint(Some(format!(
        "http://user:{password}@127.0.0.1:{port}/search"
    )));

    let accepted = accepted_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("accept result");
    handle.join().expect("server thread");
    let error = result.expect_err("endpoint credentials must be rejected");
    assert!(
        error.to_ascii_lowercase().contains("embedded credentials"),
        "expected credential rejection, got: {error}"
    );
    assert!(!error.contains(password), "endpoint secret leaked: {error}");
    assert!(
        !accepted,
        "websearch must reject endpoint credentials before opening a socket"
    );
}

#[tokio::test]
async fn websearch_does_not_follow_redirects_with_embedded_credentials() {
    let _guard = WEBSEARCH_TEST_LOCK.lock().await;
    let (target_listener, target_port) = bind_loopback();
    target_listener
        .set_nonblocking(true)
        .expect("set target listener nonblocking");
    let (target_accepted_tx, target_accepted_rx) = mpsc::channel();
    let target_handle = thread::spawn(move || {
        let start = SystemTime::now();
        loop {
            match target_listener.accept() {
                Ok((_stream, _)) => {
                    target_accepted_tx.send(true).expect("send accepted");
                    return;
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    if start.elapsed().expect("elapsed") > Duration::from_millis(500) {
                        target_accepted_tx.send(false).expect("send no accept");
                        return;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(err) => panic!("target accept failed: {err}"),
            }
        }
    });
    let (redirect_listener, redirect_port) = bind_loopback();
    let password = "websearch-redirect-password";
    let redirect_handle = thread::spawn(move || {
        let (mut stream, _) = redirect_listener.accept().expect("accept redirect request");
        let _ = read_request(&mut stream);
        let response = format!(
            "HTTP/1.1 302 Found\r\nlocation: http://user:{password}@127.0.0.1:{target_port}/probe\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
        );
        stream
            .write_all(response.as_bytes())
            .expect("write redirect response");
    });
    let config = WebSearchConfig::duckduckgo()
        .with_endpoint(Some(format!("http://127.0.0.1:{redirect_port}/redirect")))
        .expect("endpoint config");
    let ctx = ToolContext::new(std::env::temp_dir()).with_session(unique_session());

    let output = WebSearchTool::new(config)
        .execute(
            serde_json::json!({
                "query": "Rust programming language"
            }),
            &ctx,
        )
        .await;

    redirect_handle.join().expect("redirect server thread");
    let target_accepted = target_accepted_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("target accept result");
    target_handle.join().expect("target server thread");
    assert!(output.is_error);
    assert!(
        output
            .model_content()
            .to_ascii_lowercase()
            .contains("embedded credentials"),
        "expected credential rejection, got: {}",
        output.model_content()
    );
    assert!(
        !output.model_content().contains(password),
        "redirect secret leaked: {}",
        output.model_content()
    );
    assert!(
        !target_accepted,
        "websearch must not follow a redirect carrying credentials"
    );
}

#[tokio::test]
async fn websearch_honors_no_net_before_connecting() {
    let _guard = WEBSEARCH_TEST_LOCK.lock().await;
    let (listener, port) = bind_loopback();
    listener
        .set_nonblocking(true)
        .expect("set listener nonblocking");
    let (accepted_tx, accepted_rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let start = SystemTime::now();
        loop {
            match listener.accept() {
                Ok((_stream, _)) => {
                    accepted_tx.send(true).expect("send accepted");
                    return;
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    if start.elapsed().expect("elapsed") > Duration::from_millis(500) {
                        accepted_tx.send(false).expect("send no accept");
                        return;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(err) => panic!("accept failed: {err}"),
            }
        }
    });
    let child = Command::new(std::env::current_exe().expect("current test binary"))
        .args(["--exact", "websearch_no_net_child", "--nocapture"])
        .env("CORTEX_NO_NET", "1")
        .env(
            "CORTEX_WEBSEARCH_NO_NET_URL",
            format!("http://127.0.0.1:{port}/blocked"),
        )
        .output()
        .expect("spawn no-net child test");

    let accepted = accepted_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("accept result");
    handle.join().expect("server thread");
    assert!(
        child.status.success(),
        "child stdout:\n{}\nchild stderr:\n{}",
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr)
    );
    assert!(
        !accepted,
        "websearch must not open a socket under CORTEX_NO_NET"
    );
}

#[tokio::test]
async fn websearch_no_net_child() {
    let Some(endpoint) = std::env::var("CORTEX_WEBSEARCH_NO_NET_URL").ok() else {
        return;
    };
    let config = WebSearchConfig::duckduckgo()
        .with_endpoint(Some(endpoint))
        .expect("endpoint config");
    let ctx = ToolContext::new(std::env::temp_dir()).with_session(unique_session());

    let output = WebSearchTool::new(config)
        .execute(
            serde_json::json!({ "query": "Rust programming language" }),
            &ctx,
        )
        .await;

    assert!(output.is_error);
    let rendered = output.model_content();
    assert!(rendered.contains("network disabled"), "{rendered}");
    assert!(rendered.contains("CORTEX_NO_NET=1"), "{rendered}");
}
