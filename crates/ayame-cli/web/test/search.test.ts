import { afterEach, describe, expect, it, vi } from "vitest";

import { charLenOf, inlineWordDiff, updateCount, utf16IndexOfCol } from "../src/search.js";
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
  document.body.innerHTML = '<span id="find-count"></span>';
  await updateCount();
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

  it("marks changed word runs while preserving shared tokens", () => {
    const diff = inlineWordDiff("alpha beta gamma", "alpha delta gamma");
    expect(diff.oldParts.map((p) => [p.text, p.changed])).toEqual([
      ["alpha ", false],
      ["beta", true],
      [" gamma", false],
    ]);
    expect(diff.newParts.map((p) => [p.text, p.changed])).toEqual([
      ["alpha ", false],
      ["delta", true],
      [" gamma", false],
    ]);
  });
});

describe("search count request generation (#123)", () => {
  it("aborts an older request and ignores its late response", async () => {
    document.body.innerHTML = '<span id="find-count"></span>';
    vi.stubGlobal("requestAnimationFrame", vi.fn(() => 1));
    const first = deferredResponse();
    const second = deferredResponse();
    const fetchMock = vi.fn().mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
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
