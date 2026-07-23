use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

use axum::http::StatusCode;
use ayame_core::wal::{self, WalCompactionPlan, WalWriter};
use ayame_core::{Document, EditSession, EditStats, Encoding, OpenOptions};
use serde::{Deserialize, Serialize};

use super::analysis::{AnalysisProfile, AnalysisStore};
use super::markers::MarkerSession;
use super::ops::WorkerInput;
use super::{bad_request, internal, ApiError};

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

/// State of a tab that is open but not currently focused. The focused tab's
/// document and edits live in `Workspace::doc`/`edits` so every existing
/// endpoint keeps operating on "the active document" unchanged; switching tabs
/// just swaps that live state with an entry here.
struct InactiveTab {
    doc: Shared,
    edits: EditSession,
    markers: MarkerSession,
    /// Aside files (see [`Workspace::aside_files`]) travelling with the tab.
    aside_files: Vec<PathBuf>,
    /// Pending crash-recovery decision travelling with the tab (see
    /// [`Workspace::recoverable`]).
    recoverable: Option<usize>,
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
    tabs: TabList,
}

impl Workspace {
    /// The active document, if any.
    pub(super) fn doc(&self) -> Option<&Shared> {
        self.doc.as_ref()
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

    /// Park the currently active tab's live state so a different tab can take
    /// over the `doc`/`edits` slots.
    fn park_active(&mut self) {
        if let Some(aid) = self.tabs.active {
            if let Some(doc) = self.doc.clone() {
                // Parked sessions carry NO crash-log writer: exactly one live
                // logger exists per file, and it belongs to the focused tab.
                // The log file itself stays on disk with everything committed
                // so far; re-selecting the tab re-attaches and snapshots.
                self.edits.set_wal(None);
                let edits = std::mem::take(&mut self.edits);
                let markers = std::mem::take(&mut self.markers);
                let aside_files = std::mem::take(&mut self.aside_files);
                let recoverable = self.recoverable.take();
                self.tabs.inactive.insert(
                    aid,
                    InactiveTab {
                        doc,
                        edits,
                        markers,
                        aside_files,
                        recoverable,
                    },
                );
            }
        }
    }

    /// Make `doc` a brand-new tab and focus it (used by open / new / upload).
    /// Returns aside files orphaned by the transition (an active tab whose
    /// document was gone cannot be parked); the caller deletes them outside
    /// the workspace lock.
    fn focus_tab(&mut self, id: u64, cache_root: Option<&Path>) -> Result<(), ApiError> {
        if self.tabs.active == Some(id) {
            return Ok(());
        }
        if !self.tabs.order.contains(&id) {
            return Err(bad_request("no such tab"));
        }
        self.park_active();
        self.tabs.active = Some(id);
        match self.tabs.inactive.remove(&id) {
            Some(t) => {
                self.doc = Some(t.doc);
                self.edits = t.edits;
                self.markers = t.markers;
                self.aside_files = t.aside_files;
                self.recoverable = t.recoverable;
            }
            None => {
                self.doc = None;
                self.edits = EditSession::default();
                self.markers = MarkerSession::default();
                self.recoverable = None;
            }
        }
        attach_live_wal(cache_root, self);
        Ok(())
    }

    fn tab_with_path_literal(&self, path: &Path) -> Option<u64> {
        self.tabs.order.iter().copied().find(|&id| {
            let doc = if self.tabs.active == Some(id) {
                self.doc.as_ref()
            } else {
                self.tabs.inactive.get(&id).map(|t| &t.doc)
            };
            doc.is_some_and(|doc| doc.path() == path)
        })
    }

    fn wal_path_in_use(&self, cache_root: &Path, wal_path: &Path) -> bool {
        let active = self
            .doc
            .as_ref()
            .is_some_and(|doc| wal::wal_path_for(cache_root, doc.path()) == wal_path);
        active
            || self
                .tabs
                .inactive
                .values()
                .any(|tab| wal::wal_path_for(cache_root, tab.doc.path()) == wal_path)
    }

    fn install_new_tab(
        &mut self,
        doc: Shared,
        wal: WalSetup,
        cache_root: Option<&Path>,
    ) -> Vec<PathBuf> {
        self.park_active();
        let orphaned = std::mem::take(&mut self.aside_files);
        let id = self.tabs.next_id;
        self.tabs.next_id += 1;
        self.tabs.order.push(id);
        self.tabs.active = Some(id);
        self.doc = Some(doc);
        self.edits = EditSession::default();
        self.markers = MarkerSession::default();
        self.recoverable = None;
        match wal {
            WalSetup::Attach(w) => self.edits.set_wal(Some(*w)),
            WalSetup::Create => attach_live_wal(cache_root, self),
            WalSetup::Recoverable(n) => self.recoverable = Some(n),
            WalSetup::Off => {}
        }
        orphaned
    }
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
fn attach_live_wal(cache_root: Option<&Path>, ws: &mut Workspace) {
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

struct TailPollSnapshot {
    doc: Shared,
    revision: u64,
    has_edits: bool,
    known_bytes: u64,
    known_lines: u64,
}

struct WalPolicyWork {
    doc: Shared,
    revision: u64,
    sync_file: Option<std::fs::File>,
    compact: Option<(EditSession, WalCompactionPlan)>,
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
    /// First crash-log failure not yet shown to the user. Filled by the WAL
    /// policy tick (which consumes [`EditSession::take_wal_error`]) or by a
    /// failed post-save log reset; drained once by the next stat response.
    /// LEAF lock, same discipline as `find_snapshot`.
    wal_error: Mutex<Option<String>>,
    /// Bounded multi-rule log-analysis operations. Each operation pins one
    /// immutable document/edit generation and is evicted after four entries.
    analysis: AnalysisStore,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct SessionState {
    pub(super) paths: Vec<String>,
    pub(super) active_path: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct UiState {
    pub(super) recent_files: Vec<String>,
    pub(super) search_history: Vec<String>,
    pub(super) session: SessionState,
    #[serde(default)]
    pub(super) analysis_profiles: Vec<AnalysisProfile>,
    #[serde(default)]
    pub(super) active_analysis_profile: Option<String>,
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
                doc: shared,
                edits,
                markers: MarkerSession::default(),
                aside_files: Vec::new(),
                recoverable,
                tabs,
            }),
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

    fn ui_state_path(&self) -> Option<PathBuf> {
        self.open_opts
            .cache_dir
            .as_ref()
            .map(|d| d.join("ui-state.json"))
    }

    pub(super) fn load_ui_state(&self) -> UiState {
        let Some(path) = self.ui_state_path() else {
            return UiState::default();
        };
        let Ok(bytes) = std::fs::read(path) else {
            return UiState::default();
        };
        let mut ui: UiState = serde_json::from_slice(&bytes).unwrap_or_default();
        (ui.analysis_profiles, ui.active_analysis_profile) =
            super::analysis::sanitize_persisted_profiles(
                ui.analysis_profiles,
                ui.active_analysis_profile,
            );
        ui
    }

    pub(super) fn save_ui_state(&self, mut ui: UiState) -> Result<UiState, ApiError> {
        ui.recent_files = clean_path_list(ui.recent_files, 24);
        ui.search_history = clean_string_list(ui.search_history, 50);
        ui.session.paths = clean_path_list(ui.session.paths, SESSION_MAX_PATHS);
        if let Some(active) = ui.session.active_path.take() {
            ui.session.active_path = clean_one_string(active);
        }
        (ui.analysis_profiles, ui.active_analysis_profile) =
            super::analysis::sanitize_persisted_profiles(
                ui.analysis_profiles,
                ui.active_analysis_profile,
            );
        let Some(path) = self.ui_state_path() else {
            return Ok(ui);
        };
        let json = serde_json::to_vec_pretty(&ui).map_err(internal)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(internal)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json).map_err(internal)?;
        std::fs::rename(&tmp, &path).map_err(internal)?;
        Ok(ui)
    }

    pub(super) fn save_session_snapshot(&self) -> Result<UiState, ApiError> {
        let mut ui = self.load_ui_state();
        let tabs = self.tabs_response().tabs;
        // Scratch/untitled buffers can't be reopened next launch, so keep them
        // out of the snapshot (they would only consume slots and fail to open).
        ui.session.paths = tabs
            .iter()
            .map(|t| t.path.clone())
            .filter(|p| !super::workspace::is_scratch_path(p))
            .collect();
        ui.session.active_path = tabs
            .iter()
            .find(|t| t.active)
            .map(|t| t.path.clone())
            .filter(|p| !super::workspace::is_scratch_path(p));
        self.save_ui_state(ui)
    }

    pub(super) async fn restore_session(&self) -> Result<(), ApiError> {
        let session = self.load_ui_state().session;
        if session.paths.is_empty() {
            return Ok(());
        }
        let paths = clean_path_list(session.paths, SESSION_MAX_PATHS);
        for path in &paths {
            if let Err(e) = self.open_path(path.clone()).await {
                eprintln!("ayame: session restore skipped '{}': {}", path, e.message());
            }
        }
        if let Some(active) = session.active_path {
            let tabs = self.tabs_response().tabs;
            if let Some(tab) = tabs.iter().find(|t| t.path == active) {
                self.switch_tab(tab.id).await?;
            }
        }
        Ok(())
    }

    /// Record a crash-log failure for one-shot surfacing via stat.
    pub(super) fn note_wal_error(&self, msg: String) {
        let mut slot = self.wal_error.lock().unwrap_or_else(|p| p.into_inner());
        slot.get_or_insert(msg);
    }

    /// Drain the pending crash-log failure (shown once in the next stat).
    pub(super) fn take_wal_error(&self) -> Option<String> {
        self.wal_error
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
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
    pub(super) async fn switch_tab(&self, id: u64) -> Result<(), ApiError> {
        let _transitions = self.transitions.lock().await;
        // Leaf lock, taken while no ws guard is held (see `find_snapshot`).
        self.invalidate_dirty_snapshot();
        self.write(|ws| ws.focus_tab(id, self.open_opts.cache_dir.as_deref()))
    }

    /// Move an open tab before another tab, or to the end when `before_id` is
    /// absent. The active document and every inactive edit session stay put;
    /// only the user-visible order changes.
    pub(super) async fn reorder_tab(
        &self,
        id: u64,
        before_id: Option<u64>,
    ) -> Result<(), ApiError> {
        let _transitions = self.transitions.lock().await;
        self.write(|ws| {
            let Some(from) = ws.tabs.order.iter().position(|tab_id| *tab_id == id) else {
                return Err(bad_request("no such tab"));
            };
            if before_id == Some(id) {
                return Ok(());
            }
            if before_id.is_some_and(|before| !ws.tabs.order.contains(&before)) {
                return Err(bad_request("no such destination tab"));
            }

            ws.tabs.order.remove(from);
            let to = before_id
                .and_then(|before| ws.tabs.order.iter().position(|tab_id| *tab_id == before))
                .unwrap_or(ws.tabs.order.len());
            ws.tabs.order.insert(to, id);
            Ok(())
        })
    }

    /// Close a tab; if it was active, focus a neighbor (or empty the workspace).
    pub(super) async fn close_tab(&self, id: u64) {
        let _transitions = self.transitions.lock().await;
        self.invalidate_dirty_snapshot();
        let cache_root = self.open_opts.cache_dir.clone();
        let (asides, dead_wal) = self.write(|ws| {
            let Some(idx) = ws.tabs.order.iter().position(|x| *x == id) else {
                return (Vec::new(), None);
            };
            ws.tabs.order.remove(idx);
            // Closing a tab is a graceful discard of its unsaved edits (the
            // client confirms dirty closes first): its crash log goes with it
            // — UNLESS a recovery decision is still pending; that log belongs
            // to a previous process and stays until restored or discarded.
            let mut dead_wal: Option<PathBuf> = None;
            let mut wal_path_of = |doc: &Shared, recoverable: Option<usize>| {
                if recoverable.is_none() {
                    if let Some(root) = cache_root.as_deref() {
                        dead_wal = Some(wal::wal_path_for(root, doc.path()));
                    }
                }
            };
            let mut asides = match ws.tabs.inactive.remove(&id) {
                Some(t) => {
                    wal_path_of(&t.doc, t.recoverable);
                    t.aside_files
                }
                None => Vec::new(),
            };
            if ws.tabs.active != Some(id) {
                if let (Some(root), Some(path)) = (cache_root.as_deref(), dead_wal.as_deref()) {
                    if ws.wal_path_in_use(root, path) {
                        dead_wal = None;
                    }
                }
                return (asides, dead_wal); // closed a background tab; active state untouched
            }
            // The closed tab was active: its document goes away with it.
            if let Some(doc) = ws.doc.clone() {
                wal_path_of(&doc, ws.recoverable);
            }
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
                    // Replacing `edits` drops the closed tab's session — and
                    // with it any writer handle, so the log file is deletable.
                    ws.doc = Some(t.doc);
                    ws.edits = t.edits;
                    ws.markers = t.markers;
                    ws.aside_files = t.aside_files;
                    ws.recoverable = t.recoverable;
                }
                None => {
                    ws.doc = None;
                    ws.edits = EditSession::default();
                    ws.markers = MarkerSession::default();
                    ws.recoverable = None;
                }
            }
            // The neighbor is live now: re-attach its crash log.
            attach_live_wal(cache_root.as_deref(), ws);
            if let (Some(root), Some(path)) = (cache_root.as_deref(), dead_wal.as_deref()) {
                if ws.wal_path_in_use(root, path) {
                    dead_wal = None;
                }
            }
            (asides, dead_wal)
        });
        // The closed tab's document handle is gone (or going): its aside
        // files are deletable now. Outside the lock; failures are retried at
        // shutdown via the pid-scoped sweep on the next open.
        remove_aside_files(asides);
        if let Some(p) = dead_wal {
            let _ = std::fs::remove_file(p);
        }
    }

    /// Detach a tab for a window-to-window handoff (issue #35): remove it
    /// exactly like [`AppState::close_tab`], but KEEP its crash log on disk —
    /// fsynced before returning — so the adopting window can replay the
    /// unsaved edits through the normal `/api/edit/recover` path. Refused
    /// with 409 when the tab holds unsaved edits but has no crash log to
    /// carry them (`--no-cache`, or logging degraded): moving it would
    /// silently drop the edits, which is exactly what this endpoint exists to
    /// prevent.
    pub(super) async fn detach_tab(&self, id: u64) -> Result<(), ApiError> {
        let _transitions = self.transitions.lock().await;
        self.invalidate_dirty_snapshot();
        let cache_root = self.open_opts.cache_dir.clone();
        let (asides, sync_file) = self.write(|ws| {
            let Some(idx) = ws.tabs.order.iter().position(|x| *x == id) else {
                return Err(ApiError::new(
                    StatusCode::NOT_FOUND,
                    "not_found",
                    "no such tab",
                ));
            };
            // Clone the log's file handle before the session (and with it the
            // live writer) is dropped; a cloned handle syncs the same file.
            let (dirty, sync_file, recoverable) = if ws.tabs.active == Some(id) {
                let dirty = ws.doc.as_ref().is_some_and(|d| ws.edits.stats(d).dirty);
                (
                    dirty,
                    ws.edits.wal_sync_file().ok().flatten(),
                    ws.recoverable,
                )
            } else {
                match ws.tabs.inactive.get(&id) {
                    Some(t) => (
                        t.edits.stats(&t.doc).dirty,
                        t.edits.wal_sync_file().ok().flatten(),
                        t.recoverable,
                    ),
                    None => (false, None, None),
                }
            };
            if dirty && sync_file.is_none() && recoverable.is_none() {
                return Err(ApiError::new(
                    StatusCode::CONFLICT,
                    "conflict",
                    "unsaved edits have no crash log to hand off",
                ));
            }
            ws.tabs.order.remove(idx);
            let mut asides = match ws.tabs.inactive.remove(&id) {
                Some(t) => t.aside_files, // dropping `t` releases its writer handle
                None => Vec::new(),
            };
            if ws.tabs.active == Some(id) {
                asides.append(&mut ws.aside_files);
                // Same neighbor-focus dance as close_tab; replacing `edits`
                // drops the detached session's writer so the log file is free
                // for the adopting process.
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
                        ws.markers = t.markers;
                        ws.aside_files = t.aside_files;
                        ws.recoverable = t.recoverable;
                    }
                    None => {
                        ws.doc = None;
                        ws.edits = EditSession::default();
                        ws.markers = MarkerSession::default();
                        ws.recoverable = None;
                    }
                }
                attach_live_wal(cache_root.as_deref(), ws);
            }
            Ok((asides, sync_file))
        })?;
        // The handoff contract: the log is durable before the caller spawns
        // (or signals) the adopting window. Surfaced on failure — proceeding
        // could hand off a log a power loss would tear.
        if let Some(f) = sync_file {
            f.sync_data().map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("crash log sync failed: {e}"),
                )
            })?;
        }
        remove_aside_files(asides);
        Ok(())
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
                // UI-facing path (and the name derived from it): never leak a
                // Windows verbatim prefix.
                let path = super::workspace::display_path(&path);
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

    /// Poll the active document for appended data (`tail -f`). When it grew and
    /// the session has no pending edits, the refreshed document is opened off
    /// the workspace lock and installed only if the active tab and edit
    /// revision are still unchanged; a shrink or external replacement reports
    /// `changed` so the client can reopen. Blocking (stat + mmap + appended-byte
    /// scan), so
    /// callers should run it off the async runtime.
    pub(super) fn poll_tail(&self) -> TailStatus {
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
                        ws.doc = Some(Arc::new(new_doc.take().unwrap()));
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
            ws.doc = None; // edits stay: they are restored if the replace fails
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
        let opts = self.open_opts.clone();
        let p = path.clone();
        let doc = tokio::task::spawn_blocking(move || Document::open(&p, &opts))
            .await
            .map_err(internal)?
            .map_err(|e| {
                bad_request(format!(
                    "reopening '{}': {e}",
                    super::workspace::display_path(&path)
                ))
            })?;
        let asides = self.write(|ws| {
            ws.doc = Some(Arc::new(doc));
            // Fresh log for the reloaded base (in-place sort commits arrive
            // here with a just-cleared overlay; the failed-replace restore
            // path re-snapshots its still-pending edits).
            attach_live_wal(self.open_opts.cache_dir.as_deref(), ws);
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
        let opts = self.open_opts.clone();
        let p = path.clone();
        let doc = tokio::task::spawn_blocking(move || Document::open(&p, &opts))
            .await
            .map_err(internal)?
            .map_err(|e| {
                bad_request(format!(
                    "reopening '{}': {e}",
                    super::workspace::display_path(&path)
                ))
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
            ws.doc = Some(Arc::new(doc));
            ws.edits = EditSession::default();
            ws.markers = MarkerSession::default();
            // Reset: a clean session over the file as it now exists on disk
            // gets a fresh, empty log for the new base identity.
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
        let mut opts = self.open_opts.clone();
        opts.encoding = Some(enc);
        let p = path.clone();
        let doc = tokio::task::spawn_blocking(move || Document::open(&p, &opts))
            .await
            .map_err(internal)?
            .map_err(|e| {
                bad_request(format!(
                    "reopening '{}' as {}: {e}",
                    super::workspace::display_path(&path),
                    enc.label()
                ))
            })?;
        let asides = self.write(|ws| {
            ws.doc = Some(Arc::new(doc));
            ws.edits = EditSession::default();
            ws.markers = MarkerSession::default();
            // The encoding is part of the log's base identity: start a fresh
            // log recorded against the forced-encoding view.
            attach_live_wal(self.open_opts.cache_dir.as_deref(), ws);
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
    /// opening a duplicate — choosing an already-open file means "go to that
    /// file", the same as every tabbed editor.
    pub(super) async fn open_path(&self, path: String) -> Result<(), ApiError> {
        if let Some(id) = self.tab_with_path(Path::new(&path)).await {
            return self.switch_tab(id).await;
        }
        let opts = self.open_opts.clone();
        let p = path.clone();
        let (doc, wal_setup) = tokio::task::spawn_blocking(move || {
            super::workspace::sweep_stale_asides(Path::new(&p));
            let doc = Document::open(&p, &opts)?;
            // Detect crash leftovers before any fresh writer can truncate them.
            // The writer itself is created only after the transition-lock
            // duplicate recheck below.
            let wal_setup = wal_prepare_for_open(opts.cache_dir.as_deref(), &doc);
            Ok::<_, ayame_core::Error>((doc, wal_setup))
        })
        .await
        .map_err(internal)?
        .map_err(|e| {
            bad_request(format!(
                "opening '{}': {e}",
                super::workspace::strip_verbatim(&path)
            ))
        })?;
        let _transitions = self.transitions.lock().await;
        let target = doc.path().to_path_buf();
        let orphaned = self.write(|ws| -> Result<Vec<std::path::PathBuf>, ApiError> {
            if let Some(id) = ws.tab_with_path_literal(&target) {
                ws.focus_tab(id, self.open_opts.cache_dir.as_deref())?;
                return Ok(Vec::new());
            }
            Ok(ws.install_new_tab(
                Arc::new(doc),
                wal_setup,
                self.open_opts.cache_dir.as_deref(),
            ))
        })?;
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

    /// Graceful-shutdown crash-log policy: CLEAN sessions delete their log
    /// (nothing to recover; a fresh one is created on the next open), DIRTY
    /// sessions leave it in place — it is the recovery artifact the next
    /// process will offer to replay. Undecided recoverable logs stay too.
    pub(super) fn cleanup_wal_files(&self) {
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
    pub(super) fn wal_policy_tick(&self) {
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
    pub(super) async fn recover_wal(&self, discard: bool) -> Result<(EditStats, usize), ApiError> {
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
    // Current scratch dirs are "ayame-srv-untitled-…"; restored sessions may
    // still carry the pre-rename "ayame-untitled-<pid>" form.
    if super::workspace::is_scratch_path(path) {
        return if basename == "untitled.txt" {
            "untitled".to_string()
        } else {
            basename
        };
    }
    basename
}

/// How many open files a saved session may carry. Comfortably above any
/// realistic tab count so restoring ~100 tabs never silently drops the tail.
const SESSION_MAX_PATHS: usize = 512;

fn clean_path_list(list: Vec<String>, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    for value in list {
        let Some(value) = clean_one_string(value) else {
            continue;
        };
        if !out.iter().any(|x| x == &value) {
            out.push(value);
        }
        if out.len() >= max {
            break;
        }
    }
    out
}

fn clean_string_list(list: Vec<String>, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    for value in list {
        let Some(value) = clean_one_string(value) else {
            continue;
        };
        if !out.iter().any(|x| x == &value) {
            out.push(value);
        }
        if out.len() >= max {
            break;
        }
    }
    out
}

fn clean_one_string(value: String) -> Option<String> {
    let value: String = value
        .chars()
        .filter(|c| !c.is_control())
        .take(4096)
        .collect();
    let value = value.trim().to_string();
    (!value.is_empty()).then_some(value)
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
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct TabInfo {
    id: u64,
    name: String,
    path: String,
    dirty: bool,
    active: bool,
}

#[derive(Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(super) struct TabsResponse {
    tabs: Vec<TabInfo>,
}

pub(crate) type SharedState = Arc<AppState>;

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::*;

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ayame-state-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn scratch_file(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(bytes).unwrap();
        path
    }

    #[test]
    fn a_hundred_session_paths_survive_the_cap() {
        // Regression for #52: a ~100-tab session must not be truncated to 64.
        let paths: Vec<String> = (0..100).map(|i| format!("/files/f{i}.txt")).collect();
        let cleaned = clean_path_list(paths.clone(), SESSION_MAX_PATHS);
        assert_eq!(cleaned.len(), 100);
        assert_eq!(cleaned, paths);
    }

    fn wal_opts(cache: &Path) -> OpenOptions {
        OpenOptions {
            cache_dir: Some(cache.to_path_buf()),
            ..OpenOptions::default()
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_open_of_same_file_installs_one_tab_and_one_wal() {
        let dir = scratch_dir("open-dedup");
        let path = scratch_file(&dir, "same.txt", b"same\n");
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
        let path = scratch_file(&dir, "guard.txt", b"guard\n");
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
                InactiveTab {
                    doc: ws.doc.as_ref().unwrap().clone(),
                    edits: EditSession::default(),
                    markers: MarkerSession::default(),
                    aside_files: Vec::new(),
                    recoverable: None,
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
        let a = scratch_file(&dir, "a.txt", b"a\n");
        let b = scratch_file(&dir, "b.txt", b"b\n");
        let c = scratch_file(&dir, "c.txt", b"c\n");
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
        let a = scratch_file(&dir, "a.txt", b"a\n");
        let b = scratch_file(&dir, "b.txt", b"b\n");
        let c = scratch_file(&dir, "c.txt", b"c\n");
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
