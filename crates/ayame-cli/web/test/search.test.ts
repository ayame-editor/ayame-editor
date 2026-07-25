import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("../src/edits.js", () => ({
  applyBatchPlain: vi.fn(async () => {}),
  applyRange: vi.fn(async () => {}),
  enqueueEdit: vi.fn(async (fn) => fn()),
  gotoLine: vi.fn(),
}));
vi.mock("../src/dialogs.js", () => ({
  askForm: vi.fn(),
  hideLoading: vi.fn(),
  showLoading: vi.fn(),
  showMessage: vi.fn(),
}));
vi.mock("../src/notifications.js", () => ({ flashCount: vi.fn() }));

import {
  buildMatcher,
  COUNT_DEBOUNCE_MS,
  findStep,
  renderGrepResults,
  replaceAll,
  scheduleCount,
  updateCount,
} from "../src/search.js";
import { applyBatchPlain, enqueueEdit } from "../src/edits.js";
import { hideLoading, showLoading } from "../src/dialogs.js";
import { flashCount } from "../src/notifications.js";
import { charLenOf, utf16IndexOfCol } from "../src/text.js";
import { state } from "../src/state.js";

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

function deferredResponse() {
  let resolve!: (value: Response) => void;
  const promise = new Promise<Response>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

afterEach(async () => {
  state.query = "";
  state.regexError = false;
  state.lastMatch = null;
  document.body.innerHTML = '<span id="find-count"></span>';
  await updateCount();
  vi.useRealTimers();
  vi.unstubAllGlobals();
  vi.clearAllMocks();
});

describe("search pure helpers", () => {
  it("maps Unicode-scalar columns to UTF-16 indexes", () => {
    const text = "a😀b";
    expect(charLenOf(text)).toBe(3);
    expect(utf16IndexOfCol(text, 0)).toBe(0);
    expect(utf16IndexOfCol(text, 1)).toBe(1);
    expect(utf16IndexOfCol(text, 2)).toBe(3);
    expect(utf16IndexOfCol(text, 3)).toBe(4);
  });

  it("shares exact comma-aware gutter sizing with grep results (#252)", () => {
    document.body.innerHTML = '<div id="grep-results"></div>';
    const response = {
      hits: [
        { path: "/tmp/a.log", line: 66, col: 0, text: "small" },
        { path: "/tmp/a.log", line: 999, col: 0, text: "large" },
      ],
      truncated: false,
      files_scanned: 1,
      files_truncated: false,
    };

    state.settings.lineNumberCommas = true;
    renderGrepResults(response, "", false);
    const view = document.getElementById("grep-results")!;
    expect(view.style.getPropertyValue("--gutter-ch")).toBe("5ch");
    expect([...view.querySelectorAll(".grep-ln")].map((el) => el.textContent)).toEqual([
      "67",
      "1,000",
    ]);

    state.settings.lineNumberCommas = false;
    renderGrepResults(response, "", false);
    expect(view.style.getPropertyValue("--gutter-ch")).toBe("4ch");
    expect(view.querySelectorAll(".grep-ln")[1]?.textContent).toBe("1000");
    state.settings.lineNumberCommas = true;
  });
});

describe("matcher construction (#129)", () => {
  function mountFind() {
    document.body.innerHTML =
      '<div id="find-wrap"><input id="find"></div><span id="find-count"></span>';
  }

  it("escapes literal queries and applies case-insensitive flags", () => {
    mountFind();
    Object.assign(state, { query: "a+b", regex: false, ci: true, word: false });

    buildMatcher();

    expect(state.matcher?.flags).toContain("i");
    expect("A+B".match(state.matcher || /$^/)).toEqual(["A+B"]);
    expect(document.getElementById("find-wrap")?.classList.contains("error")).toBe(false);
  });

  it("falls back from the Unicode whole-word wrapper when the plain regex is valid", () => {
    mountFind();
    Object.assign(state, { query: "\\8", regex: true, ci: false, word: true });

    buildMatcher();

    expect(state.regexError).toBe(false);
    expect(state.matcherWordFallback).toBe(true);
    expect(state.matcher?.flags).toBe("g");
  });

  it("marks a regex invalid when both wrapped and plain forms fail", () => {
    mountFind();
    Object.assign(state, { query: "[", regex: true, ci: false, word: true });

    buildMatcher();

    expect(state.matcher).toBeNull();
    expect(state.regexError).toBe(true);
    expect(document.getElementById("find-wrap")?.classList.contains("error")).toBe(true);
  });
});

describe("replace all batching (#129)", () => {
  function prepareReplace() {
    document.body.innerHTML =
      '<div><input id="find"></div><input id="replace-input" value="bar"><span id="find-count"></span>';
    Object.assign(state, {
      stat: { open: true },
      query: "foo",
      regex: false,
      ci: false,
      word: false,
      regexError: false,
      matcherWordFallback: false,
      lastMatch: null,
    });
    vi.stubGlobal(
      "requestAnimationFrame",
      vi.fn(() => 1),
    );
  }

  it("continues truncated search pages without duplicating line edits", async () => {
    prepareReplace();
    const lines = ["foo x", "none", "foo foo", "none", "none", "none", "foo"];
    const fetchMock = vi.fn(async (input) => {
      const path = String(input);
      if (path.includes("/api/search?") && path.includes("max=20000")) {
        if (path.includes("start=0")) {
          return jsonResponse({
            hits: [
              { line: 2, byte: 2, byte_len: 3 },
              { line: 4, byte: 20, byte_len: 1 },
            ],
            truncated: true,
          });
        }
        if (path.includes("start=21")) {
          return jsonResponse({
            hits: [
              { line: 4, byte: 21, byte_len: 3 },
              { line: 8, byte: 40, byte_len: 3 },
            ],
            truncated: false,
          });
        }
      }
      if (path === "/api/lines?start=2&count=7") {
        return jsonResponse({
          lines: lines.map((text, index) => ({ number: index + 2, text })),
          total: 9,
        });
      }
      if (path.includes("/api/search?") && path.includes("max=2000")) {
        return jsonResponse({ hits: [], truncated: false });
      }
      throw new Error(`unexpected fetch ${path}`);
    });
    vi.stubGlobal("fetch", fetchMock);

    await replaceAll();

    expect(fetchMock.mock.calls.map(([input]) => String(input))).toContainEqual(
      expect.stringContaining("start=21&max=20000"),
    );
    expect(enqueueEdit).toHaveBeenCalledOnce();
    expect(applyBatchPlain).toHaveBeenCalledWith([
      { l0: 2, c0: 0, l1: 2, c1: 5, text: "bar x" },
      { l0: 4, c0: 0, l1: 4, c1: 7, text: "bar bar" },
      { l0: 8, c0: 0, l1: 8, c1: 3, text: "bar" },
    ]);
    expect(showLoading).toHaveBeenCalledOnce();
    expect(hideLoading).toHaveBeenCalledOnce();
  });

  it("flushes edit batches at the 2000-edit boundary", async () => {
    prepareReplace();
    const hitCount = 2001;
    const hits = Array.from({ length: hitCount }, (_, line) => ({
      line,
      byte: line * 4,
      byte_len: 3,
    }));
    const fetchMock = vi.fn(async (input) => {
      const path = String(input);
      if (path.includes("/api/search?") && path.includes("max=20000")) {
        return jsonResponse({ hits, truncated: false });
      }
      if (path.includes("/api/search?") && path.includes("max=2000")) {
        return jsonResponse({ hits: [], truncated: false });
      }
      if (path.startsWith("/api/lines?")) {
        const url = new URL(path, "http://localhost");
        const start = Number(url.searchParams.get("start"));
        const count = Number(url.searchParams.get("count"));
        return jsonResponse({
          lines: Array.from({ length: count }, (_, offset) => ({
            number: start + offset,
            text: "foo",
          })),
          total: hitCount,
        });
      }
      throw new Error(`unexpected fetch ${path}`);
    });
    vi.stubGlobal("fetch", fetchMock);

    await replaceAll();

    expect(applyBatchPlain).toHaveBeenCalledTimes(2);
    expect(vi.mocked(applyBatchPlain).mock.calls[0][0]).toHaveLength(2000);
    expect(vi.mocked(applyBatchPlain).mock.calls[1][0]).toHaveLength(1);
  });
});

describe("find navigation wrapping (#129)", () => {
  it.each([
    ["next", 103, 0, { line: 1, byte: 4, byte_len: 3 }],
    ["prev", 100, 200, { line: 18, byte: 180, byte_len: 3 }],
  ])("wraps %s from the opposite document edge", async (dir, initialFrom, wrapFrom, hit) => {
    document.body.innerHTML =
      '<div><input id="find"></div><span id="find-count"></span><div id="viewport"></div>';
    Object.defineProperty(document.getElementById("viewport"), "clientHeight", {
      configurable: true,
      value: 180,
    });
    Object.assign(state, {
      stat: { open: true },
      query: "foo",
      regex: false,
      ci: false,
      word: false,
      total: 20,
      first: 0,
      lastMatch: { byte: 100, len: dir === "next" ? 3 : 0 },
      cache: {
        start: 0,
        lines: Array.from({ length: 20 }, (_, number) => ({ number, text: "foo" })),
      },
    });
    vi.stubGlobal(
      "requestAnimationFrame",
      vi.fn(() => 1),
    );
    const fetchMock = vi.fn(async (input) => {
      const path = String(input);
      if (path === "/api/linebyte?line=20") return jsonResponse({ byte: 200 });
      if (path.includes(`/api/find?dir=${dir}&from=${initialFrom}`)) {
        return jsonResponse({ hit: null });
      }
      if (path.includes(`/api/find?dir=${dir}&from=${wrapFrom}`)) {
        return jsonResponse({ hit });
      }
      if (path.includes("/api/search?")) {
        return jsonResponse({ hits: [hit], truncated: false });
      }
      throw new Error(`unexpected fetch ${path}`);
    });
    vi.stubGlobal("fetch", fetchMock);

    await findStep(dir);

    expect(fetchMock.mock.calls.map(([input]) => String(input))).toEqual(
      expect.arrayContaining([
        expect.stringContaining(`/api/find?dir=${dir}&from=${initialFrom}`),
        expect.stringContaining(`/api/find?dir=${dir}&from=${wrapFrom}`),
      ]),
    );
    expect(state.lastMatch).toEqual({ byte: hit.byte, len: hit.byte_len });
    expect(state.caret.line).toBe(hit.line);
    expect(flashCount).toHaveBeenCalledOnce();
  });
});

describe("search count request generation (#123)", () => {
  it("aborts an older request and ignores its late response", async () => {
    document.body.innerHTML = '<span id="find-count"></span>';
    vi.stubGlobal(
      "requestAnimationFrame",
      vi.fn(() => 1),
    );
    const first = deferredResponse();
    const second = deferredResponse();
    const fetchMock = vi
      .fn()
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise);
    vi.stubGlobal("fetch", fetchMock);

    state.query = "old";
    const oldCount = updateCount();
    state.query = "new";
    const newCount = updateCount();

    const firstSignal = fetchMock.mock.calls[0][1].signal as AbortSignal;
    expect(firstSignal.aborted).toBe(true);

    second.resolve(jsonResponse({ hits: [{ byte: 2 }], truncated: false }));
    await newCount;
    expect(state.searchHits).toEqual([{ byte: 2 }]);
    expect(document.getElementById("find-count")?.textContent).toBe("1 matches");

    first.resolve(jsonResponse({ hits: [{ byte: 1 }, { byte: 3 }], truncated: false }));
    await oldCount;
    expect(state.searchHits).toEqual([{ byte: 2 }]);
    expect(document.getElementById("find-count")?.textContent).toBe("1 matches");
  });
});

describe("live incremental count while typing (#162)", () => {
  it("debounces a burst of keystrokes into a single count request", async () => {
    vi.useFakeTimers();
    document.body.innerHTML = '<span id="find-count"></span>';
    vi.stubGlobal(
      "requestAnimationFrame",
      vi.fn(() => 1),
    );
    const fetchMock = vi
      .fn()
      .mockResolvedValue(
        jsonResponse({ hits: [{ byte: 0 }, { byte: 5 }, { byte: 9 }], truncated: false }),
      );
    vi.stubGlobal("fetch", fetchMock);

    // Three quick keystrokes; each schedules a debounced refresh.
    state.regexError = false;
    state.lastMatch = null;
    state.query = "a";
    scheduleCount();
    state.query = "ab";
    scheduleCount();
    state.query = "abc";
    scheduleCount();
    expect(fetchMock).not.toHaveBeenCalled(); // nothing fired yet — coalesced

    await vi.advanceTimersByTimeAsync(COUNT_DEBOUNCE_MS);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(fetchMock.mock.calls[0][0]).toContain("q=abc");
    expect(document.getElementById("find-count")?.textContent).toBe("3 matches");
  });

  it("does not query for an empty or invalid-regex query", async () => {
    vi.useFakeTimers();
    document.body.innerHTML = '<span id="find-count"></span>';
    const fetchMock = vi.fn().mockResolvedValue(jsonResponse({ hits: [], truncated: false }));
    vi.stubGlobal("fetch", fetchMock);

    state.query = "";
    state.regexError = false;
    scheduleCount();
    state.query = "a(";
    state.regexError = true;
    scheduleCount();

    await vi.advanceTimersByTimeAsync(COUNT_DEBOUNCE_MS);
    expect(fetchMock).not.toHaveBeenCalled();
  });
});
