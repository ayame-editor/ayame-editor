// Ayame Editor — shared UI persistence.
//
// Native WebViews can have separate localStorage profiles. The server-backed
// UI state keeps recents, search history, and session restore data shared
// across windows while preserving localStorage as the browser fallback.

import {
  ANALYSIS_PROFILES_KEY,
  RECENT_KEY,
  RECENT_MAX,
  SEARCH_HISTORY_KEY,
  state,
} from "./state.js";
import { api, apiPost } from "./api.js";
import { normalizeAnalysisProfiles } from "./analysis-model.js";
import type { AnalysisProfile } from "./types/api.js";
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
  const analysisProfiles = normalizeAnalysisProfiles(ui.analysis_profiles);
  const active = String(ui.active_analysis_profile || "").trim();
  return {
    recent_files: cleanList(ui.recent_files, RECENT_MAX),
    search_history: cleanList(ui.search_history, 50),
    session: {
      paths: cleanList(ui.session?.paths, 64),
      active_path: String(ui.session?.active_path || "").trim() || null,
    },
    analysis_profiles: analysisProfiles,
    active_analysis_profile: analysisProfiles.some((profile) => profile.id === active)
      ? active
      : null,
  };
}

export async function hydrateSharedUiState() {
  try {
    const remote = normalizeUiState(await api<UiState>("/api/ui_state"));
    const localRecent = localList(RECENT_KEY, RECENT_MAX);
    const localHistory = localList(SEARCH_HISTORY_KEY, 50);
    let localProfiles = [];
    try {
      localProfiles = normalizeAnalysisProfiles(
        JSON.parse(localStorage.getItem(ANALYSIS_PROFILES_KEY) || "[]"),
      );
    } catch {
      // ignore malformed local fallback
    }
    const merged = normalizeUiState({
      ...remote,
      recent_files: remote.recent_files.length ? remote.recent_files : localRecent,
      search_history: remote.search_history.length ? remote.search_history : localHistory,
      analysis_profiles: remote.analysis_profiles.length ? remote.analysis_profiles : localProfiles,
    });
    sharedUiState = merged;
    state.history = merged.search_history;
    state.analysisProfiles = merged.analysis_profiles;
    state.activeAnalysisProfile = merged.active_analysis_profile;
    if (
      merged.recent_files.length !== remote.recent_files.length ||
      merged.search_history.length !== remote.search_history.length ||
      merged.analysis_profiles.length !== remote.analysis_profiles.length
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

// The base object a partial UI-state write merges into. When boot hydration
// failed `sharedUiState` is null; spreading null yields `{}`, which would send
// empty search_history + session and wipe the server's stored copy (issue #73).
// Re-read the current state first; if that also fails, return null so the caller
// skips the server write and keeps only the local fallback.
async function currentUiStateBase(): Promise<UiState | null> {
  if (sharedUiState) return sharedUiState;
  try {
    sharedUiState = normalizeUiState(await api<UiState>("/api/ui_state"));
    return sharedUiState;
  } catch {
    return null;
  }
}

export function saveRecentFilesShared(list) {
  const recent = cleanList(list, RECENT_MAX);
  saveLocalList(RECENT_KEY, recent, RECENT_MAX);
  void currentUiStateBase().then((base) => {
    if (!base) return; // can't read current state — don't clobber the server
    void saveSharedUiState(normalizeUiState({ ...base, recent_files: recent }));
  });
}

export function loadSearchHistoryShared() {
  return sharedUiState?.search_history || localList(SEARCH_HISTORY_KEY, 50);
}

export function saveSearchHistoryShared(list) {
  const history = cleanList(list, 50);
  saveLocalList(SEARCH_HISTORY_KEY, history, 50);
  void currentUiStateBase().then((base) => {
    if (!base) return; // can't read current state — don't clobber the server
    void saveSharedUiState(normalizeUiState({ ...base, search_history: history }));
  });
}

export function loadAnalysisProfilesShared(): {
  profiles: AnalysisProfile[];
  active: string | null;
} {
  if (sharedUiState) {
    return {
      profiles: sharedUiState.analysis_profiles,
      active: sharedUiState.active_analysis_profile,
    };
  }
  try {
    const profiles = normalizeAnalysisProfiles(
      JSON.parse(localStorage.getItem(ANALYSIS_PROFILES_KEY) || "[]"),
    );
    return { profiles, active: state.activeAnalysisProfile };
  } catch {
    return { profiles: [], active: null };
  }
}

export function saveAnalysisProfilesShared(profiles, active) {
  const clean = normalizeAnalysisProfiles(profiles);
  const activeId = clean.some((profile) => profile.id === active) ? active : null;
  state.analysisProfiles = clean;
  state.activeAnalysisProfile = activeId;
  try {
    localStorage.setItem(ANALYSIS_PROFILES_KEY, JSON.stringify(clean));
  } catch {
    // ignore private-mode quota errors
  }
  void currentUiStateBase().then((base) => {
    if (!base) return;
    void saveSharedUiState(
      normalizeUiState({
        ...base,
        analysis_profiles: clean,
        active_analysis_profile: activeId,
      }),
    );
  });
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
