import { describe, expect, it, vi } from "vitest";

// workspace.js pulls in the whole editor module graph; mock the side-effecting
// modules so importing the pure drag-out predicate stays cheap. (Same approach
// as edits.test.ts.) dom.js is left real so the untitled-path check is genuine.
vi.mock("../src/editor.js", () => ({
  clearLineCache: vi.fn(),
  focusEditor: vi.fn(),
  render: vi.fn(),
  scheduleRender: vi.fn(),
  setCaret: vi.fn(),
}));
vi.mock("../src/save.js", () => ({
  expectWalHandoff: vi.fn(),
  maybeOfferWalRecovery: vi.fn(),
  noteWalError: vi.fn(),
  savingCount: 0,
}));
vi.mock("../src/menus.js", () => ({
  fileMenuVisible: vi.fn(() => false),
  hideFileMenu: vi.fn(),
  initMenuBar: vi.fn(),
  updateStatusMeta: vi.fn(),
}));
vi.mock("../src/edits.js", () => ({
  setFollowTail: vi.fn(),
  settleEditQueue: vi.fn(),
}));
vi.mock("../src/search.js", () => ({
  flashCount: vi.fn(),
}));
vi.mock("../src/app.js", () => ({
  confirmCloseLastTab: vi.fn(),
  isNativeApp: vi.fn(() => true),
  nativeOpenDialog: vi.fn(),
  nativeSaveDialog: vi.fn(),
  openNewWindow: vi.fn(),
  requestEditorClose: vi.fn(),
}));

import { isNativeApp } from "../src/app.js";
import { state } from "../src/state.js";
import {
  canDragOutToNewWindow,
  canHandoffDirtyTab,
  closeTabsSequentially,
  renderPathCrumbs,
} from "../src/workspace.js";

function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

describe("tab drag-out to a new window (#35)", () => {
  it("allows a clean, saved on-disk file to spawn its own window", () => {
    expect(canDragOutToNewWindow({ path: "/home/u/app.log", dirty: false })).toBe(true);
    expect(canDragOutToNewWindow({ path: "C:\\logs\\app.txt", dirty: false })).toBe(true);
  });

  it("allows a dirty on-disk tab in the native build (WAL handoff)", () => {
    expect(canHandoffDirtyTab({ path: "/home/u/app.log", dirty: true })).toBe(true);
    expect(canDragOutToNewWindow({ path: "/home/u/app.log", dirty: true })).toBe(true);
  });

  it("refuses a dirty tab in the browser build (no per-window tab ownership)", () => {
    vi.mocked(isNativeApp).mockReturnValue(false);
    try {
      expect(canHandoffDirtyTab({ path: "/home/u/app.log", dirty: true })).toBe(false);
      expect(canDragOutToNewWindow({ path: "/home/u/app.log", dirty: true })).toBe(false);
      // Clean tabs still tear out in the browser build.
      expect(canDragOutToNewWindow({ path: "/home/u/app.log", dirty: false })).toBe(true);
    } finally {
      vi.mocked(isNativeApp).mockReturnValue(true);
    }
  });

  it("refuses a fileless tab (no path to reopen)", () => {
    expect(canDragOutToNewWindow({ path: "", dirty: false })).toBe(false);
    expect(canDragOutToNewWindow({ dirty: false })).toBe(false);
  });

  it("refuses an untitled scratch buffer, dirty or clean (no stable cross-process key)", () => {
    expect(
      canDragOutToNewWindow({
        path: "/tmp/ayame-srv-untitled-abc123-0-0/untitled.txt",
        dirty: false,
      }),
    ).toBe(false);
    expect(
      canHandoffDirtyTab({
        path: "/tmp/ayame-srv-untitled-abc123-0-0/untitled.txt",
        dirty: true,
      }),
    ).toBe(false);
  });

  it("refuses a missing tab", () => {
    expect(canDragOutToNewWindow(null)).toBe(false);
    expect(canDragOutToNewWindow(undefined)).toBe(false);
  });
});

describe("Close Other Tabs ordering (#123)", () => {
  it("waits for each close before starting the next", async () => {
    const first = deferred();
    const second = deferred();
    const close = vi.fn((id: number) => (id === 1 ? first.promise : second.promise));

    const pending = closeTabsSequentially([1, 2], close);
    expect(close).toHaveBeenCalledTimes(1);
    expect(close).toHaveBeenLastCalledWith(1);

    first.resolve();
    await vi.waitFor(() => expect(close).toHaveBeenCalledTimes(2));
    expect(close).toHaveBeenLastCalledWith(2);

    second.resolve();
    await pending;
  });
});

describe("localized opener breadcrumbs (#189)", () => {
  it("uses the selected language for the Windows computer root", () => {
    const originalLanguage = state.settings.language;
    const host = document.createElement("nav");
    try {
      state.settings.language = "ja";
      renderPathCrumbs(host, "C:\\Users", vi.fn());
      expect(host.querySelector("button")?.textContent).toBe("この PC");

      state.settings.language = "en";
      renderPathCrumbs(host, "C:\\Users", vi.fn());
      expect(host.querySelector("button")?.textContent).toBe("This PC");
    } finally {
      state.settings.language = originalLanguage;
    }
  });
});
