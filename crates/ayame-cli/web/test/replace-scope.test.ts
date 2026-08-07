// Scoped replace (#173): the arithmetic that decides which part of a line a
// replace-all may rewrite, and the template expansion it writes with. Pure
// functions, so they are exercised straight rather than through the find bar.
import { describe, expect, it } from "vitest";

import {
  expandTemplate,
  replaceWithinWindow,
  scopeFromSelection,
  scopeLineRange,
  scopeWindow,
  WHOLE_LINE,
} from "../src/replace-scope.js";

function at(line: number, col: number) {
  return { line, col };
}

describe("selection to replace scope", () => {
  it("takes a multi-line selection as-is", () => {
    const scope = scopeFromSelection({ start: at(3, 4), end: at(9, 2) });
    expect(scope).toEqual({ start: at(3, 4), end: at(9, 2), rect: false });
  });

  // Pressing "replace all" with no selection means the document; a caret is
  // not a scope, and treating it as one would make the button do nothing.
  it("is no scope at all without a selection", () => {
    expect(scopeFromSelection(null)).toBeNull();
  });

  it("is no scope for an empty selection", () => {
    expect(scopeFromSelection({ start: at(2, 5), end: at(2, 5) })).toBeNull();
  });

  // A rectangular selection is drawn corner to corner in any direction; the
  // scope is the box it describes.
  it("normalizes a rectangle drawn right-to-left and bottom-to-top", () => {
    const scope = scopeFromSelection({ start: at(9, 12), end: at(4, 3), rect: true });
    expect(scope).toEqual({ start: at(4, 3), end: at(9, 12), rect: true });
  });
});

describe("the part of a line a scoped replace may touch", () => {
  it("is the whole line with no scope", () => {
    expect(scopeWindow(null, 0)).toEqual(WHOLE_LINE);
    expect(scopeLineRange(null)).toEqual({ first: 0, last: Infinity });
  });

  const scope = { start: at(3, 4), end: at(6, 8), rect: false };

  it("skips lines outside the selection entirely", () => {
    expect(scopeWindow(scope, 2)).toBeNull();
    expect(scopeWindow(scope, 7)).toBeNull();
    expect(scopeLineRange(scope)).toEqual({ first: 3, last: 6 });
  });

  it("starts at the anchor column on the first line", () => {
    expect(scopeWindow(scope, 3)).toEqual({ from: 4, to: Infinity });
  });

  it("stops at the head column on the last line", () => {
    expect(scopeWindow(scope, 6)).toEqual({ from: 0, to: 8 });
  });

  it("covers whole lines in between", () => {
    expect(scopeWindow(scope, 4)).toEqual({ from: 0, to: Infinity });
  });

  it("is bounded on both ends for a single-line selection", () => {
    const oneLine = { start: at(5, 2), end: at(5, 9), rect: false };
    expect(scopeWindow(oneLine, 5)).toEqual({ from: 2, to: 9 });
  });

  it("applies the rectangle's columns to every line it spans", () => {
    const rect = { start: at(2, 3), end: at(5, 7), rect: true };
    for (const line of [2, 3, 4, 5]) {
      expect(scopeWindow(rect, line)).toEqual({ from: 3, to: 7 });
    }
    expect(scopeWindow(rect, 6)).toBeNull();
  });
});

describe("replacing inside a column window", () => {
  const literal = (source: string) => new RegExp(source, "g");

  it("rewrites every match when the whole line is in scope", () => {
    const out = replaceWithinWindow("foo bar foo", literal("foo"), "baz", false, WHOLE_LINE);
    expect(out).toEqual({ text: "baz bar baz", count: 2 });
  });

  it("leaves matches outside the window alone", () => {
    // "foo bar foo": only the second occurrence (cols 8..11) is selected.
    const out = replaceWithinWindow("foo bar foo", literal("foo"), "baz", false, {
      from: 8,
      to: Infinity,
    });
    expect(out).toEqual({ text: "foo bar baz", count: 1 });
  });

  // Half-in is out: rewriting a match that merely starts inside the selection
  // would edit characters the user did not select.
  it("skips a match that runs past the end of the window", () => {
    const out = replaceWithinWindow("xx foobar", literal("foobar"), "Z", false, {
      from: 0,
      to: 6,
    });
    expect(out).toEqual({ text: "xx foobar", count: 0 });
  });

  it("reports no change when nothing in the window matched", () => {
    const out = replaceWithinWindow("alpha beta", literal("gamma"), "Z", false, WHOLE_LINE);
    expect(out).toEqual({ text: "alpha beta", count: 0 });
  });

  // Columns are Unicode scalars, string indices are UTF-16 code units; an
  // astral character ahead of the window makes the two disagree.
  it("measures the window in characters, not UTF-16 units", () => {
    const out = replaceWithinWindow("🌸ab ab", literal("ab"), "Z", false, { from: 4, to: 6 });
    expect(out).toEqual({ text: "🌸ab Z", count: 1 });
  });

  it("does not carry a shared matcher's lastIndex between lines", () => {
    const shared = literal("a");
    shared.lastIndex = 5;
    expect(replaceWithinWindow("aaa", shared, "b", false, WHOLE_LINE)).toEqual({
      text: "bbb",
      count: 3,
    });
  });
});

describe("replacement template expansion", () => {
  function match(source: string, subject: string) {
    return subject.match(new RegExp(source))!;
  }

  // In literal mode "$" is a character the user typed, not syntax.
  it("uses the template verbatim in literal mode", () => {
    expect(expandTemplate(match("b", "abc"), "$& and $1", false)).toBe("$& and $1");
  });

  it("expands the whole match and numbered groups", () => {
    const m = match("(\\w+)@(\\w+)", "mail bob@example rest");
    expect(expandTemplate(m, "<$&|$2/$1>", true)).toBe("<bob@example|example/bob>");
  });

  it("expands named groups", () => {
    const m = match("(?<user>\\w+)@", "bob@host");
    expect(expandTemplate(m, "[$<user>]", true)).toBe("[bob]");
  });

  it("expands the before and after context and the escaped dollar", () => {
    const m = match("b", "abc");
    expect(expandTemplate(m, "$`|$'|$$", true)).toBe("a|c|$");
  });

  it("leaves a group reference that does not exist alone", () => {
    const m = match("(a)", "a");
    expect(expandTemplate(m, "$1$7", true)).toBe("a$7");
  });

  // "$12" is group 12 when the pattern has one, and group 1 followed by "2"
  // when it does not — the same climb-down String.replace performs.
  it("prefers a two-digit group but falls back to one digit plus a literal", () => {
    const many = "0123456789ab".split("").map((c) => `(${c})`).join("");
    const m = match(many, "0123456789ab");
    expect(expandTemplate(m, "$12", true)).toBe("b");
    expect(expandTemplate(match("(a)", "a"), "$12", true)).toBe("a2");
  });

  // The whole-word matcher wraps the pattern in lookaround; expanding from the
  // match object means the context lookaround needs is never re-required.
  it("expands a lookaround-wrapped match without re-running the pattern", () => {
    const m = match("(?<![\\p{L}\\p{N}_])(?:cat)(?![\\p{L}\\p{N}_])", "a cat here");
    expect(expandTemplate(m, "[$&]", true)).toBe("[cat]");
  });
});
