// Compatibility facade and explicit wiring for workspace responsibilities.
import { $ } from "./dom.js";
import { setSaveWorkspaceService } from "./save.js";
import { fileMenuVisible, hideFileMenu } from "./menu-surface.js";
import { setBrowseService } from "./browse.js";
import { setRecentService } from "./recent.js";
import {
  commitOpener,
  initDropZone,
  initOpener,
  newUntitled,
  onDocumentOpened,
  openFileDialog,
  openPath,
  showSaveDialog,
} from "./opener.js";
import { initTabs, refreshTabs, selectTab, setTabsWorkspaceService } from "./tabs.js";

export * from "./browse.js";
export * from "./recent.js";
export * from "./opener.js";
export * from "./tabs.js";

let workspaceInitialized = false;

export function initWorkspace() {
  if (workspaceInitialized) return;
  workspaceInitialized = true;

  setBrowseService({ commitOpener, openPath });
  setRecentService({ openPath });
  setTabsWorkspaceService({ newUntitled, onDocumentOpened, openPath });
  setSaveWorkspaceService({
    onDocumentOpened,
    openPath,
    refreshTabs,
    selectTab,
    showSaveDialog,
  });

  document.addEventListener("pointerdown", (event) => {
    if (fileMenuVisible() && !(event.target as any).closest(".menu-shell")) hideFileMenu();
  });
  $("new-file").addEventListener("click", () => {
    hideFileMenu();
    void newUntitled();
  });
  $("open-file").addEventListener("click", () => {
    hideFileMenu();
    void openFileDialog();
  });
  initOpener();
  initTabs();
  initDropZone();
}
