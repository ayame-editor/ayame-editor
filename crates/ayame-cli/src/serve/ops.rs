use std::future;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};

use anyhow::Context;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use ayame_core::{Document, EditSession, DEFAULT_PARALLEL_REPLACE_CHUNK_LINES};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::Notify;
use tokio::time::Duration;

use crate::temp_paths;

use super::state::{lock_recover, DirtySnapshotCache};
use super::{bad_request, default_suffix_path, edit, internal, workspace, ApiError, SharedState};

const WORKER_TIMEOUT: Duration = Duration::from_secs(300);
const ARTIFACT_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);
static REQ_SEQ: AtomicU64 = AtomicU64::new(0);

pub(super) struct ArtifactOperation {
    id: String,
    kind: String,
    processed_lines: AtomicU64,
    total_lines: AtomicU64,
    done: AtomicBool,
    canceled: AtomicBool,
    message: Mutex<Option<String>>,
    notify_cancel: Notify,
}

impl ArtifactOperation {
    fn new(id: String, kind: &str, total_lines: u64) -> ArtifactOperation {
        ArtifactOperation {
            id,
            kind: kind.to_string(),
            processed_lines: AtomicU64::new(0),
            total_lines: AtomicU64::new(total_lines),
            done: AtomicBool::new(false),
            canceled: AtomicBool::new(false),
            message: Mutex::new(None),
            notify_cancel: Notify::new(),
        }
    }

    fn status(&self) -> ArtifactOpStatus {
        let processed = self.processed_lines.load(AtomicOrdering::Relaxed);
        let total = self.total_lines.load(AtomicOrdering::Relaxed);
        ArtifactOpStatus {
            id: self.id.clone(),
            kind: self.kind.clone(),
            processed_lines: processed.min(total),
            total_lines: total,
            percent: if total == 0 {
                100.0
            } else {
                (processed.min(total) as f64 / total as f64) * 100.0
            },
            done: self.done.load(AtomicOrdering::Relaxed),
            canceled: self.canceled.load(AtomicOrdering::Relaxed),
            message: lock_recover(&self.message).clone(),
        }
    }

    fn set_message(&self, message: impl Into<String>) {
        *lock_recover(&self.message) = Some(message.into());
    }
}

#[derive(Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct ArtifactOpStatus {
    id: String,
    kind: String,
    processed_lines: u64,
    total_lines: u64,
    percent: f64,
    done: bool,
    canceled: bool,
    message: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct OperationQuery {
    id: String,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct OperationCancelRequest {
    id: String,
}

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

/// What a dirty-aware *read* (find/search/analysis) runs against. The live
/// identity and edit revision pin the source generation; `doc` is either that
/// clean mmap or the revision-keyed materialized snapshot. `_input` keeps a
/// dirty snapshot alive for background analysis and on-demand navigation.
#[derive(Clone)]
pub(super) struct DirtyView {
    live_doc: Arc<Document>,
    edit_revision: u64,
    doc: Arc<Document>,
    _input: Option<Arc<WorkerInput>>,
}

impl DirtyView {
    pub(super) fn doc(&self) -> &Arc<Document> {
        &self.doc
    }

    /// The on-disk file a worker child should read for this view.
    pub(super) fn path(&self) -> &Path {
        self.doc.path()
    }

    pub(super) fn live_doc(&self) -> &Arc<Document> {
        &self.live_doc
    }

    pub(super) const fn edit_revision(&self) -> u64 {
        self.edit_revision
    }

    pub(super) fn is_clean(&self) -> bool {
        self._input.is_none()
    }
}

/// Get (or build) the [`DirtyView`] for the active session. Building = one
/// materialization + one `Document::open` (no index-cache dir, so throwaway
/// snapshots never pollute the on-disk cache), inside `spawn_blocking`.
/// Staleness is checked on every call: a revision bump or doc change drops
/// the old snapshot. Two racing builders at the same revision produce
/// equivalent snapshots; the last store wins and the loser's temp is cleaned
/// up when its guard drops.
pub(super) async fn dirty_view(state: &SharedState) -> Result<DirtyView, ApiError> {
    let (doc, dirty, edit_revision) = state.doc_dirty_view_source()?;
    let Some(edits) = dirty else {
        // Clean session: any cached snapshot is stale by definition (its
        // revision can't match a future dirty one) — drop it so its temp goes.
        state.invalidate_dirty_snapshot();
        return Ok(DirtyView {
            live_doc: doc.clone(),
            edit_revision,
            doc,
            _input: None,
        });
    };
    let revision = edits.revision();
    if let Some((snapshot, input)) = state.cached_dirty_snapshot(&doc, revision) {
        return Ok(DirtyView {
            live_doc: doc,
            edit_revision: revision,
            doc: snapshot,
            _input: Some(input),
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
        doc: doc.clone(),
        revision,
        input: input.clone(),
        snapshot: snapshot.clone(),
    });
    Ok(DirtyView {
        live_doc: doc,
        edit_revision: revision,
        doc: snapshot,
        _input: Some(input),
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

pub(super) async fn api_operation_status(
    State(state): State<SharedState>,
    Query(q): Query<OperationQuery>,
) -> Result<Json<ArtifactOpStatus>, ApiError> {
    let op = lookup_operation(&state, &q.id)?;
    Ok(Json(op.status()))
}

pub(super) async fn api_operation_cancel(
    State(state): State<SharedState>,
    Json(req): Json<OperationCancelRequest>,
) -> Result<Json<ArtifactOpStatus>, ApiError> {
    let op = lookup_operation(&state, &req.id)?;
    op.canceled.store(true, AtomicOrdering::Relaxed);
    op.set_message("canceled");
    op.notify_cancel.notify_waiters();
    Ok(Json(op.status()))
}

#[derive(Deserialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct SortSaveRequest {
    #[serde(default)]
    op_id: Option<String>,
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
    keys: Option<Vec<usize>>,
    #[serde(default)]
    numeric: bool,
    #[serde(default)]
    reverse: bool,
    #[serde(default)]
    delim: Option<String>,
    #[serde(default)]
    csv: bool,
}

/// The shape every artifact endpoint has: materialize the dirty-aware input,
/// decide a target path and a worker command from it, run the worker, and
/// answer with the artifact.
///
/// Sort, replace, case and grep each wrote that flow out (#106). What actually
/// differs is `plan` — the target and the command — so that is the parameter
/// and the rest is here once, including the `drop(wd)` that must outlive the
/// worker child and is easy to forget.
async fn artifact_endpoint<F>(
    state: &SharedState,
    kind: &'static str,
    label: &'static str,
    op_id: Option<&str>,
    plan: F,
) -> Result<Json<ArtifactResponse>, ApiError>
where
    F: FnOnce(&WorkerDoc) -> Result<WorkerPlan, ApiError>,
{
    let wd = dirty_aware_input(state, label).await?;
    // `wd` is dropped on every path below, failed plan included: it owns the
    // materialized snapshot and must not leak.
    let mut planned = plan(&wd)?;
    let res = run_artifact_worker(
        state,
        kind,
        &mut planned.cmd,
        &planned.target,
        wd.total_lines,
        op_id,
    )
    .await;
    if let Some(scratch) = planned.scratch {
        let _ = tokio::fs::remove_dir_all(&scratch).await;
    }
    drop(wd); // the snapshot guard must outlive the worker child
    res.map(Json)
}

/// What an endpoint decided to run, given its materialized input.
struct WorkerPlan {
    /// Where the artifact is written.
    target: PathBuf,
    /// The worker invocation that writes it.
    cmd: Command,
    /// A scratch directory to remove once the worker has finished — sort's
    /// external-merge spill. `None` for the transforms, which stream.
    scratch: Option<PathBuf>,
}

impl WorkerPlan {
    fn new(target: PathBuf, cmd: Command) -> WorkerPlan {
        WorkerPlan {
            target,
            cmd,
            scratch: None,
        }
    }

    fn spilling(target: PathBuf, cmd: Command, scratch: PathBuf) -> WorkerPlan {
        WorkerPlan {
            target,
            cmd,
            scratch: Some(scratch),
        }
    }
}

/// `--jobs` / `--chunk-lines` for the line-local transforms (replace, case,
/// grep-lines), which the three endpoints spelled out identically. Zero jobs
/// means "the Rayon default" in the worker, so an unset request still says so
/// explicitly rather than depending on the worker's own default.
fn append_parallel_args(cmd: &mut Command, jobs: Option<usize>, chunk_lines: Option<u64>) {
    cmd.arg("--jobs")
        .arg(jobs.unwrap_or(0).to_string())
        .arg("--chunk-lines")
        .arg(
            chunk_lines
                .unwrap_or(DEFAULT_PARALLEL_REPLACE_CHUNK_LINES)
                .to_string(),
        );
}

pub(super) async fn api_sort_save(
    State(state): State<SharedState>,
    Json(req): Json<SortSaveRequest>,
) -> Result<Json<ArtifactResponse>, ApiError> {
    if req.in_place {
        return sort_save_in_place(state, req).await;
    }
    let op_id = req.op_id.clone();
    artifact_endpoint(&state, "sort", "sort-save", op_id.as_deref(), |wd| {
        // No explicit destination: sort into a scratch file (the GUI opens the
        // result as a new tab instead of asking for a path first).
        let target = match req.path.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
            Some(p) => PathBuf::from(p),
            None => default_sorted_temp_path(wd.doc.path()).map_err(internal)?,
        };
        let dir = spawn_dir("sort-save-spill").map_err(internal)?;
        let cmd = sort_command(&wd.doc, wd.input.path(), &target, &req, &dir)?;
        Ok(WorkerPlan::spilling(target, cmd, dir))
    })
    .await
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
    let res = run_artifact_worker(
        &state,
        "sort",
        &mut cmd,
        &stage,
        total_lines,
        req.op_id.as_deref(),
    )
    .await;
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
    let mut cmd = worker_command()?;
    cmd.arg("sort").arg(input);
    append_worker_encoding(&mut cmd, doc);
    cmd.arg("--out").arg(out);
    if let Some(keys) = req.keys.as_ref().filter(|keys| !keys.is_empty()) {
        let value = keys
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(",");
        cmd.arg("--key").arg(value);
    } else if let Some(key) = req.key {
        cmd.arg("--key").arg(key.to_string());
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
    if req.csv {
        cmd.arg("--csv");
    }
    cmd.arg("--spill-dir").arg(spill_dir);
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
    op_id: Option<String>,
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
}

pub(super) async fn api_replace_save(
    State(state): State<SharedState>,
    Json(req): Json<ReplaceSaveRequest>,
) -> Result<Json<ArtifactResponse>, ApiError> {
    let op_id = req.op_id.clone();
    artifact_endpoint(&state, "replace", "replace-save", op_id.as_deref(), |wd| {
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
        append_parallel_args(&mut cmd, req.jobs, req.chunk_lines);
        Ok(WorkerPlan::new(target, cmd))
    })
    .await
}

#[derive(Deserialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct CaseSaveRequest {
    #[serde(default)]
    op_id: Option<String>,
    #[serde(default)]
    path: Option<String>,
    mode: String,
    #[serde(default)]
    jobs: Option<usize>,
    #[serde(default)]
    chunk_lines: Option<u64>,
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
    let op_id = req.op_id.clone();
    artifact_endpoint(&state, "case", "case-save", op_id.as_deref(), |wd| {
        let target = requested_or_default(wd.doc.path(), req.path.as_deref(), &mode);
        let mut cmd = worker_command()?;
        cmd.arg("case").arg(wd.input.path());
        append_worker_encoding(&mut cmd, &wd.doc);
        cmd.arg(mode).arg("--out").arg(&target);
        // Same chunked-parallel plumbing as replace: case conversion is
        // line-local, so huge files scale with cores in the worker.
        append_parallel_args(&mut cmd, req.jobs, req.chunk_lines);
        Ok(WorkerPlan::new(target, cmd))
    })
    .await
}

#[derive(Deserialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct GrepSaveRequest {
    #[serde(default)]
    op_id: Option<String>,
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
    let op_id = req.op_id.clone();
    artifact_endpoint(&state, "grep", "grep-save", op_id.as_deref(), |wd| {
        let target = requested_or_default(wd.doc.path(), req.path.as_deref(), "grep");
        // Checked here (same message as save-as) because the worker's stderr is
        // discarded: its own Conflict would surface as an opaque 502, and the web
        // overwrite-confirm flow keys off this text.
        if !req.overwrite && target.exists() {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "exists",
                format!("{} は既に存在します", workspace::display_path(&target)),
            ));
        }
        let cmd = grep_save_command(&wd.doc, wd.input.path(), &target, &req)?;
        Ok(WorkerPlan::new(target, cmd))
    })
    .await
}

/// Build the `ayame grep-lines` worker invocation for [`api_grep_save`].
fn grep_save_command(
    doc: &Document,
    input: &Path,
    out: &Path,
    req: &GrepSaveRequest,
) -> Result<Command, ApiError> {
    let mut cmd = worker_command()?;
    cmd.arg("grep-lines").arg(input);
    append_worker_encoding(&mut cmd, doc);
    cmd.arg(&req.query).arg("--out").arg(out);
    if req.regex {
        cmd.arg("--regex");
    }
    if req.ci {
        cmd.arg("--ignore-case");
    }
    if req.word {
        cmd.arg("--whole-word");
    }
    if req.overwrite {
        cmd.arg("--overwrite");
    }
    // Same chunked-parallel plumbing as replace: line extraction is
    // line-local, so huge files scale with cores in the worker.
    cmd.arg("--jobs")
        .arg(req.jobs.unwrap_or(0).to_string())
        .arg("--chunk-lines")
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
    #[serde(default)]
    op_id: Option<String>,
    /// Lines per output part (must be >= 1).
    lines: u64,
    /// Output directory; null/absent = the source file's directory.
    #[serde(default)]
    dir: Option<String>,
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

    let out = wait_worker_output_tracked(
        &state,
        "split",
        &mut cmd,
        ARTIFACT_TIMEOUT,
        req.op_id.as_deref(),
        wd.total_lines,
    )
    .await;
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

#[derive(Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct SearchResponse {
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

pub(super) async fn api_search(
    State(state): State<SharedState>,
    Query(q): Query<SearchQuery>,
) -> Result<Json<SearchResponse>, ApiError> {
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

    let out = wait_worker_output(&state, "search", &mut cmd, WORKER_TIMEOUT).await;
    drop(view); // the snapshot guard must outlive the worker child
    let res: ayame_core::SearchResult = parse_worker_json("search", &out?)?;
    Ok(Json(res.into()))
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

#[derive(Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct GrepResponse {
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
pub(super) async fn api_grep(
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
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
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

// ---- misc ----------------------------------------------------------------------

#[derive(Deserialize)]
pub(super) struct LineByteQuery {
    line: u64,
    #[serde(default)]
    col: Option<u64>,
}

#[derive(Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
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

fn valid_operation_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
}

fn register_operation(
    state: &SharedState,
    id: Option<&str>,
    kind: &str,
    total_lines: u64,
) -> Result<Option<Arc<ArtifactOperation>>, ApiError> {
    let Some(id) = id.map(str::trim).filter(|id| !id.is_empty()) else {
        return Ok(None);
    };
    if !valid_operation_id(id) {
        return Err(bad_request("invalid operation id"));
    }
    let op = Arc::new(ArtifactOperation::new(id.to_string(), kind, total_lines));
    state.artifact_ops().insert(id.to_string(), op.clone());
    Ok(Some(op))
}

/// Drops an operation out of the session's map when the worker call returns,
/// so finished ops don't accumulate for the life of the `serve` process —
/// every operation carries a fresh random id, so without this the map would
/// grow unbounded across a long session. A status poll that arrives after
/// removal just 404s, which the client's best-effort poller ignores.
struct OperationGuard {
    state: SharedState,
    id: String,
}

impl Drop for OperationGuard {
    fn drop(&mut self) {
        // `artifact_ops()` recovers from poisoning rather than unwrapping:
        // this Drop can fire while unwinding a panicking request, where a
        // second panic would abort the whole process instead of failing one
        // request (#106).
        self.state.artifact_ops().remove(&self.id);
    }
}

/// Register `op_id` (if any) and return the tracking handle plus a guard that
/// evicts it from the map when dropped. Callers bind the guard for the whole
/// worker call: `let (op, _guard) = tracked_operation(...)?;`.
fn tracked_operation(
    state: &SharedState,
    op_id: Option<&str>,
    kind: &str,
    total_lines: u64,
) -> Result<(Option<Arc<ArtifactOperation>>, Option<OperationGuard>), ApiError> {
    let op = register_operation(state, op_id, kind, total_lines)?;
    let guard = op.as_ref().map(|o| OperationGuard {
        state: state.clone(),
        id: o.id.clone(),
    });
    Ok((op, guard))
}

/// Small public facade for non-Ayame subprocesses. It keeps the operation in
/// the same status/cancel registry as sort/grep without exposing the registry
/// internals to the external-action module.
pub(super) struct TrackedExternalOperation {
    op: Option<Arc<ArtifactOperation>>,
    _guard: Option<OperationGuard>,
}

pub(super) fn track_external_operation(
    state: &SharedState,
    op_id: Option<&str>,
    kind: &str,
) -> Result<TrackedExternalOperation, ApiError> {
    let (op, guard) = tracked_operation(state, op_id, kind, 1)?;
    Ok(TrackedExternalOperation { op, _guard: guard })
}

impl TrackedExternalOperation {
    pub(super) async fn canceled(&self) {
        wait_for_cancel(self.op.clone()).await;
    }

    pub(super) fn finish(&self, message: &str, canceled: bool) {
        if let Some(op) = &self.op {
            op.processed_lines.store(1, AtomicOrdering::Relaxed);
            op.canceled.store(canceled, AtomicOrdering::Relaxed);
            op.done.store(true, AtomicOrdering::Relaxed);
            op.set_message(message);
        }
    }
}

fn lookup_operation(state: &SharedState, id: &str) -> Result<Arc<ArtifactOperation>, ApiError> {
    state
        .artifact_ops()
        .get(id)
        .cloned()
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "not_found", "operation not found"))
}

/// A [`Command`] re-invoking this same binary — every op worker is an isolated
/// `ayame <subcommand>` child process.
///
/// The program comes from the startup fingerprint rather than a fresh
/// `current_exe()`: a self-update that landed mid-session has already replaced
/// that file, and spawning it would either fail outright (Linux, where the
/// path now reads `… (deleted)`) or quietly run a different build against this
/// server's protocol (#137). Both become [`restart_required`] here, and the
/// version stamp lets the worker refuse the same skew if the update lands
/// between this check and the exec.
fn worker_command() -> Result<Command, ApiError> {
    let exe = crate::worker::worker_program().map_err(|blocked| match blocked {
        crate::worker::SpawnBlocked::Replaced => restart_required(),
        crate::worker::SpawnBlocked::Unknown => {
            internal("this process's executable could not be located")
        }
    })?;
    let mut cmd = Command::new(exe);
    cmd.env(crate::worker::VERSION_ENV, crate::worker::VERSION);
    Ok(cmd)
}

/// The error every worker-backed endpoint returns once the editor has been
/// updated underneath itself. Deliberately not a 502 `worker_failed`: nothing
/// went wrong with the operation, the process just needs to be restarted, and
/// the open document and its unsaved edits are untouched either way.
fn restart_required() -> ApiError {
    ApiError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "restart_required",
        "Ayame was updated on disk while this editor was running - restart \
         Ayame to run this operation. The open file and any unsaved edits are \
         unaffected.",
    )
}

fn append_worker_encoding(cmd: &mut Command, doc: &Document) {
    cmd.arg("--encoding").arg(doc.stat().encoding.label());
}

/// The 502 every endpoint returns when its worker child exits unsuccessfully.
fn worker_failed(kind: &str, status: std::process::ExitStatus) -> ApiError {
    // The worker refused the job because it is a different build than this
    // server — an update landed between `worker_command`'s check and the exec
    // (#137). Same story for the user, so the same answer.
    if status.code() == Some(i32::from(crate::worker::VERSION_MISMATCH_EXIT)) {
        return restart_required();
    }
    ApiError::new(
        StatusCode::BAD_GATEWAY,
        "worker_failed",
        format!(
            "{kind} worker {} - the engine is unaffected",
            describe_status(status)
        ),
    )
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

async fn run_artifact_worker(
    state: &SharedState,
    kind: &str,
    cmd: &mut Command,
    target: &Path,
    lines: u64,
    op_id: Option<&str>,
) -> Result<ArtifactResponse, ApiError> {
    let (op, _op_guard) = tracked_operation(state, op_id, kind, lines)?;
    let (status, _) =
        supervise_worker(kind, cmd, Capture::None, ARTIFACT_TIMEOUT, op.clone()).await?;
    if !status.success() {
        if let Some(op) = &op {
            op.done.store(true, AtomicOrdering::Relaxed);
            op.set_message("failed");
        }
        return Err(worker_failed(kind, status));
    }
    let bytes = tokio::fs::metadata(target).await.map_err(internal)?.len();
    if let Some(op) = &op {
        let total = op.total_lines.load(AtomicOrdering::Relaxed);
        op.processed_lines.store(total, AtomicOrdering::Relaxed);
        op.done.store(true, AtomicOrdering::Relaxed);
    }
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

/// What the caller wants back from the worker besides its exit status.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Capture {
    /// stdout is discarded: artifact writers report through the filesystem.
    None,
    /// stdout is read to end: the JSON-printing workers (search, split).
    Stdout,
}

/// Spawn `cmd`, wait for it, and honour the operation's timeout and cancel.
///
/// The supervision — child, progress reader on stderr, and the
/// wait/timeout/cancel `select!` — was written out once per return shape
/// (#106). The shapes differ; the supervision does not. `Capture` names the
/// difference, and the stdout reader is always awaited before an error
/// propagates so a killed child never leaves a task reading a dead pipe.
async fn supervise_worker(
    kind: &str,
    cmd: &mut Command,
    capture: Capture,
    timeout_after: Duration,
    op: Option<Arc<ArtifactOperation>>,
) -> Result<(std::process::ExitStatus, Vec<u8>), ApiError> {
    if op.is_some() {
        cmd.arg("--progress");
    }
    cmd.stdout(match capture {
        Capture::Stdout => Stdio::piped(),
        Capture::None => Stdio::null(),
    })
    .stderr(if op.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    })
    .kill_on_drop(true);
    let mut child = cmd.spawn().map_err(internal)?;
    let stdout_task = match capture {
        Capture::Stdout => {
            let mut stdout = child
                .stdout
                .take()
                .ok_or_else(|| internal("worker stdout was not piped"))?;
            Some(tokio::spawn(async move {
                let mut buf = Vec::new();
                stdout.read_to_end(&mut buf).await.map(|_| buf)
            }))
        }
        Capture::None => None,
    };
    let progress_task = child.stderr.take().map(|stderr| {
        let op = op.clone();
        tokio::spawn(async move { read_progress_lines(stderr, op).await })
    });
    let sleep = tokio::time::sleep(timeout_after);
    tokio::pin!(sleep);
    let cancel = wait_for_cancel(op.clone());
    tokio::pin!(cancel);

    let status_res = tokio::select! {
        waited = child.wait() => waited.map_err(internal),
        _ = &mut sleep => {
            let _ = child.kill().await;
            Err(ApiError::new(
                StatusCode::GATEWAY_TIMEOUT,
                "timeout",
                format!(
                    "{kind} worker timed out after {}s - the engine is unaffected",
                    timeout_after.as_secs()
                ),
            ))
        }
        _ = &mut cancel => {
            if let Some(op) = &op {
                op.canceled.store(true, AtomicOrdering::Relaxed);
                op.done.store(true, AtomicOrdering::Relaxed);
                op.set_message("canceled");
            }
            let _ = child.kill().await;
            Err(ApiError::new(
                StatusCode::CONFLICT,
                "canceled",
                format!("{kind} operation canceled"),
            ))
        }
    };
    let stdout = match stdout_task {
        Some(task) => task.await.map_err(internal)?.map_err(internal)?,
        None => Vec::new(),
    };
    if let Some(task) = progress_task {
        let _ = task.await;
    }
    Ok((status_res?, stdout))
}

/// Spawn a JSON-printing worker (stdout piped, stderr discarded) and wait for
/// its output, killing it on timeout.
async fn wait_worker_output(
    state: &SharedState,
    kind: &str,
    cmd: &mut Command,
    timeout_after: Duration,
) -> Result<std::process::Output, ApiError> {
    wait_worker_output_tracked(state, kind, cmd, timeout_after, None, 0).await
}

async fn wait_worker_output_tracked(
    state: &SharedState,
    kind: &str,
    cmd: &mut Command,
    timeout_after: Duration,
    op_id: Option<&str>,
    total_lines: u64,
) -> Result<std::process::Output, ApiError> {
    let (op, _op_guard) = tracked_operation(state, op_id, kind, total_lines)?;
    let (status, stdout) =
        supervise_worker(kind, cmd, Capture::Stdout, timeout_after, op.clone()).await?;
    if let Some(op) = &op {
        let total = op.total_lines.load(AtomicOrdering::Relaxed);
        op.processed_lines.store(total, AtomicOrdering::Relaxed);
        op.done.store(true, AtomicOrdering::Relaxed);
    }
    Ok(std::process::Output {
        status,
        stdout,
        stderr: Vec::new(),
    })
}

async fn wait_for_cancel(op: Option<Arc<ArtifactOperation>>) {
    match op {
        Some(op) => loop {
            if op.canceled.load(AtomicOrdering::Relaxed) {
                return;
            }
            op.notify_cancel.notified().await;
        },
        None => future::pending::<()>().await,
    }
}

async fn read_progress_lines(
    stderr: tokio::process::ChildStderr,
    op: Option<Arc<ArtifactOperation>>,
) {
    let Some(op) = op else {
        return;
    };
    let mut lines = BufReader::new(stderr).lines();
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                if let Some((done, total)) = parse_progress_line(&line) {
                    op.total_lines.store(total, AtomicOrdering::Relaxed);
                    op.processed_lines
                        .store(done.min(total), AtomicOrdering::Relaxed);
                }
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }
}

fn parse_progress_line(line: &str) -> Option<(u64, u64)> {
    let mut parts = line.split('\t');
    if parts.next()? != "ayame-progress" {
        return None;
    }
    let done = parts.next()?.parse().ok()?;
    let total = parts.next()?.parse().ok()?;
    Some((done, total))
}

#[cfg(test)]
mod tests;
