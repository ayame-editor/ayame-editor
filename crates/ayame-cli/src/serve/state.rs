use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard};

use axum::http::StatusCode;
use ayame_core::{Document, EditSession, Encoding, OpenOptions, TailRefresh};
use serde::Serialize;

use super::ops::WorkerInput;
use super::{bad_request, internal};

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

/// State of a tab that is open but not currently focused. The focused tab's
/// document and edits live in `Workspace::doc`/`edits` so every existing
/// endpoint keeps operating on "the active document" unchanged; switching tabs
/// just swaps that live state with an entry here.
struct InactiveTab {
    doc: Shared,
    edits: EditSession,
    /// Aside files (see [`Workspace::aside_files`]) travelling with the tab.
    aside_files: Vec<PathBuf>,
}

#[derive(Default)]
struct TabList {
    order: Vec<u64>,
    active: Option<u64>,
    inactive: HashMap<u64, InactiveTab>,
    next_id: u64,
}

/// Everything mutable about the workspace: the active document, its edit
/// overlay, and the tab list. Kept in ONE struct behind ONE lock so no request
/// can ever observe a document paired with another document's edits.
pub(super) struct Workspace {
    doc: Option<Shared>,
    pub(super) edits: EditSession,
    /// Aside files left behind by in-place saves of the ACTIVE document: the
    /// pre-save file is renamed to a hidden sibling so the live mmap keeps its
    /// inode readable. Each save best-effort deletes the accumulated entries
    /// (only a still-mapped file on Windows survives that); whatever remains
    /// is removed when the document is closed or replaced, and on shutdown.
    aside_files: Vec<PathBuf>,
    tabs: TabList,
}

impl Workspace {
    /// The active document, if any.
    pub(super) fn doc(&self) -> Option<&Shared> {
        self.doc.as_ref()
    }

    /// The active document and its edit overlay, or a 409 when the workspace
    /// is empty.
    pub(super) fn doc_and_edits(&self) -> Result<(&Shared, &EditSession), (StatusCode, String)> {
        match &self.doc {
            Some(doc) => Ok((doc, &self.edits)),
            None => Err(no_document()),
        }
    }

    /// Like [`Workspace::doc_and_edits`] but with a mutable overlay, split so
    /// a handler can mutate the edits while reading the document.
    pub(super) fn doc_and_edits_mut(
        &mut self,
    ) -> Result<(&Shared, &mut EditSession), (StatusCode, String)> {
        let Workspace { doc, edits, .. } = self;
        match doc {
            Some(doc) => Ok((doc, edits)),
            None => Err(no_document()),
        }
    }

    /// Park the currently active tab's live state so a different tab can take
    /// over the `doc`/`edits` slots.
    fn park_active(&mut self) {
        if let Some(aid) = self.tabs.active {
            if let Some(doc) = self.doc.clone() {
                let edits = std::mem::take(&mut self.edits);
                let aside_files = std::mem::take(&mut self.aside_files);
                self.tabs.inactive.insert(
                    aid,
                    InactiveTab {
                        doc,
                        edits,
                        aside_files,
                    },
                );
            }
        }
    }

    /// Make `doc` a brand-new tab and focus it (used by open / new / upload).
    /// Returns aside files orphaned by the transition (an active tab whose
    /// document was gone cannot be parked); the caller deletes them outside
    /// the workspace lock.
    fn install_new_tab(&mut self, doc: Shared) -> Vec<PathBuf> {
        self.park_active();
        let orphaned = std::mem::take(&mut self.aside_files);
        let id = self.tabs.next_id;
        self.tabs.next_id += 1;
        self.tabs.order.push(id);
        self.tabs.active = Some(id);
        self.doc = Some(doc);
        self.edits = EditSession::default();
        orphaned
    }
}

fn no_document() -> (StatusCode, String) {
    (
        StatusCode::CONFLICT,
        "no file is open — open one first".to_string(),
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
    /// Content generation the snapshot carries — what
    /// [`AppState::commit_in_place_save`] records as "on disk" after the
    /// staged bytes land, so a save stays correct even if edits (or undo)
    /// arrive between the commit-point validation and the marker update.
    content_gen: u64,
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
}

impl AppState {
    pub(super) fn new(doc: Option<Document>, open_opts: OpenOptions) -> AppState {
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
        AppState {
            ws: RwLock::new(Workspace {
                doc: shared,
                edits: EditSession::default(),
                aside_files: Vec::new(),
                tabs,
            }),
            transitions: tokio::sync::Mutex::new(()),
            open_opts,
            find_snapshot: Mutex::new(None),
            snapshot_builds: AtomicU64::new(0),
        }
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

    /// Focus an already-open tab.
    pub(super) async fn switch_tab(&self, id: u64) -> Result<(), (StatusCode, String)> {
        let _transitions = self.transitions.lock().await;
        // Leaf lock, taken while no ws guard is held (see `find_snapshot`).
        self.invalidate_dirty_snapshot();
        self.write(|ws| {
            if ws.tabs.active == Some(id) {
                return Ok(());
            }
            if !ws.tabs.order.contains(&id) {
                return Err(bad_request("no such tab"));
            }
            ws.park_active();
            ws.tabs.active = Some(id);
            match ws.tabs.inactive.remove(&id) {
                Some(t) => {
                    ws.doc = Some(t.doc);
                    ws.edits = t.edits;
                    ws.aside_files = t.aside_files;
                }
                None => {
                    // The tab exists but its state is gone (e.g. a failed
                    // in-place save left it unloaded). Show it as empty rather
                    // than leaking the previous tab's document into it.
                    ws.doc = None;
                    ws.edits = EditSession::default();
                }
            }
            Ok(())
        })
    }

    /// Close a tab; if it was active, focus a neighbor (or empty the workspace).
    pub(super) async fn close_tab(&self, id: u64) {
        let _transitions = self.transitions.lock().await;
        self.invalidate_dirty_snapshot();
        let asides = self.write(|ws| {
            let Some(idx) = ws.tabs.order.iter().position(|x| *x == id) else {
                return Vec::new();
            };
            ws.tabs.order.remove(idx);
            let mut asides = ws
                .tabs
                .inactive
                .remove(&id)
                .map(|t| t.aside_files)
                .unwrap_or_default();
            if ws.tabs.active != Some(id) {
                return asides; // closed a background tab; active state untouched
            }
            // The closed tab was active: its document goes away with it.
            asides.append(&mut ws.aside_files);
            // Pick the neighbor at the same slot.
            let next = ws
                .tabs
                .order
                .get(idx)
                .or_else(|| ws.tabs.order.last())
                .copied();
            ws.tabs.active = next;
            match next.and_then(|nid| ws.tabs.inactive.remove(&nid)) {
                Some(t) => {
                    ws.doc = Some(t.doc);
                    ws.edits = t.edits;
                    ws.aside_files = t.aside_files;
                }
                None => {
                    ws.doc = None;
                    ws.edits = EditSession::default();
                }
            }
            asides
        });
        // The closed tab's document handle is gone (or going): its aside
        // files are deletable now. Outside the lock; failures are retried at
        // shutdown via the pid-scoped sweep on the next open.
        remove_aside_files(asides);
    }

    /// Snapshot every open tab for the tab bar.
    pub(super) fn tabs_response(&self) -> TabsResponse {
        self.read(|ws| {
            let active = ws.tabs.active;
            let mut out = Vec::with_capacity(ws.tabs.order.len());
            for &id in &ws.tabs.order {
                let (path, dirty) = if Some(id) == active {
                    match &ws.doc {
                        Some(d) => (d.path().to_path_buf(), ws.edits.stats(d).dirty),
                        None => continue,
                    }
                } else {
                    match ws.tabs.inactive.get(&id) {
                        Some(t) => (t.doc.path().to_path_buf(), t.edits.stats(&t.doc).dirty),
                        None => continue,
                    }
                };
                let path = path.to_string_lossy().to_string();
                out.push(TabInfo {
                    id,
                    name: tab_name(&path),
                    path,
                    dirty,
                    active: Some(id) == active,
                });
            }
            TabsResponse { tabs: out }
        })
    }

    /// The open document, if any (cheap `Arc` clone).
    pub(super) fn doc_opt(&self) -> Option<Shared> {
        self.read(|ws| ws.doc.clone())
    }

    /// Owned snapshot of the active document + edits + revision, taken under
    /// one lock acquisition. The `EditSession` clone here is the one clone
    /// that is genuinely needed (long saves must not hold the lock).
    pub(super) fn edit_snapshot(&self) -> Result<EditSnapshot, (StatusCode, String)> {
        self.read(|ws| {
            let (doc, edits) = ws.doc_and_edits()?;
            Ok(EditSnapshot {
                doc: doc.clone(),
                edits: edits.clone(),
                revision: edits.revision(),
                content_gen: edits.content_gen(),
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
    pub(super) fn doc_and_dirty_edits(
        &self,
    ) -> Result<(Shared, Option<EditSession>), (StatusCode, String)> {
        self.read(|ws| {
            let (doc, edits) = ws.doc_and_edits()?;
            let dirty = (edits.has_edits() || edits.is_dirty()).then(|| edits.clone());
            Ok((doc.clone(), dirty))
        })
    }

    pub(super) fn open_options(&self) -> OpenOptions {
        self.open_opts.clone()
    }

    /// Poll the active document for appended data (`tail -f`). When it grew and
    /// the session has no pending edits, the line index is extended in place
    /// over just the new bytes; a shrink or external replacement reports
    /// `changed` so the client can reopen. Blocking (stat + mmap + scan), so
    /// callers should run it off the async runtime.
    pub(super) fn poll_tail(&self) -> TailStatus {
        // Cheap peek under a short read lock: an Arc handle plus whether the
        // overlay holds edits (we only follow a clean, unedited view — the
        // overlay's line anchors reference the original document).
        let (doc, has_edits) = self.read(|ws| match ws.doc() {
            Some(d) => (Some(d.clone()), ws.edits.has_edits()),
            None => (None, false),
        });
        let Some(doc) = doc else {
            return TailStatus::closed();
        };
        let known = doc.byte_len();
        let disk = match doc.disk_len() {
            Ok(n) => n,
            // A stat failure (the file vanished) reads as "changed externally".
            Err(_) => {
                let mut s = TailStatus::at(doc.line_count(), known);
                s.changed = true;
                return s;
            }
        };
        if disk == known {
            return TailStatus::at(doc.line_count(), known);
        }
        if disk < known || known == 0 {
            // Shrunk/rotated, or grown from empty (encoding never detected).
            let mut s = TailStatus::at(doc.line_count(), known);
            s.changed = true;
            return s;
        }
        // Growth detected. If edits are pending we do not follow — report it so
        // the client stops auto-scrolling into a view its overlay predates.
        if has_edits {
            let mut s = TailStatus::at(doc.line_count(), known);
            s.pending_edits = true;
            return s;
        }
        // Release our extra handle and the find snapshot's handle so the live
        // document becomes uniquely owned and can be extended in place. (The
        // find snapshot is a leaf lock, taken here with no ws guard held.)
        drop(doc);
        self.invalidate_dirty_snapshot();
        self.write(|ws| {
            let Some(arc) = ws.doc.as_mut() else {
                return TailStatus::closed();
            };
            match Arc::get_mut(arc) {
                Some(doc) => match doc.refresh_tail() {
                    Ok(TailRefresh::Grew) => {
                        let mut s = TailStatus::at(doc.line_count(), doc.byte_len());
                        s.grew = true;
                        s
                    }
                    Ok(TailRefresh::Reindex) => {
                        let mut s = TailStatus::at(doc.line_count(), doc.byte_len());
                        s.changed = true;
                        s
                    }
                    Ok(TailRefresh::Unchanged) | Err(_) => {
                        TailStatus::at(doc.line_count(), doc.byte_len())
                    }
                },
                // An in-flight op still holds a handle: leave it for next tick.
                None => TailStatus::at(arc.line_count(), arc.byte_len()),
            }
        })
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
    pub(super) fn detach_for_overwrite(
        &self,
        snap: &EditSnapshot,
    ) -> Result<(), (StatusCode, String)> {
        self.write(|ws| {
            let same_doc = ws.doc.as_ref().is_some_and(|d| Arc::ptr_eq(d, &snap.doc));
            if !same_doc || ws.edits.revision() != snap.revision {
                return Err((
                    StatusCode::CONFLICT,
                    "the document changed while saving — nothing was overwritten; save again"
                        .to_string(),
                ));
            }
            ws.doc = None; // edits stay: they are restored if the replace fails
            Ok(())
        })
    }

    /// The staged bytes reached the target file: the pending edits are now on
    /// disk, so clear the overlay (the reloaded document IS the edited text).
    /// Used by full-reload commits (in-place sort); the in-place SAVE keeps
    /// the overlay and history via [`AppState::commit_in_place_save`] instead.
    pub(super) fn mark_edits_saved(&self) {
        self.write(|ws| ws.edits = EditSession::default());
        self.invalidate_dirty_snapshot(); // clean session: the snapshot temp can go
    }

    /// Commit point of an in-place save WITHOUT a reload: verify the workspace
    /// still matches `snap` (same document identity, same edit revision) so
    /// the staged bytes are known to be the current view. The document, the
    /// overlay and the undo/redo history stay untouched — that is what lets
    /// undo cross the save. Caller must hold the transitions lock and, on Ok,
    /// finish with [`AppState::commit_in_place_save`] after the file swap.
    pub(super) fn confirm_overwrite(
        &self,
        snap: &EditSnapshot,
    ) -> Result<(), (StatusCode, String)> {
        self.read(|ws| {
            let same_doc = ws.doc.as_ref().is_some_and(|d| Arc::ptr_eq(d, &snap.doc));
            if !same_doc || ws.edits.revision() != snap.revision {
                return Err((
                    StatusCode::CONFLICT,
                    "the document changed while saving — nothing was overwritten; save again"
                        .to_string(),
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
        let pending = self.write(|ws| {
            ws.edits.mark_saved_at(snap.content_gen);
            if let Some(aside) = aside {
                ws.aside_files.push(aside);
            }
            std::mem::take(&mut ws.aside_files)
        });
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
    pub(super) async fn install_reloaded(&self, path: PathBuf) -> Result<(), (StatusCode, String)> {
        let opts = self.open_opts.clone();
        let p = path.clone();
        let doc = tokio::task::spawn_blocking(move || Document::open(&p, &opts))
            .await
            .map_err(internal)?
            .map_err(|e| bad_request(format!("reopening '{}': {e}", path.display())))?;
        let asides = self.write(|ws| {
            ws.doc = Some(Arc::new(doc));
            std::mem::take(&mut ws.aside_files)
        });
        remove_aside_files(asides);
        self.invalidate_dirty_snapshot(); // the doc identity changed
        Ok(())
    }

    /// Revert the active document to its last SAVED state: reload from disk
    /// (the file on disk IS the saved state, in-place saves included) with a
    /// fresh, clean edit session, and drop the old document's aside files.
    /// Caller must hold the transitions lock.
    pub(super) async fn reload_reverted(&self, path: PathBuf) -> Result<(), (StatusCode, String)> {
        let opts = self.open_opts.clone();
        let p = path.clone();
        let doc = tokio::task::spawn_blocking(move || Document::open(&p, &opts))
            .await
            .map_err(internal)?
            .map_err(|e| bad_request(format!("reopening '{}': {e}", path.display())))?;
        let asides = self.write(|ws| {
            ws.doc = Some(Arc::new(doc));
            ws.edits = EditSession::default();
            std::mem::take(&mut ws.aside_files)
        });
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
    ) -> Result<(), (StatusCode, String)> {
        let mut opts = self.open_opts.clone();
        opts.encoding = Some(enc);
        let p = path.clone();
        let doc = tokio::task::spawn_blocking(move || Document::open(&p, &opts))
            .await
            .map_err(internal)?
            .map_err(|e| {
                bad_request(format!(
                    "reopening '{}' as {}: {e}",
                    path.display(),
                    enc.label()
                ))
            })?;
        let asides = self.write(|ws| {
            ws.doc = Some(Arc::new(doc));
            ws.edits = EditSession::default();
            std::mem::take(&mut ws.aside_files)
        });
        remove_aside_files(asides);
        self.invalidate_dirty_snapshot();
        Ok(())
    }

    /// Open `path` with the workspace's options and make it the active document
    /// in a brand-new tab. The blocking open/index runs off the async runtime;
    /// the install itself is one atomic workspace mutation, serialized against
    /// other transitions. Stale aside files of `path` (crash leftovers from
    /// any previous session) are swept before opening.
    ///
    /// A path that is ALREADY open in some tab focuses that tab instead of
    /// opening a duplicate — clicking a file in the explorer means "go to that
    /// file", the same as every tabbed editor.
    pub(super) async fn open_path(&self, path: String) -> Result<(), (StatusCode, String)> {
        if let Some(id) = self.tab_with_path(Path::new(&path)).await {
            return self.switch_tab(id).await;
        }
        let opts = self.open_opts.clone();
        let p = path.clone();
        let doc = tokio::task::spawn_blocking(move || {
            super::workspace::sweep_stale_asides(Path::new(&p));
            Document::open(&p, &opts)
        })
        .await
        .map_err(internal)?
        .map_err(|e| bad_request(format!("opening '{path}': {e}")))?;
        let _transitions = self.transitions.lock().await;
        let orphaned = self.write(|ws| ws.install_new_tab(Arc::new(doc)));
        remove_aside_files(orphaned);
        self.invalidate_dirty_snapshot(); // a different tab is active now
        Ok(())
    }

    /// The tab (if any) whose document is `path`, compared literally first and
    /// then by canonical path so `C:\x\f.txt` and `\\?\C:\x\f.txt`, or a path
    /// reached through a symlink, still match. Filesystem calls run off the
    /// async runtime and never under the workspace lock.
    async fn tab_with_path(&self, path: &Path) -> Option<u64> {
        let target = path.to_path_buf();
        let tabs: Vec<(u64, PathBuf)> = self.read(|ws| {
            ws.tabs
                .order
                .iter()
                .filter_map(|&id| {
                    let doc = if ws.tabs.active == Some(id) {
                        ws.doc.as_ref()
                    } else {
                        ws.tabs.inactive.get(&id).map(|t| &t.doc)
                    }?;
                    Some((id, doc.path().to_path_buf()))
                })
                .collect()
        });
        if tabs.is_empty() {
            return None;
        }
        tokio::task::spawn_blocking(move || {
            let canon = std::fs::canonicalize(&target).ok();
            tabs.into_iter()
                .find(|(_, p)| {
                    *p == target
                        || canon
                            .as_deref()
                            .is_some_and(|c| std::fs::canonicalize(p).is_ok_and(|pc| pc == c))
                })
                .map(|(id, _)| id)
        })
        .await
        .ok()
        .flatten()
    }

    /// Best-effort deletion of every tracked aside file (all tabs), for the
    /// graceful-shutdown path. Failures are ignored — mapped files refuse
    /// deletion on Windows; the on-open sweep collects them next time.
    pub(super) fn cleanup_aside_files(&self) {
        let all = self.write(|ws| {
            let mut v = std::mem::take(&mut ws.aside_files);
            for tab in ws.tabs.inactive.values_mut() {
                v.append(&mut tab.aside_files);
            }
            v
        });
        remove_aside_files(all);
    }
}

/// Best-effort deletion of aside files; returns the ones that still exist but
/// could not be deleted (e.g. mapped on Windows) so the caller can keep
/// tracking them.
fn remove_aside_files(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    paths
        .into_iter()
        .filter(|p| std::fs::remove_file(p).is_err() && p.exists())
        .collect()
}

/// Friendly tab label: "untitled" for scratch buffers, else the file's basename.
fn tab_name(path: &str) -> String {
    let basename = Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string());
    if path.contains("ayame-untitled-") {
        return if basename == "untitled.txt" {
            "untitled".to_string()
        } else {
            basename
        };
    }
    basename
}

/// Result of a `tail -f` poll on the active document. `lines`/`bytes` are the
/// current totals so the client can grow its scrollbar even when it decides not
/// to auto-scroll.
#[derive(Serialize)]
pub(super) struct TailStatus {
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
    pub(super) fn closed() -> TailStatus {
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

#[derive(Serialize)]
struct TabInfo {
    id: u64,
    name: String,
    path: String,
    dirty: bool,
    active: bool,
}

#[derive(Serialize)]
pub(super) struct TabsResponse {
    tabs: Vec<TabInfo>,
}

pub(crate) type SharedState = Arc<AppState>;
