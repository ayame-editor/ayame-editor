import { beforeEach, describe, expect, it, vi } from "vitest";

// gotoLine reaches into the editor for the caret and viewport; mock those (and
// edits.js's other side-effecting imports) so we can drive it headlessly and
// assert exactly which line it targets. Mirrors the setup in edits.test.ts.
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
vi.mock("../src/notifications.js", () => ({ flashCount: vi.fn() }));

import { commas } from "../src/dom.js";
import { setCaret } from "../src/editor.js";
import { gotoLine } from "../src/edits.js";
import { state } from "../src/state.js";

// Ayame targets ten billion lines; JavaScript integers are exact up to 2^53-1.
const TEN_BILLION = 10_000_000_000;
const MAX_SAFE = Number.MAX_SAFE_INTEGER; // 9,007,199,254,740,991

describe("line-number precision at extreme scale (#53)", () => {
  it("formats line counts exactly up to the safe-integer ceiling", () => {
    expect(commas(17_586_323)).toBe("17,586,323");
    expect(commas(TEN_BILLION)).toBe("10,000,000,000");
    expect(commas(MAX_SAFE)).toBe("9,007,199,254,740,991");
    // No rounding: a line and its neighbour never collapse to the same label.
    expect(commas(TEN_BILLION)).not.toBe(commas(TEN_BILLION + 1));
  });

  it("keeps ten billion well inside the exactly-representable range", () => {
    expect(Number.isSafeInteger(TEN_BILLION)).toBe(true);
    expect(MAX_SAFE).toBeGreaterThan(TEN_BILLION);
    expect(TEN_BILLION - 1).not.toBe(TEN_BILLION);
    expect(TEN_BILLION + 1).not.toBe(TEN_BILLION);
  });

  describe("goto line", () => {
    beforeEach(() => {
      vi.mocked(setCaret).mockClear();
      state.total = TEN_BILLION;
    });

    it("jumps to an exact 1-based line near ten billion", () => {
      gotoLine(TEN_BILLION); // the final line
      expect(setCaret).toHaveBeenCalledWith(TEN_BILLION - 1, 0);
    });

    it("parses digit-grouped input without losing precision", () => {
      gotoLine("10,000,000,000");
      expect(setCaret).toHaveBeenCalledWith(TEN_BILLION - 1, 0);
    });

    it("clamps a beyond-end target to the exact last line", () => {
      gotoLine(TEN_BILLION * 2);
      expect(setCaret).toHaveBeenCalledWith(TEN_BILLION - 1, 0);
    });

    it("ignores empty or non-positive input", () => {
      gotoLine(0);
      gotoLine("abc");
      expect(setCaret).not.toHaveBeenCalled();
    });
  });
});
