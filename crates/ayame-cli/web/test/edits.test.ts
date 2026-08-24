import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../src/editor.js", () => ({
  cacheLineResponse: vi.fn((start: number, response: { lines: unknown[]; total: number }) => {
    state.view.cache = { start, lines: response.lines };
    state.view.total = response.total;
  }),
  cachedLine: vi.fn(() => null),
  focusEditor: vi.fn(),
  maxFirst: vi.fn(() => 0),
  refreshChangeHistoryOverview: vi.fn(() => Promise.resolve()),
  render: vi.fn(),
  revealCaret: vi.fn(),
  revealLine: vi.fn(),
  rowsVisible: vi.fn(() => 8),
  setActiveLine: vi.fn((line: number) => {
    state.caret.activeLine = line;
  }),
  setCaret: vi.fn(),
  setFirst: vi.fn(),
  setSearchHits: vi.fn((hits) => {
    state.search.hits = hits;
  }),
  setSelection: vi.fn((selection) => {
    state.caret.selection = selection;
  }),
}));
vi.mock("../src/notifications.js", () => ({ flashCount: vi.fn() }));

import {
  applyCaseMode,
  applyRange,
  backspace,
  deleteLines,
  duplicateLines,
  enqueueEdit,
  insertNewline,
  moveLines,
  reloadViewport,
  settleEditQueue,
  typeAssistedText,
} from "../src/edits.js";
import { state } from "../src/state.js";
import { refreshChangeHistoryOverview, setCaret } from "../src/editor.js";
import { activeFoldMap, collapseBlock } from "../src/fold-state.js";

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
    state.doc.stat = { open: true };
    state.doc.generation = 1;
    state.caret.editGeneration = 0;
    state.view.total = 10;
    state.view.first = 0;
    state.caret.position = { line: 1, col: 4 };
    state.caret.activeLine = 1;
    state.caret.goalCol = 4;
    state.view.cache = { start: 0, lines: [] };
    state.caret.selection = null;
    state.caret.extraCursors = [];
    state.view.loadToken = 0;
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

    state.doc.generation++;

    await expect(queued).resolves.toBeNull();
    expect(ran).toBe(false);
  });

  it("clears stale fold coordinates before edit/undo/redo queue work", async () => {
    state.doc.stat = { open: true, path: "/tmp/edit.json" };
    collapseBlock({ start: 1, end: 8, level: 0 });
    expect(activeFoldMap().size).toBe(1);

    await enqueueEdit(async () => null);

    expect(activeFoldMap().size).toBe(0);
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
    state.caret.editGeneration++;
    state.caret.position = { line: 2, col: 1 };
    edit.resolve(jsonResponse({ stats: { total_lines: 3 }, caret_line: 0, caret_col: 9 }));

    await pending;

    expect(state.caret.position).toEqual({ line: 2, col: 1 });
    expect(state.caret.activeLine).toBe(2);
    expect(state.caret.goalCol).toBe(4);
  });

  it("reloadViewport bumps loadToken to invalidate older viewport fetches", async () => {
    vi.mocked(fetch).mockResolvedValue(
      jsonResponse({ lines: [{ number: 0, text: "alpha" }], total: 1 }),
    );
    state.view.loadToken = 41;

    await reloadViewport();

    expect(state.view.loadToken).toBe(42);
    expect(state.view.cache.lines).toEqual([{ number: 0, text: "alpha" }]);
  });

  it("drops a stale position overview when its refresh fails", async () => {
    vi.mocked(fetch).mockResolvedValue(jsonResponse({ lines: [], total: 1 }));
    vi.mocked(refreshChangeHistoryOverview).mockRejectedValueOnce(new Error("overview offline"));
    state.markers.changeHistoryOverview = { revision: 1 };
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});

    await reloadViewport();

    expect(state.markers.changeHistoryOverview).toBeNull();
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
    state.caret.selection = {
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
    state.caret.editGeneration++;
    state.caret.selection = null;
    state.caret.position = { line: 0, col: 1 };
    state.caret.activeLine = 0;
    edit.resolve(jsonResponse({ stats: { total_lines: 10 } }));

    await settleEditQueue();

    expect(state.caret.selection).toBeNull();
    expect(state.caret.position).toEqual({ line: 0, col: 1 });
    expect(setCaret).not.toHaveBeenCalled();
  });
});

describe("batched input assistance (#246)", () => {
  beforeEach(() => {
    state.doc.stat = { open: true, path: "/tmp/example.rs" };
    state.doc.generation = 1;
    state.caret.editGeneration = 0;
    state.caret.position = { line: 0, col: 0 };
    state.caret.activeLine = 0;
    state.caret.goalCol = 0;
    state.caret.selection = null;
    state.caret.extraCursors = [];
    state.view.first = 0;
    state.view.total = 2;
    state.view.cache = { start: 0, lines: [] };
    state.settings.closePairs = true;
    state.settings.selectionEnclosure = true;
    state.settings.autoIndent = true;
    vi.clearAllMocks();
  });

  afterEach(async () => {
    await settleEditQueue();
    vi.unstubAllGlobals();
  });

  function captureInputEdits(lines: string[]) {
    const batches: unknown[][] = [];
    const ranges: unknown[] = [];
    state.view.total = lines.length;
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
          return jsonResponse({
            stats: { total_lines: lines.length },
            carets: body.edits.map((edit) => ({
              line: edit.l0,
              col: edit.c0 + Array.from(edit.text).length,
            })),
          });
        }
        if (path === "/api/edit/replace_range") {
          const body = JSON.parse(String(init?.body));
          ranges.push(body);
          return jsonResponse({
            stats: { total_lines: lines.length },
            caret_line: body.l0,
            caret_col: body.c0 + Array.from(body.text).length,
          });
        }
        throw new Error(`unexpected fetch ${path}`);
      }),
    );
    return { batches, ranges };
  }

  it("auto-closes at multiple carets as one undo batch", async () => {
    const captured = captureInputEdits(["ab", "cd"]);
    state.caret.position = { line: 0, col: 1 };
    state.caret.extraCursors = [{ line: 1, col: 1 }];

    await typeAssistedText("(");

    expect(captured.batches).toEqual([
      [
        { l0: 0, c0: 1, l1: 0, c1: 1, text: "()" },
        { l0: 1, c0: 1, l1: 1, c1: 1, text: "()" },
      ],
    ]);
    expect(state.caret.position).toEqual({ line: 0, col: 2 });
    expect(state.caret.extraCursors).toEqual([{ line: 1, col: 2 }]);
  });

  it("encloses a rectangular selection with boundary-only edits", async () => {
    const captured = captureInputEdits(["abcd", "efgh"]);
    state.caret.position = { line: 1, col: 3 };
    state.caret.selection = {
      anchor: { line: 0, col: 1 },
      head: { line: 1, col: 3 },
      rect: true,
    };

    await typeAssistedText("[");

    expect(captured.batches).toEqual([
      [
        { l0: 0, c0: 1, l1: 0, c1: 1, text: "[" },
        { l0: 0, c0: 3, l1: 0, c1: 3, text: "]" },
        { l0: 1, c0: 1, l1: 1, c1: 1, text: "[" },
        { l0: 1, c0: 3, l1: 1, c1: 3, text: "]" },
      ],
    ]);
  });

  it("skips an existing closer locally without creating an undo edit", async () => {
    const captured = captureInputEdits(["call()"]);
    state.caret.position = { line: 0, col: 5 };

    await typeAssistedText(")");

    expect(captured.batches).toEqual([]);
    expect(state.caret.position).toEqual({ line: 0, col: 6 });
  });

  it("deletes both sides of an empty pair with one range edit", async () => {
    const captured = captureInputEdits(["call()"]);
    state.caret.position = { line: 0, col: 5 };

    backspace();
    await settleEditQueue();

    expect(captured.ranges).toEqual([{ l0: 0, c0: 4, l1: 0, c1: 6, text: "" }]);
  });

  it("inherits indentation and adds one scheme-safe brace level", async () => {
    const captured = captureInputEdits(["  if ready {"]);
    state.caret.position = { line: 0, col: 12 };

    await insertNewline();

    expect(captured.batches).toEqual([[{ l0: 0, c0: 12, l1: 0, c1: 12, text: "\n    " }]]);
  });
});

describe("whole-line edit construction (#129)", () => {
  beforeEach(() => {
    state.doc.stat = { open: true };
    state.doc.generation = 1;
    state.caret.editGeneration = 0;
    state.view.first = 0;
    state.caret.position = { line: 1, col: 0 };
    state.caret.activeLine = 1;
    state.caret.goalCol = 0;
    state.view.cache = { start: 0, lines: [] };
    state.caret.selection = null;
    state.caret.extraCursors = [];
    state.view.loadToken = 0;
    vi.clearAllMocks();
  });

  afterEach(async () => {
    await settleEditQueue();
    vi.unstubAllGlobals();
  });

  async function captureLineEdits(lines: string[], action: () => void) {
    const batches: unknown[][] = [];
    state.view.total = lines.length;
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
    state.caret.position = { line: 1, col: 3 };
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
    state.caret.selection = sel;
    state.caret.position = { line: 2, col: 1 };
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
