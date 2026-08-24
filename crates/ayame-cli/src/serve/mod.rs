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
use ayame_core::{Document, Encoding, Eol};
use serde::Serialize;
use tower_http::catch_panic::CatchPanicLayer;

use crate::{first_opt, has_flag, open_opts, parse_for};

mod actions;
mod analysis;
mod assets;
mod completion;
mod edit;
mod error;
mod inspect;
mod markers;
mod ops;
mod position;
mod recognize;
mod security;
mod state;
#[cfg(feature = "typegen")]
pub(crate) mod typegen;
pub(crate) mod workspace;

use error::ApiError;
use security::NetPolicy;
use state::{AppState, DiskCheckResponse, SharedState, TabsResponse, TailStatus, UiState};

/// Hard cap on lines returned in one viewport request, so a hostile/buggy
/// client can never ask us to materialize the whole file.
const MAX_VIEW: u64 = 20_000;

pub fn cmd_serve(args: &[String]) -> Result<()> {
    let (pos, opts, flags) = parse_for("serve", args)?;
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
    // Fingerprint our own executable while it is still the current version:
    // op workers are spawned from it, and an update landing mid-session must
    // be caught rather than silently spawning a different build (#137).
    crate::worker::snapshot_executable();
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
        .route("/themes.css", get(assets::themes_css))
        .route("/favicon.svg", get(assets::favicon_svg))
        .route("/ayame-logo.svg", get(assets::ayame_logo_svg))
        .route("/iris-watercolor.png", get(assets::iris_watercolor_png))
        .route("/api/stat", get(api_stat))
        .route("/api/tail/poll", post(api_tail_poll))
        .route("/api/disk/check", post(api_disk_check))
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
            "/api/position/resolve",
            post(position::api_position_resolve),
        )
        .route("/api/selection/recognize", post(recognize::api_recognize))
        .route("/api/completion", post(completion::api_completion))
        .route("/api/inspect", post(inspect::api_inspect))
        .route("/api/inspect/parse", post(inspect::api_parse_escape))
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
        .route(
            "/api/markers/range-counts",
            get(markers::api_marker_range_counts),
        )
        .route("/api/change-history", get(markers::api_change_history))
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
        .route("/api/actions/run", post(actions::api_external_action_run))
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
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
#[cfg_attr(feature = "typegen", ts(optional_fields))]
pub(super) struct StatResponse {
    /// Whether a file is currently open. When false, every file field is
    /// absent and the front-end shows its open/welcome screen.
    open: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lines: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    encoding: Option<Encoding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    eol: Option<Eol>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bom_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stride: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    checkpoints: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    index_bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    index_ms: Option<u128>,
    #[serde(skip_serializing_if = "Option::is_none")]
    from_cache: Option<bool>,
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
                path: Some(file.path),
                bytes: Some(file.bytes),
                lines: Some(file.lines),
                encoding: Some(file.encoding),
                eol: Some(file.eol),
                bom_bytes: Some(file.bom_bytes),
                stride: Some(file.stride),
                checkpoints: Some(file.checkpoints),
                index_bytes: Some(file.index_bytes),
                index_ms: Some(file.index_ms),
                from_cache: Some(file.from_cache),
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
            path: None,
            bytes: None,
            lines: None,
            encoding: None,
            eol: None,
            bom_bytes: None,
            stride: None,
            checkpoints: None,
            index_bytes: None,
            index_ms: None,
            from_cache: None,
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

/// The filesystem-open endpoint returns the same stat shape, but its distinct
/// generated name lets callers type the endpoint without hand-written aliases.
#[derive(Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct OpenResponse {
    #[serde(flatten)]
    stat: StatResponse,
}

impl From<StatResponse> for OpenResponse {
    fn from(stat: StatResponse) -> Self {
        Self { stat }
    }
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

/// Has anything else written the open file since we last read or wrote it?
/// One `stat`, so the client can ask on every window focus and before every
/// overwrite instead of only while tail-follow happens to be on (#163).
async fn api_disk_check(State(state): State<SharedState>) -> Json<DiskCheckResponse> {
    let status = tokio::task::spawn_blocking(move || state.disk_check())
        .await
        .unwrap_or(DiskCheckResponse {
            open: false,
            changed: false,
        });
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
mod test_support;
#[cfg(test)]
mod tests;
