// Go to Line entry parsing.
//
// The old handler was `parseInt(input.replace(/[^0-9]/g, ""))`: every form
// below other than a bare number silently went to a different line.
import { describe, expect, it } from "vitest";

import { parseGotoTarget } from "../src/goto-target.js";

const context = { currentLine: 500, totalLines: 200_000 };
const parse = (input: string, over: Partial<typeof context> = {}) =>
  parseGotoTarget(input, { ...context, ...over });

describe("parseGotoTarget", () => {
  it("reads a bare line number, with or without the gutter's grouping", () => {
    expect(parse("120")).toEqual({ line: 120, column: 1 });
    expect(parse("  120  ")).toEqual({ line: 120, column: 1 });
    expect(parse("1,234")).toEqual({ line: 1234, column: 1 });
    expect(parse("123,456")).toEqual({ line: 123_456, column: 1 });
  });

  it("reads line:column instead of gluing the digits together", () => {
    // Used to land on line 1208.
    expect(parse("120:8")).toEqual({ line: 120, column: 8 });
    expect(parse("1,234:56")).toEqual({ line: 1234, column: 56 });
    expect(parse("120:0")).toEqual({ line: 120, column: 1 });
  });

  it("reads a percentage as a position through the file", () => {
    // Used to land on line 50.
    expect(parse("50%")).toEqual({ line: 100_001, column: 1 });
    expect(parse("0%")).toEqual({ line: 1, column: 1 });
    expect(parse("100%")).toEqual({ line: 200_000, column: 1 });
    expect(parse("12.5%")).toEqual({ line: 25_001, column: 1 });
    expect(parse("50%", { totalLines: 1 })).toEqual({ line: 1, column: 1 });
  });

  it("reads a signed number relative to the caret", () => {
    expect(parse("+20")).toEqual({ line: 520, column: 1 });
    expect(parse("-20")).toEqual({ line: 480, column: 1 });
    expect(parse("+1,000:4")).toEqual({ line: 1500, column: 4 });
  });

  it("clamps to the document instead of failing", () => {
    expect(parse("0")).toEqual({ line: 1, column: 1 });
    expect(parse("999999999")).toEqual({ line: 200_000, column: 1 });
    expect(parse("-9999")).toEqual({ line: 1, column: 1 });
    expect(parse("+999999999")).toEqual({ line: 200_000, column: 1 });
    expect(parse("7", { totalLines: 0 })).toEqual({ line: 1, column: 1 });
  });

  it("folds full-width input left on by an IME", () => {
    expect(parse("１２０：８")).toEqual({ line: 120, column: 8 });
    expect(parse("５０％")).toEqual({ line: 100_001, column: 1 });
    expect(parse("＋２０")).toEqual({ line: 520, column: 1 });
  });

  it("refuses what it cannot read rather than guessing a line", () => {
    for (const input of [
      "",
      "   ",
      "abc",
      "12,5", // a mistyped 12:5, not line 125
      "1,23,456",
      "12:",
      ":5",
      "12:5:1",
      "150%",
      "+50%",
      "-",
      "1e3",
      "12.5",
      "0x10",
    ]) {
      expect(parse(input), JSON.stringify(input)).toBeNull();
    }
  });

  it("refuses a number too large to be a line", () => {
    expect(parse("99999999999999999999")).toBeNull();
    expect(parse("5:99999999999999999999")).toBeNull();
  });
});
