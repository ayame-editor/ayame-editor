// Ayame Editor — menus module. Type-stripped to JS at build time (build.rs, oxc).
import { $, commas, displayName, humanBytes, setModalOpen } from "./dom.js";
import { DEFAULT_KEYMAP, KEYMAP_ACTIONS, state } from "./state.js";
import { applyStaticI18n, currentLocale, t } from "./i18n.js";
import { openNewWindow, setAppTitle } from "./app.js";
import { grepToFile, saveCopy, saveFile, showConvert, sortSave, splitFile } from "./save.js";
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
  flashCount,
  findStep,
  grepFolder,
  selectNextOccurrence,
  showFind,
  updateCount,
  updateFindCountLabel,
} from "./search.js";
import { askPrompt, formVisible, promptVisible, showMessage } from "./dialogs.js";
import { anyModalOpen } from "./input.js";
import {
  displayShortcut,
  eventShortcut,
  isBindableShortcut,
  matchesShortcut as eventMatchesShortcut,
  normalizeShortcut,
  sanitizeKeymap,
} from "./keys.js";
import {
  closeTab,
  configureOpener,
  newUntitled,
  openFileDialog,
  openerVisible,
  renderRecentFiles,
  renderTabs,
  selectRelativeTab,
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

export const APP_MENUS = ["file", "edit", "selection", "view", "help"];
const DROPDOWN_MENUS = [...APP_MENUS, "tools"];

export const MENU_ID_ACTIONS = [
  ["new-file", "newFile"],
  ["open-file", "openFile"],
  ["save-file", "saveFile"],
  ["save-copy", "saveAs"],
];

export function fileMenuVisible() {
  return DROPDOWN_MENUS.some((id) => !$(`${id}-menu`).classList.contains("hidden"));
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
    const syntax = $("menu-toggle-syntax");
    if (syntax) {
      const on = state.settings.syntaxHighlight !== false;
      syntax.classList.toggle("checked", on);
      syntax.setAttribute("aria-checked", String(on));
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
  if (id === "edit") {
    // Cut/Copy need a selection to act on — match the context menu, which
    // already disables them, instead of offering dead commands (#186).
    const hasSel = hasTextSelection();
    for (const action of ["cut", "copy"]) {
      const item = $("edit-menu").querySelector<HTMLButtonElement>(
        `[data-menu-action="${action}"]`,
      );
      if (item) item.disabled = !hasSel;
    }
  }
  $(`${id}-menu`).classList.remove("hidden");
  $(`${id}-menu-button`).classList.add("on");
  $(`${id}-menu-button`).setAttribute("aria-expanded", "true");
}

export function hideFileMenu(focusButton = false) {
  let focused = false;
  for (const id of DROPDOWN_MENUS) {
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
  return eventMatchesShortcut(e, shortcutList(action));
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
  rebuildGlobalShortcutActions();
  document.querySelectorAll("[data-key-action]").forEach((el) => {
    el.textContent = displayShortcut(shortcutFor((el as any).dataset.keyAction));
  });
  document.querySelectorAll("[data-key-static]").forEach((el) => {
    el.textContent = displayShortcut((el as HTMLElement).dataset.keyStatic);
  });
  const hint = (labelKey, action) => {
    const key = displayShortcut(shortcutFor(action));
    const text = t(labelKey);
    return key ? `${text} (${key})` : text;
  };
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
    input.className = "keymap-input input-control";
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
  const input = $("palette-input");
  const query = input.value;
  const visible = paletteItems.filter((item) => paletteMatches(item, query));
  paletteIndex = Math.max(0, Math.min(paletteIndex, visible.length - 1));
  list.textContent = "";
  const frag = document.createDocumentFragment();
  visible.forEach((item, index) => {
    const row = document.createElement("button");
    row.type = "button";
    row.id = `palette-option-${index}`;
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
  const active = list.querySelector(".palette-row.active");
  if (active) input.setAttribute("aria-activedescendant", active.id);
  else input.removeAttribute("aria-activedescendant");
  active?.scrollIntoView({ block: "nearest" });
}

export function showCommandPalette() {
  hideFileMenu();
  if (promptVisible() || formVisible() || commandPaletteVisible()) return;
  paletteItems = commandPaletteItems();
  paletteIndex = 0;
  $("palette-input").value = "";
  $("palette-input").setAttribute("aria-expanded", "true");
  setModalOpen($("command-palette"), true);
  renderCommandPalette();
  queueMicrotask(() => $("palette-input").focus());
}

export function hideCommandPalette() {
  setModalOpen($("command-palette"), false);
  $("palette-input").setAttribute("aria-expanded", "false");
  $("palette-input").removeAttribute("aria-activedescendant");
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
  // Only saveSelection is context-menu-specific; clipboard and editor actions
  // share the normal menu dispatcher.
  let out;
  if (action === "saveSelection") out = saveSelectionToFile();
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
    menu.querySelectorAll<HTMLButtonElement>("[data-ctx]").forEach((el) => {
      const a = el.dataset.ctx;
      el.disabled = (a === "cut" || a === "copy" || a === "saveSelection") && !hasSel;
    });
    menu.classList.remove("hidden");
    const mw = menu.offsetWidth;
    const mh = menu.offsetHeight;
    menu.style.left = `${Math.max(4, Math.min(e.clientX, window.innerWidth - mw - 8))}px`;
    menu.style.top = `${Math.max(4, Math.min(e.clientY, window.innerHeight - mh - 8))}px`;
  });
  menu.querySelectorAll<HTMLButtonElement>("[data-ctx]").forEach((el) => {
    el.addEventListener("click", () => runCtxAction(el.dataset.ctx));
  });
  document.addEventListener("pointerdown", (e) => {
    if (ctxMenuVisible() && !(e.target as any).closest("#ctx-menu")) hideCtxMenu();
  });
}

// ---- reusable pop-up menu (tab / font-size right-click) ---------------------

export interface PopupMenuItem {
  label?: string;
  action?: () => void;
  disabled?: boolean;
  checked?: boolean;
  separator?: boolean;
}

// A lightweight context menu positioned at (x, y). Reuses the .file-menu look
// and the .ctx-menu pointer positioning, and dismisses on outside click / Esc.
export function showPopupMenu(x: number, y: number, items: PopupMenuItem[]) {
  document.getElementById("popup-menu")?.remove();
  const menu = document.createElement("div");
  menu.id = "popup-menu";
  menu.className = "file-menu ctx-menu";
  menu.setAttribute("role", "menu");
  const close = () => {
    menu.remove();
    document.removeEventListener("pointerdown", onDown, true);
    document.removeEventListener("keydown", onKey, true);
  };
  const onDown = (e: Event) => {
    if (!(e.target as any)?.closest?.("#popup-menu")) close();
  };
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      close();
    }
  };
  for (const it of items) {
    if (it.separator) {
      const sep = document.createElement("div");
      sep.className = "menu-sep";
      menu.append(sep);
      continue;
    }
    const b = document.createElement("button");
    b.type = "button";
    b.className = "menu-item" + (it.checked ? " checked" : "");
    b.setAttribute("role", "menuitem");
    b.disabled = !!it.disabled;
    const label = document.createElement("span");
    label.className = "menu-label";
    label.textContent = it.label || "";
    b.append(label);
    b.addEventListener("click", () => {
      close();
      it.action?.();
    });
    menu.append(b);
  }
  document.body.append(menu);
  const mw = menu.offsetWidth;
  const mh = menu.offsetHeight;
  menu.style.left = `${Math.max(4, Math.min(x, window.innerWidth - mw - 8))}px`;
  menu.style.top = `${Math.max(4, Math.min(y, window.innerHeight - mh - 8))}px`;
  // Defer wiring the dismiss listeners so the opening right-click doesn't close it.
  setTimeout(() => {
    document.addEventListener("pointerdown", onDown, true);
    document.addEventListener("keydown", onKey, true);
  }, 0);
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
    $("st-enc").setAttribute("aria-label", t("status.encodingValue", { value: "—" }));
    $("st-eol").setAttribute("aria-label", t("status.eolValue", { value: "—" }));
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
  const encoding =
    s.bom_bytes > 0 ? t("status.encWithBom", { enc: enc(s.encoding) }) : enc(s.encoding);
  const lineEnding = eol(s.eol);
  $("st-enc").textContent = encoding;
  $("st-enc").setAttribute("aria-label", t("status.encodingValue", { value: encoding }));
  $("st-eol").textContent = lineEnding;
  $("st-eol").setAttribute("aria-label", t("status.eolValue", { value: lineEnding }));
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
    bytes: humanBytes(s.bytes, currentLocale()),
    checkpoints: commas(s.checkpoints),
    indexBytes: humanBytes(s.index_bytes, currentLocale()),
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
      "utf-16le": "UTF-16 LE",
      "utf-16be": "UTF-16 BE",
      "shift-jis": "Shift_JIS",
      "euc-jp": "EUC-JP",
      ascii: "ASCII",
    }[e] || String(e)
  );
}

export function eol(e) {
  // LF/CRLF/CR are universal; "混在"/"なし" (Mixed/None) are words, so localize.
  return (
    { lf: "LF", crlf: "CRLF", cr: "CR", mixed: t("status.eolMixed"), none: t("status.eolNone") }[
      e
    ] || String(e)
  );
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

const showHelp = () => showMessage(t("help.title"), t("help.body"));
const showAbout = () => showMessage(t("help.about"), t("help.aboutBody"));

// The find-bar option buttons are lit when their *labelled* meaning is active.
// `opt-case` is labelled "Match Case" but the underlying `state.ci` flag means
// *ignore* case, so it must light up when ci is false (issue #70). The other
// toggles light up on their own truthy value.
function optButtonLit(key): boolean {
  return key === "ci" ? !state.ci : !!state[key];
}

// Sync the lit state of the three find-bar option buttons to `state` without
// toggling anything — used on init so "Match Case" reflects the default.
export function refreshFindOptButtons() {
  for (const [id, key] of [
    ["opt-case", "ci"],
    ["opt-word", "word"],
    ["opt-regex", "regex"],
  ]) {
    const pressed = optButtonLit(key);
    $(id).classList.toggle("on", pressed);
    $(id).setAttribute("aria-pressed", String(pressed));
  }
}

export function toggleOpt(key, id) {
  state[key] = !state[key];
  const pressed = optButtonLit(key);
  $(id).classList.toggle("on", pressed);
  $(id).setAttribute("aria-pressed", String(pressed));
  state.lastMatch = null;
  state.searchHits = null;
  state.searchTruncated = false;
  buildMatcher();
  scheduleRender();
  if (state.query) updateCount();
}

export const ACTIONS: Record<
  string,
  { run: () => any; globalShortcut?: boolean; editorOnly?: boolean }
> = {
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
  paste: { run: pasteFromClipboard, editorOnly: true },
  toggleWhitespace: {
    run: () => updateSetting("showWhitespace", !state.settings.showWhitespace),
  },
  toggleSyntaxHighlight: {
    run: () => updateSetting("syntaxHighlight", state.settings.syntaxHighlight === false),
  },
  toggleZenkakuUnderline: {
    run: () => updateSetting("zenkakuUnderline", !state.settings.zenkakuUnderline),
  },
  toggleWordWrap: { run: () => updateSetting("wordWrap", !state.settings.wordWrap) },
  toggleFollowTail: { run: () => setFollowTail(!state.followTail) },
  nextTab: { run: () => selectRelativeTab(1), globalShortcut: true },
  prevTab: { run: () => selectRelativeTab(-1), globalShortcut: true },
  settings: { run: showSettings, globalShortcut: true },
  help: { run: showHelp, globalShortcut: true },
  about: { run: showAbout, globalShortcut: true },
  sortSave: { run: sortSave, globalShortcut: true },
  splitFile: { run: splitFile, globalShortcut: true },
  grepFolder: { run: grepFolder, globalShortcut: true },
  grepSave: { run: grepToFile, globalShortcut: true },
  caseUpper: { run: () => transformSelection("upper"), globalShortcut: true, editorOnly: true },
  caseLower: { run: () => transformSelection("lower"), globalShortcut: true, editorOnly: true },
  caseCamel: { run: () => transformSelection("camel"), globalShortcut: true, editorOnly: true },
  casePascal: { run: () => transformSelection("pascal"), globalShortcut: true, editorOnly: true },
  caseSnake: { run: () => transformSelection("snake"), globalShortcut: true, editorOnly: true },
  caseKebab: { run: () => transformSelection("kebab"), globalShortcut: true, editorOnly: true },
  caseConstant: {
    run: () => transformSelection("constant"),
    globalShortcut: true,
    editorOnly: true,
  },
  keymap: { run: showKeymap, globalShortcut: true },
  newFile: { run: newUntitled, globalShortcut: true },
  newWindow: { run: openNewWindow, globalShortcut: true },
  openFile: { run: openFileDialog, globalShortcut: true },
  saveFile: { run: saveFile, globalShortcut: true },
  saveAs: { run: saveCopy, globalShortcut: true },
  encoding: { run: showConvert },
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
  "newFile",
  "newWindow",
  "gotoLine",
  "closeTab",
  "nextTab",
  "prevTab",
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
  "splitFile",
  "grepFolder",
  "grepSave",
  "settings",
  "keymap",
  "selectAll",
  "copy",
  "cut",
  "caseUpper",
  "caseLower",
  "caseCamel",
  "casePascal",
  "caseSnake",
  "caseKebab",
  "caseConstant",
  "redo",
  "undo",
];

let globalShortcutActionsByKey = new Map<string, string[]>();

// The action table is rebuilt when settings/key hints change, not for every
// keydown. Preserve GLOBAL_SHORTCUT_ACTIONS order so conflicts resolve exactly
// as the previous linear scan did.
export function rebuildGlobalShortcutActions() {
  const next = new Map<string, string[]>();
  for (const action of GLOBAL_SHORTCUT_ACTIONS) {
    for (const shortcut of shortcutList(action)) {
      const actions = next.get(shortcut) || [];
      if (!actions.includes(action)) actions.push(action);
      next.set(shortcut, actions);
    }
  }
  globalShortcutActionsByKey = next;
}

export function shortcutActionFromEvent(e, inField = false) {
  const shortcut = eventShortcut(e);
  if (!shortcut) return null;
  if (!globalShortcutActionsByKey.size) rebuildGlobalShortcutActions();
  for (const action of globalShortcutActionsByKey.get(shortcut) || []) {
    const entry = ACTIONS[action];
    if (!entry?.globalShortcut || (inField && entry.editorOnly)) continue;
    return action;
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

// ---- ARIA menubar keyboard contract (#161) ---------------------------------
// The five triggers that make up the role=menubar. The ツール dropdown lives in
// the toolbar (not the menubar), so it is excluded from left/right roving.
function menubarButtons(): HTMLElement[] {
  return APP_MENUS.map((id) => $(`${id}-menu-button`));
}

// Enabled, focusable rows inside a dropdown, in DOM order — arrow keys skip
// separators and disabled entries so focus never lands on a dead row.
function menuItemsOf(id): HTMLElement[] {
  return [...$(`${id}-menu`).querySelectorAll<HTMLElement>(".menu-item")].filter(
    (el) => !(el as HTMLButtonElement).disabled,
  );
}

// Roving tabindex: exactly one menubar trigger is in the Tab order at a time.
function setMenubarRoving(idx: number) {
  const btns = menubarButtons();
  const i = ((idx % btns.length) + btns.length) % btns.length;
  btns.forEach((b, j) => b.setAttribute("tabindex", j === i ? "0" : "-1"));
  return btns[i];
}

// Open a menubar menu and move focus onto an item (first, or last for ArrowUp).
function openMenuWithFocus(id, edge: "first" | "last" = "first") {
  showAppMenu(id);
  const bi = APP_MENUS.indexOf(id);
  if (bi >= 0) setMenubarRoving(bi);
  const items = menuItemsOf(id);
  if (items.length) (edge === "last" ? items[items.length - 1] : items[0]).focus();
}

// F10 (and the native path) drops keyboard focus onto the menubar.
export function focusMenubar() {
  if (anyModalOpen()) return;
  const btns = menubarButtons();
  const active = btns.find((b) => b.getAttribute("tabindex") === "0") || btns[0];
  active.focus();
}

function onMenubarButtonKey(e: KeyboardEvent, id) {
  const idx = APP_MENUS.indexOf(id);
  const last = APP_MENUS.length - 1;
  const moveTo = (ni: number) => {
    const wasOpen = fileMenuVisible();
    setMenubarRoving(ni).focus();
    if (wasOpen)
      showAppMenu(APP_MENUS[((ni % APP_MENUS.length) + APP_MENUS.length) % APP_MENUS.length]);
  };
  switch (e.key) {
    case "ArrowRight":
      e.preventDefault();
      moveTo(idx + 1);
      break;
    case "ArrowLeft":
      e.preventDefault();
      moveTo(idx - 1);
      break;
    case "Home":
      e.preventDefault();
      moveTo(0);
      break;
    case "End":
      e.preventDefault();
      moveTo(last);
      break;
    case "ArrowDown":
      e.preventDefault();
      openMenuWithFocus(id, "first");
      break;
    case "ArrowUp":
      e.preventDefault();
      openMenuWithFocus(id, "last");
      break;
    // Enter / Space activate the trigger through its native click below, which
    // opens the menu and moves focus to the first item.
    case "Escape":
      if (fileMenuVisible()) {
        e.preventDefault();
        hideFileMenu();
      }
      break;
  }
}

function onMenuKey(e: KeyboardEvent, id) {
  const items = menuItemsOf(id);
  const menuIdx = APP_MENUS.indexOf(id); // -1 for the toolbar ツール menu
  const pos = items.indexOf(document.activeElement as HTMLElement);
  switch (e.key) {
    case "ArrowDown":
      e.preventDefault();
      if (items.length) items[pos < 0 ? 0 : (pos + 1) % items.length].focus();
      break;
    case "ArrowUp":
      e.preventDefault();
      if (items.length)
        items[pos < 0 ? items.length - 1 : (pos - 1 + items.length) % items.length].focus();
      break;
    case "Home":
      e.preventDefault();
      items[0]?.focus();
      break;
    case "End":
      e.preventDefault();
      items[items.length - 1]?.focus();
      break;
    case "ArrowRight":
      if (menuIdx >= 0) {
        e.preventDefault();
        openMenuWithFocus(APP_MENUS[(menuIdx + 1) % APP_MENUS.length], "first");
      }
      break;
    case "ArrowLeft":
      if (menuIdx >= 0) {
        e.preventDefault();
        openMenuWithFocus(APP_MENUS[(menuIdx - 1 + APP_MENUS.length) % APP_MENUS.length], "first");
      }
      break;
    case "Escape":
      e.preventDefault();
      hideFileMenu();
      $(`${id}-menu-button`).focus();
      break;
    case "Tab":
      // Leave the menu the way a native menu does: close and let Tab move on.
      hideFileMenu();
      break;
    // Enter / Space fall through to the button's native activation.
  }
}

export function initMenuBar() {
  for (const id of DROPDOWN_MENUS) {
    const button = $(`${id}-menu-button`);
    button.addEventListener("click", (e) => {
      e.stopPropagation();
      const open = !$(`${id}-menu`).classList.contains("hidden");
      if (open) {
        hideFileMenu();
      } else if ((e as MouseEvent).detail === 0) {
        // Keyboard activation (Enter/Space synthesize a click with detail 0):
        // open and drop focus onto the first item.
        openMenuWithFocus(id, "first");
      } else {
        showAppMenu(id);
      }
    });
    button.addEventListener("keydown", (e) => onMenubarButtonKey(e, id));
    $(`${id}-menu`).addEventListener("keydown", (e) => onMenuKey(e, id));
    // Items are reached with the arrow keys, never a plain Tab stop.
    menuItemsOf(id).forEach((item) => item.setAttribute("tabindex", "-1"));
    if (APP_MENUS.includes(id)) {
      button.addEventListener("pointerenter", () => {
        if (fileMenuVisible()) showAppMenu(id);
      });
    }
  }
  // Roving tabindex: only the first menubar trigger is tabbable to start with,
  // and whichever trigger takes focus becomes the single Tab stop.
  const btns = menubarButtons();
  btns.forEach((b, j) => b.setAttribute("tabindex", j === 0 ? "0" : "-1"));
  $("menubar").addEventListener("focusin", (e) => {
    const i = btns.indexOf(e.target as HTMLElement);
    if (i >= 0) btns.forEach((b, j) => b.setAttribute("tabindex", j === i ? "0" : "-1"));
  });
  // F10 moves keyboard focus to the menubar (Alt is already spent on search /
  // line-move bindings, so the menubar uses the other standard activation key).
  document.addEventListener("keydown", (e) => {
    if (
      e.key === "F10" &&
      !e.ctrlKey &&
      !e.metaKey &&
      !e.altKey &&
      !e.shiftKey &&
      !anyModalOpen()
    ) {
      e.preventDefault();
      focusMenubar();
    }
  });
  document.querySelectorAll("[data-menu-action]").forEach((item) => {
    item.addEventListener("click", () => runMenuAction((item as any).dataset.menuAction));
  });
}
