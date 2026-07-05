// Ayame Editor — save module. Type-stripped to JS at build time (build.rs, oxc).
import { $, commas, displayPath, isUntitled, joinPath, pathDirName, setModalOpen } from "./dom.js";
import { DEFAULT_SETTINGS, state } from "./state.js";
import { currentLocale, serverMessage, t, weekdayNames } from "./i18n.js";
import { api, apiPost } from "./api.js";
import { dirtyCloseMessage, hasDirtyDocuments, postNativeMessage } from "./app.js";
import { clearLineCache, focusEditor, render, setCaret } from "./editor.js";
import { enc, eol, hideFileMenu, updateStatusMeta } from "./menus.js";
import { reloadViewport, settleEditQueue } from "./edits.js";
import { flashCount } from "./search.js";
import { askConfirm, askForm, hideLoading, showLoading, showMessage } from "./dialogs.js";
import { onDocumentOpened, refreshTabs, showSaveDialog, updateTreeActive } from "./workspace.js";
import { saveSettings } from "./settings.js";

// Never let the native window kill the process while a save is in flight; the
// close request is answered "cancel" and retried once the save settles.
// While saving: key edits are blocked (onEditKey) and IME/beforeinput commits
// wait inside enqueueEdit so confirmed text is delayed, not lost.
export let savingCount = 0;

export let savingWaiters = [];

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

window.__ayameNativeCloseRequested = () => {
  if (savingCount > 0) {
    pendingNativeClose = true;
    flashCount(t("dialog.exit.savingWillClose"));
    postNativeMessage("ayame:close-cancel");
    return;
  }
  if (!hasDirtyDocuments()) {
    postNativeMessage("ayame:close-ok");
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
    if (ok) postNativeMessage("ayame:close-ok");
  });
};

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
  refreshTree = false,
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
  if (refreshTree) updateTreeActive();
}

// Shared tail of every save-as-style save (名前を付けて保存 / クイックメモ保存):
// the current tab becomes the saved file (the server swaps the active tab's
// document to the new path), exactly like every desktop editor — no leftover
// untitled tab, no extra tab for the saved file. Also remembers the folder as
// 前回の保存先 for the next untitled buffer.
export async function finishSaveAs(res) {
  if (res.switched) {
    // Same tab, new document identity: refresh in place, keep the caret.
    await reloadActiveDocument({ refreshTree: true });
  } else {
    // The workspace changed while saving (rare): fall back to focusing the
    // saved file — the server dedupes, so this never duplicates a tab.
    onDocumentOpened(await apiPost("/api/open", { path: res.path }));
  }
  rememberSaveDir(res.path);
  flashCount(t("file.saved", { path: displayPath(res.path) }));
}

// 前回の保存先: persisted so untitled buffers suggest the folder you last
// saved into (survives restarts).
export function rememberSaveDir(path) {
  const dir = pathDirName(displayPath(path));
  if (!dir) return;
  state.settings = { ...state.settings, lastSaveDir: dir };
  saveSettings(state.settings);
}

export async function saveCopy() {
  if (savingCount > 0) {
    flashCount(t("editor.savingWait"));
    return;
  }
  await settleEditQueue();
  const target = await showSaveDialog(t("menu.saveAs"), await suggestedSaveAsPath());
  if (!target) return;
  savingCount++;
  setSavingUI();
  try {
    const res = await apiPost("/api/edit/save", { ...target, switch_to_saved: true });
    await finishSaveAs(res);
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
          const res = await apiPost("/api/edit/save", {
            ...target,
            overwrite: true,
            switch_to_saved: true,
          });
          await finishSaveAs(res);
          return;
        } catch (retryError) {
          finalError = retryError;
        }
      } else {
        return;
      }
    }
    flashCount(t("error.saveError"), "error");
    showMessage(t("error.saveError"), serverMessage(finalError.message));
  } finally {
    savingCount--;
    setSavingUI();
    retryPendingNativeClose();
  }
}

export function isExistsError(e) {
  const msg = String(e?.message || e || "");
  return /already exists|既に存在/.test(msg);
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
    listing = await api(`/api/browse?dir=${encodeURIComponent(dir)}`);
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

export async function saveFile() {
  if (!state.stat?.open) return;
  if (savingCount > 0) {
    flashCount(t("editor.savingWait"));
    return;
  }
  await settleEditQueue();
  if (isUntitled(state.stat.path)) {
    // Untitled buffers always go through the save dialog; the dialog is
    // pre-filled with the expanded name template (see suggestedSaveAsPath).
    await saveCopy();
    return;
  }
  savingCount++;
  setSavingUI();
  try {
    const res = await apiPost("/api/edit/save", { overwrite: true });
    clearLineCache();
    await refreshStat();
    await reloadViewport();
    render();
    flashCount(t("file.saved", { path: displayPath(res.path) }));
  } catch (e) {
    flashCount(t("error.saveError"), "error");
    showMessage(t("error.saveError"), serverMessage(e.message));
  } finally {
    savingCount--;
    setSavingUI();
    retryPendingNativeClose();
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
  const encOpt = { utf8: "utf-8", "utf-8": "utf-8", "shift-jis": "shift-jis", "euc-jp": "euc-jp" };
  $("convert-enc").value = encOpt[state.stat.encoding] || "utf-8";
  const l = state.stat.eol;
  $("convert-eol").value = ["lf", "crlf", "cr"].includes(l) ? l : "lf";
  // Prefill 「BOMを付ける」 from the file's current BOM, then gray it out unless
  // the chosen 文字コード is UTF-8 (a UTF-8 BOM is the only one we can write).
  $("convert-bom").checked = state.stat.bom_bytes > 0;
  syncConvertBom();
  setModalOpen($("convert-modal"), true);
  queueMicrotask(() => $("convert-enc").focus());
}

// The BOM option only applies to UTF-8 output; disable it otherwise.
export function syncConvertBom() {
  const isUtf8 = $("convert-enc").value === "utf-8";
  $("convert-bom").disabled = !isUtf8;
  $("convert-bom-row").classList.toggle("disabled", !isUtf8);
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
    const res = await apiPost("/api/edit/save", {
      overwrite: true,
      encoding,
      eol: lineEnding,
      bom,
    });
    if (res.switched) {
      await reloadActiveDocument();
    } else {
      onDocumentOpened(await apiPost("/api/open", { path: res.path }));
    }
    flashCount(t("dialog.convert.savedAs", { enc: enc(encoding), eol: eol(lineEnding) }));
  } catch (e) {
    flashCount(t("dialog.convert.saveError"), "error");
    showMessage(t("dialog.convert.go"), serverMessage(e.message));
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
    await apiPost("/api/reopen_encoding", { encoding });
    await reloadActiveDocument();
    flashCount(t("dialog.convert.reopenedAs", { enc: enc(encoding) }));
  } catch (e) {
    flashCount(t("dialog.convert.reopenError"), "error");
    showMessage(t("dialog.convert.reopen"), serverMessage(e.message));
  }
}

// ソート: sorts the current tab in place — unsaved edits included — and
// overwrites the original file on disk. All options sit in one form.
export async function sortSave() {
  if (!state.stat?.open) return;
  const f = await askForm(
    t("menu.sort"),
    [
      {
        id: "key",
        type: "text",
        label: t("dialog.sort.keyColumn"),
        placeholder: t("dialog.sort.keyPlaceholder"),
        title: t("dialog.sort.keyTitle"),
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
  const key = keyText === "" ? null : Number(keyText);
  if (keyText !== "" && (!Number.isInteger(key) || key < 1)) {
    flashCount(t("dialog.sort.keyInvalid"), "error");
    return;
  }
  showLoading(t("dialog.sort.running"));
  try {
    await apiPost("/api/sort/save", {
      in_place: true,
      key,
      numeric: !!f.numeric,
      reverse: f.order === "desc",
      delim: key != null && f.delim ? f.delim : null,
    });
    state.sel = null;
    state.extraCursors = [];
    setCaret(0, 0);
    await reloadActiveDocument({ bumpGeneration: false, keepCaret: false, refreshTabList: false });
    flashCount(t("dialog.sort.done"));
  } catch (e) {
    flashCount(t("dialog.sort.error"), "error");
    showMessage(t("dialog.sort.error"), serverMessage(e.message));
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
  showLoading(t("dialog.split.running"));
  try {
    const dir = String(f.dir || "").trim();
    const res = await apiPost("/api/split/save", { lines, dir: dir || null });
    flashCount(t("dialog.split.done", { count: res.count, path: displayPath(res.files[0]) }));
  } catch (e) {
    flashCount(t("dialog.split.error"), "error");
    showMessage(t("dialog.split.error"), serverMessage(e.message));
  } finally {
    hideLoading();
  }
}

// ---- crash recovery (server-side WAL) ---------------------------------------

// Guard: one recoverable document produces one dialog, even if open/select
// events race while the modal is up.
export let walPromptBusy = false;

// The server found a crash log with unsaved edits for the just-opened
// document (stat.recoverable). Nothing is applied automatically: offer the
// choice — 復元 replays the log into the live session, 破棄 deletes it.
export async function maybeOfferWalRecovery(stat) {
  const n = stat?.recoverable;
  if (!n || walPromptBusy) return;
  walPromptBusy = true;
  try {
    const restore = await askConfirm(t("recover.title"), t("recover.found", { n: commas(n) }), {
      okLabel: t("recover.restore"),
      cancelLabel: t("recover.discard"),
    });
    await apiPost("/api/edit/recover", restore ? {} : { discard: true });
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
