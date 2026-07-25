// Ayame Editor — dropdown content, localization, and dynamic menu state.
import { $, displayName, displayPath, pathDirName } from "./dom.js";
import { state } from "./state.js";
import { applyStaticI18n, t } from "./i18n.js";
import { hasTextSelection } from "./selection.js";
import { updateTailUI } from "./edits.js";
import { updateFindCountLabel } from "./findbar.js";
import {
  configureOpener,
  loadRecentFiles,
  openRecent,
  openerVisible,
  renderRecentFiles,
  renderTabs,
} from "./workspace.js";
import { populateLanguageSelect } from "./settings.js";
import { hideFileMenu } from "./menu-surface.js";
import { updateStatusMeta, updateStatusPos } from "./status.js";
import { keymapVisible, renderKeymapRows, updateKeyHints } from "./keymap-menu.js";
import {
  commandPaletteItems,
  commandPaletteVisible,
  renderCommandPalette,
  setPaletteItems,
} from "./palette.js";

export function showAppMenu(id) {
  hideFileMenu();
  if (id === "file") {
    renderFileMenuRecentFiles();
    const saveAll = document.getElementById("save-all") as HTMLButtonElement | null;
    const close = document.getElementById("close-tab") as HTMLButtonElement | null;
    if (saveAll) saveAll.disabled = !(state.tabs || []).some((tab) => tab.dirty);
    if (close) close.disabled = !(state.tabs || []).some((tab) => tab.active);
  }
  if (id === "view") {
    for (const [id, on] of [
      ["menu-toggle-ws", !!state.settings.showWhitespace],
      ["menu-toggle-syntax", state.settings.syntaxHighlight !== false],
      ["menu-toggle-change-history", state.settings.showChangeHistory !== false],
      ["menu-toggle-minimap", state.settings.minimap !== false],
      ["menu-toggle-zsp-underline", !!state.settings.zenkakuUnderline],
      ["menu-toggle-wrap", !!state.settings.wordWrap],
      ["menu-toggle-tail", state.followTail],
    ] as const) {
      const item = $(id);
      if (!item) continue;
      item.classList.toggle("checked", on);
      item.setAttribute("aria-checked", String(on));
    }
  }
  if (id === "edit") {
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

export function renderFileMenuRecentFiles() {
  const section = document.getElementById("file-menu-recent-section");
  const box = document.getElementById("file-menu-recents");
  if (!section || !box) return;
  const list = loadRecentFiles();
  box.textContent = "";
  section.classList.toggle("hidden", !list.length);
  for (const path of list) {
    const item = document.createElement("button");
    item.type = "button";
    item.className = "menu-item";
    item.setAttribute("role", "menuitem");
    item.setAttribute("tabindex", "-1");
    item.title = displayPath(path);
    item.setAttribute("aria-label", `${t("menu.recentFiles")}: ${displayPath(path)}`);
    const name = document.createElement("span");
    name.className = "menu-label";
    name.textContent = displayName(path);
    const dir = document.createElement("span");
    dir.className = "menu-key menu-recent-path";
    dir.textContent = pathDirName(displayPath(path)) || "";
    item.append(name, dir);
    item.addEventListener("click", () => {
      hideFileMenu();
      void openRecent(path);
    });
    box.append(item);
  }
}

export function applyLocale() {
  applyStaticI18n();
  populateLanguageSelect();
  updateKeyHints();
  updateStatusMeta();
  updateStatusPos();
  updateFindCountLabel();
  updateTailUI();
  if (openerVisible()) configureOpener(state.openerMode);
  if (state.tabs?.length) renderTabs(state.tabs);
  if (keymapVisible()) renderKeymapRows();
  if (commandPaletteVisible()) {
    setPaletteItems(commandPaletteItems());
    renderCommandPalette();
  }
  renderRecentFiles();
  renderFileMenuRecentFiles();
}
