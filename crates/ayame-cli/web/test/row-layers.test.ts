// Row text layering: syntax spans, analysis-rule marks, and Find marks.
//
// These three used to be two mutually exclusive render paths: any analysis
// matcher or Find query replaced the syntax pass outright. Every install
// starts with the built-in "Log basics" profile, so its matchers were always
// present and syntax highlighting never rendered at all. The suites that
// covered highlighting reset `state.analysis.matchers` to `[]`, which is the
// one state a real session is never in.
import { beforeEach, describe, expect, it } from "vitest";

import { compileAnalysisMatchers, defaultAnalysisProfile } from "../src/analysis-model.js";
import { fillRow } from "../src/editor.js";
import { composeRowSegments, searchRanges } from "../src/row-layers.js";
import { clearSyntaxCache } from "../src/syntax.js";
import { DEFAULT_SETTINGS, state } from "../src/state.js";

function freshRow() {
  const row = document.createElement("div");
  row.append(document.createElement("span"), document.createElement("span"));
  return row;
}

function textOf(row: HTMLElement) {
  return (row.lastChild as HTMLElement).textContent;
}

function useDefaultAnalysisProfile() {
  const profile = defaultAnalysisProfile();
  state.analysis.matchers = compileAnalysisMatchers(profile);
  state.analysis.visibleRuleIds = new Set(profile.rules.map((rule) => rule.id));
}

describe("composeRowSegments", () => {
  it("returns one plain segment when no layer applies", () => {
    expect(composeRowSegments(5, null, [], [])).toEqual([
      { start: 0, end: 5, syntax: null, analysis: null, search: null },
    ]);
  });

  it("returns nothing for an empty line", () => {
    expect(composeRowSegments(0, null, [], [])).toEqual([]);
  });

  it("cuts at every layer boundary and keeps each layer's identity", () => {
    // "const answer = 42"
    const syntax = [
      { kind: "keyword" as const, text: "const" },
      { kind: "plain" as const, text: " answer = " },
      { kind: "number" as const, text: "42" },
    ];
    const rule = { start: 3, end: 8, color: "warn", overlap: false, ruleIds: ["r"] };
    const match = { start: 6, end: 12 };
    const segments = composeRowSegments(17, syntax, [rule], [match]);
    expect(segments.map((s) => [s.start, s.end, s.syntax])).toEqual([
      [0, 3, "keyword"],
      [3, 5, "keyword"],
      [5, 6, null],
      [6, 8, null],
      [8, 12, null],
      [12, 15, null],
      [15, 17, "number"],
    ]);
    expect(segments.map((s) => s.analysis === rule)).toEqual([
      false,
      true,
      true,
      true,
      false,
      false,
      false,
    ]);
    expect(segments.map((s) => s.search === match)).toEqual([
      false,
      false,
      false,
      true,
      true,
      false,
      false,
    ]);
  });

  it("covers the whole line when the syntax spans stop short of it", () => {
    const segments = composeRowSegments(6, [{ kind: "keyword", text: "let" }], [], []);
    expect(segments).toEqual([
      { start: 0, end: 3, syntax: "keyword", analysis: null, search: null },
      { start: 3, end: 6, syntax: null, analysis: null, search: null },
    ]);
  });

  it("ignores ranges that fall outside the line", () => {
    const segments = composeRowSegments(4, null, [], [{ start: 2, end: 9 }]);
    expect(segments.map((s) => [s.start, s.end, !!s.search])).toEqual([
      [0, 2, false],
      [2, 4, true],
    ]);
  });
});

describe("searchRanges", () => {
  it("collects every non-empty match and never stalls on an empty one", () => {
    expect(searchRanges("a1b22c", /\d*/g)).toEqual([
      { start: 1, end: 2 },
      { start: 3, end: 5 },
    ]);
    expect(searchRanges("abc", null)).toEqual([]);
  });

  it("restarts a matcher left mid-string by an earlier row", () => {
    const matcher = /b/g;
    matcher.lastIndex = 2;
    expect(searchRanges("abc", matcher)).toEqual([{ start: 1, end: 2 }]);
  });
});

describe("row rendering composes every layer", () => {
  beforeEach(() => {
    clearSyntaxCache();
    state.settings = { ...DEFAULT_SETTINGS, syntaxHighlight: true };
    state.syntax = { configured: false, favorites: [], mappings: [], overrides: {} };
    state.search.matcher = null;
    state.analysis.matchers = [];
    state.analysis.visibleRuleIds = new Set();
    state.markers.bookmarks = new Set();
    state.markers.changeSaved = new Set();
    state.markers.changeUnsaved = new Set();
    state.markers.changeDeleted = new Set();
    state.doc.stat = { open: true, path: "/w/app.ts" } as never;
  });

  it("keeps syntax highlighting while the default analysis profile is active", () => {
    useDefaultAnalysisProfile();
    const row = freshRow();
    fillRow(row, 3, { text: 'const label = "ready"; // note' });
    expect(row.querySelector(".syn-keyword")?.textContent).toBe("const");
    expect(row.querySelector(".syn-string")?.textContent).toBe('"ready"');
    expect(row.querySelector(".syn-comment")?.textContent).toBe("// note");
    expect(row.querySelector(".analysis-mark")).toBeNull();
    expect(textOf(row)).toBe('const label = "ready"; // note');
  });

  it("keeps syntax highlighting while a Find query is active", () => {
    state.search.matcher = /label/g;
    const row = freshRow();
    fillRow(row, 3, { text: 'const label = "ready";' });
    expect(row.querySelector("mark")?.textContent).toBe("label");
    expect(row.querySelector(".syn-keyword")?.textContent).toBe("const");
    expect(row.querySelector(".syn-string")?.textContent).toBe('"ready"');
    expect(textOf(row)).toBe('const label = "ready";');
  });

  it("keeps one continuous mark across the tokens a match spans", () => {
    state.search.matcher = /const label = "re/g;
    const row = freshRow();
    fillRow(row, 3, { text: 'const label = "ready";' });
    const marks = row.querySelectorAll("mark");
    expect(marks).toHaveLength(1);
    expect(marks[0].textContent).toBe('const label = "re');
    // The tokens inside the match are still tagged, nested under the mark.
    expect(marks[0].querySelector(".syn-keyword")?.textContent).toBe("const");
    expect(marks[0].querySelector(".syn-string")?.textContent).toBe('"re');
    expect(textOf(row)).toBe('const label = "ready";');
  });

  it("layers an analysis mark over syntax tokens without splitting it", () => {
    useDefaultAnalysisProfile();
    state.doc.stat = { open: true, path: "/w/app.log" } as never;
    const row = freshRow();
    fillRow(row, 0, { text: "2026-06-30T00:00:02Z ERROR failed to open file" });
    const marks = row.querySelectorAll<HTMLElement>(".analysis-mark");
    expect(marks).toHaveLength(1);
    expect(marks[0].textContent).toBe("ERROR");
    expect(marks[0].dataset.analysisColor).toBe("danger");
    // The log scheme's own level token survives inside the rule's mark.
    expect(marks[0].querySelector(".syn-level-error")?.textContent).toBe("ERROR");
    expect(textOf(row)).toBe("2026-06-30T00:00:02Z ERROR failed to open file");
  });

  it("nests the Find mark inside the analysis mark it overlaps", () => {
    useDefaultAnalysisProfile();
    state.search.matcher = /ERR/g;
    state.doc.stat = { open: true, path: "/w/notes.txt" } as never;
    const row = freshRow();
    fillRow(row, 0, { text: "an ERROR here" });
    const analysis = row.querySelector(".analysis-mark");
    expect(analysis?.textContent).toBe("ERROR");
    expect(analysis?.querySelector("mark")?.textContent).toBe("ERR");
    expect(textOf(row)).toBe("an ERROR here");
  });

  it("renders plain text untouched when highlighting is off and nothing matches", () => {
    state.settings = { ...DEFAULT_SETTINGS, syntaxHighlight: false };
    useDefaultAnalysisProfile();
    const row = freshRow();
    fillRow(row, 3, { text: "const quiet = 1;" });
    const tx = row.lastChild as HTMLElement;
    expect(tx.children).toHaveLength(0);
    expect(tx.textContent).toBe("const quiet = 1;");
  });

  it("marks trailing spaces once, on the last piece of the line", () => {
    state.settings = { ...DEFAULT_SETTINGS, syntaxHighlight: true, showWhitespace: true };
    state.search.matcher = /let/g;
    const row = freshRow();
    fillRow(row, 3, { text: "let a = 1;  " });
    expect(row.querySelectorAll(".ws-trail")).toHaveLength(2);
    expect(row.querySelectorAll(".ws-eol")).toHaveLength(1);
  });
});
