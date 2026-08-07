import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../src/api.js", () => ({ api: vi.fn(), apiPost: vi.fn() }));

import { api, apiPost } from "../src/api.js";
import {
  beaconSessionSnapshot,
  hydrateSharedUiState,
  loadAnalysisProfilesShared,
  loadRecentFilesShared,
  loadSearchHistoryShared,
  restoreSessionSnapshot,
  saveRecentFilesShared,
  saveSearchHistoryShared,
  saveSessionSnapshot,
} from "../src/persistence.js";
import {
  ANALYSIS_PROFILES_KEY,
  DEFAULT_SETTINGS,
  RECENT_KEY,
  SEARCH_HISTORY_KEY,
  state,
} from "../src/state.js";

function memoryStorage(initial: Record<string, string> = {}): Storage {
  const values = new Map(Object.entries(initial));
  return {
    get length() {
      return values.size;
    },
    clear: () => values.clear(),
    getItem: (key) => values.get(key) ?? null,
    key: (index) => [...values.keys()][index] ?? null,
    removeItem: (key) => values.delete(key),
    setItem: (key, value) => values.set(key, String(value)),
  };
}

const emptyUiState = () => ({
  recent_files: [],
  search_history: [],
  session: { paths: [], active_path: null },
  analysis_profiles: [],
  active_analysis_profile: null,
});

describe("shared UI persistence (#188)", () => {
  beforeEach(async () => {
    vi.clearAllMocks();
    vi.stubGlobal("localStorage", memoryStorage());
    state.settings = { ...DEFAULT_SETTINGS, keymap: {}, customThemes: {} };
    state.search.history = [];
    state.analysis.profiles = [];
    state.analysis.activeProfile = null;

    // Each test starts without a hydrated in-memory snapshot.
    vi.mocked(api).mockRejectedValue(new Error("offline"));
    await hydrateSharedUiState();
    vi.clearAllMocks();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("merges normalized local fallbacks into an empty server snapshot", async () => {
    localStorage.setItem(RECENT_KEY, JSON.stringify([" /a.txt ", "/a.txt", "", "/b.txt"]));
    localStorage.setItem(SEARCH_HISTORY_KEY, JSON.stringify([" error ", "error", "warn"]));
    const remote = {
      ...emptyUiState(),
      session: { paths: ["/session.txt"], active_path: "/session.txt" },
    };
    vi.mocked(api).mockResolvedValue(remote);
    vi.mocked(apiPost).mockImplementation(async (_path, body) => body);

    await hydrateSharedUiState();

    expect(loadRecentFilesShared()).toEqual(["/a.txt", "/b.txt"]);
    expect(loadSearchHistoryShared()).toEqual(["error", "warn"]);
    expect(state.search.history).toEqual(["error", "warn"]);
    expect(apiPost).toHaveBeenCalledWith(
      "/api/ui_state",
      expect.objectContaining({
        recent_files: ["/a.txt", "/b.txt"],
        search_history: ["error", "warn"],
        session: remote.session,
      }),
    );
  });

  it("round-trips partial list updates without erasing the server session", async () => {
    const remote = {
      ...emptyUiState(),
      recent_files: ["/old.txt"],
      search_history: ["old query"],
      session: { paths: ["/keep.txt"], active_path: "/keep.txt" },
    };
    vi.mocked(api).mockResolvedValue(remote);
    vi.mocked(apiPost).mockImplementation(async (_path, body) => body);
    await hydrateSharedUiState();
    vi.clearAllMocks();

    saveRecentFilesShared([" /new.txt ", "/new.txt", "/two.txt"]);
    saveSearchHistoryShared([" needle ", "needle", "second"]);

    expect(JSON.parse(localStorage.getItem(RECENT_KEY)!)).toEqual(["/new.txt", "/two.txt"]);
    expect(JSON.parse(localStorage.getItem(SEARCH_HISTORY_KEY)!)).toEqual(["needle", "second"]);
    await vi.waitFor(() => expect(apiPost).toHaveBeenCalledTimes(2));
    for (const [, body] of vi.mocked(apiPost).mock.calls) {
      expect(body).toMatchObject({ session: remote.session });
    }
    expect(loadRecentFilesShared()).toEqual(["/new.txt", "/two.txt"]);
    expect(loadSearchHistoryShared()).toEqual(["needle", "second"]);
  });

  it("falls back safely when local JSON is malformed and the server is unavailable", async () => {
    localStorage.setItem(RECENT_KEY, "{");
    localStorage.setItem(SEARCH_HISTORY_KEY, "not-json");
    localStorage.setItem(ANALYSIS_PROFILES_KEY, "[");
    vi.mocked(api).mockRejectedValue(new Error("offline"));

    await hydrateSharedUiState();

    expect(loadRecentFilesShared()).toEqual([]);
    expect(loadSearchHistoryShared()).toEqual([]);
    expect(loadAnalysisProfilesShared()).toEqual({ profiles: [], active: null });
  });

  it("keeps the local write but skips a destructive partial server write when re-read fails", async () => {
    vi.mocked(api).mockRejectedValue(new Error("offline"));

    saveRecentFilesShared([" /offline.txt "]);

    expect(JSON.parse(localStorage.getItem(RECENT_KEY)!)).toEqual(["/offline.txt"]);
    await vi.waitFor(() => expect(api).toHaveBeenCalledWith("/api/ui_state"));
    expect(apiPost).not.toHaveBeenCalled();
  });

  it("uses the session endpoints and respects the restore-session preference", async () => {
    vi.mocked(apiPost).mockResolvedValueOnce({ open: true, path: "/restored.txt" });
    await expect(restoreSessionSnapshot()).resolves.toEqual({
      open: true,
      path: "/restored.txt",
    });
    expect(apiPost).toHaveBeenCalledWith("/api/session/restore", {});

    vi.clearAllMocks();
    state.settings.restoreSession = false;
    await saveSessionSnapshot();
    expect(apiPost).not.toHaveBeenCalled();

    state.settings.restoreSession = true;
    vi.mocked(apiPost).mockResolvedValue(emptyUiState());
    await saveSessionSnapshot();
    expect(apiPost).toHaveBeenCalledWith("/api/session/save", {});

    vi.mocked(apiPost).mockRejectedValueOnce(new Error("closing"));
    await expect(saveSessionSnapshot()).resolves.toBeUndefined();
  });

  it("queues a beacon only when session restore is enabled", () => {
    const sendBeacon = vi.fn(() => true);
    vi.stubGlobal("navigator", { sendBeacon });

    state.settings.restoreSession = false;
    expect(beaconSessionSnapshot()).toBe(false);
    expect(sendBeacon).not.toHaveBeenCalled();

    state.settings.restoreSession = true;
    expect(beaconSessionSnapshot()).toBe(true);
    expect(sendBeacon).toHaveBeenCalledOnce();
    expect(sendBeacon.mock.calls[0][0]).toBe("/api/session/save");
    expect(sendBeacon.mock.calls[0][1]).toBeInstanceOf(Blob);
  });
});
