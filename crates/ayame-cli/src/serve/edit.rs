use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use ayame_core::{BatchEdit, EditLine, EditStats, Encoding, Eol, SaveResult};
use serde::{Deserialize, Serialize};

use super::{bad_request, default_save_copy_path, internal, workspace, SharedState, MAX_VIEW};

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
    // Take a short, consistent snapshot, then do potentially large line
    // decoding off the async runtime worker thread.
    let snapshot = state.read(|ws| {
        // An empty workspace has no lines; answer with an empty page rather
        // than an error so the viewport can render nothing gracefully.
        ws.doc().map(|doc| (doc.clone(), ws.edits.clone()))
    });
    let Some((doc, edits)) = snapshot else {
        return Json(LinesResponse {
            start: q.start,
            total: 0,
            lines: Vec::new(),
        });
    };
    let start = q.start;
    let response = tokio::task::spawn_blocking(move || LinesResponse {
        start,
        total: edits.total_lines(&doc),
        lines: edits.lines(&doc, start, count),
    })
    .await
    .unwrap_or_else(|_| LinesResponse {
        start,
        total: 0,
        lines: Vec::new(),
    });
    Json(response)
}

/// Replace the span (l0,c0)..(l1,c1) with `text` (possibly multi-line) as one
/// undo step — the primitive the Notepad-style editor commits against. Column
/// offsets are Unicode scalar (char) counts into the decoded line text.
#[derive(Deserialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
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

/// Apply one edit per caret — a multi-cursor commit — as a single undo step.
/// Every range refers to the view BEFORE the batch and the ranges must not
/// overlap; the response returns the post-batch caret for each edit in
/// request order.
#[derive(Deserialize)]
pub(super) struct ReplaceBatchRequest {
    edits: Vec<BatchEdit>,
}

#[derive(Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct CaretPosition {
    line: u64,
    col: usize,
}

#[derive(Serialize)]
pub(super) struct ReplaceBatchResponse {
    stats: EditStats,
    carets: Vec<CaretPosition>,
}

pub(super) async fn api_edit_replace_batch(
    State(state): State<SharedState>,
    Json(req): Json<ReplaceBatchRequest>,
) -> Result<Json<ReplaceBatchResponse>, (StatusCode, String)> {
    state.write(|ws| {
        let (doc, edits) = ws.doc_and_edits_mut()?;
        let carets = edits.replace_batch(doc, &req.edits).map_err(bad_request)?;
        Ok(Json(ReplaceBatchResponse {
            stats: edits.stats(doc),
            carets: carets
                .into_iter()
                .map(|(line, col)| CaretPosition { line, col })
                .collect(),
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
    /// 名前を付けて保存 semantics: after a successful save the ACTIVE TAB shows
    /// the saved file (instead of leaving the tab on the old document and
    /// opening the saved copy as a second tab).
    #[serde(default)]
    switch_to_saved: bool,
    /// 変換して保存: target 文字コード (e.g. "shift_jis"). When either this or
    /// `eol` is set the whole file is re-encoded, so the save runs as a
    /// rewrite-then-reload rather than the fast incremental path.
    #[serde(default)]
    encoding: Option<String>,
    /// 変換して保存: target 改行コード ("lf" / "crlf" / "cr").
    #[serde(default)]
    eol: Option<String>,
    /// 変換して保存: whether a UTF-8 target file gets a leading BOM. Only
    /// meaningful when the target 文字コード is UTF-8 (ignored otherwise). When
    /// omitted the file's current BOM state is preserved.
    #[serde(default)]
    bom: Option<bool>,
}

/// [`SaveResult`] plus whether the active tab now shows the saved file.
#[derive(Serialize)]
pub(super) struct EditSaveResponse {
    /// UI-facing path (verbatim prefix stripped) — see [`workspace::display_path`].
    path: String,
    bytes: u64,
    lines: u64,
    switched: bool,
}

impl EditSaveResponse {
    fn from_result(res: SaveResult, switched: bool) -> Self {
        EditSaveResponse {
            path: workspace::display_path(&res.path),
            bytes: res.bytes,
            lines: res.lines,
            switched,
        }
    }
}

pub(super) async fn api_edit_save(
    State(state): State<SharedState>,
    Json(req): Json<EditSaveRequest>,
) -> Result<Json<EditSaveResponse>, (StatusCode, String)> {
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
    // ---- 変換して保存: re-encode every line to a chosen 文字コード / 改行コード.
    // This rewrites the whole file (the fast path copies untouched lines raw),
    // so it runs as save-then-reload: the active tab reopens the converted
    // bytes, refreshing the encoding/eol shown in the status bar.
    if req.encoding.is_some() || req.eol.is_some() {
        let cur = snap.doc.stat();
        let enc = match req.encoding.as_deref() {
            Some(name) => Encoding::parse(name)
                .map(|e| {
                    if e == Encoding::Ascii {
                        Encoding::Utf8
                    } else {
                        e
                    }
                })
                .ok_or_else(|| bad_request(format!("unknown encoding '{name}'")))?,
            None => cur.encoding,
        };
        if enc.is_wide() {
            return Err(bad_request(format!("{} での保存は未対応です", enc.label())));
        }
        let eol = match req.eol.as_deref() {
            Some(name) => Eol::parse(name)
                .ok_or_else(|| bad_request(format!("unknown line ending '{name}'")))?,
            None => match cur.eol {
                Eol::Mixed | Eol::None => Eol::Lf,
                other => other,
            },
        };
        // A UTF-8 BOM is only meaningful for UTF-8 output; default to the
        // file's current BOM state so an unspecified request round-trips it.
        let with_bom = req.bom.unwrap_or(cur.bom_bytes > 0);
        let overwrite = req.overwrite || same_path(&target, &active_path);
        let target_for_save = target.clone();
        let doc_for_save = snap.doc.clone();
        let edits_for_save = snap.take_edits();
        let res = tokio::task::spawn_blocking(move || {
            edits_for_save.save_converted(
                &doc_for_save,
                target_for_save,
                enc,
                eol,
                with_bom,
                overwrite,
            )
        })
        .await
        .map_err(internal)?
        .map_err(bad_request)?;
        // Refresh the active tab from the converted file. Skipped (switched =
        // false) if edits landed while the rewrite was streaming, so no newer
        // edit is dropped — the client then falls back to a normal open.
        let switched = switch_active_to_saved(&state, &snap, &res.path, &active_path).await;
        return Ok(Json(EditSaveResponse::from_result(res, switched)));
    }

    let reload_active = req.overwrite && same_path(&target, &active_path);
    if !reload_active {
        // Saving a copy elsewhere never mutates workspace state, so the
        // snapshot alone is enough — edits made during the save simply stay
        // pending against the still-open document.
        let target_for_save = target.clone();
        let doc_for_save = snap.doc.clone();
        let edits_for_save = snap.take_edits();
        let res = tokio::task::spawn_blocking(move || {
            if req.overwrite {
                edits_for_save.save_to_path_overwrite(&doc_for_save, target_for_save)
            } else {
                edits_for_save.save_to_path(&doc_for_save, target_for_save)
            }
        })
        .await
        .map_err(internal)?
        .map_err(bad_request)?;
        let mut switched = false;
        if req.switch_to_saved {
            switched = switch_active_to_saved(&state, &snap, &res.path, &active_path).await;
        }
        return Ok(Json(EditSaveResponse::from_result(res, switched)));
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

    // Phase 2 — commit WITHOUT reloading. Serialized against every other
    // doc-slot transition; rejected with 409 (stage discarded) if edits or the
    // tab changed while phase 1 was streaming, so nothing is ever silently
    // thrown away. The current file is renamed to a hidden aside sibling —
    // renaming an open/mmap'd file works on Unix and on Windows, and the live
    // mmap keeps reading the old inode through the new name — then the stage
    // takes over the target name. The document handle, the overlay and the
    // undo history all stay untouched: the view is unchanged and undo keeps
    // working ACROSS the save; only the saved-content marker moves.
    let _transitions = state.lock_transitions().await;
    if let Err(e) = state.confirm_overwrite(&snap) {
        let _ = tokio::fs::remove_file(&stage).await;
        return Err(e);
    }

    let aside = workspace::aside_path(&target);
    let stage_for_swap = stage.clone();
    let target_for_swap = target.clone();
    let swapped = tokio::task::spawn_blocking(move || {
        swap_in_staged_file(&stage_for_swap, &target_for_swap, aside)
    })
    .await
    .map_err(internal)?;
    match swapped {
        Ok(aside_used) => {
            state.commit_in_place_save(&snap, aside_used);
            // The active tab already shows the saved path: report it as
            // switched so 名前を付けて保存 onto the same file refreshes in
            // place instead of opening anything.
            Ok(Json(EditSaveResponse {
                path: workspace::display_path(&target),
                bytes: saved.bytes,
                lines: saved.lines,
                switched: true,
            }))
        }
        // The swap either rolled back (target intact) or kept the stage (its
        // path is in the error). The session is untouched either way — the
        // user's edits are still pending, nothing was lost.
        Err(e) => Err(internal(e)),
    }
}

/// Move the fully-written stage file onto `target`, preserving the current
/// target file under `aside` so a live mmap of it stays readable. Returns the
/// aside path when the target existed (`None` when there was nothing to move
/// aside). On failure `target` is restored where possible and the stage file
/// is KEPT (its path is in the error) — it holds the freshly saved bytes.
fn swap_in_staged_file(
    stage: &Path,
    target: &Path,
    aside: PathBuf,
) -> std::io::Result<Option<PathBuf>> {
    let aside_used = match std::fs::rename(target, &aside) {
        Ok(()) => Some(aside),
        // The target vanished externally; nothing to preserve.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(keep_stage_error(e, stage)),
    };
    if let Err(e) = std::fs::rename(stage, target) {
        // Never leave the target name dangling: put the old file back.
        if let Some(aside) = &aside_used {
            let _ = std::fs::rename(aside, target);
        }
        return Err(keep_stage_error(e, stage));
    }
    Ok(aside_used)
}

/// Make `saved` the active tab's document (fresh, clean session — the saved
/// bytes ARE the view). Skipped (returns false) when the workspace moved on
/// while the save was streaming — the revision is re-checked inside the
/// installing write, so an edit racing the reload is never clobbered; the
/// client then falls back to opening the file normally. On a real switch the
/// old path's crash log describes edits that were just saved — drop it.
async fn switch_active_to_saved(
    state: &SharedState,
    snap: &super::state::EditSnapshot,
    saved: &Path,
    active_path: &Path,
) -> bool {
    let _transitions = state.lock_transitions().await;
    if state.confirm_overwrite(snap).is_err() {
        return false; // cheap pre-check: skip the reopen on a sure conflict
    }
    let switched = state
        .reload_reverted_if_unchanged(saved.to_path_buf(), snap)
        .await
        .is_ok();
    if switched && !same_path(saved, active_path) {
        discard_wal_for(state, active_path).await;
    }
    switched
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

/// A unique sibling path of `target` for staging a full rewrite (shared with
/// the in-place sort in `ops`): same directory, hence same volume, so the
/// final rename is atomic.
pub(super) fn overwrite_stage_path(target: &Path) -> PathBuf {
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
pub(super) fn replace_existing_file(stage: &Path, target: &Path) -> std::io::Result<u64> {
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
        format!(
            "{e}; the saved data is preserved at '{}'",
            workspace::display_path(stage)
        ),
    )
}

/// Apply — or discard — the crash log reported by `stat.recoverable` for the
/// active document. `{}` restores (replays the log into the live session and
/// re-arms logging); `{"discard": true}` deletes the log and continues clean.
#[derive(Deserialize, Default)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct RecoverRequest {
    #[serde(default)]
    discard: bool,
}

#[derive(Serialize)]
pub(super) struct RecoverResponse {
    stats: EditStats,
    /// Transactions replayed (0 for a discard, and for a restore whose whole
    /// state came from a compaction snapshot).
    replayed: usize,
}

pub(super) async fn api_edit_recover(
    State(state): State<SharedState>,
    Json(req): Json<RecoverRequest>,
) -> Result<Json<RecoverResponse>, (StatusCode, String)> {
    let (stats, replayed) = state.recover_wal(req.discard).await?;
    Ok(Json(RecoverResponse { stats, replayed }))
}

/// After 名前を付けて保存 switched the active tab to the saved file, the OLD
/// path's crash log describes edits that now live (saved) in the new file:
/// drop it so reopening the old path never offers to "recover" edits the
/// user already saved elsewhere. (The old session — and with it the writer
/// handle — was already replaced by the switch.)
async fn discard_wal_for(state: &SharedState, path: &Path) {
    if let Some(root) = state.wal_root() {
        let p = ayame_core::wal::wal_path_for(root, path);
        let _ = tokio::fs::remove_file(p).await;
    }
}

/// Revert to the last SAVED state. Since in-place saves leave the on-disk
/// file holding exactly the saved bytes (the mmap'd pre-save inode lives on
/// under an aside name), "revert" is a reload from disk: fresh document,
/// empty overlay, empty history, clean saved marker.
pub(super) async fn api_edit_revert(
    State(state): State<SharedState>,
) -> Result<Json<EditStats>, (StatusCode, String)> {
    let _transitions = state.lock_transitions().await;
    let path = state.read(|ws| ws.doc_and_edits().map(|(doc, _)| doc.path().to_path_buf()))?;
    state.reload_reverted(path).await?;
    state.read(|ws| {
        let (doc, edits) = ws.doc_and_edits()?;
        Ok(Json(edits.stats(doc)))
    })
}

#[derive(Deserialize)]
pub(super) struct ReopenRequest {
    encoding: String,
}

/// Reopen the active file forcing a specific 文字コード — the recovery path when
/// auto-detection guessed wrong and the file reads as mojibake. Drops any
/// pending edits (the client confirms first).
pub(super) async fn api_reopen_encoding(
    State(state): State<SharedState>,
    Json(req): Json<ReopenRequest>,
) -> Result<Json<EditStats>, (StatusCode, String)> {
    let enc = Encoding::parse(&req.encoding)
        .map(|e| {
            if e == Encoding::Ascii {
                Encoding::Utf8
            } else {
                e
            }
        })
        .ok_or_else(|| bad_request(format!("unknown encoding '{}'", req.encoding)))?;
    if enc.is_wide() {
        return Err(bad_request(format!(
            "{} での再読込は未対応です",
            enc.label()
        )));
    }
    let _transitions = state.lock_transitions().await;
    let path = state.read(|ws| ws.doc_and_edits().map(|(doc, _)| doc.path().to_path_buf()))?;
    state.reload_with_encoding(path, enc).await?;
    state.read(|ws| {
        let (doc, edits) = ws.doc_and_edits()?;
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

/// Save the current selection (normal range or rectangle) to a file. Streams
/// in batches so the size is NOT limited by the clipboard cap; every batch
/// revalidates the pinned generation (document identity + edit revision) so a
/// concurrent edit or tab change aborts the write (409) instead of producing
/// a mixed-generation file. Output is the same text the clipboard copy would
/// produce: decoded view lines joined with '\n'.
#[derive(Deserialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct SelectionSaveRequest {
    path: String,
    #[serde(default)]
    overwrite: bool,
    #[serde(default)]
    rect: bool,
    l0: u64,
    c0: usize,
    l1: u64,
    c1: usize,
}

#[derive(Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct SelectionSaveResponse {
    path: String,
    lines: u64,
    bytes: u64,
}

const SELECTION_BATCH: u64 = 8192;

pub(super) async fn api_selection_save(
    State(state): State<SharedState>,
    Json(req): Json<SelectionSaveRequest>,
) -> Result<Json<SelectionSaveResponse>, (StatusCode, String)> {
    if req.path.trim().is_empty() {
        return Err(bad_request("保存先パスが空です"));
    }
    if req.l1 < req.l0 {
        return Err(bad_request("選択範囲が不正です"));
    }
    // A zero-width rectangle (c1 == c0) is a valid caret column: every line
    // contributes an empty piece, i.e. a newline-only column. Only a reversed
    // column range is rejected.
    if req.rect && req.c1 < req.c0 {
        return Err(bad_request("矩形選択の列範囲が不正です"));
    }
    let target = PathBuf::from(req.path.trim());
    if target.exists() && !req.overwrite {
        return Err((
            StatusCode::CONFLICT,
            format!("{} は既に存在します", workspace::display_path(&target)),
        ));
    }
    tokio::task::spawn_blocking(move || write_selection_to_file(&state, &req, &target))
        .await
        .map_err(internal)?
        .map(Json)
}

/// The exact state a selection export's coordinates refer to: the document
/// (by identity), the edit revision, and the view's total line count.
struct SelectionPin {
    doc: Arc<ayame_core::Document>,
    revision: u64,
    total: u64,
}

/// Pin the current view generation for a selection export.
fn pin_selection(ws: &super::state::Workspace) -> Option<SelectionPin> {
    let doc = ws.doc()?;
    Some(SelectionPin {
        doc: doc.clone(),
        revision: ws.edits.revision(),
        total: ws.edits.total_lines(doc),
    })
}

/// One batch of view lines, re-validated against the pin: same document
/// IDENTITY (a tab switch/open swaps the doc but starts a fresh session whose
/// revision and line count can coincide), same revision, same total. `None`
/// means the view moved under the export and the caller must abort.
fn pinned_selection_batch(
    ws: &super::state::Workspace,
    pin: &SelectionPin,
    start: u64,
    count: u64,
) -> Option<Vec<EditLine>> {
    let doc = ws.doc()?;
    if !Arc::ptr_eq(doc, &pin.doc)
        || ws.edits.revision() != pin.revision
        || ws.edits.total_lines(doc) != pin.total
    {
        return None;
    }
    Some(ws.edits.lines(doc, start, count))
}

fn write_selection_to_file(
    state: &SharedState,
    req: &SelectionSaveRequest,
    target: &Path,
) -> Result<SelectionSaveResponse, (StatusCode, String)> {
    use std::io::Write as _;
    // Pin the generation the selection coordinates refer to.
    let Some(pin) = state.read(pin_selection) else {
        return Err(bad_request("ファイルが開かれていません"));
    };
    let total0 = pin.total;
    if total0 == 0 || req.l0 >= total0 {
        return Err(bad_request("選択範囲が不正です (行が範囲外)"));
    }
    let last = req.l1.min(total0 - 1);
    // If the requested end line got clamped, take the (new) last line whole.
    let eff_c1 = if last == req.l1 { req.c1 } else { usize::MAX };

    let stage = overwrite_stage_path(target);
    // Client-facing label of the stage file (verbatim prefix stripped).
    let stage_label = workspace::display_path(&stage);
    let file =
        std::fs::File::create(&stage).map_err(|e| internal(format!("{stage_label}: {e}")))?;
    let mut out = std::io::BufWriter::new(file);
    let mut bytes: u64 = 0;
    let mut first = true;
    let mut start = req.l0;
    while start <= last {
        let count = SELECTION_BATCH.min(last - start + 1);
        let batch = state.read(|ws| pinned_selection_batch(ws, &pin, start, count));
        let Some(batch) = batch else {
            let _ = std::fs::remove_file(&stage);
            return Err((
                StatusCode::CONFLICT,
                "書き出し中に編集またはタブ切替が入ったため中断しました。もう一度実行してください"
                    .into(),
            ));
        };
        for (i, line) in batch.iter().enumerate() {
            let no = start + i as u64;
            let piece: String = if req.rect {
                line.text
                    .chars()
                    .skip(req.c0)
                    .take(req.c1.saturating_sub(req.c0))
                    .collect()
            } else {
                let from = if no == req.l0 { req.c0 } else { 0 };
                let to = if no == last { eff_c1 } else { usize::MAX };
                line.text
                    .chars()
                    .skip(from)
                    .take(to.saturating_sub(from))
                    .collect()
            };
            if !first {
                out.write_all(b"\n")
                    .map_err(|e| internal(format!("{stage_label}: {e}")))?;
                bytes += 1;
            }
            first = false;
            out.write_all(piece.as_bytes())
                .map_err(|e| internal(format!("{stage_label}: {e}")))?;
            bytes += piece.len() as u64;
        }
        start += count;
    }
    out.flush()
        .map_err(|e| internal(format!("{stage_label}: {e}")))?;
    drop(out);
    replace_existing_file(&stage, target)
        .map_err(|e| internal(format!("{}: {e}", workspace::display_path(target))))?;
    Ok(SelectionSaveResponse {
        path: workspace::display_path(target),
        lines: last - req.l0 + 1,
        bytes,
    })
}

#[cfg(test)]
mod tests {
    use ayame_core::{Document, OpenOptions};

    use super::super::state::AppState;
    use super::*;

    fn scratch_file(name: &str, contents: &[u8]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ayame-edit-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!(
            "{}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            name
        ));
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn selection_batch_rejects_a_swapped_document_with_matching_generation() {
        let fa = scratch_file("sel-pin-a.txt", b"a\nb\n");
        let fb = scratch_file("sel-pin-b.txt", b"x\ny\n");
        let doc = Document::open(&fa, &OpenOptions::default()).unwrap();
        let state: SharedState = Arc::new(AppState::new(Some(doc), OpenOptions::default()));

        let pin = state.read(pin_selection).expect("a document is open");
        assert_eq!((pin.revision, pin.total), (0, 2));
        assert!(
            state
                .read(|ws| pinned_selection_batch(ws, &pin, 0, 2))
                .is_some(),
            "the pinned document itself answers the batch"
        );

        // Swap in a DIFFERENT document whose fresh session has the same
        // revision (0) and the same total line count (2): only the identity
        // check can tell the two views apart.
        state
            .open_path(fb.to_string_lossy().to_string())
            .await
            .unwrap();
        assert!(
            state
                .read(|ws| pinned_selection_batch(ws, &pin, 0, 2))
                .is_none(),
            "a swapped document must abort the export even when revision and total coincide"
        );

        let _ = std::fs::remove_file(&fa);
        let _ = std::fs::remove_file(&fb);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn switch_to_saved_rejects_an_edit_that_raced_the_reload() {
        let fa = scratch_file("switch-race-a.txt", b"a\nb\n");
        let fb = scratch_file("switch-race-b.txt", b"x\ny\n");
        let doc = Document::open(&fa, &OpenOptions::default()).unwrap();
        let state: SharedState = Arc::new(AppState::new(Some(doc), OpenOptions::default()));

        let mut snap = state.edit_snapshot().unwrap();
        let _ = snap.take_edits(); // as api_edit_save does before streaming

        // An edit lands between the snapshot and the reload commit (the
        // save's streaming window) — the edit endpoints never take the
        // transitions lock, so only the revision re-check inside the
        // installing write can catch this.
        state.write(|ws| {
            let doc = ws.doc().unwrap().clone();
            ws.edits
                .replace_range(&doc, 0, 0, 0, 0, "typed during save")
                .unwrap();
        });

        {
            let _transitions = state.lock_transitions().await;
            let res = state.reload_reverted_if_unchanged(fb.clone(), &snap).await;
            assert_eq!(
                res.err().map(|(code, _)| code),
                Some(StatusCode::CONFLICT),
                "a racing edit must reject the clean-session reload with 409"
            );
        }
        // The racing edit survived — the session was not clobbered.
        assert!(
            state.read(|ws| ws.edits.has_edits()),
            "the racing edit must still be pending after the rejected switch"
        );

        let _ = std::fs::remove_file(&fa);
        let _ = std::fs::remove_file(&fb);
    }
}
