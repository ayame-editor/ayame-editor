// Ayame Editor — save module. Type-stripped to JS at build time (build.rs, oxc).
import { $, commas, displayPath, isUntitled, joinPath, pathDirName, setModalOpen } from "./dom.js";
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
import { enc, eol, hideFileMenu, updateStatusMeta } from "./menus.js";
import { reloadViewport, settleEditQueue } from "./edits.js";
import { flashCount, lastGrep } from "./search.js";
import {
  askConfirm,
  askForm,
  hideLoading,
  newOperationId,
  showLoading,
  showMessage,
} from "./dialogs.js";
import { onDocumentOpened, openPath, refreshTabs, selectTab, showSaveDialog } from "./workspace.js";
import { saveSettings } from "./settings.js";
import { beaconSessionSnapshot, saveSessionSnapshot } from "./persistence.js";
import type {
  ArtifactResponse,
  BrowseResponse,
  EditSaveRequest,
  EditSaveResponse,
  GrepSaveRequest,
  OpenRequest,
  RecoverRequest,
  ReopenRequest,
  SortSaveRequest,
  SplitSaveRequest,
} from "./types/api.js";

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
  postNativeMessage("ayame:close-ok");
}

window.__ayameNativeCloseRequested = () => {
  if (savingCount > 0) {
    pendingNativeClose = true;
    flashCount(t("dialog.exit.savingWillClose"));
    postNativeMessage("ayame:close-cancel");
    return;
  }
  if (!hasDirtyDocuments()) {
    void confirmNativeCloseOk();
    return;
  }
  // Release the native close request first (it times out after a few
  // seconds), then ask with the in-app dialog; a confirmed close posts the
  // ok separately — the Rust side exits on it regardless of timing.
  postNativeMessage("ayame:close-cancel");
  askConfirm(t("dialog.exit.title"), dirtyCloseMessage(), {
    okLabel: t("dialog.exit.withoutSaving"),
    danger: true,
  }).then((ok) => {
    if (ok) void confirmNativeCloseOk();
  });
};

window.addEventListener("pagehide", () => {
  // A plain fetch is aborted as the page unloads; use sendBeacon so the final
  // snapshot survives (issue #73). Fall back to the async save only if the
  // browser refuses the beacon.
  if (!beaconSessionSnapshot()) void saveSessionSnapshot();
});

export function retryPendingNativeClose() {
  if (pendingNativeClose && savingCount === 0) {
    pendingNativeClose = false;
    window.__ayameNativeCloseRequested();
  }
}

window.addEventListener("beforeunload", (e) => {
  if (!hasDirtyDocuments()) return;
  e.preventDefault();
  e.returnValue = "";
});

export async function refreshStat() {
  state.stat = await api("/api/stat");
  state.total = state.stat.view_lines ?? state.stat.lines;
  noteWalError(state.stat);
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
} = {}) {
  if (bumpGeneration) {
    state.docGen++;
    state.editGen++;
  }
  clearLineCache();
  await refreshStat();
  await reloadViewport();
  if (keepCaret)
    setCaret(Math.min(state.caret.line, Math.max(0, state.total - 1)), state.caret.col);
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
  if (res.switched) {
    // Same tab, new document identity: refresh in place, keep the caret.
    await reloadActiveDocument();
  } else {
    // The workspace changed while saving (rare): fall back to focusing the
    // saved file — the server dedupes, so this never duplicates a tab.
    onDocumentOpened(await apiPost<unknown, OpenRequest>("/api/open", { path: res.path }));
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
    const res = await apiPost<EditSaveResponse, EditSaveRequest>(
      "/api/edit/save",
      editSaveRequest({ ...target, switch_to_saved: true }),
    );
    await finishSaveAs(res, { announce });
    return true;
  } catch (e) {
    let finalError = e;
    if (!target.overwrite && isExistsError(e)) {
      const ok = await askConfirm(
        t("dialog.overwrite.title"),
        t("dialog.overwrite.ask", { name: displayPath(target.path) }),
        {
          okLabel: t("dialog.overwrite.ok"),
          danger: true,
        },
      );
      if (ok) {
        try {
          const res = await apiPost<EditSaveResponse, EditSaveRequest>(
            "/api/edit/save",
            editSaveRequest({
              ...target,
              overwrite: true,
              switch_to_saved: true,
            }),
          );
          await finishSaveAs(res, { announce });
          return true;
        } catch (retryError) {
          finalError = retryError;
        }
      } else {
        return false;
      }
    }
    flashCount(t("error.saveError"), "error");
    showMessage(t("error.saveError"), serverMessage(finalError));
    return false;
  } finally {
    savingCount--;
    setSavingUI();
    retryPendingNativeClose();
  }
}

export function isExistsError(e) {
  return isApiErrorCode(e, "exists");
}

// 名前を付けて保存 opens on the current file's own folder and name (Windows
// standard); untitled buffers suggest the expanded 新規ファイル名 template
// inside 前回の保存先. Async because {seq} numbering needs the folder listing.
export async function suggestedSaveAsPath() {
  const p = state.stat?.path || "";
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
  if (!state.stat?.open) return false;
  if (savingCount > 0) {
    flashCount(t("editor.savingWait"));
    return false;
  }
  await settleEditQueue();
  if (isUntitled(state.stat.path)) {
    // Untitled buffers always go through the save dialog; the dialog is
    // pre-filled with the expanded name template (see suggestedSaveAsPath).
    return saveCopy({ announce });
  }
  savingCount++;
  setSavingUI();
  try {
    const res = await apiPost<EditSaveResponse, EditSaveRequest>(
      "/api/edit/save",
      editSaveRequest({ overwrite: true }),
    );
    clearLineCache();
    await refreshStat();
    await reloadViewport();
    render();
    if (announce) flashCount(t("file.saved", { path: displayPath(res.path) }));
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
  const tabs = [...(state.tabs || [])];
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
  if (!state.stat?.open) return;
  if (isUntitled(state.stat.path)) {
    // Nothing on disk to convert yet — save it first.
    flashCount(t("dialog.convert.saveFirst"));
    saveCopy();
    return;
  }
  hideFileMenu();
  // Prefill the pickers with the file's current encoding / line ending. The
  // stat strings are the core enum's kebab-case (Utf8 → "utf8"); map them onto
  // the select's option values.
  const encOpt = {
    utf8: "utf-8",
    "utf-8": "utf-8",
    "utf-16le": "utf-16le",
    "utf-16be": "utf-16be",
    "shift-jis": "shift-jis",
    "euc-jp": "euc-jp",
  };
  $("convert-enc").value = encOpt[state.stat.encoding] || "utf-8";
  const l = state.stat.eol;
  $("convert-eol").value = ["lf", "crlf", "cr"].includes(l) ? l : "lf";
  // Prefill 「BOMを付ける」 from the file's current BOM, then gray it out unless
  // the chosen 文字コード supports a Unicode BOM.
  $("convert-bom").checked = state.stat.bom_bytes > 0;
  syncConvertBom();
  setModalOpen($("convert-modal"), true);
  queueMicrotask(() => $("convert-enc").focus());
}

// The BOM option only applies to Unicode output; disable it otherwise.
export function syncConvertBom() {
  const encoding = $("convert-enc").value;
  const supportsBom = ["utf-8", "utf-16le", "utf-16be"].includes(encoding);
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
  if (!state.stat?.open) return;
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
      onDocumentOpened(await apiPost<unknown, OpenRequest>("/api/open", { path: res.path }));
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
  if (!state.stat?.open) return;
  if (isUntitled(state.stat.path)) {
    flashCount(t("dialog.convert.noSavedFile"));
    return;
  }
  if (state.stat.dirty) {
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
  if (!state.stat?.open) return;
  const detectedFormat = sortFormatForPath(state.stat.path);
  const f = await askForm(
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
  if (!state.stat?.open) return;
  const f = await askForm(
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
  if (!state.stat?.open) return;
  const f = await askForm(
    t("menu.grepSave"),
    [
      {
        id: "query",
        type: "text",
        label: t("dialog.grep.query"),
        value: lastGrep.query || state.query || "",
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
  if (!state.stat?.open || !rule?.pattern) return;
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
  const p = state.stat?.path || "";
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
  const opId = newOperationId("grep");
  showLoading(t("dialog.grepSave.running"), { opId, cancel: true });
  let res;
  try {
    res = await apiPost<ArtifactResponse, GrepSaveRequest>("/api/grep/save", {
      op_id: opId,
      path: target.path,
      query,
      regex: opts.regex,
      ci: opts.ci,
      word: opts.word,
      overwrite: !!target.overwrite,
      jobs: null,
      chunk_lines: null,
    });
  } catch (e) {
    hideLoading();
    // The in-app picker doesn't confirm overwrites itself (the OS dialog
    // does): same conflict-confirm-retry flow as save-as.
    if (!target.overwrite && isExistsError(e)) {
      const ok = await askConfirm(
        t("dialog.overwrite.title"),
        t("dialog.overwrite.ask", { name: displayPath(target.path) }),
        { okLabel: t("dialog.overwrite.ok"), danger: true },
      );
      if (ok) await runGrepSave(query, opts, { ...target, overwrite: true });
      return;
    }
    flashCount(t("dialog.grepSave.error"), "error");
    showMessage(t("dialog.grepSave.error"), serverMessage(e));
    return;
  }
  hideLoading();
  flashCount(t("file.saved", { path: displayPath(res.path) }));
  // Open the extracted lines so the result is immediately inspectable.
  await openPath(res.path);
}

function suggestedBookmarkPath() {
  const path = state.stat?.path || "";
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
  if (!state.stat?.open) return;
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
  showLoading(t("bookmark.saving"));
  let result;
  try {
    result = await apiPost<MarkerSaveResponse, MarkerSaveRequest>("/api/markers/save", {
      kind: "bookmark",
      path: target.path,
      overwrite: !!target.overwrite,
    });
  } catch (error) {
    hideLoading();
    if (!target.overwrite && isExistsError(error)) {
      const overwrite = await askConfirm(
        t("dialog.overwrite.title"),
        t("dialog.overwrite.ask", { name: displayPath(target.path) }),
        { okLabel: t("dialog.overwrite.ok"), danger: true },
      );
      if (overwrite) await runBookmarkSave({ ...target, overwrite: true });
      return;
    }
    flashCount(t("bookmark.saveError"), "error");
    showMessage(t("bookmark.saveError"), serverMessage(error));
    return;
  }
  hideLoading();
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
