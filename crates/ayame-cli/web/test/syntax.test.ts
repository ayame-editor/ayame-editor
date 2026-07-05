import { describe, expect, it } from "vitest";

import { highlightSpans, languageForPath } from "../src/syntax.js";

describe("syntax helpers", () => {
  it("detects common languages from file names", () => {
    expect(languageForPath("/tmp/app.ts")).toBe("javascript");
    expect(languageForPath("Cargo.lock")).toBe("json");
    expect(languageForPath("server.log")).toBe("log");
  });

  it("marks JSON keys, strings, numbers, and literals", () => {
    const spans = highlightSpans('{"ok": true, "name": "ayame", "n": 42}', "data.json")!;
    expect(spans.map((s) => [s.kind, s.text]).filter(([kind]) => kind !== "plain")).toEqual([
      ["op", "{"],
      ["key", '"ok"'],
      ["op", ":"],
      ["literal", "true"],
      ["op", ","],
      ["key", '"name"'],
      ["op", ":"],
      ["string", '"ayame"'],
      ["op", ","],
      ["key", '"n"'],
      ["op", ":"],
      ["number", "42"],
      ["op", "}"],
    ]);
  });

  it("marks code keywords and line comments without scanning past the row", () => {
    const spans = highlightSpans("export function run() { return true } // done", "app.ts")!;
    expect(spans.some((s) => s.kind === "keyword" && s.text === "export")).toBe(true);
    expect(spans.some((s) => s.kind === "function" && s.text === "run")).toBe(true);
    expect(spans.at(-1)).toEqual({ kind: "comment", text: "// done" });
  });

  it("highlights log levels for extensionless log-like lines", () => {
    const spans = highlightSpans("2026-07-05 20:54:59 ERROR release failed", "")!;
    expect(spans.map((s) => s.kind)).toContain("level-error");
  });
});
