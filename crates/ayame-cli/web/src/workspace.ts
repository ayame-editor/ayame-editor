// Ayame Editor — workspace module. Type-stripped to JS at build time (build.rs, oxc).
import {
  $,
  displayPath,
  humanBytes,
  iconSvg,
  isAbsolutePath,
  isUntitled,
  joinPath,
  pathBaseName,
  pathCrumbs,
  pathDirName,
  setModalOpen,
} from "./dom.js";
import { RECENT_KEY, RECENT_MAX, TREE_KEY, state } from "./state.js";
import { serverMessage, t } from "./i18n.js";
import { api, apiPost } from "./api.js";
import { confirmCloseLastTab, requestEditorClose } from "./app.js";
import { maybeOfferWalRecovery, noteWalError, savingCount } from "./save.js";
import { clearLineCache, focusEditor, render, scheduleRender, setCaret } from "./editor.js";
import { fileMenuVisible, hideFileMenu, initMenuBar, updateStatusMeta } from "./menus.js";
import { setFollowTail, settleEditQueue } from "./edits.js";
import { flashCount } from "./search.js";
import { askConfirm, hideLoading, showLoading, showMessage } from "./dialogs.js";
import { saveSettings } from "./settings.js";
import type { BrowseResponse, OpenRequest, TabIdRequest, TabsResponse } from "./types/api.js";

// ---- workspace: open / browse / drag&drop ----------------------------------

export function openerVisible() {
  return !$("opener").classList.contains("hidden");
}

export function showOpener() {
  configureOpener("open");
  setModalOpen($("opener"), true);
  browse(null);
  const inp = $("opener-input");
  inp.value = "";
  queueMicrotask(() => inp.focus());
}

export function showSaveDialog(title, suggestedPath): Promise<any> {
  return new Promise((resolve) => {
    configureOpener("save", title);
    state.openerResolve = resolve;
    const inp = $("opener-input");
    const dir = pathDirName(suggestedPath) || localStorage.getItem(TREE_KEY) || ".";
    inp.value = pathBaseName(suggestedPath) || "untitled.txt";
    setModalOpen($("opener"), true);
    browse(dir);
    queueMicrotask(() => {
      inp.focus();
      inp.select();
    });
  });
}

export function configureOpener(mode, title?) {
  state.openerMode = mode;
  const save = mode === "save";
  const m = $("opener");
  m.classList.toggle("save-mode", save);
  $("opener-title").textContent = title || t(save ? "menu.saveAs" : "dialog.open.title");
  $("opener-input-label").textContent = t(save ? "dialog.open.fileName" : "dialog.open.path");
  $("opener-input").placeholder = t(
    save ? "dialog.open.namePlaceholder" : "dialog.open.pathPlaceholder",
  );
  $("opener-open").textContent = t(save ? "menu.save" : "menu.open");
  $("opener-folder").textContent = t(save ? "dialog.open.location" : "dialog.open.folder");
  $("opener-folder").title = t(save ? "dialog.open.folderToExplorer" : "dialog.open.folderToTree");
  $("opener-hint").textContent = t(save ? "dialog.open.hintSave" : "dialog.open.hintOpen");
  openerMsg("");
  renderRecentFiles();
}

export function hideOpener() {
  if (state.openerMode === "save") {
    finishSaveDialog(null);
    return;
  }
  // The opener doubles as the welcome screen: don't let it close while there is
  // no document to fall back to.
  if (!state.stat?.open) return;
  setModalOpen($("opener"), false);
  focusEditor();
}

export function finishSaveDialog(value) {
  const resolve = state.openerResolve;
  state.openerResolve = null;
  state.openerMode = "open";
  setModalOpen($("opener"), false);
  configureOpener("open");
  focusEditor();
  if (resolve) resolve(value);
}

export function openerMsg(text, busy = false) {
  const el = $("opener-msg");
  el.textContent = text || "";
  el.classList.toggle("busy", !!text && busy);
}

export async function browse(dir) {
  openerMsg(t("dialog.open.loading"), true);
  try {
    const q = dir == null ? "" : `?dir=${encodeURIComponent(dir)}`;
    const res = await api<BrowseResponse>(`/api/browse${q}`);
    renderBrowse(res);
    openerMsg("");
  } catch (e) {
    openerMsg(t("dialog.open.dirError", { msg: serverMessage(e.message) }));
  }
}

export function renderBrowse(res) {
  state.openerDir = res.dir;
  state.openerEntries = res.entries || [];
  renderCwdCrumbs(res.dir);
  const list = $("opener-list");
  list.textContent = "";
  if (res.parent) {
    list.append(browseRow({ name: "..", path: res.parent, is_dir: true }, true));
  }
  for (const ent of res.entries) list.append(browseRow(ent, false));
  list.scrollTop = 0;
}

export function renderCwdCrumbs(path) {
  const cwd = $("opener-cwd");
  const clean = String(path || "").replace(/^\\\\\?\\/, "");
  cwd.textContent = "";
  cwd.title = clean;
  for (const [i, crumb] of pathCrumbs(clean).entries()) {
    if (i > 0) {
      const sep = document.createElement("span");
      sep.className = "cwd-sep";
      sep.setAttribute("aria-hidden", "true");
      sep.append(iconSvg("i-chevron-right"));
      cwd.append(sep);
    }
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "cwd-crumb";
    btn.textContent = crumb.label;
    btn.title = crumb.path;
    btn.addEventListener("click", () => browse(crumb.path));
    cwd.append(btn);
  }
}

export function browseRow(ent, isUp) {
  const row = document.createElement("button");
  row.className = "opener-row" + (ent.is_dir ? " dir" : "") + (isUp ? " up" : "");
  row.type = "button";
  // The kind ("フォルダ" / "ファイル") moved from visible text into the icon;
  // keep it for screen readers via the row's accessible name.
  row.setAttribute(
    "aria-label",
    isUp ? t("tree.up") : `${ent.is_dir ? t("dialog.open.folder") : t("menu.file")}: ${ent.name}`,
  );
  const ic = document.createElement("span");
  ic.className = "ic";
  ic.setAttribute("aria-hidden", "true");
  ic.append(iconSvg(isUp ? "i-folder-up" : ent.is_dir ? "i-folder" : "i-file"));
  const nm = document.createElement("span");
  nm.className = "nm";
  nm.textContent = isUp ? t("tree.up") : ent.name;
  const sz = document.createElement("span");
  sz.className = "sz";
  sz.textContent = ent.is_dir ? "" : humanBytes(ent.size);
  row.append(ic, nm, sz);
  row.addEventListener("click", () => {
    if (ent.is_dir) browse(ent.path);
    else if (state.openerMode === "save") {
      $("opener-input").value = ent.name;
      markPickedFile(ent.name);
      $("opener-input").focus();
    } else openPath(ent.path);
  });
  row.addEventListener("dblclick", () => {
    if (!ent.is_dir && state.openerMode === "save") commitOpener();
  });
  return row;
}

// ---- recent files (最近使ったファイル) --------------------------------------
//
// A best-effort, browser-only history of recently opened paths. Kept in
// localStorage (most-recent-first, deduped, capped) so it survives reloads
// without any server/state changes. Surfaced as a shortcut list in the opener.

export function loadRecentFiles() {
  try {
    const raw = JSON.parse(localStorage.getItem(RECENT_KEY) || "[]");
    return Array.isArray(raw) ? raw.filter((x) => typeof x === "string").slice(0, RECENT_MAX) : [];
  } catch {
    return [];
  }
}

export function saveRecentFiles(list) {
  try {
    localStorage.setItem(RECENT_KEY, JSON.stringify(list.slice(0, RECENT_MAX)));
  } catch {
    // ignore private-mode quota errors; recents are best-effort
  }
}

// Record a freshly opened file at the head of the list. Untitled scratch
// buffers never qualify.
export function pushRecentFile(path) {
  const p = (path || "").trim();
  if (!p || isUntitled(p)) return;
  const list = [p, ...loadRecentFiles().filter((x) => x !== p)].slice(0, RECENT_MAX);
  saveRecentFiles(list);
}

// Forget a path (e.g. it no longer opens) so the list stays trustworthy.
export function dropRecentFile(path) {
  saveRecentFiles(loadRecentFiles().filter((x) => x !== path));
}

// Open a recent entry through the normal open path; drop it if it's gone.
export async function openRecent(path) {
  const ok = await openPath(path);
  if (!ok) {
    dropRecentFile(path);
    renderRecentFiles();
  }
}

export function renderRecentFiles() {
  const box = $("opener-recent");
  if (!box) return;
  // The recent shortcut only makes sense when opening, not when saving.
  const list = state.openerMode === "save" ? [] : loadRecentFiles();
  box.textContent = "";
  if (!list.length) {
    box.classList.add("hidden");
    return;
  }
  const head = document.createElement("div");
  head.className = "opener-recent-head";
  head.textContent = t("dialog.open.recent");
  box.append(head);
  for (const path of list) box.append(recentRow(path));
  box.classList.remove("hidden");
}

export function recentRow(path) {
  const row = document.createElement("button");
  row.className = "opener-row recent";
  row.type = "button";
  row.title = path;
  row.setAttribute("aria-label", `${t("dialog.open.recent")}: ${pathBaseName(path) || path}`);
  const ic = document.createElement("span");
  ic.className = "ic";
  ic.setAttribute("aria-hidden", "true");
  ic.append(iconSvg("i-clock"));
  const nm = document.createElement("span");
  nm.className = "nm";
  nm.textContent = pathBaseName(path) || path;
  const dir = document.createElement("span");
  dir.className = "sz";
  dir.textContent = pathDirName(path) || "";
  row.append(ic, nm, dir);
  row.addEventListener("click", () => openRecent(path));
  return row;
}

export function markPickedFile(name) {
  for (const row of $("opener-list").querySelectorAll(".opener-row")) {
    row.classList.toggle("picked", row.querySelector(".nm")?.textContent === name);
  }
}

export async function saveDialogTarget() {
  const raw = $("opener-input").value.trim();
  if (!raw) {
    openerMsg(t("dialog.open.enterFileName"));
    return null;
  }
  const path = isAbsolutePath(raw) ? raw : joinPath(state.openerDir, raw);
  const base = pathBaseName(path);
  const existing = state.openerEntries.find((e) => !e.is_dir && e.name === base);
  const overwrite = !!existing;
  if (overwrite) {
    const ok = await askConfirm(
      t("dialog.overwrite.title"),
      t("dialog.overwrite.ask", { name: base }),
      {
        okLabel: t("dialog.overwrite.ok"),
        danger: true,
      },
    );
    if (!ok) return null;
  }
  return { path, overwrite };
}

export async function commitOpener() {
  if (state.openerMode === "save") {
    const target = await saveDialogTarget();
    if (target) finishSaveDialog(target);
    return;
  }
  // Like save mode: a typed relative name means "in the folder being
  // browsed", not relative to wherever the server process was launched.
  const raw = $("opener-input").value.trim();
  if (!raw) return;
  openPath(isAbsolutePath(raw) ? raw : joinPath(state.openerDir, raw));
}

// A pristine untitled buffer (never typed in, never saved) is replaced when a
// real file is opened, Notepad++/VS Code-style — otherwise every launch would
// leave an empty "untitled" tab dangling next to the opened file.
export function pristineUntitledTabId() {
  const active = (state.tabs || []).find((t) => t.active);
  if (!active || active.dirty || !isUntitled(active.path)) return null;
  if (state.stat?.dirty || state.stat?.can_undo) return null;
  return active.id;
}

export async function closeTabSilently(id) {
  if (id == null) return;
  try {
    // Re-check against the server's current truth: only a still-open,
    // background, still-clean tab is closed.
    const r = await api<TabsResponse>("/api/tabs");
    const tab = (r.tabs || []).find((t) => t.id === id);
    if (!tab || tab.active || tab.dirty) return;
    await apiPost<unknown, TabIdRequest>("/api/tabs/close", { id });
    refreshTabs();
  } catch {
    // non-fatal: the extra tab just stays
  }
}

export async function openPath(path) {
  const p = (path || "").trim();
  if (!p) return false;
  await settleEditQueue();
  const pristine = pristineUntitledTabId();
  openerMsg(t("dialog.open.opening"), true);
  showLoading(t("dialog.open.loadingFile", { name: pathBaseName(p) || p }));
  try {
    const stat = await apiPost<unknown, OpenRequest>("/api/open", { path: p });
    onDocumentOpened(stat);
    await closeTabSilently(pristine);
    return true;
  } catch (e) {
    reportOpenError(t("error.cannotOpen", { msg: serverMessage(e.message) }));
    return false;
  } finally {
    hideLoading();
  }
}

export async function uploadFile(file) {
  await settleEditQueue();
  const pristine = pristineUntitledTabId();
  openerMsg(t("dialog.open.loadingName", { name: file.name }), true);
  showLoading(t("dialog.open.loadingFile", { name: file.name }));
  try {
    const r = await fetch(`/api/upload?name=${encodeURIComponent(file.name)}`, {
      method: "POST",
      body: file,
    });
    if (!r.ok) throw new Error((await r.text()) || r.statusText);
    onDocumentOpened(await r.json());
    await closeTabSilently(pristine);
  } catch (e) {
    reportOpenError(t("error.loadErrorMsg", { msg: serverMessage(e.message) }));
  } finally {
    hideLoading();
  }
}

export async function uploadFiles(files) {
  for (const file of Array.from(files || [])) {
    if (file) await uploadFile(file);
  }
}

// Surface an open/upload failure where the user is looking: inside the opener if
// it's up, otherwise in the toolbar (and an alert if a doc is already open).
export function reportOpenError(msg) {
  if (openerVisible()) {
    openerMsg(msg);
  } else if (state.stat?.open) {
    flashCount(t("error.loadError"), "error");
    showMessage(t("error.loadError"), msg);
  } else {
    showOpener();
    openerMsg(msg);
  }
}

export function onDocumentOpened(stat) {
  state.docGen++;
  state.editGen++; // stale in-flight edit responses must not reposition this tab
  setFollowTail(false); // following is per-document; a new doc/tab starts un-followed
  state.stat = stat;
  pushRecentFile(stat.path);
  state.total = stat.view_lines ?? stat.lines ?? 0;
  // Fresh document: reset navigation, search, and caret state.
  state.first = 0;
  state.caret = { line: 0, col: 0 };
  state.goalCol = 0;
  state.activeLine = 0;
  state.sel = null;
  state.extraCursors = [];
  state.lastMatch = null;
  state.searchHits = null;
  state.searchTruncated = false;
  $("find-count").textContent = "";
  clearLineCache();
  setModalOpen($("opener"), false);
  updateStatusMeta();
  render();
  refreshTabs();
  updateTreeActive();
  focusEditor();
  noteWalError(stat);
  maybeOfferWalRecovery(stat); // async on purpose: the open itself is done
}

export function hasFiles(e) {
  const t = e.dataTransfer;
  return !!t && Array.from(t.types || []).includes("Files");
}

export function initDropZone() {
  const dz = $("dropzone");
  let depth = 0;
  window.addEventListener("dragenter", (e) => {
    if (!hasFiles(e)) return;
    e.preventDefault();
    depth++;
    dz.classList.remove("hidden");
  });
  window.addEventListener("dragover", (e) => {
    if (!hasFiles(e)) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = "copy";
  });
  window.addEventListener("dragleave", (e) => {
    if (!hasFiles(e)) return;
    depth = Math.max(0, depth - 1);
    if (depth === 0) dz.classList.add("hidden");
  });
  window.addEventListener("drop", (e) => {
    if (!hasFiles(e)) return;
    e.preventDefault();
    depth = 0;
    dz.classList.add("hidden");
    uploadFiles(e.dataTransfer.files);
  });
}

// ---- tabs ------------------------------------------------------------------

let refreshTabsSeq = 0;

export async function refreshTabs() {
  const seq = ++refreshTabsSeq;
  try {
    const r = await api<TabsResponse>("/api/tabs");
    if (seq !== refreshTabsSeq) return;
    renderTabs(r.tabs);
  } catch {
    // non-fatal: the tab bar just won't update
  }
}

export function renderTabs(list) {
  state.tabs = list;
  const c = $("tabs");
  c.setAttribute("role", "tablist");
  c.textContent = "";
  for (const tab of list) {
    const el = document.createElement("div");
    el.className = "tab" + (tab.active ? " active" : "") + (tab.dirty ? " dirty" : "");
    el.dataset.id = String(tab.id);
    el.title = displayPath(tab.path);
    el.setAttribute("role", "tab");
    el.setAttribute("aria-selected", tab.active ? "true" : "false");
    el.tabIndex = 0;
    const dot = document.createElement("span");
    dot.className = "tab-dot";
    const nm = document.createElement("span");
    nm.className = "tab-name";
    nm.textContent = tab.name;
    const x = document.createElement("button");
    x.type = "button";
    x.className = "tab-x";
    x.append(iconSvg("i-close"));
    x.title = t("common.close");
    x.setAttribute("aria-label", t("tab.closeName", { name: tab.name }));
    el.append(dot, nm, x);
    el.addEventListener("click", () => {
      if (!tab.active) selectTab(tab.id);
    });
    el.addEventListener("keydown", (e) => {
      if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        if (!tab.active) selectTab(tab.id);
      } else if (e.key === "Delete") {
        e.preventDefault();
        closeTab(tab.id);
      }
    });
    el.addEventListener("mousedown", (e) => {
      if (e.button === 1) {
        e.preventDefault();
        closeTab(tab.id); // middle-click closes
      }
    });
    x.addEventListener("click", (e) => {
      e.stopPropagation();
      closeTab(tab.id);
    });
    c.append(el);
  }
}

export async function selectTab(id) {
  try {
    await settleEditQueue();
    onDocumentOpened(await apiPost<unknown, TabIdRequest>("/api/tabs/select", { id }));
  } catch (e) {
    flashCount(t("tab.switchError"));
    console.error(e);
  }
}

export async function closeTab(id) {
  await settleEditQueue();
  // A save in flight is a hard barrier for BOTH branches: closing the tab
  // server-side while its save is still writing races the commit.
  if (savingCount > 0) {
    flashCount(t("editor.savingWait"));
    return;
  }
  const tab = state.tabs.find((x) => x.id === id);
  const isLast = (state.tabs || []).length <= 1;
  if (isLast) {
    if (!(await confirmCloseLastTab(tab))) return;
    if (requestEditorClose()) return;
  } else if (tab && tab.dirty) {
    const ok = await askConfirm(t("tab.close"), t("tab.confirmDiscard", { name: tab.name }), {
      okLabel: t("tab.discardClose"),
      danger: true,
    });
    if (!ok) return;
  }
  try {
    const stat = await apiPost<{ open: boolean }, TabIdRequest>("/api/tabs/close", { id });
    if (!stat.open) {
      await newUntitled(); // closed the last tab → open a fresh page
    } else {
      onDocumentOpened(stat);
    }
  } catch (e) {
    flashCount(t("tab.closeError"));
    console.error(e);
  }
}

// ---- sidebar file tree ------------------------------------------------------

export function sidebarOpen() {
  return !$("sidebar").classList.contains("hidden");
}

export function setSidebar(open) {
  $("sidebar").classList.toggle("hidden", !open);
  $("toggle-sidebar").classList.toggle("on", open);
  state.settings = { ...state.settings, sidebar: open };
  saveSettings(state.settings);
  if (open && !state.treeLoaded) {
    state.treeLoaded = true;
    treeSetRoot(localStorage.getItem(TREE_KEY) || null);
  }
  scheduleRender(); // viewport width changed
}

// Load `dir` (or the server default when null) as the tree root.
export async function treeSetRoot(dir) {
  try {
    const q = dir ? `?dir=${encodeURIComponent(dir)}` : "";
    const res = await api<BrowseResponse>(`/api/browse${q}`);
    state.treeParent = res.parent;
    $("sb-root").textContent = displayPath(res.dir);
    $("sb-root").title = displayPath(res.dir);
    try {
      localStorage.setItem(TREE_KEY, res.dir);
    } catch {
      // ignore quota
    }
    const tree = $("tree");
    tree.textContent = "";
    tree.append(renderTreeEntries(res.entries, 0));
  } catch {
    // A stale saved root: fall back to the server default once.
    if (dir) {
      treeSetRoot(null);
    } else {
      $("tree").textContent = "";
    }
  }
}

export function renderTreeEntries(entries, depth) {
  const frag = document.createDocumentFragment();
  for (const ent of entries) frag.append(renderTreeNode(ent, depth));
  return frag;
}

export function renderTreeNode(ent, depth) {
  const row = document.createElement("div");
  row.className = "tnode " + (ent.is_dir ? "dir" : "file");
  row.dataset.path = ent.path;
  row.style.setProperty("--depth", String(depth));
  if (!ent.is_dir && ent.path === state.stat?.path) row.classList.add("active");
  const indent = document.createElement("span");
  indent.className = "tindent";
  for (let i = 0; i < depth; i++) {
    const guide = document.createElement("span");
    guide.className = "tguide";
    indent.append(guide);
  }
  const chev = document.createElement("span");
  chev.className = "chev";
  chev.setAttribute("aria-hidden", "true");
  const icon = document.createElement("span");
  icon.className = "ticon " + (ent.is_dir ? "folder" : `file ${treeFileClass(ent.name)}`);
  icon.setAttribute("aria-hidden", "true");
  const nm = document.createElement("span");
  nm.className = "tname";
  nm.textContent = ent.name;
  row.append(indent, chev, icon, nm);

  if (!ent.is_dir) {
    if (typeof ent.size === "number") {
      const meta = document.createElement("span");
      meta.className = "tmeta";
      meta.textContent = humanBytes(ent.size);
      row.append(meta);
    }
    row.title = displayPath(ent.path);
    row.addEventListener("click", (e) => {
      e.stopPropagation();
      // Opens in a new tab; a file that is already open just gets focused
      // (the server dedupes by path).
      openPath(ent.path);
    });
    return row;
  }

  // Folder: lazily load children on first expand.
  const kids = document.createElement("div");
  kids.className = "tkids";
  kids.style.display = "none";
  let loaded = false;
  row.addEventListener("click", async (e) => {
    e.stopPropagation();
    const opening = kids.style.display === "none";
    row.classList.toggle("open", opening);
    if (opening && !loaded) {
      loaded = true;
      try {
        const res = await api<BrowseResponse>(`/api/browse?dir=${encodeURIComponent(ent.path)}`);
        kids.append(renderTreeEntries(res.entries, depth + 1));
      } catch {
        loaded = false;
      }
    }
    kids.style.display = opening ? "block" : "none";
  });
  const frag = document.createDocumentFragment();
  frag.append(row, kids);
  return frag;
}

export function treeFileClass(name) {
  const ext =
    String(name || "")
      .split(".")
      .pop()
      ?.toLowerCase() || "";
  if (ext === "md" || ext === "markdown") return "md";
  if (ext === "py") return "py";
  if (ext === "json") return "json";
  if (ext === "csv" || ext === "tsv" || ext === "xlsx") return "data";
  return "text";
}

export function updateTreeActive() {
  const path = state.stat?.path || "";
  document.querySelectorAll("#tree .tnode.file").forEach((row) => {
    row.classList.toggle("active", !!path && (row as any).dataset.path === path);
  });
}

export function initTree() {
  $("toggle-sidebar").addEventListener("click", () => setSidebar(!sidebarOpen()));
  $("sb-close").addEventListener("click", () => setSidebar(false));
  $("sb-up").addEventListener("click", () => {
    if (state.treeParent) treeSetRoot(state.treeParent);
  });
  $("opener-folder").addEventListener("click", () => {
    if (!state.openerDir) return;
    if (!sidebarOpen()) setSidebar(true);
    state.treeLoaded = true;
    treeSetRoot(state.openerDir);
    if (state.openerMode === "save") {
      openerMsg(t("dialog.open.folderShown"));
      return;
    }
    hideOpener();
  });
  // Apply persisted visibility.
  if (state.settings.sidebar) setSidebar(true);
}

// Start a fresh empty "untitled" buffer with a blank editable first line, so
// the app opens to a usable page (like Notepad) instead of a dialog.
export async function newUntitled() {
  try {
    await settleEditQueue();
    onDocumentOpened(await apiPost("/api/new", {}));
    // The buffer already has one empty line; drop the caret in, Notepad-style.
    setCaret(0, 0);
    focusEditor();
  } catch (e) {
    showOpener();
    openerMsg(t("error.newBuffer", { msg: serverMessage(e.message) }));
  }
}

export function initWorkspace() {
  initMenuBar();
  document.addEventListener("pointerdown", (e) => {
    if (fileMenuVisible() && !(e.target as any).closest(".menu-shell")) hideFileMenu();
  });
  $("new-file").addEventListener("click", () => {
    hideFileMenu();
    newUntitled();
  });
  $("open-file").addEventListener("click", () => {
    hideFileMenu();
    showOpener();
  });
  $("opener-close").addEventListener("click", hideOpener);
  $("opener-open").addEventListener("click", commitOpener);
  $("opener-input").addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      commitOpener();
    } else if (e.key === "Escape") {
      e.preventDefault();
      hideOpener();
    }
  });
  // Click on the dim backdrop (outside the panel) closes the dialog.
  $("opener").addEventListener("click", (e) => {
    if (e.target === $("opener")) hideOpener();
  });
  $("new-tab").addEventListener("click", () => newUntitled());
  initDropZone();
}
