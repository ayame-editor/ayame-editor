// Ayame Editor — opener modal, open/save targets, uploads, and document activation.
import {
  $,
  isAbsolutePath,
  isUntitled,
  joinPath,
  pathBaseName,
  pathDirName,
  setModalOpen,
} from "./dom.js";
import { BROWSE_KEY, state } from "./state.js";
import { serverMessage, t } from "./i18n.js";
import { api, apiPost } from "./api.js";
import { isNativeApp, nativeOpenDialog, nativeSaveDialog } from "./app.js";
import { maybeOfferWalRecovery, noteWalError } from "./save.js";
import {
  clearLineCache,
  focusEditor,
  render,
  setActiveLine,
  setCaret,
  setSearchHits,
  setSelection,
} from "./editor.js";
import { updateStatusMeta } from "./status.js";
import { setFollowTail, settleEditQueue } from "./edits.js";
import { flashCount } from "./notifications.js";
import { askConfirm, hideLoading, showLoading, showMessage } from "./dialogs.js";
import {
  browse,
  DRIVES_DIR,
  handleOpenerListKeydown,
  moveOpenerSelection,
  onOpenerListFocus,
  openerMsg,
  resetOpenerSelection,
} from "./browse.js";
import { pushRecentFile, renderRecentFiles } from "./recent.js";
import { refreshTabs } from "./tabs.js";
import type {
  OpenRequest,
  OpenResponse,
  StatResponse,
  TabIdRequest,
  TabsResponse,
} from "./types/api.js";
import {
  currentOpenerMode,
  resolveOpener,
  setOpenerMode,
  setOpenerResolver,
} from "./opener-state.js";
import type { SaveDialogTarget } from "./opener-state.js";

export function onOpenerListKeydown(event) {
  handleOpenerListKeydown(event, hideOpener);
}

export function onOpenerInputKeydown(event) {
  if (event.key === "ArrowDown" || event.key === "ArrowUp") {
    event.preventDefault();
    moveOpenerSelection(event.key === "ArrowDown" ? 1 : -1);
  } else if (event.key === "Enter") {
    event.preventDefault();
    commitOpener();
  }
}

export function openerVisible() {
  return !$("opener").classList.contains("hidden");
}

export function showOpener() {
  configureOpener("open");
  setModalOpen($("opener"), true);
  void browse(null);
  const input = $("opener-input");
  input.value = "";
  queueMicrotask(() => input.focus());
}

export async function openFileDialog() {
  if (!isNativeApp()) {
    showOpener();
    return;
  }
  const current = state.doc.stat?.path || "";
  const directory = !isUntitled(current) ? pathDirName(current) || "" : "";
  const paths = await nativeOpenDialog(directory);
  for (const path of paths) await openPath(path);
}

export function showSaveDialog(
  title: string,
  suggestedPath: string,
): Promise<SaveDialogTarget | null> {
  if (isNativeApp()) {
    return nativeSaveDialog(
      pathDirName(suggestedPath) || "",
      pathBaseName(suggestedPath) || "untitled.txt",
    ).then((path) => (path ? { path, overwrite: true } : null));
  }
  return new Promise<SaveDialogTarget | null>((resolve) => {
    configureOpener("save", title);
    setOpenerResolver<SaveDialogTarget | null>((value) => resolve(value));
    const input = $("opener-input");
    const directory = pathDirName(suggestedPath) || localStorage.getItem(BROWSE_KEY) || ".";
    input.value = pathBaseName(suggestedPath) || "untitled.txt";
    setModalOpen($("opener"), true);
    void browse(directory);
    queueMicrotask(() => {
      input.focus();
      input.select();
    });
  });
}

export function showFolderDialog(title, startDir): Promise<string | null> {
  return new Promise<string | null>((resolve) => {
    configureOpener("folder", title);
    setOpenerResolver<string | null>((value) => resolve(value));
    setModalOpen($("opener"), true);
    void browse(startDir || localStorage.getItem(BROWSE_KEY) || null);
    queueMicrotask(() => ($("opener-open") as HTMLButtonElement).focus());
  });
}

export function finishFolderDialog(value: string | null) {
  setOpenerMode("open");
  setModalOpen($("opener"), false);
  configureOpener("open");
  resolveOpener(value);
}

export function configureOpener(mode, title?) {
  setOpenerMode(mode);
  const save = mode === "save";
  const folder = mode === "folder";
  const modal = $("opener");
  modal.classList.toggle("save-mode", save);
  modal.classList.toggle("folder-mode", folder);
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
  $("opener-new").classList.toggle("hidden", save || folder);
  $("opener-hint").textContent = t(
    save ? "dialog.open.hintSave" : folder ? "dialog.open.hintFolder" : "dialog.open.hintOpen",
  );
  openerMsg("");
  renderRecentFiles();
}

export function hideOpener() {
  resetOpenerSelection();
  if (currentOpenerMode() === "save") {
    finishSaveDialog(null);
    return;
  }
  if (currentOpenerMode() === "folder") {
    finishFolderDialog(null);
    return;
  }
  setModalOpen($("opener"), false);
  if (!state.doc.stat?.open) {
    void newUntitled();
    return;
  }
  focusEditor();
}

export function finishSaveDialog(value: SaveDialogTarget | null) {
  setOpenerMode("open");
  setModalOpen($("opener"), false);
  configureOpener("open");
  focusEditor();
  resolveOpener(value);
}

export async function saveDialogTarget() {
  const raw = $("opener-input").value.trim();
  if (!raw) {
    openerMsg(t("dialog.open.enterFileName"));
    return null;
  }
  if (!isAbsolutePath(raw) && state.opener.dir === DRIVES_DIR) {
    openerMsg(t("dialog.open.pickFolderFirst"));
    return null;
  }
  const path = isAbsolutePath(raw) ? raw : joinPath(state.opener.dir, raw);
  const base = pathBaseName(path);
  const existing = state.opener.entries.find((entry) => !entry.is_dir && entry.name === base);
  const overwrite = !!existing;
  if (overwrite) {
    const ok = await askConfirm(
      t("dialog.overwrite.title"),
      t("dialog.overwrite.ask", { name: base }),
      { okLabel: t("dialog.overwrite.ok"), danger: true },
    );
    if (!ok) return null;
  }
  return { path, overwrite };
}

export async function commitOpener() {
  if (currentOpenerMode() === "folder") {
    const raw = $("opener-input").value.trim();
    const directory = raw && isAbsolutePath(raw) ? raw : state.opener.dir;
    if (!directory || directory === DRIVES_DIR) {
      openerMsg(t("dialog.open.pickFolderFirst"));
      return;
    }
    finishFolderDialog(directory);
    return;
  }
  if (currentOpenerMode() === "save") {
    const target = await saveDialogTarget();
    if (target) finishSaveDialog(target);
    return;
  }
  const raw = $("opener-input").value.trim();
  if (!raw) return;
  void openPath(isAbsolutePath(raw) ? raw : joinPath(state.opener.dir, raw));
}

export function pristineUntitledTabId() {
  const active = (state.doc.tabs || []).find((tab) => tab.active);
  if (!active || active.dirty || !isUntitled(active.path)) return null;
  if (state.doc.stat?.dirty || state.doc.stat?.can_undo) return null;
  return active.id;
}

export async function closeTabSilently(id) {
  if (id == null) return;
  try {
    const response = await api<TabsResponse>("/api/tabs");
    const tab = (response.tabs || []).find((entry) => entry.id === id);
    if (!tab || tab.active || tab.dirty) return;
    await apiPost<unknown, TabIdRequest>("/api/tabs/close", { id });
    void refreshTabs();
  } catch {
    // Non-fatal: the extra tab just stays.
  }
}

export async function openPath(path) {
  const value = (path || "").trim();
  if (!value) return false;
  await settleEditQueue();
  const pristine = pristineUntitledTabId();
  openerMsg(t("dialog.open.opening"), true);
  showLoading(t("dialog.open.loadingFile", { name: pathBaseName(value) || value }));
  try {
    const stat = await apiPost<OpenResponse, OpenRequest>("/api/open", { path: value });
    onDocumentOpened(stat);
    await closeTabSilently(pristine);
    return true;
  } catch (error) {
    reportOpenError(t("error.cannotOpen", { msg: serverMessage(error) }));
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
    const response = await fetch(`/api/upload?name=${encodeURIComponent(file.name)}`, {
      method: "POST",
      body: file,
    });
    if (!response.ok) throw new Error((await response.text()) || response.statusText);
    onDocumentOpened(await response.json());
    await closeTabSilently(pristine);
  } catch (error) {
    reportOpenError(t("error.loadErrorMsg", { msg: serverMessage(error) }));
  } finally {
    hideLoading();
  }
}

export async function uploadFiles(files) {
  for (const file of Array.from(files || [])) {
    if (file) await uploadFile(file);
  }
}

export function reportOpenError(message) {
  if (openerVisible()) {
    openerMsg(message);
  } else if (state.doc.stat?.open) {
    flashCount(t("error.loadError"), "error");
    showMessage(t("error.loadError"), message);
  } else {
    showOpener();
    openerMsg(message);
  }
}

export function onDocumentOpened(stat: StatResponse) {
  if (!stat.open || !stat.path) return;
  state.doc.generation++;
  state.caret.editGeneration++;
  setFollowTail(false);
  state.doc.stat = stat;
  pushRecentFile(stat.path);
  state.view.total = stat.view_lines ?? stat.lines ?? 0;
  state.view.first = 0;
  state.caret.position = { line: 0, col: 0 };
  state.caret.goalCol = 0;
  setActiveLine(0);
  setSelection(null);
  state.caret.extraCursors = [];
  state.search.lastMatch = null;
  setSearchHits(null);
  state.search.truncated = false;
  $("find-count").textContent = "";
  clearLineCache();
  setModalOpen($("opener"), false);
  updateStatusMeta();
  render();
  void refreshTabs();
  focusEditor();
  noteWalError(stat);
  maybeOfferWalRecovery(stat);
  void import("./analysis.js").then(({ handleAnalysisDocumentOpened }) =>
    handleAnalysisDocumentOpened(stat.path),
  );
}

export function hasFiles(event) {
  const transfer = event.dataTransfer;
  return !!transfer && Array.from(transfer.types || []).includes("Files");
}

export function initDropZone() {
  const dropzone = $("dropzone");
  let depth = 0;
  window.addEventListener("dragenter", (event) => {
    if (!hasFiles(event)) return;
    event.preventDefault();
    depth++;
    dropzone.classList.remove("hidden");
  });
  window.addEventListener("dragover", (event) => {
    if (!hasFiles(event)) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "copy";
  });
  window.addEventListener("dragleave", (event) => {
    if (!hasFiles(event)) return;
    depth = Math.max(0, depth - 1);
    if (depth === 0) dropzone.classList.add("hidden");
  });
  window.addEventListener("drop", (event) => {
    if (!hasFiles(event)) return;
    event.preventDefault();
    depth = 0;
    dropzone.classList.add("hidden");
    void uploadFiles(event.dataTransfer.files);
  });
}

export async function newUntitled() {
  try {
    await settleEditQueue();
    onDocumentOpened(await apiPost<StatResponse>("/api/new", {}));
    setCaret(0, 0);
    focusEditor();
  } catch (error) {
    showOpener();
    openerMsg(t("error.newBuffer", { msg: serverMessage(error) }));
  }
}

export function initOpener() {
  $("opener-close").addEventListener("click", hideOpener);
  $("opener-new").addEventListener("click", () => {
    setModalOpen($("opener"), false);
    void newUntitled();
  });
  $("opener-open").addEventListener("click", () => void commitOpener());
  $("opener-input").addEventListener("keydown", onOpenerInputKeydown);
  for (const id of ["opener-recent", "opener-list"]) {
    $(id).addEventListener("focus", onOpenerListFocus);
    $(id).addEventListener("keydown", onOpenerListKeydown);
  }
}
