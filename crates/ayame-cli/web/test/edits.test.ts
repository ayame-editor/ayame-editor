import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../src/editor.js", () => ({
  cachedLine: vi.fn(() => null),
  focusEditor: vi.fn(),
  maxFirst: vi.fn(() => 0),
  render: vi.fn(),
  revealCaret: vi.fn(),
  revealLine: vi.fn(),
  rowsVisible: vi.fn(() => 8),
  setCaret: vi.fn(),
  setFirst: vi.fn(),
}));
vi.mock("../src/save.js", () => ({
  refreshStat: vi.fn(async () => {}),
  savingCount: 0,
  waitForSavingDone: vi.fn(async () => {}),
}));
vi.mock("../src/search.js", () => ({
  charLenOf: (s: string) => Array.from(s).length,
  flashCount: vi.fn(),
}));

import { applyRange, enqueueEdit, reloadViewport, settleEditQueue } from "../src/edits.js";
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

describe("edit generation guards", () => {
  beforeEach(() => {
    state.stat = { open: true };
    state.docGen = 1;
    state.editGen = 0;
    state.total = 10;
    state.first = 0;
    state.caret = { line: 1, col: 4 };
    state.activeLine = 1;
    state.goalCol = 4;
    state.cache = { start: 0, lines: [] };
    state.loadToken = 0;
    vi.stubGlobal("fetch", vi.fn());
  });

  afterEach(async () => {
    await settleEditQueue();
    vi.unstubAllGlobals();
  });

  it("drops queued edits whose document generation went stale before execution", async () => {
    let ran = false;
    const queued = enqueueEdit(() => {
      ran = true;
      return "ran";
    });

    state.docGen++;

    await expect(queued).resolves.toBeNull();
    expect(ran).toBe(false);
  });

  it("does not overwrite user caret motion when an edit response arrives late", async () => {
    const edit = deferredResponse();
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockImplementation((input) => {
      const path = String(input);
      if (path === "/api/edit/replace_range") return edit.promise;
      if (path.startsWith("/api/lines?")) {
        return Promise.resolve(jsonResponse({ lines: [], total: 3 }));
      }
      throw new Error(`unexpected fetch ${path}`);
    });

    const pending = applyRange(1, 4, 1, 4, "x");
    await Promise.resolve();
    state.editGen++;
    state.caret = { line: 2, col: 1 };
    edit.resolve(jsonResponse({ stats: { total_lines: 3 }, caret_line: 0, caret_col: 9 }));

    await pending;

    expect(state.caret).toEqual({ line: 2, col: 1 });
    expect(state.activeLine).toBe(2);
    expect(state.goalCol).toBe(4);
  });

  it("reloadViewport bumps loadToken to invalidate older viewport fetches", async () => {
    vi.mocked(fetch).mockResolvedValue(
      jsonResponse({ lines: [{ number: 0, text: "alpha" }], total: 1 }),
    );
    state.loadToken = 41;

    await reloadViewport();

    expect(state.loadToken).toBe(42);
    expect(state.cache.lines).toEqual([{ number: 0, text: "alpha" }]);
  });
});
