use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard};

use axum::http::StatusCode;
use ayame_core::{Document, EditSession, OpenOptions};
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
                self.tabs.inactive.insert(aid, InactiveTab { doc, edits });
            }
        }
    }

    /// Make `doc` a brand-new tab and focus it (used by open / new / upload).
    fn install_new_tab(&mut self, doc: Shared) {
        self.park_active();
        let id = self.tabs.next_id;
        self.tabs.next_id += 1;
        self.tabs.order.push(id);
        self.tabs.active = Some(id);
        self.doc = Some(doc);
        self.edits = EditSession::default();
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
        self.write(|ws| {
            let Some(idx) = ws.tabs.order.iter().position(|x| *x == id) else {
                return;
            };
            ws.tabs.order.remove(idx);
            ws.tabs.inactive.remove(&id);
            if ws.tabs.active != Some(id) {
                return; // closed a background tab; active state untouched
            }
            // The closed tab was active — pick the neighbor at the same slot.
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
                }
                None => {
                    ws.doc = None;
                    ws.edits = EditSession::default();
                }
            }
        });
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
            })
        })
    }

    /// The active document plus a clone of the edit overlay ONLY when it is
    /// dirty — what worker materialization and diff need. Clean sessions skip
    /// the copy entirely.
    pub(super) fn doc_and_dirty_edits(
        &self,
    ) -> Result<(Shared, Option<EditSession>), (StatusCode, String)> {
        self.read(|ws| {
            let (doc, edits) = ws.doc_and_edits()?;
            let dirty = edits.is_dirty().then(|| edits.clone());
            Ok((doc.clone(), dirty))
        })
    }

    pub(super) fn open_options(&self) -> OpenOptions {
        self.open_opts.clone()
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
    pub(super) fn mark_edits_saved(&self) {
        self.write(|ws| ws.edits = EditSession::default());
        self.invalidate_dirty_snapshot(); // clean session: the snapshot temp can go
    }

    /// Open `path` (blocking work off the async runtime) and install it in the
    /// detached active slot. Used by the in-place save to reload the saved
    /// file, and to restore the original document when the replace failed.
    /// Never touches the edit overlay. Caller must hold the transitions lock.
    pub(super) async fn install_reloaded(&self, path: PathBuf) -> Result<(), (StatusCode, String)> {
        let opts = self.open_opts.clone();
        let p = path.clone();
        let doc = tokio::task::spawn_blocking(move || Document::open(&p, &opts))
            .await
            .map_err(internal)?
            .map_err(|e| bad_request(format!("reopening '{}': {e}", path.display())))?;
        self.write(|ws| ws.doc = Some(Arc::new(doc)));
        self.invalidate_dirty_snapshot(); // the doc identity changed
        Ok(())
    }

    /// Open `path` with the workspace's options and make it the active document
    /// in a brand-new tab. The blocking open/index runs off the async runtime;
    /// the install itself is one atomic workspace mutation, serialized against
    /// other transitions.
    pub(super) async fn open_path(&self, path: String) -> Result<(), (StatusCode, String)> {
        let opts = self.open_opts.clone();
        let p = path.clone();
        let doc = tokio::task::spawn_blocking(move || Document::open(&p, &opts))
            .await
            .map_err(internal)?
            .map_err(|e| bad_request(format!("opening '{path}': {e}")))?;
        let _transitions = self.transitions.lock().await;
        self.write(|ws| ws.install_new_tab(Arc::new(doc)));
        self.invalidate_dirty_snapshot(); // a different tab is active now
        Ok(())
    }
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
