//! The save-to-artifact endpoints: sort, replace, case, grep and split.
//!
//! Each runs a worker child over the dirty-aware input and answers with the
//! file (for split, the part files) it wrote.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering as AtomicOrdering;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use ayame_core::{Document, DEFAULT_PARALLEL_REPLACE_CHUNK_LINES};
use serde::{Deserialize, Serialize};
use tokio::process::Command;

use super::super::{
    bad_request, default_suffix_path, edit, internal, workspace, ApiError, SharedState,
};
use super::registry::tracked_operation;
use super::worker::{
    append_worker_encoding, dirty_aware_input, materialize_worker_input, parse_worker_json,
    spawn_dir, supervise_worker, wait_worker_output_tracked, worker_command, worker_failed,
    Capture, WorkerDoc, WorkerInput, ARTIFACT_TIMEOUT,
};

// ---- save-to-artifact endpoints -----------------------------------------------

#[derive(Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(in crate::serve) struct ArtifactResponse {
    /// UI-facing path (verbatim prefix stripped) — see [`workspace::display_path`].
    path: String,
    bytes: u64,
    lines: u64,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(in crate::serve) struct SortSaveRequest {
    #[serde(default)]
    pub(super) op_id: Option<String>,
    #[serde(default)]
    pub(super) path: Option<String>,
    /// Sort the open file onto itself (the `path` field is ignored): the sort
    /// output atomically replaces the file, the document reloads, and the
    /// (already incorporated) edit overlay is cleared.
    #[serde(default)]
    pub(super) in_place: bool,
    #[serde(default)]
    pub(super) key: Option<usize>,
    #[serde(default)]
    pub(super) keys: Option<Vec<usize>>,
    #[serde(default)]
    pub(super) numeric: bool,
    #[serde(default)]
    pub(super) reverse: bool,
    #[serde(default)]
    pub(super) delim: Option<String>,
    #[serde(default)]
    pub(super) csv: bool,
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

fn requested_or_default(path: &Path, requested: Option<&str>, suffix: &str) -> PathBuf {
    requested
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| default_suffix_path(path, suffix))
}

pub(in crate::serve) async fn api_sort_save(
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
pub(super) fn sort_command(
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
pub(super) fn default_sorted_temp_path(source: &Path) -> std::io::Result<PathBuf> {
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
pub(in crate::serve) struct ReplaceSaveRequest {
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

pub(in crate::serve) async fn api_replace_save(
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
pub(in crate::serve) struct CaseSaveRequest {
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

pub(in crate::serve) async fn api_case_save(
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
pub(in crate::serve) struct GrepSaveRequest {
    #[serde(default)]
    pub(super) op_id: Option<String>,
    /// Output file; null/absent = a `.grep` suffixed sibling default.
    #[serde(default)]
    pub(super) path: Option<String>,
    pub(super) query: String,
    #[serde(default)]
    pub(super) regex: bool,
    #[serde(default)]
    pub(super) ci: bool,
    #[serde(default)]
    pub(super) word: bool,
    /// Replace an existing output file (the OS save dialog already asked).
    #[serde(default)]
    pub(super) overwrite: bool,
    #[serde(default)]
    pub(super) jobs: Option<usize>,
    #[serde(default)]
    pub(super) chunk_lines: Option<u64>,
}

/// `POST /api/grep/save` — "grep して保存" (issue #38): extract every line of
/// the active document (unsaved edits included) matching the query into a new
/// file. The flags carry the search bar's exact semantics (`regex`, `ci`,
/// `word`), and the streaming worker keeps memory bounded on multi-GB files.
pub(in crate::serve) async fn api_grep_save(
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
pub(super) fn grep_save_command(
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
pub(in crate::serve) struct SplitSaveRequest {
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
pub(in crate::serve) async fn api_split_save(
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
