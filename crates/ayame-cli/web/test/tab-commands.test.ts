// Bulk tab closing and reopen-closed-tab (#175). The selection rules are pure
// over the tab list, so they are checked directly; the closed-tab stack is
// checked through the real close path against a stubbed server.
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../src/edits.js", () => ({
  settleEditQueue: vi.fn(async () => {}),
}));
vi.mock("../src/dialogs.js", () => ({ askConfirm: vi.fn(async () => true) }));
vi.mock("../src/notifications.js", () => ({ flashCount: vi.fn() }));
vi.mock("../src/popup-menu.js", () => ({ showPopupMenu: vi.fn() }));
// Assert on message keys rather than one locale's wording.
vi.mock("../src/i18n.js", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../src/i18n.js")>()),
  t: (key: string) => key,
}));

import {
  CLOSED_TAB_HISTORY_MAX,
  closeTab,
  closedTabHistory,
  hasClosedTabs,
  rememberClosedTab,
  reopenClosedTab,
  savedTabs,
  setTabsWorkspaceService,
  tabCloseMenuItems,
  tabsOtherThan,
  tabsToRightOf,
} from "../src/tabs.js";
import { state } from "../src/state.js";

function tabs(...specs: Array<[number, string, boolean?]>) {
  return specs.map(([id, name, dirty]) => ({
    id,
    name,
    path: `/w/${name}`,
    dirty: !!dirty,
    active: false,
  }));
}

describe("which tabs a bulk close covers", () => {
  const list = tabs([1, "a"], [2, "b"], [3, "c", true], [4, "d"]);

  // Closed far-end first so the ids stay valid as the list shrinks, and the
  // tab the menu was opened on stays put.
  it("takes everything right of the anchor, rightmost first", () => {
    expect(tabsToRightOf(list, 2)).toEqual([4, 3]);
    expect(tabsToRightOf(list, 4)).toEqual([]);
    expect(tabsToRightOf(list, 99)).toEqual([]);
  });

  it("takes everything but the kept tab", () => {
    expect(tabsOtherThan(list, 2)).toEqual([1, 3, 4]);
    expect(tabsOtherThan(list)).toEqual([1, 2, 3, 4]);
  });

  // "Close saved" is the tidy-up that must never raise a discard prompt, so a
  // dirty tab is simply not in the set.
  it("leaves unsaved tabs out of the saved set", () => {
    expect(savedTabs(list)).toEqual([1, 2, 4]);
  });

  it("survives an empty workspace", () => {
    expect(tabsToRightOf([], 1)).toEqual([]);
    expect(tabsOtherThan(null)).toEqual([]);
    expect(savedTabs(undefined)).toEqual([]);
  });
});

describe("the tab context menu's close commands", () => {
  function labels(items) {
    return items.filter((item) => !item.separator).map((item) => [item.label, !!item.disabled]);
  }

  it("offers every close variant plus reopen", () => {
    const list = tabs([1, "a"], [2, "b"], [3, "c", true]);
    expect(labels(tabCloseMenuItems(list[0], list)).map(([label]) => label)).toEqual([
      "tab.closeOthers",
      "tab.closeToRight",
      "tab.closeSaved",
      "tab.closeAll",
      "tab.reopenClosed",
    ]);
  });

  // A menu entry that would close nothing says so rather than looking live.
  it("disables what would be a no-op", () => {
    const only = tabs([1, "a", true]);
    const items = Object.fromEntries(labels(tabCloseMenuItems(only[0], only)));
    expect(items["tab.closeOthers"]).toBe(true);
    expect(items["tab.closeToRight"]).toBe(true);
    expect(items["tab.closeSaved"]).toBe(true); // the only tab is dirty
    expect(items["tab.closeAll"]).toBe(false);
  });
});

describe("reopening a closed tab", () => {
  const opened: string[] = [];

  beforeEach(async () => {
    opened.length = 0;
    setTabsWorkspaceService({
      onDocumentOpened: () => {},
      newUntitled: async () => {},
      openPath: async (path: string) => {
        opened.push(path);
        return true;
      },
    });
    // Drain whatever a previous test left on the stack.
    while (hasClosedTabs()) await reopenClosedTab();
    opened.length = 0;
  });

  it("hands back the most recently closed path first", async () => {
    rememberClosedTab({ path: "/w/one.txt" });
    rememberClosedTab({ path: "/w/two.txt" });

    await reopenClosedTab();
    await reopenClosedTab();

    expect(opened).toEqual(["/w/two.txt", "/w/one.txt"]);
    expect(hasClosedTabs()).toBe(false);
  });

  it("does nothing when there is nothing to reopen", async () => {
    await expect(reopenClosedTab()).resolves.toBe(false);
    expect(opened).toEqual([]);
  });

  // An untitled buffer's file is this session's own scratch; "reopening" that
  // path would resurrect a file rather than a buffer.
  it("does not record untitled buffers", () => {
    rememberClosedTab({ path: "/tmp/ayame-srv-untitled-1234/untitled.txt" });
    rememberClosedTab({ path: "" });
    expect(hasClosedTabs()).toBe(false);
  });

  it("keeps one entry per path, newest first", () => {
    rememberClosedTab({ path: "/w/a.txt" });
    rememberClosedTab({ path: "/w/b.txt" });
    rememberClosedTab({ path: "/w/a.txt" });
    expect(closedTabHistory()).toEqual(["/w/a.txt", "/w/b.txt"]);
  });

  it("keeps the stack bounded", () => {
    for (let i = 0; i < CLOSED_TAB_HISTORY_MAX + 5; i++) rememberClosedTab({ path: `/w/${i}.txt` });
    expect(closedTabHistory()).toHaveLength(CLOSED_TAB_HISTORY_MAX);
    expect(closedTabHistory()[0]).toBe(`/w/${CLOSED_TAB_HISTORY_MAX + 4}.txt`);
  });

  // Only a close the server accepted may leave an entry behind; otherwise
  // Ctrl+Shift+T would offer a tab that is still open.
  it("records a path only once the close actually landed", async () => {
    state.doc.tabs = tabs([1, "a.txt"], [2, "b.txt"]) as never;
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response(JSON.stringify({ open: true }), { status: 200 })),
    );
    await closeTab(1);
    expect(closedTabHistory()).toEqual(["/w/a.txt"]);

    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response("nope", { status: 500 })),
    );
    await closeTab(2);
    expect(closedTabHistory()).toEqual(["/w/a.txt"]);
  });
});
