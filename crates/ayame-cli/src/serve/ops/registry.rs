//! The session's registry of long-running operations.
//!
//! An endpoint that was handed an `op_id` registers an [`ArtifactOperation`]
//! here for as long as its work runs; `/api/ops/status` and `/api/ops/cancel`
//! look it up by that id to report progress and to stop it.

use std::future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

use super::super::state::lock_recover;
use super::super::{bad_request, ApiError, SharedState};

pub(in crate::serve) struct ArtifactOperation {
    id: String,
    kind: String,
    pub(super) processed_lines: AtomicU64,
    pub(super) total_lines: AtomicU64,
    pub(super) done: AtomicBool,
    pub(super) canceled: AtomicBool,
    pub(super) message: Mutex<Option<String>>,
    notify_cancel: Notify,
}

impl ArtifactOperation {
    pub(super) fn new(id: String, kind: &str, total_lines: u64) -> ArtifactOperation {
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

    pub(super) fn status(&self) -> ArtifactOpStatus {
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

    pub(super) fn set_message(&self, message: impl Into<String>) {
        *lock_recover(&self.message) = Some(message.into());
    }
}

#[derive(Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(in crate::serve) struct ArtifactOpStatus {
    id: String,
    kind: String,
    processed_lines: u64,
    total_lines: u64,
    percent: f64,
    done: bool,
    canceled: bool,
    pub(super) message: Option<String>,
}

#[derive(Deserialize)]
pub(in crate::serve) struct OperationQuery {
    id: String,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(in crate::serve) struct OperationCancelRequest {
    id: String,
}

pub(in crate::serve) async fn api_operation_status(
    State(state): State<SharedState>,
    Query(q): Query<OperationQuery>,
) -> Result<Json<ArtifactOpStatus>, ApiError> {
    let op = lookup_operation(&state, &q.id)?;
    Ok(Json(op.status()))
}

pub(in crate::serve) async fn api_operation_cancel(
    State(state): State<SharedState>,
    Json(req): Json<OperationCancelRequest>,
) -> Result<Json<ArtifactOpStatus>, ApiError> {
    let op = lookup_operation(&state, &req.id)?;
    op.canceled.store(true, AtomicOrdering::Relaxed);
    op.set_message("canceled");
    op.notify_cancel.notify_waiters();
    Ok(Json(op.status()))
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
pub(super) struct OperationGuard {
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
pub(super) fn tracked_operation(
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
pub(in crate::serve) struct TrackedExternalOperation {
    op: Option<Arc<ArtifactOperation>>,
    _guard: Option<OperationGuard>,
}

pub(in crate::serve) fn track_external_operation(
    state: &SharedState,
    op_id: Option<&str>,
    kind: &str,
) -> Result<TrackedExternalOperation, ApiError> {
    let (op, guard) = tracked_operation(state, op_id, kind, 1)?;
    Ok(TrackedExternalOperation { op, _guard: guard })
}

impl TrackedExternalOperation {
    pub(in crate::serve) async fn canceled(&self) {
        wait_for_cancel(self.op.clone()).await;
    }

    pub(in crate::serve) fn finish(&self, message: &str, canceled: bool) {
        if let Some(op) = &self.op {
            op.processed_lines.store(1, AtomicOrdering::Relaxed);
            op.canceled.store(canceled, AtomicOrdering::Relaxed);
            op.done.store(true, AtomicOrdering::Relaxed);
            op.set_message(message);
        }
    }
}

pub(super) fn lookup_operation(
    state: &SharedState,
    id: &str,
) -> Result<Arc<ArtifactOperation>, ApiError> {
    state
        .artifact_ops()
        .get(id)
        .cloned()
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "not_found", "operation not found"))
}

pub(super) async fn wait_for_cancel(op: Option<Arc<ArtifactOperation>>) {
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
