// Ayame Editor — side-effect-free replacement scoping and template expansion.
//
// "Replace every occurrence of this name, but only inside this function" is the
// replace people actually reach for; without a scope, replace-all is the button
// nobody dares press (#173). The maths for "which part of this line may the
// replacement touch" lives here, away from the DOM and the network, so it can
// be exercised directly.
import { utf16IndexOfCol } from "./text.js";

// A half-open span of a single line, in Unicode scalar (char) offsets — the
// same units the editor's columns use. `to: Infinity` means "to end of line".
export interface ColumnWindow {
  from: number;
  to: number;
}

export interface ScopePoint {
  line: number;
  col: number;
}

// The user's selection, normalized: `start` never comes after `end`, and for a
// rectangular selection the columns apply to every line in the range.
export interface ReplaceScope {
  start: ScopePoint;
  end: ScopePoint;
  rect: boolean;
}

export const WHOLE_LINE: ColumnWindow = { from: 0, to: Infinity };

/// Which part of `line` a scoped replace may rewrite, or `null` when the line
/// lies outside the scope entirely. A `null` scope means the whole document, so
/// every line is fully in play.
export function scopeWindow(scope: ReplaceScope | null, line: number): ColumnWindow | null {
  if (!scope) return WHOLE_LINE;
  if (line < scope.start.line || line > scope.end.line) return null;
  if (scope.rect) return { from: scope.start.col, to: scope.end.col };
  const from = line === scope.start.line ? scope.start.col : 0;
  const to = line === scope.end.line ? scope.end.col : Infinity;
  return { from, to };
}

/// Lines the scope can possibly touch, as an inclusive pair. `null` scope means
/// the whole document.
export function scopeLineRange(scope: ReplaceScope | null): { first: number; last: number } {
  if (!scope) return { first: 0, last: Infinity };
  return { first: scope.start.line, last: scope.end.line };
}

/// A selection worth scoping to. A caret with nothing selected is not a scope —
/// it would make replace-all a no-op, which is not what pressing it means.
export function scopeFromSelection(range): ReplaceScope | null {
  if (!range) return null;
  const rect = !!range.rect;
  const start = { line: range.start.line, col: range.start.col };
  const end = { line: range.end.line, col: range.end.col };
  if (rect) {
    const cols = [start.col, end.col].sort((a, b) => a - b);
    return {
      start: { line: Math.min(start.line, end.line), col: cols[0] },
      end: { line: Math.max(start.line, end.line), col: cols[1] },
      rect: true,
    };
  }
  if (start.line === end.line && start.col === end.col) return null;
  return { start, end, rect: false };
}

/// Expand a replacement template against one concrete match, with JavaScript's
/// `String.replace` substitution rules.
///
/// Built from the match object rather than by re-running the pattern over the
/// matched text: the whole-word matcher wraps the user's pattern in lookaround,
/// and a second run over the match alone would see none of the context that
/// lookaround needs. In literal (non-regex) mode `$` is just a character and
/// the template is used verbatim, which is what a user typing `$` means.
export function expandTemplate(match: RegExpMatchArray, template: string, regexMode: boolean) {
  if (!regexMode) return template;
  const subject = typeof match.input === "string" ? match.input : "";
  const at = match.index ?? 0;
  const groups = match.groups || {};
  return template.replace(/\$(\$|&|`|'|<([^>]*)>|\d{1,2})/g, (whole, token, name) => {
    if (token === "$") return "$";
    if (token === "&") return match[0];
    if (token === "`") return subject.slice(0, at);
    if (token === "'") return subject.slice(at + match[0].length);
    if (name !== undefined) return groups[name] ?? "";
    // $1..$99, longest first: "$12" is group 12 when it exists, else group 1
    // followed by a literal "2" — the same climb-down String.replace does.
    const digits: string = token;
    const twoDigit = Number(digits);
    if (digits.length === 2 && twoDigit >= 1 && twoDigit < match.length) {
      return match[twoDigit] ?? "";
    }
    const oneDigit = Number(digits[0]);
    if (oneDigit >= 1 && oneDigit < match.length) {
      return (match[oneDigit] ?? "") + digits.slice(1);
    }
    return whole; // no such group: JavaScript leaves the token alone
  });
}

/// Rewrite the matches of `matcher` that fall entirely inside `window`.
///
/// Whole-match containment, not overlap: a match that merely starts inside the
/// selection is left alone, because rewriting it would edit text the user did
/// not select. Returns the new line text and how many matches were replaced —
/// counted here rather than in a separate pass, so the reported number is
/// exactly what was written.
export function replaceWithinWindow(
  text: string,
  matcher: RegExp,
  template: string,
  regexMode: boolean,
  window: ColumnWindow,
): { text: string; count: number } {
  const from = window.from > 0 ? utf16IndexOfCol(text, window.from) : 0;
  const to = window.to === Infinity ? text.length : utf16IndexOfCol(text, window.to);
  if (to <= from) return { text, count: 0 };
  // A fresh instance per line: `matcher` is a shared /g/ regex whose lastIndex
  // would otherwise leak from one line into the next.
  const re = new RegExp(matcher.source, matcher.flags);
  let out = "";
  let last = 0;
  let count = 0;
  for (const m of text.matchAll(re)) {
    const at = m.index ?? 0;
    const end = at + m[0].length;
    if (at < from || end > to) continue;
    out += text.slice(last, at) + expandTemplate(m, template, regexMode);
    last = end;
    count++;
  }
  if (!count) return { text, count: 0 };
  return { text: out + text.slice(last), count };
}
