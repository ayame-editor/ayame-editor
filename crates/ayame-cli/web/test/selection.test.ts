import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../src/edits.js", () => ({
  lineLensFor: vi.fn(),
  pasteText: vi.fn(),
  typeText: vi.fn(),
}));
vi.mock("../src/search.js", () => ({ flashCount: vi.fn() }));
vi.mock("../src/editor.js", () => ({
  caretX: vi.fn(() => 0),
  charWidth: vi.fn(() => 8),
  coordsFromEvent: vi.fn(),
  focusEditor: vi.fn(),
  lineChars: vi.fn(() => []),
  lineLen: vi.fn(() => 0),
  moveCaret: vi.fn(),
  rowsVisible: vi.fn(() => 10),
  scheduleRender: vi.fn(),
  setCaret: vi.fn(),
  setFirst: vi.fn(),
}));
vi.mock("../src/dialogs.js", () => ({
  askConfirm: vi.fn(),
  askForm: vi.fn(),
  hideLoading: vi.fn(),
  showLoading: vi.fn(),
  showMessage: vi.fn(),
}));

import { allCursors, normalizedRange, rangeKey, selectionRanges } from "../src/selection.js";
import { state } from "../src/state.js";

describe("selection range algebra", () => {
  beforeEach(() => {
    state.caret = { line: 3, col: 4 };
    state.sel = null;
    state.extraCursors = [];
  });

  it("normalizes reversed endpoints and gives ranges stable keys", () => {
    const r = normalizedRange({ line: 9, col: 2 }, { line: 4, col: 8 });

    expect(r).toEqual({
      start: { line: 4, col: 8 },
      end: { line: 9, col: 2 },
    });
    expect(rangeKey(r)).toBe("4:8:9:2");
  });

  it("dedupes cursors while preserving the primary marker", () => {
    state.extraCursors = [
      { line: 3, col: 4 },
      { line: 1, col: 9 },
      { line: 9, col: 0 },
      { line: 1, col: 9 },
    ];

    expect(allCursors().map(({ line, col, primary }) => [line, col, primary])).toEqual([
      [1, 9, false],
      [3, 4, true],
      [9, 0, false],
    ]);
  });

  it("dedupes equivalent selected ranges from primary and extra cursors", () => {
    state.sel = {
      anchor: { line: 4, col: 5 },
      head: { line: 2, col: 1 },
    };
    state.extraCursors = [
      {
        line: 9,
        col: 0,
        sel: { anchor: { line: 2, col: 1 }, head: { line: 4, col: 5 } },
      },
      {
        line: 10,
        col: 0,
        sel: { anchor: { line: 7, col: 0 }, head: { line: 7, col: 2 } },
      },
    ];

    expect(selectionRanges().map((range) => rangeKey(range))).toEqual(["2:1:4:5", "7:0:7:2"]);
  });
});
