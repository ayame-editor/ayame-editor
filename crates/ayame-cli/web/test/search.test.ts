import { afterEach, describe, expect, it, vi } from "vitest";

import {
  COUNT_DEBOUNCE_MS,
  renderGrepResults,
  scheduleCount,
  updateCount,
} from "../src/search.js";
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
