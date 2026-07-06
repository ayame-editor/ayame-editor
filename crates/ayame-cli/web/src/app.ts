// Ayame Editor — app module. Type-stripped to JS at build time (build.rs, oxc).
import { displayName } from "./dom.js";
import { state } from "./state.js";
import { t } from "./i18n.js";
import { askConfirm } from "./dialogs.js";

export let lastNativeTitle = "";

export function postNativeMessage(msg) {
  try {
    if (window.ipc && typeof window.ipc.postMessage === "function") {
      window.ipc.postMessage(msg);
    }
  } catch {
    // The web build has no native IPC; title/close still work in the browser.
  }
}

export function setAppTitle(title) {
  const next = title || "Ayame Editor";
  document.title = next;
  if (lastNativeTitle !== next) {
    lastNativeTitle = next;
    postNativeMessage(`ayame:title:${next}`);
  }
}

export function dirtyTabNames() {
  const names = [];
  for (const t of state.tabs || []) {
    if (t.dirty && t.name) names.push(t.name);
  }
  if (state.stat?.dirty && names.length === 0) names.push(displayName(state.stat.path));
  return [...new Set(names)].filter(Boolean);
}

export function hasDirtyDocuments() {
  return !!state.stat?.dirty || dirtyTabNames().length > 0;
}

export function dirtyCloseMessage() {
  const dirty = dirtyTabNames();
  const shown = dirty.slice(0, 5).join(", ");
  const more = dirty.length > 5 ? ` ${t("dialog.exit.moreFiles", { n: dirty.length - 5 })}` : "";
  const suffix = shown ? `\n\n${shown}${more}` : "";
  return `${t("dialog.exit.unsavedAsk")}${suffix}`;
}

export function isNativeApp() {
  return !!(window.ipc && typeof window.ipc.postMessage === "function");
}

export function requestEditorClose() {
  if (isNativeApp()) {
    postNativeMessage("ayame:close-ok");
    return true;
  }
  window.close();
  return false;
}

// 新規ウィンドウ: native builds ask the Rust side to spawn a fresh window
// process (contract: the "ayame:new-window" IPC message); the plain browser
// build just opens the app URL in a new tab/window. `recover` marks a
// dirty-tab handoff: the new window auto-replays the detached crash log
// (contract: "ayame:new-window-recover", issue #35).
export function openNewWindow(path = "", recover = false) {
  if (isNativeApp()) {
    const msg = !path ? "ayame:new-window" : `ayame:new-window${recover ? "-recover" : ""}:${path}`;
    postNativeMessage(msg);
    return;
  }
  window.open(location.href, "_blank");
}

// ---- native OS file dialogs (desktop build) ---------------------------------
//
// The GUI shell owns the real dialogs (rfd): the page sends an IPC request and
// the Rust side answers through window.__ayame*DialogDone. One dialog can be
// in flight at a time — the OS dialog is modal, so a second request can only
// come from a stale code path; it cancels the first cleanly instead of
// leaving its promise dangling.

let nativeSaveResolve = null;

let nativeOpenResolve = null;

// Ask the OS save dialog for a target path. Resolves with the chosen absolute
// path, or null when the user cancels. The OS dialog handles the overwrite
// confirmation itself.
export function nativeSaveDialog(dir, name): Promise<string | null> {
  return new Promise((resolve) => {
    if (nativeSaveResolve) nativeSaveResolve(null);
    nativeSaveResolve = resolve;
    postNativeMessage(
      `ayame:pick-save:${JSON.stringify({ dir: String(dir || ""), name: String(name || "") })}`,
    );
  });
}

// Ask the OS open dialog for one or more files. Resolves with an array of
// absolute paths (empty when the user cancels).
export function nativeOpenDialog(dir = ""): Promise<string[]> {
  return new Promise((resolve) => {
    if (nativeOpenResolve) nativeOpenResolve([]);
    nativeOpenResolve = resolve;
    postNativeMessage(`ayame:pick-open:${JSON.stringify({ dir: String(dir || "") })}`);
  });
}

window.__ayameSaveDialogDone = (path) => {
  const resolve = nativeSaveResolve;
  nativeSaveResolve = null;
  if (resolve) resolve(typeof path === "string" && path.trim() ? path : null);
};

window.__ayameOpenDialogDone = (paths) => {
  const resolve = nativeOpenResolve;
  nativeOpenResolve = null;
  if (!resolve) return;
  const list = Array.isArray(paths) ? paths.filter((p) => typeof p === "string" && p.trim()) : [];
  resolve(list);
};

export async function confirmCloseLastTab(tab) {
  if (tab?.dirty) {
    return askConfirm(t("dialog.exit.title"), t("dialog.exit.unsavedNamed", { name: tab.name }), {
      okLabel: t("dialog.exit.withoutSaving"),
      danger: true,
    });
  }
  if (state.settings.confirmLastTabClose === false) return true;
  return askConfirm(t("dialog.exit.title"), t("dialog.exit.lastTabAsk"), {
    okLabel: t("dialog.exit.exit"),
  });
}
