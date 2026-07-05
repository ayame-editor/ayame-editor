// Ayame Editor — menus module. Type-stripped to JS at build time (build.rs, oxc).
import { $, commas, displayName, displayShortcut, humanBytes, setModalOpen } from "./dom.js";
import { DEFAULT_KEYMAP, KEYMAP_ACTIONS, state } from "./state.js";
import { applyStaticI18n, t } from "./i18n.js";
import { openNewWindow, setAppTitle } from "./app.js";
import { saveCopy, saveFile, sortSave, splitFile } from "./save.js";
import { coordsFromEvent, focusEditor, scheduleRender, setCaret } from "./editor.js";
import {
  addCursorAbove,
  addCursorBelow,
  copySelection,
  cutSelection,
  hasTextSelection,
  pasteFromClipboard,
  posInsideSelection,
  saveSelectionToFile,
  selectAll,
} from "./selection.js";
import {
  deleteLines,
  duplicateLines,
  gotoLine,
  moveLines,
  redoEdit,
  setFollowTail,
  transformSelection,
  undoEdit,
  updateTailUI,
} from "./edits.js";
import {
  buildMatcher,
  diffFile,
  flashCount,
  findStep,
  grepFolder,
  selectNextOccurrence,
  showFind,
  updateCount,
  updateFindCountLabel,
} from "./search.js";
import { askPrompt, formVisible, promptVisible } from "./dialogs.js";
import { anyModalOpen } from "./input.js";
import { isBindableShortcut, normalizeShortcut, sanitizeKeymap } from "./shortcuts.js";
import {
  closeTab,
  configureOpener,
  newUntitled,
  openerVisible,
  renderRecentFiles,
  renderTabs,
  setSidebar,
  showOpener,
  sidebarOpen,
} from "./workspace.js";
import {
  hideSettings,
  isKeymapDoc,
  isThemeDoc,
  populateLanguageSelect,
  saveSettings,
  showSettings,
  updateSetting,
} from "./settings.js";

export { isBindableShortcut, normalizeShortcut, sanitizeKeymap };

export const APP_MENUS = ["file", "edit", "selection", "view", "tools"];

export const MENU_ID_ACTIONS = [
  ["new-file", "newFile"],
  ["open-file", "openFile"],
  ["save-file", "saveFile"],
  ["save-copy", "saveAs"],
];

export function fileMenuVisible() {
  return APP_MENUS.some((id) => !$(`${id}-menu`).classList.contains("hidden"));
}

export function showAppMenu(id) {
  hideFileMenu();
  if (id === "view") {
    const ws = $("menu-toggle-ws");
    if (ws) {
      const on = !!state.settings.showWhitespace;
      ws.classList.toggle("checked", on);
      ws.setAttribute("aria-checked", String(on));
    }
    const zu = $("menu-toggle-zsp-underline");
    if (zu) {
      const zon = !!state.settings.zenkakuUnderline;
      zu.classList.toggle("checked", zon);
      zu.setAttribute("aria-checked", String(zon));
    }
    const wrap = $("menu-toggle-wrap");
    if (wrap) {
      const on = !!state.settings.wordWrap;
      wrap.classList.toggle("checked", on);
      wrap.setAttribute("aria-checked", String(on));
    }
    const tail = $("menu-toggle-tail");
    if (tail) {
      tail.classList.toggle("checked", state.followTail);
      tail.setAttribute("aria-checked", String(state.followTail));
    }
  }
  $(`${id}-menu`).classList.remove("hidden");
  $(`${id}-menu-button`).classList.add("on");
  $(`${id}-menu-button`).setAttribute("aria-expanded", "true");
}

export function hideFileMenu(focusButton = false) {
  let focused = false;
  for (const id of APP_MENUS) {
    const menu = $(`${id}-menu`);
    const button = $(`${id}-menu-button`);
    const wasOpen = !menu.classList.contains("hidden");
    menu.classList.add("hidden");
    button.classList.remove("on");
    button.setAttribute("aria-expanded", "false");
    if (focusButton && wasOpen && !focused) {
      button.focus();
      focused = true;
    }
  }
}

export function applyLocale() {
  applyStaticI18n();
  populateLanguageSelect(); // the "auto" label is localized; refresh it too
  updateKeyHints();
  updateStatusMeta();
  updateStatusPos();
  updateFindCountLabel();
  updateTailUI();
  // The opener sets its texts per mode (open / save); re-derive them.
  if (openerVisible()) configureOpener(state.openerMode);
  if (state.tabs?.length) renderTabs(state.tabs);
  if (keymapVisible()) renderKeymapRows();
  if (commandPaletteVisible()) {
    setPaletteItems(commandPaletteItems());
    renderCommandPalette();
  }
  renderRecentFiles();
}

export function eventShortcut(e) {
  if (["Control", "Shift", "Alt", "Meta"].includes(e.key)) return "";
  let key = e.key;
  if (key.length === 1) key = key.toUpperCase();
  else if (/^f\d+$/i.test(key)) key = key.toUpperCase();
  else key = key[0].toUpperCase() + key.slice(1);
  return [(e.ctrlKey || e.metaKey) && "Ctrl", e.shiftKey && "Shift", e.altKey && "Alt", key]
    .filter(Boolean)
    .join("+");
}

export function shortcutList(action) {
  const custom =
    state.settings.keymap && Object.prototype.hasOwnProperty.call(state.settings.keymap, action)
      ? state.settings.keymap[action]
      : DEFAULT_KEYMAP[action];
  const list = Array.isArray(custom) ? custom : [custom];
  return list.map(normalizeShortcut).filter(Boolean);
}

export function shortcutFor(action) {
  return shortcutList(action)[0] || "";
}

export function matchesShortcut(e, action) {
  const ev = eventShortcut(e);
  return !!ev && shortcutList(action).includes(ev);
}

export function setKeymap(action, shortcut) {
  const normalized = normalizeShortcut(shortcut);
  if (normalized && !isBindableShortcut(normalized)) {
    flashCount(t("keymap.conflictKey"));
    return;
  }
  state.settings = {
    ...state.settings,
    keymap: { ...state.settings.keymap, [action]: normalized },
  };
  saveSettings(state.settings);
  updateKeyHints();
  renderKeymapRows();
}

export function resetKeymap() {
  state.settings = { ...state.settings, keymap: {} };
  saveSettings(state.settings);
  updateKeyHints();
  renderKeymapRows();
}

export function updateKeyHints() {
  document.querySelectorAll("[data-key-action]").forEach((el) => {
    el.textContent = displayShortcut(shortcutFor((el as any).dataset.keyAction));
  });
  const hint = (labelKey, action) => {
    const key = displayShortcut(shortcutFor(action));
    const text = t(labelKey);
    return key ? `${text} (${key})` : text;
  };
  $("toggle-sidebar").title = hint("menu.explorer", "toggleSidebar");
  $("toggle-sidebar").setAttribute("aria-label", t("menu.explorer"));
  $("undo-edit").title = hint("menu.undo", "undo");
  $("undo-edit").setAttribute("aria-label", t("menu.undo"));
  $("redo-edit").title = hint("menu.redo", "redo");
  $("redo-edit").setAttribute("aria-label", t("menu.redo"));
  $("find").placeholder = hint("menu.find", "find");
  $("find-expand").title = hint("find.showReplace", "replace");
  $("find-expand").setAttribute("aria-label", t("find.showReplace"));
  $("find-prev").title = hint("find.prev", "findPrev");
  $("find-prev").setAttribute("aria-label", t("find.prev"));
  $("find-next").title = hint("find.next", "findNext");
  $("find-next").setAttribute("aria-label", t("find.next"));
  $("opt-case").title = hint("find.matchCase", "searchCase");
  $("opt-word").title = hint("find.wholeWord", "searchWord");
  $("opt-regex").title = hint("find.regex", "searchRegex");
  $("new-tab").title = hint("toolbar.newTab", "newFile");
  $("new-tab").setAttribute("aria-label", t("toolbar.newTab"));
  $("hidden-input").setAttribute("aria-label", t("editor.label"));
}

export function keymapVisible() {
  return !$("keymap-modal").classList.contains("hidden");
}

export function showKeymap() {
  hideSettings();
  renderKeymapRows();
  setModalOpen($("keymap-modal"), true);
  queueMicrotask(() => $("keymap-list").querySelector("input")?.focus());
}

export function hideKeymap() {
  setModalOpen($("keymap-modal"), false);
  focusEditor();
}

export function renderKeymapRows() {
  const list = $("keymap-list");
  if (!list) return;
  const used = new Map();
  for (const [action] of KEYMAP_ACTIONS) {
    for (const key of shortcutList(action)) used.set(key, (used.get(key) || 0) + 1);
  }
  list.textContent = "";
  const frag = document.createDocumentFragment();
  for (const [action, label] of KEYMAP_ACTIONS) {
    const row = document.createElement("label");
    const shortcut = shortcutFor(action);
    row.className = "keymap-row";
    if (shortcut && used.get(shortcut) > 1) row.classList.add("conflict");
    const name = document.createElement("span");
    name.className = "keymap-label";
    name.textContent = t(label);
    const input = document.createElement("input");
    input.className = "keymap-input";
    input.readOnly = true;
    input.value = displayShortcut(shortcut);
    input.placeholder = t("keymap.unassigned");
    input.dataset.action = action;
    input.addEventListener("keydown", (e) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape") {
        hideKeymap();
        return;
      }
      if (e.key === "Backspace" || e.key === "Delete") {
        setKeymap(action, "");
        return;
      }
      const shortcut = eventShortcut(e);
      if (shortcut) setKeymap(action, shortcut);
    });
    row.append(name, input);
    frag.append(row);
  }
  list.append(frag);
}

export let paletteItems = [];

export let paletteIndex = 0;

// applyLocale (another module) rebuilds the palette on a language switch;
// it does so via this setter since imports are read-only bindings.
export function setPaletteItems(v) {
  paletteItems = v;
}

export function commandPaletteVisible() {
  return !$("command-palette").classList.contains("hidden");
}

export function commandLabelFromElement(el) {
  return el?.querySelector(".menu-label")?.textContent?.trim() || "";
}

export function commandPaletteItems() {
  const keymapLabels = new Map(KEYMAP_ACTIONS.map(([action, label]) => [action, t(label)]));
  const seen = new Set();
  const items = [];
  const add = (action, label = "") => {
    if (!action || seen.has(action)) return;
    seen.add(action);
    // `label` comes from the DOM, which applyStaticI18n already localized.
    const text = label || keymapLabels.get(action) || action;
    items.push({
      action,
      label: text.replace(/\.\.\.$/, ""),
      shortcut: shortcutFor(action),
    });
  };
  for (const [id, action] of MENU_ID_ACTIONS) add(action, commandLabelFromElement($(id)));
  document.querySelectorAll("[data-menu-action]").forEach((el) => {
    add((el as any).dataset.menuAction, commandLabelFromElement(el));
  });
  // No explicit label: actions without a DOM menu item resolve through
  // keymapLabels, which already maps action → t(labelKey).
  for (const [action] of KEYMAP_ACTIONS) add(action);
  return items;
}

export function paletteMatches(item, query) {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  const hay = `${item.label} ${item.action} ${item.shortcut}`.toLowerCase();
  return q.split(/\s+/).every((part) => hay.includes(part));
}

export function renderCommandPalette() {
  const list = $("palette-list");
  const query = $("palette-input").value;
  const visible = paletteItems.filter((item) => paletteMatches(item, query));
  paletteIndex = Math.max(0, Math.min(paletteIndex, visible.length - 1));
  list.textContent = "";
  const frag = document.createDocumentFragment();
  visible.forEach((item, index) => {
    const row = document.createElement("button");
    row.type = "button";
    row.className = "palette-row";
    row.classList.toggle("active", index === paletteIndex);
    row.setAttribute("role", "option");
    row.setAttribute("aria-selected", index === paletteIndex ? "true" : "false");
    const label = document.createElement("span");
    label.className = "palette-label";
    label.textContent = item.label;
    const key = document.createElement("span");
    key.className = "palette-key";
    key.textContent = displayShortcut(item.shortcut);
    row.append(label, key);
    row.addEventListener("mouseenter", () => {
      if (paletteIndex === index) return;
      paletteIndex = index;
      renderCommandPalette();
    });
    row.addEventListener("click", () => executePaletteItem(item));
    frag.append(row);
  });
  list.append(frag);
  list.querySelector(".palette-row.active")?.scrollIntoView({ block: "nearest" });
}

export function showCommandPalette() {
  hideFileMenu();
  if (promptVisible() || formVisible() || commandPaletteVisible()) return;
  paletteItems = commandPaletteItems();
  paletteIndex = 0;
  $("palette-input").value = "";
  setModalOpen($("command-palette"), true);
  renderCommandPalette();
  queueMicrotask(() => $("palette-input").focus());
}

export function hideCommandPalette() {
  setModalOpen($("command-palette"), false);
  focusEditor();
}

export function movePalette(delta) {
  const visible = paletteItems.filter((item) => paletteMatches(item, $("palette-input").value));
  if (!visible.length) return;
  paletteIndex = (paletteIndex + delta + visible.length) % visible.length;
  renderCommandPalette();
}

export function executePaletteItem(item) {
  if (!item) return;
  hideCommandPalette();
  queueMicrotask(() => runMenuAction(item.action));
}

export function initCommandPalette() {
  $("palette-close").addEventListener("click", hideCommandPalette);
  $("command-palette").addEventListener("click", (e) => {
    if (e.target === $("command-palette")) hideCommandPalette();
  });
  $("palette-input").addEventListener("input", () => {
    paletteIndex = 0;
    renderCommandPalette();
  });
  $("palette-input").addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      e.preventDefault();
      hideCommandPalette();
      return;
    }
    if (e.key === "ArrowDown") {
      e.preventDefault();
      movePalette(1);
      return;
    }
    if (e.key === "ArrowUp") {
      e.preventDefault();
      movePalette(-1);
      return;
    }
    if (e.key === "Enter") {
      e.preventDefault();
      const visible = paletteItems.filter((item) => paletteMatches(item, $("palette-input").value));
      executePaletteItem(visible[paletteIndex]);
    }
  });
}

// ---- editor context menu ----------------------------------------------------

export function ctxMenuVisible() {
  return !$("ctx-menu").classList.contains("hidden");
}

export function hideCtxMenu() {
  $("ctx-menu").classList.add("hidden");
}

export function runCtxAction(action) {
  hideCtxMenu();
  // Only the two context-menu-specific actions live here; everything else
  // (cut / copy / selectAll / find / replace / sortSave / diffFile /
  // splitFile) shares the menu dispatcher.
  let out;
  if (action === "paste") out = pasteFromClipboard();
  else if (action === "saveSelection") out = saveSelectionToFile();
  else out = runMenuAction(action);
  // A context-menu click leaves focus on the (now hidden) menu item, killing
  // keyboard input after cut/copy etc. Put focus back in the editor once the
  // action settles — unless it opened its own focus target (a modal, or the
  // find bar).
  return Promise.resolve(out).finally(() => {
    if (!anyModalOpen() && !state.findOpen) focusEditor();
  });
}

export function initContextMenu() {
  const menu = $("ctx-menu");
  $("viewport").addEventListener("contextmenu", (e) => {
    e.preventDefault();
    if (!state.stat?.open || anyModalOpen()) return;
    // Right-click inside the selection keeps it as the action target;
    // outside it, the caret moves to the click point first (editor standard).
    const p = coordsFromEvent(e);
    if (!posInsideSelection(p)) {
      state.sel = null;
      setCaret(p.line, p.col);
      scheduleRender();
    }
    // Zero-width rect selections count as empty for the text actions.
    const hasSel = hasTextSelection();
    menu.querySelectorAll("[data-ctx]").forEach((el) => {
      const a = el.dataset.ctx;
      el.disabled = (a === "cut" || a === "copy" || a === "saveSelection") && !hasSel;
    });
    menu.classList.remove("hidden");
    const mw = menu.offsetWidth;
    const mh = menu.offsetHeight;
    menu.style.left = `${Math.max(4, Math.min(e.clientX, window.innerWidth - mw - 8))}px`;
    menu.style.top = `${Math.max(4, Math.min(e.clientY, window.innerHeight - mh - 8))}px`;
  });
  menu.querySelectorAll("[data-ctx]").forEach((el) => {
    el.addEventListener("click", () => runCtxAction(el.dataset.ctx));
  });
  document.addEventListener("pointerdown", (e) => {
    if (ctxMenuVisible() && !(e.target as any).closest("#ctx-menu")) hideCtxMenu();
  });
}

// ---- status bar ------------------------------------------------------------

export function updateStatusMeta() {
  const s = state.stat;
  if (!s) {
    setAppTitle("Ayame Editor");
    return;
  }
  if (!s.open) {
    for (const id of ["st-enc", "st-eol", "st-edit", "st-index"]) {
      $(id).textContent = "—";
    }
    $("st-edit").title = "";
    $("st-index").title = "";
    $("st-pos").textContent = t("status.line0");
    $("undo-edit").disabled = true;
    $("redo-edit").disabled = true;
    $("apply-theme").classList.add("hidden");
    $("apply-keymap").classList.add("hidden");
    setAppTitle("Ayame Editor");
    return;
  }
  const name = displayName(s.path);
  const dirtyMark = s.dirty ? "* " : "";
  setAppTitle(`${dirtyMark}${name} - Ayame Editor`);
  $("apply-theme").classList.toggle("hidden", !isThemeDoc(s.path));
  $("apply-keymap").classList.toggle("hidden", !isKeymapDoc(s.path));
  const lines = s.view_lines ?? s.lines;
  $("st-enc").textContent = s.bom_bytes > 0 ? `${enc(s.encoding)} (BOM)` : enc(s.encoding);
  $("st-eol").textContent = eol(s.eol);
  // Deliberately terse: the bar shows state, the tooltip carries the numbers.
  $("st-edit").textContent = s.dirty ? t("status.unsaved") : t("status.saved");
  $("st-edit").title = s.dirty
    ? t("status.unsavedDetail", {
        added: commas(s.inserted_lines),
        changed: commas(s.replaced_lines),
        deleted: commas(s.deleted_lines),
      })
    : t("status.allSaved");
  $("undo-edit").disabled = !s.can_undo;
  $("redo-edit").disabled = !s.can_redo;
  $("st-index").textContent = t("status.indexOk");
  $("st-index").title = t("status.indexDetail", {
    lines: commas(lines),
    bytes: humanBytes(s.bytes),
    checkpoints: commas(s.checkpoints),
    indexBytes: humanBytes(s.index_bytes),
    indexMs: s.index_ms,
  });
  // Keep the active tab's unsaved-dot (and the tabs model behind
  // beforeunload / close confirmations) in sync as you type.
  const at = $("tabs").querySelector(".tab.active");
  if (at) at.classList.toggle("dirty", !!s.dirty);
  const activeTab = (state.tabs || []).find((t) => t.active);
  if (activeTab) activeTab.dirty = !!s.dirty;
}

export function enc(e) {
  // Keys match the core Encoding enum's kebab-case serialization (Utf8 → "utf8").
  return (
    {
      utf8: "UTF-8",
      "utf-8": "UTF-8",
      "shift-jis": "Shift_JIS",
      "euc-jp": "EUC-JP",
      ascii: "ASCII",
    }[e] || String(e)
  );
}

export function eol(e) {
  return { lf: "LF", crlf: "CRLF", cr: "CR", mixed: "Mixed", none: "None" }[e] || String(e);
}

export function updateStatusPos() {
  if (state.total === 0) {
    $("st-pos").textContent = t("status.line0");
    return;
  }
  const pos = t("status.pos", {
    line: commas(state.caret.line + 1),
    col: commas(state.caret.col + 1),
  });
  const n = state.extraCursors.length;
  $("st-pos").textContent = n ? t("status.posCursors", { pos, n: n + 1 }) : pos;
}

const closeActiveTab = () => {
  const active = state.tabs.find((t) => t.active);
  if (active) closeTab(active.id);
};

const promptGotoLine = () => {
  askPrompt(t("menu.gotoLine"), t("dialog.gotoLine.label")).then((v) => {
    if (v != null) gotoLine(v);
  });
};

export function toggleOpt(key, id) {
  state[key] = !state[key];
  $(id).classList.toggle("on", state[key]);
  state.lastMatch = null;
  state.searchHits = null;
  state.searchTruncated = false;
  buildMatcher();
  scheduleRender();
  if (state.query) updateCount();
}

export const ACTIONS: Record<string, { run: () => any; globalShortcut?: boolean; editorOnly?: boolean }> = {
  commandPalette: { run: showCommandPalette, globalShortcut: true },
  undo: { run: undoEdit, globalShortcut: true, editorOnly: true },
  redo: { run: redoEdit, globalShortcut: true, editorOnly: true },
  find: { run: () => showFind(), globalShortcut: true },
  replace: { run: () => showFind(true), globalShortcut: true },
  gotoLine: { run: promptGotoLine, globalShortcut: true },
  selectAll: { run: selectAll, globalShortcut: true, editorOnly: true },
  selectNextOccurrence: { run: selectNextOccurrence },
  addCursorAbove: { run: addCursorAbove },
  addCursorBelow: { run: addCursorBelow },
  duplicateLine: { run: duplicateLines },
  moveLineUp: { run: () => moveLines(-1) },
  moveLineDown: { run: () => moveLines(1) },
  deleteLine: { run: deleteLines },
  copy: { run: copySelection, globalShortcut: true, editorOnly: true },
  cut: { run: cutSelection, globalShortcut: true, editorOnly: true },
  toggleSidebar: { run: () => setSidebar(!sidebarOpen()), globalShortcut: true },
  toggleWhitespace: {
    run: () => updateSetting("showWhitespace", !state.settings.showWhitespace),
  },
  toggleZenkakuUnderline: {
    run: () => updateSetting("zenkakuUnderline", !state.settings.zenkakuUnderline),
  },
  toggleWordWrap: { run: () => updateSetting("wordWrap", !state.settings.wordWrap) },
  toggleFollowTail: { run: () => setFollowTail(!state.followTail) },
  settings: { run: showSettings, globalShortcut: true },
  sortSave: { run: sortSave, globalShortcut: true },
  diffFile: { run: diffFile, globalShortcut: true },
  splitFile: { run: splitFile, globalShortcut: true },
  grepFolder: { run: grepFolder, globalShortcut: true },
  caseUpper: { run: () => transformSelection("upper"), globalShortcut: true, editorOnly: true },
  caseLower: { run: () => transformSelection("lower"), globalShortcut: true, editorOnly: true },
  keymap: { run: showKeymap, globalShortcut: true },
  newFile: { run: newUntitled, globalShortcut: true },
  newWindow: { run: openNewWindow, globalShortcut: true },
  openFile: { run: showOpener, globalShortcut: true },
  saveFile: { run: saveFile, globalShortcut: true },
  saveAs: { run: saveCopy, globalShortcut: true },
  closeTab: { run: closeActiveTab, globalShortcut: true },
  findPrev: { run: () => findStep("prev"), globalShortcut: true },
  findNext: { run: () => findStep("next"), globalShortcut: true },
  searchCase: { run: () => toggleOpt("ci", "opt-case"), globalShortcut: true },
  searchRegex: { run: () => toggleOpt("regex", "opt-regex"), globalShortcut: true },
  searchWord: { run: () => toggleOpt("word", "opt-word"), globalShortcut: true },
};

export function runAction(action) {
  return ACTIONS[action]?.run();
}

const GLOBAL_SHORTCUT_ACTIONS = [
  "commandPalette",
  "openFile",
  "toggleSidebar",
  "newFile",
  "newWindow",
  "gotoLine",
  "closeTab",
  "find",
  "replace",
  "saveAs",
  "saveFile",
  "findPrev",
  "findNext",
  "searchCase",
  "searchRegex",
  "searchWord",
  "sortSave",
  "diffFile",
  "splitFile",
  "grepFolder",
  "settings",
  "keymap",
  "selectAll",
  "copy",
  "cut",
  "caseUpper",
  "caseLower",
  "redo",
  "undo",
];

export function shortcutActionFromEvent(e, inField = false) {
  for (const action of GLOBAL_SHORTCUT_ACTIONS) {
    const entry = ACTIONS[action];
    if (!entry?.globalShortcut || (inField && entry.editorOnly)) continue;
    if (matchesShortcut(e, action)) return action;
  }
  return null;
}

export function runMenuAction(action) {
  hideFileMenu();
  // A modal owns the UI. Every menu action either opens a dialog or acts on
  // the document hidden behind the modal, and the native macOS menu can fire
  // at any time — so ALL actions are ignored while a modal is open. (In-page
  // menus are unreachable then; this guards the native path.)
  if (anyModalOpen()) return;
  return runAction(action);
}

// Native menu dispatcher: the macOS (Rust) side calls this via evaluate_script
// with the same action ids the in-page menus use.
window.__ayameMenu = runMenuAction;

export function initMenuBar() {
  for (const id of APP_MENUS) {
    const button = $(`${id}-menu-button`);
    button.addEventListener("click", (e) => {
      e.stopPropagation();
      const open = !$(`${id}-menu`).classList.contains("hidden");
      if (open) hideFileMenu();
      else showAppMenu(id);
    });
    button.addEventListener("pointerenter", () => {
      if (fileMenuVisible()) showAppMenu(id);
    });
  }
  document.querySelectorAll("[data-menu-action]").forEach((item) => {
    item.addEventListener("click", () => runMenuAction((item as any).dataset.menuAction));
  });
}
