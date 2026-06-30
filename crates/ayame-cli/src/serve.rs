//! `ayame serve` — a local web viewer for one large file.
//!
//! The browser only ever holds the visible viewport; everything else is fetched
//! on demand from these endpoints, which are thin wrappers over `ayame-core`.
//! A `CatchPanicLayer` turns any unexpected panic in a single request into a
//! 500 instead of taking the process down — stability is a feature here.

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use ayame_core::{Document, SearchOptions};
use serde::{Deserialize, Serialize};
use tower_http::catch_panic::CatchPanicLayer;

use crate::{first_opt, open_opts, parse};

const INDEX_HTML: &str = include_str!("../web/index.html");
const APP_JS: &str = include_str!("../web/app.js");
const STYLE_CSS: &str = include_str!("../web/style.css");

/// Hard cap on lines returned in one viewport request, so a hostile/buggy
/// client can never ask us to materialize the whole file.
const MAX_VIEW: u64 = 20_000;

type Shared = Arc<Document>;

pub fn cmd_serve(args: &[String]) -> Result<()> {
    let (pos, opts, _flags) = parse(args, &["--encoding", "--stride", "--host", "--port"]);
    let path = pos.first().context("expected a FILE argument")?.clone();
    let host = first_opt(&opts, &["--host"]).unwrap_or("127.0.0.1").to_string();
    let port: u16 = first_opt(&opts, &["--port"]).unwrap_or("8777").parse().context("--port must be a number")?;

    eprintln!("ayame: opening and indexing '{path}' …");
    let doc = Document::open(&path, &open_opts(&opts)?)
        .with_context(|| format!("opening '{path}'"))?;
    let s = doc.stat();
    eprintln!(
        "ayame: {} lines, {} bytes, {} — indexed in {} ms ({} checkpoints, {} bytes resident)",
        crate::commas(s.lines),
        crate::commas(s.bytes),
        s.encoding.label(),
        s.index_ms,
        crate::commas(s.checkpoints as u64),
        crate::commas(s.index_bytes as u64),
    );

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("building tokio runtime")?;
    rt.block_on(serve(Arc::new(doc), host, port))
}

async fn serve(doc: Shared, host: String, port: u16) -> Result<()> {
    let app = Router::new()
        .route("/", get(index))
        .route("/app.js", get(app_js))
        .route("/style.css", get(style_css))
        .route("/api/stat", get(api_stat))
        .route("/api/lines", get(api_lines))
        .route("/api/search", get(api_search))
        .route("/api/find", get(api_find))
        .route("/api/linebyte", get(api_linebyte))
        .layer(CatchPanicLayer::new())
        .with_state(doc);

    let addr: SocketAddr = format!("{host}:{port}").parse().context("invalid host/port")?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    eprintln!("ayame: viewer ready at http://{addr}/  (Ctrl+C to stop)");

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

fn asset(content_type: &'static str, body: &'static str) -> Response {
    ([(header::CONTENT_TYPE, content_type)], body).into_response()
}

// ---- API ----------------------------------------------------------------------

async fn api_stat(State(doc): State<Shared>) -> Json<ayame_core::FileStat> {
    Json(doc.stat())
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
    lines: Vec<ayame_core::Line>,
}

async fn api_lines(State(doc): State<Shared>, Query(q): Query<LinesQuery>) -> Json<LinesResponse> {
    let count = q.count.min(MAX_VIEW);
    Json(LinesResponse {
        start: q.start,
        total: doc.line_count(),
        lines: doc.lines(q.start, count),
    })
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
    #[serde(default)]
    regex: bool,
    #[serde(default)]
    ci: bool,
    #[serde(default)]
    start: u64,
    #[serde(default = "default_max")]
    max: usize,
}

fn default_max() -> usize {
    2000
}

async fn api_search(
    State(doc): State<Shared>,
    Query(q): Query<SearchQuery>,
) -> Result<Json<ayame_core::SearchResult>, (StatusCode, String)> {
    let res = doc
        .search(&SearchOptions {
            query: q.q,
            regex: q.regex,
            case_sensitive: !q.ci,
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
    State(doc): State<Shared>,
    Query(q): Query<FindQuery>,
) -> Result<Json<FindResponse>, (StatusCode, String)> {
    let hit = if q.dir == "prev" {
        doc.find_prev(&q.q, q.regex, !q.ci, q.from).map_err(bad_request)?
    } else {
        doc.find_next(&q.q, q.regex, !q.ci, q.from).map_err(bad_request)?
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
    State(doc): State<Shared>,
    Query(q): Query<LineByteQuery>,
) -> Json<LineByteResponse> {
    Json(LineByteResponse { byte: doc.line_start_byte(q.line) })
}

fn bad_request(e: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, e.to_string())
}
