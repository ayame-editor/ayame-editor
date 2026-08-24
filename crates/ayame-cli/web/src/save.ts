// Ayame Editor — save module. Type-stripped to JS at build time (build.rs, oxc).
import { $, commas, displayPath, isUntitled, joinPath, pathDirName, setModalOpen } from "./dom.js";
import { conversionEncodingValue, encodingSupportsBom } from "./encodings.js";
import { DEFAULT_SETTINGS, state } from "./state.js";
import { currentLocale, serverMessage, t, weekdayNames } from "./i18n.js";
import {
  api,
  apiPost,
  isApiErrorCode,
  type MarkerSaveRequest,
  type MarkerSaveResponse,
} from "./api.js";
import { dirtyCloseMessage, hasDirtyDocuments, postNativeMessage } from "./app.js";
import { clearLineCache, focusEditor, render, setCaret } from "./editor.js";
import { enc, eol, updateStatusMeta } from "./status.js";
import { reloadViewport, setEditSaveService, settleEditQueue } from "./edits.js";
import { flashCount } from "./notifications.js";
import { lastGrep } from "./grep-state.js";
import {
  askConfirm,
  askForm,
  CONFIRM_ALT,
  hideLoading,
  newOperationId,
  showLoading,
  showMessage,
} from "./dialogs.js";
import {
  beaconSessionSnapshot,
  migrateSyntaxOverrideShared,
  saveSessionSnapshot,
} from "./persistence.js";
import { withOverwriteRetry } from "./saveflow.js";
import { clearAllActiveFolds, migrateFoldDocument } from "./fold-state.js";
import type {
  ArtifactResponse,
  BrowseResponse,
  DiskCheckResponse,
  EditSaveRequest,
  EditSaveResponse,
  GrepSaveRequest,
  OpenRequest,
  OpenResponse,
  RecoverRequest,
  ReopenRequest,
  StatResponse,
  SortSaveRequest,
  SplitSaveRequest,
} from "./types/api.js";

let onDocumentOpened = (_stat) => {};
let openPath = async (_path) => {};
let refreshTabs = async () => {};
let selectTab = async (_id) => {};
let showSaveDialog = async (_title, _suggestedPath) => null;
let saveSettings = (_settings) => false;

export function setSaveWorkspaceService(service) {
  onDocumentOpened = service.onDocumentOpened;
  openPath = service.openPath;
  refreshTabs = service.refreshTabs;
  selectTab = service.selectTab;
  showSaveDialog = service.showSaveDialog;
}

export function setSaveSettingsWriter(writer) {
  saveSettings = writer;
}

// Never let the native window kill the process while a save is in flight; the
// close request is answered "cancel" and retried once the save settles.
// While saving: key edits are blocked (onEditKey) and IME/beforeinput commits
// wait inside enqueueEdit so confirmed text is delayed, not lost.
export let savingCount = 0;

export let savingWaiters = [];

function editSaveRequest(req: Partial<EditSaveRequest>): EditSaveRequest {
  return {
    path: null,
    overwrite: false,
    switch_to_saved: false,
    encoding: null,
    eol: null,
    bom: null,
    force: false,
    ...req,
  };
}

export function setSavingUI() {
  const on = savingCount > 0;
  document.documentElement.classList.toggle("saving", on);
  $("st-saving")?.classList.toggle("hidden", !on);
  if (!on && savingWaiters.length) {
    const waiters = savingWaiters;
    savingWaiters = [];
    for (const resolve of waiters) resolve();
  }
}

export function waitForSavingDone(): Promise<void> {
  if (savingCount === 0) return Promise.resolve();
  return new Promise((resolve) => savingWaiters.push(resolve));
}

export let pendingNativeClose = false;

async function confirmNativeCloseOk() {
  await saveSessionSnapshot();
  postNativeMessage({ type: "close_confirmed" });
}

function onNativeCloseRequested() {
  if (savingCount > 0) {
    pendingNativeClose = true;
    flashCount(t("dialog.exit.savingWillClose"));
    postNativeMessage({ type: "close_canceled" });
    return;
  }
  if (!hasDirtyDocuments()) {
    void confirmNativeCloseOk();
    return;
  }
  // Release the native close request first (it times out after a few
  // seconds), then ask with the in-app dialog; a confirmed close posts the
  // ok separately — the Rust side exits on it regardless of timing.
  postNativeMessage({ type: "close_canceled" });
  askConfirm(t("dialog.exit.title"), dirtyCloseMessage(), {
    okLabel: t("dialog.exit.withoutSaving"),
    danger: true,
  }).then((ok) => {
    if (ok) void confirmNativeCloseOk();
  });
}

function onPageHide() {
  // A plain fetch is aborted as the page unloads; use sendBeacon so the final
  // snapshot survives (issue #73). Fall back to the async save only if the
  // browser refuses the beacon.
  if (!beaconSessionSnapshot()) void saveSessionSnapshot();
}

function onBeforeUnload(e) {
  if (!hasDirtyDocuments()) return;
  e.preventDefault();
  e.returnValue = "";
}

let saveInitialized = false;

// Register cross-module services and browser lifecycle hooks explicitly during
// boot so importing save helpers has no global side effects.
export function initSave() {
  if (saveInitialized) return;
  saveInitialized = true;
  setEditSaveService({
    refreshStat,
    savingCount: () => savingCount,
    waitForSavingDone,
  });
  window.__ayameNativeCloseRequested = onNativeCloseRequested;
  window.addEventListener("pagehide", onPageHide);
  window.addEventListener("beforeunload", onBeforeUnload);
  // Coming back to the editor is when an external rewrite is worth reporting:
  // the user has just been somewhere else, which is where the rewrite came
  // from. `visibilitychange` covers the browser build, where a background tab
  // never sees a window focus event (#163).
  window.addEventListener("focus", () => void checkExternalChange());
  document.addEventListener("visibilitychange", () => {
    if (!document.hidden) void checkExternalChange();
  });
}

export function retryPendingNativeClose() {
  if (pendingNativeClose && savingCount === 0) {
    pendingNativeClose = false;
    onNativeCloseRequested();
  }
}

export async function refreshStat() {
  state.doc.stat = await api<StatResponse>("/api/stat");
  state.view.total = state.doc.stat.view_lines ?? state.doc.stat.lines;
  noteWalError(state.doc.stat);
  updateStatusMeta();
}

// One-shot warning when the server had to disable its crash log (an I/O
// problem with the log never blocks editing; the stat response carries the
// reason exactly once, so showing it whenever present shows it once).
export function noteWalError(stat) {
  if (stat && stat.wal_error) {
    flashCount(t("recover.walDisabled", { msg: serverMessage(stat.wal_error) }), "error");
  }
}

export async function reloadActiveDocument({
  bumpGeneration = true,
  keepCaret = true,
  refreshTabList = true,
  keepFolds = false,
} = {}) {
  if (bumpGeneration) {
    state.doc.generation++;
    state.caret.editGeneration++;
  }
  if (!keepFolds) clearAllActiveFolds();
  clearLineCache();
  await refreshStat();
  await reloadViewport();
  if (keepCaret)
    setCaret(
      Math.min(state.caret.position.line, Math.max(0, state.view.total - 1)),
      state.caret.position.col,
    );
  render();
  if (refreshTabList) refreshTabs();
}

// Shared tail of every save-as-style save (名前を付けて保存 / クイックメモ保存):
// the current tab becomes the saved file (the server swaps the active tab's
// document to the new path), exactly like every desktop editor — no leftover
// untitled tab, no extra tab for the saved file. Also remembers the folder as
// 前回の保存先 for the next untitled buffer.
interface SaveOptions {
  announce?: boolean;
}

export async function finishSaveAs(res, { announce = true }: SaveOptions = {}) {
  const previousPath = state.doc.stat?.path || "";
  if (res.switched) {
    // Same tab, new document identity: refresh in place, keep the caret.
    migrateSyntaxOverrideShared(previousPath, res.path);
    migrateFoldDocument(previousPath, res.path);
    await reloadActiveDocument({ keepFolds: true });
  } else {
    // The workspace changed while saving (rare): fall back to focusing the
    // saved file — the server dedupes, so this never duplicates a tab.
    onDocumentOpened(await apiPost<OpenResponse, OpenRequest>("/api/open", { path: res.path }));
  }
  rememberSaveDir(res.path);
  if (announce) flashCount(t("file.saved", { path: displayPath(res.path) }));
}

// 前回の保存先: persisted so untitled buffers suggest the folder you last
// saved into (survives restarts).
export function rememberSaveDir(path) {
  const dir = pathDirName(displayPath(path));
  if (!dir) return;
  state.settings = { ...state.settings, lastSaveDir: dir };
  saveSettings(state.settings);
}

export async function saveCopy(options: SaveOptions = {}) {
  const { announce = true } = options;
  if (savingCount > 0) {
    flashCount(t("editor.savingWait"));
    return false;
  }
  await settleEditQueue();
  const target = await showSaveDialog(t("menu.saveAs"), await suggestedSaveAsPath());
  if (!target) return false;
  savingCount++;
  setSavingUI();
  try {
    const res = await withOverwriteRetry(
      target.path,
      (overwrite) =>
        apiPost<EditSaveResponse, EditSaveRequest>(
          "/api/edit/save",
          editSaveRequest({ ...target, overwrite, switch_to_saved: true }),
        ),
      !!target.overwrite,
    );
    if (!res) return false;
    await finishSaveAs(res, { announce });
    return true;
  } catch (e) {
    flashCount(t("error.saveError"), "error");
    showMessage(t("error.saveError"), serverMessage(e));
    return false;
  } finally {
    savingCount--;
    setSavingUI();
    retryPendingNativeClose();
  }
}

export { isExistsError } from "./saveflow.js";

// 名前を付けて保存 opens on the current file's own folder and name (Windows
// standard); untitled buffers suggest the expanded 新規ファイル名 template
// inside 前回の保存先. Async because {seq} numbering needs the folder listing.
export async function suggestedSaveAsPath() {
  const p = state.doc.stat?.path || "";
  if (p && !isUntitled(p)) return p;
  const tpl = state.settings.memoName || DEFAULT_SETTINGS.memoName;
  const dir = (state.settings.lastSaveDir || "").trim();
  let listing = null;
  try {
    listing = await api<BrowseResponse>(`/api/browse?dir=${encodeURIComponent(dir)}`);
  } catch {
    listing = null; // folder gone / unreadable → suggest without collision info
  }
  const taken = new Set(
    ((listing && listing.entries) || []).filter((e) => !e.is_dir).map((e) => e.name),
  );
  const name = expandNameTemplate(tpl, taken).trim() || "untitled.txt";
  const baseDir = (listing && listing.dir) || dir;
  return baseDir ? joinPath(baseDir, name) : name;
}

// Expand the 新規ファイル名 template. Date/time come from the current local
// time; weekday names ({ddd} short / {dddd} long) follow the app language via
// currentLocale(); {seq} (and zero-padded {seq2}/{seq3}/{seq4}) resolve to the
// smallest number not already taken in the target folder (existingNames — a Set
// or array of the folder's file names). {date}=YYYYMMDD, {time}=HHMMSS,
// {datetime}=YYYYMMDD-HHMMSS; {mm}=month, {MM}=minutes (all zero-padded). The
// first parameter is deliberately not named `t` — that is the i18n helper.
export function expandNameTemplate(tpl, existingNames = null) {
  const d = new Date();
  const p2 = (n) => String(n).padStart(2, "0");
  const yyyy = String(d.getFullYear()).padStart(4, "0");
  const wk = weekdayNames(currentLocale());
  const map = {
    "{yyyy}": yyyy,
    "{yy}": yyyy.slice(-2),
    "{mm}": p2(d.getMonth() + 1),
    "{dd}": p2(d.getDate()),
    "{HH}": p2(d.getHours()),
    "{MM}": p2(d.getMinutes()),
    "{ss}": p2(d.getSeconds()),
    "{ddd}": wk.short[d.getDay()],
    "{dddd}": wk.long[d.getDay()],
  };
  map["{date}"] = `${yyyy}${map["{mm}"]}${map["{dd}"]}`;
  map["{time}"] = `${map["{HH}"]}${map["{MM}"]}${map["{ss}"]}`;
  map["{datetime}"] = `${map["{date}"]}-${map["{time}"]}`;
  const base = String(tpl || "").replace(
    /\{(?:yyyy|yy|mm|dd|HH|MM|ss|ddd|dddd|date|time|datetime)\}/g,
    (m) => map[m],
  );
  const taken = existingNames instanceof Set ? existingNames : new Set(existingNames || []);
  // Every {seq*} token in one name resolves to the same number.
  const expandSeq = (n) =>
    base
      .replace(/\{seq4\}/g, String(n).padStart(4, "0"))
      .replace(/\{seq3\}/g, String(n).padStart(3, "0"))
      .replace(/\{seq2\}/g, String(n).padStart(2, "0"))
      .replace(/\{seq\}/g, String(n));
  let name;
  if (/\{seq[234]?\}/.test(base)) {
    name = expandSeq(1); // fallback if every number up to the cap is taken
    for (let n = 1; n <= 9999; n++) {
      const cand = expandSeq(n);
      if (!taken.has(cand)) {
        name = cand;
        break;
      }
    }
  } else {
    name = base;
  }
  // Templates without {seq} keep the classic "-2 / -3 …" suffix on collision.
  if (taken.has(name)) name = freeMemoName(name, taken) || name;
  return name;
}

// "note.txt" taken → "note-2.txt", "note-3.txt", … (before the extension).
// null after 99 collisions — let the save dialog's own numbering take over.
export function freeMemoName(name, taken) {
  if (!taken.has(name)) return name;
  const dot = name.lastIndexOf(".");
  const stem = dot > 0 ? name.slice(0, dot) : name;
  const ext = dot > 0 ? name.slice(dot) : "";
  for (let i = 2; i <= 99; i++) {
    const cand = `${stem}-${i}${ext}`;
    if (!taken.has(cand)) return cand;
  }
  return null;
}

export async function saveFile(options: SaveOptions = {}) {
  const { announce = true } = options;
  if (!state.doc.stat?.open) return false;
  if (savingCount > 0) {
    flashCount(t("editor.savingWait"));
    return false;
  }
  await settleEditQueue();
  if (isUntitled(state.doc.stat.path)) {
    // Untitled buffers always go through the save dialog; the dialog is
    // pre-filled with the expanded name template (see suggestedSaveAsPath).
    return saveCopy({ announce });
  }
  savingCount++;
  setSavingUI();
  try {
    return await saveOverActiveFile(announce, false);
  } finally {
    savingCount--;
    setSavingUI();
    retryPendingNativeClose();
  }
}

// One overwrite attempt of the open file. The server refuses with
// `disk_changed` when somebody else wrote the file after we read it (#163);
// that is a question for the user, not an error, so it is asked and — if they
// choose to overwrite anyway — retried once with `force`.
async function saveOverActiveFile(announce: boolean, force: boolean) {
  try {
    const res = await apiPost<EditSaveResponse, EditSaveRequest>(
      "/api/edit/save",
      editSaveRequest({ overwrite: true, force }),
    );
    clearLineCache();
    await refreshStat();
    await reloadViewport();
    render();
    if (announce) flashCount(t("file.saved", { path: displayPath(res.path) }));
    return true;
  } catch (e) {
    if (!force && isApiErrorCode(e, "disk_changed")) {
      const choice = await askExternalChange("save");
      if (choice === "reload") return await reloadFromDisk();
      if (choice === "overwrite") return await saveOverActiveFile(announce, true);
      return false;
    }
    flashCount(t("error.saveError"), "error");
    showMessage(t("error.saveError"), serverMessage(e));
    return false;
  }
}

// ---- external changes (#163) ------------------------------------------------
//
// Another process rewriting the open file — a build, a log rotation, another
// editor — used to be invisible unless tail-follow happened to be on, so the
// next save silently buried it. The server tracks what the file looked like
// when this session last read or wrote it; the client asks on window focus and
// the server re-checks under its own lock before any overwrite lands.

/// True when the open file has been written by something other than this
/// session. Never throws: a failed probe is reported as "no change" so a
/// transient error cannot pop a dialog.
export async function diskChanged(): Promise<boolean> {
  if (!state.doc.stat?.open) return false;
  try {
    const res = await apiPost<DiskCheckResponse>("/api/disk/check");
    return !!res.open && !!res.changed;
  } catch {
    return false;
  }
}

type ExternalChangeChoice = "reload" | "overwrite" | "dismiss";

// `reason` picks the default action: coming back to the window, reloading is
// what the user almost always wants; interrupted mid-save, they asked to write
// and overwriting stays the primary answer.
async function askExternalChange(reason: "focus" | "save"): Promise<ExternalChangeChoice> {
  const dirty = !!state.doc.stat?.dirty;
  const message = dirty ? t("externalChange.messageDirty") : t("externalChange.message");
  if (reason === "save") {
    const answer = await askConfirm(t("externalChange.title"), message, {
      okLabel: t("externalChange.overwrite"),
      altLabel: t("externalChange.reload"),
      danger: true,
    });
    if (answer === CONFIRM_ALT) return "reload";
    return answer ? "overwrite" : "dismiss";
  }
  const answer = await askConfirm(t("externalChange.title"), message, {
    okLabel: t("externalChange.reload"),
    cancelLabel: t("externalChange.keep"),
    danger: dirty,
  });
  return answer ? "reload" : "dismiss";
}

// Re-read the file from disk, dropping the overlay: `/api/edit/revert` returns
// the tab to the file as it now exists. The caller has already asked.
async function reloadFromDisk() {
  try {
    await apiPost("/api/edit/revert");
    await reloadActiveDocument();
    flashCount(t("externalChange.reloaded"));
    return true;
  } catch (e) {
    flashCount(t("externalChange.reloadError"), "error");
    showMessage(t("externalChange.title"), serverMessage(e));
    return false;
  }
}

// One dialog at a time: focus can bounce (dialog open, alt-tab, native menus)
// and every bounce would otherwise queue another identical question.
let externalChangePending = false;

/// Whether a focus-triggered check is worth making at all. Split out from
/// [`checkExternalChange`] so the skip rules can be exercised on their own.
export function externalChangeWatchable(): boolean {
  if (externalChangePending || savingCount > 0) return false;
  // An untitled buffer's backing file is this session's own scratch: nothing
  // else knows the path, so nothing else can have written it.
  if (!state.doc.stat?.open || isUntitled(state.doc.stat.path)) return false;
  // Tail-follow already polls, reports, and adopts appended bytes; asking on
  // top of it would double up on every line the log gains.
  return !state.doc.followTail;
}

export async function checkExternalChange() {
  if (!externalChangeWatchable()) return;
  externalChangePending = true;
  try {
    if (!(await diskChanged())) return;
    if ((await askExternalChange("focus")) === "reload") await reloadFromDisk();
  } finally {
    externalChangePending = false;
  }
}

type SaveAllTab = { id: number; active?: boolean; dirty?: boolean };

// Save dirty tabs in a deterministic order and restore the user's original
// active tab afterwards. The injected operations keep the ordering contract
// testable without a live server.
export async function saveTabsSequentially(
  tabs: SaveAllTab[],
  select: (id: number) => Promise<boolean | void>,
  save: (tab: SaveAllTab) => Promise<boolean>,
) {
  const dirty = tabs.filter((tab) => tab.dirty);
  const original = tabs.find((tab) => tab.active)?.id ?? null;
  let current = original;
  let saved = 0;

  for (const tab of dirty) {
    if (current !== tab.id) {
      if ((await select(tab.id)) === false) continue;
      current = tab.id;
    }
    if (await save(tab)) saved++;
  }

  if (original != null && current !== original) await select(original);
  return { saved, total: dirty.length };
}

let savingAll = false;

export async function saveAllTabs() {
  if (savingAll || savingCount > 0) {
    flashCount(t("editor.savingWait"));
    return { saved: 0, total: 0 };
  }
  const tabs = [...(state.doc.tabs || [])];
  const total = tabs.filter((tab) => tab.dirty).length;
  if (!total) {
    flashCount(t("status.allSaved"));
    return { saved: 0, total: 0 };
  }

  savingAll = true;
  try {
    const result = await saveTabsSequentially(
      tabs,
      (id) => selectTab(id),
      () => saveFile({ announce: false }),
    );
    await refreshTabs();
    flashCount(
      result.saved === result.total
        ? t("file.savedAll", { n: result.saved })
        : t("file.savedAllPartial", result),
    );
    return result;
  } finally {
    savingAll = false;
  }
}

// ---- 変換して保存 (文字コード / 改行コード) --------------------------------

export function convertVisible() {
  return !$("convert-modal").classList.contains("hidden");
}

export function showConvert() {
  if (!state.doc.stat?.open) return;
  if (isUntitled(state.doc.stat.path)) {
    // Nothing on disk to convert yet — save it first.
    flashCount(t("dialog.convert.saveFirst"));
    saveCopy();
    return;
  }
  // Prefill the pickers with the file's current encoding / line ending. The
  // stat strings are the core enum's kebab-case (Utf8 → "utf8"); map them onto
  // the select's option values.
  $("convert-enc").value = conversionEncodingValue(state.doc.stat.encoding);
  const l = state.doc.stat.eol;
  $("convert-eol").value = ["lf", "crlf", "cr"].includes(l) ? l : "lf";
  // Prefill 「BOMを付ける」 from the file's current BOM, then gray it out unless
  // the chosen 文字コード supports a Unicode BOM.
  $("convert-bom").checked = state.doc.stat.bom_bytes > 0;
  syncConvertBom();
  setModalOpen($("convert-modal"), true);
  queueMicrotask(() => $("convert-enc").focus());
}

// The BOM option only applies to Unicode output; disable it otherwise.
export function syncConvertBom() {
  const encoding = $("convert-enc").value;
  const supportsBom = encodingSupportsBom(encoding);
  $("convert-bom").disabled = !supportsBom;
  $("convert-bom-row").classList.toggle("disabled", !supportsBom);
}

export function hideConvert() {
  setModalOpen($("convert-modal"), false);
  focusEditor();
}

// Rewrite the current file in the chosen 文字コード / 改行コード. Every line is
// re-encoded server-side, so the active tab reloads the converted bytes.
export async function convertSave(encoding, lineEnding, bom) {
  if (!state.doc.stat?.open) return;
  if (savingCount > 0) {
    flashCount(t("editor.savingWait"));
    return;
  }
  await settleEditQueue();
  savingCount++;
  setSavingUI();
  try {
    const res = await apiPost<EditSaveResponse, EditSaveRequest>(
      "/api/edit/save",
      editSaveRequest({
        overwrite: true,
        encoding,
        eol: lineEnding,
        bom,
      }),
    );
    if (res.switched) {
      await reloadActiveDocument();
    } else {
      onDocumentOpened(await apiPost<OpenResponse, OpenRequest>("/api/open", { path: res.path }));
    }
    flashCount(t("dialog.convert.savedAs", { enc: enc(encoding), eol: eol(lineEnding) }));
  } catch (e) {
    flashCount(t("dialog.convert.saveError"), "error");
    showMessage(t("dialog.convert.go"), serverMessage(e));
  } finally {
    savingCount--;
    setSavingUI();
    retryPendingNativeClose();
  }
}

// Re-read the current file forcing a 文字コード — recovery when auto-detection
// guessed wrong and the text shows mojibake. Non-destructive to the file, but
// drops any unsaved edits, so confirm first.
export async function reopenWithEncoding(encoding) {
  if (!state.doc.stat?.open) return;
  if (isUntitled(state.doc.stat.path)) {
    flashCount(t("dialog.convert.noSavedFile"));
    return;
  }
  if (state.doc.stat.dirty) {
    const ok = await askConfirm(t("dialog.convert.reopen"), t("dialog.convert.discardAsk"), {
      okLabel: t("dialog.convert.discardOk"),
      danger: true,
    });
    if (!ok) return;
  }
  await settleEditQueue();
  try {
    await apiPost<unknown, ReopenRequest>("/api/reopen_encoding", { encoding });
    await reloadActiveDocument();
    flashCount(t("dialog.convert.reopenedAs", { enc: enc(encoding) }));
  } catch (e) {
    flashCount(t("dialog.convert.reopenError"), "error");
    showMessage(t("dialog.convert.reopen"), serverMessage(e));
  }
}

export function sortFormatForPath(path): "csv" | "tsv" | "text" {
  const sourcePath = String(path || "").toLowerCase();
  if (sourcePath.endsWith(".csv")) return "csv";
  if (sourcePath.endsWith(".tsv") || sourcePath.endsWith(".tab")) return "tsv";
  return "text";
}

export function parseSortKeys(text): number[] {
  const raw = String(text || "").trim();
  return raw
    ? raw
        .split(/[,;\s、，]+/u)
        .filter(Boolean)
        .map(Number)
    : [];
}

// ソート: include unsaved edits, write a private temporary result, and open it
// in a new tab. The source document is never overwritten by the GUI action.
export async function sortSave() {
  if (!state.doc.stat?.open) return;
  const detectedFormat = sortFormatForPath(state.doc.stat.path);
  const f = await askForm<{
    key: string;
    format: string;
    delim: string;
    numeric: boolean;
    order: string;
  }>(
    t("menu.sort"),
    [
      {
        id: "key",
        type: "text",
        label: t("dialog.sort.keyColumns"),
        placeholder: t("dialog.sort.keyPlaceholder"),
        title: t("dialog.sort.keyTitle"),
      },
      {
        id: "format",
        type: "select",
        label: t("dialog.sort.format"),
        value: detectedFormat,
        options: [
          ["csv", t("dialog.sort.formatCsv")],
          ["tsv", t("dialog.sort.formatTsv")],
          ["text", t("dialog.sort.formatText")],
          ["delimited", t("dialog.sort.formatDelimited")],
        ],
      },
      {
        id: "delim",
        type: "text",
        label: t("dialog.sort.delimiter"),
        value: ",",
        placeholder: ",",
        title: t("dialog.sort.delimiterTitle"),
      },
      {
        id: "numeric",
        type: "check",
        label: t("dialog.sort.numeric"),
        value: false,
        title: t("dialog.sort.numericTitle"),
      },
      {
        id: "order",
        type: "select",
        label: t("dialog.sort.order"),
        options: [
          ["asc", t("dialog.sort.asc")],
          ["desc", t("dialog.sort.desc")],
        ],
      },
      {
        id: "_hint",
        type: "hint",
        label: t("dialog.sort.hint"),
      },
    ],
    t("menu.sort"),
  );
  if (!f) return;
  const keyText = String(f.key || "").trim();
  const keys = parseSortKeys(keyText);
  if (keys.some((key) => !Number.isInteger(key) || key < 1)) {
    flashCount(t("dialog.sort.keyInvalid"), "error");
    return;
  }
  if (f.format === "text" && keys.length) {
    flashCount(t("dialog.sort.textHasNoColumns"), "error");
    return;
  }
  const csv = f.format === "csv" || f.format === "tsv";
  const delim =
    f.format === "csv"
      ? ","
      : f.format === "tsv"
        ? "\t"
        : f.format === "delimited"
          ? String(f.delim || ",")
          : null;
  const opId = newOperationId("sort");
  showLoading(t("dialog.sort.running"), { opId, cancel: true });
  try {
    const res = await apiPost<ArtifactResponse, SortSaveRequest>("/api/sort/save", {
      op_id: opId,
      path: null,
      // GUI sort is deliberately non-destructive: the server writes a private
      // temporary result and we open it as a new tab.
      in_place: false,
      key: keys[0] ?? null,
      keys: keys.length ? keys : null,
      numeric: !!f.numeric,
      reverse: f.order === "desc",
      delim,
      csv,
    });
    await openPath(res.path);
    flashCount(t("dialog.sort.newDone", { path: displayPath(res.path) }));
  } catch (e) {
    flashCount(t("dialog.sort.error"), "error");
    showMessage(t("dialog.sort.error"), serverMessage(e));
  } finally {
    hideLoading();
  }
}

// ファイル分割: writes the current document (unsaved edits included) out as
// multiple files of at most N lines each; the original file is untouched.
export async function splitFile() {
  if (!state.doc.stat?.open) return;
  const f = await askForm<{ lines: string; dir: string }>(
    t("menu.split"),
    [
      { id: "lines", type: "text", label: t("dialog.split.linesPer"), value: "1000000" },
      {
        id: "dir",
        type: "text",
        label: t("dialog.split.outDir"),
        value: "",
        placeholder: t("dialog.split.outDirPlaceholder"),
      },
      {
        id: "_hint",
        type: "hint",
        label: t("dialog.split.hint"),
      },
    ],
    t("dialog.split.go"),
  );
  if (!f) return;
  const lines = Number(String(f.lines || "").trim());
  if (!Number.isInteger(lines) || lines < 1) {
    flashCount(t("dialog.split.linesInvalid"), "error");
    return;
  }
  const opId = newOperationId("split");
  showLoading(t("dialog.split.running"), { opId, cancel: true });
  try {
    const dir = String(f.dir || "").trim();
    const res = await apiPost<{ files: string[]; count: number }, SplitSaveRequest>(
      "/api/split/save",
      { op_id: opId, lines, dir: dir || null },
    );
    flashCount(t("dialog.split.done", { count: res.count, path: displayPath(res.files[0]) }));
  } catch (e) {
    flashCount(t("dialog.split.error"), "error");
    showMessage(t("dialog.split.error"), serverMessage(e));
  } finally {
    hideLoading();
  }
}

// grep して保存: extract only the lines matching a pattern (the search bar's
// exact regex/case/word semantics) from the current document — unsaved edits
// included — into a file picked with the save dialog. The extraction streams
// through an isolated worker, so multi-GB files complete in bounded memory.
export async function grepToFile() {
  if (!state.doc.stat?.open) return;
  const f = await askForm<{ query: string; ci: boolean; word: boolean; regex: boolean }>(
    t("menu.grepSave"),
    [
      {
        id: "query",
        type: "text",
        label: t("dialog.grep.query"),
        value: lastGrep.query || state.search.query || "",
        placeholder: t("dialog.grep.queryPlaceholder"),
      },
      { id: "ci", type: "check", label: t("dialog.grep.ignoreCase"), value: lastGrep.ci },
      { id: "word", type: "check", label: t("find.wholeWord"), value: lastGrep.word },
      { id: "regex", type: "check", label: t("find.regex"), value: lastGrep.regex },
      { id: "_hint", type: "hint", label: t("dialog.grepSave.hint") },
    ],
    t("dialog.grepSave.go"),
  );
  if (!f) return;
  const query = String(f.query || "");
  if (!query.trim()) return;
  // Shared memory with フォルダ内検索 so both dialogs recall the same options.
  Object.assign(lastGrep, { query, ci: !!f.ci, word: !!f.word, regex: !!f.regex });
  const target = await showSaveDialog(t("menu.grepSave"), suggestedGrepPath());
  if (!target) return;
  await runGrepSave(query, { ci: !!f.ci, word: !!f.word, regex: !!f.regex }, target);
}

/// Run the existing bounded grep-to-file worker with one saved analysis rule.
/// The analysis UI supplies the rule; this helper deliberately reuses the same
/// save picker, overwrite handling, progress/cancel flow, and result tab.
export async function grepRuleToFile(rule) {
  if (!state.doc.stat?.open || !rule?.pattern) return;
  Object.assign(lastGrep, {
    query: rule.pattern,
    ci: !rule.case_sensitive,
    word: !!rule.whole_word,
    regex: !!rule.regex,
  });
  const target = await showSaveDialog(t("menu.grepSave"), suggestedGrepPath());
  if (!target) return;
  await runGrepSave(
    rule.pattern,
    {
      ci: !rule.case_sensitive,
      word: !!rule.whole_word,
      regex: !!rule.regex,
    },
    target,
  );
}

// "app.log" → sibling "app.grep.log"; untitled buffers suggest grep.txt in
// 前回の保存先 (the same folder save-as would suggest).
function suggestedGrepPath() {
  const p = state.doc.stat?.path || "";
  if (!p || isUntitled(p)) {
    return joinPath((state.settings.lastSaveDir || "").trim(), "grep.txt");
  }
  const shown = displayPath(p);
  const dir = pathDirName(shown);
  const base = shown.split(/[\\/]/).pop() || "grep.txt";
  const dot = base.lastIndexOf(".");
  const name = dot > 0 ? `${base.slice(0, dot)}.grep${base.slice(dot)}` : `${base}.grep.txt`;
  return joinPath(dir, name);
}

async function runGrepSave(query, opts, target) {
  let res;
  try {
    res = await withOverwriteRetry(
      target.path,
      async (overwrite) => {
        const opId = newOperationId("grep");
        showLoading(t("dialog.grepSave.running"), { opId, cancel: true });
        try {
          return await apiPost<ArtifactResponse, GrepSaveRequest>("/api/grep/save", {
            op_id: opId,
            path: target.path,
            query,
            regex: opts.regex,
            ci: opts.ci,
            word: opts.word,
            overwrite,
            jobs: null,
            chunk_lines: null,
          });
        } finally {
          hideLoading();
        }
      },
      !!target.overwrite,
    );
  } catch (e) {
    flashCount(t("dialog.grepSave.error"), "error");
    showMessage(t("dialog.grepSave.error"), serverMessage(e));
    return;
  }
  if (!res) return;
  flashCount(t("file.saved", { path: displayPath(res.path) }));
  // Open the extracted lines so the result is immediately inspectable.
  await openPath(res.path);
}

function suggestedBookmarkPath() {
  const path = state.doc.stat?.path || "";
  if (!path || isUntitled(path)) {
    return joinPath((state.settings.lastSaveDir || "").trim(), "bookmarks.txt");
  }
  const shown = displayPath(path);
  const dir = pathDirName(shown);
  const base = shown.split(/[\\/]/).pop() || "bookmarks.txt";
  const dot = base.lastIndexOf(".");
  const name =
    dot > 0 ? `${base.slice(0, dot)}.bookmarks${base.slice(dot)}` : `${base}.bookmarks.txt`;
  return joinPath(dir, name);
}

export async function bookmarksToFile() {
  if (!state.doc.stat?.open) return;
  if (savingCount > 0) {
    flashCount(t("editor.savingWait"));
    return;
  }
  await settleEditQueue();
  const target = await showSaveDialog(t("bookmark.save"), suggestedBookmarkPath());
  if (!target) return;
  await runBookmarkSave(target);
}

async function runBookmarkSave(target) {
  let result;
  try {
    result = await withOverwriteRetry(
      target.path,
      async (overwrite) => {
        showLoading(t("bookmark.saving"));
        try {
          return await apiPost<MarkerSaveResponse, MarkerSaveRequest>("/api/markers/save", {
            kind: "bookmark",
            path: target.path,
            overwrite,
          });
        } finally {
          hideLoading();
        }
      },
      !!target.overwrite,
    );
  } catch (error) {
    flashCount(t("bookmark.saveError"), "error");
    showMessage(t("bookmark.saveError"), serverMessage(error));
    return;
  }
  if (!result) return;
  flashCount(
    t("bookmark.saved", {
      count: commas(result.lines),
      path: displayPath(result.path),
    }),
  );
  await openPath(result.path);
}

// ---- crash recovery (server-side WAL) ---------------------------------------

// Guard: one recoverable document produces one dialog, even if open/select
// events race while the modal is up.
export let walPromptBusy = false;

// Paths whose next recoverable log is a deliberate window-to-window tab
// handoff (issue #35), not a crash: replay it silently instead of showing
// the crash-recovery prompt. One-shot per path.
const walHandoffPaths = new Set();

export function expectWalHandoff(path) {
  if (typeof path === "string" && path.trim()) walHandoffPaths.add(displayPath(path));
}

// The server found a crash log with unsaved edits for the just-opened
// document (stat.recoverable). Nothing is applied automatically: offer the
// choice — 復元 replays the log into the live session, 破棄 deletes it.
// Exception: a path registered via expectWalHandoff is an adopted tab whose
// edits the user deliberately moved here — those replay without asking.
export async function maybeOfferWalRecovery(stat) {
  if (walPromptBusy) return;
  // Consume the one-shot handoff flag on this path's open, whether or not a
  // log turned up: a handoff whose WAL vanished must not leave a stale flag
  // that later silently skips a genuine crash-recovery prompt for the same
  // path. In the real tear-out path the source fsyncs and detaches before the
  // new window spawns, so `recoverable` is always present here.
  const handoff = walHandoffPaths.delete(displayPath(stat?.path || ""));
  const n = stat?.recoverable;
  if (!n) return;
  walPromptBusy = true;
  try {
    if (handoff) {
      try {
        await apiPost<unknown, RecoverRequest>("/api/edit/recover", { discard: false });
      } catch {
        // The source window may still be releasing the log (its detach lands
        // milliseconds after our drop); one short retry covers that.
        await new Promise((resolve) => setTimeout(resolve, 400));
        await apiPost<unknown, RecoverRequest>("/api/edit/recover", { discard: false });
      }
      clearLineCache();
      await refreshStat();
      await reloadViewport();
      render();
      flashCount(t("tab.handoffDone"));
      return;
    }
    const restore = await askConfirm(t("recover.title"), t("recover.found", { n: commas(n) }), {
      okLabel: t("recover.restore"),
      cancelLabel: t("recover.discard"),
    });
    await apiPost<unknown, RecoverRequest>("/api/edit/recover", { discard: !restore });
    clearLineCache();
    await refreshStat();
    await reloadViewport();
    render();
    if (restore) {
      flashCount(t("recover.restored", { n: commas(n) }));
    } else {
      flashCount(t("recover.discarded"));
    }
  } catch (e) {
    flashCount(t("recover.error"), "error");
    console.error(e);
  } finally {
    walPromptBusy = false;
  }
}
