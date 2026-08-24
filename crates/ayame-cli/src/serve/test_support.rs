//! Shared helpers for `serve` unit and in-process endpoint tests.

use std::io::Write as _;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ayame_core::{Document, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::router;
use super::security::NetPolicy;
use super::state::{AppState, SharedState};

/// A unique, isolated directory for one test. Per-test directories prevent
/// parallel save tests from seeing one another's temporary aside files.
pub(crate) fn scratch_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("ayame-serve-test-{}", std::process::id()))
        .join(format!(
            "{}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock before Unix epoch")
                .as_nanos(),
            name.replace(['/', '\\'], "_"),
        ));
    std::fs::create_dir_all(&dir).expect("create test scratch directory");
    dir
}

/// Create a test file in its own isolated directory.
pub(crate) fn scratch_file(name: &str, contents: &[u8]) -> PathBuf {
    let dir = scratch_dir(name);
    scratch_file_in(&dir, name, contents)
}

/// Create a named test file inside an existing scratch directory.
pub(crate) fn scratch_file_in(dir: &Path, name: &str, contents: &[u8]) -> PathBuf {
    let path = dir.join(name);
    let mut file = std::fs::File::create(&path).expect("create test scratch file");
    file.write_all(contents).expect("write test scratch file");
    path
}

/// A unique crash-log cache root for one WAL test.
pub(crate) fn scratch_cache(name: &str) -> PathBuf {
    scratch_dir(&format!("wal-{name}")).join("cache")
}

pub(crate) fn wal_opts(cache: &Path) -> OpenOptions {
    OpenOptions {
        cache_dir: Some(cache.to_path_buf()),
        ..OpenOptions::default()
    }
}

/// Serve the real router (loopback policy) on an ephemeral port.
pub(crate) async fn start_server(path: &Path) -> SocketAddr {
    start_server_with_state(path).await.0
}

/// Start the real router with explicit open options.
pub(crate) async fn start_server_with_opts(path: &Path, opts: OpenOptions) -> SocketAddr {
    start_server_inner(path, opts).await.0
}

/// Start the real router and return its shared state for internal assertions.
pub(crate) async fn start_server_with_state(path: &Path) -> (SocketAddr, SharedState) {
    start_server_inner(path, OpenOptions::default()).await
}

async fn start_server_inner(path: &Path, opts: OpenOptions) -> (SocketAddr, SharedState) {
    let doc = Document::open(path, &opts).expect("open server test document");
    let state = Arc::new(AppState::new(Some(doc), opts));
    let app = router(state.clone(), Arc::new(NetPolicy::loopback()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind server test listener");
    let addr = listener.local_addr().expect("read server test address");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (addr, state)
}

/// Minimal raw HTTP/1.1 client (avoids an additional dev dependency).
pub(crate) async fn send_full(addr: SocketAddr, raw: String) -> String {
    let mut stream = tokio::net::TcpStream::connect(addr)
        .await
        .expect("connect to server test listener");
    stream
        .write_all(raw.as_bytes())
        .await
        .expect("write server test request");
    let mut bytes = Vec::new();
    stream
        .read_to_end(&mut bytes)
        .await
        .expect("read server test response");
    String::from_utf8_lossy(&bytes).to_string()
}

/// Return `(status, body)`, stripping HTTP headers and tolerating chunk
/// framing for tests that only inspect the API payload.
pub(crate) async fn send(addr: SocketAddr, raw: String) -> (u16, String) {
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

pub(crate) fn get(path: &str, host: &str) -> String {
    format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n")
}

pub(crate) fn post_json(path: &str, host: &str, origin: Option<&str>, body: &str) -> String {
    let origin_line = origin
        .map(|origin| format!("Origin: {origin}\r\n"))
        .unwrap_or_default();
    format!(
        "POST {path} HTTP/1.1\r\nHost: {host}\r\n{origin_line}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

pub(crate) fn post_raw(
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
