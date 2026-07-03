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

use std::net::SocketAddr;
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

mod assets;
mod edit;
mod ops;
mod security;
mod state;
pub(crate) mod workspace;

use security::NetPolicy;
use state::{AppState, SharedState, TabsResponse};

/// Hard cap on lines returned in one viewport request, so a hostile/buggy
/// client can never ask us to materialize the whole file.
const MAX_VIEW: u64 = 20_000;

pub fn cmd_serve(args: &[String]) -> Result<()> {
    let (pos, opts, flags) = parse_checked(
        args,
        &["--encoding", "--stride", "--host", "--port", "--cache-dir"],
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
            eprintln!("ayame: no file open yet — open one (drag & drop or 開く)");
            None
        }
    };
    Ok(AppState::new(doc, open_options))
}

/// Build the axum router. Every endpoint is a thin wrapper over `ayame-core`;
/// the same router serves both the CLI `serve` and the native GUI window.
fn router(state: SharedState, policy: Arc<NetPolicy>) -> Router {
    Router::new()
        .route("/", get(assets::index))
        .route("/app.js", get(assets::app_js))
        .route("/style.css", get(assets::style_css))
        .route("/favicon.svg", get(assets::favicon_svg))
        .route("/ayame-logo.svg", get(assets::ayame_logo_svg))
        .route("/iris-watercolor.png", get(assets::iris_watercolor_png))
        .route("/api/stat", get(api_stat))
        .route("/api/open", post(workspace::api_open))
        .route("/api/new", post(workspace::api_new))
        .route("/api/tabs", get(workspace::api_tabs))
        .route("/api/tabs/select", post(workspace::api_tabs_select))
        .route("/api/tabs/close", post(workspace::api_tabs_close))
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
        .route("/api/sort/save", post(ops::api_sort_save))
        .route("/api/replace/save", post(ops::api_replace_save))
        .route("/api/case/save", post(ops::api_case_save))
        .route("/api/split/save", post(ops::api_split_save))
        .route("/api/search", get(ops::api_search))
        .route("/api/find", get(ops::api_find))
        .route("/api/diff", get(ops::api_diff))
        .route("/api/linebyte", get(ops::api_linebyte))
        .layer(axum::middleware::from_fn_with_state(
            policy,
            security::guard,
        ))
        .layer(CatchPanicLayer::new())
        .with_state(state)
}

async fn serve(
    state: SharedState,
    host: String,
    port: u16,
    policy: Arc<NetPolicy>,
    remote_active: bool,
) -> Result<()> {
    let app = router(state.clone(), policy);
    let addr: SocketAddr = format!("{host}:{port}")
        .parse()
        .context("invalid host/port")?;
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
    // Graceful shutdown: drop the scratch this process accumulated (uploads,
    // untitled buffers, unsaved sort results, in-place save aside files).
    state.cleanup_aside_files();
    workspace::cleanup_temp_dirs();
    result
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
}

pub(super) fn stat_response(state: &AppState) -> StatResponse {
    // One read acquisition: doc and edits are mutually consistent and nothing
    // (in particular not the undo history) is cloned.
    state.read(|ws| match ws.doc_and_edits() {
        Ok((doc, edits)) => {
            let edit = edits.stats(doc);
            StatResponse {
                open: true,
                file: Some(doc.stat()),
                view_lines: edit.total_lines,
                dirty: edit.dirty,
                revision: edit.revision,
                inserted_lines: edit.inserted_lines,
                replaced_lines: edit.replaced_lines,
                deleted_lines: edit.deleted_lines,
                can_undo: edit.can_undo,
                can_redo: edit.can_redo,
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
        },
    })
}

async fn api_stat(State(state): State<SharedState>) -> Json<StatResponse> {
    Json(stat_response(&state))
}

fn bad_request(e: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, e.to_string())
}

fn internal(e: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
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

    fn scratch_file(name: &str, contents: &[u8]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ayame-serve-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!(
            "{}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            name
        ));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(contents).unwrap();
        path
    }

    /// Serve the real router (loopback policy) on an ephemeral port.
    async fn start_server(path: &Path) -> SocketAddr {
        start_server_with_state(path).await.0
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

    /// Minimal raw HTTP/1.1 client (avoids extra dev-dependencies): returns
    /// (status, body-ish text — headers stripped, chunk framing tolerated).
    async fn send(addr: SocketAddr, raw: String) -> (u16, String) {
        let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
        s.write_all(raw.as_bytes()).await.unwrap();
        let mut buf = Vec::new();
        s.read_to_end(&mut buf).await.unwrap();
        let text = String::from_utf8_lossy(&buf).to_string();
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

    #[test]
    fn verbatim_prefix_is_stripped_for_display() {
        use super::workspace::strip_verbatim;
        assert_eq!(strip_verbatim(r"\\?\C:\Users\x\f.txt"), r"C:\Users\x\f.txt");
        assert_eq!(strip_verbatim(r"\\?\UNC\srv\share\f"), r"\\srv\share\f");
        assert_eq!(strip_verbatim("/tmp/f.txt"), "/tmp/f.txt");
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
