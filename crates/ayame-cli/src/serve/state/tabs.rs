use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::http::StatusCode;
use ayame_core::wal;
use ayame_core::{DiskState, Document, EditSession};
use serde::Serialize;

use super::wal_policy::{attach_live_wal, wal_prepare_for_open, WalSetup};
use super::{AppState, Shared, Workspace};
use crate::serve::markers::MarkerSession;
use crate::serve::{bad_request, internal, ApiError};

/// State of a tab that is open but not currently focused. The focused tab's
/// document and edits live in `Workspace::doc`/`edits` so every existing
/// endpoint keeps operating on "the active document" unchanged; switching tabs
/// just swaps that live state with an entry here.
pub(super) struct InactiveTab {
    pub(super) doc: Shared,
    pub(super) edits: EditSession,
    pub(super) markers: MarkerSession,
    /// Aside files (see [`Workspace::aside_files`]) travelling with the tab.
    pub(super) aside_files: Vec<PathBuf>,
    /// Pending crash-recovery decision travelling with the tab (see
    /// [`Workspace::recoverable`]).
    pub(super) recoverable: Option<usize>,
    /// External-change baseline (see [`Workspace::disk_baseline`]) travelling
    /// with the tab, so a file rewritten while its tab sat in the background
    /// is still reported when the tab comes back.
    pub(super) disk_baseline: Option<DiskState>,
}

#[derive(Default)]
pub(super) struct TabList {
    pub(super) order: Vec<u64>,
    pub(super) active: Option<u64>,
    pub(super) inactive: HashMap<u64, InactiveTab>,
    pub(super) next_id: u64,
}

impl Workspace {
    /// Replace the live tab state and return what was installed before it.
    /// This is the only place that lists the fields travelling with a tab, so
    /// adding another per-tab value changes one production path.
    fn install_tab_state(&mut self, tab: Option<InactiveTab>) -> Option<InactiveTab> {
        let previous = self.doc.take().map(|doc| {
            // Parked and removed sessions carry no live writer handle.
            self.edits.set_wal(None);
            InactiveTab {
                doc,
                edits: std::mem::take(&mut self.edits),
                markers: std::mem::take(&mut self.markers),
                aside_files: std::mem::take(&mut self.aside_files),
                recoverable: self.recoverable.take(),
                disk_baseline: self.disk_baseline.take(),
            }
        });
        match tab {
            Some(t) => {
                self.doc = Some(t.doc);
                self.edits = t.edits;
                self.markers = t.markers;
                self.aside_files = t.aside_files;
                self.recoverable = t.recoverable;
                self.disk_baseline = t.disk_baseline;
            }
            None => {
                self.doc = None;
                self.edits = EditSession::default();
                self.markers = MarkerSession::default();
                self.aside_files = Vec::new();
                self.recoverable = None;
                self.disk_baseline = None;
            }
        }
        previous
    }

    /// Park the currently active tab's live state so a different tab can take
    /// over the `doc`/`edits` slots.
    fn park_active(&mut self) {
        if let Some(aid) = self.tabs.active {
            if let Some(tab) = self.install_tab_state(None) {
                self.tabs.inactive.insert(aid, tab);
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
        let tab = self.tabs.inactive.remove(&id);
        let previous = self.install_tab_state(tab);
        debug_assert!(previous.is_none());
        attach_live_wal(cache_root, self);
        Ok(())
    }

    /// Remove `id` and, when it was active, install the neighbor at the same
    /// visible slot (or the previous final tab). The removed inactive state is
    /// returned so the caller can clean up its WAL and aside files.
    fn remove_tab_and_focus_neighbor(&mut self, id: u64) -> Option<InactiveTab> {
        let idx = self.tabs.order.iter().position(|tab_id| *tab_id == id)?;
        self.tabs.order.remove(idx);
        if self.tabs.active == Some(id) {
            let next = self
                .tabs
                .order
                .get(idx)
                .or_else(|| self.tabs.order.last())
                .copied();
            self.tabs.active = next;
            let neighbor = next.and_then(|neighbor_id| self.tabs.inactive.remove(&neighbor_id));
            self.install_tab_state(neighbor)
        } else {
            self.tabs.inactive.remove(&id)
        }
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
        self.set_doc(Some(doc));
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
            if !ws.tabs.order.contains(&id) {
                return (Vec::new(), None);
            }
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
            let was_active = ws.tabs.active == Some(id);
            if was_active {
                if let Some(doc) = ws.doc.clone() {
                    wal_path_of(&doc, ws.recoverable);
                }
            } else if let Some(tab) = ws.tabs.inactive.get(&id) {
                wal_path_of(&tab.doc, tab.recoverable);
            }

            let mut asides = Vec::new();
            if let Some(mut removed) = ws.remove_tab_and_focus_neighbor(id) {
                asides.append(&mut removed.aside_files);
            }
            if was_active {
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
            let was_active = ws.tabs.active == Some(id);
            let mut asides = Vec::new();
            if let Some(mut removed) = ws.remove_tab_and_focus_neighbor(id) {
                asides.append(&mut removed.aside_files);
            }
            if was_active {
                attach_live_wal(cache_root.as_deref(), ws);
            }
            Ok((asides, sync_file))
        })?;
        // The handoff contract: the log is durable before the caller spawns
        // (or signals) the adopting window. Surfaced on failure — proceeding
        // could hand off a log a power loss would tear.
        if let Some(f) = sync_file {
            f.sync_data().map_err(|e| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    format!("crash log sync failed: {e}"),
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
                let path = crate::serve::workspace::display_path(&path);
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
    /// other transitions. Stale aside files of `path` (crash leftovers from
    /// any previous session) are swept before opening.
    ///
    /// A path that is ALREADY open in some tab focuses that tab instead of
    /// opening a duplicate — choosing an already-open file means "go to that
    /// file", the same as every tabbed editor.
    pub(in crate::serve) async fn open_path(&self, path: String) -> Result<(), ApiError> {
        if let Some(id) = self.tab_with_path(Path::new(&path)).await {
            return self.switch_tab(id).await;
        }
        let opts = self.open_opts.clone();
        let p = path.clone();
        let (doc, wal_setup) = tokio::task::spawn_blocking(move || {
            crate::serve::workspace::sweep_stale_asides(Path::new(&p));
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
                crate::serve::workspace::strip_verbatim(&path)
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
    pub(in crate::serve) fn cleanup_aside_files(&self) {
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
pub(super) fn remove_aside_files(paths: Vec<PathBuf>) -> Vec<PathBuf> {
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
    if crate::serve::workspace::is_scratch_path(path) {
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
