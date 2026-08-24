use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

use axum::http::StatusCode;
use ayame_core::wal;
use ayame_core::{Document, EditSession, Encoding, OpenOptions};
use serde::Serialize;

use super::analysis::AnalysisStore;
use super::markers::MarkerSession;
use super::ops::WorkerInput;
use super::{bad_request, internal, ApiError};

mod tabs;
mod tail;
mod ui_state;
mod wal_policy;

#[cfg(test)]
use tabs::InactiveTab;
#[cfg(feature = "typegen")]
pub(super) use tabs::TabInfo;
use tabs::{remove_aside_files, TabList};
pub(super) use tabs::{TabsResponse, Workspace};
pub(super) use tail::TailStatus;
pub(super) use ui_state::UiState;
#[cfg(feature = "typegen")]
pub(super) use ui_state::{SessionState, SyntaxMapping, SyntaxOverride};
use wal_policy::{attach_live_wal, wal_setup_for_open, WalSetup};

type Shared = Arc<Document>;

/// Lock a `RwLock` for reading, recovering from poisoning.
///
/// A panic inside one request is already turned into a single 500 by the
/// `CatchPanicLayer`; letting the poison flag turn every *subsequent* request
/// into a 500 would violate "stability is a feature". The data guarded here
/// has no invariant that survives-a-panic recovery can break which the
/// revision/identity checks at save-commit time don't already cover.
pub(super) fn read_lock<T>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(|p| p.into_inner())
}

/// Lock a `RwLock` for writing, recovering from poisoning. See [`read_lock`].
pub(super) fn write_lock<T>(lock: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    lock.write().unwrap_or_else(|p| p.into_inner())
}

/// Lock a `Mutex`, recovering from poisoning. See [`read_lock`] for the "a
/// poisoned lock must not take down every later request" rationale. This matters
/// most when the lock is taken inside a `Drop` that can run while unwinding a
/// panicking request: `.lock().unwrap()` there would double-panic into a process
/// abort, the opposite of "stability is a feature" (#106).
pub(super) fn lock_recover<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
    lock.lock().unwrap_or_else(|p| p.into_inner())
}

/// A consistent, owned snapshot of the active document and its edits, taken
/// under one lock acquisition. Long-running work (saving, diffing, worker
/// materialization) operates on this so it never holds the workspace lock;
/// [`AppState::detach_for_overwrite`] later verifies the workspace still
/// matches it (same document identity, same edit revision) before committing.
pub(super) struct EditSnapshot {
    pub(super) doc: Shared,
    pub(super) edits: EditSession,
    revision: u64,
}

impl EditSnapshot {
    /// Move the snapshot's edit session out (e.g. into a blocking save task)
    /// without another deep clone. The identity used by
    /// [`AppState::detach_for_overwrite`] (document + revision) stays intact.
    pub(super) fn take_edits(&mut self) -> EditSession {
        std::mem::take(&mut self.edits)
    }
}

/// A materialized snapshot of the dirty buffer, keyed by (document identity,
/// edit revision). `/api/find` (and `/api/search`) run against this so hits
/// line up with the edited view, without re-streaming the whole overlay on
/// every incremental call: the snapshot is built once per revision and reused
/// until an edit, save, or tab change makes it stale.
pub(super) struct DirtySnapshotCache {
    /// Identity of the live document the snapshot was taken from — compared
    /// with `Arc::ptr_eq`, the same identity basis `detach_for_overwrite` uses.
    pub(super) doc: Arc<Document>,
    /// Edit revision the snapshot reflects.
    pub(super) revision: u64,
    /// Guard keeping the materialized temp file alive. `Arc` so a request can
    /// hold the file open (e.g. across a search worker child) even if the
    /// cache slot is invalidated mid-flight; the temp dir is removed when the
    /// last holder drops.
    pub(super) input: Arc<WorkerInput>,
    /// An `ayame_core::Document` opened over the materialized file (no index
    /// cache dir — throwaway snapshots must not pollute the on-disk cache).
    pub(super) snapshot: Arc<Document>,
}

pub(crate) struct AppState {
    /// The whole mutable workspace behind a single `RwLock`.
    ///
    /// LOCKING DESIGN / INVARIANTS
    ///
    /// 1. `doc`, `edits` and `tabs` are only ever read or written together,
    ///    under one continuous acquisition of this lock (`AppState::read` /
    ///    `AppState::write`). A request can therefore never observe the doc of
    ///    one tab paired with the edits of another, and an edit can never land
    ///    between "clone state" and "swap state" of a tab switch.
    /// 2. Guards never live across an `.await` (the closure-based accessors
    ///    make that impossible), so slow work — indexing a file, streaming a
    ///    save — never blocks the viewport.
    /// 3. Multi-step transitions that must span an `.await` (in-place save:
    ///    detach → rename → reopen → install; open; tab switch/close) are
    ///    serialized by `transitions` below, and the in-place save re-validates
    ///    an [`EditSnapshot`] (same `Arc<Document>` identity AND same
    ///    `EditSession::revision`) at its commit point. Stale commits are
    ///    rejected with 409 instead of silently discarding edits made while
    ///    the save was streaming.
    ws: RwLock<Workspace>,
    /// In-flight artifact operations (sort/replace/case/grep/split), keyed by
    /// the client-supplied op id, for progress polling and cancellation.
    ///
    /// Owned by the session rather than the process (#106): a `static` map was
    /// shared by every `AppState` a test built, and — being reachable from a
    /// `Drop` that can run while a request unwinds — its poisoning had to be
    /// recovered from anyway. Here it is per-session, and `lock_recover` is the
    /// same helper the workspace lock uses.
    artifact_ops: Mutex<HashMap<String, Arc<super::ops::ArtifactOperation>>>,
    /// Serializes multi-step doc-slot transitions (open / new / upload / tab
    /// switch / tab close / in-place save commit). Held across `.await`s,
    /// hence a tokio mutex. Ordering rule: acquire `transitions` BEFORE `ws`;
    /// never acquire `transitions` while a `ws` guard is held.
    transitions: tokio::sync::Mutex<()>,
    /// The open options (encoding/stride/cache) picked on the command line,
    /// reused whenever a new file is opened from the browser.
    open_opts: OpenOptions,
    /// Revision-keyed snapshot of the dirty buffer for find/search (see
    /// [`DirtySnapshotCache`]).
    ///
    /// LOCKING: this is a LEAF lock, strictly below both `transitions` and
    /// `ws` in the `transitions → ws` order: it is never held while `ws` is
    /// taken (accessors read the workspace first, drop that guard, then touch
    /// this slot), never held across an `.await`, and nothing else is locked
    /// while it is held.
    find_snapshot: Mutex<Option<DirtySnapshotCache>>,
    /// How many snapshots have been materialized+opened (test observability).
    snapshot_builds: AtomicU64,
    /// First crash-log failure not yet shown to the user. Filled by the WAL
    /// policy tick (which consumes [`EditSession::take_wal_error`]) or by a
    /// failed post-save log reset; drained once by the next stat response.
    /// LEAF lock, same discipline as `find_snapshot`.
    wal_error: Mutex<Option<String>>,
    /// Bounded multi-rule log-analysis operations. Each operation pins one
    /// immutable document/edit generation and is evicted after four entries.
    analysis: AnalysisStore,
}

impl AppState {
    pub(super) fn new(doc: Option<Document>, open_opts: OpenOptions) -> AppState {
        // The document passed on the command line is an "open" like any
        // other: inspect its crash log before the first writer is created.
        let wal_setup = match &doc {
            Some(d) => wal_setup_for_open(open_opts.cache_dir.as_deref(), d),
            None => WalSetup::Off,
        };
        let shared = doc.map(Arc::new);
        let mut tabs = TabList {
            next_id: 1,
            ..TabList::default()
        };
        if shared.is_some() {
            let id = tabs.next_id;
            tabs.next_id += 1;
            tabs.order.push(id);
            tabs.active = Some(id);
        }
        let mut edits = EditSession::default();
        let mut recoverable = None;
        match wal_setup {
            WalSetup::Attach(w) => edits.set_wal(Some(*w)),
            WalSetup::Create => {}
            WalSetup::Recoverable(n) => recoverable = Some(n),
            WalSetup::Off => {}
        }
        AppState {
            ws: RwLock::new(Workspace {
                disk_baseline: shared.as_ref().and_then(|d| d.disk_state()),
                doc: shared,
                edits,
                markers: MarkerSession::default(),
                aside_files: Vec::new(),
                recoverable,
                tabs,
            }),
            artifact_ops: Mutex::new(HashMap::new()),
            transitions: tokio::sync::Mutex::new(()),
            open_opts,
            find_snapshot: Mutex::new(None),
            snapshot_builds: AtomicU64::new(0),
            wal_error: Mutex::new(None),
            analysis: AnalysisStore::default(),
        }
    }

    pub(super) fn analysis_store(&self) -> &AnalysisStore {
        &self.analysis
    }

    /// Root directory for crash logs — the same per-user cache dir the index
    /// uses (`--cache-dir` / `AYAME_CACHE_DIR`; `None` under `--no-cache`).
    pub(super) fn wal_root(&self) -> Option<&Path> {
        self.open_opts.cache_dir.as_deref()
    }

    /// The cached dirty snapshot, if it still matches (same document identity,
    /// same edit revision). A stale entry is dropped on the spot — checking on
    /// every use is the invalidation strategy — which also removes its temp
    /// file (unless a request still holds the guard).
    pub(super) fn cached_dirty_snapshot(
        &self,
        doc: &Arc<Document>,
        revision: u64,
    ) -> Option<(Arc<Document>, Arc<WorkerInput>)> {
        let mut slot = self.find_snapshot.lock().unwrap_or_else(|p| p.into_inner());
        match slot.as_ref() {
            Some(c) if Arc::ptr_eq(&c.doc, doc) && c.revision == revision => {
                Some((c.snapshot.clone(), c.input.clone()))
            }
            Some(_) => {
                *slot = None; // stale: revision bumped or the tab changed
                None
            }
            None => None,
        }
    }

    /// Install a freshly built snapshot (replacing whatever was cached).
    pub(super) fn store_dirty_snapshot(&self, cache: DirtySnapshotCache) {
        self.snapshot_builds.fetch_add(1, AtomicOrdering::Relaxed);
        let mut slot = self.find_snapshot.lock().unwrap_or_else(|p| p.into_inner());
        *slot = Some(cache);
    }

    /// Drop the cached snapshot (and, with it, the materialized temp file).
    /// Called from the natural staleness hooks — tab switch/close, open,
    /// reload after save — so temps never linger; correctness never depends
    /// on it because every use re-validates identity + revision anyway.
    pub(super) fn invalidate_dirty_snapshot(&self) {
        let mut slot = self.find_snapshot.lock().unwrap_or_else(|p| p.into_inner());
        *slot = None;
    }

    /// Number of dirty snapshots built so far (asserted by serve tests).
    #[cfg(test)]
    pub(super) fn dirty_snapshot_builds(&self) -> u64 {
        self.snapshot_builds.load(AtomicOrdering::Relaxed)
    }

    /// Run `f` with a consistent read-only view of the workspace. The guard
    /// cannot escape or be held across an `.await`.
    pub(super) fn read<R>(&self, f: impl FnOnce(&Workspace) -> R) -> R {
        f(&read_lock(&self.ws))
    }

    /// Run `f` with exclusive access to the workspace. See [`AppState::read`].
    pub(super) fn write<R>(&self, f: impl FnOnce(&mut Workspace) -> R) -> R {
        f(&mut write_lock(&self.ws))
    }

    /// The open document, if any (cheap `Arc` clone).
    pub(super) fn doc_opt(&self) -> Option<Shared> {
        self.read(|ws| ws.doc.clone())
    }

    /// Owned snapshot of the active document + edits + revision, taken under
    /// one lock acquisition. The `EditSession` clone here is the one clone
    /// that is genuinely needed (long saves must not hold the lock).
    pub(super) fn edit_snapshot(&self) -> Result<EditSnapshot, ApiError> {
        self.read(|ws| {
            let (doc, edits) = ws.doc_and_edits()?;
            Ok(EditSnapshot {
                doc: doc.clone(),
                edits: edits.clone(),
                revision: edits.revision(),
            })
        })
    }

    /// The active document plus a clone of the edit overlay when the view
    /// cannot be answered by the document alone — what worker materialization
    /// and diff need. `None` guarantees BOTH stand-ins are faithful: the
    /// mmap'd document object (overlay empty) and the file at its path (the
    /// session is not dirty against the last save, and after an in-place save
    /// that path holds exactly the saved view). Sessions passing both checks
    /// skip the copy entirely.
    pub(super) fn doc_and_dirty_edits(&self) -> Result<(Shared, Option<EditSession>), ApiError> {
        let (doc, dirty, _revision) = self.doc_dirty_view_source()?;
        Ok((doc, dirty))
    }

    /// Atomic source pin for a dirty-aware read. The live document identity,
    /// optional overlay snapshot, and edit revision come from one workspace
    /// guard so a background analysis can reject results from a later edit or
    /// tab without mixing generations.
    pub(super) fn doc_dirty_view_source(
        &self,
    ) -> Result<(Shared, Option<EditSession>, u64), ApiError> {
        self.read(|ws| {
            let (doc, edits) = ws.doc_and_edits()?;
            let dirty = (edits.has_edits() || edits.is_dirty()).then(|| edits.clone());
            Ok((doc.clone(), dirty, edits.revision()))
        })
    }

    pub(super) fn open_options(&self) -> OpenOptions {
        self.open_opts.clone()
    }

    /// The session's in-flight artifact operations. See the field docs for why
    /// this is per-session rather than a process global.
    pub(super) fn artifact_ops(
        &self,
    ) -> MutexGuard<'_, HashMap<String, Arc<super::ops::ArtifactOperation>>> {
        lock_recover(&self.artifact_ops)
    }

    /// Serialize a multi-step doc-slot transition against all others. See the
    /// locking notes on [`AppState::ws`].
    pub(super) async fn lock_transitions(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.transitions.lock().await
    }

    /// Commit point of an in-place save: verify the workspace still matches
    /// `snap` (same document identity, same edit revision), then detach the
    /// document so its mmap is released before the file is replaced on disk.
    /// The edit overlay is left in place so it can be restored if the replace
    /// fails; on success the caller resets it via [`AppState::mark_edits_saved`].
    ///
    /// Returns 409 when edits arrived or the tab changed while the save was
    /// streaming — the caller must discard its staged output and the client
    /// retries, so nothing is ever silently lost.
    ///
    /// Caller must hold the transitions lock.
    pub(super) fn detach_for_overwrite(&self, snap: &EditSnapshot) -> Result<(), ApiError> {
        self.write(|ws| {
            let same_doc = ws.doc.as_ref().is_some_and(|d| Arc::ptr_eq(d, &snap.doc));
            if !same_doc || ws.edits.revision() != snap.revision {
                return Err(ApiError::new(
                    StatusCode::CONFLICT,
                    "conflict",
                    "the document changed while saving — nothing was overwritten; save again",
                ));
            }
            // Edits stay: they are restored if the replace fails. The baseline
            // goes with the document — whatever lands at the path next is
            // installed by a reload, which seeds a fresh one.
            ws.set_doc(None);
            Ok(())
        })
    }

    /// Whether the active document's file has been written by something other
    /// than this session. One `stat`; safe to call on every window focus.
    pub(super) fn disk_check(&self) -> DiskCheckResponse {
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
    pub(super) fn confirm_disk_unchanged(&self) -> Result<(), ApiError> {
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

    /// The staged bytes reached the target file: the pending edits are now on
    /// disk, so clear the overlay (the reloaded document IS the edited text).
    /// Used by full-reload commits (in-place sort); the in-place SAVE keeps
    /// the overlay and history via [`AppState::commit_in_place_save`] instead.
    pub(super) fn mark_edits_saved(&self) {
        self.write(|ws| {
            ws.edits = EditSession::default();
            ws.markers = MarkerSession::default();
        });
        self.invalidate_dirty_snapshot(); // clean session: the snapshot temp can go
    }

    /// Commit point of an in-place save WITHOUT a reload: verify the workspace
    /// still matches `snap` (same document identity, same edit revision) so
    /// the staged bytes are known to be the current view. The document, the
    /// overlay and the undo/redo history stay untouched — that is what lets
    /// undo cross the save. Caller must hold the transitions lock and, on Ok,
    /// finish with [`AppState::commit_in_place_save`] after the file swap.
    pub(super) fn confirm_overwrite(&self, snap: &EditSnapshot) -> Result<(), ApiError> {
        self.read(|ws| {
            let same_doc = ws.doc.as_ref().is_some_and(|d| Arc::ptr_eq(d, &snap.doc));
            if !same_doc || ws.edits.revision() != snap.revision {
                return Err(ApiError::new(
                    StatusCode::CONFLICT,
                    "conflict",
                    "the document changed while saving — nothing was overwritten; save again",
                ));
            }
            Ok(())
        })
    }

    /// The staged bytes replaced the target file (the pre-save file lives on
    /// under `aside`, if the target existed): record that the SNAPSHOTTED
    /// content generation is what's on disk — edits that raced the rename
    /// simply leave the session dirty — and register the aside file with the
    /// active tab. All accumulated aside files are then deleted best-effort
    /// (on Unix they all go, the mmap keeps its inode; on Windows the mapped
    /// one refuses and stays tracked for the close/shutdown hooks).
    ///
    /// Caller must hold the transitions lock (the active tab cannot change
    /// between [`AppState::confirm_overwrite`] and this call).
    pub(super) fn commit_in_place_save(&self, snap: &EditSnapshot, aside: Option<PathBuf>) {
        let mut reset_err = None;
        let pending = self.write(|ws| {
            ws.edits.mark_saved_from(&snap.edits);
            ws.markers.sync_change_history(&ws.edits, &snap.doc);
            // The saved bytes ARE the file now: the base identity (len/mtime)
            // changed, so the crash log must restart from the new header —
            // its old records must never replay onto the new base.
            // reset_for_save also captures the overlay that produced the new
            // base, so later snapshots (compaction, undo past the save point)
            // are REBASED onto the saved file instead of carrying stale
            // old-base anchors. Edits do not take the transitions lock, so one
            // can still land during the filesystem swap after confirm_overwrite;
            // in that case capture the saved snapshot and immediately snapshot
            // the newer live view against it (#75). Failures degrade the writer
            // and surface once via take_wal_error().
            if ws.edits.wal().is_some() {
                match wal::Header::for_document(&snap.doc) {
                    Ok(header) if ws.edits.revision() == snap.revision => {
                        ws.edits.wal_reset_for_save(&snap.doc, header)
                    }
                    Ok(header) => {
                        ws.edits
                            .wal_reset_for_save_from(&snap.doc, header, &snap.edits);
                        ws.edits.wal_compact();
                    }
                    Err(e) => {
                        ws.edits.set_wal(None);
                        reset_err = Some(format!("crash log disabled: {e}"));
                    }
                }
            }
            // The file at the path is our own staged output now: adopt it as
            // the external-change baseline, or the very next check would read
            // this save as somebody else's write (#163).
            ws.reseed_disk_baseline();
            if let Some(aside) = aside {
                ws.aside_files.push(aside);
            }
            std::mem::take(&mut ws.aside_files)
        });
        if let Some(e) = reset_err {
            self.note_wal_error(e);
        }
        let survivors = remove_aside_files(pending);
        if !survivors.is_empty() {
            self.write(|ws| {
                let mut kept = survivors;
                kept.append(&mut ws.aside_files);
                ws.aside_files = kept;
            });
        }
        // NOTE: no snapshot invalidation — the view, the document identity and
        // the revision are all unchanged by a save, so a cached dirty snapshot
        // is still exactly the view.
    }

    /// Open `path` (blocking work off the async runtime) and install it in the
    /// detached active slot. Used by the in-place sort to reload the sorted
    /// file, and to restore the original document when a replace failed.
    /// Never touches the edit overlay. The replaced document's aside files are
    /// deleted (its mmap is gone with it). Caller must hold the transitions
    /// lock.
    pub(super) async fn install_reloaded(&self, path: PathBuf) -> Result<(), ApiError> {
        self.reload(path, None, None, false).await
    }

    /// Revert the active document to its last SAVED state: reload from disk
    /// (the file on disk IS the saved state, in-place saves included) with a
    /// fresh, clean edit session, and drop the old document's aside files.
    /// Caller must hold the transitions lock.
    pub(super) async fn reload_reverted(&self, path: PathBuf) -> Result<(), ApiError> {
        self.reload(path, None, None, true).await
    }

    /// Like [`AppState::reload_reverted`], but commits only if the workspace
    /// still matches `snap` (same document identity, same edit revision) —
    /// re-checked INSIDE the write that installs the reloaded document, so an
    /// edit that lands while the reload is reopening the file is never
    /// clobbered. Save-then-reload commits (変換して保存, save-as switch) must
    /// use this: they clear the overlay, and the transitions lock alone does
    /// not exclude edit requests. Caller must hold the transitions lock.
    pub(super) async fn reload_reverted_if_unchanged(
        &self,
        path: PathBuf,
        snap: &EditSnapshot,
    ) -> Result<(), ApiError> {
        self.reload(path, None, Some(snap), true).await
    }

    /// Shared reload pipeline for sort/revert/save-as/encoding transitions.
    /// `encoding_override` adjusts only this open, `snap` adds an atomic
    /// identity+revision commit check, and `reset_edits` selects whether the
    /// existing overlay survives installation.
    async fn reload(
        &self,
        path: PathBuf,
        encoding_override: Option<Encoding>,
        snap: Option<&EditSnapshot>,
        reset_edits: bool,
    ) -> Result<(), ApiError> {
        let mut opts = self.open_opts.clone();
        opts.encoding = encoding_override.or(opts.encoding);
        let p = path.clone();
        let doc = tokio::task::spawn_blocking(move || Document::open(&p, &opts))
            .await
            .map_err(internal)?
            .map_err(|e| {
                let display = super::workspace::display_path(&path);
                match encoding_override {
                    Some(encoding) => bad_request(format!(
                        "reopening '{display}' as {}: {e}",
                        encoding.label()
                    )),
                    None => bad_request(format!("reopening '{display}': {e}")),
                }
            })?;
        let asides = self.write(|ws| {
            if let Some(snap) = snap {
                let same_doc = ws.doc.as_ref().is_some_and(|d| Arc::ptr_eq(d, &snap.doc));
                if !same_doc || ws.edits.revision() != snap.revision {
                    return Err(ApiError::new(
                        StatusCode::CONFLICT,
                        "conflict",
                        "the document changed while saving — nothing was overwritten; save again",
                    ));
                }
            }
            ws.set_doc(Some(Arc::new(doc)));
            if reset_edits {
                ws.edits = EditSession::default();
                ws.markers = MarkerSession::default();
            }
            // Fresh log for the reloaded base. When the overlay survives a
            // failed-replace restore it is immediately snapshotted by
            // `attach_live_wal`; clean resets start with an empty log.
            attach_live_wal(self.open_opts.cache_dir.as_deref(), ws);
            Ok(std::mem::take(&mut ws.aside_files))
        })?;
        remove_aside_files(asides);
        self.invalidate_dirty_snapshot(); // the doc identity changed
        Ok(())
    }

    /// Reopen `path` as the active document, forcing `enc` instead of detecting
    /// the encoding — the recovery path when auto-detection guessed wrong and
    /// the file shows mojibake. Any pending edit session is dropped (the caller
    /// warns first). Caller must hold the transitions lock.
    pub(super) async fn reload_with_encoding(
        &self,
        path: PathBuf,
        enc: Encoding,
    ) -> Result<(), ApiError> {
        self.reload(path, Some(enc), None, true).await
    }
}

/// Answer to "has anything else written this file since we read it?" (#163).
/// Polled by the client when the window regains focus and before an overwrite,
/// so the user is warned rather than silently burying somebody else's work.
#[derive(Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct DiskCheckResponse {
    /// Whether a file is open at all; `changed` is meaningless when false.
    pub(super) open: bool,
    /// The file on disk is no longer the one this session last read or wrote.
    pub(super) changed: bool,
}

pub(crate) type SharedState = Arc<AppState>;

#[cfg(test)]
mod tests;
