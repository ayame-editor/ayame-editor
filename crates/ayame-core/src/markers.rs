//! Sparse, line-oriented document markers.
//!
//! A marker set stores only lines that actually carry a marker. It therefore
//! stays constant-size for an unmarked document and scales with `M` (the
//! number of markers), never with the document's line count. This matters for
//! Ayame's ten-billion-line design floor.
//!
//! The set deliberately knows nothing about UI or search. Bookmarks, saved
//! search rules, and change-history markers share the same ordered primitive;
//! the server layers session lifetime and edit-history coordination on top.

use std::collections::BTreeSet;
use std::ops::Range;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// Hard safety limit for markers of one kind in one document session.
///
/// This is an API admission limit, not a line-count-derived allocation. An
/// internal undo may temporarily restore markers beyond the cap rather than
/// silently losing user state, but ordinary insertion refuses at the limit.
pub const MAX_MARKERS_PER_KIND: usize = 1_000_000;

/// Marker categories supported by the common sparse store.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MarkerKind {
    Bookmark,
    SearchRule,
    ChangeSaved,
    ChangeUnsaved,
}

impl MarkerKind {
    pub const ALL: [MarkerKind; 4] = [
        MarkerKind::Bookmark,
        MarkerKind::SearchRule,
        MarkerKind::ChangeSaved,
        MarkerKind::ChangeUnsaved,
    ];

    const fn index(self) -> usize {
        match self {
            MarkerKind::Bookmark => 0,
            MarkerKind::SearchRule => 1,
            MarkerKind::ChangeSaved => 2,
            MarkerKind::ChangeUnsaved => 3,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            MarkerKind::Bookmark => "bookmark",
            MarkerKind::SearchRule => "search-rule",
            MarkerKind::ChangeSaved => "change-saved",
            MarkerKind::ChangeUnsaved => "change-unsaved",
        }
    }
}

impl FromStr for MarkerKind {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "bookmark" => Ok(MarkerKind::Bookmark),
            "search-rule" => Ok(MarkerKind::SearchRule),
            "change-saved" => Ok(MarkerKind::ChangeSaved),
            "change-unsaved" => Ok(MarkerKind::ChangeUnsaved),
            other => Err(Error::InvalidInput(format!(
                "unknown marker kind '{other}'"
            ))),
        }
    }
}

/// One marker in a [`MarkerSet`].
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LineMarker {
    pub kind: MarkerKind,
    pub line: u64,
}

/// A line replacement used to keep markers aligned with edited content.
///
/// Semantics intentionally match line-oriented editors:
///
/// * insertion (`old_lines == 0`) shifts existing lines at `start`;
/// * deletion (`new_lines == 0`) removes markers in the deleted span;
/// * replacement preserves a marker on the first line, removes markers on the
///   replaced continuation lines, and shifts the untouched suffix.
///
/// Text-only edits use `old_lines == new_lines == 1` and are a no-op here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LineEdit {
    pub start: u64,
    pub old_lines: u64,
    pub new_lines: u64,
}

impl LineEdit {
    #[must_use]
    pub const fn replacement(start: u64, old_lines: u64, new_lines: u64) -> Self {
        Self {
            start,
            old_lines,
            new_lines,
        }
    }

    #[must_use]
    pub const fn inverse(self) -> Self {
        Self {
            start: self.start,
            old_lines: self.new_lines,
            new_lines: self.old_lines,
        }
    }
}

/// Reversible marker-coordinate operation.
///
/// `restore` contains markers removed by the opposite direction. Keeping
/// only those affected entries makes history proportional to changed markers,
/// never to the document line count or the whole marker set.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkerTransform {
    pub edit: LineEdit,
    pub restore: Vec<LineMarker>,
}

impl MarkerTransform {
    #[must_use]
    pub const fn new(edit: LineEdit) -> Self {
        Self {
            edit,
            restore: Vec::new(),
        }
    }
}

/// Sparse, ordered markers partitioned by marker kind.
#[derive(Clone, Debug, Default)]
pub struct MarkerSet {
    lines: [BTreeSet<u64>; 4],
}

impl MarkerSet {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lines.iter().all(BTreeSet::is_empty)
    }

    #[must_use]
    pub fn len(&self, kind: MarkerKind) -> usize {
        self.lines[kind.index()].len()
    }

    #[must_use]
    pub fn total_len(&self) -> usize {
        self.lines.iter().map(BTreeSet::len).sum()
    }

    #[must_use]
    pub fn contains(&self, kind: MarkerKind, line: u64) -> bool {
        self.lines[kind.index()].contains(&line)
    }

    /// Insert a marker, enforcing the per-kind admission cap.
    ///
    /// Returns `true` only when the set changed.
    pub fn insert(&mut self, kind: MarkerKind, line: u64) -> Result<bool> {
        let set = &mut self.lines[kind.index()];
        if set.contains(&line) {
            return Ok(false);
        }
        if set.len() >= MAX_MARKERS_PER_KIND {
            return Err(Error::InvalidInput(format!(
                "{} marker limit reached ({MAX_MARKERS_PER_KIND})",
                kind.as_str()
            )));
        }
        set.insert(line);
        Ok(true)
    }

    /// Toggle a marker and return its new marked state.
    pub fn toggle(&mut self, kind: MarkerKind, line: u64) -> Result<bool> {
        if self.remove(kind, line) {
            Ok(false)
        } else {
            self.insert(kind, line)?;
            Ok(true)
        }
    }

    pub fn remove(&mut self, kind: MarkerKind, line: u64) -> bool {
        self.lines[kind.index()].remove(&line)
    }

    pub fn clear(&mut self, kind: MarkerKind) {
        self.lines[kind.index()].clear();
    }

    /// First marker at or after `line`, in `O(log M)`.
    #[must_use]
    pub fn next_at_or_after(&self, kind: MarkerKind, line: u64) -> Option<u64> {
        self.lines[kind.index()].range(line..).next().copied()
    }

    /// Last marker at or before `line`, in `O(log M)`.
    #[must_use]
    pub fn previous_at_or_before(&self, kind: MarkerKind, line: u64) -> Option<u64> {
        self.lines[kind.index()].range(..=line).next_back().copied()
    }

    #[must_use]
    pub fn first(&self, kind: MarkerKind) -> Option<u64> {
        self.lines[kind.index()].first().copied()
    }

    #[must_use]
    pub fn last(&self, kind: MarkerKind) -> Option<u64> {
        self.lines[kind.index()].last().copied()
    }

    /// Enumerate at most `limit` markers in `[start, end)`.
    ///
    /// The iterator begins with one tree lookup and visits only returned
    /// entries: `O(log M + K)`.
    #[must_use]
    pub fn range(&self, kind: MarkerKind, start: u64, end: u64, limit: usize) -> Vec<u64> {
        if start >= end || limit == 0 {
            return Vec::new();
        }
        self.lines[kind.index()]
            .range(start..end)
            .take(limit)
            .copied()
            .collect()
    }

    /// Enumerate at most `limit` markers at or after `start`.
    #[must_use]
    pub fn from(&self, kind: MarkerKind, start: u64, limit: usize) -> Vec<u64> {
        if limit == 0 {
            return Vec::new();
        }
        self.lines[kind.index()]
            .range(start..)
            .take(limit)
            .copied()
            .collect()
    }

    /// All marker kinds present in `[start, end)`, ordered by kind then line.
    #[must_use]
    pub fn all_in_range(&self, start: u64, end: u64, limit_per_kind: usize) -> Vec<LineMarker> {
        let mut out = Vec::new();
        for kind in MarkerKind::ALL {
            out.extend(
                self.range(kind, start, end, limit_per_kind)
                    .into_iter()
                    .map(|line| LineMarker { kind, line }),
            );
        }
        out
    }

    /// Apply one line-coordinate transform and return its inverse.
    ///
    /// `restore` is applied after shifting into the target coordinate space.
    /// It bypasses the admission cap because undo must not silently discard a
    /// marker that existed before the edit.
    pub fn apply(&mut self, transform: MarkerTransform) -> Result<MarkerTransform> {
        let edit = transform.edit;
        let old_end = edit
            .start
            .checked_add(edit.old_lines)
            .ok_or_else(|| Error::InvalidInput("marker line edit overflow".into()))?;
        let new_end = edit
            .start
            .checked_add(edit.new_lines)
            .ok_or_else(|| Error::InvalidInput("marker line edit overflow".into()))?;

        // Preflight growth before removing or moving anything. A rejected
        // transform must leave the set byte-for-byte equivalent so the caller
        // can reject the matching content edit transactionally.
        if let Some(growth) = new_end.checked_sub(old_end).filter(|growth| *growth != 0) {
            for kind in MarkerKind::ALL {
                if self
                    .last(kind)
                    .filter(|line| *line >= old_end)
                    .is_some_and(|line| line.checked_add(growth).is_none())
                {
                    return Err(Error::InvalidInput("marker line shift overflow".into()));
                }
            }
        }

        // A replacement preserves the marker on its first logical line.
        let removed_start = edit.start + u64::from(edit.old_lines > 0 && edit.new_lines > 0);
        let removed = self.take_range(removed_start..old_end);

        for kind in MarkerKind::ALL {
            self.shift_suffix(kind, old_end, new_end)?;
        }
        for marker in transform.restore {
            self.lines[marker.kind.index()].insert(marker.line);
        }

        Ok(MarkerTransform {
            edit: edit.inverse(),
            restore: removed,
        })
    }

    fn take_range(&mut self, range: Range<u64>) -> Vec<LineMarker> {
        if range.start >= range.end {
            return Vec::new();
        }
        let mut removed = Vec::new();
        for kind in MarkerKind::ALL {
            let lines: Vec<u64> = self.lines[kind.index()]
                .range(range.clone())
                .copied()
                .collect();
            for line in lines {
                self.lines[kind.index()].remove(&line);
                removed.push(LineMarker { kind, line });
            }
        }
        removed
    }

    /// Move every marker `>= old_end` so that `old_end` maps to `new_end`.
    fn shift_suffix(&mut self, kind: MarkerKind, old_end: u64, new_end: u64) -> Result<()> {
        if old_end == new_end {
            return Ok(());
        }
        let moved = self.lines[kind.index()].split_off(&old_end);
        for line in moved {
            let shifted = if new_end >= old_end {
                line.checked_add(new_end - old_end)
            } else {
                line.checked_sub(old_end - new_end)
            }
            .ok_or_else(|| Error::InvalidInput("marker line shift overflow".into()))?;
            self.lines[kind.index()].insert(shifted);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparse_queries_work_at_ten_billion_lines() {
        let mut markers = MarkerSet::default();
        assert!(markers.is_empty());
        assert_eq!(markers.total_len(), 0);

        for line in [3, 9_999_999_999, 4_000_000_000] {
            markers.insert(MarkerKind::Bookmark, line).unwrap();
        }
        assert_eq!(markers.len(MarkerKind::Bookmark), 3);
        assert_eq!(
            markers.next_at_or_after(MarkerKind::Bookmark, 4),
            Some(4_000_000_000)
        );
        assert_eq!(
            markers.previous_at_or_before(MarkerKind::Bookmark, 9_000_000_000),
            Some(4_000_000_000)
        );
        assert_eq!(
            markers.range(MarkerKind::Bookmark, 0, 10_000_000_000, 10),
            vec![3, 4_000_000_000, 9_999_999_999]
        );
    }

    #[test]
    fn replacement_preserves_first_line_removes_interior_and_shifts_suffix() {
        let mut markers = MarkerSet::default();
        for line in [4, 5, 7, 10] {
            markers.insert(MarkerKind::Bookmark, line).unwrap();
        }
        let inverse = markers
            .apply(MarkerTransform::new(LineEdit::replacement(4, 4, 2)))
            .unwrap();
        assert_eq!(markers.range(MarkerKind::Bookmark, 0, 20, 20), vec![4, 8]);

        let redo = markers.apply(inverse).unwrap();
        assert_eq!(
            markers.range(MarkerKind::Bookmark, 0, 20, 20),
            vec![4, 5, 7, 10]
        );
        markers.apply(redo).unwrap();
        assert_eq!(markers.range(MarkerKind::Bookmark, 0, 20, 20), vec![4, 8]);
    }

    #[test]
    fn marker_added_on_inserted_line_disappears_on_undo_and_returns_on_redo() {
        let mut markers = MarkerSet::default();
        markers.insert(MarkerKind::Bookmark, 20).unwrap();

        let undo = markers
            .apply(MarkerTransform::new(LineEdit::replacement(10, 1, 3)))
            .unwrap();
        markers.insert(MarkerKind::Bookmark, 11).unwrap();
        assert_eq!(markers.range(MarkerKind::Bookmark, 0, 30, 30), vec![11, 22]);

        let redo = markers.apply(undo).unwrap();
        assert_eq!(markers.range(MarkerKind::Bookmark, 0, 30, 30), vec![20]);
        markers.apply(redo).unwrap();
        assert_eq!(markers.range(MarkerKind::Bookmark, 0, 30, 30), vec![11, 22]);
    }

    #[test]
    fn marker_on_first_seeded_line_disappears_when_document_becomes_empty() {
        let mut markers = MarkerSet::default();
        let undo = markers
            .apply(MarkerTransform::new(LineEdit::replacement(0, 0, 1)))
            .unwrap();
        markers.insert(MarkerKind::Bookmark, 0).unwrap();

        let redo = markers.apply(undo).unwrap();
        assert!(markers.is_empty());
        markers.apply(redo).unwrap();
        assert!(markers.contains(MarkerKind::Bookmark, 0));
    }

    #[test]
    fn insert_and_delete_are_exact_inverses_for_every_marker_kind() {
        let mut markers = MarkerSet::default();
        for (i, kind) in MarkerKind::ALL.into_iter().enumerate() {
            markers.insert(kind, 2 + i as u64).unwrap();
            markers.insert(kind, 12 + i as u64).unwrap();
        }
        let original = markers.all_in_range(0, u64::MAX, usize::MAX);
        let undo = markers
            .apply(MarkerTransform::new(LineEdit::replacement(5, 0, 7)))
            .unwrap();
        let redo = markers.apply(undo).unwrap();
        assert_eq!(markers.all_in_range(0, u64::MAX, usize::MAX), original);
        markers.apply(redo).unwrap();
        assert_ne!(markers.all_in_range(0, u64::MAX, usize::MAX), original);
    }

    #[test]
    fn overflowing_shift_is_rejected_without_mutating_markers() {
        let mut markers = MarkerSet::default();
        markers.insert(MarkerKind::Bookmark, u64::MAX).unwrap();

        let result = markers.apply(MarkerTransform::new(LineEdit::replacement(0, 0, 1)));

        assert!(result.is_err());
        assert_eq!(markers.len(MarkerKind::Bookmark), 1);
        assert!(markers.contains(MarkerKind::Bookmark, u64::MAX));
    }
}
