use std::sync::Arc;

use axum::http::StatusCode;
use serde::Serialize;

use super::{AppState, Shared};
use crate::serve::ApiError;

struct TailPollSnapshot {
    doc: Shared,
    revision: u64,
    has_edits: bool,
    known_bytes: u64,
    known_lines: u64,
}

impl AppState {
    /// Poll the active document for appended data (`tail -f`). When it grew and
    /// the session has no pending edits, the refreshed document is opened off
    /// the workspace lock and installed only if the active tab and edit
    /// revision are still unchanged; a shrink or external replacement reports
    /// `changed` so the client can reopen. Blocking (stat + mmap + appended-byte
    /// scan), so
    /// callers should run it off the async runtime.
    pub(in crate::serve) fn poll_tail(&self) -> TailStatus {
        // Cheap peek under a short read lock: an Arc handle plus whether the
        // overlay holds edits (we only follow a clean, unedited view — the
        // overlay's line anchors reference the original document).
        let snap = self.read(|ws| {
            ws.doc().map(|doc| TailPollSnapshot {
                doc: doc.clone(),
                revision: ws.edits.revision(),
                has_edits: ws.edits.has_edits(),
                known_bytes: doc.byte_len(),
                known_lines: doc.line_count(),
            })
        });
        let Some(snap) = snap else {
            return TailStatus::closed();
        };
        if !snap.doc.path_identity_matches() {
            let mut status = TailStatus::at(snap.known_lines, snap.known_bytes);
            status.changed = true;
            return status;
        }
        let disk = match snap.doc.disk_len() {
            Ok(n) => n,
            // A stat failure (the file vanished) reads as "changed externally".
            Err(_) => {
                let mut s = TailStatus::at(snap.known_lines, snap.known_bytes);
                s.changed = true;
                return s;
            }
        };
        if disk == snap.known_bytes {
            return TailStatus::at(snap.known_lines, snap.known_bytes);
        }
        if disk < snap.known_bytes || snap.known_bytes == 0 {
            // Shrunk/rotated, or grown from empty (encoding never detected).
            let mut s = TailStatus::at(snap.known_lines, snap.known_bytes);
            s.changed = true;
            return s;
        }
        // Growth detected. If edits are pending we do not follow — report it so
        // the client stops auto-scrolling into a view its overlay predates.
        if snap.has_edits {
            let mut s = TailStatus::at(snap.known_lines, snap.known_bytes);
            s.pending_edits = true;
            return s;
        }

        // Clone the sparse index and scan only the current final line plus the
        // appended bytes. `Document::open` would re-scan the entire file and
        // create a fresh cache entry for every (len, mtime) pair (#76).
        let followed = snap.doc.follow_tail();
        let Ok(Some(new_doc)) = followed else {
            let mut s = TailStatus::at(snap.known_lines, snap.known_bytes);
            s.changed = true;
            return s;
        };
        match new_doc.byte_len().cmp(&snap.known_bytes) {
            std::cmp::Ordering::Less => {
                let mut s = TailStatus::at(snap.known_lines, snap.known_bytes);
                s.changed = true;
                s
            }
            std::cmp::Ordering::Equal => TailStatus::at(snap.known_lines, snap.known_bytes),
            std::cmp::Ordering::Greater => {
                let lines = new_doc.line_count();
                let bytes = new_doc.byte_len();
                let mut new_doc = Some(new_doc);
                let status = self.write(|ws| {
                    let Some(cur) = ws.doc.as_ref() else {
                        return TailStatus::closed();
                    };
                    let unchanged = Arc::ptr_eq(cur, &snap.doc)
                        && ws.edits.revision() == snap.revision
                        && !ws.edits.has_edits()
                        && cur.byte_len() == snap.known_bytes;
                    if unchanged {
                        // The appended bytes are now part of the view, so this
                        // growth is no longer an unseen external change.
                        ws.set_doc(Some(Arc::new(new_doc.take().unwrap())));
                        let mut s = TailStatus::at(lines, bytes);
                        s.grew = true;
                        return s;
                    }
                    let mut s = TailStatus::at(cur.line_count(), cur.byte_len());
                    if Arc::ptr_eq(cur, &snap.doc) && ws.edits.has_edits() {
                        s.pending_edits = true;
                    }
                    s
                });
                if status.grew {
                    self.invalidate_dirty_snapshot();
                }
                status
            }
        }
    }

    /// Whether the active document's file has been written by something other
    /// than this session. One `stat`; safe to call on every window focus.
    pub(in crate::serve) fn disk_check(&self) -> DiskCheckResponse {
        self.read(|ws| DiskCheckResponse {
            open: ws.doc.is_some(),
            changed: ws.disk_changed(),
        })
    }

    /// Refuse an overwrite that would bury somebody else's write. The client
    /// asks the user and retries with `force` when they choose to overwrite
    /// anyway, so this is a prompt rather than a wall.
    ///
    /// Checked with the transitions lock held, right before the swap: a
    /// client-side check alone would leave a window between the question and
    /// the answer wide enough for the external writer to land in (#163).
    pub(in crate::serve) fn confirm_disk_unchanged(&self) -> Result<(), ApiError> {
        if self.read(|ws| ws.disk_changed()) {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "disk_changed",
                "this file was changed outside the editor after it was opened — \
                 saving would overwrite those changes; reload it, or save again \
                 to overwrite them deliberately",
            ));
        }
        Ok(())
    }
}

/// Result of a `tail -f` poll on the active document. `lines`/`bytes` are the
/// current totals so the client can grow its scrollbar even when it decides not
/// to auto-scroll.
#[derive(Serialize)]
pub(in crate::serve) struct TailStatus {
    /// Whether a file is open at all.
    open: bool,
    /// New bytes were appended and the line index was extended in place.
    grew: bool,
    /// The file shrank or was replaced externally — the client should reopen.
    changed: bool,
    /// Growth was seen but NOT followed because the session has pending edits.
    pending_edits: bool,
    lines: u64,
    bytes: u64,
}

impl TailStatus {
    pub(in crate::serve) fn closed() -> TailStatus {
        TailStatus {
            open: false,
            grew: false,
            changed: false,
            pending_edits: false,
            lines: 0,
            bytes: 0,
        }
    }
    fn at(lines: u64, bytes: u64) -> TailStatus {
        TailStatus {
            open: true,
            grew: false,
            changed: false,
            pending_edits: false,
            lines,
            bytes,
        }
    }
}

/// Answer to "has anything else written this file since we read it?" (#163).
/// Polled by the client when the window regains focus and before an overwrite,
/// so the user is warned rather than silently burying somebody else's work.
#[derive(Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(in crate::serve) struct DiskCheckResponse {
    /// Whether a file is open at all; `changed` is meaningless when false.
    pub(in crate::serve) open: bool,
    /// The file on disk is no longer the one this session last read or wrote.
    pub(in crate::serve) changed: bool,
}
