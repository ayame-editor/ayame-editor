// Ayame Editor — shared UI persistence.
//
// Native WebViews can have separate localStorage profiles. The server-backed
// UI state keeps recents, search history, and session restore data shared
// across windows while preserving localStorage as the browser fallback.

import { RECENT_KEY, RECENT_MAX, SEARCH_HISTORY_KEY, state } from "./state.js";
import { api, apiPost } from "./api.js";
import type { UiState } from "./types/api.js";

let sharedUiState: UiState | null = null;

function cleanList(list, max) {
  const out = [];
  for (const value of Array.isArray(list) ? list : []) {
    const text = String(value || "").trim();
    if (!text || out.includes(text)) continue;
    out.push(text);
    if (out.length >= max) break;
  }
  return out;
}

function localList(key, max) {
  try {
    return cleanList(JSON.parse(localStorage.getItem(key) || "[]"), max);
  } catch {
    return [];
  }
}

function saveLocalList(key, list, max) {
  try {
    localStorage.setItem(key, JSON.stringify(cleanList(list, max)));
  } catch {
    // ignore private-mode quota errors
  }
}

function normalizeUiState(ui: Partial<UiState> = {}): UiState {
  return {
    recent_files: cleanList(ui.recent_files, RECENT_MAX),
    search_history: cleanList(ui.search_history, 50),
    session: {
      paths: cleanList(ui.session?.paths, 64),
      active_path: String(ui.session?.active_path || "").trim() || null,
    },
  };
}

export async function hydrateSharedUiState() {
  try {
    const remote = normalizeUiState(await api<UiState>("/api/ui_state"));
    const localRecent = localList(RECENT_KEY, RECENT_MAX);
    const localHistory = localList(SEARCH_HISTORY_KEY, 50);
    const merged = normalizeUiState({
      ...remote,
      recent_files: remote.recent_files.length ? remote.recent_files : localRecent,
      search_history: remote.search_history.length ? remote.search_history : localHistory,
    });
    sharedUiState = merged;
    state.history = merged.search_history;
    if (
      merged.recent_files.length !== remote.recent_files.length ||
      merged.search_history.length !== remote.search_history.length
    ) {
      await saveSharedUiState(merged);
    }
  } catch {
    sharedUiState = null;
  }
}

async function saveSharedUiState(next: UiState) {
  sharedUiState = normalizeUiState(next);
  try {
    sharedUiState = normalizeUiState(
      await apiPost<UiState, UiState>("/api/ui_state", sharedUiState),
    );
  } catch {
    // local fallback remains available
  }
}

export function loadRecentFilesShared() {
  return sharedUiState?.recent_files || localList(RECENT_KEY, RECENT_MAX);
}

export function saveRecentFilesShared(list) {
  const recent = cleanList(list, RECENT_MAX);
  saveLocalList(RECENT_KEY, recent, RECENT_MAX);
  const next = normalizeUiState({ ...sharedUiState, recent_files: recent });
  void saveSharedUiState(next);
}

export function loadSearchHistoryShared() {
  return sharedUiState?.search_history || localList(SEARCH_HISTORY_KEY, 50);
}

export function saveSearchHistoryShared(list) {
  const history = cleanList(list, 50);
  saveLocalList(SEARCH_HISTORY_KEY, history, 50);
  const next = normalizeUiState({ ...sharedUiState, search_history: history });
  void saveSharedUiState(next);
}

export async function restoreSessionSnapshot(): Promise<{ open: boolean; [key: string]: unknown }> {
  return apiPost("/api/session/restore", {});
}

export async function saveSessionSnapshot() {
  if (state.settings.restoreSession === false) return;
  try {
    sharedUiState = normalizeUiState(await apiPost<UiState>("/api/session/save", {}));
  } catch {
    // close paths must never be blocked by persistence
  }
}

// Flush the session snapshot during page unload. A plain fetch (as issued by
// saveSessionSnapshot) is aborted when the document is torn down, so the final
// snapshot is frequently lost; sendBeacon is delivered by the browser after the
// page is gone (issue #73). The server builds the snapshot from an empty body,
// so no payload is needed. Returns true if the beacon was queued.
export function beaconSessionSnapshot(): boolean {
  if (state.settings.restoreSession === false) return false;
  try {
    const body = new Blob(["{}"], { type: "application/json" });
    return navigator.sendBeacon("/api/session/save", body);
  } catch {
    return false;
  }
}
