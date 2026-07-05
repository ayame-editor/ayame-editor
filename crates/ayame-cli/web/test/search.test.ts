import { describe, expect, it } from "vitest";

import { charLenOf, inlineWordDiff, utf16IndexOfCol } from "../src/search.js";

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
