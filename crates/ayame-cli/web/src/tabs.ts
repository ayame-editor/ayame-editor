// Ayame Editor — tab rendering, ordering, switching, closing, and drag/drop.
import { $, displayPath, iconSvg, isUntitled, pathDirName } from "./dom.js";
import { state } from "./state.js";
import { t } from "./i18n.js";
import { api, apiPost } from "./api.js";
import { confirmCloseLastTab, isNativeApp, openNewWindow, requestEditorClose } from "./app.js";
import { expectWalHandoff, saveCopy, savingCount } from "./save.js";
import { showPopupMenu } from "./popup-menu.js";
import { settleEditQueue } from "./edits.js";
import { flashCount } from "./notifications.js";
import { askConfirm } from "./dialogs.js";
import type { TabIdRequest, TabReorderRequest, TabsResponse } from "./types/api.js";

let activateDocument = (_stat) => {};
let createUntitled = async () => {};
let openDocumentPath = async (_path) => false;

export function setTabsWorkspaceService(service) {
  activateDocument = service.onDocumentOpened;
  createUntitled = service.newUntitled;
  openDocumentPath = service.openPath;
}

let refreshTabsSeq = 0;
const TAB_DRAG_TYPE = "application/x-ayame-tab";
let draggingTabId: number | null = null;
let localDropHandledId: number | null = null;

export function tabOrderAfterMove(list, id, beforeId) {
  const next = [...list];
  const from = next.findIndex((tab) => tab.id === id);
  if (from < 0) return null;
  if (beforeId === id) return next;
  if (beforeId != null && !next.some((tab) => tab.id === beforeId)) return null;
  const [moved] = next.splice(from, 1);
  const to = beforeId == null ? next.length : next.findIndex((tab) => tab.id === beforeId);
  next.splice(to, 0, moved);
  return next;
}

export function tabDropBeforeId(list, draggedId, targetId, after) {
  const draggedIndex = list.findIndex((tab) => tab.id === draggedId);
  if (draggedIndex < 0) return null;
  if (draggedId === targetId) return list[draggedIndex + 1]?.id ?? null;
  const remaining = list.filter((tab) => tab.id !== draggedId);
  const targetIndex = remaining.findIndex((tab) => tab.id === targetId);
  if (targetIndex < 0) return null;
  return remaining[targetIndex + (after ? 1 : 0)]?.id ?? null;
}

export function ensureActiveTabVisible(container = document.getElementById("tabs")) {
  container
    ?.querySelector<HTMLElement>(".tab.active")
    ?.scrollIntoView?.({ block: "nearest", inline: "nearest" });
}

export function showTabList() {
  const button = $("tab-list");
  const rect = button.getBoundingClientRect();
  showPopupMenu(
    rect.right,
    rect.bottom,
    (state.tabs || []).map((tab) => {
      const folder = tab.path && !isUntitled(tab.path) ? pathDirName(displayPath(tab.path)) : "";
      return {
        label: folder ? `${tab.name} — ${folder}` : tab.name,
        checked: !!tab.active,
        action: () => {
          if (!tab.active) void selectTab(tab.id);
        },
      };
    }),
  );
}

export async function reorderTab(id, beforeId) {
  const reordered = tabOrderAfterMove(state.tabs || [], id, beforeId);
  if (!reordered) return false;
  if (reordered.every((tab, index) => tab.id === state.tabs[index]?.id)) return true;
  try {
    const response = await apiPost<TabsResponse, TabReorderRequest>("/api/tabs/reorder", {
      id,
      before_id: beforeId,
    });
    renderTabs(response.tabs);
    return true;
  } catch (error) {
    flashCount(t("tab.reorderError"), "error");
    console.error(error);
    return false;
  }
}

function eventTab(event: Event) {
  return event.target instanceof Element ? event.target.closest<HTMLElement>(".tab") : null;
}

function beforeIdAtPointer(event: DragEvent, draggedId: number) {
  const target = eventTab(event);
  if (!target) return null;
  const box = target.getBoundingClientRect();
  return tabDropBeforeId(
    state.tabs || [],
    draggedId,
    Number(target.dataset.id),
    event.clientX >= box.left + box.width / 2,
  );
}

function clearTabDropTarget() {
  for (const tab of document.querySelectorAll(".tab-drop-before, .tab-drop-after")) {
    tab.classList.remove("tab-drop-before", "tab-drop-after");
  }
}

function markTabDropTarget(event: DragEvent, draggedId: number) {
  clearTabDropTarget();
  const target = eventTab(event);
  if (!target || Number(target.dataset.id) === draggedId) return;
  const box = target.getBoundingClientRect();
  target.classList.add(
    event.clientX >= box.left + box.width / 2 ? "tab-drop-after" : "tab-drop-before",
  );
}

export async function refreshTabs() {
  const sequence = ++refreshTabsSeq;
  try {
    const response = await api<TabsResponse>("/api/tabs");
    if (sequence !== refreshTabsSeq) return;
    renderTabs(response.tabs);
  } catch {
    // Non-fatal: the tab bar just won't update.
  }
}

export async function closeTabsSequentially(ids, close = closeTab) {
  for (const id of ids) await close(id);
}

export function renderTabs(list) {
  state.tabs = list;
  const container = $("tabs");
  const listButton = document.getElementById("tab-list") as HTMLButtonElement | null;
  if (listButton) listButton.disabled = list.length === 0;
  container.setAttribute("role", "tablist");
  container.textContent = "";
  for (const tab of list) {
    const element = document.createElement("div");
    element.className = "tab" + (tab.active ? " active" : "") + (tab.dirty ? " dirty" : "");
    element.dataset.id = String(tab.id);
    element.title = displayPath(tab.path);
    element.setAttribute("role", "tab");
    element.setAttribute("aria-selected", tab.active ? "true" : "false");
    element.tabIndex = 0;
    element.draggable = true;
    const dot = document.createElement("span");
    dot.className = "tab-dot";
    const name = document.createElement("span");
    name.className = "tab-name";
    name.textContent = tab.name;
    const close = document.createElement("button");
    close.type = "button";
    close.className = "tab-x";
    close.append(iconSvg("i-close"));
    close.title = t("common.close");
    close.setAttribute("aria-label", t("tab.closeName", { name: tab.name }));
    element.append(dot, name, close);
    element.addEventListener("click", () => {
      if (!tab.active) void selectTab(tab.id);
    });
    element.addEventListener("keydown", (event) => {
      if (event.key === "Enter" || event.key === " ") {
        event.preventDefault();
        if (!tab.active) void selectTab(tab.id);
      } else if (event.key === "Delete") {
        event.preventDefault();
        void closeTab(tab.id);
      }
    });
    element.addEventListener("mousedown", (event) => {
      if (event.button === 1) {
        event.preventDefault();
        void closeTab(tab.id);
      }
    });
    element.addEventListener("contextmenu", (event) => {
      event.preventDefault();
      showPopupMenu(event.clientX, event.clientY, [
        { label: t("common.close"), action: () => closeTab(tab.id) },
        {
          label: t("menu.saveAs"),
          action: async () => {
            if (!tab.active) await selectTab(tab.id);
            void saveCopy();
          },
        },
        {
          label: t("tab.copyPath"),
          disabled: !tab.path,
          action: () => {
            navigator.clipboard?.writeText(displayPath(tab.path)).catch(() => {});
          },
        },
        { separator: true },
        {
          label: t("tab.closeOthers"),
          disabled: state.tabs.length < 2,
          action: async () => {
            const others = state.tabs
              .filter((entry) => entry.id !== tab.id)
              .map((entry) => entry.id);
            await closeTabsSequentially(others);
          },
        },
      ]);
    });
    element.addEventListener("dragstart", (event) => startTabDrag(event, tab));
    element.addEventListener("dragend", (event) => void finishTabDrag(event, tab));
    close.addEventListener("click", (event) => {
      event.stopPropagation();
      void closeTab(tab.id);
    });
    container.append(element);
  }
  ensureActiveTabVisible(container);
}

export function tabDragPayload(tab) {
  return {
    sourceWindowId: state.windowId,
    id: tab.id,
    path: tab.path,
    name: tab.name,
    dirty: !!tab.dirty,
  };
}

export function startTabDrag(event, tab) {
  if (!event.dataTransfer) return;
  if (event.target instanceof Element && event.target.closest(".tab-x")) {
    event.preventDefault();
    return;
  }
  draggingTabId = tab.id;
  if (event.currentTarget instanceof HTMLElement) event.currentTarget.classList.add("dragging");
  event.dataTransfer.effectAllowed = "move";
  event.dataTransfer.setData(TAB_DRAG_TYPE, JSON.stringify(tabDragPayload(tab)));
  event.dataTransfer.setData("text/plain", tab.path || tab.name || "");
}

export function canHandoffDirtyTab(tab) {
  return isNativeApp() && !!tab?.path && !isUntitled(tab.path);
}

export function canDragOutToNewWindow(tab) {
  if (!tab || !tab.path || isUntitled(tab.path)) return false;
  return !tab.dirty || canHandoffDirtyTab(tab);
}

export async function finishTabDrag(event, tab) {
  clearTabDropTarget();
  if (event.currentTarget instanceof HTMLElement) event.currentTarget.classList.remove("dragging");
  draggingTabId = null;
  if (localDropHandledId === tab.id) {
    localDropHandledId = null;
    return;
  }
  if (savingCount > 0) return;
  const dropped = event.dataTransfer?.dropEffect === "move";
  const outside =
    event.clientX < 0 ||
    event.clientY < 0 ||
    event.clientX > window.innerWidth ||
    event.clientY > window.innerHeight;
  if (tab.dirty && !canHandoffDirtyTab(tab)) {
    if (dropped || outside) flashCount(t("tab.moveDirty"), "error");
    return;
  }
  if (dropped) {
    if (tab.dirty) await detachMovedTab(tab.id);
    else await closeMovedTab(tab.id);
  } else if (outside) {
    if (!canDragOutToNewWindow(tab)) return;
    if (tab.dirty) {
      let stat;
      try {
        stat = await apiPost<{ open: boolean }, TabIdRequest>("/api/tabs/detach", { id: tab.id });
      } catch (error) {
        flashCount(t("tab.handoffError"), "error");
        console.error(error);
        return;
      }
      openNewWindow(tab.path, true);
      if (!stat.open) requestEditorClose();
      else activateDocument(stat);
    } else {
      openNewWindow(tab.path);
      await closeMovedTab(tab.id);
    }
  }
}

export function initTabDropTarget() {
  const container = $("tabs");
  const acceptsDirty = (payload) => isNativeApp() && !!payload.path && !isUntitled(payload.path);
  container.addEventListener("dragover", (event) => {
    if (draggingTabId != null) {
      event.preventDefault();
      if (event.dataTransfer) event.dataTransfer.dropEffect = "move";
      markTabDropTarget(event, draggingTabId);
      return;
    }
    const raw = event.dataTransfer?.getData(TAB_DRAG_TYPE);
    if (!raw) return;
    const payload = parseTabDragPayload(raw);
    if (!payload || payload.sourceWindowId === state.windowId) return;
    if (payload.dirty && !acceptsDirty(payload)) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "move";
  });
  container.addEventListener("dragleave", (event) => {
    if (draggingTabId != null && !container.contains(event.relatedTarget as Node | null)) {
      clearTabDropTarget();
    }
  });
  container.addEventListener("drop", async (event) => {
    if (draggingTabId != null) {
      const id = draggingTabId;
      const beforeId = beforeIdAtPointer(event, id);
      event.preventDefault();
      localDropHandledId = id;
      clearTabDropTarget();
      await reorderTab(id, beforeId);
      return;
    }
    const payload = parseTabDragPayload(event.dataTransfer?.getData(TAB_DRAG_TYPE));
    if (!payload || payload.sourceWindowId === state.windowId) return;
    event.preventDefault();
    if (payload.dirty) {
      if (!acceptsDirty(payload)) {
        flashCount(t("tab.moveDirty"), "error");
        return;
      }
      expectWalHandoff(payload.path);
      await new Promise((resolve) => setTimeout(resolve, 250));
      await openDocumentPath(payload.path);
      return;
    }
    await openDocumentPath(payload.path);
  });
}

export async function detachMovedTab(id) {
  try {
    const stat = await apiPost<{ open: boolean }, TabIdRequest>("/api/tabs/detach", { id });
    if (!stat.open) requestEditorClose();
    else activateDocument(stat);
  } catch (error) {
    flashCount(t("tab.handoffError"), "error");
    console.error(error);
  }
}

export function parseTabDragPayload(raw) {
  try {
    const payload = JSON.parse(raw || "{}");
    if (!payload || typeof payload.path !== "string" || !payload.path.trim()) return null;
    return payload;
  } catch {
    return null;
  }
}

export async function closeMovedTab(id) {
  try {
    const stat = await apiPost<{ open: boolean }, TabIdRequest>("/api/tabs/close", { id });
    if (!stat.open) requestEditorClose();
    else activateDocument(stat);
  } catch (error) {
    flashCount(t("tab.closeError"));
    console.error(error);
  }
}

export async function selectTab(id) {
  try {
    await settleEditQueue();
    activateDocument(await apiPost<unknown, TabIdRequest>("/api/tabs/select", { id }));
    return true;
  } catch (error) {
    flashCount(t("tab.switchError"));
    console.error(error);
    return false;
  }
}

export function selectRelativeTab(delta) {
  const tabs = state.tabs || [];
  if (tabs.length < 2) return;
  const active = tabs.findIndex((tab) => tab.active);
  const from = active < 0 ? 0 : active;
  const next = tabs[(from + delta + tabs.length) % tabs.length];
  if (next && !next.active) void selectTab(next.id);
}

export async function closeTab(id) {
  await settleEditQueue();
  if (savingCount > 0) {
    flashCount(t("editor.savingWait"));
    return;
  }
  const tab = state.tabs.find((entry) => entry.id === id);
  const isLast = (state.tabs || []).length <= 1;
  if (isLast) {
    if (!(await confirmCloseLastTab(tab))) return;
    if (requestEditorClose()) return;
  } else if (tab && tab.dirty) {
    const ok = await askConfirm(t("tab.close"), t("tab.confirmDiscard", { name: tab.name }), {
      okLabel: t("tab.discardClose"),
      danger: true,
    });
    if (!ok) return;
  }
  try {
    const stat = await apiPost<{ open: boolean }, TabIdRequest>("/api/tabs/close", { id });
    if (!stat.open) await createUntitled();
    else activateDocument(stat);
  } catch (error) {
    flashCount(t("tab.closeError"));
    console.error(error);
  }
}

export function initTabs() {
  $("new-tab").addEventListener("click", () => void createUntitled());
  $("tab-list").addEventListener("click", showTabList);
  initTabDropTarget();
}
