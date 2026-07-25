import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../src/editor.js", () => ({
  cacheLineResponse: vi.fn((start: number, response: { lines: unknown[]; total: number }) => {
    state.cache = { start, lines: response.lines };
    state.total = response.total;
  }),
  cachedLine: vi.fn(() => null),
  focusEditor: vi.fn(),
  maxFirst: vi.fn(() => 0),
  refreshChangeHistoryOverview: vi.fn(() => Promise.resolve()),
  render: vi.fn(),
  revealCaret: vi.fn(),
  revealLine: vi.fn(),
  rowsVisible: vi.fn(() => 8),
  setCaret: vi.fn(),
  setFirst: vi.fn(),
}));
vi.mock("../src/notifications.js", () => ({ flashCount: vi.fn() }));

import {
  applyCaseMode,
  applyRange,
  backspace,
  deleteLines,
  duplicateLines,
  enqueueEdit,
  moveLines,
  reloadViewport,
  settleEditQueue,
} from "../src/edits.js";
import { state } from "../src/state.js";
import { refreshChangeHistoryOverview, setCaret } from "../src/editor.js";

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
    state.sel = null;
    state.extraCursors = [];
    state.loadToken = 0;
    vi.clearAllMocks();
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

  it("drops a stale position overview when its refresh fails", async () => {
    vi.mocked(fetch).mockResolvedValue(jsonResponse({ lines: [], total: 1 }));
    vi.mocked(refreshChangeHistoryOverview).mockRejectedValueOnce(new Error("overview offline"));
    state.changeHistoryOverview = { revision: 1 };
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});

    await reloadViewport();

    expect(state.changeHistoryOverview).toBeNull();
    expect(consoleError).toHaveBeenCalledOnce();
    consoleError.mockRestore();
  });

  it("does not restore a zero-width rectangle after the user moves mid-delete", async () => {
    const edit = deferredResponse();
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockImplementation((input) => {
      const path = String(input);
      if (path === "/api/edit/replace_batch") return edit.promise;
      if (path.startsWith("/api/lines?")) {
        return Promise.resolve(jsonResponse({ lines: [{ number: 1, text: "abcdef" }], total: 10 }));
      }
      throw new Error(`unexpected fetch ${path}`);
    });
    state.sel = {
      anchor: { line: 1, col: 3 },
      head: { line: 2, col: 3 },
      rect: true,
    };

    backspace();
    await vi.waitFor(() =>
      expect(fetchMock).toHaveBeenCalledWith(
        "/api/edit/replace_batch",
        expect.objectContaining({ method: "POST" }),
      ),
    );
    state.editGen++;
    state.sel = null;
    state.caret = { line: 0, col: 1 };
    state.activeLine = 0;
    edit.resolve(jsonResponse({ stats: { total_lines: 10 } }));

    await settleEditQueue();

    expect(state.sel).toBeNull();
    expect(state.caret).toEqual({ line: 0, col: 1 });
    expect(setCaret).not.toHaveBeenCalled();
  });
});

describe("whole-line edit construction (#129)", () => {
  beforeEach(() => {
    state.stat = { open: true };
    state.docGen = 1;
    state.editGen = 0;
    state.first = 0;
    state.caret = { line: 1, col: 0 };
    state.activeLine = 1;
    state.goalCol = 0;
    state.cache = { start: 0, lines: [] };
    state.sel = null;
    state.extraCursors = [];
    state.loadToken = 0;
    vi.clearAllMocks();
  });

  afterEach(async () => {
    await settleEditQueue();
    vi.unstubAllGlobals();
  });

  async function captureLineEdits(lines: string[], action: () => void) {
    const batches: unknown[][] = [];
    state.total = lines.length;
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input, init?: RequestInit) => {
        const path = String(input);
        if (path.startsWith("/api/lines?")) {
          const url = new URL(path, "http://localhost");
          const start = Number(url.searchParams.get("start"));
          const count = Number(url.searchParams.get("count"));
          return jsonResponse({
            lines: lines.slice(start, start + count).map((text, offset) => ({
              number: start + offset,
              text,
            })),
            total: lines.length,
          });
        }
        if (path === "/api/edit/replace_batch") {
          const body = JSON.parse(String(init?.body));
          batches.push(body.edits);
          return jsonResponse({ stats: { total_lines: lines.length } });
        }
        throw new Error(`unexpected fetch ${path}`);
      }),
    );

    action();
    await settleEditQueue();
    return batches;
  }

  it("duplicates the selected line block with one insertion edit", async () => {
    state.caret = { line: 1, col: 3 };
    const batches = await captureLineEdits(["alpha", "bravo", "charlie"], duplicateLines);

    expect(batches).toEqual([[{ l0: 1, c0: 5, l1: 1, c1: 5, text: "\nbravo" }]]);
  });

  it.each([
    [
      "up",
      -1,
      { anchor: { line: 1, col: 0 }, head: { line: 2, col: 1 } },
      { l0: 0, c0: 0, l1: 2, c1: 1, text: "b\nc\na" },
    ],
    [
      "down",
      1,
      { anchor: { line: 1, col: 0 }, head: { line: 2, col: 1 } },
      { l0: 1, c0: 0, l1: 3, c1: 1, text: "d\nb\nc" },
    ],
  ])("moves a selected block %s with one replacement edit", async (_label, dir, sel, edit) => {
    state.sel = sel;
    state.caret = { line: 2, col: 1 };
    const batches = await captureLineEdits(["a", "b", "c", "d"], () => moveLines(dir));

    expect(batches).toEqual([[edit]]);
  });

  it("deletes a middle line through the start of its successor", async () => {
    const batches = await captureLineEdits(["alpha", "bravo", "charlie"], deleteLines);

    expect(batches).toEqual([[{ l0: 1, c0: 0, l1: 2, c1: 0, text: "" }]]);
  });
});

describe("applyCaseMode", () => {
  it("keeps whole-string upper/lower semantics", () => {
    expect(applyCaseMode("Hello World", "upper")).toBe("HELLO WORLD");
    expect(applyCaseMode("Ｈｅｌｌｏ", "lower")).toBe("ｈｅｌｌｏ");
  });

  it("rewrites identifier runs per style, mirroring the core transform", () => {
    expect(applyCaseMode("hello_world code", "camel")).toBe("helloWorld code");
    expect(applyCaseMode("helloWorld", "snake")).toBe("hello_world");
    expect(applyCaseMode("HTTPServer v2Beta", "snake")).toBe("http_server v2_beta");
    expect(applyCaseMode("hello-world", "pascal")).toBe("HelloWorld");
    expect(applyCaseMode("HelloWorld", "kebab")).toBe("hello-world");
    expect(applyCaseMode("helloWorld", "constant")).toBe("HELLO_WORLD");
  });

  it("leaves non-ASCII text and doubled separators alone", () => {
    expect(applyCaseMode("日本語 snake_case です", "camel")).toBe("日本語 snakeCase です");
    expect(applyCaseMode("foo--bar", "pascal")).toBe("Foo--Bar");
  });
});
