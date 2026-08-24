import { describe, expect, it } from "vitest";

import {
  completionPrefix,
  emptyPairRange,
  newlineIndent,
  pairCloser,
  shouldAutoClose,
  shouldSkipCloser,
  wordsFromText,
} from "../src/input-assist.js";

describe("deterministic input assistance (#246)", () => {
  it("pairs delimiters, skips an existing close, and detects empty pairs", () => {
    expect(pairCloser("{")).toBe("}");
    expect(shouldAutoClose("value = ", 8, "{")).toBe(true);
    expect(shouldAutoClose("value = \\", 9, '"')).toBe(false);
    expect(shouldSkipCloser("call()", 5, ")")).toBe(true);
    expect(emptyPairRange("call()", 5)).toEqual({ start: 4, end: 6 });
  });

  it("inherits indentation and adds one conservative provider level", () => {
    expect(newlineIndent("  child", 7, "indent")).toBe("  ");
    expect(newlineIndent("  child:", 8, "indent")).toBe("    ");
    expect(newlineIndent("\tif ready {", 11, "brace")).toBe("\t\t");
    expect(newlineIndent("  <child>", 9, "markup")).toBe("    ");
    expect(newlineIndent("  message", 9, "log")).toBe("  ");
  });

  it("extracts Unicode prefixes and bounded-size document words", () => {
    expect(completionPrefix("const 日本語Name", 13)).toBe("日本語Name");
    expect(wordsFromText("alpha 日本語 beta_2 x")).toEqual(["alpha", "日本語", "beta_2"]);
  });
});
