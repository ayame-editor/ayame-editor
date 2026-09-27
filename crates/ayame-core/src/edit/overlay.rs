//! Overlay snapshots and save-time rebase sources for crash recovery.

use super::*;

/// Serializable image of the overlay for WAL compaction snapshots
/// ([`crate::wal`]). Carries exactly what a recovered session needs to show
/// the same text again: the anchor map, plus whether that content differed
/// from the last save. The undo/redo history is deliberately NOT serialized —
/// a recovered session starts with an empty history below the replayed
/// suffix.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct OverlaySnapshot {
    events: BTreeMap<u64, EditEvent>,
    dirty: bool,
}

impl OverlaySnapshot {
    /// Whether restoring this snapshot yields anything besides a clean,
    /// as-opened session (i.e. it is worth recovering).
    pub(crate) fn is_effective(&self) -> bool {
        self.dirty || !self.events.is_empty()
    }
}

/// Everything captured at a save-time crash-log reset that a LATER snapshot
/// needs to re-anchor the live overlay onto the just-saved file.
///
/// After an in-place save, the disk file is `old base + overlay_at_save`
/// while the live session keeps the OLD mmap [`Document`] and its overlay —
/// that is what lets undo cross the save. The session overlay's anchors
/// therefore keep referring to the old base, and serializing it verbatim into
/// a log whose header names the NEW base would replay onto the wrong lines.
/// [`RebaseSource::rebase`] converts between the two coordinate spaces.
#[derive(Clone, Debug)]
pub(crate) struct RebaseSource {
    /// The overlay at the moment of the save — exactly the delta
    /// `old base → new (saved) base`, anchored to the old base.
    events: BTreeMap<u64, EditEvent>,
    /// Old-base text of every line `events` replaces or deletes. These are
    /// the only anchors whose original text a rebase can need: a line the
    /// save left untouched is carried verbatim by the new base itself, and a
    /// live `replacement` is always distinct from its original (the mutators
    /// normalize an identical replacement to `None`).
    originals: BTreeMap<u64, String>,
    /// Old-base line count (the append anchor of the old coordinate space).
    old_lines: u64,
}

impl RebaseSource {
    /// Re-anchor `session`'s overlay — still keyed by OLD-base lines — onto
    /// the new base (= old base with `self.events` applied), producing the
    /// overlay a fresh session over the SAVED file needs to show exactly
    /// `session`'s current view.
    ///
    /// Both anchor maps are walked in one ascending merge. The save overlay
    /// alone determines where each old line landed in the new base: old line
    /// `o` maps to `o + (save-inserts at anchors ≤ o) − (save-deletions at
    /// anchors < o)`, and the new base is dense in that order — a region per
    /// old anchor: first the save's insert lines, then (unless the save
    /// deleted it) the anchor line itself. Within each region:
    ///
    /// * live inserts map positionally onto the save's insert lines
    ///   (replacement where the text differs, deletion of the surplus saved
    ///   lines, fresh insertions — anchored just past the region — for the
    ///   surplus live lines);
    /// * an anchor line alive in both emits a replacement when its effective
    ///   text changed since the save;
    /// * alive-at-save, deleted-now emits a deletion of the mapped line;
    /// * deleted-at-save, alive-now re-inserts the live text (the captured
    ///   original when no live replacement exists) after the surplus inserts,
    ///   preserving view order.
    ///
    /// Only view equality matters: recovery restores the overlay with an
    /// empty history, so this mapping never needs to reproduce edit identity.
    pub(crate) fn rebase(&self, session: &EditSession) -> OverlaySnapshot {
        let default_ev = EditEvent::default();
        let mut out: BTreeMap<u64, EditEvent> = BTreeMap::new();
        let n = self.old_lines;
        // Line-count effects of the save overlay BEFORE the current anchor.
        let mut ins_before: u64 = 0;
        let mut del_before: u64 = 0;
        let anchors: BTreeSet<u64> = self
            .events
            .keys()
            .chain(session.events.keys())
            .copied()
            .collect();
        for &o in &anchors {
            let s = self.events.get(&o).unwrap_or(&default_ev);
            let t = session.events.get(&o).unwrap_or(&default_ev);
            // Anchors are ≤ old_lines by construction; clamp defensively,
            // mirroring `locate`.
            let o_clamped = o.min(n);
            let is_append = o >= n;
            // New-base position of this region's first line.
            let p = o_clamped + ins_before - del_before;
            let k = s.inserts.len();

            // The save's insert lines are real new-base lines p..p+k; make
            // them show the live insert list.
            let shared = k.min(t.inserts.len());
            for (i, (now, saved)) in t.inserts.iter().zip(&s.inserts).enumerate() {
                if now != saved {
                    out.entry(p + i as u64).or_default().replacement = Some(now.clone());
                }
            }
            for i in shared..k {
                out.entry(p + i as u64).or_default().deleted = true;
            }
            // First new-base line PAST the saved inserts: the anchor line
            // itself when the save kept it, otherwise the next region.
            // Inserting here lands just before either — view order holds.
            let after_inserts = p + k as u64;
            for now in t.inserts.iter().skip(k) {
                out.entry(after_inserts)
                    .or_default()
                    .inserts
                    .push(now.clone());
            }

            // The anchor line itself (the append anchor has none).
            if !is_append {
                if !s.deleted {
                    if t.deleted {
                        out.entry(after_inserts).or_default().deleted = true;
                    } else {
                        let new_text: Option<&String> = match (&t.replacement, &s.replacement) {
                            // Replaced in both: differ ⇒ re-replace.
                            (Some(now), Some(saved)) => (now != saved).then_some(now),
                            // Replaced now, original at save: a normalized
                            // replacement always differs from the original
                            // (= what the save wrote).
                            (Some(now), None) => Some(now),
                            // Original now, replaced at save: the view shows
                            // the OLD base's text, which the new base no
                            // longer carries.
                            (None, Some(_)) => self.originals.get(&o),
                            (None, None) => None,
                        };
                        if let Some(text) = new_text {
                            out.entry(after_inserts).or_default().replacement = Some(text.clone());
                        }
                    }
                } else if !t.deleted {
                    // Deleted at save (absent from the new base) but alive
                    // now: resurrect it as an insertion, after any surplus
                    // live inserts pushed above.
                    let text = t
                        .replacement
                        .clone()
                        .or_else(|| self.originals.get(&o).cloned())
                        .unwrap_or_default();
                    out.entry(after_inserts).or_default().inserts.push(text);
                }
            }

            ins_before += k as u64;
            if !is_append && s.deleted {
                del_before += 1;
            }
        }
        OverlaySnapshot {
            events: out,
            dirty: session.is_dirty(),
        }
    }
}

impl EditSession {
    /// Serializable image of the overlay for a compaction snapshot.
    pub(crate) fn overlay_snapshot(&self) -> OverlaySnapshot {
        OverlaySnapshot {
            events: self.events.clone(),
            dirty: self.is_dirty(),
        }
    }

    /// Capture what a later [`RebaseSource::rebase`] needs, at the moment
    /// this session's content is being written to disk: the overlay (= the
    /// delta old base → new base) plus the old-base text of every line it
    /// replaces or deletes.
    pub(crate) fn rebase_source(&self, doc: &Document) -> RebaseSource {
        let old_lines = doc.line_count();
        let mut originals = BTreeMap::new();
        for (&anchor, ev) in &self.events {
            if anchor < old_lines && (ev.deleted || ev.replacement.is_some()) {
                if let Some(text) = doc.line(anchor) {
                    originals.insert(anchor, text);
                }
            }
        }
        RebaseSource {
            events: self.events.clone(),
            originals,
            old_lines,
        }
    }

    /// Restore a compaction snapshot: the overlay exactly as recorded, an
    /// empty undo/redo history (deliberately not serialized — see
    /// [`crate::wal`]), and dirtiness relative to the base file on disk (the
    /// disk holds the as-opened content after a crash, so `saved_gen` is 0).
    pub(crate) fn restore_overlay(&mut self, snap: OverlaySnapshot) {
        self.events = snap.events;
        self.saved_events = Arc::new(BTreeMap::new());
        self.undo.clear();
        self.redo.clear();
        self.saved_gen = 0;
        self.content_gen = if snap.dirty { 1 } else { 0 };
        self.next_gen = 2;
        self.bump();
    }
}
