//! The supervisor side of an op worker (`crate::worker` holds the spawn
//! contract shared with the child).
//!
//! How an `ayame <subcommand>` child is invoked, supervised (timeout, cancel,
//! progress on stderr) and its result decoded, followed by what it reads: the
//! document as the user sees it, unsaved edits materialized into a scratch
//! snapshot.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::Arc;

use anyhow::Context;
use axum::http::StatusCode;
use ayame_core::{Document, EditSession};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio::time::Duration;

use crate::temp_paths;

use super::super::state::DirtySnapshotCache;
use super::super::{internal, ApiError, SharedState};
use super::registry::{tracked_operation, wait_for_cancel, ArtifactOperation};

pub(super) const WORKER_TIMEOUT: Duration = Duration::from_secs(300);
pub(super) const ARTIFACT_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);
static REQ_SEQ: AtomicU64 = AtomicU64::new(0);

pub(super) fn spawn_dir(kind: &str) -> std::io::Result<PathBuf> {
    let n = REQ_SEQ.fetch_add(1, AtomicOrdering::Relaxed);
    temp_paths::create_private_temp_dir(&format!("srv-{kind}-{n}"))
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
pub(super) fn worker_command() -> Result<Command, ApiError> {
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

pub(super) fn append_worker_encoding(cmd: &mut Command, doc: &Document) {
    cmd.arg("--encoding").arg(doc.stat().encoding.label());
}

/// The 502 every endpoint returns when its worker child exits unsuccessfully.
pub(super) fn worker_failed(kind: &str, status: std::process::ExitStatus) -> ApiError {
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
pub(super) fn parse_worker_json<T: serde::de::DeserializeOwned>(
    kind: &str,
    out: &std::process::Output,
) -> Result<T, ApiError> {
    if !out.status.success() {
        return Err(worker_failed(kind, out.status));
    }
    serde_json::from_slice(&out.stdout).map_err(internal)
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
pub(super) enum Capture {
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
pub(super) async fn supervise_worker(
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
pub(super) async fn wait_worker_output(
    state: &SharedState,
    kind: &str,
    cmd: &mut Command,
    timeout_after: Duration,
) -> Result<std::process::Output, ApiError> {
    wait_worker_output_tracked(state, kind, cmd, timeout_after, None, 0).await
}

pub(super) async fn wait_worker_output_tracked(
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

pub(super) fn parse_progress_line(line: &str) -> Option<(u64, u64)> {
    let mut parts = line.split('\t');
    if parts.next()? != "ayame-progress" {
        return None;
    }
    let done = parts.next()?.parse().ok()?;
    let total = parts.next()?.parse().ok()?;
    Some((done, total))
}

// ---- dirty-buffer materialization --------------------------------------------

/// The file an op worker subprocess should read: either the document's own
/// on-disk path (clean session) or a scratch copy with the unsaved edit
/// overlay applied, so workers see what the user sees. The scratch directory
/// is removed when this guard drops.
pub(in crate::serve) enum WorkerInput {
    /// The session was clean — workers can read the original file directly.
    OnDisk(PathBuf),
    /// Dirty buffer materialized into a scratch file we own.
    Materialized { path: PathBuf, dir: PathBuf },
}

impl WorkerInput {
    pub(in crate::serve) fn path(&self) -> &Path {
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
pub(super) struct WorkerDoc {
    pub(super) doc: Arc<Document>,
    pub(super) input: WorkerInput,
    pub(super) total_lines: u64,
}

pub(super) async fn dirty_aware_input(
    state: &SharedState,
    kind: &'static str,
) -> Result<WorkerDoc, ApiError> {
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
pub(in crate::serve) struct DirtyView {
    live_doc: Arc<Document>,
    edit_revision: u64,
    doc: Arc<Document>,
    _input: Option<Arc<WorkerInput>>,
}

impl DirtyView {
    pub(in crate::serve) fn doc(&self) -> &Arc<Document> {
        &self.doc
    }

    /// The on-disk file a worker child should read for this view.
    pub(in crate::serve) fn path(&self) -> &Path {
        self.doc.path()
    }

    pub(in crate::serve) fn live_doc(&self) -> &Arc<Document> {
        &self.live_doc
    }

    pub(in crate::serve) const fn edit_revision(&self) -> u64 {
        self.edit_revision
    }

    pub(in crate::serve) fn is_clean(&self) -> bool {
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
pub(in crate::serve) async fn dirty_view(state: &SharedState) -> Result<DirtyView, ApiError> {
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
