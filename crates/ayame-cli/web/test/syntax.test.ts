import { describe, expect, it } from "vitest";

import {
  highlightSpans,
  languageForPath,
  resolveSyntaxScheme,
  SYNTAX_LINE_LIMIT,
  SYNTAX_SCHEMES,
} from "../src/syntax.js";

describe("syntax helpers", () => {
  it("detects common languages from file names", () => {
    expect(languageForPath("/tmp/app.ts")).toBe("javascript");
    expect(languageForPath("Cargo.lock")).toBe("json");
    expect(languageForPath("server.log")).toBe("log");
    expect(languageForPath("Dockerfile.dev")).toBe("dockerfile");
    expect(languageForPath("nginx.conf")).toBe("nginx");
    expect(languageForPath("settings.properties")).toBe("ini");
    expect(languageForPath("table.tsv")).toBe("tsv");
    expect(languageForPath("pom.xml")).toBe("xml");
    expect(languageForPath("main.cpp")).toBe("c");
    expect(languageForPath("Application.java")).toBe("java");
    expect(languageForPath("Program.cs")).toBe("csharp");
    expect(languageForPath("httpd.conf")).toBe("apache");
    expect(languageForPath("Makefile")).toBe("makefile");
  });

  it("resolves user globs in their declared order before automatic aliases", () => {
    const mappings = [
      { glob: "*.conf", scheme: "apache" as const },
      { glob: "*.conf", scheme: "nginx" as const },
    ];
    expect(languageForPath("custom.conf", mappings)).toBe("apache");
    expect(resolveSyntaxScheme("custom.conf", "plain", mappings)).toBe("plain");
  });

  it("derives unique scheme ids and UI metadata from one registry", () => {
    expect(new Set(SYNTAX_SCHEMES.map((scheme) => scheme.id)).size).toBe(SYNTAX_SCHEMES.length);
    expect(SYNTAX_SCHEMES.every((scheme) => scheme.labelKey && scheme.categoryKey)).toBe(true);
    expect(
      SYNTAX_SCHEMES.every(
        (scheme) =>
          scheme.contextLines == null ||
          (Number.isSafeInteger(scheme.contextLines) &&
            scheme.contextLines >= 0 &&
            scheme.contextLines <= 8),
      ),
    ).toBe(true);
    expect(SYNTAX_SCHEMES.map((scheme) => scheme.id)).toEqual(
      expect.arrayContaining(["toml", "xml", "c", "java", "csharp", "csv", "tsv"]),
    );
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

  it("honors manual schemes for extensionless files and Plain Text", () => {
    const shell = highlightSpans("if true; then echo ok; fi", "README", { scheme: "shell" })!;
    expect(shell.some((span) => span.kind === "keyword" && span.text === "if")).toBe(true);
    expect(highlightSpans('{"looks":"json"}', "data.json", { scheme: "plain" })).toBeNull();
  });

  it("highlights config keys and delimited headers with semantic tokens", () => {
    const toml = highlightSpans('name = "ayame" # app', "Cargo.toml")!;
    expect(toml.map((span) => span.kind)).toEqual(
      expect.arrayContaining(["key", "string", "comment"]),
    );
    const csv = highlightSpans('name,"message"', "events.csv", { line: 0 })!;
    expect(csv.filter((span) => span.kind === "heading").map((span) => span.text)).toEqual([
      "name",
      '"message"',
    ]);
    const quoted = highlightSpans('"a ""quoted"" value",42', "events.csv", { line: 1 })!;
    expect(quoted.find((span) => span.kind === "string")?.text).toBe('"a ""quoted"" value"');
    expect(quoted.find((span) => span.kind === "number")?.text).toBe("42");
  });

  it("never lexes beyond the bounded prefix of a giant single line", () => {
    const text = `const value = 1;${"x".repeat(SYNTAX_LINE_LIMIT * 2)}`;
    const spans = highlightSpans(text, "huge.ts")!;
    expect(spans.some((span) => span.kind === "keyword" && span.text === "const")).toBe(true);
    expect(spans.map((span) => span.text).join("")).toBe(text);
  });
});
