// Ayame Editor — configurable shortcuts and keymap editor.
import { $, el, setModalOpen } from "./dom.js";
import { DEFAULT_KEYMAP, KEYMAP_ACTIONS, state } from "./state.js";
import { t } from "./i18n.js";
import { focusEditor } from "./editor.js";
import { flashCount } from "./notifications.js";
import {
  displayShortcut,
  eventShortcut,
  isBindableShortcut,
  matchesShortcut as eventMatchesShortcut,
  normalizeShortcut,
} from "./keys.js";
import { hideSettings, saveSettings } from "./settings.js";
import { syncNativeMenu } from "./native-menu.js";

let rebuildShortcutMap = () => {};

export function setShortcutMapRebuilder(rebuild) {
  rebuildShortcutMap = rebuild;
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
  rebuildShortcutMap();
  syncNativeMenu(shortcutFor);
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
  $<HTMLInputElement>("find").placeholder = hint("menu.find", "find");
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
    const row = el("label", "keymap-row");
    const shortcut = shortcutFor(action);
    if (shortcut && used.get(shortcut) > 1) row.classList.add("conflict");
    const name = el("span", "keymap-label", t(label));
    const input = el("input", "keymap-input input-control");
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
