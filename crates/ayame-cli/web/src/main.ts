// Ayame Editor front-end.
//
// Design rule: the browser never holds more than the visible window. Lines are
// fetched on demand from the local server; vertical position is tracked as a
// *line number* (not pixels), so navigation is exact for any file size — ten
// lines or Ayame Editor's minimum ten-billion-line scale. A custom scrollbar maps line
// position to a thumb, side-stepping the browser's ~33M-pixel element-height
// ceiling entirely.

import { $, displayName, setModalOpen } from "./dom.js";
import { state } from "./state.js";
import { t } from "./i18n.js";
import { apiPost } from "./api.js";
import { initApp, postNativeMessage } from "./app.js";
import {
  expectWalHandoff,
  initSave,
  maybeOfferWalRecovery,
  refreshStat,
  setSaveSettingsWriter,
} from "./save.js";
import { focusEditor, initScrollbar, render, setStatusPositionRenderer } from "./editor.js";
import { initSelection } from "./selection.js";
import { initCommandPalette, initContextMenu, initMenuBar, initMenus } from "./menus.js";
import { updateStatusMeta, updateStatusPos } from "./status.js";
import { flashCount } from "./notifications.js";
import { loadSearchHistory } from "./search.js";
import { hideLoading, showLoading } from "./dialogs.js";
import { initEditor, initEvents } from "./input.js";
import {
  initWorkspace,
  newUntitled,
  onDocumentOpened,
  openPath,
  refreshTabs,
} from "./workspace.js";
import { initSettings, saveSettings } from "./settings.js";
import { initBookmarks } from "./bookmarks.js";
import { initMinimap } from "./minimap.js";
import { handleAnalysisDocumentOpened, initAnalysis } from "./analysis.js";
import { hydrateSharedUiState, restoreSessionSnapshot } from "./persistence.js";
import type { OpenRequest, OpenResponse } from "./types/api.js";
import { initSyntaxUi } from "./syntax-ui.js";
import { initFolding } from "./fold-actions.js";
import { initCompletion } from "./completion.js";
import { initInspector } from "./inspector.js";

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
  initApp();
  initSave();
  initMenus();
  setSaveSettingsWriter(saveSettings);
  setStatusPositionRenderer(updateStatusPos);
  state.search.history = loadSearchHistory();
  initSettings();
  await hydrateSharedUiState();
  initSyntaxUi();
  state.search.history = loadSearchHistory();
  initAnalysis();
  initCommandPalette();
  initScrollbar();
  initMinimap();
  initEvents();
  initEditor();
  initCompletion();
  initInspector();
  initBookmarks();
  initSelection();
  initFolding();
  initMenuBar();
  initWorkspace();
  initContextMenu();
  try {
    await refreshStat();
  } catch (e) {
    setModalOpen($("overlay"), true);
    $("overlay").textContent = `${t("error.serverUnreachable")}: ${e.message}`;
    postNativeMessage({ type: "ready" }); // still show the window so the error is visible
    return;
  }
  updateStatusMeta();
  if (state.doc.stat?.open) handleAnalysisDocumentOpened(state.doc.stat.path);
  // Native launch with a FILE argument: the window appears immediately and the
  // (possibly long) first-index happens behind this progress overlay.
  const pending = typeof window.__ayamePendingOpen === "string" ? window.__ayamePendingOpen : "";
  if (!state.doc.stat.open && pending) {
    showLoading(t("dialog.open.openingName", { name: displayName(pending) }));
    postNativeMessage({ type: "ready" });
    try {
      // A window spawned by a dirty-tab handoff (issue #35): the detached
      // tab's crash log replays silently instead of prompting.
      if (window.__ayamePendingRecover) expectWalHandoff(pending);
      onDocumentOpened(await apiPost<OpenResponse, OpenRequest>("/api/open", { path: pending }));
    } catch (e) {
      flashCount(t("error.cannotOpen", { msg: pending }), "error");
      console.error(e);
      await newUntitled();
    } finally {
      hideLoading();
    }
    return;
  }
  if (!state.doc.stat.open && state.settings.restoreSession !== false) {
    showLoading(t("dialog.open.loading"));
    postNativeMessage({ type: "ready" });
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
  if (!state.doc.stat.open) {
    await newUntitled(); // open to a blank untitled page, not the file dialog
  } else {
    focusEditor();
    render();
    refreshTabs();
    // A document passed on the command line goes through refreshStat, not
    // onDocumentOpened — offer its crash recovery here.
    maybeOfferWalRecovery(state.doc.stat);
  }
  postNativeMessage({ type: "ready" });
}

boot();
