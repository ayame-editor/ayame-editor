// Ayame Editor — editor context menu.
import { $ } from "./dom.js";
import { state } from "./state.js";
import { coordsFromEvent, focusEditor, scheduleRender, setCaret } from "./editor.js";
import { hasTextSelection, posInsideSelection, saveSelectionToFile } from "./selection.js";
import { anyModalOpen } from "./modal-state.js";
import { runMenuAction } from "./menu-actions.js";

export function ctxMenuVisible() {
  return !$("ctx-menu").classList.contains("hidden");
}

export function hideCtxMenu() {
  $("ctx-menu").classList.add("hidden");
}

export function runCtxAction(action) {
  hideCtxMenu();
  let out;
  if (action === "saveSelection") out = saveSelectionToFile();
  else out = runMenuAction(action);
  return Promise.resolve(out).finally(() => {
    if (!anyModalOpen() && !state.findOpen) focusEditor();
  });
}

export function initContextMenu() {
  const menu = $("ctx-menu");
  $("viewport").addEventListener("contextmenu", (e) => {
    e.preventDefault();
    if (!state.stat?.open || anyModalOpen()) return;
    const p = coordsFromEvent(e);
    if (!posInsideSelection(p)) {
      state.sel = null;
      setCaret(p.line, p.col);
      scheduleRender();
    }
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
