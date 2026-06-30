//! `ayame serve` — a local web editor for one large file.
//!
//! The browser only ever holds the visible viewport; everything else is fetched
//! on demand from these endpoints, which are thin wrappers over `ayame-core`.
//! A `CatchPanicLayer` turns any unexpected panic in a single request into a
//! 500 instead of taking the process down — stability is a feature here.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};
use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use ayame_core::{
    Document, EditLine, EditSession, EditStats, FileStat, Line, SaveResult, SearchOptions,
};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::{timeout, Duration};
use tower_http::catch_panic::CatchPanicLayer;

use crate::{first_opt, open_opts, parse};

const INDEX_HTML: &str = include_str!("../web/index.html");
const APP_JS: &str = include_str!("../web/app.js");
const STYLE_CSS: &str = include_str!("../web/style.css");
const FAVICON_SVG: &str = include_str!("../web/favicon.svg");

/// Hard cap on lines returned in one viewport request, so a hostile/buggy
/// client can never ask us to materialize the whole file.
const MAX_VIEW: u64 = 20_000;
const WORKER_TIMEOUT: Duration = Duration::from_secs(300);

type Shared = Arc<Document>;

struct AppState {
    doc: RwLock<Shared>,
    edits: RwLock<EditSession>,
}

impl AppState {
    fn new(doc: Document) -> AppState {
        AppState {
            doc: RwLock::new(Arc::new(doc)),
            edits: RwLock::new(EditSession::default()),
        }
    }

    fn doc(&self) -> Shared {
        self.doc.read().expect("document lock poisoned").clone()
    }

    fn edits(&self) -> EditSession {
        self.edits.read().expect("edit lock poisoned").clone()
    }
}

type SharedState = Arc<AppState>;

pub fn cmd_serve(args: &[String]) -> Result<()> {
    let (pos, opts, flags) = parse(
        args,
        &["--encoding", "--stride", "--host", "--port", "--cache-dir"],
    );
    let path = pos.first().context("expected a FILE argument")?.clone();
    let host = first_opt(&opts, &["--host"])
        .unwrap_or("127.0.0.1")
        .to_string();
    let port: u16 = first_opt(&opts, &["--port"])
        .unwrap_or("8777")
        .parse()
        .context("--port must be a number")?;

    eprintln!("ayame: opening and indexing '{path}' …");
    let doc = Document::open(&path, &open_opts(&opts, &flags)?)
        .with_context(|| format!("opening '{path}'"))?;
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

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("building tokio runtime")?;
    rt.block_on(serve(Arc::new(AppState::new(doc)), host, port))
}

async fn serve(state: SharedState, host: String, port: u16) -> Result<()> {
    let app = Router::new()
        .route("/", get(index))
        .route("/app.js", get(app_js))
        .route("/style.css", get(style_css))
        .route("/favicon.svg", get(favicon_svg))
        .route("/api/stat", get(api_stat))
        .route("/api/lines", get(api_lines))
        .route("/api/edit/status", get(api_edit_status))
        .route("/api/edit/line", post(api_edit_line))
        .route("/api/edit/insert", post(api_edit_insert))
        .route("/api/edit/delete", post(api_edit_delete))
        .route("/api/edit/save", post(api_edit_save))
        .route("/api/edit/revert", post(api_edit_revert))
        .route("/api/sort/save", post(api_sort_save))
        .route("/api/replace/save", post(api_replace_save))
        .route("/api/case/save", post(api_case_save))
        .route("/api/search", get(api_search))
        .route("/api/find", get(api_find))
        .route("/api/linebyte", get(api_linebyte))
        .route("/api/sort", get(api_sort))
        .route("/api/group", get(api_group))
        .route("/api/top", get(api_top))
        .route("/api/distinct", get(api_distinct))
        .layer(CatchPanicLayer::new())
        .with_state(state);

    let addr: SocketAddr = format!("{host}:{port}")
        .parse()
        .context("invalid host/port")?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    eprintln!("ayame: editor ready at http://{addr}/  (Ctrl+C to stop)");

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
            eprintln!("\nayame: shutting down");
        })
        .await
        .context("server error")?;
    Ok(())
}

// ---- static assets ------------------------------------------------------------

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn app_js() -> Response {
    asset("application/javascript; charset=utf-8", APP_JS)
}

async fn style_css() -> Response {
    asset("text/css; charset=utf-8", STYLE_CSS)
}

async fn favicon_svg() -> Response {
    asset("image/svg+xml; charset=utf-8", FAVICON_SVG)
}

fn asset(content_type: &'static str, body: &'static str) -> Response {
    ([(header::CONTENT_TYPE, content_type)], body).into_response()
}

// ---- API ----------------------------------------------------------------------

#[derive(Serialize)]
struct StatResponse {
    #[serde(flatten)]
    file: FileStat,
    view_lines: u64,
    dirty: bool,
    revision: u64,
    inserted_lines: u64,
    replaced_lines: u64,
    deleted_lines: u64,
}

async fn api_stat(State(state): State<SharedState>) -> Json<StatResponse> {
    let doc = state.doc();
    let edits = state.edits();
    let edit = edits.stats(&doc);
    Json(StatResponse {
        file: doc.stat(),
        view_lines: edit.total_lines,
        dirty: edit.dirty,
        revision: edit.revision,
        inserted_lines: edit.inserted_lines,
        replaced_lines: edit.replaced_lines,
        deleted_lines: edit.deleted_lines,
    })
}

#[derive(Deserialize)]
struct LinesQuery {
    start: u64,
    count: u64,
}

#[derive(Serialize)]
struct LinesResponse {
    start: u64,
    total: u64,
    lines: Vec<EditLine>,
}

async fn api_lines(
    State(state): State<SharedState>,
    Query(q): Query<LinesQuery>,
) -> Json<LinesResponse> {
    let doc = state.doc();
    let edits = state.edits();
    let count = q.count.min(MAX_VIEW);
    Json(LinesResponse {
        start: q.start,
        total: edits.total_lines(&doc),
        lines: edits.lines(&doc, q.start, count),
    })
}

async fn api_edit_status(State(state): State<SharedState>) -> Json<EditStats> {
    let doc = state.doc();
    Json(state.edits().stats(&doc))
}

#[derive(Deserialize)]
struct EditLineRequest {
    line: u64,
    text: String,
}

async fn api_edit_line(
    State(state): State<SharedState>,
    Json(req): Json<EditLineRequest>,
) -> Result<Json<EditStats>, (StatusCode, String)> {
    let doc = state.doc();
    let mut edits = state.edits.write().expect("edit lock poisoned");
    edits
        .replace_line(&doc, req.line, req.text)
        .map_err(bad_request)?;
    Ok(Json(edits.stats(&doc)))
}

async fn api_edit_insert(
    State(state): State<SharedState>,
    Json(req): Json<EditLineRequest>,
) -> Result<Json<EditStats>, (StatusCode, String)> {
    let doc = state.doc();
    let mut edits = state.edits.write().expect("edit lock poisoned");
    edits
        .insert_line_before(&doc, req.line, req.text)
        .map_err(bad_request)?;
    Ok(Json(edits.stats(&doc)))
}

#[derive(Deserialize)]
struct EditDeleteRequest {
    line: u64,
}

async fn api_edit_delete(
    State(state): State<SharedState>,
    Json(req): Json<EditDeleteRequest>,
) -> Result<Json<EditStats>, (StatusCode, String)> {
    let doc = state.doc();
    let mut edits = state.edits.write().expect("edit lock poisoned");
    edits.delete_line(&doc, req.line).map_err(bad_request)?;
    Ok(Json(edits.stats(&doc)))
}

#[derive(Deserialize)]
struct EditSaveRequest {
    #[serde(default)]
    path: Option<String>,
}

#[derive(Serialize)]
struct ArtifactResponse {
    path: PathBuf,
    bytes: u64,
    lines: u64,
}

async fn api_edit_save(
    State(state): State<SharedState>,
    Json(req): Json<EditSaveRequest>,
) -> Result<Json<SaveResult>, (StatusCode, String)> {
    let doc = state.doc();
    let edits = state.edits();
    let target = req
        .path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| default_save_copy_path(doc.path()));
    let res = tokio::task::spawn_blocking(move || edits.save_to_path(&doc, target))
        .await
        .map_err(internal)?
        .map_err(bad_request)?;
    Ok(Json(res))
}

async fn api_edit_revert(State(state): State<SharedState>) -> Json<EditStats> {
    let doc = state.doc();
    let mut edits = state.edits.write().expect("edit lock poisoned");
    edits.clear();
    Json(edits.stats(&doc))
}

#[derive(Deserialize)]
struct SortSaveRequest {
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    key: Option<usize>,
    #[serde(default)]
    numeric: bool,
    #[serde(default)]
    reverse: bool,
    #[serde(default)]
    delim: Option<String>,
}

async fn api_sort_save(
    State(state): State<SharedState>,
    Json(req): Json<SortSaveRequest>,
) -> Result<Json<ArtifactResponse>, (StatusCode, String)> {
    let doc = state.doc();
    let target = requested_or_default(doc.path(), req.path.as_deref(), "sorted");
    let exe = std::env::current_exe().map_err(internal)?;
    let dir = spawn_dir("sort-save");
    tokio::fs::create_dir_all(&dir).await.map_err(internal)?;
    let mut cmd = Command::new(&exe);
    cmd.arg("sort").arg(doc.path()).arg("--out").arg(&target);
    if let Some(k) = req.key {
        cmd.arg("--key").arg(k.to_string());
    }
    if req.numeric {
        cmd.arg("--numeric");
    }
    if req.reverse {
        cmd.arg("--reverse");
    }
    if let Some(d) = req.delim.as_deref().filter(|d| !d.is_empty()) {
        cmd.arg("--delim").arg(d);
    }
    cmd.arg("--spill-dir").arg(&dir);
    let res = run_artifact_worker("sort", &mut cmd, &target, doc.line_count()).await;
    let _ = tokio::fs::remove_dir_all(&dir).await;
    res.map(Json)
}

#[derive(Deserialize)]
struct ReplaceSaveRequest {
    #[serde(default)]
    path: Option<String>,
    find: String,
    replacement: String,
    #[serde(default)]
    regex: bool,
    #[serde(default)]
    ci: bool,
}

async fn api_replace_save(
    State(state): State<SharedState>,
    Json(req): Json<ReplaceSaveRequest>,
) -> Result<Json<ArtifactResponse>, (StatusCode, String)> {
    let doc = state.doc();
    let target = requested_or_default(doc.path(), req.path.as_deref(), "replaced");
    let exe = std::env::current_exe().map_err(internal)?;
    let mut cmd = Command::new(&exe);
    cmd.arg("replace")
        .arg(doc.path())
        .arg(req.find)
        .arg(req.replacement)
        .arg("--out")
        .arg(&target);
    if req.regex {
        cmd.arg("--regex");
    }
    if req.ci {
        cmd.arg("--ignore-case");
    }
    run_artifact_worker("replace", &mut cmd, &target, doc.line_count())
        .await
        .map(Json)
}

#[derive(Deserialize)]
struct CaseSaveRequest {
    #[serde(default)]
    path: Option<String>,
    mode: String,
}

async fn api_case_save(
    State(state): State<SharedState>,
    Json(req): Json<CaseSaveRequest>,
) -> Result<Json<ArtifactResponse>, (StatusCode, String)> {
    let doc = state.doc();
    let mode = req.mode.trim().to_ascii_lowercase();
    if !matches!(mode.as_str(), "upper" | "lower") {
        return Err(bad_request("mode must be upper or lower"));
    }
    let target = requested_or_default(doc.path(), req.path.as_deref(), &mode);
    let exe = std::env::current_exe().map_err(internal)?;
    let mut cmd = Command::new(&exe);
    cmd.arg("case")
        .arg(doc.path())
        .arg(mode)
        .arg("--out")
        .arg(&target);
    run_artifact_worker("case", &mut cmd, &target, doc.line_count())
        .await
        .map(Json)
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
    #[serde(default)]
    regex: bool,
    #[serde(default)]
    ci: bool,
    #[serde(default)]
    word: bool,
    #[serde(default)]
    start: u64,
    #[serde(default = "default_max")]
    max: usize,
}

fn default_max() -> usize {
    2000
}

async fn api_search(
    State(state): State<SharedState>,
    Query(q): Query<SearchQuery>,
) -> Result<Json<ayame_core::SearchResult>, (StatusCode, String)> {
    let doc = state.doc();
    let res = doc
        .search(&SearchOptions {
            query: q.q,
            regex: q.regex,
            case_sensitive: !q.ci,
            whole_word: q.word,
            start_byte: q.start,
            max_hits: q.max.min(100_000),
        })
        .map_err(bad_request)?;
    Ok(Json(res))
}

#[derive(Deserialize)]
struct FindQuery {
    q: String,
    #[serde(default)]
    regex: bool,
    #[serde(default)]
    ci: bool,
    #[serde(default)]
    word: bool,
    /// "next" or "prev".
    dir: String,
    /// Anchor byte: search starts at/after (next) or strictly before (prev).
    #[serde(default)]
    from: u64,
}

#[derive(Serialize)]
struct FindResponse {
    hit: Option<ayame_core::SearchHit>,
}

async fn api_find(
    State(state): State<SharedState>,
    Query(q): Query<FindQuery>,
) -> Result<Json<FindResponse>, (StatusCode, String)> {
    let doc = state.doc();
    let hit = if q.dir == "prev" {
        doc.find_prev(&q.q, q.regex, !q.ci, q.word, q.from)
            .map_err(bad_request)?
    } else {
        doc.find_next(&q.q, q.regex, !q.ci, q.word, q.from)
            .map_err(bad_request)?
    };
    Ok(Json(FindResponse { hit }))
}

#[derive(Deserialize)]
struct LineByteQuery {
    line: u64,
}

#[derive(Serialize)]
struct LineByteResponse {
    byte: Option<u64>,
}

async fn api_linebyte(
    State(state): State<SharedState>,
    Query(q): Query<LineByteQuery>,
) -> Json<LineByteResponse> {
    let doc = state.doc();
    Json(LineByteResponse {
        byte: doc.line_start_byte(q.line),
    })
}

fn bad_request(e: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, e.to_string())
}

fn default_save_copy_path(path: &Path) -> PathBuf {
    default_suffix_path(path, "edited")
}

fn requested_or_default(path: &Path, requested: Option<&str>, suffix: &str) -> PathBuf {
    requested
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| default_suffix_path(path, suffix))
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

// ---- isolated op workers (sort / group) -------------------------------------
//
// Heavy or risky operations run in a SEPARATE PROCESS spawned per request. If a
// worker blows up — even an uncatchable SIGABRT/OOM — only that child dies; this
// engine keeps serving the viewport and every other request. That process
// boundary is the real "designed to crash" guarantee.

static REQ_SEQ: AtomicU64 = AtomicU64::new(0);
const ARTIFACT_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);

fn internal(e: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

fn spawn_dir(kind: &str) -> std::path::PathBuf {
    let n = REQ_SEQ.fetch_add(1, AtomicOrdering::Relaxed);
    std::env::temp_dir().join(format!("ayame-srv-{kind}-{}-{}", std::process::id(), n))
}

async fn run_artifact_worker(
    kind: &str,
    cmd: &mut Command,
    target: &Path,
    lines: u64,
) -> Result<ArtifactResponse, (StatusCode, String)> {
    cmd.stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let status = wait_worker_for(kind, cmd, ARTIFACT_TIMEOUT).await?;
    if !status.success() {
        return Err((
            StatusCode::BAD_GATEWAY,
            format!(
                "{kind} worker {} — the engine is unaffected",
                describe_status(status)
            ),
        ));
    }
    let bytes = tokio::fs::metadata(target).await.map_err(internal)?.len();
    Ok(ArtifactResponse {
        path: target.to_path_buf(),
        bytes,
        lines,
    })
}

fn describe_status(s: std::process::ExitStatus) -> String {
    if let Some(c) = s.code() {
        format!("failed (exit code {c})")
    } else {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            if let Some(sig) = s.signal() {
                return format!("crashed (signal {sig})");
            }
        }
        "terminated abnormally".to_string()
    }
}

#[derive(Deserialize)]
struct SortQuery {
    #[serde(default)]
    k: Option<usize>,
    #[serde(default)]
    numeric: bool,
    #[serde(default)]
    reverse: bool,
    #[serde(default)]
    delim: Option<String>,
    #[serde(default = "preview_limit")]
    limit: u64,
}
fn preview_limit() -> u64 {
    500
}

#[derive(Serialize)]
struct SortResponse {
    total: u64,
    lines: Vec<Line>,
    truncated: bool,
}

async fn api_sort(
    State(state): State<SharedState>,
    Query(q): Query<SortQuery>,
) -> Result<Json<SortResponse>, (StatusCode, String)> {
    let doc = state.doc();
    let exe = std::env::current_exe().map_err(internal)?;
    let dir = spawn_dir("sort");
    tokio::fs::create_dir_all(&dir).await.map_err(internal)?;
    let order = dir.join("order.bin");

    let mut cmd = Command::new(&exe);
    cmd.arg("sort").arg(doc.path());
    if let Some(k) = q.k {
        cmd.arg("--key").arg(k.to_string());
    }
    if q.numeric {
        cmd.arg("--numeric");
    }
    if q.reverse {
        cmd.arg("--reverse");
    }
    if let Some(d) = q.delim.as_deref().filter(|d| !d.is_empty()) {
        cmd.arg("--delim").arg(d);
    }
    cmd.arg("--out-order")
        .arg(&order)
        .arg("--spill-dir")
        .arg(&dir);
    cmd.stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);

    let outcome = run_sort_worker(cmd, doc.as_ref(), &order, q.limit).await;
    let _ = tokio::fs::remove_dir_all(&dir).await; // gentle cleanup, success or not
    outcome
}

async fn run_sort_worker(
    mut cmd: Command,
    doc: &Document,
    order: &std::path::Path,
    limit: u64,
) -> Result<Json<SortResponse>, (StatusCode, String)> {
    let status = wait_worker("sort", &mut cmd).await?;
    if !status.success() {
        return Err((
            StatusCode::BAD_GATEWAY,
            format!(
                "sort worker {} — the engine is unaffected",
                describe_status(status)
            ),
        ));
    }
    let total = tokio::fs::metadata(order).await.map_err(internal)?.len() / 8;
    // Only read the previewed prefix of the ordering, never the whole thing
    // (which could be tens of GB for a ten-billion-line sort).
    let want = (limit.min(total) * 8) as usize;
    let mut buf = vec![0u8; want];
    let mut f = tokio::fs::File::open(order).await.map_err(internal)?;
    f.read_exact(&mut buf).await.map_err(internal)?;
    let lines = lines_from_ordering_prefix(doc, &buf);
    Ok(Json(SortResponse {
        total,
        lines,
        truncated: total > limit,
    }))
}

#[derive(Deserialize)]
struct GroupQuery {
    #[serde(default)]
    k: Option<usize>,
    #[serde(default)]
    value: Option<usize>,
    #[serde(default)]
    delim: Option<String>,
    #[serde(default = "group_limit")]
    limit: usize,
}
fn group_limit() -> usize {
    1000
}

#[derive(Serialize)]
struct GroupResponse {
    rows: Vec<String>,
    truncated: bool,
}

async fn api_group(
    State(state): State<SharedState>,
    Query(q): Query<GroupQuery>,
) -> Result<Json<GroupResponse>, (StatusCode, String)> {
    let doc = state.doc();
    let exe = std::env::current_exe().map_err(internal)?;
    let dir = spawn_dir("group");
    tokio::fs::create_dir_all(&dir).await.map_err(internal)?;
    let groups = dir.join("groups.tsv");

    let mut cmd = Command::new(&exe);
    cmd.arg("group").arg(doc.path());
    if let Some(k) = q.k {
        cmd.arg("--key").arg(k.to_string());
    }
    if let Some(v) = q.value {
        cmd.arg("--value").arg(v.to_string());
    }
    if let Some(d) = q.delim.as_deref().filter(|d| !d.is_empty()) {
        cmd.arg("--delim").arg(d);
    }
    cmd.arg("--spill-dir")
        .arg(&dir)
        .arg("--out-groups")
        .arg(&groups);
    cmd.stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);

    let status = wait_worker("group", &mut cmd).await;
    let rows = match status {
        Ok(status) if status.success() => read_group_preview(&groups, q.limit).await,
        Ok(status) => Err((
            StatusCode::BAD_GATEWAY,
            format!(
                "group worker {} — the engine is unaffected",
                describe_status(status)
            ),
        )),
        Err(e) => Err(e),
    };
    let _ = tokio::fs::remove_dir_all(&dir).await;
    rows.map(Json)
}

async fn wait_worker(
    kind: &str,
    cmd: &mut Command,
) -> Result<std::process::ExitStatus, (StatusCode, String)> {
    wait_worker_for(kind, cmd, WORKER_TIMEOUT).await
}

async fn wait_worker_for(
    kind: &str,
    cmd: &mut Command,
    timeout_after: Duration,
) -> Result<std::process::ExitStatus, (StatusCode, String)> {
    let mut child = cmd.spawn().map_err(internal)?;
    match timeout(timeout_after, child.wait()).await {
        Ok(waited) => waited.map_err(internal),
        Err(_) => {
            let _ = child.kill().await;
            Err((
                StatusCode::GATEWAY_TIMEOUT,
                format!(
                    "{kind} worker timed out after {}s — the engine is unaffected",
                    timeout_after.as_secs()
                ),
            ))
        }
    }
}

async fn read_group_preview(
    path: &std::path::Path,
    limit: usize,
) -> Result<GroupResponse, (StatusCode, String)> {
    let file = tokio::fs::File::open(path).await.map_err(internal)?;
    let mut reader = tokio::io::BufReader::new(file);
    let mut rows = Vec::new();
    let mut line = String::new();
    while rows.len() < limit {
        line.clear();
        let n = reader.read_line(&mut line).await.map_err(internal)?;
        if n == 0 {
            break;
        }
        trim_line_end(&mut line);
        rows.push(line.clone());
    }
    line.clear();
    let truncated = reader.read_line(&mut line).await.map_err(internal)? > 0;
    Ok(GroupResponse { rows, truncated })
}

fn trim_line_end(s: &mut String) {
    while matches!(s.as_bytes().last(), Some(b'\n' | b'\r')) {
        s.pop();
    }
}

#[derive(Deserialize)]
struct TopQuery {
    #[serde(default)]
    k: Option<usize>,
    #[serde(default)]
    numeric: bool,
    #[serde(default)]
    min: bool,
    #[serde(default)]
    delim: Option<String>,
    #[serde(default = "top_limit")]
    n: usize,
}
fn top_limit() -> usize {
    10
}

#[derive(Serialize)]
struct TopResponse {
    lines: Vec<Line>,
}

async fn api_top(
    State(state): State<SharedState>,
    Query(q): Query<TopQuery>,
) -> Result<Json<TopResponse>, (StatusCode, String)> {
    let doc = state.doc();
    let exe = std::env::current_exe().map_err(internal)?;
    let dir = spawn_dir("top");
    tokio::fs::create_dir_all(&dir).await.map_err(internal)?;
    let order = dir.join("top.bin");
    let n = q.n.min(10_000);

    let mut cmd = Command::new(&exe);
    cmd.arg("top").arg(doc.path()).arg("-n").arg(n.to_string());
    if let Some(k) = q.k {
        cmd.arg("--key").arg(k.to_string());
    }
    if q.numeric {
        cmd.arg("--numeric");
    }
    if q.min {
        cmd.arg("--min");
    }
    if let Some(d) = q.delim.as_deref().filter(|d| !d.is_empty()) {
        cmd.arg("--delim").arg(d);
    }
    cmd.arg("--out-order").arg(&order);
    cmd.stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);

    let result = match wait_worker("top", &mut cmd).await {
        Ok(status) if status.success() => read_top_ordering(doc.as_ref(), &order, n as u64).await,
        Ok(status) => Err((
            StatusCode::BAD_GATEWAY,
            format!(
                "top worker {} — the engine is unaffected",
                describe_status(status)
            ),
        )),
        Err(e) => Err(e),
    };
    let _ = tokio::fs::remove_dir_all(&dir).await;
    result.map(Json)
}

async fn read_top_ordering(
    doc: &Document,
    order: &std::path::Path,
    limit: u64,
) -> Result<TopResponse, (StatusCode, String)> {
    let total = tokio::fs::metadata(order).await.map_err(internal)?.len() / 8;
    let want = (limit.min(total) * 8) as usize;
    let mut buf = vec![0u8; want];
    let mut f = tokio::fs::File::open(order).await.map_err(internal)?;
    f.read_exact(&mut buf).await.map_err(internal)?;
    Ok(TopResponse {
        lines: lines_from_ordering_prefix(doc, &buf),
    })
}

fn lines_from_ordering_prefix(doc: &Document, buf: &[u8]) -> Vec<Line> {
    buf.chunks_exact(8)
        .filter_map(|c| {
            let ln = u64::from_le_bytes(c.try_into().unwrap());
            doc.line(ln).map(|text| Line { number: ln, text })
        })
        .collect()
}

#[derive(Deserialize)]
struct DistinctQuery {
    #[serde(default)]
    k: Option<usize>,
    #[serde(default)]
    delim: Option<String>,
    #[serde(default)]
    precision: Option<u32>,
}

#[derive(Serialize)]
struct DistinctResponse {
    estimate: u64,
}

async fn api_distinct(
    State(state): State<SharedState>,
    Query(q): Query<DistinctQuery>,
) -> Result<Json<DistinctResponse>, (StatusCode, String)> {
    let doc = state.doc();
    let exe = std::env::current_exe().map_err(internal)?;
    let mut cmd = Command::new(&exe);
    cmd.arg("distinct").arg(doc.path());
    if let Some(k) = q.k {
        cmd.arg("--key").arg(k.to_string());
    }
    if let Some(d) = q.delim.as_deref().filter(|d| !d.is_empty()) {
        cmd.arg("--delim").arg(d);
    }
    if let Some(p) = q.precision {
        cmd.arg("--precision").arg(p.to_string());
    }
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);

    let out = wait_worker_output("distinct", &mut cmd).await?;
    if !out.status.success() {
        return Err((
            StatusCode::BAD_GATEWAY,
            format!(
                "distinct worker {} — the engine is unaffected",
                describe_status(out.status)
            ),
        ));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let estimate = text
        .lines()
        .next()
        .unwrap_or("0")
        .trim()
        .parse::<u64>()
        .map_err(bad_request)?;
    Ok(Json(DistinctResponse { estimate }))
}

async fn wait_worker_output(
    kind: &str,
    cmd: &mut Command,
) -> Result<std::process::Output, (StatusCode, String)> {
    let child = cmd.spawn().map_err(internal)?;
    match timeout(WORKER_TIMEOUT, child.wait_with_output()).await {
        Ok(waited) => waited.map_err(internal),
        Err(_) => Err((
            StatusCode::GATEWAY_TIMEOUT,
            format!(
                "{kind} worker timed out after {}s — the engine is unaffected",
                WORKER_TIMEOUT.as_secs()
            ),
        )),
    }
}
