use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, LazyLock, Mutex as StdMutex};

use anyhow::Context;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use ayame_core::{Document, EditSession, Line, DEFAULT_PARALLEL_REPLACE_CHUNK_LINES};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::Notify;
use tokio::time::{timeout, Duration};

use crate::diff::{diff_documents, DiffKind, DiffResult};
use crate::temp_paths;

use super::error::{self, ApiError};
use super::state::DirtySnapshotCache;
use super::{bad_request, default_suffix_path, edit, internal, workspace, SharedState};

const WORKER_TIMEOUT: Duration = Duration::from_secs(300);
const ARTIFACT_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);
static REQ_SEQ: AtomicU64 = AtomicU64::new(0);

// ---- dirty-buffer materialization --------------------------------------------

/// The file an op worker subprocess should read: either the document's own
/// on-disk path (clean session) or a scratch copy with the unsaved edit
/// overlay applied, so workers see what the user sees. The scratch directory
/// is removed when this guard drops.
pub(super) enum WorkerInput {
    /// The session was clean — workers can read the original file directly.
    OnDisk(PathBuf),
    /// Dirty buffer materialized into a scratch file we own.
    Materialized { path: PathBuf, dir: PathBuf },
}

impl WorkerInput {
    pub(super) fn path(&self) -> &Path {
        match self {
            WorkerInput::OnDisk(p) => p,
            WorkerInput::Materialized { path, .. } => path,
        }
    }
}

impl Drop for WorkerInput {
    fn drop(&mut self) {
        if let WorkerInput::Materialized { dir, .. } = self {
            let _ = std::fs::remove_dir_all(&*dir);
        }
    }
}

/// Materialize the current buffer for a worker if (and only if) there are
/// unsaved edits. Blocking (streams the whole file when dirty) — call from a
/// blocking context. The temp dir is cleaned up even when the save fails,
/// because the guard is constructed before the write.
pub(super) fn materialize_worker_input(
    doc: &Document,
    dirty_edits: Option<&EditSession>,
    kind: &str,
) -> anyhow::Result<WorkerInput> {
    let Some(edits) = dirty_edits else {
        return Ok(WorkerInput::OnDisk(doc.path().to_path_buf()));
    };
    let dir = spawn_dir(kind).with_context(|| format!("creating private worker dir for {kind}"))?;
    let path = dir.join("current.txt");
    let input = WorkerInput::Materialized {
        path: path.clone(),
        dir,
    };
    edits
        .save_to_path(doc, &path)
        .context("materializing unsaved edits for the worker")?;
    Ok(input)
}

/// Snapshot the active document and hand back a worker-readable input path
/// that reflects unsaved edits, plus the effective (overlay-aware) line count.
struct WorkerDoc {
    doc: Arc<Document>,
    input: WorkerInput,
    total_lines: u64,
}

async fn dirty_aware_input(state: &SharedState, kind: &'static str) -> Result<WorkerDoc, ApiError> {
    let (doc, dirty) = state.doc_and_dirty_edits()?;
    let total_lines = match &dirty {
        Some(edits) => edits.total_lines(&doc),
        None => doc.line_count(),
    };
    let doc_for_write = doc.clone();
    let input = tokio::task::spawn_blocking(move || {
        materialize_worker_input(&doc_for_write, dirty.as_ref(), kind)
    })
    .await
    .map_err(internal)?
    .map_err(internal)?;
    Ok(WorkerDoc {
        doc,
        input,
        total_lines,
    })
}

/// What a dirty-aware *read* (find/search) runs against: the live document
/// when the session is clean, or the revision-keyed materialized snapshot
/// (built at most once per revision, cached in the app state) when it is
/// dirty. Hit line numbers / byte offsets are then view-accurate, because the
/// materialized file IS the view.
enum DirtyView {
    Clean(Arc<Document>),
    Snapshot {
        doc: Arc<Document>,
        /// Keeps the materialized temp alive for this request, even if the
        /// cache slot is invalidated while a search worker child still reads it.
        _input: Arc<WorkerInput>,
    },
}

impl DirtyView {
    fn doc(&self) -> &Arc<Document> {
        match self {
            DirtyView::Clean(doc) => doc,
            DirtyView::Snapshot { doc, .. } => doc,
        }
    }

    /// The on-disk file a worker child should read for this view.
    fn path(&self) -> &Path {
        self.doc().path()
    }
}

/// Get (or build) the [`DirtyView`] for the active session. Building = one
/// materialization + one `Document::open` (no index-cache dir, so throwaway
/// snapshots never pollute the on-disk cache), inside `spawn_blocking`.
/// Staleness is checked on every call: a revision bump or doc change drops
/// the old snapshot. Two racing builders at the same revision produce
/// equivalent snapshots; the last store wins and the loser's temp is cleaned
/// up when its guard drops.
async fn dirty_view(state: &SharedState) -> Result<DirtyView, ApiError> {
    let (doc, dirty) = state.doc_and_dirty_edits()?;
    let Some(edits) = dirty else {
        // Clean session: any cached snapshot is stale by definition (its
        // revision can't match a future dirty one) — drop it so its temp goes.
        state.invalidate_dirty_snapshot();
        return Ok(DirtyView::Clean(doc));
    };
    let revision = edits.revision();
    if let Some((snapshot, input)) = state.cached_dirty_snapshot(&doc, revision) {
        return Ok(DirtyView::Snapshot {
            doc: snapshot,
            _input: input,
        });
    }
    let scratch_opts = ayame_core::OpenOptions {
        cache_dir: None,
        // Force the live document's encoding. The snapshot bytes are written in
        // that encoding, and a user who reopened the file with a corrected
        // encoding (reload_with_encoding never updates state.open_opts) must not
        // have auto-detection silently pick the wrong one again (issue #75):
        // otherwise find/search on the dirty buffer runs under the wrong
        // encoding and Japanese queries stop matching.
        encoding: Some(doc.encoding()),
        ..state.open_options()
    };
    let doc_for_build = doc.clone();
    let (input, snapshot) = tokio::task::spawn_blocking(
        move || -> anyhow::Result<(Arc<WorkerInput>, Arc<Document>)> {
            let input = materialize_worker_input(&doc_for_build, Some(&edits), "dirty-view")?;
            let snapshot = Document::open(input.path(), &scratch_opts)
                .with_context(|| format!("opening snapshot {}", input.path().display()))?;
            Ok((Arc::new(input), Arc::new(snapshot)))
        },
    )
    .await
    .map_err(internal)?
    .map_err(internal)?;
    state.store_dirty_snapshot(DirtySnapshotCache {
        doc,
        revision,
        input: input.clone(),
        snapshot: snapshot.clone(),
    });
    Ok(DirtyView::Snapshot {
        doc: snapshot,
        _input: input,
    })
}

// ---- save-to-artifact endpoints -----------------------------------------------

#[derive(Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct ArtifactResponse {
    /// UI-facing path (verbatim prefix stripped) — see [`workspace::display_path`].
    path: String,
    bytes: u64,
    lines: u64,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct SortSaveRequest {
    #[serde(default)]
    path: Option<String>,
    /// Sort the open file onto itself (the `path` field is ignored): the sort
    /// output atomically replaces the file, the document reloads, and the
    /// (already incorporated) edit overlay is cleared.
    #[serde(default)]
    in_place: bool,
    #[serde(default)]
    key: Option<usize>,
    #[serde(default)]
    numeric: bool,
    #[serde(default)]
    reverse: bool,
    #[serde(default)]
    delim: Option<String>,
    /// Client-generated id used to poll `/api/op/progress` and hit
    /// `/api/op/cancel` while this (potentially minutes-long) worker runs (#78).
    #[serde(default)]
    op_id: Option<String>,
}

pub(super) async fn api_sort_save(
    State(state): State<SharedState>,
    Json(req): Json<SortSaveRequest>,
) -> Result<Json<ArtifactResponse>, ApiError> {
    if req.in_place {
        return sort_save_in_place(state, req).await;
    }
    let wd = dirty_aware_input(&state, "sort-save").await?;
    // No explicit destination: sort into a scratch file (the GUI opens the
    // result as a new tab instead of asking for a path first).
    let target = match req.path.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => PathBuf::from(p),
        None => default_sorted_temp_path(wd.doc.path()).map_err(internal)?,
    };
    let dir = spawn_dir("sort-save-spill").map_err(internal)?;
    let mut cmd = sort_command(&wd.doc, wd.input.path(), &target, &req, &dir)?;
    let res = run_artifact_worker(
        "sort",
        &mut cmd,
        &target,
        wd.total_lines,
        req.op_id.as_deref(),
    )
    .await;
    let _ = tokio::fs::remove_dir_all(&dir).await;
    drop(wd); // remove the materialized input, if any
    res.map(Json)
}

/// Sort the active document onto itself. Phase 1 runs the ordinary
/// dirty-aware sort worker into a stage file next to the target without
/// holding any lock; phase 2 commits with exactly the machinery (and the
/// guarantees) of the in-place save in [`edit`]: serialized against other
/// doc-slot transitions, revalidated against the snapshot's revision — edits
/// that arrived while the worker ran get a 409 and the stage is discarded —
/// then atomic replace, reload, and the overlay (already baked into the
/// sorted bytes) is cleared.
async fn sort_save_in_place(
    state: SharedState,
    req: SortSaveRequest,
) -> Result<Json<ArtifactResponse>, ApiError> {
    // Consistent snapshot (doc + edits + revision) under one lock acquisition.
    let mut snap = state.edit_snapshot()?;
    let target = snap.doc.path().to_path_buf();
    let total_lines = snap.edits.total_lines(&snap.doc);
    let dirty = snap.edits.is_dirty().then(|| snap.take_edits());
    let doc_for_input = snap.doc.clone();
    let input = tokio::task::spawn_blocking(move || {
        materialize_worker_input(&doc_for_input, dirty.as_ref(), "sort-save")
    })
    .await
    .map_err(internal)?
    .map_err(internal)?;

    // Phase 1 — sort into a stage sibling of the target (same volume, so the
    // final rename is atomic). No lock is held: typing stays live.
    let stage = edit::overwrite_stage_path(&target);
    let dir = spawn_dir("sort-save-spill").map_err(internal)?;
    let mut cmd = sort_command(&snap.doc, input.path(), &stage, &req, &dir)?;
    let res =
        run_artifact_worker("sort", &mut cmd, &stage, total_lines, req.op_id.as_deref()).await;
    let _ = tokio::fs::remove_dir_all(&dir).await;
    drop(input); // remove the materialized input, if any
    if let Err(e) = res {
        let _ = tokio::fs::remove_file(&stage).await;
        return Err(e);
    }

    // Phase 2 — commit. Serialized against every other doc-slot transition;
    // rejected with 409 (stage discarded) if edits or the tab changed while
    // the worker was sorting, so nothing is ever silently thrown away.
    let _transitions = state.lock_transitions().await;
    if let Err(e) = state.detach_for_overwrite(&snap) {
        let _ = tokio::fs::remove_file(&stage).await;
        return Err(e);
    }
    drop(snap); // release our mmap handle before replacing the file (Windows)

    let stage_for_replace = stage.clone();
    let target_for_replace = target.clone();
    let replaced = tokio::task::spawn_blocking(move || {
        edit::replace_existing_file(&stage_for_replace, &target_for_replace)
    })
    .await
    .map_err(internal)?;
    match replaced {
        Ok(bytes) => {
            // The sorted bytes on disk already include the pending overlay
            // (the worker read the materialized buffer), so the reloaded
            // document IS the edited text: clear the overlay.
            state.mark_edits_saved();
            state.install_reloaded(target.clone()).await?;
            Ok(Json(ArtifactResponse {
                path: workspace::display_path(&target),
                bytes,
                lines: total_lines,
            }))
        }
        Err(e) => {
            // Best effort: reopen the original document. The overlay was left
            // untouched by the detach, so the user's edits are still pending
            // (and the staged bytes are preserved on disk for recovery).
            let _ = state.install_reloaded(target).await;
            Err(internal(e))
        }
    }
}

/// Build the `ayame sort` worker invocation shared by both sort modes.
fn sort_command(
    doc: &Document,
    input: &Path,
    out: &Path,
    req: &SortSaveRequest,
    spill_dir: &Path,
) -> Result<Command, ApiError> {
    use crate::cli::wire;
    let mut cmd = worker_command()?;
    cmd.arg(wire::sort::CMD).arg(input);
    append_worker_encoding(&mut cmd, doc);
    cmd.arg(wire::OUT).arg(out);
    if let Some(k) = req.key {
        cmd.arg(wire::sort::KEY).arg(k.to_string());
    }
    if req.numeric {
        cmd.arg(wire::sort::NUMERIC);
    }
    if req.reverse {
        cmd.arg(wire::sort::REVERSE);
    }
    if let Some(d) = req.delim.as_deref().filter(|d| !d.is_empty()) {
        cmd.arg(wire::sort::DELIM).arg(d);
    }
    cmd.arg(wire::sort::SPILL_DIR).arg(spill_dir);
    Ok(cmd)
}

/// Where a sort lands when the client didn't pick a destination: a scratch
/// file under the OS temp dir, named after the source ("app.log" →
/// "app.sorted.log"), unique within this server's scratch directory.
fn default_sorted_temp_path(source: &Path) -> std::io::Result<PathBuf> {
    let dir = workspace::sorted_dir_result()?;
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "output".to_string());
    let ext = source
        .extension()
        .map(|s| format!(".{}", s.to_string_lossy()))
        .unwrap_or_default();
    Ok(workspace::unique_upload_path(
        &dir,
        &format!("{stem}.sorted{ext}"),
    ))
}

#[derive(Deserialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct ReplaceSaveRequest {
    #[serde(default)]
    path: Option<String>,
    find: String,
    replacement: String,
    #[serde(default)]
    regex: bool,
    #[serde(default)]
    ci: bool,
    #[serde(default)]
    jobs: Option<usize>,
    #[serde(default)]
    chunk_lines: Option<u64>,
    #[serde(default)]
    op_id: Option<String>,
}

pub(super) async fn api_replace_save(
    State(state): State<SharedState>,
    Json(req): Json<ReplaceSaveRequest>,
) -> Result<Json<ArtifactResponse>, ApiError> {
    let wd = dirty_aware_input(&state, "replace-save").await?;
    let target = requested_or_default(wd.doc.path(), req.path.as_deref(), "replaced");
    let mut cmd = worker_command()?;
    cmd.arg("replace").arg(wd.input.path());
    append_worker_encoding(&mut cmd, &wd.doc);
    cmd.arg(req.find)
        .arg(req.replacement)
        .arg("--out")
        .arg(&target);
    if req.regex {
        cmd.arg("--regex");
    }
    if req.ci {
        cmd.arg("--ignore-case");
    }
    cmd.arg("--jobs")
        .arg(req.jobs.unwrap_or(0).to_string())
        .arg("--chunk-lines")
        .arg(
            req.chunk_lines
                .unwrap_or(DEFAULT_PARALLEL_REPLACE_CHUNK_LINES)
                .to_string(),
        );
    let res = run_artifact_worker(
        "replace",
        &mut cmd,
        &target,
        wd.total_lines,
        req.op_id.as_deref(),
    )
    .await;
    drop(wd);
    res.map(Json)
}

#[derive(Deserialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct CaseSaveRequest {
    #[serde(default)]
    path: Option<String>,
    mode: String,
    #[serde(default)]
    jobs: Option<usize>,
    #[serde(default)]
    chunk_lines: Option<u64>,
    #[serde(default)]
    op_id: Option<String>,
}

pub(super) async fn api_case_save(
    State(state): State<SharedState>,
    Json(req): Json<CaseSaveRequest>,
) -> Result<Json<ArtifactResponse>, ApiError> {
    let mode = req.mode.trim().to_ascii_lowercase();
    if ayame_core::CaseMode::parse(&mode).is_none() {
        return Err(bad_request(
            "mode must be one of upper|lower|camel|pascal|snake|kebab|constant",
        ));
    }
    let wd = dirty_aware_input(&state, "case-save").await?;
    let target = requested_or_default(wd.doc.path(), req.path.as_deref(), &mode);
    let mut cmd = worker_command()?;
    cmd.arg("case").arg(wd.input.path());
    append_worker_encoding(&mut cmd, &wd.doc);
    cmd.arg(mode).arg("--out").arg(&target);
    // Same chunked-parallel plumbing as replace: case conversion is
    // line-local, so huge files scale with cores in the worker.
    cmd.arg("--jobs")
        .arg(req.jobs.unwrap_or(0).to_string())
        .arg("--chunk-lines")
        .arg(
            req.chunk_lines
                .unwrap_or(DEFAULT_PARALLEL_REPLACE_CHUNK_LINES)
                .to_string(),
        );
    let res = run_artifact_worker(
        "case",
        &mut cmd,
        &target,
        wd.total_lines,
        req.op_id.as_deref(),
    )
    .await;
    drop(wd);
    res.map(Json)
}

#[derive(Deserialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct GrepSaveRequest {
    /// Output file; null/absent = a `.grep` suffixed sibling default.
    #[serde(default)]
    path: Option<String>,
    query: String,
    #[serde(default)]
    regex: bool,
    #[serde(default)]
    ci: bool,
    #[serde(default)]
    word: bool,
    /// Replace an existing output file (the OS save dialog already asked).
    #[serde(default)]
    overwrite: bool,
    #[serde(default)]
    jobs: Option<usize>,
    #[serde(default)]
    chunk_lines: Option<u64>,
    #[serde(default)]
    op_id: Option<String>,
}

/// `POST /api/grep/save` — "grep して保存" (issue #38): extract every line of
/// the active document (unsaved edits included) matching the query into a new
/// file. The flags carry the search bar's exact semantics (`regex`, `ci`,
/// `word`), and the streaming worker keeps memory bounded on multi-GB files.
pub(super) async fn api_grep_save(
    State(state): State<SharedState>,
    Json(req): Json<GrepSaveRequest>,
) -> Result<Json<ArtifactResponse>, ApiError> {
    if req.query.is_empty() {
        return Err(bad_request("query is empty"));
    }
    let wd = dirty_aware_input(&state, "grep-save").await?;
    let target = requested_or_default(wd.doc.path(), req.path.as_deref(), "grep");
    // Checked here (same message as save-as) because the worker's stderr is
    // discarded: its own Conflict would surface as an opaque 502, and the web
    // overwrite-confirm flow keys off this text.
    if !req.overwrite && target.exists() {
        return Err(error::exists(&workspace::display_path(&target)));
    }
    let mut cmd = grep_save_command(&wd.doc, wd.input.path(), &target, &req)?;
    let res = run_artifact_worker(
        "grep",
        &mut cmd,
        &target,
        wd.total_lines,
        req.op_id.as_deref(),
    )
    .await;
    drop(wd);
    res.map(Json)
}

/// Build the `ayame grep-lines` worker invocation for [`api_grep_save`].
fn grep_save_command(
    doc: &Document,
    input: &Path,
    out: &Path,
    req: &GrepSaveRequest,
) -> Result<Command, ApiError> {
    use crate::cli::wire;
    let mut cmd = worker_command()?;
    cmd.arg(wire::grep_lines::CMD).arg(input);
    append_worker_encoding(&mut cmd, doc);
    cmd.arg(&req.query).arg(wire::OUT).arg(out);
    if req.regex {
        cmd.arg(wire::grep_lines::REGEX);
    }
    if req.ci {
        cmd.arg(wire::grep_lines::IGNORE_CASE);
    }
    if req.word {
        cmd.arg(wire::grep_lines::WHOLE_WORD);
    }
    if req.overwrite {
        cmd.arg(wire::grep_lines::OVERWRITE);
    }
    // Same chunked-parallel plumbing as replace: line extraction is
    // line-local, so huge files scale with cores in the worker.
    cmd.arg(wire::grep_lines::JOBS)
        .arg(req.jobs.unwrap_or(0).to_string())
        .arg(wire::grep_lines::CHUNK_LINES)
        .arg(
            req.chunk_lines
                .unwrap_or(DEFAULT_PARALLEL_REPLACE_CHUNK_LINES)
                .to_string(),
        );
    Ok(cmd)
}

// ---- split --------------------------------------------------------------------

#[derive(Deserialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct SplitSaveRequest {
    /// Lines per output part (must be >= 1).
    lines: u64,
    /// Output directory; null/absent = the source file's directory.
    #[serde(default)]
    dir: Option<String>,
    #[serde(default)]
    op_id: Option<String>,
}

/// `POST /api/split/save` — split the active document (unsaved edits included)
/// into `lines`-line part files. Runs as an isolated worker child, like sort:
/// the worker prints the created file list as JSON (`ayame split --json`) and
/// the response relays it verbatim: `{"files": [...], "count", "total_lines"}`
/// (`files` capped at the first [`ayame_core::SPLIT_RESULT_MAX_FILES`] paths;
/// `count` is the real total).
pub(super) async fn api_split_save(
    State(state): State<SharedState>,
    Json(req): Json<SplitSaveRequest>,
) -> Result<Json<ayame_core::SplitResult>, ApiError> {
    if req.lines == 0 {
        return Err(bad_request("lines must be at least 1"));
    }
    let wd = dirty_aware_input(&state, "split-save").await?;
    // Both the default output directory and the part-name stem come from the
    // ORIGINAL document path — never from the materialized temp snapshot.
    let source = wd.doc.path().to_path_buf();
    let out_dir = req
        .dir
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| default_split_dir(&source));
    let name = source
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "output".to_string());
    let mut cmd = worker_command()?;
    cmd.arg("split").arg(wd.input.path());
    append_worker_encoding(&mut cmd, &wd.doc);
    cmd.arg("--lines")
        .arg(req.lines.to_string())
        .arg("--out-dir")
        .arg(&out_dir)
        .arg("--name")
        .arg(&name)
        .arg("--json");
    if matches!(wd.input, WorkerInput::Materialized { .. }) {
        // The snapshot is read once and discarded: keep its index out of the
        // persistent cache.
        cmd.arg("--no-cache");
    }

    let out = match req.op_id.as_deref() {
        Some(id) => wait_worker_output_tracked("split", &mut cmd, wd.total_lines, id).await,
        None => wait_worker_output("split", &mut cmd, ARTIFACT_TIMEOUT).await,
    };
    drop(wd); // remove the materialized input, if any
    let mut res: ayame_core::SplitResult = parse_worker_json("split", &out?)?;
    // UI-facing part list: never leak a Windows verbatim prefix.
    for f in &mut res.files {
        *f = PathBuf::from(workspace::display_path(f));
    }
    Ok(Json(res))
}

/// Where parts land when the client didn't pick a directory: next to the
/// source file.
fn default_split_dir(source: &Path) -> PathBuf {
    source
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

// ---- search -------------------------------------------------------------------

#[derive(Deserialize)]
pub(super) struct SearchQuery {
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

pub(super) async fn api_search(
    State(state): State<SharedState>,
    Query(q): Query<SearchQuery>,
) -> Result<Json<ayame_core::SearchResult>, ApiError> {
    // Search what the user sees: a dirty buffer runs against the cached
    // materialized snapshot so hits (line numbers, byte anchors) line up with
    // the edited view — and repeated searches reuse one materialization.
    let view = dirty_view(&state).await?;
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

    let out = wait_worker_output("search", &mut cmd, WORKER_TIMEOUT).await;
    drop(view); // the snapshot guard must outlive the worker child
    let res: ayame_core::SearchResult = parse_worker_json("search", &out?)?;
    Ok(Json(res))
}

// ---- grep (recursive directory search) -----------------------------------------

#[derive(Deserialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct GrepRequest {
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

/// `POST /api/grep` — recursive multi-file search. Like `/api/diff`, the heavy
/// work (a directory walk + a search over each file) runs inside
/// `spawn_blocking` and the structured hits are returned as JSON. This searches
/// on-disk files, so it does not see the active buffer's unsaved edits.
pub(super) async fn api_grep(
    State(state): State<SharedState>,
    Json(req): Json<GrepRequest>,
) -> Result<Json<ayame_core::GrepResult>, ApiError> {
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
    Ok(Json(result))
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
pub(super) struct FindQuery {
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
pub(super) struct FindResponse {
    hit: Option<ayame_core::SearchHit>,
}

pub(super) async fn api_find(
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

// ---- diff ----------------------------------------------------------------------

#[derive(Deserialize)]
pub(super) struct DiffQuery {
    path: String,
    #[serde(default = "diff_max_hunks")]
    max_hunks: usize,
    #[serde(default = "diff_max_lines")]
    max_lines: u64,
    #[serde(default = "diff_window")]
    window: u64,
}

fn diff_max_hunks() -> usize {
    200
}

fn diff_window() -> u64 {
    128
}

fn diff_max_lines() -> u64 {
    80
}

#[derive(Serialize)]
pub(super) struct WebDiffResponse {
    old_path: String,
    new_path: String,
    current_dirty: bool,
    old_lines: u64,
    new_lines: u64,
    hunks: Vec<WebDiffHunk>,
    hunk_count: u64,
    omitted_hunks: u64,
    added: u64,
    deleted: u64,
    modified: u64,
    max_lines_per_hunk: u64,
}

#[derive(Serialize)]
pub(super) struct WebDiffHunk {
    kind: DiffKind,
    old_start: u64,
    old_len: u64,
    new_start: u64,
    new_len: u64,
    old_preview: Vec<Line>,
    new_preview: Vec<Line>,
    old_truncated: bool,
    new_truncated: bool,
}

pub(super) async fn api_diff(
    State(state): State<SharedState>,
    Query(q): Query<DiffQuery>,
) -> Result<Json<WebDiffResponse>, ApiError> {
    let path = q.path.trim().to_string();
    if path.is_empty() {
        return Err(bad_request("path is empty"));
    }
    let (old, dirty) = state.doc_and_dirty_edits()?;
    let current_dirty = dirty.is_some();
    // Both response paths are UI-facing: verbatim prefixes are stripped.
    let old_path = workspace::display_path(old.path());
    let new_path = workspace::strip_verbatim(&path);
    let open_options = state.open_options();
    let max_hunks = q.max_hunks.min(100_000);
    let max_lines = q.max_lines.clamp(1, 500);
    let window = q.window.max(1);
    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<WebDiffResponse> {
        let new = Document::open(&path, &open_options)
            .with_context(|| format!("opening '{}'", workspace::strip_verbatim(&path)))?;
        let input = materialize_worker_input(old.as_ref(), dirty.as_ref(), "diff-current")?;
        let response = match &input {
            WorkerInput::Materialized { path: current, .. } => {
                // The scratch copy is read once and discarded: skip the index
                // cache so it doesn't accumulate dead entries. (`open_options`
                // has no later use, so it can be moved rather than cloned.)
                let scratch_opts = ayame_core::OpenOptions {
                    cache_dir: None,
                    // Same as dirty_view: the materialized current buffer is in
                    // the live doc's encoding, so force it rather than letting
                    // auto-detection re-guess (and possibly re-misdetect) it
                    // (issue #75). The comparison file `new` keeps auto-detect.
                    encoding: Some(old.encoding()),
                    ..open_options
                };
                let current = Document::open(current, &scratch_opts)
                    .with_context(|| format!("opening {}", current.display()))?;
                let diff = diff_documents(&current, &new, max_hunks, window);
                web_diff_response(
                    &current,
                    &new,
                    diff,
                    old_path,
                    new_path,
                    current_dirty,
                    max_lines,
                )
            }
            WorkerInput::OnDisk(_) => {
                let diff = diff_documents(old.as_ref(), &new, max_hunks, window);
                web_diff_response(
                    old.as_ref(),
                    &new,
                    diff,
                    old_path,
                    new_path,
                    current_dirty,
                    max_lines,
                )
            }
        };
        drop(input);
        Ok(response)
    })
    .await
    .map_err(internal)?
    .map_err(bad_request)?;
    Ok(Json(result))
}

fn web_diff_response(
    old: &Document,
    new: &Document,
    result: DiffResult,
    old_path: String,
    new_path: String,
    current_dirty: bool,
    max_lines: u64,
) -> WebDiffResponse {
    let hunks = result
        .hunks
        .iter()
        .map(|h| {
            let old_count = h.old_len.min(max_lines);
            let new_count = h.new_len.min(max_lines);
            WebDiffHunk {
                kind: h.kind,
                old_start: h.old_start,
                old_len: h.old_len,
                new_start: h.new_start,
                new_len: h.new_len,
                old_preview: old.lines(h.old_start, old_count),
                new_preview: new.lines(h.new_start, new_count),
                old_truncated: h.old_len > old_count,
                new_truncated: h.new_len > new_count,
            }
        })
        .collect();
    WebDiffResponse {
        old_path,
        new_path,
        current_dirty,
        old_lines: result.old_lines,
        new_lines: result.new_lines,
        hunks,
        hunk_count: result.hunk_count,
        omitted_hunks: result.omitted_hunks,
        added: result.added,
        deleted: result.deleted,
        modified: result.modified,
        max_lines_per_hunk: max_lines,
    }
}

// ---- misc ----------------------------------------------------------------------

#[derive(Deserialize)]
pub(super) struct LineByteQuery {
    line: u64,
    #[serde(default)]
    col: Option<u64>,
}

#[derive(Serialize)]
pub(super) struct LineByteResponse {
    byte: Option<u64>,
}

pub(super) async fn api_linebyte(
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

fn requested_or_default(path: &Path, requested: Option<&str>, suffix: &str) -> PathBuf {
    requested
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| default_suffix_path(path, suffix))
}

fn spawn_dir(kind: &str) -> std::io::Result<PathBuf> {
    let n = REQ_SEQ.fetch_add(1, AtomicOrdering::Relaxed);
    temp_paths::create_private_temp_dir(&format!("srv-{kind}-{n}"))
}

/// A [`Command`] re-invoking this same binary — every op worker is an isolated
/// `ayame <subcommand>` child process.
fn worker_command() -> Result<Command, ApiError> {
    let exe = std::env::current_exe().map_err(internal)?;
    Ok(Command::new(exe))
}

fn append_worker_encoding(cmd: &mut Command, doc: &Document) {
    cmd.arg(crate::cli::wire::ENCODING)
        .arg(doc.stat().encoding.label());
}

/// The 502 every endpoint returns when its worker child exits unsuccessfully.
fn worker_failed(kind: &str, status: std::process::ExitStatus) -> ApiError {
    ApiError::from((
        StatusCode::BAD_GATEWAY,
        format!(
            "{kind} worker {} - the engine is unaffected",
            describe_status(status)
        ),
    ))
}

/// Check a JSON-printing worker's exit status and parse its stdout.
fn parse_worker_json<T: serde::de::DeserializeOwned>(
    kind: &str,
    out: &std::process::Output,
) -> Result<T, ApiError> {
    if !out.status.success() {
        return Err(worker_failed(kind, out.status));
    }
    serde_json::from_slice(&out.stdout).map_err(internal)
}

// ---- long-op progress + cancel registry (#78) --------------------------------

/// Live state for one in-flight worker op, shared between the request future
/// running the worker and the `/api/op/*` polling/cancel endpoints.
struct OpEntry {
    /// Lines processed so far, fed from the worker's `--progress` stderr.
    done: AtomicU64,
    /// Total lines to process (the overlay-aware line count at launch).
    total: AtomicU64,
    /// Set once the worker exits (or is cancelled) so the client stops polling.
    finished: AtomicBool,
    /// Fired by `/api/op/cancel` to kill the worker child.
    cancel: Notify,
}

/// Process-wide table of tracked ops, keyed by the client-generated `op_id`.
static OP_REGISTRY: LazyLock<StdMutex<HashMap<String, Arc<OpEntry>>>> =
    LazyLock::new(|| StdMutex::new(HashMap::new()));

/// RAII registration: inserts the op on construction and removes it on drop, so
/// the table never leaks an entry even if the worker future panics.
struct OpTracker {
    id: String,
    entry: Arc<OpEntry>,
}

impl OpTracker {
    fn register(id: &str, total: u64) -> OpTracker {
        let entry = Arc::new(OpEntry {
            done: AtomicU64::new(0),
            total: AtomicU64::new(total),
            finished: AtomicBool::new(false),
            cancel: Notify::new(),
        });
        OP_REGISTRY
            .lock()
            .unwrap()
            .insert(id.to_string(), entry.clone());
        OpTracker {
            id: id.to_string(),
            entry,
        }
    }
}

impl Drop for OpTracker {
    fn drop(&mut self) {
        OP_REGISTRY.lock().unwrap().remove(&self.id);
    }
}

fn lookup_op(id: &str) -> Option<Arc<OpEntry>> {
    OP_REGISTRY.lock().unwrap().get(id).cloned()
}

/// Parse one machine-readable progress line (`ayame-progress\t{done}\t{total}`)
/// emitted by [`crate::cli::progress`] under `--progress`.
fn parse_progress_line(line: &str) -> Option<(u64, u64)> {
    let rest = line.strip_prefix("ayame-progress\t")?;
    let mut parts = rest.split('\t');
    let done = parts.next()?.trim().parse().ok()?;
    let total = parts.next()?.trim().parse().ok()?;
    Some((done, total))
}

#[derive(Deserialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct OpQuery {
    id: String,
}

#[derive(Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct OpProgress {
    done: u64,
    total: u64,
    /// False once the worker has finished (or was never tracked): the client
    /// stops polling and relies on the main request resolving.
    active: bool,
}

/// `GET /api/op/progress?id=…` — current line count for a tracked worker op.
pub(super) async fn api_op_progress(Query(q): Query<OpQuery>) -> Json<OpProgress> {
    match lookup_op(&q.id) {
        Some(e) => Json(OpProgress {
            done: e.done.load(AtomicOrdering::Relaxed),
            total: e.total.load(AtomicOrdering::Relaxed),
            active: !e.finished.load(AtomicOrdering::Relaxed),
        }),
        None => Json(OpProgress {
            done: 0,
            total: 0,
            active: false,
        }),
    }
}

/// `POST /api/op/cancel?id=…` — request that a tracked worker child be killed.
/// Idempotent: unknown ids are a no-op (the op already finished).
pub(super) async fn api_op_cancel(Query(q): Query<OpQuery>) -> StatusCode {
    if let Some(e) = lookup_op(&q.id) {
        // `notify_one` stores a permit even if the worker isn't awaiting yet,
        // so a cancel that races ahead of the `.notified()` still lands.
        e.cancel.notify_one();
    }
    StatusCode::NO_CONTENT
}

/// Run a worker child while streaming its `--progress` output into the op
/// registry and honoring a cancel request, killing the child on cancel or
/// timeout. Used for the [`run_artifact_worker`] ops when the client supplied
/// an `op_id`.
async fn wait_worker_tracked(
    kind: &str,
    cmd: &mut Command,
    total: u64,
    op_id: &str,
) -> Result<std::process::ExitStatus, ApiError> {
    cmd.arg("--progress")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let tracker = OpTracker::register(op_id, total);
    let mut child = cmd.spawn().map_err(internal)?;
    if let Some(stderr) = child.stderr.take() {
        let entry = tracker.entry.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if let Some((done, total)) = parse_progress_line(&line) {
                    entry.done.store(done, AtomicOrdering::Relaxed);
                    entry.total.store(total, AtomicOrdering::Relaxed);
                }
            }
        });
    }
    let result = tokio::select! {
        waited = child.wait() => waited.map_err(internal),
        _ = tracker.entry.cancel.notified() => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            Err(ApiError::from((
                StatusCode::from_u16(499).unwrap_or(StatusCode::CONFLICT),
                format!("{kind} worker cancelled"),
            )))
        }
        _ = tokio::time::sleep(ARTIFACT_TIMEOUT) => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            Err(ApiError::from((
                StatusCode::GATEWAY_TIMEOUT,
                format!("{kind} worker timed out after {}s - the engine is unaffected", ARTIFACT_TIMEOUT.as_secs()),
            )))
        }
    };
    tracker.entry.finished.store(true, AtomicOrdering::Relaxed);
    result
}

async fn run_artifact_worker(
    kind: &str,
    cmd: &mut Command,
    target: &Path,
    lines: u64,
    op_id: Option<&str>,
) -> Result<ArtifactResponse, ApiError> {
    let status = match op_id {
        Some(id) => wait_worker_tracked(kind, cmd, lines, id).await?,
        None => {
            cmd.stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true);
            wait_worker_for(kind, cmd, ARTIFACT_TIMEOUT).await?
        }
    };
    if !status.success() {
        return Err(worker_failed(kind, status));
    }
    let bytes = tokio::fs::metadata(target).await.map_err(internal)?.len();
    Ok(ArtifactResponse {
        path: workspace::display_path(target),
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

async fn wait_worker_for(
    kind: &str,
    cmd: &mut Command,
    timeout_after: Duration,
) -> Result<std::process::ExitStatus, ApiError> {
    let mut child = cmd.spawn().map_err(internal)?;
    match timeout(timeout_after, child.wait()).await {
        Ok(waited) => waited.map_err(internal),
        Err(_) => {
            let _ = child.kill().await;
            Err(ApiError::from((
                StatusCode::GATEWAY_TIMEOUT,
                format!(
                    "{kind} worker timed out after {}s - the engine is unaffected",
                    timeout_after.as_secs()
                ),
            )))
        }
    }
}

/// Spawn a JSON-printing worker (stdout piped, stderr discarded) and wait for
/// its output, killing it on timeout.
async fn wait_worker_output(
    kind: &str,
    cmd: &mut Command,
    timeout_after: Duration,
) -> Result<std::process::Output, ApiError> {
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let child = cmd.spawn().map_err(internal)?;
    match timeout(timeout_after, child.wait_with_output()).await {
        Ok(waited) => waited.map_err(internal),
        Err(_) => Err(ApiError::from((
            StatusCode::GATEWAY_TIMEOUT,
            format!(
                "{kind} worker timed out after {}s - the engine is unaffected",
                timeout_after.as_secs()
            ),
        ))),
    }
}

/// Like [`wait_worker_output`] but streams the worker's `--progress` into the op
/// registry and honors cancel (used for `split` when an `op_id` was supplied).
async fn wait_worker_output_tracked(
    kind: &str,
    cmd: &mut Command,
    total: u64,
    op_id: &str,
) -> Result<std::process::Output, ApiError> {
    cmd.arg("--progress")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let tracker = OpTracker::register(op_id, total);
    let mut child = cmd.spawn().map_err(internal)?;
    // Drain stdout (the JSON result) concurrently so a large payload can't
    // deadlock against a full pipe while we wait on the child.
    let stdout = child.stdout.take();
    let stdout_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        if let Some(mut out) = stdout {
            let _ = out.read_to_end(&mut buf).await;
        }
        buf
    });
    if let Some(stderr) = child.stderr.take() {
        let entry = tracker.entry.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if let Some((done, total)) = parse_progress_line(&line) {
                    entry.done.store(done, AtomicOrdering::Relaxed);
                    entry.total.store(total, AtomicOrdering::Relaxed);
                }
            }
        });
    }
    let status = tokio::select! {
        waited = child.wait() => waited.map_err(internal),
        _ = tracker.entry.cancel.notified() => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            Err(ApiError::from((
                StatusCode::from_u16(499).unwrap_or(StatusCode::CONFLICT),
                format!("{kind} worker cancelled"),
            )))
        }
        _ = tokio::time::sleep(ARTIFACT_TIMEOUT) => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            Err(ApiError::from((
                StatusCode::GATEWAY_TIMEOUT,
                format!("{kind} worker timed out after {}s - the engine is unaffected", ARTIFACT_TIMEOUT.as_secs()),
            )))
        }
    };
    tracker.entry.finished.store(true, AtomicOrdering::Relaxed);
    let status = status?;
    let stdout = stdout_task.await.map_err(internal)?;
    Ok(std::process::Output {
        status,
        stdout,
        stderr: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use ayame_core::{Encoding, OpenOptions};

    use super::*;

    fn scratch_file(name: &str, contents: &[u8]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ayame-ops-test-{}", std::process::id()));
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

    fn command_args(cmd: &Command) -> Vec<String> {
        cmd.as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    fn assert_has_arg_pair(cmd: &Command, key: &str, value: &str) {
        let args = command_args(cmd);
        assert!(
            args.windows(2)
                .any(|pair| pair[0] == key && pair[1] == value),
            "missing {key} {value:?} in args: {args:?}"
        );
    }

    #[test]
    fn sort_worker_command_inherits_the_open_document_encoding() {
        let path = scratch_file("forced-sjis.txt", b"a\nb\n");
        let doc = Document::open(
            &path,
            &OpenOptions {
                encoding: Some(Encoding::ShiftJis),
                ..OpenOptions::default()
            },
        )
        .unwrap();
        let req = SortSaveRequest {
            path: None,
            in_place: false,
            key: None,
            numeric: false,
            reverse: false,
            delim: None,
            op_id: None,
        };
        let out = path.with_extension("sorted");
        let spill = path.with_extension("spill");

        let cmd = sort_command(&doc, &path, &out, &req, &spill).unwrap();

        assert_has_arg_pair(&cmd, "--encoding", "Shift_JIS");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn clean_session_uses_the_on_disk_path() {
        let path = scratch_file("clean.txt", b"a\nb\n");
        let doc = Document::open(&path, &OpenOptions::default()).unwrap();
        let input = materialize_worker_input(&doc, None, "test").unwrap();
        assert_eq!(input.path(), doc.path());
        assert!(matches!(input, WorkerInput::OnDisk(_)));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn dirty_session_materializes_and_cleans_up() {
        let path = scratch_file("dirty.txt", b"alpha\nbeta\n");
        let doc = Document::open(&path, &OpenOptions::default()).unwrap();
        let mut edits = EditSession::default();
        edits.replace_line(&doc, 1, "BETA".into()).unwrap();
        assert!(edits.is_dirty());

        let input = materialize_worker_input(&doc, Some(&edits), "test").unwrap();
        let materialized = input.path().to_path_buf();
        assert_ne!(materialized, path);
        assert_eq!(
            std::fs::read(&materialized).unwrap(),
            b"alpha\nBETA\n",
            "the worker must see the overlay, not the stale file"
        );

        drop(input);
        assert!(
            !materialized.exists(),
            "materialized scratch file must be removed on drop"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn grep_save_command_carries_search_flags_and_parallel_defaults() {
        let path = scratch_file("grep-src.log", b"a\nb\n");
        let doc = Document::open(&path, &OpenOptions::default()).unwrap();
        let req = GrepSaveRequest {
            path: None,
            query: "ERROR".into(),
            regex: true,
            ci: true,
            word: true,
            overwrite: true,
            jobs: None,
            chunk_lines: None,
            op_id: None,
        };
        let out = path.with_extension("grep");

        let cmd = grep_save_command(&doc, &path, &out, &req).unwrap();

        let args = command_args(&cmd);
        assert_eq!(args[0], "grep-lines");
        for flag in ["--regex", "--ignore-case", "--whole-word", "--overwrite"] {
            assert!(args.iter().any(|a| a == flag), "missing {flag} in {args:?}");
        }
        assert_has_arg_pair(&cmd, "--jobs", "0");
        assert_has_arg_pair(
            &cmd,
            "--chunk-lines",
            &DEFAULT_PARALLEL_REPLACE_CHUNK_LINES.to_string(),
        );
        assert_has_arg_pair(&cmd, "--out", &out.to_string_lossy());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn sorted_temp_path_is_named_after_the_source() {
        let p = default_sorted_temp_path(Path::new("/data/app.log")).unwrap();
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        assert!(
            name.starts_with("app.sorted") && name.ends_with("log"),
            "unexpected name {name}"
        );
        assert!(p.starts_with(std::env::temp_dir()));
    }
}
