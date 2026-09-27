//! Sparse edit overlay for huge files.
//!
//! The base document remains an immutable mmap. Edits are stored as a small
//! line-oriented patch set keyed by original line number, then saved by streaming
//! original bytes plus patched fragments to a new file.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::OpenOptions;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::fsync::{fsync_parent, replace_with_staged, temp_path};
use crate::markers::MAX_MARKERS_PER_KIND;
use crate::wal::{LoggedOp, WalWriter};
use crate::{Document, Error, Result};

mod change;
mod history;
mod mutate;
mod overlay;
mod save;
#[cfg(test)]
mod tests;
mod wal;

pub use change::ChangeHistory;
pub(crate) use history::HISTORY_LIMIT;
use history::{HistoryEntry, UndoOp, UndoRecord};
pub(crate) use overlay::{OverlaySnapshot, RebaseSource};

#[derive(Debug)]
pub struct EditSession {
    events: BTreeMap<u64, EditEvent>,
    undo: Vec<HistoryEntry>,
    redo: Vec<HistoryEntry>,
    revision: u64,
    /// Identity of the current CONTENT. Unlike `revision` — which increases on
    /// every state change, undo/redo included, so optimistic save commits can
    /// detect interleaved changes — this value is RESTORED by undo/redo:
    /// walking back to a previously seen content reproduces its generation, so
    /// equality with `saved_gen` answers "is the view exactly the last-saved
    /// text?" even across undo/redo round-trips over a save.
    content_gen: u64,
    /// Allocator for fresh content generations. Never reused, so two distinct
    /// contents can never share a generation.
    next_gen: u64,
    /// The content generation last written to disk (0 = the document as
    /// opened). See [`EditSession::mark_saved`].
    saved_gen: u64,
    /// Sparse overlay that produced the last successfully saved bytes. Both
    /// this map and `events` stay anchored to the immutable, as-opened mmap,
    /// so change-history markers can compare the two without rereading or
    /// materializing the document. A save clones only edited anchors, then
    /// read-only viewport snapshots share that immutable map through `Arc`;
    /// memory remains proportional to edits, never to the document line count.
    saved_events: Arc<BTreeMap<u64, EditEvent>>,
    /// Attached crash log ([`crate::wal`]): every committed transaction is
    /// mirrored into it. Deliberately excluded from `Clone` — see the manual
    /// impl below.
    wal: Option<WalWriter>,
    /// First WAL write failure, kept so the caller can surface it once
    /// ([`EditSession::take_wal_error`]). The writer itself is dropped: an
    /// I/O problem with the crash log must never fail the edit.
    wal_error: Option<String>,
}

impl Default for EditSession {
    fn default() -> EditSession {
        EditSession {
            events: BTreeMap::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            revision: 0,
            content_gen: 0,
            next_gen: 1,
            saved_gen: 0,
            saved_events: Arc::new(BTreeMap::new()),
            wal: None,
            wal_error: None,
        }
    }
}

/// Cloning copies the FULL editing state (overlay, history, generations) but
/// deliberately NOT the WAL attachment. Clones are taken as save snapshots and
/// parked tab copies; if they carried the writer, one committed transaction
/// could be logged twice (or the single log file written from two owners).
/// The live session in the workspace is the only logger — a clone that should
/// log gets its own writer via [`EditSession::set_wal`].
impl Clone for EditSession {
    fn clone(&self) -> EditSession {
        EditSession {
            events: self.events.clone(),
            undo: self.undo.clone(),
            redo: self.redo.clone(),
            revision: self.revision,
            content_gen: self.content_gen,
            next_gen: self.next_gen,
            saved_gen: self.saved_gen,
            saved_events: Arc::clone(&self.saved_events),
            wal: None,
            wal_error: None,
        }
    }
}

impl EditSession {
    /// Clone only the content needed to render a stable read-only view.
    ///
    /// Unlike [`Clone`], this deliberately omits undo/redo records, WAL state,
    /// and pending WAL errors. The immutable last-saved overlay is shared, so
    /// viewport requests preserve read semantics without cloning it.
    #[must_use]
    pub fn view_clone(&self) -> EditSession {
        EditSession {
            events: self.events.clone(),
            undo: Vec::new(),
            redo: Vec::new(),
            revision: self.revision,
            content_gen: self.content_gen,
            next_gen: self.next_gen,
            saved_gen: self.saved_gen,
            saved_events: Arc::clone(&self.saved_events),
            wal: None,
            wal_error: None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct EditEvent {
    /// Lines inserted before this original line. The special anchor
    /// `original_line_count` means "append after the original file".
    inserts: Vec<String>,
    replacement: Option<String>,
    deleted: bool,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub struct EditLine {
    pub number: u64,
    pub text: String,
    pub edited: bool,
    pub inserted: bool,
    pub original_line: Option<u64>,
    /// True when `text` shows only the first [`Document::MAX_VIEW_LINE_BYTES`]
    /// of a longer original line (#201). Such a line is display-only: splicing
    /// edits into it are refused (they would rebuild the line from truncated
    /// text), and exports must not persist its text.
    pub truncated: bool,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct EditStats {
    pub dirty: bool,
    pub revision: u64,
    pub total_lines: u64,
    pub inserted_lines: u64,
    pub replaced_lines: u64,
    pub deleted_lines: u64,
    pub can_undo: bool,
    pub can_redo: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct SaveResult {
    pub path: PathBuf,
    pub bytes: u64,
    pub lines: u64,
}

/// One caret's edit inside a [`EditSession::replace_batch`] call: the span
/// (l0,c0)..(l1,c1) is replaced by `text`, with exactly the semantics of
/// [`EditSession::replace_range`]. All coordinates refer to the shared view
/// BEFORE the batch; columns are Unicode scalar (char) counts.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BatchEdit {
    pub l0: u64,
    pub c0: usize,
    pub l1: u64,
    pub c1: usize,
    pub text: String,
}

enum LineRef {
    Original(u64),
    Replaced(u64),
    Inserted { anchor: u64, index: usize },
}

fn capped_edit_text(text: &str, max_bytes: usize) -> (String, bool) {
    if text.len() <= max_bytes {
        return (text.to_owned(), false);
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_owned(), true)
}

impl EditSession {
    /// Whether the current content differs from the last save — the flag
    /// editors show as "unsaved changes". Compares content GENERATIONS, not
    /// overlay emptiness: after a save the (kept) history still allows undo,
    /// and undoing past the save makes the session dirty again even though the
    /// overlay may be empty, while redoing back to the exact saved state reads
    /// clean. A fresh session (generation 0, nothing saved yet) reads clean.
    pub fn is_dirty(&self) -> bool {
        self.content_gen != self.saved_gen
    }

    /// Whether the overlay holds any deviation from the RAW document (mmap).
    /// Distinct from [`EditSession::is_dirty`]: after an in-place save the
    /// session is clean (content == disk) yet the overlay is non-empty
    /// (content != the still-mapped pre-save document).
    pub fn has_edits(&self) -> bool {
        !self.events.is_empty()
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Identity of the current content; restored by undo/redo. Capture this
    /// alongside a snapshot that will be written to disk, then pass it to
    /// [`EditSession::mark_saved_at`] once the bytes land.
    pub fn content_gen(&self) -> u64 {
        self.content_gen
    }

    /// Record that the CURRENT content has been written to disk. Leaves the
    /// overlay and the undo/redo history untouched, so editing history — and
    /// undo across the save — keeps working.
    pub fn mark_saved(&mut self) {
        self.saved_gen = self.content_gen;
        self.saved_events = Arc::new(self.events.clone());
    }

    /// Record that the content identified by `gen` (a value previously
    /// returned by [`EditSession::content_gen`]) is what reached the disk.
    /// Use when edits may have arrived between snapshotting and the write
    /// completing: the session then stays dirty until the user undoes back to
    /// the generation that was actually saved.
    pub fn mark_saved_at(&mut self, gen: u64) {
        self.saved_gen = gen;
        if let Some(events) = self.events_at_generation(gen) {
            self.saved_events = Arc::new(events);
        }
    }

    /// Record the exact snapshot that reached disk. This is the preferred
    /// optimistic-save commit: it remains exact even when later edits race the
    /// filesystem swap or the target generation has fallen out of the bounded
    /// undo stack.
    pub fn mark_saved_from(&mut self, saved: &EditSession) {
        self.saved_gen = saved.content_gen;
        self.saved_events = Arc::new(saved.events.clone());
    }

    pub fn stats(&self, doc: &Document) -> EditStats {
        let original = doc.line_count();
        let mut inserted = 0u64;
        let mut replaced = 0u64;
        let mut deleted = 0u64;
        for (&anchor, ev) in &self.events {
            inserted += ev.inserts.len() as u64;
            if anchor < original {
                if ev.deleted {
                    deleted += 1;
                } else if ev.replacement.is_some() {
                    replaced += 1;
                }
            }
        }
        EditStats {
            dirty: self.is_dirty(),
            revision: self.revision,
            total_lines: original + inserted - deleted,
            inserted_lines: inserted,
            replaced_lines: replaced,
            deleted_lines: deleted,
            can_undo: self.can_undo(),
            can_redo: self.can_redo(),
        }
    }

    pub fn total_lines(&self, doc: &Document) -> u64 {
        self.stats(doc).total_lines
    }

    pub fn lines(&self, doc: &Document, start: u64, count: u64) -> Vec<EditLine> {
        let total = self.total_lines(doc);
        let end = start.saturating_add(count).min(total);
        let mut out = Vec::with_capacity((end - start).min(4096) as usize);
        let original_total = doc.line_count();
        let mut logical_pos = 0u64;
        let mut orig = 0u64;

        for (&anchor, ev) in &self.events {
            let anchor = anchor.min(original_total);
            if anchor < orig {
                continue;
            }
            let unchanged = anchor - orig;
            push_original_view_lines(&mut out, doc, orig, logical_pos, unchanged, start, end);
            logical_pos += unchanged;
            orig = anchor;

            for (index, text) in ev.inserts.iter().enumerate() {
                if logical_pos >= start && logical_pos < end {
                    let (text, truncated) = capped_edit_text(text, Document::MAX_VIEW_LINE_BYTES);
                    out.push(EditLine {
                        number: logical_pos,
                        text,
                        edited: true,
                        inserted: true,
                        original_line: None,
                        truncated,
                    });
                }
                logical_pos += 1;
                if logical_pos >= end && index + 1 == ev.inserts.len() {
                    return out;
                }
            }

            if anchor < original_total {
                if ev.deleted {
                    orig += 1;
                } else {
                    if logical_pos >= start && logical_pos < end {
                        if let Some(text) = &ev.replacement {
                            let (text, truncated) =
                                capped_edit_text(text, Document::MAX_VIEW_LINE_BYTES);
                            out.push(EditLine {
                                number: logical_pos,
                                text,
                                edited: true,
                                inserted: false,
                                original_line: Some(anchor),
                                truncated,
                            });
                        } else {
                            push_original_view_lines(
                                &mut out,
                                doc,
                                anchor,
                                logical_pos,
                                1,
                                start,
                                end,
                            );
                        }
                    }
                    logical_pos += 1;
                    orig += 1;
                }
            }
            if logical_pos >= end {
                return out;
            }
        }
        push_original_view_lines(
            &mut out,
            doc,
            orig,
            logical_pos,
            original_total.saturating_sub(orig),
            start,
            end,
        );
        out
    }

    pub fn line(&self, doc: &Document, logical: u64) -> Option<EditLine> {
        self.line_capped(doc, logical, Document::MAX_VIEW_LINE_BYTES)
    }

    /// Map a logical editor line back to the mmap-backed source when that
    /// mapping still exists. The boolean reports whether the logical text is
    /// edited: replaced lines retain an original line number for history but
    /// their current characters no longer have authoritative source bytes;
    /// inserted lines have no original line at all.
    pub fn line_origin(&self, doc: &Document, logical: u64) -> Option<(Option<u64>, bool)> {
        match self.locate(logical, doc.line_count())? {
            LineRef::Original(original) => Some((Some(original), false)),
            LineRef::Replaced(original) => Some((Some(original), true)),
            LineRef::Inserted { .. } => Some((None, true)),
        }
    }

    /// Read one overlay-resolved line through a caller-selected view cap.
    /// This applies equally to mmap-backed and unsaved inserted/replaced text,
    /// so bounded consumers never clone a giant edit before enforcing their
    /// own memory budget.
    pub fn line_capped(&self, doc: &Document, logical: u64, max_bytes: usize) -> Option<EditLine> {
        let max_bytes = max_bytes.min(Document::MAX_VIEW_LINE_BYTES);
        match self.locate(logical, doc.line_count())? {
            LineRef::Original(orig) => {
                let (text, truncated) = doc.line_view_capped(orig, max_bytes)?;
                Some(EditLine {
                    number: logical,
                    text,
                    edited: false,
                    inserted: false,
                    original_line: Some(orig),
                    truncated,
                })
            }
            LineRef::Replaced(orig) => {
                let source = self.events.get(&orig)?.replacement.as_ref()?;
                let (text, truncated) = capped_edit_text(source, max_bytes);
                Some(EditLine {
                    number: logical,
                    text,
                    edited: true,
                    inserted: false,
                    original_line: Some(orig),
                    truncated,
                })
            }
            LineRef::Inserted { anchor, index } => {
                let source = self.events.get(&anchor)?.inserts.get(index)?;
                let (text, truncated) = capped_edit_text(source, max_bytes);
                Some(EditLine {
                    number: logical,
                    text,
                    edited: true,
                    inserted: true,
                    original_line: None,
                    truncated,
                })
            }
        }
    }

    /// Current (overlay-resolved) text of a logical line, for rebuilding the
    /// line around a splice. Refuses lines past the view cap: rebuilding one
    /// from truncated text would silently drop the rest of the line (#201),
    /// which is strictly worse than the error.
    fn line_text(&self, doc: &Document, logical: u64) -> Result<String> {
        let line = self
            .line(doc, logical)
            .ok_or_else(|| Error::InvalidInput(format!("line {} is out of range", logical + 1)))?;
        if line.truncated {
            return Err(Error::UnsupportedFeature(format!(
                "line {} is longer than {} bytes and cannot be edited in place; \
                 use replace / grep-lines / case transforms for such lines",
                logical + 1,
                Document::MAX_VIEW_LINE_BYTES
            )));
        }
        Ok(line.text)
    }

    fn locate(&self, logical: u64, original_total: u64) -> Option<LineRef> {
        let mut logical_pos = 0u64;
        let mut orig = 0u64;

        for (&anchor, ev) in &self.events {
            let anchor = anchor.min(original_total);
            if anchor < orig {
                continue;
            }
            let unchanged = anchor - orig;
            if logical < logical_pos + unchanged {
                return Some(LineRef::Original(orig + (logical - logical_pos)));
            }
            logical_pos += unchanged;
            orig = anchor;

            let inserted = ev.inserts.len() as u64;
            if logical < logical_pos + inserted {
                return Some(LineRef::Inserted {
                    anchor,
                    index: (logical - logical_pos) as usize,
                });
            }
            logical_pos += inserted;

            if anchor < original_total {
                if ev.deleted {
                    orig += 1;
                } else {
                    if logical == logical_pos {
                        return if ev.replacement.is_some() {
                            Some(LineRef::Replaced(anchor))
                        } else {
                            Some(LineRef::Original(anchor))
                        };
                    }
                    logical_pos += 1;
                    orig += 1;
                }
            }
        }

        let unchanged = original_total - orig;
        if logical < logical_pos + unchanged {
            Some(LineRef::Original(orig + (logical - logical_pos)))
        } else {
            None
        }
    }
}

fn push_original_view_lines(
    out: &mut Vec<EditLine>,
    doc: &Document,
    orig_start: u64,
    logical_start: u64,
    count: u64,
    view_start: u64,
    view_end: u64,
) {
    let span_start = logical_start.max(view_start);
    let span_end = logical_start.saturating_add(count).min(view_end);
    if span_start >= span_end {
        return;
    }
    let mut orig = orig_start + (span_start - logical_start);
    let mut logical = span_start;
    let mut remaining = span_end - span_start;
    const BATCH: u64 = 8192;
    while remaining > 0 {
        let batch = doc.raw_line_ranges(orig, remaining.min(BATCH));
        if batch.is_empty() {
            break;
        }
        let advanced = batch.len() as u64;
        for (line_no, raw) in batch {
            let (text, truncated) = doc
                .encoding()
                .decode_line_capped(raw, Document::MAX_VIEW_LINE_BYTES);
            out.push(EditLine {
                number: logical,
                text,
                edited: false,
                inserted: false,
                original_line: Some(line_no),
                truncated,
            });
            logical += 1;
        }
        orig += advanced;
        remaining -= advanced;
    }
}
