import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  cacheLineResponse,
  coordsFromEvent,
  ensureData,
  fillEofRow,
  fillRow,
  formatLineNo,
  lineNumberChars,
  maxFirst,
  revealCaret,
  renderSearchTicks,
  rowsFullyVisible,
  rowsVisible,
} from "../src/editor.js";
import { LINE_HEIGHT, setLineHeight, state } from "../src/state.js";

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

async function flushPromises() {
  await new Promise((resolve) => setTimeout(resolve, 0));
}

describe("viewport row calculations (#129)", () => {
  beforeEach(() => {
    document.body.innerHTML = '<div id="viewport"></div>';
    setLineHeight(18);
    state.settings.ruler = false;
  });

  function setViewportHeight(height: number) {
    Object.defineProperty(document.getElementById("viewport"), "clientHeight", {
      configurable: true,
      value: height,
    });
  }

  it.each([
    ["exact rows", 3 * LINE_HEIGHT, 3, 3],
    ["partial bottom row", 3 * LINE_HEIGHT + 1, 4, 3],
    ["less than one row", LINE_HEIGHT - 1, 1, 1],
  ])("distinguishes visible and fully visible rows: %s", (_label, height, visible, fully) => {
    setViewportHeight(height as number);
    expect(rowsVisible()).toBe(visible);
    expect(rowsFullyVisible()).toBe(fully);
  });

  it("subtracts the ruler height before calculating rows", () => {
    state.settings.ruler = true;
    setViewportHeight(3 * LINE_HEIGHT + 18);
    expect(rowsVisible()).toBe(3);
    expect(rowsFullyVisible()).toBe(3);
  });

  it("clamps the final first row so the last line and EOF marker are fully visible", () => {
    state.total = 10;
    setViewportHeight(3 * LINE_HEIGHT);
    expect(maxFirst()).toBe(8);

    state.total = 2;
    expect(maxFirst()).toBe(0);
  });

  it("clamps revealCaret against fully visible rows", () => {
    document.body.insertAdjacentHTML("beforeend", '<div id="content"></div>');
    const content = document.getElementById("content")!;
    Object.defineProperty(content, "clientWidth", { configurable: true, value: 200 });
    setViewportHeight(3 * LINE_HEIGHT);
    state.total = 20;
    state.first = 0;
    state.caret = { line: 10, col: 0 };

    revealCaret();
    expect(state.first).toBe(8);

    state.first = 10;
    state.caret = { line: 2, col: 0 };
    revealCaret();
    expect(state.first).toBe(2);
  });

  it("maps pointer rows and clamps them to document bounds", () => {
    document.body.insertAdjacentHTML("beforeend", '<div id="content"></div>');
    const content = document.getElementById("content")!;
    vi.spyOn(content, "getBoundingClientRect").mockReturnValue({
      top: 100,
      left: 50,
      right: 250,
      bottom: 300,
      width: 200,
      height: 200,
      x: 50,
      y: 100,
      toJSON: () => ({}),
    });
    state.first = 10;
    state.total = 20;

    expect(coordsFromEvent({ clientX: 40, clientY: 100 + 2 * LINE_HEIGHT + 1 })).toEqual({
      line: 12,
      col: 0,
    });
    expect(coordsFromEvent({ clientX: 40, clientY: 0 }).line).toBe(10);
    expect(coordsFromEvent({ clientX: 40, clientY: 1000 }).line).toBe(19);
  });
});

describe("line-number formatting (#49)", () => {
  it("groups line numbers with commas by default", () => {
    state.settings.lineNumberCommas = true;
    expect(formatLineNo(1)).toBe("1");
    expect(formatLineNo(1000)).toBe("1,000");
    expect(formatLineNo(17_586_323)).toBe("17,586,323");
  });

  it("shows plain digits when the setting is off", () => {
    state.settings.lineNumberCommas = false;
    expect(formatLineNo(17_586_323)).toBe("17586323");
    state.settings.lineNumberCommas = true;
  });

  it("treats an unset flag as commas-on (the default)", () => {
    delete state.settings.lineNumberCommas;
    expect(formatLineNo(1000)).toBe("1,000");
    state.settings.lineNumberCommas = true;
  });

  it("sizes the gutter to exactly the maximum formatted line number (#252)", () => {
    state.settings.lineNumberCommas = true;
    expect(lineNumberChars(0)).toBe(1);
    expect(lineNumberChars(67)).toBe(2);
    expect(lineNumberChars(999)).toBe(3);
    expect(lineNumberChars(1000)).toBe(5);
    expect(lineNumberChars(10_000_000_000)).toBe(14);

    state.settings.lineNumberCommas = false;
    expect(lineNumberChars(1000)).toBe(4);
    expect(lineNumberChars(10_000_000_000)).toBe(11);
    state.settings.lineNumberCommas = true;
  });
});

describe("sparse bookmark gutter rendering (#241)", () => {
  it("replaces the marker cache together with each viewport page", () => {
    state.bookmarks = new Set([99]);
    cacheLineResponse(40, {
      lines: [{ number: 40, text: "visible" }],
      markers: [{ kind: "bookmark", line: 40 }],
      total: 10_000_000_000,
    });
    expect(state.bookmarks).toEqual(new Set([40]));
    expect(state.cache.start).toBe(40);
    expect(state.total).toBe(10_000_000_000);
  });

  it("marks only bookmarked pooled rows and exposes the gutter as a button", () => {
    const row = document.createElement("div");
    row.append(document.createElement("span"), document.createElement("span"));
    state.bookmarks = new Set([41]);

    fillRow(row, 41, { text: "marked" });
    expect(row.classList.contains("bookmarked")).toBe(true);
    expect(row.firstElementChild?.getAttribute("role")).toBe("button");
    expect(row.firstElementChild?.getAttribute("aria-label")).toContain("42");

    fillRow(row, 42, { text: "plain" });
    expect(row.classList.contains("bookmarked")).toBe(false);
  });
});

describe("sparse change-history rendering (#243)", () => {
  beforeEach(() => {
    state.settings.showChangeHistory = true;
    state.changeSaved = new Set();
    state.changeUnsaved = new Set();
    state.changeDeleted = new Set();
  });

  it("replaces all change partitions with the viewport marker generation", () => {
    state.changeSaved = new Set([99]);
    cacheLineResponse(4, {
      lines: [{ number: 4, text: "changed" }],
      markers: [
        { kind: "change-unsaved", line: 4 },
        { kind: "change-deleted", line: 4 },
        { kind: "change-saved", line: 5 },
      ],
      total: 5,
    });
    expect(state.changeUnsaved).toEqual(new Set([4]));
    expect(state.changeSaved).toEqual(new Set([5]));
    expect(state.changeDeleted).toEqual(new Set([4]));
  });

  it("uses status plus a non-color deletion shape and exposes focus text", () => {
    const row = document.createElement("div");
    row.append(document.createElement("span"), document.createElement("span"));
    state.changeUnsaved = new Set([7]);
    state.changeDeleted = new Set([7]);

    fillRow(row, 7, { text: "next line" });
    expect(row.classList.contains("change-unsaved")).toBe(true);
    expect(row.classList.contains("change-deleted")).toBe(true);
    expect(row.firstElementChild?.getAttribute("tabindex")).toBe("0");
    expect(row.firstElementChild?.getAttribute("aria-label")).toMatch(/未保存|Unsaved/);
  });

  it("renders a deletion at logical EOF and obeys the display toggle", () => {
    const row = document.createElement("div");
    row.append(document.createElement("span"), document.createElement("span"));
    state.total = 3;
    state.changeSaved = new Set([3]);
    state.changeDeleted = new Set([3]);

    fillEofRow(row);
    expect(row.classList.contains("change-saved")).toBe(true);
    expect(row.classList.contains("change-deleted")).toBe(true);
    expect(row.firstElementChild?.getAttribute("role")).toBe("img");

    state.settings.showChangeHistory = false;
    fillEofRow(row);
    expect(row.classList.contains("change-saved")).toBe(false);
    expect(row.firstElementChild?.hasAttribute("aria-label")).toBe(false);
  });

  it("renders bounded saved/unsaved overview bins from the shared marker summary", () => {
    document.body.innerHTML = '<div id="vticks"></div>';
    state.query = "";
    state.searchHits = null;
    state.analysisStatus = null;
    state.changeHistoryOverview = {
      revision: 4,
      total_lines: 10_000_000_000,
      saved: { count: 1, histogram: [0, 1, 0, 0] },
      unsaved: { count: 1, histogram: [0, 0, 0, 1] },
      deleted: { count: 1, histogram: [0, 0, 0, 1] },
      limit_reached: false,
    };

    renderSearchTicks(100);
    expect(document.querySelectorAll(".change-vtick")).toHaveLength(2);
    expect(document.querySelector(".change-unsaved-vtick.change-deleted-vtick")).not.toBeNull();
    expect(document.getElementById("vticks")?.getAttribute("aria-label")).toMatch(/1/);
  });
});

describe("editor load generation", () => {
  beforeEach(() => {
    state.total = 1000;
    state.cache = { start: 0, lines: [] };
    state.loadToken = 0;
    vi.stubGlobal("fetch", vi.fn());
  });

  it("ignores stale line fetches after a newer loadToken supersedes them", async () => {
    const resolves: ((value: Response) => void)[] = [];
    vi.mocked(fetch).mockImplementation(
      () =>
        new Promise<Response>((resolve) => {
          resolves.push(resolve);
        }),
    );

    ensureData(0, 1);
    ensureData(700, 1);

    expect(state.loadToken).toBe(2);
    resolves[0](jsonResponse({ lines: [{ number: 0, text: "stale" }], total: 1000 }));
    await flushPromises();
    expect(state.cache.lines).toEqual([]);

    state.loadToken++;
    resolves[1](jsonResponse({ lines: [{ number: 700, text: "also stale" }], total: 1000 }));
    await flushPromises();
    expect(state.cache.lines).toEqual([]);
  });
});
