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
import { BROWSE_KEY, state } from "./state.js";
import { currentLocale, serverMessage, t } from "./i18n.js";
import { api, apiPost } from "./api.js";
import {
  confirmCloseLastTab,
  isNativeApp,
  nativeOpenDialog,
  nativeSaveDialog,
  openNewWindow,
  requestEditorClose,
} from "./app.js";
import {
  expectWalHandoff,
  maybeOfferWalRecovery,
  noteWalError,
  saveCopy,
  savingCount,
} from "./save.js";
import { clearLineCache, focusEditor, render, setCaret } from "./editor.js";
import {
  fileMenuVisible,
  hideFileMenu,
  initMenuBar,
  showPopupMenu,
  updateStatusMeta,
} from "./menus.js";
import { setFollowTail, settleEditQueue } from "./edits.js";
import { flashCount } from "./search.js";
import { askConfirm, hideLoading, showLoading, showMessage } from "./dialogs.js";
import { loadRecentFilesShared, saveRecentFilesShared } from "./persistence.js";
import type {
  BrowseResponse,
  OpenRequest,
  TabIdRequest,
  TabReorderRequest,
  TabsResponse,
} from "./types/api.js";

// ---- workspace: open / browse / drag&drop ----------------------------------

let openerActiveIndex = -1;
let openerOptionSeq = 0;

export function openerOptions(): HTMLElement[] {
  return [
    ...document.querySelectorAll<HTMLElement>(
      "#opener-recent .opener-row, #opener-list .opener-row",
    ),
  ];
}

export function resetOpenerSelection() {
  openerActiveIndex = -1;
  for (const id of ["opener-recent", "opener-list"]) {
    document.getElementById(id)?.removeAttribute("aria-activedescendant");
  }
  for (const row of openerOptions()) {
    row.classList.remove("active");
    row.setAttribute("aria-selected", "false");
  }
}

function setOpenerActiveIndex(index, focusList = false) {
  const options = openerOptions();
  if (!options.length) {
    resetOpenerSelection();
    return null;
  }
  openerActiveIndex = (index + options.length) % options.length;
  const active = options[openerActiveIndex];
  for (const row of options) {
    const selected = row === active;
    row.classList.toggle("active", selected);
    row.setAttribute("aria-selected", String(selected));
  }
  for (const id of ["opener-recent", "opener-list"]) {
    document.getElementById(id)?.removeAttribute("aria-activedescendant");
  }
  const owner = active.closest<HTMLElement>('[role="listbox"]');
  owner?.setAttribute("aria-activedescendant", active.id);
  if (focusList) owner?.focus();
  active.scrollIntoView?.({ block: "nearest" });
  return active;
}

export function moveOpenerSelection(delta, focusList = true) {
  const options = openerOptions();
  if (!options.length) return null;
  const next =
    openerActiveIndex < 0 ? (delta < 0 ? options.length - 1 : 0) : openerActiveIndex + delta;
  return setOpenerActiveIndex(next, focusList);
}

function prepareOpenerOption(row: HTMLElement) {
  row.id = `opener-option-${++openerOptionSeq}`;
  row.tabIndex = -1;
  row.setAttribute("role", "option");
  row.setAttribute("aria-selected", "false");
  row.addEventListener("mouseenter", () => {
    const index = openerOptions().indexOf(row);
    if (index >= 0) setOpenerActiveIndex(index);
  });
}

export function onOpenerListFocus(e) {
  const owner = e.currentTarget as HTMLElement;
  const options = openerOptions();
  const active = options[openerActiveIndex];
  if (active && owner.contains(active)) return;
  const first = options.findIndex((row) => owner.contains(row));
  if (first >= 0) setOpenerActiveIndex(first);
}

export function onOpenerListKeydown(e) {
  if (e.key === "Escape") {
    e.preventDefault();
    hideOpener();
    return;
  }
  if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    e.preventDefault();
    moveOpenerSelection(e.key === "ArrowDown" ? 1 : -1);
    return;
  }
  if (e.key === "Home" || e.key === "End") {
    e.preventDefault();
    const options = openerOptions();
    if (options.length) setOpenerActiveIndex(e.key === "Home" ? 0 : options.length - 1, true);
    return;
  }
  if (e.key === "Enter" || e.key === " ") {
    const active = openerOptions()[openerActiveIndex];
    if (active) {
      e.preventDefault();
      active.click();
    }
  }
}

export function onOpenerInputKeydown(e) {
  if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    e.preventDefault();
    moveOpenerSelection(e.key === "ArrowDown" ? 1 : -1);
  } else if (e.key === "Enter") {
    e.preventDefault();
    commitOpener();
  } else if (e.key === "Escape") {
    e.preventDefault();
    hideOpener();
  }
}

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

// ファイルを開く: the desktop build calls the OS's own open dialog; the
// browser build falls back to the in-app picker (a web page cannot hand the
// server a real local path any other way).
export async function openFileDialog() {
  if (!isNativeApp()) {
    showOpener();
    return;
  }
  const current = state.stat?.path || "";
  const dir = !isUntitled(current) ? pathDirName(current) || "" : "";
  const paths = await nativeOpenDialog(dir);
  for (const p of paths) await openPath(p);
}

export function showSaveDialog(title, suggestedPath): Promise<any> {
  if (isNativeApp()) {
    // OS dialog: it confirms overwrites itself, so a returned path is final.
    return nativeSaveDialog(
      pathDirName(suggestedPath) || "",
      pathBaseName(suggestedPath) || "untitled.txt",
    ).then((path) => (path ? { path, overwrite: true } : null));
  }
  return new Promise((resolve) => {
    configureOpener("save", title);
    state.openerResolve = resolve;
    const inp = $("opener-input");
    const dir = pathDirName(suggestedPath) || localStorage.getItem(BROWSE_KEY) || ".";
    inp.value = pathBaseName(suggestedPath) || "untitled.txt";
    setModalOpen($("opener"), true);
    browse(dir);
    queueMicrotask(() => {
      inp.focus();
      inp.select();
    });
  });
}

// Pick a folder with the in-app browser (issue #79.1). Both builds use it: the
// OS open dialog only returns files, and folder targets (grep's search root)
// need a directory. Resolves with the chosen absolute path, or null on cancel.
export function showFolderDialog(title, startDir): Promise<string | null> {
  return new Promise((resolve) => {
    configureOpener("folder", title);
    state.openerResolve = resolve;
    setModalOpen($("opener"), true);
    browse(startDir || localStorage.getItem(BROWSE_KEY) || null);
    queueMicrotask(() => ($("opener-open") as HTMLButtonElement).focus());
  });
}

export function finishFolderDialog(value) {
  const resolve = state.openerResolve;
  state.openerResolve = null;
  state.openerMode = "open";
  setModalOpen($("opener"), false);
  configureOpener("open");
  // No focusEditor() here: the folder picker is opened over another modal (the
  // grep form), so control returns there, not to the editor.
  if (resolve) resolve(value);
}

export function configureOpener(mode, title?) {
  state.openerMode = mode;
  const save = mode === "save";
  const folder = mode === "folder";
  const m = $("opener");
  m.classList.toggle("save-mode", save);
  m.classList.toggle("folder-mode", folder);
  $("opener-title").textContent =
    title || t(save ? "menu.saveAs" : folder ? "dialog.open.chooseFolder" : "dialog.open.title");
  $("opener-input-label").textContent = t(
    save ? "dialog.open.fileName" : folder ? "dialog.open.folderPath" : "dialog.open.path",
  );
  $("opener-input").placeholder = t(
    save
      ? "dialog.open.namePlaceholder"
      : folder
        ? "dialog.open.folderPlaceholder"
        : "dialog.open.pathPlaceholder",
  );
  $("opener-open").textContent = t(
    save ? "menu.save" : folder ? "dialog.open.chooseFolder" : "menu.open",
  );
  // "New File" only makes sense on the welcome/open screen, not save/folder.
  $("opener-new").classList.toggle("hidden", save || folder);
  $("opener-hint").textContent = t(
    save ? "dialog.open.hintSave" : folder ? "dialog.open.hintFolder" : "dialog.open.hintOpen",
  );
  openerMsg("");
  renderRecentFiles();
}

export function hideOpener() {
  resetOpenerSelection();
  if (state.openerMode === "save") {
    finishSaveDialog(null);
    return;
  }
  if (state.openerMode === "folder") {
    finishFolderDialog(null);
    return;
  }
  setModalOpen($("opener"), false);
  // The opener doubles as the welcome screen. Refusing to close it with nothing
  // open was a dead end (Esc/✕ inert, no "New File"); instead land on a fresh
  // untitled buffer so there is always a way out (#174).
  if (!state.stat?.open) {
    newUntitled();
    return;
  }
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
    openerMsg(t("dialog.open.dirError", { msg: serverMessage(e) }));
  }
}

export function renderBrowse(res) {
  state.openerDir = res.dir;
  state.openerEntries = res.entries || [];
  try {
    localStorage.setItem(BROWSE_KEY, res.dir);
  } catch {
    // Browser storage is only a convenience; browsing still works without it.
  }
  renderCwdCrumbs(res.dir);
  // Folder mode: keep the editable path field tracking the folder in view, so
  // "Choose Folder" and a typed override agree on what's being picked.
  if (state.openerMode === "folder") $("opener-input").value = res.dir;
  const list = $("opener-list");
  list.textContent = "";
  if (res.parent) {
    list.append(browseRow({ name: "..", path: res.parent, is_dir: true }, true));
  }
  for (const ent of res.entries) list.append(browseRow(ent, false));
  list.scrollTop = 0;
  resetOpenerSelection();
}

// The server's virtual drive-list level ("PC" in Windows Explorer terms);
// must match DRIVES_DIR in serve/workspace.rs.
export const DRIVES_DIR = "::";

// Breadcrumb renderer (issue #81.3): clears `host` and fills it with clickable
// path segments, each invoking `onNavigate(segmentPath)`.
export function renderPathCrumbs(host: HTMLElement, path, onNavigate: (p: string) => void) {
  const clean = String(path || "").replace(/^\\\\\?\\/, "");
  host.textContent = "";
  host.title = clean;
  let crumbs = pathCrumbs(clean);
  if (clean === DRIVES_DIR) {
    crumbs = [{ label: t("dialog.open.thisPc"), path: DRIVES_DIR }];
  } else if (/^[A-Za-z]:[\\/]/.test(clean)) {
    // Windows: a "PC" root crumb in front of the drive, so other drives are
    // one click away (the drive list is also the ".." of every drive root).
    crumbs = [{ label: t("dialog.open.thisPc"), path: DRIVES_DIR }, ...crumbs];
  }
  for (const [i, crumb] of crumbs.entries()) {
    if (i > 0) {
      const sep = document.createElement("span");
      sep.className = "cwd-sep";
      sep.setAttribute("aria-hidden", "true");
      sep.append(iconSvg("i-chevron-right"));
      host.append(sep);
    }
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "cwd-crumb";
    btn.textContent = crumb.label;
    btn.title = crumb.path;
    btn.addEventListener("click", () => onNavigate(crumb.path));
    host.append(btn);
  }
}

export function renderCwdCrumbs(path) {
  renderPathCrumbs($("opener-cwd"), path, browse);
}

export function browseRow(ent, isUp) {
  const row = document.createElement("button");
  row.className = "opener-row" + (ent.is_dir ? " dir" : "") + (isUp ? " up" : "");
  row.type = "button";
  prepareOpenerOption(row);
  // The kind ("フォルダ" / "ファイル") moved from visible text into the icon;
  // keep it for screen readers via the row's accessible name.
  row.setAttribute(
    "aria-label",
    isUp
      ? t("dialog.open.up")
      : `${ent.is_dir ? t("dialog.open.folder") : t("menu.file")}: ${ent.name}`,
  );
  const ic = document.createElement("span");
  ic.className = "ic";
  ic.setAttribute("aria-hidden", "true");
  ic.append(iconSvg(isUp ? "i-folder-up" : ent.is_dir ? "i-folder" : "i-file"));
  const nm = document.createElement("span");
  nm.className = "nm";
  nm.textContent = isUp ? t("dialog.open.up") : ent.name;
  const sz = document.createElement("span");
  sz.className = "sz";
  sz.textContent = ent.is_dir ? "" : humanBytes(ent.size, currentLocale());
  row.append(ic, nm, sz);
  row.addEventListener("click", () => {
    if (ent.is_dir) browse(ent.path);
    else if (state.openerMode === "save") {
      $("opener-input").value = ent.name;
      markPickedFile(ent.name);
      $("opener-input").focus();
    } else if (state.openerMode === "open") openPath(ent.path);
    // folder mode: files are not selectable targets — ignore the click.
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
  return loadRecentFilesShared();
}

export function saveRecentFiles(list) {
  saveRecentFilesShared(list);
}

// Record a freshly opened file at the head of the list. Untitled scratch
// buffers never qualify.
export function pushRecentFile(path) {
  const p = (path || "").trim();
  if (!p || isUntitled(p)) return;
  const list = [p, ...loadRecentFiles().filter((x) => x !== p)];
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
  // The recent-files shortcut only makes sense when opening a file — not when
  // saving or picking a folder target.
  const list = state.openerMode === "open" ? loadRecentFiles() : [];
  box.textContent = "";
  resetOpenerSelection();
  if (!list.length) {
    box.classList.add("hidden");
    return;
  }
  const head = document.createElement("div");
  head.className = "opener-recent-head";
  head.setAttribute("aria-hidden", "true");
  head.textContent = t("dialog.open.recent");
  box.append(head);
  for (const path of list) box.append(recentRow(path));
  box.classList.remove("hidden");
}

export function recentRow(path) {
  const row = document.createElement("button");
  row.className = "opener-row recent";
  row.type = "button";
  prepareOpenerOption(row);
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
  if (!isAbsolutePath(raw) && state.openerDir === DRIVES_DIR) {
    // The drive list is not a directory; a bare file name has nowhere to go.
    openerMsg(t("dialog.open.pickFolderFirst"));
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
  if (state.openerMode === "folder") {
    const raw = $("opener-input").value.trim();
    const dir = raw && isAbsolutePath(raw) ? raw : state.openerDir;
    if (!dir || dir === DRIVES_DIR) {
      openerMsg(t("dialog.open.pickFolderFirst"));
      return;
    }
    finishFolderDialog(dir);
    return;
  }
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
    reportOpenError(t("error.cannotOpen", { msg: serverMessage(e) }));
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
    reportOpenError(t("error.loadErrorMsg", { msg: serverMessage(e) }));
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
const TAB_DRAG_TYPE = "application/x-ayame-tab";
let draggingTabId: number | null = null;
let localDropHandledId: number | null = null;

export function tabOrderAfterMove(list, id, beforeId) {
  const next = [...list];
  const from = next.findIndex((tab) => tab.id === id);
  if (from < 0) return null;
  if (beforeId === id) return next;
  if (beforeId != null && !next.some((tab) => tab.id === beforeId)) return null;
  const [moved] = next.splice(from, 1);
  const to = beforeId == null ? next.length : next.findIndex((tab) => tab.id === beforeId);
  next.splice(to, 0, moved);
  return next;
}

export function tabDropBeforeId(list, draggedId, targetId, after) {
  const draggedIndex = list.findIndex((tab) => tab.id === draggedId);
  if (draggedIndex < 0) return null;
  if (draggedId === targetId) return list[draggedIndex + 1]?.id ?? null;

  const remaining = list.filter((tab) => tab.id !== draggedId);
  const targetIndex = remaining.findIndex((tab) => tab.id === targetId);
  if (targetIndex < 0) return null;
  return remaining[targetIndex + (after ? 1 : 0)]?.id ?? null;
}

export function ensureActiveTabVisible(container = document.getElementById("tabs")) {
  container
    ?.querySelector<HTMLElement>(".tab.active")
    ?.scrollIntoView?.({ block: "nearest", inline: "nearest" });
}

export function showTabList() {
  const button = $("tab-list");
  const rect = button.getBoundingClientRect();
  showPopupMenu(
    rect.right,
    rect.bottom,
    (state.tabs || []).map((tab) => {
      const folder = tab.path && !isUntitled(tab.path) ? pathDirName(displayPath(tab.path)) : "";
      return {
        label: folder ? `${tab.name} — ${folder}` : tab.name,
        checked: !!tab.active,
        action: () => {
          if (!tab.active) selectTab(tab.id);
        },
      };
    }),
  );
}

export async function reorderTab(id, beforeId) {
  const reordered = tabOrderAfterMove(state.tabs || [], id, beforeId);
  if (!reordered) return false;
  if (reordered.every((tab, index) => tab.id === state.tabs[index]?.id)) return true;
  try {
    const response = await apiPost<TabsResponse, TabReorderRequest>("/api/tabs/reorder", {
      id,
      before_id: beforeId,
    });
    renderTabs(response.tabs);
    return true;
  } catch (error) {
    flashCount(t("tab.reorderError"), "error");
    console.error(error);
    return false;
  }
}

function eventTab(e: Event) {
  return e.target instanceof Element ? e.target.closest<HTMLElement>(".tab") : null;
}

function beforeIdAtPointer(e: DragEvent, draggedId: number) {
  const target = eventTab(e);
  if (!target) return null;
  const box = target.getBoundingClientRect();
  return tabDropBeforeId(
    state.tabs || [],
    draggedId,
    Number(target.dataset.id),
    e.clientX >= box.left + box.width / 2,
  );
}

function clearTabDropTarget() {
  for (const tab of document.querySelectorAll(".tab-drop-before, .tab-drop-after")) {
    tab.classList.remove("tab-drop-before", "tab-drop-after");
  }
}

function markTabDropTarget(e: DragEvent, draggedId: number) {
  clearTabDropTarget();
  const target = eventTab(e);
  if (!target || Number(target.dataset.id) === draggedId) return;
  const box = target.getBoundingClientRect();
  target.classList.add(
    e.clientX >= box.left + box.width / 2 ? "tab-drop-after" : "tab-drop-before",
  );
}

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

// Closing mutates the server's active tab and the local tab list. Keep those
// transitions ordered so a slow confirmation or close response cannot race a
// later close against stale state (#123).
export async function closeTabsSequentially(ids, close = closeTab) {
  for (const id of ids) await close(id);
}

export function renderTabs(list) {
  state.tabs = list;
  const c = $("tabs");
  const listButton = document.getElementById("tab-list") as HTMLButtonElement | null;
  if (listButton) listButton.disabled = list.length === 0;
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
    el.draggable = true;
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
    // Right-click a tab for tab-scoped actions instead of the webview default menu.
    el.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      showPopupMenu(e.clientX, e.clientY, [
        { label: t("common.close"), action: () => closeTab(tab.id) },
        {
          label: t("menu.saveAs"),
          action: async () => {
            if (!tab.active) await selectTab(tab.id);
            saveCopy();
          },
        },
        {
          label: t("tab.copyPath"),
          disabled: !tab.path,
          action: () => {
            navigator.clipboard?.writeText(displayPath(tab.path)).catch(() => {});
          },
        },
        { separator: true },
        {
          label: t("tab.closeOthers"),
          disabled: state.tabs.length < 2,
          action: async () => {
            // Snapshot ids first — closeTab mutates state.tabs as it runs.
            // Close sequentially: each closeTab settles the edit queue,
            // POSTs, and re-renders; firing N of them concurrently races
            // docGen bumps and briefly "opens" intermediate tabs.
            const others = state.tabs.filter((o) => o.id !== tab.id).map((o) => o.id);
            await closeTabsSequentially(others);
          },
        },
      ]);
    });
    el.addEventListener("dragstart", (e) => startTabDrag(e, tab));
    el.addEventListener("dragend", (e) => finishTabDrag(e, tab));
    x.addEventListener("click", (e) => {
      e.stopPropagation();
      closeTab(tab.id);
    });
    c.append(el);
  }
  ensureActiveTabVisible(c);
}

export function tabDragPayload(tab) {
  return {
    sourceWindowId: state.windowId,
    id: tab.id,
    path: tab.path,
    name: tab.name,
    dirty: !!tab.dirty,
  };
}

export function startTabDrag(e, tab) {
  if (!e.dataTransfer) return;
  if (e.target instanceof Element && e.target.closest(".tab-x")) {
    e.preventDefault();
    return;
  }
  draggingTabId = tab.id;
  if (e.currentTarget instanceof HTMLElement) e.currentTarget.classList.add("dragging");
  e.dataTransfer.effectAllowed = "move";
  e.dataTransfer.setData(TAB_DRAG_TYPE, JSON.stringify(tabDragPayload(tab)));
  e.dataTransfer.setData("text/plain", tab.path || tab.name || "");
}

// A dirty tab can move between windows only in the native build (each window
// is its own process/server) and only for a real on-disk file: the handoff
// rides the crash log — /api/tabs/detach keeps + fsyncs it, the adopting
// window replays it — and that log is keyed by the file's canonical path.
// Untitled buffers live in pid-scoped scratch dirs, so they have no stable
// cross-process key and must stay put. Browser windows on one shared server
// have no per-window tab ownership, so the old refusal stands there too.
export function canHandoffDirtyTab(tab) {
  return isNativeApp() && !!tab?.path && !isUntitled(tab.path);
}

// A tab can be torn out into its own window when it is an on-disk file; a
// dirty one additionally needs the handoff path above. Fileless/untitled
// buffers have nothing another process could reopen and must stay put rather
// than be dropped on the floor.
export function canDragOutToNewWindow(tab) {
  if (!tab || !tab.path || isUntitled(tab.path)) return false;
  return !tab.dirty || canHandoffDirtyTab(tab);
}

export async function finishTabDrag(e, tab) {
  clearTabDropTarget();
  if (e.currentTarget instanceof HTMLElement) e.currentTarget.classList.remove("dragging");
  draggingTabId = null;
  if (localDropHandledId === tab.id) {
    localDropHandledId = null;
    return;
  }
  if (savingCount > 0) return;
  const dropped = e.dataTransfer?.dropEffect === "move";
  const outside =
    e.clientX < 0 ||
    e.clientY < 0 ||
    e.clientX > window.innerWidth ||
    e.clientY > window.innerHeight;
  if (tab.dirty && !canHandoffDirtyTab(tab)) {
    if (dropped || outside) flashCount(t("tab.moveDirty"), "error");
    return;
  }
  if (dropped) {
    // Another Ayame window accepted the drop and (re)opens the path itself.
    // A dirty tab detaches — its crash log carries the unsaved edits to the
    // adopting window — while a clean one closes outright.
    if (tab.dirty) await detachMovedTab(tab.id);
    else await closeMovedTab(tab.id);
  } else if (outside) {
    // Dragging outside the window tears the tab out into its own window
    // (a fresh process for that file); keep fileless/untitled buffers here.
    if (!canDragOutToNewWindow(tab)) return;
    if (tab.dirty) {
      // Handoff order matters: detach FIRST (makes the log durable and
      // releases it), then spawn the adopting window, and only then finish
      // any close of this one — the spawn IPC must reach the native side
      // while this process is still alive.
      let stat;
      try {
        stat = await apiPost<{ open: boolean }, TabIdRequest>("/api/tabs/detach", { id: tab.id });
      } catch (err) {
        flashCount(t("tab.handoffError"), "error");
        console.error(err);
        return;
      }
      openNewWindow(tab.path, true);
      if (!stat.open) requestEditorClose();
      else onDocumentOpened(stat);
    } else {
      openNewWindow(tab.path);
      await closeMovedTab(tab.id);
    }
  }
}

export function initTabDropTarget() {
  const c = $("tabs");
  const acceptsDirty = (payload) => isNativeApp() && !!payload.path && !isUntitled(payload.path);
  c.addEventListener("dragover", (e) => {
    if (draggingTabId != null) {
      e.preventDefault();
      if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
      markTabDropTarget(e, draggingTabId);
      return;
    }
    const raw = e.dataTransfer?.getData(TAB_DRAG_TYPE);
    if (!raw) return;
    const payload = parseTabDragPayload(raw);
    if (!payload || payload.sourceWindowId === state.windowId) return;
    if (payload.dirty && !acceptsDirty(payload)) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = "move";
  });
  c.addEventListener("dragleave", (e) => {
    if (draggingTabId != null && !c.contains(e.relatedTarget as Node | null)) {
      clearTabDropTarget();
    }
  });
  c.addEventListener("drop", async (e) => {
    if (draggingTabId != null) {
      const id = draggingTabId;
      const beforeId = beforeIdAtPointer(e, id);
      e.preventDefault();
      localDropHandledId = id;
      clearTabDropTarget();
      await reorderTab(id, beforeId);
      return;
    }
    const payload = parseTabDragPayload(e.dataTransfer?.getData(TAB_DRAG_TYPE));
    if (!payload || payload.sourceWindowId === state.windowId) return;
    e.preventDefault();
    if (payload.dirty) {
      if (!acceptsDirty(payload)) {
        flashCount(t("tab.moveDirty"), "error");
        return;
      }
      // Adopt a dirty tab: the source detaches on its dragend (right after
      // this event fires), leaving its fsynced crash log behind. Give that a
      // moment, then open the path — the registered handoff makes the
      // recoverable log replay without the crash prompt.
      expectWalHandoff(payload.path);
      await new Promise((resolve) => setTimeout(resolve, 250));
      await openPath(payload.path);
      return;
    }
    await openPath(payload.path);
  });
}

// Like closeMovedTab, but through /api/tabs/detach: the tab's crash log is
// kept (and fsynced) so the adopting window can replay the unsaved edits.
export async function detachMovedTab(id) {
  try {
    const stat = await apiPost<{ open: boolean }, TabIdRequest>("/api/tabs/detach", { id });
    if (!stat.open) {
      requestEditorClose();
    } else {
      onDocumentOpened(stat);
    }
  } catch (e) {
    flashCount(t("tab.handoffError"), "error");
    console.error(e);
  }
}

export function parseTabDragPayload(raw) {
  try {
    const payload = JSON.parse(raw || "{}");
    if (!payload || typeof payload.path !== "string" || !payload.path.trim()) return null;
    return payload;
  } catch {
    return null;
  }
}

export async function closeMovedTab(id) {
  try {
    const stat = await apiPost<{ open: boolean }, TabIdRequest>("/api/tabs/close", { id });
    if (!stat.open) {
      requestEditorClose();
    } else {
      onDocumentOpened(stat);
    }
  } catch (e) {
    flashCount(t("tab.closeError"));
    console.error(e);
  }
}

export async function selectTab(id) {
  try {
    await settleEditQueue();
    onDocumentOpened(await apiPost<unknown, TabIdRequest>("/api/tabs/select", { id }));
    return true;
  } catch (e) {
    flashCount(t("tab.switchError"));
    console.error(e);
    return false;
  }
}

// Switch to the tab `delta` positions away from the active one, wrapping around
// the ends (issue #79). Ctrl+PageDown/PageUp bind here; the mouse is no longer
// the only way to change tabs.
export function selectRelativeTab(delta) {
  const tabs = state.tabs || [];
  if (tabs.length < 2) return;
  const active = tabs.findIndex((t) => t.active);
  const from = active < 0 ? 0 : active;
  const next = tabs[(from + delta + tabs.length) % tabs.length];
  if (next && !next.active) selectTab(next.id);
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
    openerMsg(t("error.newBuffer", { msg: serverMessage(e) }));
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
    openFileDialog();
  });
  $("opener-close").addEventListener("click", hideOpener);
  $("opener-new").addEventListener("click", () => {
    // Escape the welcome screen straight into a fresh buffer (#174).
    setModalOpen($("opener"), false);
    newUntitled();
  });
  $("opener-open").addEventListener("click", commitOpener);
  $("opener-input").addEventListener("keydown", onOpenerInputKeydown);
  for (const id of ["opener-recent", "opener-list"]) {
    $(id).addEventListener("focus", onOpenerListFocus);
    $(id).addEventListener("keydown", onOpenerListKeydown);
  }
  // Click on the dim backdrop (outside the panel) closes the dialog.
  $("opener").addEventListener("click", (e) => {
    if (e.target === $("opener")) hideOpener();
  });
  $("new-tab").addEventListener("click", () => newUntitled());
  $("tab-list").addEventListener("click", showTabList);
  initTabDropTarget();
  initDropZone();
}
