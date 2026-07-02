//! Sparse edit overlay for huge files.
//!
//! The base document remains an immutable mmap. Edits are stored as a small
//! line-oriented patch set keyed by original line number, then saved by streaming
//! original bytes plus patched fragments to a new file.

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::{Document, Error, Result};

#[derive(Clone, Debug, Default)]
pub struct EditSession {
    events: BTreeMap<u64, EditEvent>,
    undo: Vec<UndoRecord>,
    redo: Vec<UndoRecord>,
    revision: u64,
}

const HISTORY_LIMIT: usize = 256;

/// One undo/redo generation: the inverse steps of a single edit transaction,
/// stored in the order the forward mutations happened. Rolling back applies
/// them in reverse; applying a step yields its own inverse, so undo and redo
/// share one mechanism. A record's size is proportional to what the edit
/// touched — a keystroke records one small step — never to the size of the
/// whole overlay (the previous design cloned the entire overlay per edit).
type UndoRecord = Vec<UndoOp>;

#[derive(Clone, Debug)]
enum UndoOp {
    /// Set `replacement`/`deleted` of the event at `anchor` (inserts kept).
    SetLineState {
        anchor: u64,
        replacement: Option<String>,
        deleted: bool,
    },
    /// Overwrite `inserts[index]` at `anchor` with `text`.
    SetInsert {
        anchor: u64,
        index: usize,
        text: String,
    },
    /// Remove `inserts[index]` at `anchor`.
    RemoveInsert { anchor: u64, index: usize },
    /// Re-insert `text` at `inserts[index]` of `anchor`.
    InsertInsert {
        anchor: u64,
        index: usize,
        text: String,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct EditEvent {
    /// Lines inserted before this original line. The special anchor
    /// `original_line_count` means "append after the original file".
    inserts: Vec<String>,
    replacement: Option<String>,
    deleted: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct EditLine {
    pub number: u64,
    pub text: String,
    pub edited: bool,
    pub inserted: bool,
    pub original_line: Option<u64>,
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

enum LineRef {
    Original(u64),
    Replaced(u64),
    Inserted { anchor: u64, index: usize },
}

impl EditSession {
    pub fn is_dirty(&self) -> bool {
        !self.events.is_empty()
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) -> bool {
        let Some(record) = self.undo.pop() else {
            return false;
        };
        let inverse = self.apply_record(record);
        push_history(&mut self.redo, inverse);
        self.bump();
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(record) = self.redo.pop() else {
            return false;
        };
        let inverse = self.apply_record(record);
        push_history(&mut self.undo, inverse);
        self.bump();
        true
    }

    pub fn clear(&mut self) {
        if !self.events.is_empty() || !self.undo.is_empty() || !self.redo.is_empty() {
            self.events.clear();
            self.undo.clear();
            self.redo.clear();
            self.bump();
        }
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
        for logical in start..end {
            if let Some(line) = self.line(doc, logical) {
                out.push(line);
            }
        }
        out
    }

    pub fn line(&self, doc: &Document, logical: u64) -> Option<EditLine> {
        match self.locate(logical, doc.line_count())? {
            LineRef::Original(orig) => Some(EditLine {
                number: logical,
                text: doc.line(orig)?,
                edited: false,
                inserted: false,
                original_line: Some(orig),
            }),
            LineRef::Replaced(orig) => {
                let text = self.events.get(&orig)?.replacement.clone()?;
                Some(EditLine {
                    number: logical,
                    text,
                    edited: true,
                    inserted: false,
                    original_line: Some(orig),
                })
            }
            LineRef::Inserted { anchor, index } => {
                let text = self.events.get(&anchor)?.inserts.get(index)?.clone();
                Some(EditLine {
                    number: logical,
                    text,
                    edited: true,
                    inserted: true,
                    original_line: None,
                })
            }
        }
    }

    /// Current (overlay-resolved) text of a logical line.
    fn line_text(&self, doc: &Document, logical: u64) -> Result<String> {
        self.line(doc, logical)
            .map(|l| l.text)
            .ok_or_else(|| Error::Unsupported(format!("line {} is out of range", logical + 1)))
    }

    pub fn replace_line(&mut self, doc: &Document, logical: u64, text: String) -> Result<()> {
        let mut record = UndoRecord::new();
        self.replace_line_inner(doc, logical, text, &mut record)?;
        self.finish_change(record);
        Ok(())
    }

    /// Mutation without committing undo history (composed by `replace_range`).
    /// Appends the inverse of every actual state change to `record`.
    fn replace_line_inner(
        &mut self,
        doc: &Document,
        logical: u64,
        text: String,
        record: &mut UndoRecord,
    ) -> Result<()> {
        match self
            .locate(logical, doc.line_count())
            .ok_or_else(|| Error::Unsupported(format!("line {} is out of range", logical + 1)))?
        {
            LineRef::Original(orig) | LineRef::Replaced(orig) => {
                let replacement = if doc.line(orig).as_deref() == Some(text.as_str()) {
                    None
                } else {
                    Some(text)
                };
                let prior = self.events.get(&orig);
                let prior_replacement = prior.and_then(|ev| ev.replacement.clone());
                let prior_deleted = prior.is_some_and(|ev| ev.deleted);
                if prior_replacement == replacement && !prior_deleted {
                    // No state change; the end state is identical either way.
                    return Ok(());
                }
                record.push(UndoOp::SetLineState {
                    anchor: orig,
                    replacement: prior_replacement,
                    deleted: prior_deleted,
                });
                let ev = self.events.entry(orig).or_default();
                ev.replacement = replacement;
                ev.deleted = false;
                self.clean_anchor(orig);
            }
            LineRef::Inserted { anchor, index } => {
                if let Some(line) = self
                    .events
                    .get_mut(&anchor)
                    .and_then(|ev| ev.inserts.get_mut(index))
                {
                    if *line != text {
                        record.push(UndoOp::SetInsert {
                            anchor,
                            index,
                            text: std::mem::replace(line, text),
                        });
                    }
                }
                self.clean_anchor(anchor);
            }
        }
        Ok(())
    }

    /// Insert `text` before logical line `logical`; `logical == total_lines`
    /// appends after the current document.
    pub fn insert_line_before(&mut self, doc: &Document, logical: u64, text: String) -> Result<()> {
        let mut record = UndoRecord::new();
        self.insert_line_before_inner(doc, logical, text, &mut record)?;
        self.finish_change(record);
        Ok(())
    }

    fn insert_line_before_inner(
        &mut self,
        doc: &Document,
        logical: u64,
        text: String,
        record: &mut UndoRecord,
    ) -> Result<()> {
        let total = self.total_lines(doc);
        if logical > total {
            return Err(Error::Unsupported(format!(
                "line {} is beyond end of document",
                logical + 1
            )));
        }
        if logical == total {
            let anchor = doc.line_count();
            let ev = self.events.entry(anchor).or_default();
            record.push(UndoOp::RemoveInsert {
                anchor,
                index: ev.inserts.len(),
            });
            ev.inserts.push(text);
            return Ok(());
        }
        match self.locate(logical, doc.line_count()).unwrap() {
            LineRef::Original(orig) | LineRef::Replaced(orig) => {
                let ev = self.events.entry(orig).or_default();
                record.push(UndoOp::RemoveInsert {
                    anchor: orig,
                    index: ev.inserts.len(),
                });
                ev.inserts.push(text);
            }
            LineRef::Inserted { anchor, index } => {
                record.push(UndoOp::RemoveInsert { anchor, index });
                self.events
                    .entry(anchor)
                    .or_default()
                    .inserts
                    .insert(index, text);
            }
        }
        Ok(())
    }

    pub fn delete_line(&mut self, doc: &Document, logical: u64) -> Result<()> {
        let mut record = UndoRecord::new();
        self.delete_line_inner(doc, logical, &mut record)?;
        self.finish_change(record);
        Ok(())
    }

    fn delete_line_inner(
        &mut self,
        doc: &Document,
        logical: u64,
        record: &mut UndoRecord,
    ) -> Result<()> {
        match self
            .locate(logical, doc.line_count())
            .ok_or_else(|| Error::Unsupported(format!("line {} is out of range", logical + 1)))?
        {
            LineRef::Original(orig) | LineRef::Replaced(orig) => {
                let prior = self.events.get(&orig);
                let prior_replacement = prior.and_then(|ev| ev.replacement.clone());
                let prior_deleted = prior.is_some_and(|ev| ev.deleted);
                if prior_replacement.is_some() || !prior_deleted {
                    record.push(UndoOp::SetLineState {
                        anchor: orig,
                        replacement: prior_replacement,
                        deleted: prior_deleted,
                    });
                }
                let ev = self.events.entry(orig).or_default();
                ev.replacement = None;
                ev.deleted = true;
                self.clean_anchor(orig);
            }
            LineRef::Inserted { anchor, index } => {
                if let Some(ev) = self.events.get_mut(&anchor) {
                    if index < ev.inserts.len() {
                        let removed = ev.inserts.remove(index);
                        record.push(UndoOp::InsertInsert {
                            anchor,
                            index,
                            text: removed,
                        });
                    }
                }
                self.clean_anchor(anchor);
            }
        }
        Ok(())
    }

    /// Replace the logical span (l0,c0)..(l1,c1) with `text` (which may contain
    /// '\n'), as a SINGLE undo unit. Column offsets are Unicode scalar (char)
    /// counts into the decoded line text. Returns the caret (line, col) after
    /// the edit. `replace_range(l,c,l,c,text)` is a plain insert at (l,c).
    pub fn replace_range(
        &mut self,
        doc: &Document,
        l0: u64,
        c0: usize,
        l1: u64,
        c1: usize,
        text: &str,
    ) -> Result<(u64, usize)> {
        let total = self.total_lines(doc);
        // An empty document has no lines at all; the only valid edit is an
        // insertion at the very start, which seeds the first line(s).
        if total == 0 {
            if l0 != 0 || c0 != 0 || l1 != 0 || c1 != 0 {
                return Err(Error::Unsupported(
                    "the document is empty; only an insertion at line 1 is valid".into(),
                ));
            }
            let mut record = UndoRecord::new();
            let parts: Vec<String> = text.split('\n').map(String::from).collect();
            for (k, p) in parts.iter().enumerate() {
                self.insert_line_before_inner(doc, k as u64, p.clone(), &mut record)?;
            }
            self.finish_change(record);
            let last = parts.len() - 1;
            return Ok((last as u64, parts[last].chars().count()));
        }
        if l0 > l1 || l1 >= total {
            return Err(Error::Unsupported(format!(
                "range spans lines {}..{} outside the document",
                l0 + 1,
                l1 + 1
            )));
        }
        let first = self.line_text(doc, l0)?;
        let last = if l1 == l0 {
            first.clone()
        } else {
            self.line_text(doc, l1)?
        };
        let c0 = c0.min(first.chars().count());
        let c1 = c1.min(last.chars().count());
        let head: String = first.chars().take(c0).collect();
        let tail: String = last.chars().skip(c1).collect();

        let mut parts: Vec<String> = text.split('\n').map(String::from).collect();
        let n = parts.len();
        let li = n - 1;
        parts[0] = format!("{head}{}", parts[0]);
        parts[li] = format!("{}{tail}", parts[li]);

        // One undo step: collect every inverse into one record, commit once.
        // Delete the interior lines descending so anchors above the cursor
        // don't shift under us.
        let mut record = UndoRecord::new();
        self.replace_line_inner(doc, l0, parts[0].clone(), &mut record)?;
        for l in ((l0 + 1)..=l1).rev() {
            self.delete_line_inner(doc, l, &mut record)?;
        }
        for (k, p) in parts[1..].iter().enumerate() {
            self.insert_line_before_inner(doc, l0 + 1 + k as u64, p.clone(), &mut record)?;
        }
        self.finish_change(record);

        let caret_line = l0 + (n as u64 - 1);
        let caret_col = parts[li].chars().count() - tail.chars().count();
        Ok((caret_line, caret_col))
    }

    /// Replace the same column span on every line in `[l0, l1]` as one undo
    /// unit. Multi-line `text` is mapped row-by-row, which is what rectangular
    /// paste expects.
    pub fn replace_rect(
        &mut self,
        doc: &Document,
        l0: u64,
        l1: u64,
        c0: usize,
        c1: usize,
        text: &str,
    ) -> Result<(u64, usize)> {
        let total = self.total_lines(doc);
        if total == 0 {
            return Err(Error::Unsupported(
                "rectangular selection is not valid in an empty document".into(),
            ));
        }
        let top = l0.min(l1);
        let bottom = l0.max(l1);
        if bottom >= total {
            return Err(Error::Unsupported(format!(
                "rectangle spans lines {}..{} outside the document",
                top + 1,
                bottom + 1
            )));
        }
        let left = c0.min(c1);
        let right = c0.max(c1);
        let parts: Vec<&str> = text.split('\n').collect();
        let mut record = UndoRecord::new();
        for line in top..=bottom {
            let replacement = if parts.len() == 1 {
                parts[0]
            } else {
                parts.get((line - top) as usize).copied().unwrap_or("")
            };
            let original = self.line_text(doc, line)?;
            let len = original.chars().count();
            let start = left.min(len);
            let end = right.min(len);
            let head: String = original.chars().take(start).collect();
            let tail: String = original.chars().skip(end).collect();
            self.replace_line_inner(doc, line, format!("{head}{replacement}{tail}"), &mut record)?;
        }
        self.finish_change(record);
        let caret_line = if parts.len() == 1 {
            bottom
        } else {
            let last_row = parts.len().saturating_sub(1) as u64;
            top + last_row.min(bottom - top)
        };
        let caret_text = parts
            .get((caret_line - top) as usize)
            .or_else(|| parts.first())
            .copied()
            .unwrap_or("");
        Ok((caret_line, left + caret_text.chars().count()))
    }

    pub fn save_to_path(&self, doc: &Document, target: impl AsRef<Path>) -> Result<SaveResult> {
        self.save_to_path_inner(doc, target.as_ref(), false)
    }

    pub fn save_to_path_overwrite(
        &self,
        doc: &Document,
        target: impl AsRef<Path>,
    ) -> Result<SaveResult> {
        self.save_to_path_inner(doc, target.as_ref(), true)
    }

    fn save_to_path_inner(
        &self,
        doc: &Document,
        target: &Path,
        overwrite: bool,
    ) -> Result<SaveResult> {
        if target.exists() && !overwrite {
            return Err(Error::Unsupported(format!(
                "'{}' already exists; choose another save path",
                target.display()
            )));
        }
        self.write_stream(doc, target, overwrite)?;
        let bytes = std::fs::metadata(target)?.len();
        Ok(SaveResult {
            path: target.to_path_buf(),
            bytes,
            lines: self.total_lines(doc),
        })
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

    fn write_stream(&self, doc: &Document, target: &Path, overwrite: bool) -> Result<()> {
        let tmp = temp_path(target);
        let file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        let mut w = BufWriter::new(file);

        w.write_all(doc.prefix_bytes())?;
        let original_total = doc.line_count();
        // Walk only the (sparse) edited anchors. Every run of untouched
        // original lines between two anchors is one contiguous mmap byte
        // range, copied out with a single write. This keeps saving
        // O(edits × stride + bytes) instead of the previous per-line random
        // access, which cost O(lines × stride).
        let mut next_unwritten = 0u64;
        for (&anchor, ev) in self.events.range(..original_total) {
            copy_original_span(&mut w, doc, next_unwritten, anchor)?;
            next_unwritten = anchor;
            for text in &ev.inserts {
                write_edited_line(&mut w, doc, text, doc.default_terminator())?;
            }
            if ev.deleted {
                next_unwritten = anchor + 1;
                continue;
            }
            if let Some(text) = &ev.replacement {
                let term = doc.line_terminator(anchor).unwrap_or(b"");
                write_edited_line(&mut w, doc, text, term)?;
                next_unwritten = anchor + 1;
            }
            // An event carrying only inserts leaves its anchor line untouched;
            // `next_unwritten` stays at `anchor` so the next contiguous copy
            // starts with that original line.
        }
        copy_original_span(&mut w, doc, next_unwritten, original_total)?;

        if let Some(ev) = self.events.get(&original_total) {
            if !ev.inserts.is_empty()
                && original_total > 0
                && doc
                    .line_terminator(original_total - 1)
                    .unwrap_or(b"")
                    .is_empty()
            {
                w.write_all(doc.default_terminator())?;
            }
            for text in &ev.inserts {
                write_edited_line(&mut w, doc, text, doc.default_terminator())?;
            }
        }

        w.flush()?;
        w.get_ref().sync_all()?;
        drop(w);
        match std::fs::rename(&tmp, target) {
            Ok(()) => Ok(()),
            Err(e) => {
                if overwrite && target.exists() {
                    std::fs::remove_file(target)?;
                    match std::fs::rename(&tmp, target) {
                        Ok(()) => Ok(()),
                        Err(e) => {
                            let _ = std::fs::remove_file(&tmp);
                            Err(Error::Io(e))
                        }
                    }
                } else {
                    let _ = std::fs::remove_file(&tmp);
                    Err(Error::Io(e))
                }
            }
        }
    }

    fn clean_anchor(&mut self, anchor: u64) {
        let should_remove = self
            .events
            .get(&anchor)
            .map(|ev| ev.inserts.is_empty() && ev.replacement.is_none() && !ev.deleted)
            .unwrap_or(false);
        if should_remove {
            self.events.remove(&anchor);
        }
    }

    fn bump(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    fn finish_change(&mut self, record: UndoRecord) {
        if record.is_empty() {
            return;
        }
        push_history(&mut self.undo, record);
        self.redo.clear();
        self.bump();
    }

    /// Apply a record's steps in reverse order (unwinding one transaction) and
    /// return the record that plays the transaction back the other way.
    fn apply_record(&mut self, record: UndoRecord) -> UndoRecord {
        let mut inverse = Vec::with_capacity(record.len());
        for op in record.into_iter().rev() {
            inverse.push(self.apply_op(op));
        }
        inverse
    }

    /// Apply one inverse step and return the step that inverts it again.
    /// Records are only replayed against the exact overlay state they were
    /// recorded from, so the referenced anchors/indices always exist; the
    /// fallbacks below merely keep this total instead of panicking.
    fn apply_op(&mut self, op: UndoOp) -> UndoOp {
        match op {
            UndoOp::SetLineState {
                anchor,
                replacement,
                deleted,
            } => {
                let ev = self.events.entry(anchor).or_default();
                let inverse = UndoOp::SetLineState {
                    anchor,
                    replacement: std::mem::replace(&mut ev.replacement, replacement),
                    deleted: std::mem::replace(&mut ev.deleted, deleted),
                };
                self.clean_anchor(anchor);
                inverse
            }
            UndoOp::SetInsert {
                anchor,
                index,
                text,
            } => match self
                .events
                .get_mut(&anchor)
                .and_then(|ev| ev.inserts.get_mut(index))
            {
                Some(line) => UndoOp::SetInsert {
                    anchor,
                    index,
                    text: std::mem::replace(line, text),
                },
                None => UndoOp::SetInsert {
                    anchor,
                    index,
                    text,
                },
            },
            UndoOp::RemoveInsert { anchor, index } => {
                let removed = self
                    .events
                    .get_mut(&anchor)
                    .filter(|ev| index < ev.inserts.len())
                    .map(|ev| ev.inserts.remove(index));
                self.clean_anchor(anchor);
                match removed {
                    Some(text) => UndoOp::InsertInsert {
                        anchor,
                        index,
                        text,
                    },
                    None => UndoOp::RemoveInsert { anchor, index },
                }
            }
            UndoOp::InsertInsert {
                anchor,
                index,
                text,
            } => {
                let ev = self.events.entry(anchor).or_default();
                let index = index.min(ev.inserts.len());
                ev.inserts.insert(index, text);
                UndoOp::RemoveInsert { anchor, index }
            }
        }
    }
}

fn push_history(stack: &mut Vec<UndoRecord>, record: UndoRecord) {
    if stack.len() == HISTORY_LIMIT {
        stack.remove(0);
    }
    stack.push(record);
}

/// Copy original lines `[start, end)` (with their terminators) as one
/// contiguous byte range out of the mmap.
fn copy_original_span(mut w: impl Write, doc: &Document, start: u64, end: u64) -> Result<()> {
    if start >= end {
        return Ok(());
    }
    let bytes = doc.raw_lines_span(start, end).ok_or_else(|| {
        Error::Unsupported(format!(
            "original lines {}..{} are out of range while saving",
            start + 1,
            end
        ))
    })?;
    w.write_all(bytes)?;
    Ok(())
}

fn write_edited_line(
    mut w: impl Write,
    doc: &Document,
    text: &str,
    terminator: &[u8],
) -> Result<()> {
    let bytes = doc.encoding().encode_text(text).ok_or_else(|| {
        Error::Unsupported(format!(
            "edited text cannot be encoded as {}",
            doc.encoding().label()
        ))
    })?;
    w.write_all(&bytes)?;
    w.write_all(terminator)?;
    Ok(())
}

fn temp_path(target: &Path) -> PathBuf {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let name = target
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("ayame-save");
    parent.join(format!(
        ".{name}.ayame-tmp-{}-{}",
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

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::*;
    use crate::Encoding;
    use crate::OpenOptions as AyameOpenOptions;

    fn doc_from(bytes: &[u8]) -> (NamedTempFile, Document) {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(bytes).unwrap();
        let doc = Document::open(f.path(), &AyameOpenOptions::default()).unwrap();
        (f, doc)
    }

    fn doc_from_with_options(bytes: &[u8], opts: AyameOpenOptions) -> (NamedTempFile, Document) {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(bytes).unwrap();
        let doc = Document::open(f.path(), &opts).unwrap();
        (f, doc)
    }

    #[test]
    fn line_overlay_replaces_inserts_and_deletes() {
        let (_f, doc) = doc_from(b"a\nb\nc\n");
        let mut edits = EditSession::default();
        edits.replace_line(&doc, 1, "B".into()).unwrap();
        edits.insert_line_before(&doc, 2, "x".into()).unwrap();
        edits.delete_line(&doc, 0).unwrap();
        let lines: Vec<_> = edits
            .lines(&doc, 0, 10)
            .into_iter()
            .map(|l| l.text)
            .collect();
        assert_eq!(lines, vec!["B", "x", "c"]);
        let st = edits.stats(&doc);
        assert_eq!(st.total_lines, 3);
        assert_eq!(st.replaced_lines, 1);
        assert_eq!(st.inserted_lines, 1);
        assert_eq!(st.deleted_lines, 1);
    }

    #[test]
    fn save_stream_preserves_untouched_bytes_and_crlf() {
        let (f, doc) = doc_from(b"a\r\nb\r\nc");
        let out = f.path().with_extension("out");
        let mut edits = EditSession::default();
        edits.replace_line(&doc, 1, "B".into()).unwrap();
        edits.insert_line_before(&doc, 3, "d".into()).unwrap();
        edits.save_to_path(&doc, &out).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"a\r\nB\r\nc\r\nd\r\n");
        let _ = std::fs::remove_file(out);
    }

    #[test]
    fn save_to_path_overwrite_replaces_existing_file() {
        let (_f, doc) = doc_from(b"alpha\nbeta\n");
        let mut out = NamedTempFile::new().unwrap();
        out.write_all(b"old\n").unwrap();
        let mut edits = EditSession::default();
        edits.replace_line(&doc, 1, "BETA".into()).unwrap();
        let res = edits.save_to_path_overwrite(&doc, out.path()).unwrap();
        assert_eq!(res.lines, 2);
        assert_eq!(std::fs::read(out.path()).unwrap(), b"alpha\nBETA\n");
    }

    #[test]
    fn replacing_with_original_text_clears_dirty_state() {
        let (f, doc) = doc_from(b"a\nb\n");
        let out = f.path().with_extension("same");
        let mut edits = EditSession::default();
        edits.replace_line(&doc, 1, "B".into()).unwrap();
        assert!(edits.is_dirty());
        edits.replace_line(&doc, 1, "b".into()).unwrap();
        assert!(!edits.is_dirty());
        edits.save_to_path(&doc, &out).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"a\nb\n");
        let _ = std::fs::remove_file(out);
    }

    #[test]
    fn undo_redo_restore_sparse_overlay() {
        let (_f, doc) = doc_from(b"a\nb\nc\n");
        let mut edits = EditSession::default();
        edits.replace_line(&doc, 1, "B".into()).unwrap();
        edits.insert_line_before(&doc, 2, "x".into()).unwrap();

        let texts = |edits: &EditSession| -> Vec<String> {
            edits
                .lines(&doc, 0, 10)
                .into_iter()
                .map(|l| l.text)
                .collect()
        };

        assert_eq!(texts(&edits), vec!["a", "B", "x", "c"]);
        assert!(edits.can_undo());
        assert!(!edits.can_redo());

        assert!(edits.undo());
        assert_eq!(texts(&edits), vec!["a", "B", "c"]);
        assert!(edits.can_redo());

        assert!(edits.redo());
        assert_eq!(texts(&edits), vec!["a", "B", "x", "c"]);
    }

    #[test]
    fn new_edit_after_undo_discards_redo() {
        let (_f, doc) = doc_from(b"a\nb\nc\n");
        let mut edits = EditSession::default();
        edits.replace_line(&doc, 1, "B".into()).unwrap();
        edits.insert_line_before(&doc, 2, "x".into()).unwrap();
        assert!(edits.undo());
        assert!(edits.can_redo());

        edits.replace_line(&doc, 1, "bee".into()).unwrap();
        assert!(!edits.can_redo());
        let lines: Vec<_> = edits
            .lines(&doc, 0, 10)
            .into_iter()
            .map(|l| l.text)
            .collect();
        assert_eq!(lines, vec!["a", "bee", "c"]);
    }

    #[test]
    fn empty_file_can_be_edited_and_saved() {
        let (f, doc) = doc_from(b"");
        let out = f.path().with_extension("inserted");
        let mut edits = EditSession::default();
        edits.insert_line_before(&doc, 0, "alpha".into()).unwrap();
        edits.insert_line_before(&doc, 1, "beta".into()).unwrap();
        edits.save_to_path(&doc, &out).unwrap();
        assert_eq!(edits.stats(&doc).total_lines, 2);
        assert_eq!(std::fs::read(&out).unwrap(), b"alpha\nbeta\n");
        let _ = std::fs::remove_file(out);
    }

    fn texts(edits: &EditSession, doc: &Document) -> Vec<String> {
        edits
            .lines(doc, 0, 100)
            .into_iter()
            .map(|l| l.text)
            .collect()
    }

    #[test]
    fn replace_range_typing_within_a_line_inserts_and_moves_caret() {
        let (_f, doc) = doc_from(b"hello\nworld\n");
        let mut edits = EditSession::default();
        // Type "XY" between 'he' and 'llo' on line 0.
        let caret = edits.replace_range(&doc, 0, 2, 0, 2, "XY").unwrap();
        assert_eq!(texts(&edits, &doc), vec!["heXYllo", "world"]);
        assert_eq!(caret, (0, 4));
    }

    #[test]
    fn replace_range_enter_splits_a_line_into_two() {
        let (_f, doc) = doc_from(b"hello\n");
        let mut edits = EditSession::default();
        // Press Enter after "he": split into "he" and "llo".
        let caret = edits.replace_range(&doc, 0, 2, 0, 2, "\n").unwrap();
        assert_eq!(texts(&edits, &doc), vec!["he", "llo"]);
        assert_eq!(caret, (1, 0));
        assert_eq!(edits.stats(&doc).total_lines, 2);
    }

    #[test]
    fn replace_range_backspace_merges_two_lines() {
        let (_f, doc) = doc_from(b"foo\nbar\n");
        let mut edits = EditSession::default();
        // Backspace at start of line 1 joins it onto the end of line 0.
        let caret = edits.replace_range(&doc, 0, 3, 1, 0, "").unwrap();
        assert_eq!(texts(&edits, &doc), vec!["foobar"]);
        assert_eq!(caret, (0, 3));
        assert_eq!(edits.stats(&doc).total_lines, 1);
    }

    #[test]
    fn replace_range_multi_line_selection_replaced_with_multi_line_text() {
        let (_f, doc) = doc_from(b"aaa\nbbb\nccc\nddd\n");
        let mut edits = EditSession::default();
        // Select from (0,1) through (2,1) and replace with "X\nY\nZ".
        let caret = edits.replace_range(&doc, 0, 1, 2, 1, "X\nY\nZ").unwrap();
        assert_eq!(texts(&edits, &doc), vec!["aX", "Y", "Zcc", "ddd"]);
        assert_eq!(caret, (2, 1));
    }

    #[test]
    fn replace_range_is_a_single_undo_unit() {
        let (_f, doc) = doc_from(b"aaa\nbbb\nccc\n");
        let mut edits = EditSession::default();
        edits.replace_range(&doc, 0, 1, 2, 1, "X\nY\nZ").unwrap();
        assert_eq!(texts(&edits, &doc), vec!["aX", "Y", "Zcc"]);
        assert!(edits.undo());
        assert_eq!(texts(&edits, &doc), vec!["aaa", "bbb", "ccc"]);
        assert!(edits.redo());
        assert_eq!(texts(&edits, &doc), vec!["aX", "Y", "Zcc"]);
    }

    #[test]
    fn replace_rect_is_a_single_undo_unit() {
        let (_f, doc) = doc_from(b"abcd\nefgh\nijkl\n");
        let mut edits = EditSession::default();
        let caret = edits.replace_rect(&doc, 0, 2, 1, 3, "X").unwrap();
        assert_eq!(caret, (2, 2));
        assert_eq!(texts(&edits, &doc), vec!["aXd", "eXh", "iXl"]);
        assert!(edits.undo());
        assert_eq!(texts(&edits, &doc), vec!["abcd", "efgh", "ijkl"]);
        assert!(edits.redo());
        assert_eq!(texts(&edits, &doc), vec!["aXd", "eXh", "iXl"]);
    }

    #[test]
    fn replace_rect_maps_multiline_text_by_row() {
        let (_f, doc) = doc_from(b"abcd\nefgh\nijkl\n");
        let mut edits = EditSession::default();
        let caret = edits.replace_rect(&doc, 0, 2, 1, 3, "X\nYY\nZ").unwrap();
        assert_eq!(caret, (2, 2));
        assert_eq!(texts(&edits, &doc), vec!["aXd", "eYYh", "iZl"]);
        assert!(edits.undo());
        assert_eq!(texts(&edits, &doc), vec!["abcd", "efgh", "ijkl"]);
    }

    #[test]
    fn replace_range_paste_multiline_into_a_caret() {
        let (_f, doc) = doc_from(b"start\nend\n");
        let mut edits = EditSession::default();
        // Paste "one\ntwo\nthree" at (0,5) — the end of line 0.
        let caret = edits
            .replace_range(&doc, 0, 5, 0, 5, "one\ntwo\nthree")
            .unwrap();
        assert_eq!(texts(&edits, &doc), vec!["startone", "two", "three", "end"]);
        assert_eq!(caret, (2, 5));
    }

    #[test]
    fn replace_range_counts_unicode_scalars_not_bytes() {
        let (_f, doc) = doc_from("あいう\nかきく\n".as_bytes());
        let mut edits = EditSession::default();
        // Replace the middle char of line 0 ('い', chars 1..2) with "X".
        let caret = edits.replace_range(&doc, 0, 1, 0, 2, "X").unwrap();
        assert_eq!(texts(&edits, &doc), vec!["あXう", "かきく"]);
        assert_eq!(caret, (0, 2));
    }

    #[test]
    fn replace_range_clamps_columns_past_line_end() {
        let (_f, doc) = doc_from(b"hi\n");
        let mut edits = EditSession::default();
        // Columns beyond the line length clamp to the end.
        let caret = edits.replace_range(&doc, 0, 99, 0, 99, "!").unwrap();
        assert_eq!(texts(&edits, &doc), vec!["hi!"]);
        assert_eq!(caret, (0, 3));
    }

    #[test]
    fn replace_range_rejects_out_of_range_lines() {
        let (_f, doc) = doc_from(b"a\nb\n");
        let mut edits = EditSession::default();
        assert!(edits.replace_range(&doc, 1, 0, 5, 0, "x").is_err());
        assert!(edits.replace_range(&doc, 3, 0, 3, 0, "x").is_err());
    }

    #[test]
    fn replace_range_can_seed_an_empty_document() {
        let (_f, doc) = doc_from(b"");
        assert_eq!(doc.line_count(), 0);
        let mut edits = EditSession::default();
        // Typing into a 0-line file inserts the first line.
        let caret = edits.replace_range(&doc, 0, 0, 0, 0, "hi").unwrap();
        assert_eq!(texts(&edits, &doc), vec!["hi"]);
        assert_eq!(caret, (0, 2));
        // A multi-line paste into an empty doc seeds several lines.
        let (_f2, doc2) = doc_from(b"");
        let mut e2 = EditSession::default();
        let caret2 = e2.replace_range(&doc2, 0, 0, 0, 0, "one\ntwo").unwrap();
        assert_eq!(texts(&e2, &doc2), vec!["one", "two"]);
        assert_eq!(caret2, (1, 3));
        // Any non-origin range on an empty doc is rejected.
        let (_f3, doc3) = doc_from(b"");
        let mut e3 = EditSession::default();
        assert!(e3.replace_range(&doc3, 0, 0, 1, 0, "x").is_err());
    }

    #[test]
    fn save_copies_untouched_runs_between_sparse_edits() {
        // Large enough for real gaps between edits; CRLF everywhere and a
        // final line without a terminator.
        let mut data = Vec::new();
        for i in 0..500u64 {
            data.extend_from_slice(format!("line {i}\r\n").as_bytes());
        }
        data.extend_from_slice(b"tail-no-eol");
        let (f, doc) = doc_from(&data);
        let out = f.path().with_extension("sparse");
        let mut edits = EditSession::default();
        // Ordered so each logical line still equals its original line number.
        edits
            .insert_line_before(&doc, 200, "INSERTED".into())
            .unwrap();
        edits.replace_line(&doc, 10, "TEN".into()).unwrap();
        edits.delete_line(&doc, 100).unwrap();
        edits.save_to_path(&doc, &out).unwrap();

        let mut expect = Vec::new();
        for i in 0..500u64 {
            if i == 100 {
                continue;
            }
            if i == 200 {
                expect.extend_from_slice(b"INSERTED\r\n");
            }
            if i == 10 {
                expect.extend_from_slice(b"TEN\r\n");
            } else {
                expect.extend_from_slice(format!("line {i}\r\n").as_bytes());
            }
        }
        expect.extend_from_slice(b"tail-no-eol");
        assert_eq!(std::fs::read(&out).unwrap(), expect);
        let _ = std::fs::remove_file(out);
    }

    #[test]
    fn undo_redo_walk_multi_step_history() {
        let (_f, doc) = doc_from(b"a\nb\n");
        let mut edits = EditSession::default();
        edits.insert_line_before(&doc, 1, "x".into()).unwrap(); // a x b
        edits.replace_line(&doc, 1, "y".into()).unwrap(); // a y b (edits the inserted line)
        edits.delete_line(&doc, 1).unwrap(); // a b
        edits.delete_line(&doc, 0).unwrap(); // b
        assert_eq!(texts(&edits, &doc), vec!["b"]);
        assert!(edits.undo());
        assert_eq!(texts(&edits, &doc), vec!["a", "b"]);
        assert!(edits.undo());
        assert_eq!(texts(&edits, &doc), vec!["a", "y", "b"]);
        assert!(edits.undo());
        assert_eq!(texts(&edits, &doc), vec!["a", "x", "b"]);
        assert!(edits.undo());
        assert_eq!(texts(&edits, &doc), vec!["a", "b"]);
        assert!(!edits.is_dirty());
        assert!(!edits.can_undo());
        // Walk the whole history forward again.
        assert!(edits.redo());
        assert!(edits.redo());
        assert!(edits.redo());
        assert!(edits.redo());
        assert_eq!(texts(&edits, &doc), vec!["b"]);
        assert!(!edits.can_redo());
    }

    #[test]
    fn history_limit_keeps_last_256_generations() {
        let (_f, doc) = doc_from(b"seed\n");
        let mut edits = EditSession::default();
        for k in 0..300u32 {
            edits.replace_line(&doc, 0, format!("v{k}")).unwrap();
        }
        let mut undone = 0;
        while edits.undo() {
            undone += 1;
        }
        assert_eq!(undone, 256);
        // 300 edits with 256 undoable generations lands on the state after
        // edit #43 (0-based), exactly like the old snapshot history did.
        assert_eq!(texts(&edits, &doc), vec!["v43"]);
        assert!(edits.is_dirty());
    }

    #[test]
    fn undo_after_paste_and_edits_inside_pasted_block() {
        let (_f, doc) = doc_from(b"top\nbottom\n");
        let mut edits = EditSession::default();
        // Paste three lines between top and bottom.
        edits
            .replace_range(&doc, 0, 3, 0, 3, "\np1\np2\np3")
            .unwrap();
        assert_eq!(texts(&edits, &doc), vec!["top", "p1", "p2", "p3", "bottom"]);
        // Type into the middle pasted line.
        edits.replace_range(&doc, 2, 2, 2, 2, "X").unwrap();
        assert_eq!(
            texts(&edits, &doc),
            vec!["top", "p1", "p2X", "p3", "bottom"]
        );
        assert!(edits.undo());
        assert_eq!(texts(&edits, &doc), vec!["top", "p1", "p2", "p3", "bottom"]);
        assert!(edits.undo());
        assert_eq!(texts(&edits, &doc), vec!["top", "bottom"]);
        assert!(edits.redo());
        assert!(edits.redo());
        assert_eq!(
            texts(&edits, &doc),
            vec!["top", "p1", "p2X", "p3", "bottom"]
        );
    }

    #[test]
    fn shift_jis_edits_are_encoded_while_untouched_bytes_are_preserved() {
        let opts = AyameOpenOptions {
            encoding: Some(Encoding::ShiftJis),
            ..AyameOpenOptions::default()
        };
        let (f, doc) = doc_from_with_options(b"\x82\xa0\r\nraw\xff\r\n", opts);
        let out = f.path().with_extension("sjis");
        let mut edits = EditSession::default();
        edits.replace_line(&doc, 0, "い".into()).unwrap();
        edits.save_to_path(&doc, &out).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"\x82\xa2\r\nraw\xff\r\n");
        let _ = std::fs::remove_file(out);
    }
}
