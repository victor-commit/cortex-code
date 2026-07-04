use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tools::{Tool, ToolContext, ToolRegistry, WebFetchTool};

static WEBFETCH_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn bind_loopback() -> (TcpListener, u16) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind ephemeral loopback listener");
    listener
        .set_nonblocking(false)
        .expect("set listener blocking");
    let port = listener.local_addr().expect("local addr").port();
    (listener, port)
}

fn serve_once(content_type: &'static str, body: Vec<u8>) -> (String, thread::JoinHandle<String>) {
    let (listener, port) = bind_loopback();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept request");
        let request = read_request(&mut stream);
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            body.len()
        );
        stream
            .write_all(response.as_bytes())
            .expect("write response head");
        stream.write_all(&body).expect("write response body");
        request
    });
    (format!("http://127.0.0.1:{port}/page"), handle)
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
    format!("webfetch-test-{}-{nanos}", std::process::id())
}

#[tokio::test]
async fn registry_advertises_webfetch_for_every_model_family() {
    for registry in [ToolRegistry::builtin(), ToolRegistry::gpt()] {
        assert!(registry.get("webfetch").is_some());
        assert!(
            registry
                .definitions()
                .iter()
                .any(|definition| definition.name == "webfetch"),
            "webfetch must be advertised to the model"
        );
    }
}

#[tokio::test]
async fn webfetch_converts_html_to_markdown_and_preserves_links() {
    let _guard = WEBFETCH_TEST_LOCK.lock().await;
    let html = br#"
        <!doctype html>
        <html>
          <head>
            <style>.secret { color: red; }</style>
            <script>console.log('do-not-include');</script>
          </head>
          <body>
            <h1>Fixture Heading</h1>
            <p>Visible paragraph with <a href="/target">a fixture link</a>.</p>
          </body>
        </html>
    "#;
    let (url, server) = serve_once("text/html; charset=utf-8", html.to_vec());
    let ctx = ToolContext::new(std::env::temp_dir()).with_session(unique_session());

    let output = WebFetchTool
        .execute(serde_json::json!({ "url": url }), &ctx)
        .await;

    let request = server.join().expect("server thread");
    assert!(!output.is_error, "got error: {}", output.model_content());
    let rendered = output.model_content();
    assert!(rendered.contains("# Fixture Heading"), "{rendered}");
    assert!(rendered.contains("Visible paragraph"), "{rendered}");
    assert!(rendered.contains("[a fixture link]("), "{rendered}");
    assert!(rendered.contains("/target"), "{rendered}");
    assert!(!rendered.contains("<h1"), "{rendered}");
    assert!(!rendered.contains("console.log"), "{rendered}");
    assert!(!rendered.contains(".secret"), "{rendered}");
    assert!(
        !request.to_ascii_lowercase().contains("authorization:"),
        "webfetch must not send auth headers:\n{request}"
    );
}

#[tokio::test]
async fn webfetch_bounds_large_text_with_a_truncation_signal() {
    let _guard = WEBFETCH_TEST_LOCK.lock().await;
    let body = "line with lots of text\n".repeat(20_000).into_bytes();
    let (url, server) = serve_once("text/plain; charset=utf-8", body);
    let ctx = ToolContext::new(std::env::temp_dir()).with_session(unique_session());

    let output = WebFetchTool
        .execute(
            serde_json::json!({
                "url": url,
                "max_bytes": 4096
            }),
            &ctx,
        )
        .await;

    let _ = server.join().expect("server thread");
    assert!(!output.is_error, "got error: {}", output.model_content());
    let rendered = output.model_content();
    assert!(rendered.contains("Content-Type: text/plain"), "{rendered}");
    assert!(
        rendered.to_ascii_lowercase().contains("truncated"),
        "large response must carry a truncation signal: {rendered}"
    );
    assert!(
        rendered.len() < 90_000,
        "large response must stay bounded, got {} bytes",
        rendered.len()
    );
}

#[tokio::test]
async fn webfetch_returns_binary_images_as_attachments() {
    let _guard = WEBFETCH_TEST_LOCK.lock().await;
    let (url, server) = serve_once("image/png", b"\x89PNG\r\n\x1a\n\x00\xff\x00\xff".to_vec());
    let ctx = ToolContext::new(std::env::temp_dir()).with_session(unique_session());

    let output = WebFetchTool
        .execute(serde_json::json!({ "url": url }), &ctx)
        .await;

    let _ = server.join().expect("server thread");
    assert!(!output.is_error, "got error: {}", output.model_content());
    let image_part = output
        .parts
        .iter()
        .find(|part| matches!(part, protocol::ToolResultPart::Image { .. }))
        .expect("image attachment part");
    let rendered = output.model_content();
    assert!(rendered.contains("Content-Type: image/png"), "{rendered}");
    assert!(rendered.contains("attachment"), "{rendered}");
    assert!(!rendered.contains('\u{fffd}'), "{rendered}");
    assert!(
        matches!(
            image_part,
            protocol::ToolResultPart::Image { media_type, path }
                if media_type == "image/png" && path.starts_with("http://127.0.0.1:")
        ),
        "{image_part:?}"
    );
}

#[tokio::test]
async fn webfetch_returns_pdf_as_document_attachment() {
    let _guard = WEBFETCH_TEST_LOCK.lock().await;
    let (url, server) = serve_once("application/pdf", b"%PDF-1.4\n%fixture\n".to_vec());
    let ctx = ToolContext::new(std::env::temp_dir()).with_session(unique_session());

    let output = WebFetchTool
        .execute(serde_json::json!({ "url": url }), &ctx)
        .await;

    let _ = server.join().expect("server thread");
    assert!(!output.is_error, "got error: {}", output.model_content());
    let document_part = output
        .parts
        .iter()
        .find(|part| matches!(part, protocol::ToolResultPart::Document { .. }))
        .expect("document attachment part");
    let rendered = output.model_content();
    assert!(
        rendered.contains("Content-Type: application/pdf"),
        "{rendered}"
    );
    assert!(
        matches!(
            document_part,
            protocol::ToolResultPart::Document { media_type, path }
                if media_type == "application/pdf" && path.starts_with("http://127.0.0.1:")
        ),
        "{document_part:?}"
    );
}

#[tokio::test]
async fn webfetch_honors_format_text_and_html() {
    let _guard = WEBFETCH_TEST_LOCK.lock().await;
    let html = br#"
        <!doctype html>
        <html>
          <body>
            <h1>Format Fixture</h1>
            <p>Visible <strong>content</strong>.</p>
          </body>
        </html>
    "#;
    let (url, server) = serve_once("text/html; charset=utf-8", html.to_vec());
    let ctx = ToolContext::new(std::env::temp_dir()).with_session(unique_session());

    let html_output = WebFetchTool
        .execute(serde_json::json!({ "url": url, "format": "html" }), &ctx)
        .await;
    let _ = server.join().expect("server thread");
    assert!(
        !html_output.is_error,
        "got error: {}",
        html_output.model_content()
    );
    let html_rendered = html_output.model_content();
    assert!(html_rendered.contains("~~~~html"), "{html_rendered}");
    assert!(
        html_rendered.contains("<h1>Format Fixture</h1>"),
        "{html_rendered}"
    );

    let (url, server) = serve_once("text/html; charset=utf-8", html.to_vec());
    let text_output = WebFetchTool
        .execute(serde_json::json!({ "url": url, "format": "text" }), &ctx)
        .await;
    let _ = server.join().expect("server thread");
    assert!(
        !text_output.is_error,
        "got error: {}",
        text_output.model_content()
    );
    let text_rendered = text_output.model_content();
    assert!(text_rendered.contains("Format Fixture"), "{text_rendered}");
    assert!(
        text_rendered.contains("Visible content."),
        "{text_rendered}"
    );
    assert!(!text_rendered.contains("<h1>"), "{text_rendered}");
}

#[tokio::test]
async fn webfetch_rejects_malformed_urls_cleanly() {
    let _guard = WEBFETCH_TEST_LOCK.lock().await;
    let ctx = ToolContext::new(std::env::temp_dir()).with_session(unique_session());

    let output = WebFetchTool
        .execute(serde_json::json!({ "url": "not-a-url" }), &ctx)
        .await;

    assert!(output.is_error);
    let rendered = output.model_content();
    assert!(rendered.contains("invalid URL"), "{rendered}");
    assert!(!rendered.contains("panicked"), "{rendered}");
}

#[tokio::test]
async fn webfetch_honors_no_net_before_connecting() {
    let _guard = WEBFETCH_TEST_LOCK.lock().await;
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
        .args(["--exact", "webfetch_no_net_child", "--nocapture"])
        .env("CORTEX_NO_NET", "1")
        .env(
            "CORTEX_WEBFETCH_NO_NET_URL",
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
        "webfetch must not open a socket under CORTEX_NO_NET"
    );
}

#[tokio::test]
async fn webfetch_no_net_child() {
    let Some(url) = std::env::var("CORTEX_WEBFETCH_NO_NET_URL").ok() else {
        return;
    };
    let ctx = ToolContext::new(std::env::temp_dir()).with_session(unique_session());

    let output = WebFetchTool
        .execute(serde_json::json!({ "url": url }), &ctx)
        .await;

    assert!(output.is_error);
    let rendered = output.model_content();
    assert!(rendered.contains("network disabled"), "{rendered}");
    assert!(rendered.contains("CORTEX_NO_NET=1"), "{rendered}");
}

#[tokio::test]
async fn webfetch_timeout_on_stalling_server_is_bounded() {
    let _guard = WEBFETCH_TEST_LOCK.lock().await;
    let (listener, port) = bind_loopback();
    let (accepted_tx, accepted_rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let (_stream, _) = listener.accept().expect("accept stalling request");
        accepted_tx.send(()).expect("send accepted");
        thread::sleep(Duration::from_millis(900));
    });
    let ctx = ToolContext::new(std::env::temp_dir()).with_session(unique_session());

    let output = WebFetchTool
        .execute(
            serde_json::json!({
                "url": format!("http://127.0.0.1:{port}/hang"),
                "timeout_ms": 200
            }),
            &ctx,
        )
        .await;

    accepted_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("server accepted the request");
    handle.join().expect("stalling server thread");
    assert!(output.is_error, "stalling server should fail");
    let rendered = output.model_content().to_ascii_lowercase();
    assert!(
        rendered.contains("timeout") || rendered.contains("transport"),
        "expected timeout/transport error, got: {}",
        output.model_content()
    );
}
