//! Shared fixtures for the serve unit and endpoint tests.
//!
//! Keeping these here gives every `serve` test module the same isolation
//! contract without making test-only helpers part of the production API.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use ayame_core::{Document, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::router;
use super::security::NetPolicy;
use super::state::{AppState, SharedState};

static FIXTURE_SEQ: AtomicU64 = AtomicU64::new(0);

fn unique_path(kind: &str, name: &str) -> PathBuf {
    let seq = FIXTURE_SEQ.fetch_add(1, Ordering::Relaxed);
    let safe_name = name.replace(['/', '\\'], "_");
    std::env::temp_dir().join(format!(
        "ayame-{kind}-test-{}-{seq}-{safe_name}",
        std::process::id()
    ))
}

/// Create a private directory for one test fixture.
pub(super) fn scratch_dir(name: &str) -> PathBuf {
    let dir = unique_path("serve", name);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Create one file in its own directory, isolated from parallel tests.
pub(super) fn scratch_file(name: &str, contents: &[u8]) -> PathBuf {
    let dir = scratch_dir(name);
    scratch_file_in(&dir, name, contents)
}

/// Create a file inside a caller-owned fixture directory.
pub(super) fn scratch_file_in(dir: &Path, name: &str, contents: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, contents).unwrap();
    path
}

/// A unique cache path for WAL tests. The server creates it on demand.
pub(super) fn scratch_cache(name: &str) -> PathBuf {
    unique_path("wal", name)
}

pub(super) fn wal_opts(cache: &Path) -> OpenOptions {
    OpenOptions {
        cache_dir: Some(cache.to_path_buf()),
        ..OpenOptions::default()
    }
}

/// Serve the real router (loopback policy) on an ephemeral port.
pub(super) async fn start_server(path: &Path) -> SocketAddr {
    start_server_with_state(path).await.0
}

/// Start a fresh state with explicit open options, as a process restart would.
pub(super) async fn start_server_with_opts(path: &Path, opts: OpenOptions) -> SocketAddr {
    let doc = Document::open(path, &opts).unwrap();
    let state = Arc::new(AppState::new(Some(doc), opts));
    serve_state(state).await
}

/// Start a server and return its state for assertions on internal counters.
pub(super) async fn start_server_with_state(path: &Path) -> (SocketAddr, SharedState) {
    let doc = Document::open(path, &OpenOptions::default()).unwrap();
    let state = Arc::new(AppState::new(Some(doc), OpenOptions::default()));
    let addr = serve_state(state.clone()).await;
    (addr, state)
}

async fn serve_state(state: SharedState) -> SocketAddr {
    let app = router(state, Arc::new(NetPolicy::loopback()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    addr
}

/// Minimal raw HTTP/1.1 client; no extra test dependency is needed.
pub(super) async fn send_full(addr: SocketAddr, raw: String) -> String {
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    stream.write_all(raw.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    String::from_utf8_lossy(&buf).to_string()
}

/// Return the status and response body, tolerating chunk framing where tests
/// only inspect the JSON payload.
pub(super) async fn send(addr: SocketAddr, raw: String) -> (u16, String) {
    let text = send_full(addr, raw).await;
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let body = text
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_string())
        .unwrap_or_default();
    (status, body)
}

pub(super) fn get(path: &str, host: &str) -> String {
    format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n")
}

pub(super) fn post_json(path: &str, host: &str, origin: Option<&str>, body: &str) -> String {
    let origin_line = origin
        .map(|origin| format!("Origin: {origin}\r\n"))
        .unwrap_or_default();
    format!(
        "POST {path} HTTP/1.1\r\nHost: {host}\r\n{origin_line}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

pub(super) fn post_raw(
    path: &str,
    host: &str,
    origin: Option<&str>,
    content_type: &str,
    body: &[u8],
) -> String {
    let origin_line = origin
        .map(|origin| format!("Origin: {origin}\r\n"))
        .unwrap_or_default();
    format!(
        "POST {path} HTTP/1.1\r\nHost: {host}\r\n{origin_line}Content-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        String::from_utf8_lossy(body)
    )
}

pub(super) fn response_json(body: &str) -> serde_json::Value {
    serde_json::from_str(body.trim()).unwrap_or_else(|error| {
        panic!("invalid JSON response ({error}): {body}");
    })
}
