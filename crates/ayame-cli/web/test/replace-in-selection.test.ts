// Replace-all with a scope and a Cancel button (#173), driven through the real
// module against a stubbed server. What matters here is the whole loop: which
// lines it asks for, which text it writes back, and that Cancel stops it.
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../src/edits.js", () => ({
  applyBatchPlain: vi.fn(async () => {}),
  applyRange: vi.fn(async () => {}),
  enqueueEdit: vi.fn(async (fn) => fn()),
  gotoLine: vi.fn(),
}));

const cancelHandlers: Array<() => void> = [];
vi.mock("../src/dialogs.js", () => ({
  askForm: vi.fn(),
  hideLoading: vi.fn(),
  setLoadingDetail: vi.fn(),
  // Remember the overlay's Cancel action so a test can press it mid-run.
  showLoading: vi.fn((_text, opts) => {
    if (opts?.onCancel) cancelHandlers.push(opts.onCancel);
  }),
  showMessage: vi.fn(),
}));
vi.mock("../src/notifications.js", () => ({ flashCount: vi.fn() }));

import { activeReplaceScope, replaceAll } from "../src/replace.js";
import { defaultInSelection, inSelectionAvailable, stepHistory } from "../src/findbar.js";
import { applyBatchPlain } from "../src/edits.js";
import { showLoading } from "../src/dialogs.js";
import { state } from "../src/state.js";

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

const DOC = ["foo one", "foo two", "foo three", "foo four"];

// A server holding DOC, with one match at column 0 of every line.
function stubServer(extra: (path: string) => Response | null = () => null) {
  const calls: string[] = [];
  const fetchMock = vi.fn(async (input) => {
    const path = String(input);
    calls.push(path);
    const custom = extra(path);
    if (custom) return custom;
    if (path.startsWith("/api/ui_state")) return jsonResponse({});
    if (path.startsWith("/api/linebyte")) {
      const line = Number(new URL(path, "http://x").searchParams.get("line"));
      return jsonResponse({ byte: line * 10 });
    }
    if (path.includes("/api/search?") && path.includes("max=20000")) {
      const start = Number(new URL(path, "http://x").searchParams.get("start"));
      return jsonResponse({
        hits: DOC.map((_, line) => ({ line, byte: line * 10, byte_len: 3 })).filter(
          (h) => h.byte >= start,
        ),
        truncated: false,
      });
    }
    if (path.includes("/api/search?")) return jsonResponse({ hits: [], truncated: false });
    if (path.startsWith("/api/lines?")) {
      const url = new URL(path, "http://x");
      const start = Number(url.searchParams.get("start"));
      const count = Number(url.searchParams.get("count"));
      return jsonResponse({
        lines: DOC.slice(start, start + count).map((text, k) => ({ number: start + k, text })),
        total: DOC.length,
      });
    }
    throw new Error(`unexpected fetch ${path}`);
  });
  vi.stubGlobal("fetch", fetchMock);
  return { fetchMock, calls };
}

function prepare(selection: unknown, inSelection: boolean) {
  document.body.innerHTML =
    '<div><input id="find"></div><input id="replace-input" value="bar">' +
    '<span id="find-count"></span><button id="opt-in-selection"></button>';
  state.doc.stat = { open: true } as never;
  Object.assign(state.search, {
    query: "foo",
    regex: false,
    caseInsensitive: false,
    word: false,
    regexError: false,
    matcherWordFallback: false,
    lastMatch: null,
    replaceOpen: true,
    inSelection,
    replaceHistory: [],
    replaceHistoryIndex: -1,
  });
  state.caret.selection = selection as never;
  vi.stubGlobal(
    "requestAnimationFrame",
    vi.fn(() => 1),
  );
}

describe("replace all scoped to the selection", () => {
  beforeEach(() => {
    vi.mocked(applyBatchPlain).mockClear();
    vi.mocked(showLoading).mockClear();
    cancelHandlers.length = 0;
  });

  it("rewrites every line when no scope is asked for", async () => {
    prepare(null, false);
    stubServer();

    await replaceAll();

    expect(vi.mocked(applyBatchPlain).mock.calls[0][0].map((e) => e.l0)).toEqual([0, 1, 2, 3]);
  });

  // The point of the feature: lines outside the selection keep their text even
  // though the search found matches on them.
  it("rewrites only the selected lines", async () => {
    prepare({ anchor: { line: 1, col: 0 }, head: { line: 2, col: 9 } }, true);
    stubServer();

    await replaceAll();

    const batch = vi.mocked(applyBatchPlain).mock.calls[0][0];
    expect(batch.map((e) => e.l0)).toEqual([1, 2]);
    expect(batch.map((e) => e.text)).toEqual(["bar two", "bar three"]);
  });

  // A match starting before the selection's first column belongs to the text
  // above the anchor, not to the selection.
  it("leaves a match before the anchor column alone", async () => {
    prepare({ anchor: { line: 1, col: 2 }, head: { line: 2, col: 9 } }, true);
    stubServer();

    await replaceAll();

    const batch = vi.mocked(applyBatchPlain).mock.calls[0][0];
    expect(batch.map((e) => e.l0)).toEqual([2]);
  });

  // Starting the search sweep at the selection instead of byte 0 is what keeps
  // "replace inside this function" cheap in a huge file.
  it("starts the search sweep at the selection, not at the top of the file", async () => {
    prepare({ anchor: { line: 2, col: 0 }, head: { line: 3, col: 8 } }, true);
    const { calls } = stubServer();

    await replaceAll();

    expect(calls.some((c) => c.includes("/api/search?") && c.includes("start=20"))).toBe(true);
    expect(calls.some((c) => c.includes("/api/search?") && c.includes("start=0&max=20000"))).toBe(
      false,
    );
  });

  it("ignores the scope when the toggle is off", async () => {
    prepare({ anchor: { line: 1, col: 0 }, head: { line: 2, col: 9 } }, false);
    expect(activeReplaceScope()).toBeNull();
    stubServer();

    await replaceAll();

    expect(vi.mocked(applyBatchPlain).mock.calls[0][0]).toHaveLength(4);
  });
});

describe("canceling replace all", () => {
  beforeEach(() => {
    vi.mocked(applyBatchPlain).mockClear();
    cancelHandlers.length = 0;
  });

  it("offers a cancel action on the overlay", async () => {
    prepare(null, false);
    stubServer();

    await replaceAll();

    const opts = vi.mocked(showLoading).mock.calls.at(-1)?.[1];
    expect(opts?.cancel).toBe(true);
    expect(typeof opts?.onCancel).toBe("function");
  });

  // Cancel pressed while the search sweep is still running: nothing has been
  // written yet, so nothing should be.
  it("stops before writing anything when canceled during the search", async () => {
    prepare(null, false);
    stubServer((path) => {
      if (path.includes("/api/search?") && path.includes("max=20000")) {
        for (const cancel of cancelHandlers) cancel();
      }
      return null;
    });

    await replaceAll();

    expect(applyBatchPlain).not.toHaveBeenCalled();
  });
});

describe("replace field history", () => {
  it("records the replacement used by a replace-all", async () => {
    prepare(null, false);
    stubServer();

    await replaceAll();

    expect(state.search.replaceHistory).toEqual(["bar"]);
  });

  it("does not record an empty replacement", async () => {
    prepare(null, false);
    (document.getElementById("replace-input") as HTMLInputElement).value = "";
    stubServer();

    await replaceAll();

    expect(state.search.replaceHistory).toEqual([]);
  });

  // Up enters at the newest entry, Down at the oldest, and neither wraps past
  // the ends — the same rules the find field has always used.
  it("steps through a history list without wrapping", () => {
    const list = ["c", "b", "a"];
    expect(stepHistory(list, -1, -1)).toBe(0);
    expect(stepHistory(list, -1, 1)).toBe(2);
    expect(stepHistory(list, 0, -1)).toBe(0);
    expect(stepHistory(list, 2, 1)).toBe(2);
    expect(stepHistory(list, 1, 1)).toBe(2);
    expect(stepHistory([], -1, -1)).toBe(-1);
  });
});

describe("when the in-selection toggle is offered", () => {
  it("starts on for a multi-line selection", () => {
    prepare({ anchor: { line: 1, col: 0 }, head: { line: 4, col: 0 } }, false);
    expect(defaultInSelection()).toBe(true);
    expect(inSelectionAvailable()).toBe(true);
  });

  // A few characters on one line is the word someone is about to search for.
  // Scoping replace-all to it would make the button a near-no-op.
  it("stays off for a selection inside one line, but remains available", () => {
    prepare({ anchor: { line: 1, col: 2 }, head: { line: 1, col: 6 } }, false);
    expect(defaultInSelection()).toBe(false);
    expect(inSelectionAvailable()).toBe(true);
  });

  it("is unavailable with nothing selected", () => {
    prepare(null, false);
    expect(defaultInSelection()).toBe(false);
    expect(inSelectionAvailable()).toBe(false);
  });
});
