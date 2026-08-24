// Row reuse across renders (#142).
//
// `render()` used to rebuild every visible row on every frame — caret moves,
// selection drags and scroll ticks included — re-tokenizing each line into a
// fresh span tree. What is measured here is how many rows actually get rebuilt
// for a given change; the DOM nodes inside a row are new objects when it is
// refilled, which makes "was this row rebuilt?" observable without spies.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  pool,
  render,
  renderedRowKey,
  resetGeometryMeasurements,
  setCaret,
  setSelectionRenderer,
} from "../src/editor.js";
import { hasSelection } from "../src/selection-model.js";
import { clearSyntaxCache, highlightSpans, syntaxCacheSize } from "../src/syntax.js";
import { DEFAULT_SETTINGS, state } from "../src/state.js";

const ROWS = 40;
// Two viewports' worth: enough to scroll into unrendered rows, small enough
// that the suite is not paying for a 500-line jsdom render per case.
const DOC = Array.from({ length: 120 }, (_, i) => `const value${i} = "line ${i}"; // note`);

function fixture() {
  document.body.innerHTML = `
    <div id="viewport">
      <div id="ruler-corner"></div><div id="ruler-inner"></div>
      <div id="content"></div>
      <div id="sel-layer"></div>
      <div id="caret"></div>
      <textarea id="hidden-input"></textarea>
      <div id="vscroll"><div id="vthumb"></div></div>
    </div>
    <div id="vticks"></div>`;
  Object.defineProperty(document.getElementById("viewport"), "clientHeight", {
    configurable: true,
    value: ROWS * 18,
  });
}

function loadDocument(lines: string[]) {
  state.settings = { ...DEFAULT_SETTINGS, ruler: false, syntaxHighlight: true };
  state.syntax = { configured: false, favorites: [], mappings: [], overrides: {} };
  state.view.cache = { start: 0, lines: lines.map((text, number) => ({ number, text })) };
  state.view.total = lines.length;
  state.view.first = 0;
  state.caret.selection = null;
  state.caret.extraCursors = [];
  state.caret.activeLine = 0;
  state.caret.position = { line: 0, col: 0 };
  state.search.matcher = null;
  state.search.hits = null;
  state.analysis.matchers = [];
  state.analysis.visibleRuleIds = new Set();
  state.markers.bookmarks = new Set();
  state.markers.changeSaved = new Set();
  state.markers.changeUnsaved = new Set();
  state.markers.changeDeleted = new Set();
  state.doc.stat = { open: true, path: "/w/app.ts" } as never;
}

// Identity of the nodes a row is showing. A refilled row has new ones.
function rowFingerprints() {
  return pool.map((row) => (row.lastChild as HTMLElement)?.firstChild ?? null);
}

function rebuiltRows(before: unknown[], after: unknown[]) {
  return before.reduce((n: number, node, i) => n + (node === after[i] ? 0 : 1), 0);
}

describe("render row reuse (#142)", () => {
  beforeEach(() => {
    fixture();
    // The real selection predicate: the current-line wash is suppressed while
    // a selection exists, which is what the drag case below turns on.
    setSelectionRenderer(() => {}, hasSelection);
    pool.splice(0);
    resetGeometryMeasurements(true);
    clearSyntaxCache();
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response(JSON.stringify({ lines: [], total: 0 }), { status: 200 })),
    );
    loadDocument(DOC);
    render();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    pool.splice(0);
  });

  it("fills every visible row on the first pass", () => {
    expect(pool.length).toBeGreaterThan(ROWS);
    expect(renderedRowKey(pool[0])).toBeDefined();
  });

  // The frame that changed nothing is the common one: a re-render triggered by
  // something outside the text (a status refresh, a scroll that did not move).
  it("rebuilds nothing when nothing changed", () => {
    const before = rowFingerprints();
    render();
    expect(rebuiltRows(before, rowFingerprints())).toBe(0);
  });

  // Arrow-key repeat: only the row losing the current-line wash and the row
  // gaining it have anything to redo.
  it("rebuilds only the two rows a caret move touches", () => {
    const before = rowFingerprints();
    setCaret(5, 0);
    render();
    expect(rebuiltRows(before, rowFingerprints())).toBe(2);
  });

  // Dragging a selection: the current-line wash is suppressed while a
  // selection exists, so the one previously-active row is the only one that
  // changes — the selection itself is painted in its own layer.
  it("rebuilds one row when a selection appears", () => {
    const before = rowFingerprints();
    state.caret.selection = { anchor: { line: 2, col: 0 }, head: { line: 9, col: 4 } } as never;
    render();
    expect(rebuiltRows(before, rowFingerprints())).toBe(1);
  });

  it("rebuilds every row when the text itself changes", () => {
    const before = rowFingerprints();
    loadDocument(DOC.map((_, i) => `changed ${i}`));
    render();
    expect(rebuiltRows(before, rowFingerprints())).toBeGreaterThan(ROWS);
  });

  it("rebuilds every row when a display setting changes", () => {
    const before = rowFingerprints();
    state.settings = { ...state.settings, showWhitespace: true };
    render();
    expect(rebuiltRows(before, rowFingerprints())).toBeGreaterThan(ROWS);
  });

  it("rebuilds visible rows when the current tab's manual scheme changes", () => {
    const before = rowFingerprints();
    state.syntax = { ...state.syntax, configured: true, overrides: { "/w/app.ts": "plain" } };
    render();
    expect(rebuiltRows(before, rowFingerprints())).toBeGreaterThan(ROWS);
    expect(document.querySelector(".syn-keyword")).toBeNull();
  });

  // Rule visibility is toggled in place on one Set, so it is the one shared
  // input compared by value rather than identity.
  it("rebuilds every row when analysis rule visibility is toggled in place", () => {
    state.analysis.visibleRuleIds.add("rule-a");
    render();
    const before = rowFingerprints();
    state.analysis.visibleRuleIds.delete("rule-a");
    render();
    expect(rebuiltRows(before, rowFingerprints())).toBeGreaterThan(ROWS);
  });

  it("rebuilds every row when the bookmark set is replaced", () => {
    const before = rowFingerprints();
    state.markers.bookmarks = new Set([3]);
    render();
    expect(rebuiltRows(before, rowFingerprints())).toBeGreaterThan(ROWS);
  });

  it("rebuilds the rows a scroll brings into view", () => {
    const before = rowFingerprints();
    state.view.first = 60;
    render();
    expect(rebuiltRows(before, rowFingerprints())).toBeGreaterThan(ROWS);
  });
});

describe("syntax tokenization memo (#142)", () => {
  beforeEach(() => clearSyntaxCache());

  it("returns the same spans for a repeated line without re-tokenizing", () => {
    const first = highlightSpans('const a = "x"; // c', "app.ts");
    const second = highlightSpans('const a = "x"; // c', "app.ts");
    expect(second).toBe(first);
    expect(syntaxCacheSize()).toBe(1);
  });

  // The language is part of the key: the same text infers differently when the
  // path does not name one.
  it("keeps one entry per language for the same text", () => {
    const text = '{"ok": true}';
    expect(highlightSpans(text, "data.json")).not.toBe(highlightSpans(text, ""));
    expect(syntaxCacheSize()).toBe(2);
  });

  it("caches the absence of a language too", () => {
    expect(highlightSpans("plain words here", "notes.unknownext")).toBeNull();
    expect(highlightSpans("plain words here", "notes.unknownext")).toBeNull();
    expect(syntaxCacheSize()).toBe(1);
  });

  it("stays bounded while scrolling through a long document", () => {
    for (let i = 0; i < 5000; i++) highlightSpans(`const v${i} = ${i};`, "app.ts");
    expect(syntaxCacheSize()).toBeLessThanOrEqual(2048);
  });

  // One enormous line would cost more to hold than to re-tokenize, and a
  // giant-file editor meets exactly that shape.
  it("does not cache very long lines", () => {
    const huge = `const x = "${"y".repeat(8192)}";`;
    expect(highlightSpans(huge, "app.ts")).not.toBeNull();
    expect(syntaxCacheSize()).toBe(0);
  });
});
