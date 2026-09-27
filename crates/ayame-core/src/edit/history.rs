//! Undo/redo history for the sparse edit overlay.
//!
//! A generation is a sparse list of inverse steps; undo and redo share one
//! mechanism, so walking history never clones the whole overlay.

use super::*;

pub(crate) const HISTORY_LIMIT: usize = 256;

/// One undo/redo generation: the inverse steps of a single edit transaction,
/// stored in the order the forward mutations happened. Rolling back applies
/// them in reverse; applying a step yields its own inverse, so undo and redo
/// share one mechanism. A record's size is proportional to what the edit
/// touched — a keystroke records one small step — never to the size of the
/// whole overlay (the previous design cloned the entire overlay per edit).
pub(super) type UndoRecord = Vec<UndoOp>;

/// A history stack entry: one undo/redo generation plus the content
/// generation of the state the entry returns to when applied. Undo and redo
/// restore `EditSession::content_gen` from here, which is what lets dirtiness
/// (content vs. last save) survive undo/redo round-trips across a save.
#[derive(Clone, Debug)]
pub(super) struct HistoryEntry {
    ops: UndoRecord,
    gen: u64,
}

#[derive(Clone, Debug)]
pub(super) enum UndoOp {
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

impl EditSession {
    /// Reconstruct a retained content generation without disturbing the live
    /// session. History is capped at 256 entries, and each replay touches only
    /// one sparse undo record. `mark_saved_from` avoids this walk whenever the
    /// caller still owns the save snapshot.
    pub(super) fn events_at_generation(&self, gen: u64) -> Option<BTreeMap<u64, EditEvent>> {
        if self.content_gen == gen {
            return Some(self.events.clone());
        }
        let mut older = self.clone();
        while older.undo() {
            if older.content_gen == gen {
                return Some(older.events);
            }
        }
        let mut newer = self.clone();
        while newer.redo() {
            if newer.content_gen == gen {
                return Some(newer.events);
            }
        }
        None
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) -> bool {
        let Some(entry) = self.undo.pop() else {
            return false;
        };
        let inverse = self.apply_record(entry.ops);
        push_history(
            &mut self.redo,
            HistoryEntry {
                ops: inverse,
                gen: self.content_gen,
            },
        );
        self.content_gen = entry.gen;
        self.bump();
        self.wal_commit(|| LoggedOp::Undo);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(entry) = self.redo.pop() else {
            return false;
        };
        let inverse = self.apply_record(entry.ops);
        push_history(
            &mut self.undo,
            HistoryEntry {
                ops: inverse,
                gen: self.content_gen,
            },
        );
        self.content_gen = entry.gen;
        self.bump();
        self.wal_commit(|| LoggedOp::Redo);
        true
    }

    /// Discard the overlay and the whole history: the content returns to the
    /// document as opened (generation 0). `saved_gen` is deliberately kept —
    /// if a save has happened since open, the disk holds that saved content,
    /// so a cleared session correctly reads dirty until saved again.
    ///
    /// A revert is NOT mirrored into an attached crash log: like a save, the
    /// caller must [`WalWriter::reset`] the log so its old records never
    /// replay onto content they no longer describe.
    pub fn clear(&mut self) {
        if !self.events.is_empty()
            || !self.undo.is_empty()
            || !self.redo.is_empty()
            || self.content_gen != 0
        {
            self.events.clear();
            self.undo.clear();
            self.redo.clear();
            self.content_gen = 0;
            self.bump();
        }
    }
    pub(super) fn clean_anchor(&mut self, anchor: u64) {
        let should_remove = self
            .events
            .get(&anchor)
            .map(|ev| ev.inserts.is_empty() && ev.replacement.is_none() && !ev.deleted)
            .unwrap_or(false);
        if should_remove {
            self.events.remove(&anchor);
        }
    }

    pub(super) fn bump(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    /// Commit `record` as one undo generation. Returns whether anything
    /// actually changed: an empty record is the shared no-op detection for
    /// every public mutator — nothing is pushed, no generation moves, and the
    /// caller must not mirror the op into the crash log either.
    pub(super) fn finish_change(&mut self, record: UndoRecord) -> bool {
        if record.is_empty() {
            return false;
        }
        push_history(
            &mut self.undo,
            HistoryEntry {
                ops: record,
                gen: self.content_gen,
            },
        );
        self.redo.clear();
        self.content_gen = self.next_gen;
        self.next_gen += 1;
        self.bump();
        true
    }
    /// Apply a record's steps in reverse order (unwinding one transaction) and
    /// return the record that plays the transaction back the other way.
    pub(super) fn apply_record(&mut self, record: UndoRecord) -> UndoRecord {
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

fn push_history(stack: &mut Vec<HistoryEntry>, entry: HistoryEntry) {
    if stack.len() == HISTORY_LIMIT {
        stack.remove(0);
    }
    stack.push(entry);
}
