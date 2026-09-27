//! Crash-log (WAL) integration for the edit session.
//!
//! Attaching, compacting, resetting, and mirroring committed transactions.
//! Every failure degrades to dropping the writer, never failing the edit.

use super::*;

impl EditSession {
    /// Attach (or detach) a crash log: every committed transaction is mirrored
    /// into `w` so unsaved edits survive a process crash (see [`crate::wal`]).
    /// Replaces any previous writer and clears a pending
    /// [`EditSession::take_wal_error`]. Attach BEFORE editing (or write a
    /// [`WalWriter::snapshot`] right after attaching to a session that already
    /// has edits) — the log only ever contains what happened after it started.
    pub fn set_wal(&mut self, w: Option<WalWriter>) {
        self.wal = w;
        self.wal_error = None;
    }

    /// The attached crash log, if any — so the caller can drive policy:
    /// [`WalWriter::reset_for_save`] on a successful save (or
    /// [`EditSession::wal_reset_for_save`] when the saved content is this
    /// session's), [`WalWriter::sync`] on its own fsync cadence,
    /// [`WalWriter::len_bytes`] for compaction thresholds.
    pub fn wal(&mut self) -> Option<&mut WalWriter> {
        self.wal.as_mut()
    }

    /// Duplicate the attached log's file handle so policy code can fsync it
    /// without borrowing the live writer. A cloned handle syncs the same file
    /// description on platforms Ayame supports; if cloning fails, the caller
    /// treats that like a sync failure and disables crash logging.
    pub fn wal_sync_file(&self) -> std::io::Result<Option<std::fs::File>> {
        self.wal.as_ref().map(WalWriter::sync_file).transpose()
    }

    /// Current crash-log size, if logging is attached.
    pub fn wal_len_bytes(&self) -> Option<u64> {
        self.wal.as_ref().map(WalWriter::len_bytes)
    }

    /// Capture everything needed to compact the log outside the workspace
    /// lock. The returned plan does not touch the live writer or its path
    /// until [`EditSession::wal_install_compaction`] is called.
    pub fn wal_compaction_plan(&self) -> Option<crate::wal::WalCompactionPlan> {
        self.wal.as_ref().and_then(WalWriter::compaction_plan)
    }

    /// Replace the attached writer with a staged compaction result. On any
    /// install error, logging degrades exactly like a write failure.
    pub fn wal_install_compaction(&mut self, staged: crate::wal::StagedWalCompaction) {
        let Some(w) = self.wal.take() else {
            staged.cleanup();
            return;
        };
        match w.install_compaction(staged) {
            Ok(w) => self.wal = Some(w),
            Err(e) => self.wal_error = Some(format!("crash log disabled: {e}")),
        }
    }

    /// First crash-log write failure, if one occurred; surfacing it consumes
    /// it. Logging degrades by dropping the writer — an I/O problem with the
    /// log must never fail the edit itself — so after this returns `Some` the
    /// session keeps editing, just without crash persistence.
    pub fn take_wal_error(&mut self) -> Option<String> {
        self.wal_error.take()
    }

    /// Compact the attached crash log to its header plus one full-overlay
    /// snapshot, superseding the per-transaction records. Callers watch
    /// [`WalWriter::len_bytes`] and compact past their threshold (e.g.
    /// 64 MiB). Failures degrade exactly like logging failures: the writer is
    /// dropped and the error kept for [`EditSession::take_wal_error`].
    ///
    /// After a plain [`WalWriter::reset`] (no session capture) the overlay
    /// cannot be expressed against the log's new base, so compaction is
    /// skipped rather than writing a wrongly-anchored snapshot — the log as
    /// written is still correct, just not compacted. A reset via
    /// [`EditSession::wal_reset_for_save`] / [`WalWriter::reset_for_save`]
    /// keeps compaction working.
    pub fn wal_compact(&mut self) {
        let Some(mut w) = self.wal.take() else { return };
        // A clean session has nothing unsaved to protect: its overlay only
        // exists so undo can cross the last save. Snapshotting it would write
        // already-saved content into the crash log, and because the log's header
        // matches the saved file the next launch reads that snapshot as a
        // recoverable crash and falsely offers to restore it (#5). Compacting a
        // clean log is redundant anyway — a save already resets it to the header.
        if !self.is_dirty() {
            self.wal = Some(w);
            return;
        }
        if !w.can_snapshot() {
            self.wal = Some(w);
            return;
        }
        match w.snapshot(self) {
            Ok(()) => self.wal = Some(w),
            Err(e) => self.wal_error = Some(format!("crash log disabled: {e}")),
        }
    }

    /// Reset the attached crash log after a successful save of THIS session's
    /// current content: `header` must describe the just-saved file (the new
    /// on-disk base) and `doc` the still-mapped pre-save document the session
    /// edits against. Captures the save-time overlay so later degradation and
    /// compaction snapshots are re-anchored onto the new base (see
    /// [`RebaseSource`]). A no-op without a writer; on error the writer is
    /// dropped and the failure surfaced through
    /// [`EditSession::take_wal_error`], mirroring the logging degradation.
    ///
    /// When the saved bytes came from a session snapshot that may have raced
    /// live edits, call [`WalWriter::reset_for_save`] with THAT snapshot
    /// session instead — the capture must describe what actually reached the
    /// disk.
    pub fn wal_reset_for_save(&mut self, doc: &Document, header: crate::wal::Header) {
        let Some(mut w) = self.wal.take() else { return };
        match w.reset_for_save(header, doc, self) {
            Ok(()) => self.wal = Some(w),
            Err(e) => self.wal_error = Some(format!("crash log disabled: {e}")),
        }
    }

    /// Reset the live writer using the edit snapshot whose bytes actually
    /// reached disk. Use this when the live session may have advanced after the
    /// save snapshot was taken; capturing `self` would describe a different
    /// base and corrupt later recovery rebases.
    pub fn wal_reset_for_save_from(
        &mut self,
        doc: &Document,
        header: crate::wal::Header,
        saved: &EditSession,
    ) {
        let Some(mut w) = self.wal.take() else { return };
        match w.reset_for_save(header, doc, saved) {
            Ok(()) => self.wal = Some(w),
            Err(e) => self.wal_error = Some(format!("crash log disabled: {e}")),
        }
    }
    /// Mirror a committed transaction into the attached crash log. Called only
    /// after a mutating public op ACTUALLY changed state (the same condition
    /// that pushes history and bumps the revision). `make` runs — and the op
    /// is materialized — only when a writer is attached, so the no-WAL hot
    /// path pays a single `Option` check.
    ///
    /// An undo/redo that walks into history the log cannot replay (entries
    /// older than the log's start: before a reset-on-save or a compaction
    /// snapshot) is degraded to a fresh snapshot of the current state, which
    /// is always replayable. A write failure NEVER fails the edit: the writer
    /// is dropped and the error kept for [`EditSession::take_wal_error`].
    pub(super) fn wal_commit(&mut self, make: impl FnOnce() -> LoggedOp) {
        if self.wal.is_none() {
            return;
        }
        let op = make();
        let Some(mut w) = self.wal.take() else { return };
        let result = if w.can_replay(&op) {
            w.log(&op)
        } else {
            w.snapshot(self)
        };
        match result {
            Ok(()) => self.wal = Some(w),
            Err(e) => self.wal_error = Some(format!("crash log disabled: {e}")),
        }
    }
    /// Detach the writer while [`crate::wal::replay`] drives this session, so
    /// replayed ops are not logged right back into the file being read.
    pub(crate) fn wal_detach(&mut self) -> Option<WalWriter> {
        self.wal.take()
    }

    pub(crate) fn wal_restore(&mut self, w: Option<WalWriter>) {
        self.wal = w;
    }
}
