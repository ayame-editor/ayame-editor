import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../src/selection.js", () => ({
  hasSelection: vi.fn(() => false),
  renderSelection: vi.fn(),
}));
vi.mock("../src/menus.js", () => ({ updateStatusPos: vi.fn() }));
vi.mock("../src/input.js", () => ({ anyModalOpen: vi.fn(() => false) }));

import {
  cacheLineResponse,
  ensureData,
  fillRow,
  formatLineNo,
  lineNumberChars,
} from "../src/editor.js";
import { state } from "../src/state.js";

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

async function flushPromises() {
  await new Promise((resolve) => setTimeout(resolve, 0));
}

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
