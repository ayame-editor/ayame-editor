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
// build just opens the app URL in a new tab/window.
export function openNewWindow(path = "") {
  if (isNativeApp()) {
    postNativeMessage(path ? `ayame:new-window:${path}` : "ayame:new-window");
    return;
  }
  window.open(location.href, "_blank");
}

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
