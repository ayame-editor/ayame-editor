//! Session-scoped sparse document markers.
//!
//! `ayame-core::MarkerSet` owns the ordered sparse data structure.  This
//! module adds the editor-session policy around it:
//!
//! * marker state travels with a tab but is never written into the document;
//! * line-changing edits and undo/redo transform markers in the same
//!   transaction order as the edit overlay;
//! * API reads enumerate only requested markers or previews, never document-
//!   sized arrays.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use anyhow::Result;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use ayame_core::{
    BatchEdit, Document, EditLine, EditSession, LineEdit, LineMarker, MarkerKind, MarkerSet,
    MarkerTransform, MAX_MARKERS_PER_KIND,
};
use serde::{Deserialize, Serialize};

use super::{bad_request, ApiError, SharedState};

const MARKER_HISTORY_LIMIT: usize = 256;
const DEFAULT_LIST_LIMIT: usize = 200;
const MAX_LIST_LIMIT: usize = 2_000;
const MAX_BULK_LINES: usize = 100_000;
const MARKER_SAVE_BATCH: usize = 2_048;
const PREVIEW_CHARS: usize = 160;
const CHANGE_OVERVIEW_BINS: usize = 2_048;

type MarkerRecord = Vec<MarkerTransform>;

/// Marker state and the coordinate-history sidecar kept in lockstep with one
/// `EditSession`.
#[derive(Debug, Default)]
pub(super) struct MarkerSession {
    set: MarkerSet,
    undo: Vec<MarkerRecord>,
    redo: Vec<MarkerRecord>,
    revision: u64,
    change_limit_reached: bool,
}

impl MarkerSession {
    pub(super) fn set(&self) -> &MarkerSet {
        &self.set
    }

    pub(super) const fn revision(&self) -> u64 {
        self.revision
    }

    /// Apply marker transforms before a content edit.  The returned record is
    /// either committed when the content revision advances or rolled back when
    /// the edit is rejected/no-op, keeping both states atomic.
    pub(super) fn begin(
        &mut self,
        edits: impl IntoIterator<Item = LineEdit>,
    ) -> Result<MarkerRecord> {
        let mut inverse = Vec::new();
        for edit in edits {
            match self.set.apply(MarkerTransform::new(edit)) {
                Ok(op) => inverse.push(op),
                Err(error) => {
                    let _ = self.apply_record(inverse);
                    return Err(error.into());
                }
            }
        }
        Ok(inverse)
    }

    /// The content edit succeeded: keep its inverse as one undo generation.
    pub(super) fn commit(&mut self, mut inverse: MarkerRecord) {
        // Change-history partitions are regenerated from EditSession after
        // every successful content transition. Do not duplicate their old
        // rows inside up to 256 coordinate-history records; rollback still
        // receives the unmodified record before this commit point.
        scrub_change_restores(&mut inverse);
        push_history(&mut self.undo, inverse);
        self.redo.clear();
        self.bump();
    }

    /// The content edit failed or was a no-op: restore the marker state and do
    /// not create a history generation.
    pub(super) fn rollback(&mut self, inverse: MarkerRecord) {
        let _ = self.apply_record(inverse);
    }

    /// Follow one successful content undo.
    pub(super) fn undo(&mut self) {
        let Some(record) = self.undo.pop() else {
            return;
        };
        let mut inverse = self.apply_record(record);
        scrub_change_restores(&mut inverse);
        push_history(&mut self.redo, inverse);
        self.bump();
    }

    /// Follow one successful content redo.
    pub(super) fn redo(&mut self) {
        let Some(record) = self.redo.pop() else {
            return;
        };
        let mut inverse = self.apply_record(record);
        scrub_change_restores(&mut inverse);
        push_history(&mut self.undo, inverse);
        self.bump();
    }

    pub(super) fn toggle(&mut self, kind: MarkerKind, line: u64) -> ayame_core::Result<bool> {
        let marked = self.set.toggle(kind, line)?;
        self.bump();
        Ok(marked)
    }

    /// Add a bounded page of markers. Input is deduplicated and ordered so a
    /// full safety cap keeps the earliest matching lines deterministically.
    pub(super) fn add_lines(
        &mut self,
        kind: MarkerKind,
        lines: impl IntoIterator<Item = u64>,
    ) -> ayame_core::Result<(usize, bool)> {
        let lines: BTreeSet<u64> = lines.into_iter().collect();
        let mut added = 0;
        let mut limit_reached = false;
        for line in lines {
            if self.set.contains(kind, line) {
                continue;
            }
            if self.set.len(kind) == MAX_MARKERS_PER_KIND {
                limit_reached = true;
                break;
            }
            self.set.insert(kind, line)?;
            added += 1;
        }
        if added != 0 {
            self.bump();
        }
        Ok((added, limit_reached))
    }

    pub(super) fn clear(&mut self, kind: MarkerKind) {
        let mut changed = self.set.len(kind) != 0;
        if self.set.len(kind) != 0 {
            self.set.clear(kind);
        }
        // A line replacement may be holding an off-screen marker in its
        // inverse record (e.g. a bookmark on a deleted line). "Clear all" is
        // explicit destruction, so scrub those saved entries too; a later
        // text undo must not resurrect a bookmark the user cleared.
        for record in self.undo.iter_mut().chain(&mut self.redo) {
            for transform in record {
                let before = transform.restore.len();
                transform.restore.retain(|marker| marker.kind != kind);
                changed |= transform.restore.len() != before;
            }
        }
        if changed {
            self.bump();
        }
    }

    /// Rebuild the three change-history partitions from the core's sparse
    /// overlay comparison. Bookmarks and future marker kinds stay untouched;
    /// this is the one source consumed by both viewport rows and overview
    /// histograms.
    pub(super) fn sync_change_history(&mut self, edits: &EditSession, doc: &Document) {
        let history = edits.change_history(doc);
        let limit_changed = self.change_limit_reached != history.limit_reached;
        self.change_limit_reached = history.limit_reached;
        let mut changed = limit_changed;
        changed |= self
            .set
            .replace_lines(MarkerKind::ChangeSaved, history.saved)
            .expect("core change history respects the marker admission cap");
        changed |= self
            .set
            .replace_lines(MarkerKind::ChangeUnsaved, history.unsaved)
            .expect("core change history respects the marker admission cap");
        changed |= self
            .set
            .replace_lines(MarkerKind::ChangeDeleted, history.deleted)
            .expect("core change history respects the marker admission cap");
        if changed {
            self.bump();
        }
    }

    fn apply_record(&mut self, record: MarkerRecord) -> MarkerRecord {
        let mut inverse = Vec::with_capacity(record.len());
        for op in record.into_iter().rev() {
            // Every record was produced by MarkerSet itself against the exact
            // preceding coordinate state; replay cannot overflow unless the
            // process already holds an impossible u64-sized document.
            if let Ok(op) = self.set.apply(op) {
                inverse.push(op);
            }
        }
        inverse
    }

    fn bump(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }
}

fn push_history(stack: &mut Vec<MarkerRecord>, record: MarkerRecord) {
    if stack.len() == MARKER_HISTORY_LIMIT {
        stack.remove(0);
    }
    stack.push(record);
}

fn is_change_kind(kind: MarkerKind) -> bool {
    matches!(
        kind,
        MarkerKind::ChangeSaved | MarkerKind::ChangeUnsaved | MarkerKind::ChangeDeleted
    )
}

fn scrub_change_restores(record: &mut MarkerRecord) {
    for transform in record {
        transform
            .restore
            .retain(|marker| !is_change_kind(marker.kind));
    }
}

/// Marker transforms in the same bottom-to-top order used by
/// `EditSession::replace_batch`.
pub(super) fn batch_line_edits(edits: &[BatchEdit], mut total_lines: u64) -> Result<Vec<LineEdit>> {
    let mut order: Vec<usize> = (0..edits.len()).collect();
    order.sort_by_key(|&i| (edits[i].l0, edits[i].c0, edits[i].l1, edits[i].c1));
    order
        .into_iter()
        .rev()
        .map(|i| {
            let edit = &edits[i];
            // EditSession treats the first insertion into a zero-line
            // document as 0 -> N. Every later entry in that same batch sees
            // the newly seeded line and follows ordinary replacement rules.
            let old_lines = if total_lines == 0 {
                0
            } else {
                edit.l1
                    .checked_sub(edit.l0)
                    .and_then(|n| n.checked_add(1))
                    .ok_or_else(|| anyhow::anyhow!("invalid or overflowing marker edit range"))?
            };
            let new_lines = edit.text.split('\n').count() as u64;
            total_lines = total_lines
                .checked_sub(old_lines)
                .and_then(|n| n.checked_add(new_lines))
                .ok_or_else(|| anyhow::anyhow!("overflowing marker document length"))?;
            Ok(LineEdit::replacement(edit.l0, old_lines, new_lines))
        })
        .collect()
}

pub(super) fn range_line_edit(total_lines: u64, l0: u64, l1: u64, text: &str) -> Result<LineEdit> {
    let old_lines = if total_lines == 0 {
        0
    } else {
        l1.checked_sub(l0)
            .and_then(|n| n.checked_add(1))
            .ok_or_else(|| anyhow::anyhow!("invalid or overflowing marker edit range"))?
    };
    Ok(LineEdit::replacement(
        l0,
        old_lines,
        text.split('\n').count() as u64,
    ))
}

fn parse_kind(value: &str) -> Result<MarkerKind, ApiError> {
    MarkerKind::from_str(value).map_err(bad_request)
}

fn parse_mutable_kind(value: &str) -> Result<MarkerKind, ApiError> {
    let kind = parse_kind(value)?;
    if is_change_kind(kind) {
        return Err(bad_request(
            "change-history markers are derived from document edits",
        ));
    }
    Ok(kind)
}

#[derive(Deserialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct MarkerToggleRequest {
    kind: String,
    line: u64,
}

#[derive(Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct MarkerMutationResponse {
    kind: String,
    line: u64,
    marked: bool,
    count: u64,
    limit: u64,
}

pub(super) async fn api_marker_toggle(
    State(state): State<SharedState>,
    Json(req): Json<MarkerToggleRequest>,
) -> Result<Json<MarkerMutationResponse>, ApiError> {
    let kind = parse_mutable_kind(&req.kind)?;
    state.write(|ws| {
        let total = {
            let (doc, edits) = ws.doc_and_edits()?;
            edits.total_lines(doc)
        };
        if req.line >= total {
            return Err(bad_request(format!(
                "line {} is outside the document",
                req.line.saturating_add(1)
            )));
        }
        let markers = ws.markers_mut();
        let marked = markers.toggle(kind, req.line).map_err(bad_request)?;
        Ok(Json(MarkerMutationResponse {
            kind: kind.as_str().into(),
            line: req.line,
            marked,
            count: markers.set().len(kind) as u64,
            limit: MAX_MARKERS_PER_KIND as u64,
        }))
    })
}

#[derive(Deserialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct MarkerBulkRequest {
    kind: String,
    lines: Vec<u64>,
}

#[derive(Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct MarkerBulkResponse {
    kind: String,
    added: u64,
    count: u64,
    limit: u64,
    limit_reached: bool,
}

/// Add one bounded search-result page without ever accepting a document-sized
/// request body. Repeated and previously marked lines are harmless.
pub(super) async fn api_marker_add(
    State(state): State<SharedState>,
    Json(req): Json<MarkerBulkRequest>,
) -> Result<Json<MarkerBulkResponse>, ApiError> {
    let kind = parse_mutable_kind(&req.kind)?;
    if req.lines.len() > MAX_BULK_LINES {
        return Err(bad_request(format!(
            "marker page exceeds {MAX_BULK_LINES} lines"
        )));
    }
    state.write(|ws| {
        let total = {
            let (doc, edits) = ws.doc_and_edits()?;
            edits.total_lines(doc)
        };
        if let Some(line) = req.lines.iter().find(|&&line| line >= total) {
            return Err(bad_request(format!(
                "line {} is outside the document",
                line.saturating_add(1)
            )));
        }
        let markers = ws.markers_mut();
        let (added, limit_reached) = markers.add_lines(kind, req.lines).map_err(bad_request)?;
        Ok(Json(MarkerBulkResponse {
            kind: kind.as_str().into(),
            added: added as u64,
            count: markers.set().len(kind) as u64,
            limit: MAX_MARKERS_PER_KIND as u64,
            limit_reached,
        }))
    })
}

#[derive(Deserialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct MarkerClearRequest {
    kind: String,
}

pub(super) async fn api_marker_clear(
    State(state): State<SharedState>,
    Json(req): Json<MarkerClearRequest>,
) -> Result<Json<MarkerMutationResponse>, ApiError> {
    let kind = parse_mutable_kind(&req.kind)?;
    state.write(|ws| {
        ws.doc_and_edits()?;
        let markers = ws.markers_mut();
        markers.clear(kind);
        Ok(Json(MarkerMutationResponse {
            kind: kind.as_str().into(),
            line: 0,
            marked: false,
            count: 0,
            limit: MAX_MARKERS_PER_KIND as u64,
        }))
    })
}

#[derive(Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct ChangeMarkerOverview {
    count: u64,
    histogram: Vec<u32>,
}

#[derive(Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct ChangeHistoryResponse {
    revision: u64,
    total_lines: u64,
    saved: ChangeMarkerOverview,
    unsaved: ChangeMarkerOverview,
    deleted: ChangeMarkerOverview,
    limit_reached: bool,
}

fn change_overview(set: &MarkerSet, kind: MarkerKind, total_lines: u64) -> ChangeMarkerOverview {
    ChangeMarkerOverview {
        count: set.len(kind) as u64,
        histogram: set.histogram(kind, total_lines, CHANGE_OVERVIEW_BINS),
    }
}

/// Fixed-size position-pane image of the same sparse MarkerSet used by
/// `/api/lines`. Even one million admitted change rows produce exactly 2,048
/// bins per shape/status kind and never one DOM/API entry per document line.
pub(super) async fn api_change_history(
    State(state): State<SharedState>,
) -> Json<ChangeHistoryResponse> {
    state.read(|ws| {
        let total_lines = ws.doc().map(|doc| ws.edits.total_lines(doc)).unwrap_or(0);
        let markers = ws.markers();
        let set = markers.set();
        Json(ChangeHistoryResponse {
            revision: markers.revision(),
            total_lines,
            saved: change_overview(set, MarkerKind::ChangeSaved, total_lines),
            unsaved: change_overview(set, MarkerKind::ChangeUnsaved, total_lines),
            deleted: change_overview(set, MarkerKind::ChangeDeleted, total_lines),
            limit_reached: markers.change_limit_reached,
        })
    })
}

#[derive(Deserialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct MarkerSaveRequest {
    kind: String,
    path: String,
    #[serde(default)]
    overwrite: bool,
}

#[derive(Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct MarkerSaveResponse {
    path: String,
    lines: u64,
    bytes: u64,
}

struct MarkerSavePin {
    doc: Arc<Document>,
    edit_revision: u64,
    marker_revision: u64,
    kind: MarkerKind,
    markers: u64,
}

fn marker_pin_matches(ws: &super::state::Workspace, pin: &MarkerSavePin) -> bool {
    ws.doc().is_some_and(|doc| Arc::ptr_eq(doc, &pin.doc))
        && ws.edits.revision() == pin.edit_revision
        && ws.markers().revision() == pin.marker_revision
}

fn pinned_marker_batch(
    ws: &super::state::Workspace,
    pin: &MarkerSavePin,
    start: u64,
) -> Option<Vec<(u64, EditLine)>> {
    if !marker_pin_matches(ws, pin) {
        return None;
    }
    let lines = ws.markers().set().from(pin.kind, start, MARKER_SAVE_BATCH);
    lines
        .into_iter()
        .map(|line| ws.edits.line(&pin.doc, line).map(|record| (line, record)))
        .collect()
}

fn marker_save_conflict() -> ApiError {
    ApiError::new(
        StatusCode::CONFLICT,
        "conflict",
        "bookmarks or document changed during export; try again",
    )
}

fn write_markers_to_file(
    state: &SharedState,
    kind: MarkerKind,
    target: &Path,
) -> Result<MarkerSaveResponse, ApiError> {
    use std::io::Write as _;

    let pin = state
        .read(|ws| {
            let doc = ws.doc()?.clone();
            Some(MarkerSavePin {
                doc,
                edit_revision: ws.edits.revision(),
                marker_revision: ws.markers().revision(),
                kind,
                markers: ws.markers().set().len(kind) as u64,
            })
        })
        .ok_or_else(|| bad_request("no document is open"))?;
    if pin.markers == 0 {
        return Err(bad_request("there are no markers to export"));
    }

    let stage = super::edit::overwrite_stage_path(target);
    let stage_label = super::workspace::display_path(&stage);
    let file = std::fs::File::create(&stage)
        .map_err(|error| super::internal(format!("{stage_label}: {error}")))?;
    let mut output = std::io::BufWriter::new(file);
    let stream_result = (|| {
        let mut bytes = 0u64;
        let mut written = 0u64;
        let mut start = 0u64;
        while written < pin.markers {
            let batch = state
                .read(|ws| pinned_marker_batch(ws, &pin, start))
                .ok_or_else(marker_save_conflict)?;
            if batch.is_empty() {
                return Err(marker_save_conflict());
            }
            for (line_number, line) in &batch {
                if line.truncated {
                    return Err(bad_request(format!(
                        "line {} exceeds the preview limit and cannot be exported safely",
                        line_number + 1
                    )));
                }
                if written != 0 {
                    output
                        .write_all(b"\n")
                        .map_err(|error| super::internal(format!("{stage_label}: {error}")))?;
                    bytes += 1;
                }
                output
                    .write_all(line.text.as_bytes())
                    .map_err(|error| super::internal(format!("{stage_label}: {error}")))?;
                bytes += line.text.len() as u64;
                written += 1;
            }
            let last = batch.last().map(|(line, _)| *line).unwrap_or(start);
            if written < pin.markers {
                start = last.checked_add(1).ok_or_else(marker_save_conflict)?;
            }
        }
        output
            .flush()
            .map_err(|error| super::internal(format!("{stage_label}: {error}")))?;
        Ok((written, bytes))
    })();
    drop(output);
    let result = stream_result.and_then(|(written, bytes)| {
        super::edit::replace_existing_file(&stage, target)
            .map_err(|error| {
                super::internal(format!(
                    "{}: {error}",
                    super::workspace::display_path(target)
                ))
            })
            .map(|_| MarkerSaveResponse {
                path: super::workspace::display_path(target),
                lines: written,
                bytes,
            })
    });
    if result.is_err() {
        let _ = std::fs::remove_file(stage);
    }
    result
}

/// Stream bookmarked view lines to a UTF-8 text file. Only sparse marker pages
/// and one bounded line batch are resident; a concurrent edit/marker change
/// aborts rather than mixing coordinate generations.
pub(super) async fn api_marker_save(
    State(state): State<SharedState>,
    Json(req): Json<MarkerSaveRequest>,
) -> Result<Json<MarkerSaveResponse>, ApiError> {
    let kind = parse_kind(&req.kind)?;
    let path = req.path.trim();
    if path.is_empty() {
        return Err(bad_request("output path is empty"));
    }
    let target = PathBuf::from(path);
    if target.exists() && !req.overwrite {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "exists",
            format!("{} already exists", super::workspace::display_path(&target)),
        ));
    }
    tokio::task::spawn_blocking(move || write_markers_to_file(&state, kind, &target))
        .await
        .map_err(super::internal)?
        .map(Json)
}

#[derive(Deserialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct MarkerListQuery {
    kind: String,
    #[serde(default)]
    start: u64,
    #[serde(default = "default_list_limit")]
    limit: usize,
}

const fn default_list_limit() -> usize {
    DEFAULT_LIST_LIMIT
}

#[derive(Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct MarkerListResponse {
    kind: String,
    total: u64,
    lines: Vec<u64>,
    truncated: bool,
}

pub(super) async fn api_markers(
    State(state): State<SharedState>,
    Query(req): Query<MarkerListQuery>,
) -> Result<Json<MarkerListResponse>, ApiError> {
    let kind = parse_kind(&req.kind)?;
    let limit = req.limit.clamp(1, MAX_LIST_LIMIT);
    state.read(|ws| {
        ws.doc_and_edits()?;
        let total = ws.markers().set().len(kind);
        let mut lines = ws.markers().set().from(kind, req.start, limit + 1);
        let truncated = lines.len() > limit;
        lines.truncate(limit);
        Ok(Json(MarkerListResponse {
            kind: kind.as_str().into(),
            total: total as u64,
            lines,
            truncated,
        }))
    })
}

#[derive(Deserialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct MarkerRangeCountsQuery {
    start: u64,
    end: u64,
}

#[derive(Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct MarkerRangeCountsResponse {
    bookmarks: u64,
    search_rules: u64,
    change_saved: u64,
    change_unsaved: u64,
    change_deleted: u64,
}

/// Sparse badge counts for a folded half-open logical-line range.
pub(super) async fn api_marker_range_counts(
    State(state): State<SharedState>,
    Query(req): Query<MarkerRangeCountsQuery>,
) -> Result<Json<MarkerRangeCountsResponse>, ApiError> {
    if req.start > req.end {
        return Err(bad_request("marker count range start exceeds end"));
    }
    state.read(|ws| {
        let total = {
            let (doc, edits) = ws.doc_and_edits()?;
            edits.total_lines(doc)
        };
        if req.end > total {
            return Err(bad_request("marker count range exceeds the document"));
        }
        let set = ws.markers().set();
        Ok(Json(MarkerRangeCountsResponse {
            bookmarks: set.range_count(MarkerKind::Bookmark, req.start, req.end) as u64,
            search_rules: set.range_count(MarkerKind::SearchRule, req.start, req.end) as u64,
            change_saved: set.range_count(MarkerKind::ChangeSaved, req.start, req.end) as u64,
            change_unsaved: set.range_count(MarkerKind::ChangeUnsaved, req.start, req.end) as u64,
            change_deleted: set.range_count(MarkerKind::ChangeDeleted, req.start, req.end) as u64,
        }))
    })
}

#[derive(Deserialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct MarkerNavigateQuery {
    kind: String,
    from: u64,
    direction: String,
    #[serde(default = "default_true")]
    wrap: bool,
}

const fn default_true() -> bool {
    true
}

#[derive(Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct MarkerNavigateResponse {
    kind: String,
    line: Option<u64>,
    count: u64,
    wrapped: bool,
}

pub(super) async fn api_marker_navigate(
    State(state): State<SharedState>,
    Query(req): Query<MarkerNavigateQuery>,
) -> Result<Json<MarkerNavigateResponse>, ApiError> {
    let kind = parse_kind(&req.kind)?;
    state.read(|ws| {
        ws.doc_and_edits()?;
        let set = ws.markers().set();
        let (line, wrapped) = match req.direction.as_str() {
            "next" => {
                let direct = req
                    .from
                    .checked_add(1)
                    .and_then(|line| set.next_at_or_after(kind, line));
                match direct {
                    Some(line) => (Some(line), false),
                    None if req.wrap => (set.first(kind), set.first(kind).is_some()),
                    None => (None, false),
                }
            }
            "previous" | "prev" => {
                let direct = req
                    .from
                    .checked_sub(1)
                    .and_then(|line| set.previous_at_or_before(kind, line));
                match direct {
                    Some(line) => (Some(line), false),
                    None if req.wrap => (set.last(kind), set.last(kind).is_some()),
                    None => (None, false),
                }
            }
            other => {
                return Err(bad_request(format!(
                    "unknown marker navigation direction '{other}'"
                )))
            }
        };
        Ok(Json(MarkerNavigateResponse {
            kind: kind.as_str().into(),
            line,
            count: set.len(kind) as u64,
            wrapped,
        }))
    })
}

#[derive(Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct MarkerPreview {
    line: u64,
    text: String,
    truncated: bool,
}

#[derive(Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct MarkerPreviewResponse {
    kind: String,
    total: u64,
    entries: Vec<MarkerPreview>,
    truncated: bool,
}

/// Paginated marker list with short line previews for the bookmark panel.
pub(super) async fn api_marker_previews(
    State(state): State<SharedState>,
    Query(req): Query<MarkerListQuery>,
) -> Result<Json<MarkerPreviewResponse>, ApiError> {
    let kind = parse_kind(&req.kind)?;
    let limit = req.limit.clamp(1, DEFAULT_LIST_LIMIT);
    let (doc, edits, total, mut lines) = state.read(|ws| {
        let (doc, edits) = ws.doc_and_edits()?;
        let set = ws.markers().set();
        Ok::<_, ApiError>((
            doc.clone(),
            edits.view_clone(),
            set.len(kind),
            set.from(kind, req.start, limit + 1),
        ))
    })?;
    let truncated = lines.len() > limit;
    lines.truncate(limit);
    let entries = tokio::task::spawn_blocking(move || {
        lines
            .into_iter()
            .filter_map(|line| {
                edits.line(&doc, line).map(|record| {
                    let mut chars = record.text.chars();
                    let text: String = chars.by_ref().take(PREVIEW_CHARS).collect();
                    MarkerPreview {
                        line,
                        text,
                        truncated: record.truncated || chars.next().is_some(),
                    }
                })
            })
            .collect()
    })
    .await
    .map_err(super::internal)?;
    Ok(Json(MarkerPreviewResponse {
        kind: kind.as_str().into(),
        total: total as u64,
        entries,
        truncated,
    }))
}

/// Convert visible marker entries into the response shape shared by
/// `/api/lines`.  Kept here so the viewport handler never knows the storage
/// representation.
pub(super) fn visible_markers(markers: &MarkerSession, start: u64, end: u64) -> Vec<LineMarker> {
    markers
        .set()
        .all_in_range(start, end, super::MAX_VIEW as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_history_tracks_replace_undo_redo_without_snapshots() {
        let mut session = MarkerSession::default();
        session.toggle(MarkerKind::Bookmark, 2).unwrap();
        session.toggle(MarkerKind::Bookmark, 8).unwrap();

        let pending = session.begin([LineEdit::replacement(3, 2, 5)]).unwrap();
        session.commit(pending);
        assert_eq!(
            session.set().range(MarkerKind::Bookmark, 0, 20, 20),
            vec![2, 11]
        );

        session.undo();
        assert_eq!(
            session.set().range(MarkerKind::Bookmark, 0, 20, 20),
            vec![2, 8]
        );
        session.redo();
        assert_eq!(
            session.set().range(MarkerKind::Bookmark, 0, 20, 20),
            vec![2, 11]
        );
    }

    #[test]
    fn derived_change_markers_never_enter_coordinate_history() {
        let mut session = MarkerSession::default();
        let pending = session.begin([LineEdit::replacement(0, 1, 2)]).unwrap();
        session.commit(pending);

        // This is the state a change-history resync can produce on the newly
        // inserted second line. A user bookmark at the same coordinate must
        // retain ordinary marker undo/redo semantics.
        session.set.insert(MarkerKind::ChangeUnsaved, 1).unwrap();
        session.set.insert(MarkerKind::Bookmark, 1).unwrap();
        session.undo();
        let redo_restore = &session.redo.last().unwrap()[0].restore;
        assert!(redo_restore
            .iter()
            .any(|marker| marker.kind == MarkerKind::Bookmark));
        assert!(redo_restore
            .iter()
            .all(|marker| !is_change_kind(marker.kind)));

        session.redo();
        assert!(session.set.contains(MarkerKind::Bookmark, 1));
        assert!(session.undo.iter().flatten().all(|transform| transform
            .restore
            .iter()
            .all(|marker| !is_change_kind(marker.kind))));
    }

    #[test]
    fn rejected_edit_rolls_marker_coordinates_back() {
        let mut session = MarkerSession::default();
        session.toggle(MarkerKind::Bookmark, 50).unwrap();
        let pending = session.begin([LineEdit::replacement(10, 1, 10)]).unwrap();
        assert!(session.set().contains(MarkerKind::Bookmark, 59));
        session.rollback(pending);
        assert!(session.set().contains(MarkerKind::Bookmark, 50));
        assert!(!session.set().contains(MarkerKind::Bookmark, 59));
    }

    #[test]
    fn batch_transforms_are_bottom_to_top() {
        let edits = [
            BatchEdit {
                l0: 2,
                c0: 0,
                l1: 2,
                c1: 0,
                text: "a\nb".into(),
            },
            BatchEdit {
                l0: 10,
                c0: 0,
                l1: 12,
                c1: 0,
                text: "x".into(),
            },
        ];
        assert_eq!(
            batch_line_edits(&edits, 20).unwrap(),
            vec![
                LineEdit::replacement(10, 3, 1),
                LineEdit::replacement(2, 1, 2)
            ]
        );
    }

    #[test]
    fn bulk_add_deduplicates_and_tracks_marker_revision() {
        let mut session = MarkerSession::default();
        assert_eq!(session.revision(), 0);
        let (added, limited) = session
            .add_lines(MarkerKind::Bookmark, [9, 2, 9, 2, 5])
            .unwrap();
        assert_eq!((added, limited), (3, false));
        assert_eq!(
            session.set().range(MarkerKind::Bookmark, 0, 20, 20),
            vec![2, 5, 9]
        );
        assert_eq!(session.revision(), 1);
    }

    #[test]
    fn first_edit_in_empty_document_is_an_insertion() {
        assert_eq!(
            range_line_edit(0, 0, 0, "first").unwrap(),
            LineEdit::replacement(0, 0, 1)
        );

        let edits = [BatchEdit {
            l0: 0,
            c0: 0,
            l1: 0,
            c1: 0,
            text: "first\nsecond".into(),
        }];
        assert_eq!(
            batch_line_edits(&edits, 0).unwrap(),
            vec![LineEdit::replacement(0, 0, 2)]
        );
    }
}
