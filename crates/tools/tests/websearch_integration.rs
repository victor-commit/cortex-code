use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tools::{Tool, ToolContext, ToolRegistry, WebSearchConfig, WebSearchProvider, WebSearchTool};

static WEBSEARCH_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn bind_loopback() -> (TcpListener, u16) {
    for port in 3200..=3299 {
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) {
            listener
                .set_nonblocking(false)
                .expect("set listener blocking");
            return (listener, port);
        }
    }
    panic!("no free loopback port in 3200-3299");
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
