use std::net::{SocketAddr, TcpListener as StdTcpListener};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use server::{ServeError, app_with_db_path, bind_addr, bind_listener, serve_with_shutdown};
use tower::ServiceExt;

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

#[tokio::test]
async fn health_route_returns_ok_json() {
    let scratch = Scratch::new("server-health");
    let response = app_with_db_path(scratch.db_path())
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("route response");

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 1024).await.expect("body");
    let body: serde_json::Value = serde_json::from_slice(&bytes).expect("health JSON");
    assert_eq!(body, serde_json::json!({"status": "ok"}));
}

#[test]
fn bind_addr_accepts_only_canonical_loopback_and_reserved_ports() {
    assert_eq!(
        bind_addr("127.0.0.1", 3200).expect("loopback"),
        "127.0.0.1:3200".parse::<SocketAddr>().expect("addr")
    );
    assert_eq!(
        bind_addr("localhost", 3299).expect("localhost maps to canonical loopback"),
        "127.0.0.1:3299".parse::<SocketAddr>().expect("addr")
    );

    assert!(matches!(
        bind_addr("0.0.0.0", 3200),
        Err(ServeError::NonLoopbackHost { .. })
    ));
    assert!(matches!(
        bind_addr("192.168.1.20", 3200),
        Err(ServeError::NonLoopbackHost { .. })
    ));
    assert!(matches!(
        bind_addr("127.0.0.1", 3199),
        Err(ServeError::PortOutOfRange { port: 3199 })
    ));
    assert!(matches!(
        bind_addr("127.0.0.1", 3300),
        Err(ServeError::PortOutOfRange { port: 3300 })
    ));
}

#[tokio::test]
async fn busy_port_reports_address_in_use_without_fallback() {
    let (_held, port) = hold_loopback_port();

    let err = match bind_listener("127.0.0.1", port).await {
        Ok(_) => panic!("bind unexpectedly succeeded on held port {port}"),
        Err(err) => err,
    };

    assert!(matches!(err, ServeError::PortBusy { port: p, .. } if p == port));
    let message = err.to_string();
    assert!(message.contains("already in use"), "{message}");
    assert!(message.contains(&port.to_string()), "{message}");
}

#[tokio::test]
async fn graceful_shutdown_releases_listener_port() {
    let std_listener = bind_free_loopback_listener();
    std_listener
        .set_nonblocking(true)
        .expect("listener nonblocking");
    let addr = std_listener.local_addr().expect("local addr");
    let listener = tokio::net::TcpListener::from_std(std_listener).expect("tokio listener");
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let scratch = Scratch::new("server-shutdown");

    let task = tokio::spawn(serve_with_shutdown(
        listener,
        app_with_db_path(scratch.db_path()),
        async move {
            let _ = shutdown_rx.await;
        },
    ));

    assert!(
        StdTcpListener::bind(addr).is_err(),
        "listener should hold {addr} before shutdown"
    );

    shutdown_tx.send(()).expect("send shutdown");
    let result = tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .expect("server stopped before timeout")
        .expect("server task joined");
    result.expect("graceful server shutdown");

    let rebound = StdTcpListener::bind(addr).expect("port released for immediate rebind");
    drop(rebound);
}

fn hold_loopback_port() -> (StdTcpListener, u16) {
    let listener = bind_free_loopback_listener();
    let port = listener.local_addr().expect("local addr").port();
    (listener, port)
}

fn bind_free_loopback_listener() -> StdTcpListener {
    for port in (server::MIN_PORT..=server::MAX_PORT).rev() {
        if let Ok(listener) = StdTcpListener::bind(("127.0.0.1", port)) {
            return listener;
        }
    }
    panic!("no free loopback port in 3200-3299");
}

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(label: &str) -> Self {
        let id = SCRATCH_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("cortex-{label}-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        Self { dir }
    }

    fn db_path(&self) -> PathBuf {
        self.dir.join("cortex.db")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
