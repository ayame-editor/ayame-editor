use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

use axum::http::StatusCode;
use ayame_core::wal;
use ayame_core::{DiskState, Document, EditSession, Encoding, OpenOptions};

use super::analysis::AnalysisStore;
use super::markers::MarkerSession;
use super::ops::WorkerInput;
use super::{bad_request, internal, ApiError};

mod tabs;
mod tail;
mod ui_state;
mod wal_policy;

#[cfg(feature = "typegen")]
pub(super) use tabs::TabInfo;
pub(super) use tabs::TabsResponse;
pub(super) use tail::{DiskCheckResponse, TailStatus};
#[cfg(feature = "typegen")]
pub(super) use ui_state::SessionState;
pub(super) use ui_state::UiState;

use tabs::{remove_aside_files as cleanup_aside_paths, TabList as WorkspaceTabs};
use wal_policy::{
    attach_live_wal as attach_wal, wal_setup_for_open as setup_wal_for_open,
    WalSetup as InitialWalSetup,
};

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

/// Everything mutable about the workspace: the active document, its edit
/// overlay, and the tab list. Kept in ONE struct behind ONE lock so no request
/// can ever observe a document paired with another document's edits.
pub(super) struct Workspace {
    doc: Option<Shared>,
    pub(super) edits: EditSession,
    /// Session-only sparse markers for the active document. This travels with
    /// tabs alongside `edits`, but is not document content and never enters
    /// save snapshots or the crash WAL.
    markers: MarkerSession,
    /// Aside files left behind by in-place saves of the ACTIVE document: the
    /// pre-save file is renamed to a hidden sibling so the live mmap keeps its
    /// inode readable. Each save best-effort deletes the accumulated entries
    /// (only a still-mapped file on Windows survives that); whatever remains
    /// is removed when the document is closed or replaced, and on shutdown.
    aside_files: Vec<PathBuf>,
    /// Pending crash-recovery decision for the ACTIVE document: `Some(n)`
    /// while its crash log holds `n` unsaved transactions from a previous
    /// process and the user has not chosen restore/discard yet
    /// (`/api/edit/recover`). While pending, NO writer is attached — creating
    /// one would truncate the very log waiting to be replayed.
    recoverable: Option<usize>,
    /// What the ACTIVE document's file looked like on disk the last time this
    /// session was the author of its contents — at open/reload, after a tail
    /// follow adopted appended bytes, and after each of our own saves. A later
    /// sample that differs means somebody else wrote the file, which the
    /// client warns about before an overwrite can bury it (#163).
    ///
    /// `None` when there is no document, or when the path could not be stat'ed
    /// at install time; a baseline we never had cannot be compared, so
    /// detection simply stays off rather than crying wolf.
    disk_baseline: Option<DiskState>,
    tabs: WorkspaceTabs,
}

impl Workspace {
    /// The active document, if any.
    pub(super) fn doc(&self) -> Option<&Shared> {
        self.doc.as_ref()
    }

    /// Install (or clear) the active document and re-seed the external-change
    /// baseline from what is on disk right now.
    ///
    /// The ONE way the `doc` slot changes for freshly read content, so no open,
    /// reload, or tail follow can leave the baseline describing the previous
    /// file. Tab focus takes [`Workspace::install_tab_state`] instead, which restores
    /// the baseline the tab was parked with.
    fn set_doc(&mut self, doc: Option<Shared>) {
        self.disk_baseline = doc.as_ref().and_then(|d| d.disk_state());
        self.doc = doc;
    }

    /// Re-seed the baseline against the document already installed — for the
    /// in-place save, which replaces the file on disk while deliberately
    /// keeping the live mapping, overlay, and undo history untouched. Without
    /// this the session would flag its own save as somebody else's write.
    fn reseed_disk_baseline(&mut self) {
        self.disk_baseline = self.doc.as_ref().and_then(|d| d.disk_state());
    }

    /// Whether the active document's file has been written by something other
    /// than this session since the baseline was taken. `false` with no
    /// document, and `false` when no baseline could be established.
    fn disk_changed(&self) -> bool {
        let Some(baseline) = self.disk_baseline else {
            return false;
        };
        self.doc
            .as_ref()
            .is_some_and(|doc| doc.disk_state() != Some(baseline))
    }

    /// Pending crash-recovery count for the ACTIVE document, if any (see the
    /// `recoverable` field docs).
    pub(super) fn recoverable(&self) -> Option<usize> {
        self.recoverable
    }

    /// The active document and its edit overlay, or a 409 when the workspace
    /// is empty.
    pub(super) fn doc_and_edits(&self) -> Result<(&Shared, &EditSession), ApiError> {
        match &self.doc {
            Some(doc) => Ok((doc, &self.edits)),
            None => Err(no_document()),
        }
    }

    /// The active document, edit overlay, and marker sidecar under the same
    /// workspace write guard. Edit endpoints use this to commit line changes
    /// and marker-coordinate transforms atomically.
    pub(super) fn doc_edits_markers_mut(
        &mut self,
    ) -> Result<(&Shared, &mut EditSession, &mut MarkerSession), ApiError> {
        let Workspace {
            doc,
            edits,
            markers,
            ..
        } = self;
        match doc {
            Some(doc) => Ok((doc, edits, markers)),
            None => Err(no_document()),
        }
    }

    pub(super) fn markers(&self) -> &MarkerSession {
        &self.markers
    }

    pub(super) fn markers_mut(&mut self) -> &mut MarkerSession {
        &mut self.markers
    }
}

fn no_document() -> ApiError {
    ApiError::new(
        StatusCode::CONFLICT,
        "conflict",
        "no file is open — open one first",
    )
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
            Some(d) => setup_wal_for_open(open_opts.cache_dir.as_deref(), d),
            None => InitialWalSetup::Off,
        };
        let shared = doc.map(Arc::new);
        let mut tabs = WorkspaceTabs {
            next_id: 1,
            ..WorkspaceTabs::default()
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
            InitialWalSetup::Attach(w) => edits.set_wal(Some(*w)),
            InitialWalSetup::Create => {}
            InitialWalSetup::Recoverable(n) => recoverable = Some(n),
            InitialWalSetup::Off => {}
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
        let survivors = cleanup_aside_paths(pending);
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
        self.reload_clean(path, None).await
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
        self.reload_clean(path, Some(snap)).await
    }

    async fn reload_clean(
        &self,
        path: PathBuf,
        snap: Option<&EditSnapshot>,
    ) -> Result<(), ApiError> {
        self.reload(path, None, snap, true).await
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
        let mut opts = self.open_opts.clone();
        opts.encoding = Some(enc);
        self.reload(path, Some(opts), None, true).await
    }

    /// The single reopen/install path. Callers select only the real policy
    /// differences: an options override, an optional stale-snapshot guard, and
    /// whether the edit and marker sessions reset with the document.
    async fn reload(
        &self,
        path: PathBuf,
        opts_override: Option<OpenOptions>,
        snap: Option<&EditSnapshot>,
        reset_edits: bool,
    ) -> Result<(), ApiError> {
        let forced_encoding = opts_override.as_ref().and_then(|opts| opts.encoding);
        let opts = opts_override.unwrap_or_else(|| self.open_opts.clone());
        let p = path.clone();
        let doc = tokio::task::spawn_blocking(move || Document::open(&p, &opts))
            .await
            .map_err(internal)?
            .map_err(|e| {
                let forced = forced_encoding
                    .map(|encoding| format!(" as {}", encoding.label()))
                    .unwrap_or_default();
                bad_request(format!(
                    "reopening '{}'{}: {e}",
                    super::workspace::display_path(&path),
                    forced
                ))
            })?;
        let asides = self.write(|ws| -> Result<Vec<PathBuf>, ApiError> {
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
            attach_wal(self.open_opts.cache_dir.as_deref(), ws);
            Ok(std::mem::take(&mut ws.aside_files))
        })?;
        cleanup_aside_paths(asides);
        self.invalidate_dirty_snapshot(); // the document identity changed
        Ok(())
    }
}

pub(crate) type SharedState = Arc<AppState>;

#[cfg(test)]
mod tests {
    use super::super::test_support::{scratch_dir, scratch_file_in, wal_opts};
    use super::*;

    #[test]
    fn a_hundred_session_paths_survive_the_cap() {
        // Regression for #52: a ~100-tab session must not be truncated to 64.
        let paths: Vec<String> = (0..100).map(|i| format!("/files/f{i}.txt")).collect();
        let cleaned = ui_state::clean_string_list(paths.clone(), ui_state::SESSION_MAX_PATHS);
        assert_eq!(cleaned.len(), 100);
        assert_eq!(cleaned, paths);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_open_of_same_file_installs_one_tab_and_one_wal() {
        let dir = scratch_dir("open-dedup");
        let path = scratch_file_in(&dir, "same.txt", b"same\n");
        let cache = dir.join("cache");
        let state = Arc::new(AppState::new(None, wal_opts(&cache)));

        let a = {
            let state = state.clone();
            let path = path.clone();
            tokio::spawn(async move { state.open_path(path.to_string_lossy().to_string()).await })
        };
        let b = {
            let state = state.clone();
            let path = path.clone();
            tokio::spawn(async move { state.open_path(path.to_string_lossy().to_string()).await })
        };

        a.await.unwrap().unwrap();
        b.await.unwrap().unwrap();

        let tabs = state.tabs_response();
        assert_eq!(
            tabs.tabs.len(),
            1,
            "duplicate tab opened: {}",
            tabs.tabs.len()
        );
        assert!(ayame_core::wal::wal_path_for(&cache, &path).exists());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn close_tab_keeps_wal_when_same_wal_path_is_still_open() {
        let dir = scratch_dir("close-wal-guard");
        let path = scratch_file_in(&dir, "guard.txt", b"guard\n");
        let cache = dir.join("cache");
        let opts = wal_opts(&cache);
        let doc = Document::open(&path, &opts).unwrap();
        let state = AppState::new(Some(doc), opts);
        let wal_path = ayame_core::wal::wal_path_for(&cache, &path);
        assert!(wal_path.exists());

        let (active_id, duplicate_id) = state.write(|ws| {
            let active_id = ws.tabs.active.unwrap();
            let duplicate_id = ws.tabs.next_id;
            ws.tabs.next_id += 1;
            ws.tabs.order.push(duplicate_id);
            ws.tabs.inactive.insert(
                duplicate_id,
                tabs::InactiveTab {
                    doc: ws.doc.as_ref().unwrap().clone(),
                    edits: EditSession::default(),
                    markers: MarkerSession::default(),
                    aside_files: Vec::new(),
                    recoverable: None,
                    disk_baseline: None,
                },
            );
            (active_id, duplicate_id)
        });

        state.close_tab(active_id).await;

        assert!(wal_path.exists(), "live duplicate lost its WAL");
        assert_eq!(state.read(|ws| ws.tabs.active), Some(duplicate_id));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn close_tab_focuses_neighbor_and_removes_asides() {
        let dir = scratch_dir("close-focus-aside");
        let a = scratch_file_in(&dir, "a.txt", b"a\n");
        let b = scratch_file_in(&dir, "b.txt", b"b\n");
        let c = scratch_file_in(&dir, "c.txt", b"c\n");
        let state = AppState::new(
            Some(Document::open(&a, &OpenOptions::default()).unwrap()),
            OpenOptions::default(),
        );
        state
            .open_path(b.to_string_lossy().to_string())
            .await
            .unwrap();
        state
            .open_path(c.to_string_lossy().to_string())
            .await
            .unwrap();

        let ids = state.tabs_response().tabs;
        let aid = ids.iter().find(|t| t.name == "a.txt").unwrap().id;
        let bid = ids.iter().find(|t| t.name == "b.txt").unwrap().id;
        let cid = ids.iter().find(|t| t.name == "c.txt").unwrap().id;
        state.switch_tab(bid).await.unwrap();

        let active_aside = super::super::workspace::aside_path(&b);
        std::fs::write(&active_aside, b"old b\n").unwrap();
        state.write(|ws| ws.aside_files.push(active_aside.clone()));
        state.close_tab(bid).await;
        assert_eq!(state.read(|ws| ws.tabs.active), Some(cid));
        assert!(!active_aside.exists(), "active aside was not removed");

        let inactive_aside = super::super::workspace::aside_path(&a);
        std::fs::write(&inactive_aside, b"old a\n").unwrap();
        state.write(|ws| {
            ws.tabs
                .inactive
                .get_mut(&aid)
                .unwrap()
                .aside_files
                .push(inactive_aside.clone());
        });
        state.close_tab(aid).await;
        assert_eq!(state.read(|ws| ws.tabs.active), Some(cid));
        assert!(!inactive_aside.exists(), "inactive aside was not removed");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reorder_tab_changes_only_the_visible_order() {
        let dir = scratch_dir("tab-reorder");
        let a = scratch_file_in(&dir, "a.txt", b"a\n");
        let b = scratch_file_in(&dir, "b.txt", b"b\n");
        let c = scratch_file_in(&dir, "c.txt", b"c\n");
        let state = AppState::new(
            Some(Document::open(&a, &OpenOptions::default()).unwrap()),
            OpenOptions::default(),
        );
        state
            .open_path(b.to_string_lossy().to_string())
            .await
            .unwrap();
        state
            .open_path(c.to_string_lossy().to_string())
            .await
            .unwrap();

        let tabs = state.tabs_response().tabs;
        let aid = tabs.iter().find(|tab| tab.name == "a.txt").unwrap().id;
        let bid = tabs.iter().find(|tab| tab.name == "b.txt").unwrap().id;
        let cid = tabs.iter().find(|tab| tab.name == "c.txt").unwrap().id;
        assert_eq!(state.read(|ws| ws.tabs.active), Some(cid));

        state.reorder_tab(cid, Some(aid)).await.unwrap();
        let names = state
            .tabs_response()
            .tabs
            .into_iter()
            .map(|tab| tab.name)
            .collect::<Vec<_>>();
        assert_eq!(names, ["c.txt", "a.txt", "b.txt"]);
        assert_eq!(state.read(|ws| ws.tabs.active), Some(cid));

        state.reorder_tab(aid, None).await.unwrap();
        let ids = state
            .tabs_response()
            .tabs
            .into_iter()
            .map(|tab| tab.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, [cid, bid, aid]);
        assert_eq!(state.read(|ws| ws.tabs.active), Some(cid));

        let before_invalid = state.read(|ws| ws.tabs.order.clone());
        assert!(state.reorder_tab(cid, Some(u64::MAX)).await.is_err());
        assert_eq!(state.read(|ws| ws.tabs.order.clone()), before_invalid);

        let _ = std::fs::remove_dir_all(dir);
    }
}
