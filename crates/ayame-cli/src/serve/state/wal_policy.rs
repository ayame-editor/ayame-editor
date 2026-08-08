use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::http::StatusCode;
use ayame_core::wal::{self, WalCompactionPlan, WalWriter};
use ayame_core::{Document, EditSession, EditStats};

use super::{AppState, Shared, Workspace};
use crate::serve::markers::MarkerSession;
use crate::serve::{bad_request, internal, ApiError};

struct WalPolicyWork {
    doc: Shared,
    revision: u64,
    sync_file: Option<std::fs::File>,
    compact: Option<(EditSession, WalCompactionPlan)>,
}

/// Outcome of the pre-open crash-log inspection ([`wal_setup_for_open`]),
/// applied when the opened document is installed as the live session.
pub(super) enum WalSetup {
    /// No cache dir configured (`--no-cache`, or none resolvable) or the log
    /// could not be created: edit without crash persistence, silently.
    Off,
    /// A fresh writer for the document (any stale/invalid log was removed).
    /// Boxed so the enum stays small on the happy paths.
    Attach(Box<WalWriter>),
    /// The log was inspected and is safe to replace, but the writer should be
    /// created only when the document is installed as the live tab.
    Create,
    /// The log holds unsaved edits from a previous process. Do NOT attach or
    /// touch it; report `recoverable` in stat and wait for
    /// `/api/edit/recover` to restore or discard.
    Recoverable(usize),
}

/// Inspect (and prepare) the crash log for a freshly opened `doc` — the one
/// place recovery is DETECTED. Stale/invalid logs are deleted silently; a
/// recoverable log is left untouched for the user's decision; otherwise a
/// fresh writer is created. Blocking file I/O: call off the async runtime.
pub(super) fn wal_setup_for_open(cache_root: Option<&Path>, doc: &Document) -> WalSetup {
    match wal_prepare_for_open(cache_root, doc) {
        WalSetup::Create => {}
        other => return other,
    }
    let Some(root) = cache_root else {
        return WalSetup::Off;
    };
    let Ok(header) = wal::Header::for_document(doc) else {
        return WalSetup::Off;
    };
    let path = wal::wal_path_for(root, doc.path());
    match WalWriter::create(&path, header) {
        Ok(w) => WalSetup::Attach(Box::new(w)),
        // Crash logging is best-effort: an unwritable cache dir must never
        // block opening the file.
        Err(_) => WalSetup::Off,
    }
}

pub(super) fn wal_prepare_for_open(cache_root: Option<&Path>, doc: &Document) -> WalSetup {
    let Some(root) = cache_root else {
        return WalSetup::Off;
    };
    let Ok(header) = wal::Header::for_document(doc) else {
        return WalSetup::Off;
    };
    let path = wal::wal_path_for(root, doc.path());
    match wal::inspect(&path, &header) {
        ayame_core::RecoveryInfo::Recoverable { transactions } => {
            // A compaction-snapshot-only log recovers with transactions == 0;
            // report at least 1 so the client knows there is something there.
            return WalSetup::Recoverable(transactions.max(1));
        }
        ayame_core::RecoveryInfo::Stale | ayame_core::RecoveryInfo::Invalid => {
            // Recorded against different bytes (or unreadable): must never
            // replay, and keeping it would only re-report forever.
            let _ = std::fs::remove_file(&path);
        }
        ayame_core::RecoveryInfo::Clean => {}
    }
    WalSetup::Create
}

/// Attach a fresh crash-log writer to the session that just became live (tab
/// switch, close-neighbor focus, revert/encoding/sort reloads). Skipped while
/// the tab still has an undecided recoverable log — creating a writer would
/// truncate it. If the session already holds edits (a parked tab coming
/// back), a full snapshot is written immediately so the log reflects reality;
/// starting from the header alone is a fresh log, i.e. the save/revert-time
/// RESET for the new base identity. Small blocking file I/O under the
/// workspace write lock (a few hundred bytes + fsync) — same order of cost as
/// the lock's other users.
pub(super) fn attach_live_wal(cache_root: Option<&Path>, ws: &mut Workspace) {
    if ws.recoverable.is_some() {
        return;
    }
    let Some(root) = cache_root else { return };
    let Some(doc) = ws.doc.clone() else { return };
    let Ok(header) = wal::Header::for_document(&doc) else {
        return;
    };
    let path = wal::wal_path_for(root, doc.path());
    match WalWriter::create(&path, header) {
        Ok(w) => {
            ws.edits.set_wal(Some(w));
            if ws.edits.has_edits() || ws.edits.is_dirty() {
                ws.edits.wal_compact();
            }
        }
        Err(_) => ws.edits.set_wal(None),
    }
}

impl AppState {
    /// Root directory for crash logs — the same per-user cache dir the index
    /// uses (`--cache-dir` / `AYAME_CACHE_DIR`; `None` under `--no-cache`).
    pub(in crate::serve) fn wal_root(&self) -> Option<&Path> {
        self.open_opts.cache_dir.as_deref()
    }

    /// Record a crash-log failure for one-shot surfacing via stat.
    pub(in crate::serve) fn note_wal_error(&self, msg: String) {
        let mut slot = self.wal_error.lock().unwrap_or_else(|p| p.into_inner());
        slot.get_or_insert(msg);
    }

    /// Drain the pending crash-log failure (shown once in the next stat).
    pub(in crate::serve) fn take_wal_error(&self) -> Option<String> {
        self.wal_error
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
    }

    /// Graceful-shutdown crash-log policy: CLEAN sessions delete their log
    /// (nothing to recover; a fresh one is created on the next open), DIRTY
    /// sessions leave it in place — it is the recovery artifact the next
    /// process will offer to replay. Undecided recoverable logs stay too.
    pub(in crate::serve) fn cleanup_wal_files(&self) {
        let Some(root) = self.open_opts.cache_dir.clone() else {
            return;
        };
        let dead: Vec<PathBuf> = self.write(|ws| {
            let mut dead = Vec::new();
            // Close the live writer first so its file is deletable everywhere.
            ws.edits.set_wal(None);
            if let Some(doc) = &ws.doc {
                if !ws.edits.is_dirty() && ws.recoverable.is_none() {
                    dead.push(wal::wal_path_for(&root, doc.path()));
                }
            }
            for tab in ws.tabs.inactive.values() {
                if !tab.edits.is_dirty() && tab.recoverable.is_none() {
                    dead.push(wal::wal_path_for(&root, tab.doc.path()));
                }
            }
            dead
        });
        for p in dead {
            let _ = std::fs::remove_file(p);
        }
    }

    /// One tick of the crash-log policy loop (runs every ~3 s on a dedicated
    /// thread): fsync the live log so committed transactions also survive
    /// power loss, compact it past the size threshold, and pick up the
    /// session's deferred write error for one-shot surfacing via stat.
    /// Potentially slow sync/stage work happens off the workspace lock; the
    /// compacted writer is installed only if no edit landed meanwhile.
    pub(in crate::serve) fn wal_policy_tick(&self) {
        /// Compact (header + one overlay snapshot) once the append log grows
        /// past this many bytes.
        const WAL_COMPACT_BYTES: u64 = 64 << 20; // 64 MiB
        if let Some(e) = self.write(|ws| ws.edits.take_wal_error()) {
            self.note_wal_error(e);
        }
        let work = self.read(|ws| {
            let doc = ws.doc()?.clone();
            let revision = ws.edits.revision();
            let sync_file = match ws.edits.wal_sync_file() {
                Ok(f) => f,
                Err(e) => return Some(Err((doc, revision, format!("crash log disabled: {e}")))),
            };
            let compact = ws
                .edits
                .wal_len_bytes()
                .is_some_and(|n| n > WAL_COMPACT_BYTES)
                .then(|| {
                    ws.edits
                        .wal_compaction_plan()
                        .map(|p| (ws.edits.clone(), p))
                })
                .flatten();
            Some(Ok(WalPolicyWork {
                doc,
                revision,
                sync_file,
                compact,
            }))
        });
        let Some(work) = work else { return };
        let work = match work {
            Ok(work) => work,
            Err((doc, revision, e)) => {
                self.disable_wal_if_unchanged(&doc, revision, e);
                return;
            }
        };

        if let Some(file) = work.sync_file {
            if let Err(e) = file.sync_data() {
                self.disable_wal_if_unchanged(
                    &work.doc,
                    work.revision,
                    format!("crash log disabled: {e}"),
                );
                return;
            }
        }

        let Some((edits, plan)) = work.compact else {
            return;
        };
        let staged = match plan.stage(&edits) {
            Ok(staged) => staged,
            Err(e) => {
                self.disable_wal_if_unchanged(
                    &work.doc,
                    work.revision,
                    format!("crash log disabled: {e}"),
                );
                return;
            }
        };
        let err = self.write(|ws| {
            let same_doc = ws.doc.as_ref().is_some_and(|d| Arc::ptr_eq(d, &work.doc));
            if same_doc && ws.edits.revision() == work.revision {
                ws.edits.wal_install_compaction(staged);
                ws.edits.take_wal_error()
            } else {
                staged.cleanup();
                None
            }
        });
        if let Some(e) = err {
            self.note_wal_error(e);
        }
    }

    fn disable_wal_if_unchanged(&self, doc: &Shared, revision: u64, error: String) {
        let err = self.write(|ws| {
            let same_doc = ws.doc.as_ref().is_some_and(|d| Arc::ptr_eq(d, doc));
            if same_doc && ws.edits.revision() == revision {
                ws.edits.set_wal(None);
                Some(error)
            } else {
                None
            }
        });
        if let Some(e) = err {
            self.note_wal_error(e);
        }
    }

    /// `/api/edit/recover`: apply — or discard — the crash log detected when
    /// the active document was opened. Serialized against every doc-slot
    /// transition; the replay happens OFF the workspace lock into a scratch
    /// session and is installed only after re-validating that the workspace
    /// still shows the same pristine document (same `Arc` identity, revision
    /// 0, no edits), the same discipline as the save commit. Returns the
    /// post-recovery stats and how many transactions were replayed.
    pub(in crate::serve) async fn recover_wal(
        &self,
        discard: bool,
    ) -> Result<(EditStats, usize), ApiError> {
        let _transitions = self.transitions.lock().await;
        let Some(root) = self.open_opts.cache_dir.clone() else {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "conflict",
                "クラッシュログは無効です（キャッシュディレクトリなし）",
            ));
        };
        let doc = self.read(|ws| {
            let (doc, _) = ws.doc_and_edits()?;
            if ws.recoverable.is_none() {
                return Err(ApiError::new(
                    StatusCode::CONFLICT,
                    "conflict",
                    "復元できるクラッシュログはありません".to_string(),
                ));
            }
            Ok(doc.clone())
        })?;
        let wal_path = wal::wal_path_for(&root, doc.path());

        if discard {
            let p = wal_path.clone();
            let _ = tokio::task::spawn_blocking(move || std::fs::remove_file(p)).await;
            // The doc slot cannot have changed (transitions held); clear the
            // pending flag and start logging normally from here.
            return self.write(|ws| {
                ws.recoverable = None;
                attach_live_wal(Some(&root), ws);
                let (doc, edits) = ws.doc_and_edits()?;
                Ok((edits.stats(doc), 0))
            });
        }

        // Replay off-lock into a scratch session over the same document, then
        // re-arm logging: a fresh log whose first record set (header + one
        // overlay snapshot) IS the recovered state, so the restored edits are
        // crash-safe again the moment they are installed.
        let doc_for_replay = doc.clone();
        let wal_for_replay = wal_path.clone();
        let replayed = tokio::task::spawn_blocking(move || {
            let mut session = EditSession::default();
            let n = wal::replay(&wal_for_replay, &doc_for_replay, &mut session)?;
            if let Ok(header) = wal::Header::for_document(&doc_for_replay) {
                if let Ok(w) = WalWriter::create(&wal_for_replay, header) {
                    session.set_wal(Some(w));
                    session.wal_compact();
                }
            }
            Ok::<_, ayame_core::Error>((session, n))
        })
        .await
        .map_err(internal)?
        .map_err(|e| bad_request(format!("クラッシュログを復元できません: {e}")))?;
        let (session, n) = replayed;

        let out = self.write(|ws| {
            let same_doc = ws.doc.as_ref().is_some_and(|d| Arc::ptr_eq(d, &doc));
            // Edits are the only thing that can race (they don't take the
            // transitions lock): replaying onto a session that moved on would
            // clobber the user's typing — reject instead.
            if !same_doc || ws.edits.revision() != 0 || ws.edits.has_edits() {
                return Err(ApiError::new(
                    StatusCode::CONFLICT,
                    "conflict",
                    "復元中に編集が入ったため中断しました。ファイルを開き直してください"
                        .to_string(),
                ));
            }
            ws.recoverable = None;
            ws.edits = session;
            // Recovery installs a different logical view whose transactions
            // have no marker sidecar in the WAL. Keeping pre-recovery line
            // numbers would silently misplace them, so invalidate safely.
            ws.markers = MarkerSession::default();
            ws.markers.sync_change_history(&ws.edits, &doc);
            let (doc, edits) = ws.doc_and_edits()?;
            Ok((edits.stats(doc), n))
        })?;
        // The view changed without an edit request: drop the find snapshot
        // (leaf lock, taken with no ws guard held).
        self.invalidate_dirty_snapshot();
        Ok(out)
    }
}
