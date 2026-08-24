use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::http::StatusCode;
use ayame_core::wal;
use ayame_core::{DiskState, Document, EditSession};
use serde::Serialize;

use super::super::markers::MarkerSession;
use super::super::{bad_request, internal, ApiError};
use super::wal_policy::{attach_live_wal, wal_prepare_for_open, WalSetup};
use super::{AppState, Shared};

/// State of a tab that is open but not currently focused. The focused tab's
/// document and edits live in `Workspace::doc`/`edits` so every existing
/// endpoint keeps operating on "the active document" unchanged.
pub(super) struct InactiveTab {
    pub(super) doc: Shared,
    pub(super) edits: EditSession,
    pub(super) markers: MarkerSession,
    /// Aside files travelling with the tab.
    pub(super) aside_files: Vec<PathBuf>,
    /// Pending crash-recovery decision travelling with the tab.
    pub(super) recoverable: Option<usize>,
    /// External-change baseline travelling with the tab.
    pub(super) disk_baseline: Option<DiskState>,
}

struct RemovedTab {
    state: InactiveTab,
    was_active: bool,
}

#[derive(Default)]
pub(super) struct TabList {
    pub(super) order: Vec<u64>,
    pub(super) active: Option<u64>,
    pub(super) inactive: HashMap<u64, InactiveTab>,
    pub(super) next_id: u64,
}

/// Everything mutable about the workspace: the active document, its edit
/// overlay, and the tab list. Kept in ONE struct behind ONE lock so no request
/// can ever observe a document paired with another document's edits.
pub(in crate::serve) struct Workspace {
    pub(super) doc: Option<Shared>,
    pub(in crate::serve) edits: EditSession,
    /// Session-only sparse markers for the active document.
    pub(super) markers: MarkerSession,
    /// Aside files left behind by in-place saves of the active document.
    pub(super) aside_files: Vec<PathBuf>,
    /// Pending crash-recovery decision for the active document.
    pub(super) recoverable: Option<usize>,
    /// The active document's last observed on-disk identity.
    pub(super) disk_baseline: Option<DiskState>,
    pub(super) tabs: TabList,
}

impl Workspace {
    /// The active document, if any.
    pub(in crate::serve) fn doc(&self) -> Option<&Shared> {
        self.doc.as_ref()
    }

    /// Install (or clear) freshly read content and re-seed its disk baseline.
    pub(super) fn set_doc(&mut self, doc: Option<Shared>) {
        self.disk_baseline = doc.as_ref().and_then(|document| document.disk_state());
        self.doc = doc;
    }

    /// Re-seed the baseline after this session replaces the on-disk file.
    pub(super) fn reseed_disk_baseline(&mut self) {
        self.disk_baseline = self.doc.as_ref().and_then(|document| document.disk_state());
    }

    /// Whether another writer changed the active file since our baseline.
    pub(super) fn disk_changed(&self) -> bool {
        let Some(baseline) = self.disk_baseline else {
            return false;
        };
        self.doc
            .as_ref()
            .is_some_and(|document| document.disk_state() != Some(baseline))
    }

    /// Install every tab-local field from one transport object.
    fn install_tab_state(&mut self, tab: Option<InactiveTab>) {
        match tab {
            Some(tab) => {
                self.doc = Some(tab.doc);
                self.edits = tab.edits;
                self.markers = tab.markers;
                self.aside_files = tab.aside_files;
                self.recoverable = tab.recoverable;
                self.disk_baseline = tab.disk_baseline;
            }
            None => {
                self.doc = None;
                self.edits = EditSession::default();
                self.markers = MarkerSession::default();
                self.aside_files.clear();
                self.recoverable = None;
                self.disk_baseline = None;
            }
        }
    }

    /// Move every field of the active tab into one transport object.
    fn take_active_tab_state(&mut self) -> Option<InactiveTab> {
        let doc = self.doc.take()?;
        Some(InactiveTab {
            doc,
            edits: std::mem::take(&mut self.edits),
            markers: std::mem::take(&mut self.markers),
            aside_files: std::mem::take(&mut self.aside_files),
            recoverable: self.recoverable.take(),
            disk_baseline: self.disk_baseline.take(),
        })
    }

    /// Remove `id` and install its neighbor when it was the active tab.
    fn remove_tab_and_focus_neighbor(&mut self, id: u64) -> Option<RemovedTab> {
        let index = self.tabs.order.iter().position(|tab_id| *tab_id == id)?;
        self.tabs.order.remove(index);
        if self.tabs.active != Some(id) {
            return self.tabs.inactive.remove(&id).map(|state| RemovedTab {
                state,
                was_active: false,
            });
        }

        let state = self.take_active_tab_state()?;
        let next = self
            .tabs
            .order
            .get(index)
            .or_else(|| self.tabs.order.last())
            .copied();
        self.tabs.active = next;
        let neighbor = next.and_then(|neighbor_id| self.tabs.inactive.remove(&neighbor_id));
        self.install_tab_state(neighbor);
        Some(RemovedTab {
            state,
            was_active: true,
        })
    }

    pub(in crate::serve) fn recoverable(&self) -> Option<usize> {
        self.recoverable
    }

    pub(in crate::serve) fn doc_and_edits(&self) -> Result<(&Shared, &EditSession), ApiError> {
        match &self.doc {
            Some(doc) => Ok((doc, &self.edits)),
            None => Err(no_document()),
        }
    }

    pub(in crate::serve) fn doc_edits_markers_mut(
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

    pub(in crate::serve) fn markers(&self) -> &MarkerSession {
        &self.markers
    }

    pub(in crate::serve) fn markers_mut(&mut self) -> &mut MarkerSession {
        &mut self.markers
    }

    /// Park the active tab so a different tab can take over the live slots.
    fn park_active(&mut self) {
        if let Some(active_id) = self.tabs.active {
            if self.doc.is_some() {
                self.edits.set_wal(None);
                if let Some(state) = self.take_active_tab_state() {
                    self.tabs.inactive.insert(active_id, state);
                }
            }
        }
    }

    fn focus_tab(&mut self, id: u64, cache_root: Option<&Path>) -> Result<(), ApiError> {
        if self.tabs.active == Some(id) {
            return Ok(());
        }
        if !self.tabs.order.contains(&id) {
            return Err(bad_request("no such tab"));
        }
        self.park_active();
        self.tabs.active = Some(id);
        let tab = self.tabs.inactive.remove(&id);
        self.install_tab_state(tab);
        attach_live_wal(cache_root, self);
        Ok(())
    }

    fn tab_with_path_literal(&self, path: &Path) -> Option<u64> {
        self.tabs.order.iter().copied().find(|&id| {
            let doc = if self.tabs.active == Some(id) {
                self.doc.as_ref()
            } else {
                self.tabs.inactive.get(&id).map(|tab| &tab.doc)
            };
            doc.is_some_and(|document| document.path() == path)
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
        self.set_doc(Some(doc));
        self.edits = EditSession::default();
        self.markers = MarkerSession::default();
        self.recoverable = None;
        match wal {
            WalSetup::Attach(writer) => self.edits.set_wal(Some(*writer)),
            WalSetup::Create => attach_live_wal(cache_root, self),
            WalSetup::Recoverable(count) => self.recoverable = Some(count),
            WalSetup::Off => {}
        }
        orphaned
    }
}

fn no_document() -> ApiError {
    ApiError::new(
        StatusCode::CONFLICT,
        "conflict",
        "no file is open — open one first",
    )
}

impl AppState {
    /// Focus an already-open tab.
    pub(in crate::serve) async fn switch_tab(&self, id: u64) -> Result<(), ApiError> {
        let _transitions = self.transitions.lock().await;
        // Leaf lock, taken while no ws guard is held (see `find_snapshot`).
        self.invalidate_dirty_snapshot();
        self.write(|ws| ws.focus_tab(id, self.open_opts.cache_dir.as_deref()))
    }

    /// Move an open tab before another tab, or to the end when `before_id` is
    /// absent. The active document and every inactive edit session stay put;
    /// only the user-visible order changes.
    pub(in crate::serve) async fn reorder_tab(
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
    pub(in crate::serve) async fn close_tab(&self, id: u64) {
        let _transitions = self.transitions.lock().await;
        self.invalidate_dirty_snapshot();
        let cache_root = self.open_opts.cache_dir.clone();
        let (asides, dead_wal) = self.write(|ws| {
            let Some(RemovedTab {
                mut state,
                was_active,
            }) = ws.remove_tab_and_focus_neighbor(id)
            else {
                return (Vec::new(), None);
            };

            // Closing a tab is a graceful discard of its unsaved edits (the
            // client confirms dirty closes first): its crash log goes with it
            // unless a previous process's recovery decision is still pending.
            let mut dead_wal = state.recoverable.is_none().then(|| {
                cache_root
                    .as_deref()
                    .map(|root| wal::wal_path_for(root, state.doc.path()))
            });
            let mut dead_wal = dead_wal.take().flatten();
            let asides = std::mem::take(&mut state.aside_files);
            // Release the removed tab's WAL writer before the neighbor opens
            // its own writer (important when both resolve to one WAL path).
            drop(state);

            if was_active {
                // The neighbor is live now: re-attach its crash log.
                attach_live_wal(cache_root.as_deref(), ws);
            }
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
        if let Some(path) = dead_wal {
            let _ = std::fs::remove_file(path);
        }
    }

    /// Detach a tab for a window-to-window handoff (issue #35): remove it
    /// exactly like [`AppState::close_tab`], but KEEP its crash log on disk —
    /// fsynced before returning — so the adopting window can replay the
    /// unsaved edits through the normal `/api/edit/recover` path.
    pub(in crate::serve) async fn detach_tab(&self, id: u64) -> Result<(), ApiError> {
        let _transitions = self.transitions.lock().await;
        self.invalidate_dirty_snapshot();
        let cache_root = self.open_opts.cache_dir.clone();
        let (asides, sync_file) = self.write(|ws| {
            if !ws.tabs.order.contains(&id) {
                return Err(ApiError::new(
                    StatusCode::NOT_FOUND,
                    "not_found",
                    "no such tab",
                ));
            }

            // Clone the log's file handle before the session (and with it the
            // live writer) is dropped; a cloned handle syncs the same file.
            let (dirty, sync_file, recoverable) = if ws.tabs.active == Some(id) {
                let dirty = ws.doc.as_ref().is_some_and(|doc| ws.edits.stats(doc).dirty);
                (
                    dirty,
                    ws.edits.wal_sync_file().ok().flatten(),
                    ws.recoverable,
                )
            } else {
                match ws.tabs.inactive.get(&id) {
                    Some(tab) => (
                        tab.edits.stats(&tab.doc).dirty,
                        tab.edits.wal_sync_file().ok().flatten(),
                        tab.recoverable,
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

            let RemovedTab {
                mut state,
                was_active,
            } = ws
                .remove_tab_and_focus_neighbor(id)
                .expect("tab existence was checked above");
            let asides = std::mem::take(&mut state.aside_files);
            // The sync handle cloned above remains valid; release the writer
            // itself before attaching the neighboring tab's writer.
            drop(state);
            if was_active {
                attach_live_wal(cache_root.as_deref(), ws);
            }
            Ok((asides, sync_file))
        })?;
        // The handoff contract: the log is durable before the caller spawns
        // (or signals) the adopting window.
        if let Some(file) = sync_file {
            file.sync_data().map_err(|error| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    format!("crash log sync failed: {error}"),
                )
            })?;
        }
        remove_aside_files(asides);
        Ok(())
    }

    /// Snapshot every open tab for the tab bar.
    pub(in crate::serve) fn tabs_response(&self) -> TabsResponse {
        self.read(|ws| {
            let active = ws.tabs.active;
            let mut out = Vec::with_capacity(ws.tabs.order.len());
            for &id in &ws.tabs.order {
                let (path, dirty) = if Some(id) == active {
                    match &ws.doc {
                        Some(doc) => (doc.path().to_path_buf(), ws.edits.stats(doc).dirty),
                        None => continue,
                    }
                } else {
                    match ws.tabs.inactive.get(&id) {
                        Some(tab) => (
                            tab.doc.path().to_path_buf(),
                            tab.edits.stats(&tab.doc).dirty,
                        ),
                        None => continue,
                    }
                };
                // UI-facing path (and the name derived from it): never leak a
                // Windows verbatim prefix.
                let path = super::super::workspace::display_path(&path);
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

    /// Open `path` with the workspace's options and make it the active document
    /// in a brand-new tab. The blocking open/index runs off the async runtime;
    /// the install itself is one atomic workspace mutation, serialized against
    /// other transitions.
    pub(in crate::serve) async fn open_path(&self, path: String) -> Result<(), ApiError> {
        if let Some(id) = self.tab_with_path(Path::new(&path)).await {
            return self.switch_tab(id).await;
        }
        let opts = self.open_opts.clone();
        let path_for_open = path.clone();
        let (doc, wal_setup) = tokio::task::spawn_blocking(move || {
            super::super::workspace::sweep_stale_asides(Path::new(&path_for_open));
            let doc = Document::open(&path_for_open, &opts)?;
            // Detect crash leftovers before any fresh writer can truncate them.
            // The writer itself is created only after the transition-lock
            // duplicate recheck below.
            let wal_setup = wal_prepare_for_open(opts.cache_dir.as_deref(), &doc);
            Ok::<_, ayame_core::Error>((doc, wal_setup))
        })
        .await
        .map_err(internal)?
        .map_err(|error| {
            bad_request(format!(
                "opening '{}': {error}",
                super::super::workspace::strip_verbatim(&path)
            ))
        })?;
        let _transitions = self.transitions.lock().await;
        let target = doc.path().to_path_buf();
        let orphaned = self.write(|ws| -> Result<Vec<PathBuf>, ApiError> {
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
    /// then by canonical path. Filesystem calls run off the async runtime and
    /// never under the workspace lock.
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
                        ws.tabs.inactive.get(&id).map(|tab| &tab.doc)
                    }?;
                    Some((id, doc.path().to_path_buf()))
                })
                .collect()
        });
        if tabs.is_empty() {
            return None;
        }
        tokio::task::spawn_blocking(move || {
            let canonical_target = std::fs::canonicalize(&target).ok();
            tabs.into_iter()
                .find(|(_, path)| {
                    *path == target
                        || canonical_target.as_deref().is_some_and(|canonical| {
                            std::fs::canonicalize(path).is_ok_and(|path| path == canonical)
                        })
                })
                .map(|(id, _)| id)
        })
        .await
        .ok()
        .flatten()
    }

    /// Best-effort deletion of every tracked aside file (all tabs), for the
    /// graceful-shutdown path.
    pub(in crate::serve) fn cleanup_aside_files(&self) {
        let all = self.write(|ws| {
            let mut paths = std::mem::take(&mut ws.aside_files);
            for tab in ws.tabs.inactive.values_mut() {
                paths.append(&mut tab.aside_files);
            }
            paths
        });
        remove_aside_files(all);
    }
}

/// Best-effort deletion of aside files; returns the ones that still exist but
/// could not be deleted (e.g. mapped on Windows) so the caller can keep
/// tracking them.
pub(super) fn remove_aside_files(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    paths
        .into_iter()
        .filter(|path| std::fs::remove_file(path).is_err() && path.exists())
        .collect()
}

/// Friendly tab label: "untitled" for scratch buffers, else the file's basename.
fn tab_name(path: &str) -> String {
    let basename = Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string());
    // Current scratch dirs are "ayame-srv-untitled-…"; restored sessions may
    // still carry the pre-rename "ayame-untitled-<pid>" form.
    if super::super::workspace::is_scratch_path(path) {
        return if basename == "untitled.txt" {
            "untitled".to_string()
        } else {
            basename
        };
    }
    basename
}

#[derive(Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(in crate::serve) struct TabInfo {
    pub(in crate::serve) id: u64,
    pub(in crate::serve) name: String,
    pub(in crate::serve) path: String,
    pub(in crate::serve) dirty: bool,
    pub(in crate::serve) active: bool,
}

#[derive(Serialize)]
#[cfg_attr(feature = "typegen", derive(ts_rs::TS))]
pub(in crate::serve) struct TabsResponse {
    pub(in crate::serve) tabs: Vec<TabInfo>,
}
