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

import { canDragOutToNewWindow } from "../src/workspace.js";

describe("tab drag-out to a new window (#35)", () => {
  it("allows a clean, saved on-disk file to spawn its own window", () => {
    expect(canDragOutToNewWindow({ path: "/home/u/app.log", dirty: false })).toBe(true);
    expect(canDragOutToNewWindow({ path: "C:\\logs\\app.txt", dirty: false })).toBe(true);
  });

  it("refuses a dirty tab (unsaved edits can't be handed to another process)", () => {
    expect(canDragOutToNewWindow({ path: "/home/u/app.log", dirty: true })).toBe(false);
  });

  it("refuses a fileless tab (no path to reopen)", () => {
    expect(canDragOutToNewWindow({ path: "", dirty: false })).toBe(false);
    expect(canDragOutToNewWindow({ dirty: false })).toBe(false);
  });

  it("refuses an untitled scratch buffer", () => {
    expect(
      canDragOutToNewWindow({
        path: "/tmp/ayame-srv-untitled-abc123-0-0/untitled.txt",
        dirty: false,
      }),
    ).toBe(false);
  });

  it("refuses a missing tab", () => {
    expect(canDragOutToNewWindow(null)).toBe(false);
    expect(canDragOutToNewWindow(undefined)).toBe(false);
  });
});
