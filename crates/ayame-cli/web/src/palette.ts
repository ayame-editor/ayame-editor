// Ayame Editor — command palette.
import { $, setModalOpen } from "./dom.js";
import { KEYMAP_ACTIONS } from "./state.js";
import { t } from "./i18n.js";
import { focusEditor } from "./editor.js";
import { formVisible, promptVisible } from "./dialogs.js";
import { displayShortcut } from "./keys.js";
import { hideFileMenu } from "./menu-surface.js";
import { MENU_ID_ACTIONS } from "./menu-constants.js";
import { shortcutFor } from "./keymap-menu.js";

export let paletteItems = [];
export let paletteIndex = 0;
let runPaletteAction = (_action) => {};

export function setPaletteActionRunner(run) {
  runPaletteAction = run;
}

// applyLocale rebuilds the palette on a language switch; use a setter because
// imported bindings themselves are read-only.
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
  queueMicrotask(() => runPaletteAction(item.action));
}

export function initCommandPalette() {
  $("palette-close").addEventListener("click", hideCommandPalette);
  $("palette-input").addEventListener("input", () => {
    paletteIndex = 0;
    renderCommandPalette();
  });
  $("palette-input").addEventListener("keydown", (e) => {
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
