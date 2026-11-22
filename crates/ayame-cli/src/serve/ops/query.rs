//! The read-only endpoints: search, recursive grep, incremental find and the
//! line-to-byte lookup.

use std::path::{Path, PathBuf};

use anyhow::Context;
use axum::extract::{Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use super::super::{bad_request, internal, workspace, ApiError, SharedState};
use super::worker::{
    append_worker_encoding, dirty_view, parse_worker_json, wait_worker_output, worker_command,
    WORKER_TIMEOUT,
};

// ---- search -------------------------------------------------------------------

#[derive(Deserialize)]
pub(in crate::serve) struct SearchQuery {
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

#[derive(Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(in crate::serve) struct SearchResponse {
    hits: Vec<ayame_core::SearchHit>,
    truncated: bool,
}

impl From<ayame_core::SearchResult> for SearchResponse {
    fn from(result: ayame_core::SearchResult) -> Self {
        Self {
            hits: result.hits,
            truncated: result.truncated,
        }
    }
}

pub(in crate::serve) async fn api_search(
    State(state): State<SharedState>,
    Query(q): Query<SearchQuery>,
) -> Result<Json<SearchResponse>, ApiError> {
    // Search what the user sees: a dirty buffer runs against the cached
    // materialized snapshot so hits (line numbers, byte anchors) line up with
    // the edited view — and repeated searches reuse one materialization.
    let view = dirty_view(&state).await?;
    // Compile the query here before spawning: the browser's regex dialect
    // accepts constructs the engine's does not (look-around, backreferences),
    // and a worker that rejects one exits unsuccessfully, which the 502 path
    // can only report as an opaque worker failure. Validating up front turns
    // it into the engine's own `search` error naming the construct.
    ayame_core::validate_search(
        &ayame_core::SearchOptions {
            query: q.q.clone(),
            regex: q.regex,
            case_sensitive: !q.ci,
            whole_word: q.word,
            start_byte: q.start,
            max_hits: q.max.min(100_000),
        },
        view.doc().stat().encoding,
    )?;
    let mut cmd = worker_command()?;
    cmd.arg("search").arg(view.path());
    append_worker_encoding(&mut cmd, view.doc());
    cmd.arg("--json")
        .arg("--max")
        .arg(q.max.min(100_000).to_string())
        .arg("--start-byte")
        .arg(q.start.to_string());
    if q.regex {
        cmd.arg("--regex");
    }
    if q.ci {
        cmd.arg("--ignore-case");
    }
    if q.word {
        cmd.arg("--whole-word");
    }
    cmd.arg("--").arg(q.q);

    let out = wait_worker_output(&state, "search", &mut cmd, WORKER_TIMEOUT).await;
    drop(view); // the snapshot guard must outlive the worker child
    let res: ayame_core::SearchResult = parse_worker_json("search", &out?)?;
    Ok(Json(res.into()))
}

// ---- grep (recursive directory search) -----------------------------------------

#[derive(Deserialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(in crate::serve) struct GrepRequest {
    query: String,
    /// Root directory to search; null/absent = the open file's directory.
    #[serde(default)]
    dir: Option<String>,
    /// Comma/space separated filename globs (`*.rs, *.toml`); empty = every file.
    #[serde(default)]
    glob: Option<String>,
    #[serde(default)]
    regex: bool,
    #[serde(default)]
    ci: bool,
    #[serde(default)]
    word: bool,
    #[serde(default = "grep_default_max")]
    max: usize,
}

fn grep_default_max() -> usize {
    2000
}

#[derive(Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(in crate::serve) struct GrepResponse {
    hits: Vec<ayame_core::GrepHit>,
    truncated: bool,
    files_scanned: u64,
    files_truncated: bool,
}

impl From<ayame_core::GrepResult> for GrepResponse {
    fn from(result: ayame_core::GrepResult) -> Self {
        Self {
            hits: result.hits,
            truncated: result.truncated,
            files_scanned: result.files_scanned,
            files_truncated: result.files_truncated,
        }
    }
}

/// `POST /api/grep` — recursive multi-file search. The heavy
/// work (a directory walk + a search over each file) runs inside
/// `spawn_blocking` and the structured hits are returned as JSON. This searches
/// on-disk files, so it does not see the active buffer's unsaved edits.
pub(in crate::serve) async fn api_grep(
    State(state): State<SharedState>,
    Json(req): Json<GrepRequest>,
) -> Result<Json<GrepResponse>, ApiError> {
    let query = req.query.trim().to_string();
    if query.is_empty() {
        return Err(bad_request("query is empty"));
    }
    // Default the root to the open file's directory so フォルダ内検索 works
    // without picking a folder first.
    let dir = match req.dir.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
        Some(d) => PathBuf::from(d),
        None => grep_default_dir(&state),
    };
    let glob = req.glob.unwrap_or_default();
    let regex = req.regex;
    let case_sensitive = !req.ci;
    let whole_word = req.word;
    let max = req.max.clamp(1, 20_000);
    // Same up-front compile as `/api/search`: a folder grep skips a file whose
    // query it cannot compile, so an unsupported pattern would otherwise look
    // like an empty result instead of naming the construct.
    ayame_core::validate_search(
        &ayame_core::SearchOptions {
            query: query.clone(),
            regex,
            case_sensitive,
            whole_word,
            start_byte: 0,
            max_hits: 1,
        },
        ayame_core::Encoding::Utf8,
    )?;
    let mut result =
        tokio::task::spawn_blocking(move || -> anyhow::Result<ayame_core::GrepResult> {
            let opts = ayame_core::GrepOptions {
                query,
                regex,
                case_sensitive,
                whole_word,
                glob,
                max_hits: max,
                ..Default::default()
            };
            ayame_core::grep_dir(&dir, &opts)
                .with_context(|| format!("searching {}", workspace::display_path(&dir)))
        })
        .await
        .map_err(internal)?
        .map_err(bad_request)?;
    // UI-facing hit paths: never leak a Windows verbatim prefix (the walk
    // inherits the root's prefix when the open file's path is canonical).
    for hit in &mut result.hits {
        hit.path = workspace::strip_verbatim(&hit.path);
    }
    Ok(Json(result.into()))
}

/// The directory a grep with no explicit root searches: the open file's parent,
/// else the server's current working directory.
fn grep_default_dir(state: &SharedState) -> PathBuf {
    state
        .doc_opt()
        .and_then(|doc| doc.path().parent().map(Path::to_path_buf))
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

// ---- find (incremental next/prev) ----------------------------------------------

#[derive(Deserialize)]
pub(in crate::serve) struct FindQuery {
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
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(in crate::serve) struct FindResponse {
    hit: Option<ayame_core::SearchHit>,
}

pub(in crate::serve) async fn api_find(
    State(state): State<SharedState>,
    Query(q): Query<FindQuery>,
) -> Result<Json<FindResponse>, ApiError> {
    // Find what the user sees: a dirty session runs against the revision-keyed
    // materialized snapshot (built once, cached), so returned line numbers and
    // byte offsets are view-accurate even with unsaved edits.
    let view = dirty_view(&state).await?;
    let doc = view.doc().clone();
    // Keep find work off the async workers so large searches never stall
    // unrelated requests.
    let hit = tokio::task::spawn_blocking(move || {
        if q.dir == "prev" {
            doc.find_prev(&q.q, q.regex, !q.ci, q.word, q.from)
        } else {
            doc.find_next(&q.q, q.regex, !q.ci, q.word, q.from)
        }
    })
    .await
    .map_err(internal)?
    .map_err(bad_request)?;
    Ok(Json(FindResponse { hit }))
}

// ---- misc ----------------------------------------------------------------------

#[derive(Deserialize)]
pub(in crate::serve) struct LineByteQuery {
    line: u64,
    #[serde(default)]
    col: Option<u64>,
}

#[derive(Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(in crate::serve) struct LineByteResponse {
    byte: Option<u64>,
}

pub(in crate::serve) async fn api_linebyte(
    State(state): State<SharedState>,
    Query(q): Query<LineByteQuery>,
) -> Result<Json<LineByteResponse>, ApiError> {
    let view = dirty_view(&state).await?;
    let doc = view.doc();
    let byte = match q.col {
        Some(col) => doc.line_col_byte(q.line, col),
        None => doc.line_start_byte(q.line),
    };
    Ok(Json(LineByteResponse { byte }))
}
