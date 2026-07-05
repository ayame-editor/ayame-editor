// Ayame Editor front-end.
//
// Design rule: the browser never holds more than the visible window. Lines are
// fetched on demand from the local server; vertical position is tracked as a
// *line number* (not pixels), so navigation is exact for any file size — ten
// lines or Ayame Editor's minimum ten-billion-line scale. A custom scrollbar maps line
// position to a thumb, side-stepping the browser's ~33M-pixel element-height
// ceiling entirely.

import { $, displayName } from "./dom.js";
import { state } from "./state.js";
import { t } from "./i18n.js";
import { apiPost } from "./api.js";
import { postNativeMessage } from "./app.js";
import { maybeOfferWalRecovery, refreshStat } from "./save.js";
import { focusEditor, initScrollbar, render } from "./editor.js";
import { initSelection } from "./selection.js";
import { initCommandPalette, initContextMenu, updateStatusMeta } from "./menus.js";
import { flashCount, loadSearchHistory } from "./search.js";
import { hideLoading, showLoading } from "./dialogs.js";
import { initEditor, initEvents } from "./input.js";
import {
  initTree,
  initWorkspace,
  newUntitled,
  onDocumentOpened,
  openPath,
  refreshTabs,
} from "./workspace.js";
import { initSettings } from "./settings.js";
import { hydrateSharedUiState, restoreSessionSnapshot } from "./persistence.js";
import type { OpenRequest } from "./types/api.js";

// ---- boot ------------------------------------------------------------------

// Native window: open files dropped onto the window (real paths, no copy).
window.__ayameOpenNativePaths = (paths) => {
  if (!Array.isArray(paths)) return;
  (async () => {
    for (const p of paths) {
      if (typeof p !== "string" || !p) continue;
      try {
        await openPath(p);
      } catch (e) {
        flashCount(t("error.cannotOpen", { msg: p }), "error");
        console.error(e);
      }
    }
  })();
};

export async function boot() {
  state.history = loadSearchHistory();
  initSettings();
  await hydrateSharedUiState();
  state.history = loadSearchHistory();
  initCommandPalette();
  initScrollbar();
  initEvents();
  initEditor();
  initSelection();
  initWorkspace();
  initTree();
  initContextMenu();
  try {
    await refreshStat();
  } catch (e) {
    $("overlay").classList.remove("hidden");
    $("overlay").textContent = `${t("error.serverUnreachable")}: ${e.message}`;
    postNativeMessage("ayame:ready"); // still show the window so the error is visible
    return;
  }
  updateStatusMeta();
  // Native launch with a FILE argument: the window appears immediately and the
  // (possibly long) first-index happens behind this progress overlay.
  const pending = typeof window.__ayamePendingOpen === "string" ? window.__ayamePendingOpen : "";
  if (!state.stat.open && pending) {
    showLoading(t("dialog.open.openingName", { name: displayName(pending) }));
    postNativeMessage("ayame:ready");
    try {
      onDocumentOpened(await apiPost<unknown, OpenRequest>("/api/open", { path: pending }));
    } catch (e) {
      flashCount(t("error.cannotOpen", { msg: pending }), "error");
      console.error(e);
      await newUntitled();
    } finally {
      hideLoading();
    }
    return;
  }
  if (!state.stat.open && state.settings.restoreSession !== false) {
    showLoading(t("dialog.open.loading"));
    postNativeMessage("ayame:ready");
    try {
      const stat = await restoreSessionSnapshot();
      if (stat?.open) {
        onDocumentOpened(stat);
      } else {
        await newUntitled();
      }
    } catch {
      await newUntitled();
    } finally {
      hideLoading();
    }
    return;
  }
  if (!state.stat.open) {
    await newUntitled(); // open to a blank untitled page, not the file dialog
  } else {
    focusEditor();
    render();
    refreshTabs();
    // A document passed on the command line goes through refreshStat, not
    // onDocumentOpened — offer its crash recovery here.
    maybeOfferWalRecovery(state.stat);
  }
  postNativeMessage("ayame:ready");
}

boot();
