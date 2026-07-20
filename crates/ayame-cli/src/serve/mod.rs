//! `ayame serve` — a local web editor for large files.
//!
//! The browser only ever holds the visible viewport; everything else is fetched
//! on demand from these endpoints, which are thin wrappers over `ayame-core`.
//! A `CatchPanicLayer` turns any unexpected panic in a single request into a
//! 500 instead of taking the process down — stability is a feature here.
//!
//! A file may be passed on the command line, or the server can start empty and
//! open one later: `/api/browse` walks the server's filesystem, `/api/open`
//! opens a file by path, and `/api/upload` streams a dropped file to disk and
//! opens it. The active document and its edit overlay live behind a single
//! workspace lock (see [`state`]) and can be swapped at runtime.
//!
//! The server binds loopback by default and refuses other addresses unless
//! `--allow-remote` is given; every request passes the Host/Origin checks in
//! [`security`] (DNS-rebinding and CSRF protection).

use std::net::{SocketAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use axum::extract::{DefaultBodyLimit, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use ayame_core::{Document, FileStat};
use serde::Serialize;
use tower_http::catch_panic::CatchPanicLayer;

use crate::{first_opt, has_flag, open_opts, parse_checked};

mod analysis;
mod assets;
mod edit;
mod error;
mod markers;
mod ops;
mod security;
mod state;
#[cfg(feature = "typegen")]
pub(crate) mod typegen;
pub(crate) mod workspace;

use error::ApiError;
use security::NetPolicy;
use state::{AppState, SharedState, TabsResponse, TailStatus, UiState};

/// Hard cap on lines returned in one viewport request, so a hostile/buggy
/// client can never ask us to materialize the whole file.
const MAX_VIEW: u64 = 20_000;

pub fn cmd_serve(args: &[String]) -> Result<()> {
    let (pos, opts, flags) = parse_checked(
        args,
        &[
            "--encoding",
            "--stride",
            "--host",
            "--port",
            "--cache-dir",
            "--scratch-dir",
        ],
        &["--no-cache", "--allow-remote"],
    )?;
    let host = first_opt(&opts, &["--host"])
        .unwrap_or("127.0.0.1")
        .to_string();
    let port: u16 = first_opt(&opts, &["--port"])
        .unwrap_or("8777")
        .parse()
        .context("--port must be a number")?;

    let allow_remote = has_flag(&flags, &["--allow-remote"]);
    let loopback = security::host_is_loopback(&host);
    if !loopback && !allow_remote {
        bail!(
            "refusing to bind non-loopback address '{host}': the editor gives \
             unauthenticated read/write access to this machine's files. \
             Re-run with --allow-remote if you really mean to expose it, \
             and only on a network you trust."
        );
    }
    let remote_active = allow_remote && !loopback;
    let policy = Arc::new(if remote_active {
        NetPolicy::remote(&host)
    } else {
        NetPolicy::loopback()
    });

    let state = Arc::new(build_state(&pos, &opts, &flags)?);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("building tokio runtime")?;
    rt.block_on(serve(state, host, port, policy, remote_active))
}

/// Open the optional FILE argument (if any) and assemble the shared app state.
/// Shared by `serve` and the native GUI so both open files the same way.
pub(crate) fn build_state(
    pos: &[String],
    opts: &std::collections::HashMap<String, String>,
    flags: &std::collections::HashSet<String>,
) -> Result<AppState> {
    configure_scratch(opts);
    // Reap scratch that crashed/killed prior sessions left behind before we
    // start piling on our own (#138). Safe: only dead PIDs' dirs are removed.
    crate::temp_paths::sweep_stale_scratch();
    let open_options = open_opts(opts, flags)?;
    let doc = match pos.first() {
        Some(path) => {
            eprintln!("ayame: opening and indexing '{path}' …");
            let doc =
                Document::open(path, &open_options).with_context(|| format!("opening '{path}'"))?;
            let s = doc.stat();
            let how = if s.from_cache {
                "loaded from cache"
            } else {
                "indexed"
            };
            eprintln!(
                "ayame: {} lines, {} bytes, {} — {} in {} ms ({} checkpoints, {} bytes resident)",
                crate::commas(s.lines),
                crate::commas(s.bytes),
                s.encoding.label(),
                how,
                s.index_ms,
                crate::commas(s.checkpoints as u64),
                crate::commas(s.index_bytes as u64),
            );
            Some(doc)
        }
        None => {
            eprintln!("ayame: no file open yet — open one (drag & drop or Open)");
            None
        }
    };
    Ok(AppState::new(doc, open_options))
}

/// Pin the scratch/spill base for this process from the flags, so worker
/// materialization and sort spill land on disk, not tmpfs (#140). Explicit
/// `--scratch-dir` wins; otherwise `--cache-dir/scratch` keeps scratch on the
/// same (disk-backed) volume as the index cache. With neither, the lazy
/// default in `temp_paths` applies (`AYAME_SCRATCH_DIR`, else the per-user
/// cache root, else the OS temp dir).
fn configure_scratch(opts: &std::collections::HashMap<String, String>) {
    if let Some(dir) = first_opt(opts, &["--scratch-dir"]) {
        crate::temp_paths::set_scratch_base(std::path::PathBuf::from(dir));
    } else if let Some(cache) = first_opt(opts, &["--cache-dir"]) {
        crate::temp_paths::set_scratch_base(std::path::Path::new(cache).join("scratch"));
    }
}

/// Build the axum router. Every endpoint is a thin wrapper over `ayame-core`;
/// the same router serves both the CLI `serve` and the native GUI window.
fn router(state: SharedState, policy: Arc<NetPolicy>) -> Router {
    Router::new()
        .route("/", get(assets::index))
        .route("/src/{*path}", get(assets::src_module))
        .route("/style.css", get(assets::style_css))
        .route("/favicon.svg", get(assets::favicon_svg))
        .route("/ayame-logo.svg", get(assets::ayame_logo_svg))
        .route("/iris-watercolor.png", get(assets::iris_watercolor_png))
        .route("/api/stat", get(api_stat))
        .route("/api/tail/poll", post(api_tail_poll))
        .route("/api/open", post(workspace::api_open))
        .route("/api/new", post(workspace::api_new))
        .route("/api/tabs", get(workspace::api_tabs))
        .route("/api/tabs/select", post(workspace::api_tabs_select))
        .route("/api/tabs/close", post(workspace::api_tabs_close))
        .route("/api/tabs/reorder", post(workspace::api_tabs_reorder))
        .route("/api/tabs/detach", post(workspace::api_tabs_detach))
        .route("/api/ui_state", get(workspace::api_ui_state))
        .route("/api/ui_state", post(workspace::api_ui_state_save))
        .route("/api/session/save", post(workspace::api_session_save))
        .route("/api/session/restore", post(workspace::api_session_restore))
        .route("/api/browse", get(workspace::api_browse))
        .route(
            "/api/upload",
            post(workspace::api_upload)
                // Defence in depth only: this layer is consulted by axum's
                // buffering extractors, NOT by the raw streaming handler,
                // which enforces the same finite cap itself.
                .layer(DefaultBodyLimit::max(
                    workspace::MAX_UPLOAD_BYTES.min(usize::MAX as u64) as usize,
                )),
        )
        .route("/api/lines", get(edit::api_lines))
        .route(
            "/api/edit/replace_range",
            post(edit::api_edit_replace_range),
        )
        .route(
            "/api/edit/replace_batch",
            post(edit::api_edit_replace_batch),
        )
        .route("/api/edit/replace_rect", post(edit::api_edit_replace_rect))
        .route("/api/edit/save", post(edit::api_edit_save))
        .route("/api/selection/save", post(edit::api_selection_save))
        .route("/api/edit/undo", post(edit::api_edit_undo))
        .route("/api/edit/redo", post(edit::api_edit_redo))
        .route("/api/edit/revert", post(edit::api_edit_revert))
        .route("/api/edit/recover", post(edit::api_edit_recover))
        .route("/api/reopen_encoding", post(edit::api_reopen_encoding))
        .route("/api/markers", get(markers::api_markers))
        .route("/api/markers/previews", get(markers::api_marker_previews))
        .route("/api/markers/navigate", get(markers::api_marker_navigate))
        .route("/api/markers/toggle", post(markers::api_marker_toggle))
        .route("/api/markers/add", post(markers::api_marker_add))
        .route("/api/markers/clear", post(markers::api_marker_clear))
        .route("/api/markers/save", post(markers::api_marker_save))
        .route("/api/analysis/start", post(analysis::api_analysis_start))
        .route("/api/analysis/status", get(analysis::api_analysis_status))
        .route("/api/analysis/cancel", post(analysis::api_analysis_cancel))
        .route(
            "/api/analysis/navigate",
            get(analysis::api_analysis_navigate),
        )
        .route("/api/analysis/hits", get(analysis::api_analysis_hits))
        .route("/api/analysis/tail", post(analysis::api_analysis_tail))
        .route("/api/ops/status", get(ops::api_operation_status))
        .route("/api/ops/cancel", post(ops::api_operation_cancel))
        .route("/api/sort/save", post(ops::api_sort_save))
        .route("/api/replace/save", post(ops::api_replace_save))
        .route("/api/case/save", post(ops::api_case_save))
        .route("/api/grep/save", post(ops::api_grep_save))
        .route("/api/split/save", post(ops::api_split_save))
        .route("/api/search", get(ops::api_search))
        .route("/api/grep", post(ops::api_grep))
        .route("/api/find", get(ops::api_find))
        .route("/api/linebyte", get(ops::api_linebyte))
        .layer(axum::middleware::from_fn(security::harden_response))
        .layer(axum::middleware::from_fn_with_state(
            policy,
            security::guard,
        ))
        .layer(CatchPanicLayer::new())
        .with_state(state)
}

/// Crash-log policy loop: every ~3 s fsync the live log (power-loss safety on
/// top of the per-commit OS flush), compact it past its size threshold, and
/// surface deferred write errors through the next stat response. A plain
/// named thread holding only a `Weak` on the state — it needs no async
/// runtime (shared by `serve` and the GUI's background server) and exits on
/// its own once the state is dropped.
fn spawn_wal_policy(state: &SharedState) {
    let weak = Arc::downgrade(state);
    let _ = std::thread::Builder::new()
        .name("ayame-wal-policy".into())
        .spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(3));
            let Some(state) = weak.upgrade() else { break };
            state.wal_policy_tick();
        });
}

async fn serve(
    state: SharedState,
    host: String,
    port: u16,
    policy: Arc<NetPolicy>,
    remote_active: bool,
) -> Result<()> {
    spawn_wal_policy(&state);
    let app = router(state.clone(), policy);
    // Resolve rather than parse: the loopback policy blesses the NAME
    // "localhost", so binding must accept it too (SocketAddr::from_str takes
    // only IP literals). Bare IPv6 literals like `::1` also come out right.
    let addr = resolve_bind_addr(&host, port)?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    if remote_active {
        eprintln!("ayame: ================================================================");
        eprintln!("ayame: WARNING: --allow-remote is enabled.");
        eprintln!("ayame: The editor is reachable from the network at {addr} with");
        eprintln!("ayame: NO authentication: anyone who can connect can read and write");
        eprintln!("ayame: files as this user. Use only on a network you trust.");
        eprintln!("ayame: ================================================================");
    }
    eprintln!("ayame: editor ready at http://{addr}/  (Ctrl+C to stop)");

    let result = axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
            eprintln!("\nayame: shutting down");
        })
        .await
        .context("server error");
    cleanup_session(&state);
    result
}

/// Drop everything this process accumulated on disk: uploads, untitled
/// buffers, unsaved sort results, in-place save aside files, and the crash
/// logs of CLEAN sessions (dirty ones stay — they are the recovery artifact
/// the next process offers to replay). Called on CLI `serve` graceful
/// shutdown AND on GUI window close, so the desktop mode stops leaking
/// scratch every session (#138).
pub(crate) fn cleanup_session(state: &SharedState) {
    state.cleanup_wal_files();
    state.cleanup_aside_files();
    workspace::cleanup_temp_dirs();
}

fn resolve_bind_addr(host: &str, port: u16) -> Result<SocketAddr> {
    (host, port)
        .to_socket_addrs()
        .with_context(|| format!("invalid host/port '{host}:{port}'"))?
        .next()
        .with_context(|| format!("host '{host}' resolved to no addresses"))
}

/// Start the editor server on an ephemeral loopback port in a dedicated
/// background thread (with its own Tokio runtime) and return the bound address.
///
/// The native GUI uses this: the window's event loop owns the main thread
/// (required on macOS), while the server runs behind it. The thread and its
/// short-lived worker children are torn down when the process exits.
#[cfg(feature = "gui")]
pub(crate) fn spawn_background(state: SharedState) -> Result<SocketAddr> {
    use std::sync::mpsc;

    spawn_wal_policy(&state);
    let (tx, rx) = mpsc::channel::<Result<SocketAddr>>();
    std::thread::Builder::new()
        .name("ayame-server".into())
        .spawn(move || {
            // The only client is our own webview and heavy work is isolated in
            // child processes, so a couple of workers is plenty — no need to
            // park one thread per core behind a desktop window.
            let rt = match tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .context("building tokio runtime")
            {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = tx.send(Err(e));
                    return;
                }
            };
            rt.block_on(async move {
                let addr: SocketAddr = ([127, 0, 0, 1], 0).into();
                let listener = match tokio::net::TcpListener::bind(addr)
                    .await
                    .with_context(|| format!("binding {addr}"))
                {
                    Ok(l) => l,
                    Err(e) => {
                        let _ = tx.send(Err(e));
                        return;
                    }
                };
                let local = match listener.local_addr().context("resolving local addr") {
                    Ok(a) => a,
                    Err(e) => {
                        let _ = tx.send(Err(e));
                        return;
                    }
                };
                // Hand the resolved address back before we start serving.
                if tx.send(Ok(local)).is_err() {
                    return; // the GUI gave up waiting; nothing to serve
                }
                let _ = axum::serve(listener, router(state, Arc::new(NetPolicy::loopback()))).await;
            });
        })
        .context("spawning server thread")?;

    rx.recv().context("server thread died before binding")?
}

// ---- API ----------------------------------------------------------------------

#[derive(Serialize)]
pub(super) struct StatResponse {
    /// Whether a file is currently open. When false, every `file` field is
    /// absent and the front-end shows its open/welcome screen.
    open: bool,
    #[serde(flatten)]
    file: Option<FileStat>,
    view_lines: u64,
    dirty: bool,
    revision: u64,
    inserted_lines: u64,
    replaced_lines: u64,
    deleted_lines: u64,
    can_undo: bool,
    can_redo: bool,
    /// Present when the active document's crash log holds `n` unsaved edit
    /// transactions from a previous process, waiting for a restore/discard
    /// decision via `POST /api/edit/recover`.
    #[serde(skip_serializing_if = "Option::is_none")]
    recoverable: Option<usize>,
    /// One-shot: crash logging failed and was disabled for this session (the
    /// front-end shows it once; editing itself is unaffected).
    #[serde(skip_serializing_if = "Option::is_none")]
    wal_error: Option<String>,
}

pub(super) fn stat_response(state: &AppState) -> StatResponse {
    // Drained once: whichever stat answers next carries the warning.
    let wal_error = state.take_wal_error();
    // One read acquisition: doc and edits are mutually consistent and nothing
    // (in particular not the undo history) is cloned.
    state.read(|ws| match ws.doc_and_edits() {
        Ok((doc, edits)) => {
            let edit = edits.stats(doc);
            let mut file = doc.stat();
            // UI-facing path: never leak a Windows verbatim prefix.
            file.path = workspace::strip_verbatim(&file.path);
            StatResponse {
                open: true,
                file: Some(file),
                view_lines: edit.total_lines,
                dirty: edit.dirty,
                revision: edit.revision,
                inserted_lines: edit.inserted_lines,
                replaced_lines: edit.replaced_lines,
                deleted_lines: edit.deleted_lines,
                can_undo: edit.can_undo,
                can_redo: edit.can_redo,
                recoverable: ws.recoverable(),
                wal_error,
            }
        }
        Err(_) => StatResponse {
            open: false,
            file: None,
            view_lines: 0,
            dirty: false,
            revision: 0,
            inserted_lines: 0,
            replaced_lines: 0,
            deleted_lines: 0,
            can_undo: false,
            can_redo: false,
            recoverable: None,
            wal_error,
        },
    })
}

async fn api_stat(State(state): State<SharedState>) -> Json<StatResponse> {
    Json(stat_response(&state))
}

/// `tail -f`: poll the active document for appended data. Growth extends the
/// line index incrementally over just the new bytes (see
/// [`Document::refresh_tail`]); a shrink/replacement is reported as `changed`.
/// The stat + mmap + tail scan are blocking, so they run off the async runtime.
async fn api_tail_poll(State(state): State<SharedState>) -> Json<TailStatus> {
    let status = tokio::task::spawn_blocking(move || state.poll_tail())
        .await
        .unwrap_or_else(|_| TailStatus::closed());
    Json(status)
}

fn bad_request(e: impl std::fmt::Display) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, "bad_request", e.to_string())
}

fn internal(e: impl std::fmt::Display) -> ApiError {
    ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string())
}

fn default_save_copy_path(path: &Path) -> PathBuf {
    default_suffix_path(path, "edited")
}

fn default_suffix_path(path: &Path, suffix: &str) -> PathBuf {
    let base = PathBuf::from(format!("{}.{}", path.display(), suffix));
    if !base.exists() {
        return base;
    }
    for n in 1..1000 {
        let p = PathBuf::from(format!("{}.{}.{n}", path.display(), suffix));
        if !p.exists() {
            return p;
        }
    }
    PathBuf::from(format!(
        "{}.{}.{}",
        path.display(),
        suffix,
        std::process::id()
    ))
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use ayame_core::OpenOptions;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    static UPLOAD_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    fn scratch_file(name: &str, contents: &[u8]) -> PathBuf {
        // Each file gets its OWN unique subdirectory. Tests run concurrently and
        // some scan `f.parent()` for leftover `.ayame-prev-*` save artifacts; a
        // shared directory would let one test see another's in-flight save files
        // and fail spuriously (a real CI flake we hit). Per-file dirs keep every
        // directory listing isolated to the test that owns it.
        let dir = std::env::temp_dir()
            .join(format!("ayame-serve-test-{}", std::process::id()))
            .join(format!(
                "{}-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                name.replace(['/', '\\'], "_"),
            ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(contents).unwrap();
        path
    }

    /// Serve the real router (loopback policy) on an ephemeral port.
    async fn start_server(path: &Path) -> SocketAddr {
        start_server_with_state(path).await.0
    }

    /// Like [`start_server`] but with explicit open options — the WAL tests
    /// need a cache dir, which `OpenOptions::default()` deliberately lacks.
    /// Each call builds a brand-new `AppState` over the same file and cache,
    /// which is exactly what a process restart after a crash looks like to
    /// the recovery machinery.
    async fn start_server_with_opts(path: &Path, opts: OpenOptions) -> SocketAddr {
        let doc = Document::open(path, &opts).unwrap();
        let state = Arc::new(AppState::new(Some(doc), opts));
        let app = router(state.clone(), Arc::new(NetPolicy::loopback()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        addr
    }

    /// A unique crash-log cache root for one WAL test.
    fn scratch_cache(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "ayame-wal-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ))
    }

    fn wal_opts(cache: &Path) -> OpenOptions {
        OpenOptions {
            cache_dir: Some(cache.to_path_buf()),
            ..OpenOptions::default()
        }
    }

    /// Like [`start_server`], but also hands back the shared state so a test
    /// can assert on internals (e.g. how often the dirty snapshot was built).
    async fn start_server_with_state(path: &Path) -> (SocketAddr, SharedState) {
        let doc = Document::open(path, &OpenOptions::default()).unwrap();
        let state = Arc::new(AppState::new(Some(doc), OpenOptions::default()));
        let app = router(state.clone(), Arc::new(NetPolicy::loopback()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (addr, state)
    }

    /// Minimal raw HTTP/1.1 client (avoids extra dev-dependencies).
    async fn send_full(addr: SocketAddr, raw: String) -> String {
        let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
        s.write_all(raw.as_bytes()).await.unwrap();
        let mut buf = Vec::new();
        s.read_to_end(&mut buf).await.unwrap();
        String::from_utf8_lossy(&buf).to_string()
    }

    /// Return (status, body-ish text), stripping headers and tolerating chunk
    /// framing for tests that only care about the API payload.
    async fn send(addr: SocketAddr, raw: String) -> (u16, String) {
        let text = send_full(addr, raw).await;
        let status = text
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let body = text
            .split_once("\r\n\r\n")
            .map(|(_, b)| b.to_string())
            .unwrap_or_default();
        (status, body)
    }

    fn get(path: &str, host: &str) -> String {
        format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n")
    }

    fn post_json(path: &str, host: &str, origin: Option<&str>, body: &str) -> String {
        let origin_line = origin
            .map(|o| format!("Origin: {o}\r\n"))
            .unwrap_or_default();
        format!(
            "POST {path} HTTP/1.1\r\nHost: {host}\r\n{origin_line}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn post_raw(
        path: &str,
        host: &str,
        origin: Option<&str>,
        content_type: &str,
        body: &[u8],
    ) -> String {
        let origin_line = origin
            .map(|o| format!("Origin: {o}\r\n"))
            .unwrap_or_default();
        format!(
            "POST {path} HTTP/1.1\r\nHost: {host}\r\n{origin_line}Content-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            String::from_utf8_lossy(body)
        )
    }

    fn response_json(body: &str) -> serde_json::Value {
        serde_json::from_str(body.trim()).unwrap_or_else(|error| {
            panic!("invalid JSON response ({error}): {body}");
        })
    }

    fn analysis_profile_json() -> serde_json::Value {
        serde_json::json!({
            "id": "test-logs",
            "name": "Test logs",
            "file_glob": "*.log",
            "rules": [
                {
                    "id": "error",
                    "name": "ERROR",
                    "pattern": "ERROR",
                    "regex": false,
                    "case_sensitive": true,
                    "whole_word": true,
                    "color": "danger",
                    "enabled": true
                },
                {
                    "id": "warn",
                    "name": "WARN",
                    "pattern": "WARN",
                    "regex": false,
                    "case_sensitive": true,
                    "whole_word": true,
                    "color": "warn",
                    "enabled": true
                },
                {
                    "id": "request",
                    "name": "Request",
                    "pattern": "request(?:_id)?=[a-z0-9]+",
                    "regex": true,
                    "case_sensitive": false,
                    "whole_word": false,
                    "color": "link",
                    "enabled": true
                }
            ]
        })
    }

    async fn wait_for_analysis(addr: SocketAddr, host: &str, id: &str) -> serde_json::Value {
        for _ in 0..300 {
            let path = format!("/api/analysis/status?id={id}");
            let (status, body) = send(addr, get(&path, host)).await;
            assert_eq!(status, 200, "body: {body}");
            let json = response_json(&body);
            if !matches!(json["phase"].as_str(), Some("scanning" | "updating")) {
                return json;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("analysis {id} did not finish");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn responses_include_browser_security_headers() {
        let f = scratch_file("security-headers.txt", b"hello\n");
        let addr = start_server(&f).await;
        let host = format!("127.0.0.1:{}", addr.port());

        let response = send_full(addr, get("/", &host)).await;
        let headers = response
            .split_once("\r\n\r\n")
            .map(|(head, _)| head.to_ascii_lowercase())
            .unwrap_or_default();
        assert!(
            headers.contains("content-security-policy: default-src 'self'"),
            "headers: {headers}"
        );
        assert!(
            headers.contains("frame-ancestors 'none'"),
            "headers: {headers}"
        );
        assert!(
            headers.contains("x-content-type-options: nosniff"),
            "headers: {headers}"
        );
        assert!(
            headers.contains("x-frame-options: deny"),
            "headers: {headers}"
        );
        assert!(
            headers.contains("referrer-policy: no-referrer"),
            "headers: {headers}"
        );

        let _ = std::fs::remove_file(&f);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn sparse_bookmarks_follow_edits_undo_redo_and_viewport_queries() {
        let f = scratch_file("bookmarks.txt", b"zero\none\ntwo\nthree\n");
        let addr = start_server(&f).await;
        let host = format!("127.0.0.1:{}", addr.port());
        let origin = format!("http://{host}");

        let (status, body) = send(
            addr,
            post_json(
                "/api/markers/toggle",
                &host,
                Some(&origin),
                r#"{"kind":"bookmark","line":2}"#,
            ),
        )
        .await;
        assert_eq!(status, 200, "body: {body}");
        assert!(body.contains("\"marked\":true"), "body: {body}");
        assert!(body.contains("\"count\":1"), "body: {body}");

        // The viewport returns only its sparse marker sidecar, never a
        // document-sized bitmap.
        let (status, body) = send(addr, get("/api/lines?start=2&count=1", &host)).await;
        assert_eq!(status, 200, "body: {body}");
        assert!(
            body.contains(r#""markers":[{"kind":"bookmark","line":2}]"#),
            "body: {body}"
        );

        // Inserting one line at the top moves the bookmark with its content.
        let (status, body) = send(
            addr,
            post_json(
                "/api/edit/replace_range",
                &host,
                Some(&origin),
                r#"{"l0":0,"c0":0,"l1":0,"c1":0,"text":"head\n"}"#,
            ),
        )
        .await;
        assert_eq!(status, 200, "body: {body}");

        let (status, body) = send(
            addr,
            get("/api/markers?kind=bookmark&start=0&limit=20", &host),
        )
        .await;
        assert_eq!(status, 200, "body: {body}");
        assert!(body.contains("\"lines\":[3]"), "body: {body}");

        let (status, body) =
            send(addr, post_json("/api/edit/undo", &host, Some(&origin), "")).await;
        assert_eq!(status, 200, "body: {body}");
        let (_, body) = send(
            addr,
            get("/api/markers?kind=bookmark&start=0&limit=20", &host),
        )
        .await;
        assert!(body.contains("\"lines\":[2]"), "body: {body}");

        let (status, body) =
            send(addr, post_json("/api/edit/redo", &host, Some(&origin), "")).await;
        assert_eq!(status, 200, "body: {body}");
        let (_, body) = send(
            addr,
            get(
                "/api/markers/navigate?kind=bookmark&from=3&direction=next&wrap=true",
                &host,
            ),
        )
        .await;
        assert!(body.contains("\"line\":3"), "body: {body}");
        assert!(body.contains("\"wrapped\":true"), "body: {body}");

        let (status, body) = send(
            addr,
            get(
                "/api/markers/previews?kind=bookmark&start=0&limit=20",
                &host,
            ),
        )
        .await;
        assert_eq!(status, 200, "body: {body}");
        assert!(body.contains("\"line\":3"), "body: {body}");
        assert!(body.contains("\"text\":\"two\""), "body: {body}");

        let (status, body) = send(
            addr,
            post_json(
                "/api/markers/clear",
                &host,
                Some(&origin),
                r#"{"kind":"bookmark"}"#,
            ),
        )
        .await;
        assert_eq!(status, 200, "body: {body}");
        assert!(body.contains("\"count\":0"), "body: {body}");

        let (status, body) = send(
            addr,
            post_json(
                "/api/markers/add",
                &host,
                Some(&origin),
                r#"{"kind":"bookmark","lines":[0,1,1]}"#,
            ),
        )
        .await;
        assert_eq!(status, 200, "body: {body}");
        assert!(body.contains("\"added\":2"), "body: {body}");
        assert!(body.contains("\"count\":2"), "body: {body}");

        let exported = f.parent().unwrap().join("bookmarks-export.txt");
        let request = serde_json::json!({
            "kind": "bookmark",
            "path": exported.to_string_lossy(),
            "overwrite": false
        })
        .to_string();
        let (status, body) = send(
            addr,
            post_json("/api/markers/save", &host, Some(&origin), &request),
        )
        .await;
        assert_eq!(status, 200, "body: {body}");
        assert!(body.contains("\"lines\":2"), "body: {body}");
        assert_eq!(std::fs::read(&exported).unwrap(), b"head\nzero");

        let (status, _) = send(
            addr,
            post_json(
                "/api/markers/toggle",
                &host,
                Some(&origin),
                r#"{"kind":"bookmark","line":18446744073709551615}"#,
            ),
        )
        .await;
        assert_eq!(status, 400);

        let _ = std::fs::remove_file(&f);
        let _ = std::fs::remove_file(exported);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stat_answers_and_rebinding_host_is_blocked() {
        let f = scratch_file("stat.txt", b"hello\nworld\n");
        let addr = start_server(&f).await;
        let host = format!("127.0.0.1:{}", addr.port());

        let (status, body) = send(addr, get("/api/stat", &host)).await;
        assert_eq!(status, 200, "body: {body}");
        assert!(body.contains("\"open\":true"), "body: {body}");
        assert!(body.contains("\"lines\":2"), "body: {body}");

        // Same endpoint through a foreign Host: DNS-rebinding protection.
        let (status, _) = send(addr, get("/api/stat", "evil.com")).await;
        assert_eq!(status, 403);
        // localhost with any port is us.
        let (status, _) = send(addr, get("/api/stat", "localhost:1")).await;
        assert_eq!(status, 200);

        let _ = std::fs::remove_file(&f);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn localhost_bind_name_resolves_to_a_bindable_socket() {
        let addr = resolve_bind_addr("localhost", 0).unwrap();
        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
        let bound = listener.local_addr().unwrap();
        assert!(bound.ip().is_loopback(), "bound non-loopback addr: {bound}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tail_poll_follows_appended_data() {
        use std::io::Write as _;
        let f = scratch_file("tail.log", b"line 0\nline 1\n");
        let addr = start_server(&f).await;
        let host = format!("127.0.0.1:{}", addr.port());
        let origin = format!("http://{host}");

        // No growth yet: grew=false, current totals reported.
        let (status, body) =
            send(addr, post_json("/api/tail/poll", &host, Some(&origin), "")).await;
        assert_eq!(status, 200, "body: {body}");
        assert!(body.contains("\"grew\":false"), "body: {body}");
        assert!(body.contains("\"lines\":2"), "body: {body}");

        // Append two lines out-of-band, then poll: the index extends in place.
        {
            let mut w = std::fs::OpenOptions::new().append(true).open(&f).unwrap();
            w.write_all(b"line 2\nline 3\n").unwrap();
            w.flush().unwrap();
        }
        let (status, body) =
            send(addr, post_json("/api/tail/poll", &host, Some(&origin), "")).await;
        assert_eq!(status, 200, "body: {body}");
        assert!(body.contains("\"grew\":true"), "body: {body}");
        assert!(body.contains("\"lines\":4"), "body: {body}");

        // The new lines are now served through the normal viewport endpoint.
        let (_, lines) = send(addr, get("/api/lines?start=0&count=10", &host)).await;
        assert!(lines.contains("line 3"), "body: {lines}");

        // Truncating the file signals an external change (client should reopen).
        std::fs::write(&f, b"reset\n").unwrap();
        let (status, body) =
            send(addr, post_json("/api/tail/poll", &host, Some(&origin), "")).await;
        assert_eq!(status, 200, "body: {body}");
        assert!(body.contains("\"changed\":true"), "body: {body}");

        let _ = std::fs::remove_file(&f);
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tail_poll_detects_rename_rotation_even_when_old_inode_did_not_shrink() {
        let f = scratch_file("tail-rotation.log", b"old 0\nold 1\n");
        let rotated = f.with_extension("log.1");
        let addr = start_server(&f).await;
        let host = format!("127.0.0.1:{}", addr.port());
        let origin = format!("http://{host}");

        std::fs::rename(&f, &rotated).unwrap();
        std::fs::write(&f, b"new file is deliberately longer\n").unwrap();
        let (status, body) =
            send(addr, post_json("/api/tail/poll", &host, Some(&origin), "")).await;
        assert_eq!(status, 200, "body: {body}");
        assert!(body.contains("\"changed\":true"), "body: {body}");
        assert!(body.contains("\"grew\":false"), "body: {body}");

        let _ = std::fs::remove_file(&f);
        let _ = std::fs::remove_file(&rotated);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tail_poll_does_not_write_a_new_index_cache_per_append() {
        fn idx_count(cache: &Path) -> usize {
            std::fs::read_dir(cache.join("v1"))
                .map(|it| {
                    it.filter_map(Result::ok)
                        .filter(|e| e.path().extension().is_some_and(|ext| ext == "idx"))
                        .count()
                })
                .unwrap_or(0)
        }

        let mut data = Vec::new();
        while data.len() < 4 * 1024 * 1024 + 1024 {
            data.extend_from_slice(b"line 0 payload payload payload payload\n");
        }
        let f = scratch_file("tail-cache.log", &data);
        let cache = scratch_cache("tail-cache");
        let addr = start_server_with_opts(
            &f,
            OpenOptions {
                cache_dir: Some(cache.clone()),
                ..OpenOptions::default()
            },
        )
        .await;
        let host = format!("127.0.0.1:{}", addr.port());
        let origin = format!("http://{host}");
        assert_eq!(idx_count(&cache), 1);

        {
            let mut w = std::fs::OpenOptions::new().append(true).open(&f).unwrap();
            w.write_all(b"line appended\n").unwrap();
            w.flush().unwrap();
        }
        let (status, body) =
            send(addr, post_json("/api/tail/poll", &host, Some(&origin), "")).await;
        assert_eq!(status, 200, "body: {body}");
        assert!(body.contains("\"grew\":true"), "body: {body}");
        assert_eq!(idx_count(&cache), 1);

        let _ = std::fs::remove_file(&f);
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn multi_rule_analysis_is_exact_capped_navigable_and_stale_after_edit() {
        let f = scratch_file(
            "analysis.log",
            b"ERROR request=abc\nWARN request=abc\nINFO request=xyz\nERROR request=abc\n",
        );
        let addr = start_server(&f).await;
        let host = format!("127.0.0.1:{}", addr.port());
        let origin = format!("http://{host}");
        let request = serde_json::json!({
            "profile": analysis_profile_json(),
            "max_hits_per_rule": 1
        })
        .to_string();
        let (status, body) = send(
            addr,
            post_json("/api/analysis/start", &host, Some(&origin), &request),
        )
        .await;
        assert_eq!(status, 200, "body: {body}");
        let id = response_json(&body)["id"].as_str().unwrap().to_string();
        let result = wait_for_analysis(addr, &host, &id).await;
        assert_eq!(result["phase"], "complete");
        let rules = result["rules"].as_array().unwrap();
        let by_id = |rule_id: &str| rules.iter().find(|rule| rule["id"] == rule_id).unwrap();
        assert_eq!(by_id("error")["count"], 2);
        assert_eq!(by_id("error")["stored_hits"], 1);
        assert_eq!(by_id("error")["truncated"], true);
        assert_eq!(by_id("warn")["count"], 1);
        assert_eq!(by_id("request")["count"], 4);
        assert_eq!(
            by_id("request")["histogram"].as_array().unwrap().len(),
            ayame_core::ANALYSIS_HISTOGRAM_BINS
        );

        let (status, body) = send(
            addr,
            get(
                &format!("/api/analysis/navigate?id={id}&rule=error&direction=next&from=0"),
                &host,
            ),
        )
        .await;
        assert_eq!(status, 200, "body: {body}");
        assert_eq!(response_json(&body)["hit"]["line"], 0);

        let (status, body) = send(
            addr,
            get(
                &format!("/api/analysis/hits?id={id}&rule=error&start=0&limit=10"),
                &host,
            ),
        )
        .await;
        assert_eq!(status, 200, "body: {body}");
        let hits = response_json(&body);
        assert_eq!(hits["total_count"], 2);
        assert_eq!(hits["stored_hits"], 1);
        assert_eq!(hits["truncated"], true);

        let (status, body) = send(
            addr,
            post_json(
                "/api/edit/replace_range",
                &host,
                Some(&origin),
                r#"{"l0":0,"c0":0,"l1":0,"c1":0,"text":"X"}"#,
            ),
        )
        .await;
        assert_eq!(status, 200, "body: {body}");
        let (_, body) = send(addr, get(&format!("/api/analysis/status?id={id}"), &host)).await;
        assert_eq!(response_json(&body)["phase"], "stale");

        let _ = std::fs::remove_dir_all(f.parent().unwrap());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn analysis_tail_rescans_only_previous_final_line_and_append() {
        let f = scratch_file("analysis-tail.log", b"ERROR first\npartial");
        let addr = start_server(&f).await;
        let host = format!("127.0.0.1:{}", addr.port());
        let origin = format!("http://{host}");
        let mut profile = analysis_profile_json();
        profile["rules"][2]["id"] = "partial".into();
        profile["rules"][2]["name"] = "Partial".into();
        profile["rules"][2]["pattern"] = "partial".into();
        profile["rules"][2]["regex"] = false.into();
        profile["rules"][2]["case_sensitive"] = true.into();
        let request = serde_json::json!({
            "profile": profile,
            "max_hits_per_rule": 10
        })
        .to_string();
        let (status, body) = send(
            addr,
            post_json("/api/analysis/start", &host, Some(&origin), &request),
        )
        .await;
        assert_eq!(status, 200, "body: {body}");
        let id = response_json(&body)["id"].as_str().unwrap().to_string();
        let initial = wait_for_analysis(addr, &host, &id).await;
        assert_eq!(initial["phase"], "complete");

        {
            let mut writer = std::fs::OpenOptions::new().append(true).open(&f).unwrap();
            writer.write_all(b" ERROR\nWARN\n").unwrap();
            writer.flush().unwrap();
        }
        let (status, body) =
            send(addr, post_json("/api/tail/poll", &host, Some(&origin), "")).await;
        assert_eq!(status, 200, "body: {body}");
        assert_eq!(response_json(&body)["grew"], true);

        let (_, body) = send(addr, get(&format!("/api/analysis/status?id={id}"), &host)).await;
        assert_eq!(response_json(&body)["tail_pending"], true);
        let (status, body) = send(
            addr,
            post_json(
                "/api/analysis/tail",
                &host,
                Some(&origin),
                &serde_json::json!({ "id": id }).to_string(),
            ),
        )
        .await;
        assert_eq!(status, 200, "body: {body}");
        let result = response_json(&body);
        assert_eq!(result["phase"], "complete");
        assert_eq!(result["tail_pending"], false);
        let rules = result["rules"].as_array().unwrap();
        let count = |rule_id: &str| {
            rules.iter().find(|rule| rule["id"] == rule_id).unwrap()["count"]
                .as_u64()
                .unwrap()
        };
        assert_eq!(count("error"), 2);
        assert_eq!(count("warn"), 1);
        assert_eq!(count("partial"), 1);

        let _ = std::fs::remove_dir_all(f.parent().unwrap());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn large_synthetic_analysis_keeps_fixed_histograms_and_sparse_hits() {
        const LINES: usize = 100_000;
        let mut data = Vec::with_capacity(LINES * 32);
        for index in 0..LINES {
            writeln!(&mut data, "ERROR WARN request_id={index:x}").unwrap();
        }
        let f = scratch_file("analysis-large.log", &data);
        let addr = start_server(&f).await;
        let host = format!("127.0.0.1:{}", addr.port());
        let origin = format!("http://{host}");
        let request = serde_json::json!({
            "profile": analysis_profile_json(),
            "max_hits_per_rule": 7
        })
        .to_string();
        let (status, body) = send(
            addr,
            post_json("/api/analysis/start", &host, Some(&origin), &request),
        )
        .await;
        assert_eq!(status, 200, "body: {body}");
        let id = response_json(&body)["id"].as_str().unwrap().to_string();
        let result = wait_for_analysis(addr, &host, &id).await;
        assert_eq!(result["phase"], "complete");
        for rule in result["rules"].as_array().unwrap() {
            assert_eq!(rule["count"], LINES as u64);
            assert_eq!(rule["stored_hits"], 7);
            assert_eq!(rule["truncated"], true);
            assert_eq!(
                rule["histogram"].as_array().unwrap().len(),
                ayame_core::ANALYSIS_HISTOGRAM_BINS
            );
        }

        let _ = std::fs::remove_dir_all(f.parent().unwrap());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cross_origin_writes_are_blocked() {
        let f = scratch_file("csrf.txt", b"a\n");
        let addr = start_server(&f).await;
        let host = format!("127.0.0.1:{}", addr.port());
        let own_origin = format!("http://{host}");

        // Foreign Origin on a state-changing request → 403 (CSRF).
        let (status, _) = send(
            addr,
            post_json("/api/edit/undo", &host, Some("http://evil.com"), ""),
        )
        .await;
        assert_eq!(status, 403);

        // Our own Origin → allowed.
        let (status, _) = send(
            addr,
            post_json("/api/edit/undo", &host, Some(&own_origin), ""),
        )
        .await;
        assert_eq!(status, 200);

        // No Origin (curl/native) → allowed.
        let (status, _) = send(addr, post_json("/api/edit/undo", &host, None, "")).await;
        assert_eq!(status, 200);

        let _ = std::fs::remove_file(&f);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn upload_sanitizes_name_and_opens_uploaded_file() {
        let _upload_guard = UPLOAD_TEST_LOCK.lock().await;
        let f = scratch_file("upload-base.txt", b"base\n");
        let addr = start_server(&f).await;
        let host = format!("127.0.0.1:{}", addr.port());
        let origin = format!("http://{host}");
        let _ = std::fs::remove_dir_all(workspace::uploads_dir());

        let (status, body) = send(
            addr,
            post_raw(
                "/api/upload?name=..%2F..%2Fescape.txt",
                &host,
                Some(&origin),
                "text/plain",
                b"uploaded\n",
            ),
        )
        .await;
        assert_eq!(status, 200, "body: {body}");
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        let path = json["path"].as_str().unwrap();
        assert!(path.ends_with("escape.txt"), "path: {path}");
        assert!(
            Path::new(path).starts_with(workspace::uploads_dir()),
            "upload escaped scratch dir: {path}"
        );
        assert_eq!(std::fs::read(path).unwrap(), b"uploaded\n");

        let (_, lines) = send(addr, get("/api/lines?start=0&count=2", &host)).await;
        assert!(lines.contains("uploaded"), "body: {lines}");

        let _ = std::fs::remove_file(&f);
        let _ = std::fs::remove_dir_all(workspace::uploads_dir());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn upload_over_limit_returns_413_and_removes_partial_file() {
        let _upload_guard = UPLOAD_TEST_LOCK.lock().await;
        let f = scratch_file("upload-limit-base.txt", b"base\n");
        let addr = start_server(&f).await;
        let host = format!("127.0.0.1:{}", addr.port());
        let origin = format!("http://{host}");
        let _ = std::fs::remove_dir_all(workspace::uploads_dir());

        let (status, body) = send(
            addr,
            post_raw(
                "/api/upload?name=too-big.txt",
                &host,
                Some(&origin),
                "text/plain",
                b"0123456789abcdefx",
            ),
        )
        .await;
        assert_eq!(status, 413, "body: {body}");
        assert!(
            !workspace::uploads_dir().join("too-big.txt").exists(),
            "partial upload was not cleaned up"
        );

        let _ = std::fs::remove_file(&f);
        let _ = std::fs::remove_dir_all(workspace::uploads_dir());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn replace_batch_applies_all_carets_as_one_undo_step() {
        let f = scratch_file("batch.txt", b"aaa\nbbb\nccc\n");
        let addr = start_server(&f).await;
        let host = format!("127.0.0.1:{}", addr.port());
        let origin = format!("http://{host}");

        let (status, body) = send(
            addr,
            post_json(
                "/api/edit/replace_batch",
                &host,
                Some(&origin),
                r#"{"edits":[{"l0":0,"c0":1,"l1":0,"c1":1,"text":"X"},{"l0":2,"c0":3,"l1":2,"c1":3,"text":"Y"}]}"#,
            ),
        )
        .await;
        assert_eq!(status, 200, "body: {body}");
        assert!(body.contains("\"dirty\":true"), "body: {body}");
        assert!(
            body.contains(r#""carets":[{"line":0,"col":2},{"line":2,"col":4}]"#),
            "body: {body}"
        );

        let (_, body) = send(addr, get("/api/lines?start=0&count=10", &host)).await;
        assert!(
            body.contains("aXaa") && body.contains("cccY"),
            "body: {body}"
        );

        // A single undo reverts every caret's edit at once.
        let (status, body) =
            send(addr, post_json("/api/edit/undo", &host, Some(&origin), "")).await;
        assert_eq!(status, 200);
        assert!(body.contains("\"dirty\":false"), "body: {body}");
        let (_, body) = send(addr, get("/api/lines?start=0&count=10", &host)).await;
        assert!(
            body.contains("aaa") && !body.contains("aXaa"),
            "body: {body}"
        );

        // Overlapping ranges are rejected without touching the text.
        let (status, _) = send(
            addr,
            post_json(
                "/api/edit/replace_batch",
                &host,
                Some(&origin),
                r#"{"edits":[{"l0":0,"c0":0,"l1":0,"c1":2,"text":"x"},{"l0":0,"c0":1,"l1":0,"c1":3,"text":"y"}]}"#,
            ),
        )
        .await;
        assert_eq!(status, 400);

        let _ = std::fs::remove_file(&f);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn find_sees_unsaved_edits_through_a_cached_snapshot() {
        let f = scratch_file("find.txt", b"alpha\nbeta\ngamma\n");
        let (addr, state) = start_server_with_state(&f).await;
        let host = format!("127.0.0.1:{}", addr.port());
        let origin = format!("http://{host}");

        // Clean session: the needle doesn't exist on disk, and no snapshot is built.
        let (status, body) = send(addr, get("/api/find?q=NEEDLE&dir=next&from=0", &host)).await;
        assert_eq!(status, 200, "body: {body}");
        assert!(body.contains("\"hit\":null"), "body: {body}");
        assert_eq!(state.dirty_snapshot_builds(), 0);

        // Insert a line "NEEDLE" after "alpha" — an UNSAVED edit only.
        let (status, body) = send(
            addr,
            post_json(
                "/api/edit/replace_range",
                &host,
                Some(&origin),
                r#"{"l0":0,"c0":5,"l1":0,"c1":5,"text":"\nNEEDLE"}"#,
            ),
        )
        .await;
        assert_eq!(status, 200, "body: {body}");
        assert!(body.contains("\"dirty\":true"), "body: {body}");

        // Find now hits the edited text at its VIEW position (line 1, byte 6
        // of "alpha\nNEEDLE\nbeta\ngamma\n") — the on-disk file is unchanged.
        let (status, body) = send(addr, get("/api/find?q=NEEDLE&dir=next&from=0", &host)).await;
        assert_eq!(status, 200, "body: {body}");
        assert!(body.contains("\"line\":1"), "body: {body}");
        assert!(body.contains("\"byte\":6"), "body: {body}");
        assert_eq!(state.dirty_snapshot_builds(), 1, "first dirty find builds");

        // Search anchors used by the web UI resolve against the same dirty view:
        // line 1, column 3 is just after "NEE" in "alpha\nNEEDLE...".
        let (status, linebyte_body) = send(addr, get("/api/linebyte?line=1&col=3", &host)).await;
        assert_eq!(status, 200, "body: {linebyte_body}");
        assert!(
            linebyte_body.contains("\"byte\":9"),
            "body: {linebyte_body}"
        );
        assert_eq!(
            state.dirty_snapshot_builds(),
            1,
            "linebyte reuses the cached dirty snapshot"
        );

        // A second find at the same revision reuses the cached snapshot.
        let (status, body2) = send(addr, get("/api/find?q=NEEDLE&dir=next&from=0", &host)).await;
        assert_eq!(status, 200);
        assert_eq!(body, body2, "cached snapshot answers identically");
        assert_eq!(
            state.dirty_snapshot_builds(),
            1,
            "no rebuild at same revision"
        );

        // Another edit bumps the revision: prepend "NEEDLE " to "beta" (view
        // line 2). A stale-cache find must see the NEW view, not the old snapshot.
        let (status, body) = send(
            addr,
            post_json(
                "/api/edit/replace_range",
                &host,
                Some(&origin),
                r#"{"l0":2,"c0":0,"l1":2,"c1":0,"text":"NEEDLE "}"#,
            ),
        )
        .await;
        assert_eq!(status, 200, "body: {body}");

        let (status, body) = send(addr, get("/api/find?q=NEEDLE&dir=next&from=7", &host)).await;
        assert_eq!(status, 200, "body: {body}");
        assert!(body.contains("\"line\":2"), "body: {body}");
        assert_eq!(
            state.dirty_snapshot_builds(),
            2,
            "revision bump rebuilds once"
        );

        // On disk nothing changed throughout.
        assert_eq!(std::fs::read(&f).unwrap(), b"alpha\nbeta\ngamma\n");
        let _ = std::fs::remove_file(&f);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn split_save_rejects_zero_lines() {
        let f = scratch_file("split-zero.txt", b"a\nb\n");
        let addr = start_server(&f).await;
        let host = format!("127.0.0.1:{}", addr.port());
        let origin = format!("http://{host}");

        let (status, body) = send(
            addr,
            post_json("/api/split/save", &host, Some(&origin), r#"{"lines":0}"#),
        )
        .await;
        assert_eq!(status, 400, "body: {body}");
        let _ = std::fs::remove_file(&f);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn edit_save_round_trip_overwrites_in_place() {
        let f = scratch_file("save.txt", b"hello\nworld\n");
        let addr = start_server(&f).await;
        let host = format!("127.0.0.1:{}", addr.port());
        let origin = format!("http://{host}");

        // Type over "hello" → "HELLO" (single undo step).
        let (status, body) = send(
            addr,
            post_json(
                "/api/edit/replace_range",
                &host,
                Some(&origin),
                r#"{"l0":0,"c0":0,"l1":0,"c1":5,"text":"HELLO"}"#,
            ),
        )
        .await;
        assert_eq!(status, 200, "body: {body}");
        assert!(body.contains("\"dirty\":true"), "body: {body}");

        // The viewport sees the overlay.
        let (status, body) = send(addr, get("/api/lines?start=0&count=10", &host)).await;
        assert_eq!(status, 200);
        assert!(body.contains("HELLO"), "body: {body}");

        // Save in place (stage → verify revision → swap → reload).
        let (status, body) = send(
            addr,
            post_json(
                "/api/edit/save",
                &host,
                Some(&origin),
                r#"{"overwrite":true}"#,
            ),
        )
        .await;
        assert_eq!(status, 200, "body: {body}");
        assert_eq!(std::fs::read(&f).unwrap(), b"HELLO\nworld\n");

        // After the save the session is clean and the file is still open.
        let (status, body) = send(addr, get("/api/stat", &host)).await;
        assert_eq!(status, 200);
        assert!(body.contains("\"open\":true"), "body: {body}");
        assert!(body.contains("\"dirty\":false"), "body: {body}");

        let _ = std::fs::remove_file(&f);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn edit_save_converts_encoding_eol_and_bom_then_reloads_tab() {
        let f = scratch_file("convert-save.txt", b"alpha\nbeta\n");
        let addr = start_server(&f).await;
        let host = format!("127.0.0.1:{}", addr.port());
        let origin = format!("http://{host}");

        let (status, body) = send(
            addr,
            post_json(
                "/api/edit/save",
                &host,
                Some(&origin),
                r#"{"overwrite":true,"encoding":"utf-8","eol":"crlf","bom":true}"#,
            ),
        )
        .await;
        assert_eq!(status, 200, "body: {body}");
        assert!(body.contains("\"switched\":true"), "body: {body}");
        assert_eq!(std::fs::read(&f).unwrap(), b"\xEF\xBB\xBFalpha\r\nbeta\r\n");

        let (status, stat) = send(addr, get("/api/stat", &host)).await;
        assert_eq!(status, 200, "body: {stat}");
        assert!(stat.contains("\"eol\":\"crlf\""), "body: {stat}");
        assert!(stat.contains("\"bom_bytes\":3"), "body: {stat}");

        let _ = std::fs::remove_file(&f);
    }

    /// The full undo-across-save contract: an in-place save keeps the undo
    /// history (clean + can_undo), undo restores the pre-save view while the
    /// disk keeps the saved bytes, redo returns to the exact saved state, and
    /// undoing past the save then saving again writes the pre-edit bytes.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn undo_crosses_in_place_save_and_second_save_writes_undone_bytes() {
        let f = scratch_file("undosave.txt", b"one\ntwo\nthree\n");
        let addr = start_server(&f).await;
        let host = format!("127.0.0.1:{}", addr.port());
        let origin = format!("http://{host}");

        let (status, body) = send(
            addr,
            post_json(
                "/api/edit/replace_range",
                &host,
                Some(&origin),
                r#"{"l0":0,"c0":0,"l1":0,"c1":0,"text":"EDIT_"}"#,
            ),
        )
        .await;
        assert_eq!(status, 200, "body: {body}");

        let (status, body) = send(
            addr,
            post_json(
                "/api/edit/save",
                &host,
                Some(&origin),
                r#"{"overwrite":true}"#,
            ),
        )
        .await;
        assert_eq!(status, 200, "body: {body}");
        assert_eq!(std::fs::read(&f).unwrap(), b"EDIT_one\ntwo\nthree\n");

        // 1. Clean after the save, and the history SURVIVED it.
        let (_, body) = send(addr, get("/api/stat", &host)).await;
        assert!(body.contains("\"dirty\":false"), "body: {body}");
        assert!(body.contains("\"can_undo\":true"), "body: {body}");

        // 2. Undo crosses the save: the view shows the pre-save text, the
        //    session is dirty again, the disk keeps the saved bytes.
        let (status, body) =
            send(addr, post_json("/api/edit/undo", &host, Some(&origin), "")).await;
        assert_eq!(status, 200);
        assert!(body.contains("\"dirty\":true"), "body: {body}");
        let (_, body) = send(addr, get("/api/lines?start=0&count=1", &host)).await;
        assert!(body.contains("\"text\":\"one\""), "body: {body}");
        assert_eq!(std::fs::read(&f).unwrap(), b"EDIT_one\ntwo\nthree\n");

        // 3. Redo returns to the EXACT saved state: clean again.
        let (status, body) =
            send(addr, post_json("/api/edit/redo", &host, Some(&origin), "")).await;
        assert_eq!(status, 200);
        assert!(body.contains("\"dirty\":false"), "body: {body}");

        // 4. Undo once more and save again: the disk now holds the pre-edit
        //    bytes even though the mmap'd document was never reloaded.
        let (status, _) = send(addr, post_json("/api/edit/undo", &host, Some(&origin), "")).await;
        assert_eq!(status, 200);
        let (status, body) = send(
            addr,
            post_json(
                "/api/edit/save",
                &host,
                Some(&origin),
                r#"{"overwrite":true}"#,
            ),
        )
        .await;
        assert_eq!(status, 200, "body: {body}");
        assert_eq!(std::fs::read(&f).unwrap(), b"one\ntwo\nthree\n");
        let (_, body) = send(addr, get("/api/stat", &host)).await;
        assert!(body.contains("\"dirty\":false"), "body: {body}");
        assert!(body.contains("\"can_redo\":true"), "body: {body}");

        // The aside files both saves created are cleaned up eagerly on Unix
        // (the live mmap keeps its inode without the name).
        if cfg!(unix) {
            let stale: Vec<_> = std::fs::read_dir(f.parent().unwrap())
                .unwrap()
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|n| n.contains(".ayame-prev-"))
                .collect();
            assert!(stale.is_empty(), "aside files left behind: {stale:?}");
        }

        let _ = std::fs::remove_file(&f);
    }

    /// `/api/edit/revert` returns to the last SAVED state (a reload from
    /// disk), not to the content the file was originally opened with.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn revert_returns_to_the_last_saved_state() {
        let f = scratch_file("revert.txt", b"aaa\nbbb\n");
        let addr = start_server(&f).await;
        let host = format!("127.0.0.1:{}", addr.port());
        let origin = format!("http://{host}");

        // Edit + save, then a second (unsaved) edit.
        let (status, _) = send(
            addr,
            post_json(
                "/api/edit/replace_range",
                &host,
                Some(&origin),
                r#"{"l0":0,"c0":0,"l1":0,"c1":3,"text":"SAVED"}"#,
            ),
        )
        .await;
        assert_eq!(status, 200);
        let (status, _) = send(
            addr,
            post_json(
                "/api/edit/save",
                &host,
                Some(&origin),
                r#"{"overwrite":true}"#,
            ),
        )
        .await;
        assert_eq!(status, 200);
        let (status, _) = send(
            addr,
            post_json(
                "/api/edit/replace_range",
                &host,
                Some(&origin),
                r#"{"l0":1,"c0":0,"l1":1,"c1":3,"text":"UNSAVED"}"#,
            ),
        )
        .await;
        assert_eq!(status, 200);

        let (status, body) = send(
            addr,
            post_json("/api/edit/revert", &host, Some(&origin), ""),
        )
        .await;
        assert_eq!(status, 200, "body: {body}");
        assert!(body.contains("\"dirty\":false"), "body: {body}");
        assert!(body.contains("\"can_undo\":false"), "body: {body}");

        // The view is the SAVED text: first edit kept, second edit gone.
        let (_, body) = send(addr, get("/api/lines?start=0&count=2", &host)).await;
        assert!(body.contains("SAVED"), "body: {body}");
        assert!(!body.contains("UNSAVED"), "body: {body}");
        assert!(body.contains("\"text\":\"bbb\""), "body: {body}");

        let _ = std::fs::remove_file(&f);
    }

    /// 名前を付けて保存 (`switch_to_saved`): the ACTIVE TAB becomes the saved
    /// file — no second tab appears and the session is clean afterwards.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn save_as_switches_the_active_tab_to_the_saved_file() {
        let f = scratch_file("saveas-src.txt", b"alpha\nbeta\n");
        let addr = start_server(&f).await;
        let host = format!("127.0.0.1:{}", addr.port());
        let origin = format!("http://{host}");

        let (status, _) = send(
            addr,
            post_json(
                "/api/edit/replace_range",
                &host,
                Some(&origin),
                r#"{"l0":0,"c0":0,"l1":0,"c1":5,"text":"ALPHA"}"#,
            ),
        )
        .await;
        assert_eq!(status, 200);

        let out = f.with_extension("saved.txt");
        let body = format!(
            r#"{{"path":"{}","switch_to_saved":true}}"#,
            out.display().to_string().replace('\\', "\\\\")
        );
        let (status, resp) = send(
            addr,
            post_json("/api/edit/save", &host, Some(&origin), &body),
        )
        .await;
        assert_eq!(status, 200, "body: {resp}");
        assert!(resp.contains("\"switched\":true"), "body: {resp}");
        assert_eq!(std::fs::read(&out).unwrap(), b"ALPHA\nbeta\n");

        // One tab, showing the saved file, clean.
        let (_, tabs) = send(addr, get("/api/tabs", &host)).await;
        assert_eq!(tabs.matches("\"id\":").count(), 1, "tabs: {tabs}");
        assert!(tabs.contains("saved.txt"), "tabs: {tabs}");
        let (_, stat) = send(addr, get("/api/stat", &host)).await;
        assert!(stat.contains("saved.txt"), "stat: {stat}");
        assert!(stat.contains("\"dirty\":false"), "stat: {stat}");
        // The original file was never touched by the save-as.
        assert_eq!(std::fs::read(&f).unwrap(), b"alpha\nbeta\n");

        let _ = std::fs::remove_file(&out);
        let _ = std::fs::remove_file(&f);
    }

    /// Opening a path that is already open in a tab focuses that tab instead
    /// of opening a duplicate.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn opening_an_already_open_file_focuses_its_tab() {
        let fa = scratch_file("dedupe-a.txt", b"a\n");
        let fb = scratch_file("dedupe-b.txt", b"b\n");
        let addr = start_server(&fa).await;
        let host = format!("127.0.0.1:{}", addr.port());
        let origin = format!("http://{host}");

        let body = format!(
            r#"{{"path":"{}"}}"#,
            fb.display().to_string().replace('\\', "\\\\")
        );
        let (status, _) = send(addr, post_json("/api/open", &host, Some(&origin), &body)).await;
        assert_eq!(status, 200);

        // Re-open the first file: no third tab, and it becomes active again.
        let body = format!(
            r#"{{"path":"{}"}}"#,
            fa.display().to_string().replace('\\', "\\\\")
        );
        let (status, _) = send(addr, post_json("/api/open", &host, Some(&origin), &body)).await;
        assert_eq!(status, 200);

        let (_, tabs) = send(addr, get("/api/tabs", &host)).await;
        assert_eq!(tabs.matches("\"id\":").count(), 2, "tabs: {tabs}");
        assert!(
            tabs.contains("dedupe-a.txt\",\"dirty\":false,\"active\":true")
                || tabs.contains("\"active\":true,\"name\":\"dedupe-a.txt\""),
            "tabs: {tabs}"
        );

        let _ = std::fs::remove_file(&fa);
        let _ = std::fs::remove_file(&fb);
    }

    /// Crash persistence, scenario 1: unsaved edits survive a "crash" (a
    /// brand-new `AppState` over the same file and cache dir — the in-process
    /// equivalent of killing and restarting the server) and are restored by
    /// `POST /api/edit/recover`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn wal_recovers_unsaved_edits_after_a_crash() {
        let f = scratch_file("wal-recover.txt", b"alpha\nbeta\n");
        let cache = scratch_cache("recover");

        // Session 1: one committed edit — never saved.
        let addr = start_server_with_opts(&f, wal_opts(&cache)).await;
        let host = format!("127.0.0.1:{}", addr.port());
        let origin = format!("http://{host}");
        let (status, body) = send(
            addr,
            post_json(
                "/api/edit/replace_range",
                &host,
                Some(&origin),
                r#"{"l0":0,"c0":0,"l1":0,"c1":5,"text":"ALPHA"}"#,
            ),
        )
        .await;
        assert_eq!(status, 200, "body: {body}");
        // The commit was mirrored into the crash log on the spot.
        let wal_path = ayame_core::wal::wal_path_for(&cache, &f);
        assert!(wal_path.exists(), "no crash log at {}", wal_path.display());

        // "Crash": the first state is simply never used again; nothing was
        // saved, the overlay lived in memory only. Restart on the same file.
        let addr2 = start_server_with_opts(&f, wal_opts(&cache)).await;
        let host2 = format!("127.0.0.1:{}", addr2.port());
        let origin2 = format!("http://{host2}");

        // The open reports the recoverable log instead of auto-applying it.
        let (status, stat) = send(addr2, get("/api/stat", &host2)).await;
        assert_eq!(status, 200);
        assert!(stat.contains("\"recoverable\":1"), "stat: {stat}");
        assert!(stat.contains("\"dirty\":false"), "stat: {stat}");
        let (_, lines) = send(addr2, get("/api/lines?start=0&count=10", &host2)).await;
        assert!(lines.contains("alpha"), "pre-recover view: {lines}");

        // Restore: the edit is back, dirty, and one transaction was replayed.
        let (status, body) = send(
            addr2,
            post_json("/api/edit/recover", &host2, Some(&origin2), "{}"),
        )
        .await;
        assert_eq!(status, 200, "body: {body}");
        assert!(body.contains("\"replayed\":1"), "body: {body}");
        assert!(body.contains("\"dirty\":true"), "body: {body}");
        let (_, lines) = send(addr2, get("/api/lines?start=0&count=10", &host2)).await;
        assert!(lines.contains("ALPHA"), "post-recover view: {lines}");
        // The recovered suffix carries real undo history.
        let (status, body) = send(
            addr2,
            post_json("/api/edit/undo", &host2, Some(&origin2), ""),
        )
        .await;
        assert_eq!(status, 200);
        assert!(body.contains("\"dirty\":false"), "body: {body}");
        // The flag is gone: a second recover has nothing to do.
        let (_, stat) = send(addr2, get("/api/stat", &host2)).await;
        assert!(!stat.contains("recoverable"), "stat: {stat}");
        let (status, _) = send(
            addr2,
            post_json("/api/edit/recover", &host2, Some(&origin2), "{}"),
        )
        .await;
        assert_eq!(status, 409);

        let _ = std::fs::remove_dir_all(&cache);
        let _ = std::fs::remove_file(&f);
    }

    /// Crash persistence, scenario 2: declining the recovery discards the log
    /// — the session stays clean and a further restart sees nothing.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn wal_discard_declines_the_recovery_and_cleans_the_log() {
        let f = scratch_file("wal-discard.txt", b"one\ntwo\n");
        let cache = scratch_cache("discard");

        let addr = start_server_with_opts(&f, wal_opts(&cache)).await;
        let host = format!("127.0.0.1:{}", addr.port());
        let origin = format!("http://{host}");
        let (status, _) = send(
            addr,
            post_json(
                "/api/edit/replace_range",
                &host,
                Some(&origin),
                r#"{"l0":1,"c0":0,"l1":1,"c1":3,"text":"TWO"}"#,
            ),
        )
        .await;
        assert_eq!(status, 200);

        // Restart, decline.
        let addr2 = start_server_with_opts(&f, wal_opts(&cache)).await;
        let host2 = format!("127.0.0.1:{}", addr2.port());
        let origin2 = format!("http://{host2}");
        let (_, stat) = send(addr2, get("/api/stat", &host2)).await;
        assert!(stat.contains("\"recoverable\":1"), "stat: {stat}");
        let (status, body) = send(
            addr2,
            post_json(
                "/api/edit/recover",
                &host2,
                Some(&origin2),
                r#"{"discard":true}"#,
            ),
        )
        .await;
        assert_eq!(status, 200, "body: {body}");
        assert!(body.contains("\"replayed\":0"), "body: {body}");
        assert!(body.contains("\"dirty\":false"), "body: {body}");
        let (_, lines) = send(addr2, get("/api/lines?start=0&count=10", &host2)).await;
        assert!(lines.contains("two") && !lines.contains("TWO"), "{lines}");
        let (_, stat) = send(addr2, get("/api/stat", &host2)).await;
        assert!(!stat.contains("recoverable"), "stat: {stat}");

        // A third "restart" finds a clean log: no recovery offer.
        let addr3 = start_server_with_opts(&f, wal_opts(&cache)).await;
        let host3 = format!("127.0.0.1:{}", addr3.port());
        let (_, stat) = send(addr3, get("/api/stat", &host3)).await;
        assert!(!stat.contains("recoverable"), "stat: {stat}");

        let _ = std::fs::remove_dir_all(&cache);
        let _ = std::fs::remove_file(&f);
    }

    /// Crash persistence, scenario 3: a successful in-place save RESETS the
    /// log onto the new file identity, so a kill + restart right after the
    /// save has nothing to recover.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn wal_is_reset_by_a_save_so_a_restart_is_clean() {
        let f = scratch_file("wal-save.txt", b"aaa\nbbb\n");
        let cache = scratch_cache("save");

        let addr = start_server_with_opts(&f, wal_opts(&cache)).await;
        let host = format!("127.0.0.1:{}", addr.port());
        let origin = format!("http://{host}");
        let (status, _) = send(
            addr,
            post_json(
                "/api/edit/replace_range",
                &host,
                Some(&origin),
                r#"{"l0":0,"c0":0,"l1":0,"c1":3,"text":"AAA"}"#,
            ),
        )
        .await;
        assert_eq!(status, 200);
        let (status, body) = send(
            addr,
            post_json(
                "/api/edit/save",
                &host,
                Some(&origin),
                r#"{"overwrite":true}"#,
            ),
        )
        .await;
        assert_eq!(status, 200, "body: {body}");
        assert_eq!(std::fs::read(&f).unwrap(), b"AAA\nbbb\n");

        // Kill + restart: the log was reset at the save commit; the reopened
        // file matches its (empty) header — nothing to recover.
        let addr2 = start_server_with_opts(&f, wal_opts(&cache)).await;
        let host2 = format!("127.0.0.1:{}", addr2.port());
        let (_, stat) = send(addr2, get("/api/stat", &host2)).await;
        assert!(!stat.contains("recoverable"), "stat: {stat}");
        assert!(stat.contains("\"dirty\":false"), "stat: {stat}");
        let (_, lines) = send(addr2, get("/api/lines?start=0&count=10", &host2)).await;
        assert!(lines.contains("AAA"), "{lines}");

        let _ = std::fs::remove_dir_all(&cache);
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn verbatim_prefix_is_stripped_for_display() {
        use super::workspace::{display_path, strip_verbatim};
        // Drive prefix goes; UNC prefix folds back to a plain UNC path.
        assert_eq!(strip_verbatim(r"\\?\C:\Users\x"), r"C:\Users\x");
        assert_eq!(strip_verbatim(r"\\?\UNC\srv\share\f"), r"\\srv\share\f");
        // Everything else passes through unchanged, on every OS.
        assert_eq!(strip_verbatim("/tmp/f.txt"), "/tmp/f.txt");
        assert_eq!(strip_verbatim(r"C:\Users\x\f.txt"), r"C:\Users\x\f.txt");
        assert_eq!(strip_verbatim(r"\\srv\share\f"), r"\\srv\share\f");
        // The Path-typed form is the same choke point.
        assert_eq!(display_path(Path::new(r"\\?\C:\Users\x")), r"C:\Users\x");
        assert_eq!(display_path(Path::new("/tmp/f.txt")), "/tmp/f.txt");
    }

    /// Issue #35 dirty-tab handoff: `/api/tabs/detach` removes the tab but
    /// KEEPS its crash log, so a second process (the adopting window) replays
    /// the unsaved edits through the ordinary recover path. `/api/tabs/close`
    /// would have deleted that log as a deliberate discard.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tabs_detach_keeps_the_wal_for_a_dirty_handoff() {
        let f = scratch_file("wal-handoff.txt", b"alpha\nbeta\n");
        let cache = scratch_cache("handoff");

        // Window A: one committed edit — never saved — then detach the tab.
        let addr = start_server_with_opts(&f, wal_opts(&cache)).await;
        let host = format!("127.0.0.1:{}", addr.port());
        let origin = format!("http://{host}");
        let (status, body) = send(
            addr,
            post_json(
                "/api/edit/replace_range",
                &host,
                Some(&origin),
                r#"{"l0":0,"c0":0,"l1":0,"c1":5,"text":"ALPHA"}"#,
            ),
        )
        .await;
        assert_eq!(status, 200, "body: {body}");
        let (status, tabs) = send(addr, get("/api/tabs", &host)).await;
        assert_eq!(status, 200);
        let tabs: serde_json::Value = serde_json::from_str(&tabs).unwrap();
        let id = tabs["tabs"][0]["id"].as_u64().unwrap();
        let (status, stat) = send(
            addr,
            post_json(
                "/api/tabs/detach",
                &host,
                Some(&origin),
                &format!(r#"{{"id":{id}}}"#),
            ),
        )
        .await;
        assert_eq!(status, 200, "detach: {stat}");
        assert!(stat.contains("\"open\":false"), "stat: {stat}");
        let wal_path = ayame_core::wal::wal_path_for(&cache, &f);
        assert!(
            wal_path.exists(),
            "detach must keep the crash log for the adopting window"
        );

        // Window B: same file, same cache — the log is offered and replays.
        let addr2 = start_server_with_opts(&f, wal_opts(&cache)).await;
        let host2 = format!("127.0.0.1:{}", addr2.port());
        let origin2 = format!("http://{host2}");
        let (status, stat) = send(addr2, get("/api/stat", &host2)).await;
        assert_eq!(status, 200);
        assert!(stat.contains("\"recoverable\":1"), "stat: {stat}");
        let (status, body) = send(
            addr2,
            post_json("/api/edit/recover", &host2, Some(&origin2), "{}"),
        )
        .await;
        assert_eq!(status, 200, "recover: {body}");
        assert!(body.contains("\"dirty\":true"), "body: {body}");
        let (_, lines) = send(addr2, get("/api/lines?start=0&count=10", &host2)).await;
        assert!(lines.contains("ALPHA"), "adopted view: {lines}");

        let _ = std::fs::remove_dir_all(&cache);
        let _ = std::fs::remove_file(&f);
    }

    /// Without a crash log to carry the edits (no cache dir), detaching a
    /// dirty tab is refused with 409 and the tab stays put — moving it would
    /// silently drop the unsaved edits.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tabs_detach_refuses_a_dirty_tab_without_a_crash_log() {
        let f = scratch_file("handoff-nocache.txt", b"alpha\nbeta\n");
        let addr = start_server(&f).await; // OpenOptions::default(): no cache dir
        let host = format!("127.0.0.1:{}", addr.port());
        let origin = format!("http://{host}");
        let (status, body) = send(
            addr,
            post_json(
                "/api/edit/replace_range",
                &host,
                Some(&origin),
                r#"{"l0":0,"c0":0,"l1":0,"c1":5,"text":"ALPHA"}"#,
            ),
        )
        .await;
        assert_eq!(status, 200, "body: {body}");
        let (_, tabs) = send(addr, get("/api/tabs", &host)).await;
        let tabs: serde_json::Value = serde_json::from_str(&tabs).unwrap();
        let id = tabs["tabs"][0]["id"].as_u64().unwrap();
        let (status, body) = send(
            addr,
            post_json(
                "/api/tabs/detach",
                &host,
                Some(&origin),
                &format!(r#"{{"id":{id}}}"#),
            ),
        )
        .await;
        assert_eq!(status, 409, "body: {body}");
        // The tab survived the refusal, edits intact.
        let (_, stat) = send(addr, get("/api/stat", &host)).await;
        assert!(stat.contains("\"open\":true"), "stat: {stat}");
        assert!(stat.contains("\"dirty\":true"), "stat: {stat}");
        let _ = std::fs::remove_file(&f);
    }

    /// Response-level guard: an error string that carries a path reaches the
    /// client with the verbatim prefix stripped. (On Linux `\\?\...` is just a
    /// weird relative name that cannot exist, and on Windows canonicalizing it
    /// fails the same way, so /api/browse answers 400 echoing the directory.)
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn browse_error_shows_the_path_without_the_verbatim_prefix() {
        let f = scratch_file("browse-verbatim.txt", b"a\n");
        let addr = start_server(&f).await;
        let host = format!("127.0.0.1:{}", addr.port());

        // dir = \\?\C:\ayame-no-such-dir, percent-encoded.
        let (status, body) = send(
            addr,
            get(
                "/api/browse?dir=%5C%5C%3F%5CC%3A%5Cayame-no-such-dir",
                &host,
            ),
        )
        .await;
        assert_eq!(status, 400, "body: {body}");
        // The error body is JSON `{code, message}`, which escapes the path's
        // backslashes — decode it before checking the displayed path.
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("json error body");
        let message = parsed["message"].as_str().unwrap_or(&body);
        assert!(message.contains(r"C:\ayame-no-such-dir"), "body: {body}");
        assert!(!message.contains(r"\\?\"), "body: {body}");

        let _ = std::fs::remove_file(&f);
    }

    /// A zero-width rectangle (c1 == c0) is a valid caret column: the export
    /// succeeds and writes one empty piece per line (a newline-only column).
    /// Only a REVERSED column range is rejected.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn zero_width_rect_selection_saves_a_newline_only_column() {
        let f = scratch_file("rect0.txt", b"ab\ncd\n");
        let addr = start_server(&f).await;
        let host = format!("127.0.0.1:{}", addr.port());
        let origin = format!("http://{host}");

        let out = f.with_extension("sel");
        let body = format!(
            r#"{{"path":"{}","rect":true,"l0":0,"c0":1,"l1":1,"c1":1}}"#,
            out.display()
        );
        let (status, resp) = send(
            addr,
            post_json("/api/selection/save", &host, Some(&origin), &body),
        )
        .await;
        assert_eq!(status, 200, "body: {resp}");
        assert!(resp.contains("\"lines\":2"), "body: {resp}");
        assert_eq!(std::fs::read(&out).unwrap(), b"\n");

        // A reversed column range is still invalid.
        let body = format!(
            r#"{{"path":"{}","overwrite":true,"rect":true,"l0":0,"c0":2,"l1":1,"c1":1}}"#,
            out.display()
        );
        let (status, _) = send(
            addr,
            post_json("/api/selection/save", &host, Some(&origin), &body),
        )
        .await;
        assert_eq!(status, 400);

        let _ = std::fs::remove_file(&out);
        let _ = std::fs::remove_file(&f);
    }
}
