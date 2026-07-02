use std::path::{Path, PathBuf};

use anyhow::Result;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use ayame_core::{EditLine, EditStats, SaveResult};
use serde::{Deserialize, Serialize};

use super::{bad_request, default_save_copy_path, internal, SharedState, MAX_VIEW};

#[derive(Deserialize)]
pub(super) struct LinesQuery {
    start: u64,
    count: u64,
}

#[derive(Serialize)]
pub(super) struct LinesResponse {
    start: u64,
    total: u64,
    lines: Vec<EditLine>,
}

pub(super) async fn api_lines(
    State(state): State<SharedState>,
    Query(q): Query<LinesQuery>,
) -> Json<LinesResponse> {
    let count = q.count.min(MAX_VIEW);
    // One read acquisition: the doc, its edit overlay and the returned lines
    // are guaranteed mutually consistent, and nothing is cloned but the page.
    state.read(|ws| {
        // An empty workspace has no lines; answer with an empty page rather
        // than an error so the viewport can render nothing gracefully.
        let Some(doc) = ws.doc() else {
            return Json(LinesResponse {
                start: q.start,
                total: 0,
                lines: Vec::new(),
            });
        };
        Json(LinesResponse {
            start: q.start,
            total: ws.edits.total_lines(doc),
            lines: ws.edits.lines(doc, q.start, count),
        })
    })
}

/// Replace the span (l0,c0)..(l1,c1) with `text` (possibly multi-line) as one
/// undo step — the primitive the Notepad-style editor commits against. Column
/// offsets are Unicode scalar (char) counts into the decoded line text.
#[derive(Deserialize)]
pub(super) struct ReplaceRangeRequest {
    l0: u64,
    c0: usize,
    l1: u64,
    c1: usize,
    text: String,
}

#[derive(Serialize)]
pub(super) struct ReplaceRangeResponse {
    stats: EditStats,
    caret_line: u64,
    caret_col: usize,
}

pub(super) async fn api_edit_replace_range(
    State(state): State<SharedState>,
    Json(req): Json<ReplaceRangeRequest>,
) -> Result<Json<ReplaceRangeResponse>, (StatusCode, String)> {
    state.write(|ws| {
        let (doc, edits) = ws.doc_and_edits_mut()?;
        let (caret_line, caret_col) = edits
            .replace_range(doc, req.l0, req.c0, req.l1, req.c1, &req.text)
            .map_err(bad_request)?;
        Ok(Json(ReplaceRangeResponse {
            stats: edits.stats(doc),
            caret_line,
            caret_col,
        }))
    })
}

#[derive(Deserialize)]
pub(super) struct ReplaceRectRequest {
    l0: u64,
    l1: u64,
    c0: usize,
    c1: usize,
    text: String,
}

pub(super) async fn api_edit_replace_rect(
    State(state): State<SharedState>,
    Json(req): Json<ReplaceRectRequest>,
) -> Result<Json<ReplaceRangeResponse>, (StatusCode, String)> {
    state.write(|ws| {
        let (doc, edits) = ws.doc_and_edits_mut()?;
        let (caret_line, caret_col) = edits
            .replace_rect(doc, req.l0, req.l1, req.c0, req.c1, &req.text)
            .map_err(bad_request)?;
        Ok(Json(ReplaceRangeResponse {
            stats: edits.stats(doc),
            caret_line,
            caret_col,
        }))
    })
}

#[derive(Deserialize)]
pub(super) struct EditSaveRequest {
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    overwrite: bool,
}

pub(super) async fn api_edit_save(
    State(state): State<SharedState>,
    Json(req): Json<EditSaveRequest>,
) -> Result<Json<SaveResult>, (StatusCode, String)> {
    // Consistent snapshot (doc + edits + revision) under one lock acquisition.
    let mut snap = state.edit_snapshot()?;
    let active_path = snap.doc.path().to_path_buf();
    let target = req
        .path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            if req.overwrite {
                active_path.clone()
            } else {
                default_save_copy_path(&active_path)
            }
        });
    let reload_active = req.overwrite && same_path(&target, &active_path);
    if !reload_active {
        // Saving a copy elsewhere never mutates workspace state, so the
        // snapshot alone is enough — edits made during the save simply stay
        // pending against the still-open document.
        let target_for_save = target.clone();
        let res = tokio::task::spawn_blocking(move || {
            if req.overwrite {
                snap.edits
                    .save_to_path_overwrite(&snap.doc, target_for_save)
            } else {
                snap.edits.save_to_path(&snap.doc, target_for_save)
            }
        })
        .await
        .map_err(internal)?
        .map_err(bad_request)?;
        return Ok(Json(res));
    }

    // In-place overwrite. Phase 1 — stream the snapshot to a stage file
    // without holding any lock, so typing stays live during a long save.
    let stage = overwrite_stage_path(&target);
    let stage_for_save = stage.clone();
    let doc_for_save = snap.doc.clone();
    let edits_for_save = snap.take_edits(); // move, not a second deep clone
    let saved = tokio::task::spawn_blocking(move || {
        edits_for_save.save_to_path(&doc_for_save, stage_for_save)
    })
    .await
    .map_err(internal)?
    .map_err(bad_request)?;

    // Phase 2 — commit. Serialized against every other doc-slot transition;
    // rejected with 409 (stage discarded) if edits or the tab changed while
    // phase 1 was streaming, so nothing is ever silently thrown away.
    let _transitions = state.lock_transitions().await;
    if let Err(e) = state.detach_for_overwrite(&snap) {
        let _ = tokio::fs::remove_file(&stage).await;
        return Err(e);
    }
    drop(snap); // release our mmap handle before replacing the file (Windows)

    let stage_for_replace = stage.clone();
    let target_for_replace = target.clone();
    let replaced = tokio::task::spawn_blocking(move || {
        replace_existing_file(&stage_for_replace, &target_for_replace)
    })
    .await
    .map_err(internal)?;
    match replaced {
        Ok(bytes) => {
            // The edited bytes are on disk: the overlay is no longer pending.
            state.mark_edits_saved();
            state.install_reloaded(target.clone()).await?;
            Ok(Json(SaveResult {
                path: target,
                bytes,
                lines: saved.lines,
            }))
        }
        Err(e) => {
            // Best effort: reopen the original document. The overlay was left
            // untouched by the detach, so the user's edits are still pending
            // (and the staged bytes are preserved on disk for recovery).
            let _ = state.install_reloaded(active_path).await;
            Err(internal(e))
        }
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn overwrite_stage_path(target: &Path) -> PathBuf {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let name = target
        .file_name()
        .map(|s| s.to_string_lossy())
        .unwrap_or_else(|| "ayame-save".into());
    parent.join(format!(
        ".{name}.ayame-overwrite-{}-{}.tmp",
        std::process::id(),
        unique_suffix()
    ))
}

fn unique_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// Move the fully-written stage file onto `target`. On failure the stage file
/// is deliberately KEPT (its path is in the error) — it holds the only copy of
/// the user's saved data at that point.
fn replace_existing_file(stage: &Path, target: &Path) -> std::io::Result<u64> {
    match std::fs::rename(stage, target) {
        Ok(()) => {}
        Err(first) if target.exists() => {
            std::fs::remove_file(target).map_err(|e| keep_stage_error(e, stage))?;
            std::fs::rename(stage, target).map_err(|second| {
                keep_stage_error(
                    std::io::Error::new(
                        second.kind(),
                        format!("replace failed after initial rename error: {first}; {second}"),
                    ),
                    stage,
                )
            })?;
        }
        Err(e) => return Err(keep_stage_error(e, stage)),
    }
    Ok(std::fs::metadata(target)?.len())
}

fn keep_stage_error(e: std::io::Error, stage: &Path) -> std::io::Error {
    std::io::Error::new(
        e.kind(),
        format!("{e}; the saved data is preserved at '{}'", stage.display()),
    )
}

pub(super) async fn api_edit_revert(
    State(state): State<SharedState>,
) -> Result<Json<EditStats>, (StatusCode, String)> {
    state.write(|ws| {
        let (doc, edits) = ws.doc_and_edits_mut()?;
        edits.clear();
        Ok(Json(edits.stats(doc)))
    })
}

pub(super) async fn api_edit_undo(
    State(state): State<SharedState>,
) -> Result<Json<EditStats>, (StatusCode, String)> {
    state.write(|ws| {
        let (doc, edits) = ws.doc_and_edits_mut()?;
        edits.undo();
        Ok(Json(edits.stats(doc)))
    })
}

pub(super) async fn api_edit_redo(
    State(state): State<SharedState>,
) -> Result<Json<EditStats>, (StatusCode, String)> {
    state.write(|ws| {
        let (doc, edits) = ws.doc_and_edits_mut()?;
        edits.redo();
        Ok(Json(edits.stats(doc)))
    })
}
