// Ayame Editor — shared UI persistence.
//
// Native WebViews can have separate localStorage profiles. The server-backed
// UI state keeps recents, search history, and session restore data shared
// across windows while preserving localStorage as the browser fallback.

import {
  ANALYSIS_PROFILES_KEY,
  RECENT_KEY,
  RECENT_MAX,
  REPLACE_HISTORY_KEY,
  SEARCH_HISTORY_KEY,
  state,
} from "./state.js";
import { api, apiPost } from "./api.js";
import { normalizeAnalysisProfiles } from "./analysis-model.js";
import type { AnalysisProfile, StatResponse } from "./types/api.js";
import type { UiState } from "./types/api.js";
import {
  defaultSyntaxPreferences,
  moveSyntaxOverride,
  normalizeSyntaxPreferences,
  SYNTAX_PREFERENCES_KEY,
  syntaxPreferenceJson,
  type SyntaxPreferences,
} from "./syntax-preference-model.js";

let sharedUiState: UiState | null = null;
let sharedUiStateWrites: Promise<void> = Promise.resolve();

function cleanList(list: unknown, max: number): string[] {
  const out: string[] = [];
  for (const value of Array.isArray(list) ? list : []) {
    const text = String(value || "").trim();
    if (!text || out.includes(text)) continue;
    out.push(text);
    if (out.length >= max) break;
  }
  return out;
}

function localList(key: string, max: number): string[] {
  try {
    return cleanList(JSON.parse(localStorage.getItem(key) || "[]"), max);
  } catch {
    return [];
  }
}

function saveLocalList(key: string, list: unknown, max: number) {
  try {
    localStorage.setItem(key, JSON.stringify(cleanList(list, max)));
  } catch {
    // ignore private-mode quota errors
  }
}

function normalizeUiState(ui: Partial<UiState> = {}): UiState {
  const analysisProfiles = normalizeAnalysisProfiles(ui.analysis_profiles);
  const active = String(ui.active_analysis_profile || "").trim();
  const syntax = normalizeSyntaxPreferences(ui);
  return {
    recent_files: cleanList(ui.recent_files, RECENT_MAX),
    search_history: cleanList(ui.search_history, 50),
    replace_history: cleanList(ui.replace_history, 50),
    session: {
      paths: cleanList(ui.session?.paths, 64),
      active_path: String(ui.session?.active_path || "").trim() || null,
    },
    analysis_profiles: analysisProfiles,
    active_analysis_profile: analysisProfiles.some((profile) => profile.id === active)
      ? active
      : null,
    syntax_configured: syntax.configured,
    syntax_favorites: syntax.favorites,
    syntax_mappings: syntax.mappings,
    syntax_overrides: Object.entries(syntax.overrides).map(([path, scheme]) => ({ path, scheme })),
  };
}

function localSyntaxPreferences(): SyntaxPreferences {
  try {
    const raw = JSON.parse(localStorage.getItem(SYNTAX_PREFERENCES_KEY) || "null");
    if (!raw || typeof raw !== "object" || Array.isArray(raw)) return defaultSyntaxPreferences();
    const normalized = normalizeSyntaxPreferences(raw);
    if (!normalized.configured) return defaultSyntaxPreferences();
    return {
      configured: normalized.configured,
      favorites: normalized.favorites,
      mappings: normalized.mappings,
      overrides: normalized.overrides,
    };
  } catch {
    return defaultSyntaxPreferences();
  }
}

function syntaxPreferencesFromUiState(ui: UiState): SyntaxPreferences {
  const normalized = normalizeSyntaxPreferences(ui);
  return {
    configured: normalized.configured,
    favorites: normalized.favorites,
    mappings: normalized.mappings,
    overrides: normalized.overrides,
  };
}

export async function hydrateSharedUiState() {
  try {
    const remote = normalizeUiState(await api<UiState>("/api/ui_state"));
    const localRecent = localList(RECENT_KEY, RECENT_MAX);
    const localHistory = localList(SEARCH_HISTORY_KEY, 50);
    const localReplaceHistory = localList(REPLACE_HISTORY_KEY, 50);
    const localSyntax = localSyntaxPreferences();
    let localProfiles: AnalysisProfile[] = [];
    try {
      localProfiles = normalizeAnalysisProfiles(
        JSON.parse(localStorage.getItem(ANALYSIS_PROFILES_KEY) || "[]"),
      );
    } catch {
      // ignore malformed local fallback
    }
    const remoteSyntax = syntaxPreferencesFromUiState(remote);
    const syntax = remoteSyntax.configured
      ? remoteSyntax
      : localSyntax.configured
        ? localSyntax
        : defaultSyntaxPreferences();
    const merged = normalizeUiState({
      ...remote,
      recent_files: remote.recent_files.length ? remote.recent_files : localRecent,
      search_history: remote.search_history.length ? remote.search_history : localHistory,
      replace_history: remote.replace_history.length ? remote.replace_history : localReplaceHistory,
      analysis_profiles: remote.analysis_profiles.length ? remote.analysis_profiles : localProfiles,
      syntax_configured: syntax.configured,
      syntax_favorites: syntax.favorites,
      syntax_mappings: syntax.mappings,
      syntax_overrides: Object.entries(syntax.overrides).map(([path, scheme]) => ({
        path,
        scheme,
      })),
    });
    sharedUiState = merged;
    state.search.history = merged.search_history;
    state.search.replaceHistory = merged.replace_history;
    state.analysis.profiles = merged.analysis_profiles;
    state.analysis.activeProfile = merged.active_analysis_profile;
    state.syntax = syntaxPreferencesFromUiState(merged);
    if (
      merged.recent_files.length !== remote.recent_files.length ||
      merged.search_history.length !== remote.search_history.length ||
      merged.replace_history.length !== remote.replace_history.length ||
      merged.analysis_profiles.length !== remote.analysis_profiles.length ||
      merged.syntax_configured !== remote.syntax_configured ||
      merged.syntax_favorites.length !== remote.syntax_favorites.length ||
      merged.syntax_mappings.length !== remote.syntax_mappings.length ||
      merged.syntax_overrides.length !== remote.syntax_overrides.length
    ) {
      await saveSharedUiState(merged);
    }
  } catch {
    sharedUiState = null;
    state.syntax = localSyntaxPreferences();
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

function queueSharedUiStateWrite(update: (base: UiState) => UiState) {
  // Recent files, search history, and analysis profiles can change in the same
  // event turn. Serialize their read-modify-writes so each update sees the
  // snapshot produced by the previous one instead of restoring stale fields.
  sharedUiStateWrites = sharedUiStateWrites.then(async () => {
    const base = await currentUiStateBase();
    if (!base) return;
    await saveSharedUiState(update(base));
  });
}

export function saveRecentFilesShared(list) {
  const recent = cleanList(list, RECENT_MAX);
  saveLocalList(RECENT_KEY, recent, RECENT_MAX);
  queueSharedUiStateWrite((base) => normalizeUiState({ ...base, recent_files: recent }));
}

export function loadSearchHistoryShared() {
  return sharedUiState?.search_history || localList(SEARCH_HISTORY_KEY, 50);
}

export function saveSearchHistoryShared(list) {
  const history = cleanList(list, 50);
  saveLocalList(SEARCH_HISTORY_KEY, history, 50);
  queueSharedUiStateWrite((base) => normalizeUiState({ ...base, search_history: history }));
}

export function loadReplaceHistoryShared() {
  return sharedUiState?.replace_history || localList(REPLACE_HISTORY_KEY, 50);
}

export function saveReplaceHistoryShared(list) {
  const history = cleanList(list, 50);
  saveLocalList(REPLACE_HISTORY_KEY, history, 50);
  void currentUiStateBase().then((base) => {
    if (!base) return; // can't read current state — don't clobber the server
    void saveSharedUiState(normalizeUiState({ ...base, replace_history: history }));
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
    return { profiles, active: state.analysis.activeProfile };
  } catch {
    return { profiles: [], active: null };
  }
}

export function saveAnalysisProfilesShared(profiles, active) {
  const clean = normalizeAnalysisProfiles(profiles);
  const activeId = clean.some((profile) => profile.id === active) ? active : null;
  state.analysis.profiles = clean;
  state.analysis.activeProfile = activeId;
  try {
    localStorage.setItem(ANALYSIS_PROFILES_KEY, JSON.stringify(clean));
  } catch {
    // ignore private-mode quota errors
  }
  queueSharedUiStateWrite((base) =>
    normalizeUiState({
      ...base,
      analysis_profiles: clean,
      active_analysis_profile: activeId,
    }),
  );
}

export function saveSyntaxPreferencesShared(preferences: SyntaxPreferences) {
  const clean = normalizeSyntaxPreferences({ ...preferences, configured: true });
  state.syntax = {
    configured: true,
    favorites: clean.favorites,
    mappings: clean.mappings,
    overrides: clean.overrides,
  };
  try {
    localStorage.setItem(
      SYNTAX_PREFERENCES_KEY,
      JSON.stringify({ configured: true, ...syntaxPreferenceJson(state.syntax) }),
    );
  } catch {
    // localStorage is only the browser fallback; the server write still runs.
  }
  queueSharedUiStateWrite((base) =>
    normalizeUiState({
      ...base,
      syntax_configured: true,
      syntax_favorites: state.syntax.favorites,
      syntax_mappings: state.syntax.mappings,
      syntax_overrides: Object.entries(state.syntax.overrides).map(([path, scheme]) => ({
        path,
        scheme,
      })),
    }),
  );
}

export function migrateSyntaxOverrideShared(oldPath: string, newPath: string) {
  if (!oldPath || !newPath || oldPath === newPath || !state.syntax.overrides[oldPath]) return false;
  saveSyntaxPreferencesShared({
    ...state.syntax,
    overrides: moveSyntaxOverride(state.syntax.overrides, oldPath, newPath),
  });
  return true;
}

export async function restoreSessionSnapshot(): Promise<StatResponse> {
  return apiPost<StatResponse>("/api/session/restore", {});
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
