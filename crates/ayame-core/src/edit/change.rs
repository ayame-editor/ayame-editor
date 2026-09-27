//! Change-history derivation: saved/unsaved/deleted markers for the view.

use super::*;

/// Sparse change-history image for the CURRENT logical view.
///
/// `saved` and `unsaved` are status rails. `deleted` is an orthogonal shape
/// flag at the next surviving line, or at `total_lines` (the logical EOF row),
/// and is therefore always paired with one of the status sets. Every vector is
/// sorted, deduplicated, and capped by [`MAX_MARKERS_PER_KIND`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChangeHistory {
    pub saved: Vec<u64>,
    pub unsaved: Vec<u64>,
    pub deleted: Vec<u64>,
    pub limit_reached: bool,
}

#[derive(Default)]
struct ChangeHistoryBuilder {
    saved: BTreeSet<u64>,
    unsaved: BTreeSet<u64>,
    deleted: BTreeSet<u64>,
    limit_reached: bool,
}

impl ChangeHistoryBuilder {
    fn insert_limited(&mut self, kind: ChangeHistoryKind, line: u64) {
        let target = match kind {
            ChangeHistoryKind::Saved => &mut self.saved,
            ChangeHistoryKind::Unsaved => &mut self.unsaved,
            ChangeHistoryKind::Deleted => &mut self.deleted,
        };
        if target.contains(&line) {
            return;
        }
        if target.len() == MAX_MARKERS_PER_KIND {
            self.limit_reached = true;
            return;
        }
        target.insert(line);
    }

    fn saved_line(&mut self, line: u64) {
        self.insert_limited(ChangeHistoryKind::Saved, line);
    }

    fn unsaved_line(&mut self, line: u64) {
        self.insert_limited(ChangeHistoryKind::Unsaved, line);
    }

    fn saved_deletion(&mut self, line: u64) {
        self.saved_line(line);
        self.insert_limited(ChangeHistoryKind::Deleted, line);
    }

    fn unsaved_deletion(&mut self, line: u64) {
        self.unsaved_line(line);
        self.insert_limited(ChangeHistoryKind::Deleted, line);
    }

    fn finish(self) -> ChangeHistory {
        ChangeHistory {
            saved: self.saved.into_iter().collect(),
            unsaved: self.unsaved.into_iter().collect(),
            deleted: self.deleted.into_iter().collect(),
            limit_reached: self.limit_reached,
        }
    }
}

enum ChangeHistoryKind {
    Saved,
    Unsaved,
    Deleted,
}

/// Compare the insertion lists at one immutable original anchor. Exact common
/// prefixes/suffixes retain their saved state; only the bounded middle is
/// paired positionally. This linear alignment correctly localizes ordinary
/// insert/delete/replace edits without an unbounded `O(n²)` sequence diff.
fn classify_insert_changes(
    out: &mut ChangeHistoryBuilder,
    position: u64,
    current: &[String],
    saved: &[String],
) {
    let mut prefix = 0usize;
    while prefix < current.len() && prefix < saved.len() && current[prefix] == saved[prefix] {
        out.saved_line(position.saturating_add(prefix as u64));
        prefix += 1;
    }

    let max_suffix = current
        .len()
        .saturating_sub(prefix)
        .min(saved.len().saturating_sub(prefix));
    let mut suffix = 0usize;
    while suffix < max_suffix
        && current[current.len() - 1 - suffix] == saved[saved.len() - 1 - suffix]
    {
        suffix += 1;
    }

    let current_end = current.len() - suffix;
    let saved_end = saved.len() - suffix;
    let current_middle = current_end - prefix;
    let saved_middle = saved_end - prefix;
    let paired = current_middle.min(saved_middle);
    for offset in 0..paired {
        let current_index = prefix + offset;
        let line = position.saturating_add(current_index as u64);
        if current[current_index] == saved[prefix + offset] {
            out.saved_line(line);
        } else {
            out.unsaved_line(line);
        }
    }
    for current_index in (prefix + paired)..current_end {
        out.unsaved_line(position.saturating_add(current_index as u64));
    }
    if saved_middle > current_middle {
        // Missing saved insertions collapse onto the first surviving suffix
        // line, the original anchor line, or the logical EOF row.
        out.unsaved_deletion(position.saturating_add(current_end as u64));
    }

    for offset in (0..suffix).rev() {
        let current_index = current.len() - 1 - offset;
        out.saved_line(position.saturating_add(current_index as u64));
    }
}

impl EditSession {
    /// Derive saved/unsaved change markers from the two sparse overlays that
    /// are authoritative for the editor view: `events` (current content) and
    /// `saved_events` (the last bytes that successfully reached disk).
    ///
    /// The walk merges edited ORIGINAL anchors only. Untouched spans are never
    /// visited, no original line text is decoded, and no line-count-sized
    /// bitmap is allocated. Runtime is `O(E + M)` and memory `O(M)`, where `E`
    /// is the number of edited anchors and `M` the admitted change markers.
    #[must_use]
    pub fn change_history(&self, doc: &Document) -> ChangeHistory {
        let original_lines = doc.line_count();
        let mut anchors = BTreeSet::new();
        anchors.extend(self.events.keys().copied());
        anchors.extend(self.saved_events.keys().copied());

        let empty = EditEvent::default();
        let mut out = ChangeHistoryBuilder::default();
        let mut inserted_before = 0u64;
        let mut deleted_before = 0u64;

        for anchor in anchors {
            let current = self.events.get(&anchor).unwrap_or(&empty);
            let saved = self.saved_events.get(&anchor).unwrap_or(&empty);
            let original = anchor.min(original_lines);
            let position = original
                .checked_add(inserted_before)
                .and_then(|line| line.checked_sub(deleted_before))
                // A valid overlay cannot overflow. Saturating to EOF keeps a
                // corrupt/internally impossible anchor from producing a
                // wrapped marker coordinate in release builds.
                .unwrap_or_else(|| self.total_lines(doc));

            classify_insert_changes(&mut out, position, &current.inserts, &saved.inserts);

            if anchor < original_lines {
                let current_line = position.saturating_add(current.inserts.len() as u64);
                match (current.deleted, saved.deleted) {
                    (true, true) => {
                        // Both views omit the original line. The saved view
                        // differs from the as-opened document, so this is a
                        // persisted deletion at the next line / EOF boundary.
                        out.saved_deletion(current_line);
                    }
                    (true, false) => out.unsaved_deletion(current_line),
                    (false, true) => out.unsaved_line(current_line),
                    (false, false) if current.replacement == saved.replacement => {
                        if saved.replacement.is_some() {
                            out.saved_line(current_line);
                        }
                    }
                    (false, false) => out.unsaved_line(current_line),
                }
            }

            inserted_before = inserted_before.saturating_add(current.inserts.len() as u64);
            if anchor < original_lines && current.deleted {
                deleted_before = deleted_before.saturating_add(1);
            }
        }

        out.finish()
    }
}
