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
  setSaveWorkspaceService: vi.fn(),
  savingCount: 0,
}));
vi.mock("../src/menu-surface.js", () => ({
  fileMenuVisible: vi.fn(() => false),
  hideFileMenu: vi.fn(),
}));
vi.mock("../src/popup-menu.js", () => ({
  showPopupMenu: vi.fn(),
}));
vi.mock("../src/status.js", () => ({
  updateStatusMeta: vi.fn(),
}));
vi.mock("../src/edits.js", () => ({
  setFollowTail: vi.fn(),
  settleEditQueue: vi.fn(),
}));
vi.mock("../src/notifications.js", () => ({ flashCount: vi.fn() }));
vi.mock("../src/app.js", () => ({
  confirmCloseLastTab: vi.fn(),
  isNativeApp: vi.fn(() => true),
  nativeOpenDialog: vi.fn(),
  nativeSaveDialog: vi.fn(),
  openNewWindow: vi.fn(),
  requestEditorClose: vi.fn(),
}));

import { isNativeApp } from "../src/app.js";
import { showPopupMenu } from "../src/popup-menu.js";
import { state } from "../src/state.js";
import {
  browseRow,
  canDragOutToNewWindow,
  canHandoffDirtyTab,
  closeTabsSequentially,
  finishTabDrag,
  moveOpenerSelection,
  onOpenerInputKeydown,
  onOpenerListKeydown,
  recentRow,
  renderTabs,
  resetOpenerSelection,
  showTabList,
  startTabDrag,
  tabDropBeforeId,
  tabOrderAfterMove,
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

describe("same-window tab ordering and overflow access (#166)", () => {
  const tabs = [
    { id: 1, name: "one.txt", path: "/tmp/one.txt", active: false, dirty: false },
    { id: 2, name: "two.txt", path: "/tmp/two.txt", active: true, dirty: true },
    { id: 3, name: "three.txt", path: "/tmp/three.txt", active: false, dirty: false },
  ];

  it("computes stable before-ids and reordered snapshots without mutating the source", () => {
    expect(tabDropBeforeId(tabs, 1, 3, true)).toBeNull();
    expect(tabDropBeforeId(tabs, 3, 1, false)).toBe(1);
    expect(tabDropBeforeId(tabs, 2, 2, false)).toBe(3);
    expect(tabOrderAfterMove(tabs, 3, 1)?.map((tab) => tab.id)).toEqual([3, 1, 2]);
    expect(tabOrderAfterMove(tabs, 1, null)?.map((tab) => tab.id)).toEqual([2, 3, 1]);
    expect(tabOrderAfterMove(tabs, 99, 1)).toBeNull();
    expect(tabs.map((tab) => tab.id)).toEqual([1, 2, 3]);
  });

  it("starts a local drag for a dirty tab while retaining cross-window guards", async () => {
    const dragged = document.createElement("div");
    const setData = vi.fn();
    const preventDefault = vi.fn();
    const dataTransfer = { effectAllowed: "none", dropEffect: "none", setData };
    const event = {
      currentTarget: dragged,
      dataTransfer,
      preventDefault,
      target: dragged,
    };

    startTabDrag(event, tabs[1]);
    expect(preventDefault).not.toHaveBeenCalled();
    expect(dataTransfer.effectAllowed).toBe("move");
    expect(setData).toHaveBeenCalledTimes(2);
    expect(dragged.classList.contains("dragging")).toBe(true);

    await finishTabDrag({ clientX: 1, clientY: 1, currentTarget: dragged, dataTransfer }, tabs[1]);
    expect(dragged.classList.contains("dragging")).toBe(false);
  });

  it("keeps the active tab visible and exposes every tab through the list button", () => {
    const originalTabs = state.tabs;
    const originalScroll = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "scrollIntoView");
    const scrollIntoView = vi.fn();
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
      configurable: true,
      value: scrollIntoView,
    });
    document.body.innerHTML = '<div id="tabs"></div><button id="tab-list"></button>';

    try {
      renderTabs(tabs);
      expect(scrollIntoView).toHaveBeenCalledWith({ block: "nearest", inline: "nearest" });
      expect((document.getElementById("tab-list") as HTMLButtonElement).disabled).toBe(false);

      showTabList();
      const items = vi.mocked(showPopupMenu).mock.calls.at(-1)?.[2] || [];
      expect(items.map((item) => item.label)).toEqual([
        "one.txt — /tmp",
        "two.txt — /tmp",
        "three.txt — /tmp",
      ]);
      expect(items.map((item) => !!item.checked)).toEqual([false, true, false]);
    } finally {
      state.tabs = originalTabs;
      document.body.textContent = "";
      if (originalScroll) {
        Object.defineProperty(HTMLElement.prototype, "scrollIntoView", originalScroll);
      } else {
        delete (HTMLElement.prototype as any).scrollIntoView;
      }
    }
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

describe("opener keyboard navigation (#185)", () => {
  it("moves from the path input through both option lists and activates with Enter", () => {
    const originalMode = state.openerMode;
    document.body.innerHTML = `
      <input id="opener-input" />
      <div id="opener-recent" role="listbox" tabindex="0"></div>
      <div id="opener-list" role="listbox" tabindex="0"></div>
    `;
    const recent = recentRow("/tmp/recent.log");
    document.getElementById("opener-recent")?.append(recent);

    const file = browseRow(
      { name: "app.log", path: "/tmp/app.log", is_dir: false, size: 12 },
      false,
    );
    const activate = vi.fn();
    file.addEventListener("click", activate);
    document.getElementById("opener-list")?.append(file);
    state.openerMode = "folder"; // the row's normal file click is intentionally inert in this mode

    try {
      resetOpenerSelection();
      const fromInput = { key: "ArrowDown", preventDefault: vi.fn() };
      onOpenerInputKeydown(fromInput);
      expect(fromInput.preventDefault).toHaveBeenCalled();
      expect(recent.getAttribute("aria-selected")).toBe("true");
      expect(document.activeElement?.id).toBe("opener-recent");
      expect(document.getElementById("opener-recent")?.getAttribute("aria-activedescendant")).toBe(
        recent.id,
      );

      moveOpenerSelection(1);
      expect(file.getAttribute("aria-selected")).toBe("true");
      expect(document.activeElement?.id).toBe("opener-list");
      expect(document.getElementById("opener-list")?.getAttribute("aria-activedescendant")).toBe(
        file.id,
      );
      expect(document.getElementById("opener-recent")?.hasAttribute("aria-activedescendant")).toBe(
        false,
      );

      const enter = { key: "Enter", preventDefault: vi.fn() };
      onOpenerListKeydown(enter);
      expect(enter.preventDefault).toHaveBeenCalled();
      expect(activate).toHaveBeenCalledOnce();
    } finally {
      state.openerMode = originalMode;
      document.body.textContent = "";
      resetOpenerSelection();
    }
  });

  it("closes an ordinary opener with Escape", () => {
    const originalStat = state.stat;
    document.body.innerHTML = `
      <div id="opener" class="modal" aria-hidden="false"></div>
      <div id="opener-recent" role="listbox"></div>
      <div id="opener-list" role="listbox"></div>
    `;
    state.stat = { open: true };
    const escape = { key: "Escape", preventDefault: vi.fn() };
    try {
      onOpenerListKeydown(escape);
      expect(escape.preventDefault).toHaveBeenCalled();
      expect(document.getElementById("opener")?.classList.contains("hidden")).toBe(true);
      expect(document.getElementById("opener")?.getAttribute("aria-hidden")).toBe("true");
    } finally {
      state.stat = originalStat;
      document.body.textContent = "";
    }
  });
});
