import { afterEach, describe, expect, it, vi } from "vitest";

import { setActiveLine, setSearchHits, setSelection } from "../src/editor.js";
import {
  currentOpenerMode,
  resolveOpener,
  setOpenerMode,
  setOpenerResolver,
} from "../src/opener-state.js";
import { state } from "../src/state.js";

afterEach(() => {
  vi.unstubAllGlobals();
  setOpenerMode("open");
  setOpenerResolver(null);
});

describe("AppState boundaries (#122)", () => {
  it("exposes typed responsibility groups without legacy flat aliases", () => {
    expect(Object.keys(state).sort()).toEqual([
      "analysis",
      "caret",
      "doc",
      "markers",
      "opener",
      "runtime",
      "search",
      "settings",
      "view",
    ]);
    expect("query" in state).toBe(false);
    expect("stat" in state).toBe(false);
    expect("sel" in state).toBe(false);
    expect("openerResolve" in state).toBe(false);
  });

  it("routes render-sensitive mutations through scheduling setters", () => {
    const requestAnimationFrame = vi.fn(() => 1);
    vi.stubGlobal("requestAnimationFrame", requestAnimationFrame);

    const selection = {
      anchor: { line: 3, col: 1 },
      head: { line: 3, col: 4 },
    };
    const hits = [{ line: 3, column: 1, byte: 7, byte_len: 3 }];

    setSelection(selection);
    setSearchHits(hits);
    setActiveLine(3);

    expect(state.caret.selection).toEqual(selection);
    expect(state.search.hits).toEqual(hits);
    expect(state.caret.activeLine).toBe(3);
    expect(requestAnimationFrame).toHaveBeenCalledOnce();
  });

  it("keeps opener mode and Promise resolution outside AppState", () => {
    const resolve = vi.fn();
    setOpenerMode("save");
    setOpenerResolver(resolve);

    resolveOpener({ path: "/tmp/note.txt", overwrite: false });
    resolveOpener(null);

    expect(currentOpenerMode()).toBe("save");
    expect(resolve).toHaveBeenCalledOnce();
    expect(resolve).toHaveBeenCalledWith({ path: "/tmp/note.txt", overwrite: false });
    expect("mode" in state.opener).toBe(false);
  });
});
