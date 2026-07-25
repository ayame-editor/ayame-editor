import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../src/editor.js", () => ({
  focusEditor: vi.fn(),
  render: vi.fn(),
  revealCaret: vi.fn(),
  scheduleRender: vi.fn(),
  setCaret: vi.fn((line: number, col: number) => {
    state.caret = { line, col };
  }),
}));
vi.mock("../src/edits.js", () => ({
  lineLensFor: vi.fn(async () => new Map()),
  reloadViewport: vi.fn(async () => {}),
  settleEditQueue: vi.fn(async () => {}),
}));
vi.mock("../src/i18n.js", () => ({
  currentLocale: () => "en-US",
  t: (key: string, params?: { msg?: string }) => (params?.msg ? `${key}: ${params.msg}` : key),
}));
vi.mock("../src/popup-menu.js", () => ({ showPopupMenu: vi.fn() }));
vi.mock("../src/search.js", () => ({
  qs: () => "q=error&regex=false&ci=false&word=false",
}));
vi.mock("../src/notifications.js", () => ({ flashCount: vi.fn() }));
vi.mock("../src/dialogs.js", () => ({
  askConfirm: vi.fn(async () => true),
  hideLoading: vi.fn(),
  showLoading: vi.fn(),
}));

import {
  bookmarkSearchMatches,
  nextBookmark,
  selectBookmarkedLines,
  toggleBookmark,
} from "../src/bookmarks.js";
import { setCaret } from "../src/editor.js";
import { settleEditQueue } from "../src/edits.js";
import { flashCount } from "../src/notifications.js";
import { DEFAULT_KEYMAP, MAX_BOOKMARK_SELECTIONS, state } from "../src/state.js";

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

describe("sparse bookmark commands (#241)", () => {
  beforeEach(() => {
    state.stat = { open: true };
    state.total = 10_000_000_000;
    state.caret = { line: 10, col: 4 };
    state.bookmarks = new Set();
    state.bookmarkCount = 0;
    state.query = "error";
    state.regex = false;
    state.ci = false;
    state.word = false;
    state.regexError = false;
    state.extraCursors = [];
    vi.clearAllMocks();
    vi.stubGlobal("fetch", vi.fn());
  });

  it("uses the conventional F2 key family", () => {
    expect(DEFAULT_KEYMAP.toggleBookmark).toBe("Ctrl+F2");
    expect(DEFAULT_KEYMAP.nextBookmark).toBe("F2");
    expect(DEFAULT_KEYMAP.previousBookmark).toBe("Shift+F2");
    expect(DEFAULT_KEYMAP.showBookmarks).toBe("Ctrl+Shift+F2");
  });

  it("toggles one line without allocating by document size", async () => {
    vi.mocked(fetch).mockResolvedValue(
      jsonResponse({
        kind: "bookmark",
        line: 9_999_999_999,
        marked: true,
        count: 1,
        limit: 1_000_000,
      }),
    );

    await toggleBookmark(9_999_999_999);

    expect(fetch).toHaveBeenCalledWith(
      "/api/markers/toggle",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({ kind: "bookmark", line: 9_999_999_999 }),
      }),
    );
    expect(state.bookmarks).toEqual(new Set([9_999_999_999]));
    expect(state.bookmarkCount).toBe(1);
    expect(settleEditQueue).toHaveBeenCalledOnce();
  });

  it("delegates wrapped next navigation to the ordered server query", async () => {
    vi.mocked(fetch).mockResolvedValue(
      jsonResponse({
        kind: "bookmark",
        line: 2,
        count: 3,
        wrapped: true,
      }),
    );

    await nextBookmark();

    expect(fetch).toHaveBeenCalledWith(
      "/api/markers/navigate?kind=bookmark&from=10&direction=next&wrap=true",
      undefined,
    );
    expect(setCaret).toHaveBeenCalledWith(2, 0, 0);
    expect(state.bookmarkCount).toBe(3);
  });

  it("refuses a document-sized multi-selection above the safety cap", async () => {
    vi.mocked(fetch).mockResolvedValue(
      jsonResponse({
        kind: "bookmark",
        total: MAX_BOOKMARK_SELECTIONS + 1,
        lines: Array.from({ length: MAX_BOOKMARK_SELECTIONS + 1 }, (_, line) => line),
        truncated: false,
      }),
    );

    await selectBookmarkedLines();

    expect(state.extraCursors).toEqual([]);
    expect(flashCount).toHaveBeenCalledWith("bookmark.selectLimit", "error");
  });

  it("deduplicates search hits by line before bulk registration", async () => {
    vi.mocked(fetch)
      .mockResolvedValueOnce(
        jsonResponse({
          hits: [
            { byte: 10, byte_len: 5, line: 2, column: 0 },
            { byte: 20, byte_len: 5, line: 2, column: 10 },
            { byte: 30, byte_len: 5, line: 8, column: 0 },
          ],
          truncated: false,
        }),
      )
      .mockResolvedValueOnce(
        jsonResponse({
          kind: "bookmark",
          added: 2,
          count: 2,
          limit: 1_000_000,
          limit_reached: false,
        }),
      )
      .mockResolvedValueOnce(
        jsonResponse({
          lines: [],
          markers: [],
          total: 10_000_000_000,
        }),
      );

    await bookmarkSearchMatches();

    expect(fetch).toHaveBeenNthCalledWith(
      2,
      "/api/markers/add",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({ kind: "bookmark", lines: [2, 8] }),
      }),
    );
    expect(state.bookmarkCount).toBe(2);
    expect(flashCount).toHaveBeenCalledWith("bookmark.matchesAdded");
  });
});
