// Pure composition of the layers one row of text can carry: line-local syntax
// tokens, analysis-rule ranges, and Find matches.
//
// The renderer used to pick exactly one of these per row — any analysis rule
// or Find query replaced the syntax pass — and the built-in analysis profile
// meant a rule was always present. Each layer is already a sorted run of
// non-overlapping ranges, so composing them is a single merge that cuts the
// line at every boundary. Kept DOM-free so the cut points can be tested
// without a renderer.

import type { AnalysisRange } from "./analysis-model.js";
import type { SyntaxKind, SyntaxSpan } from "./syntax.js";

export type TextRange = { start: number; end: number };

export type RowSegment = {
  start: number;
  end: number;
  /** Token kind colouring this piece, or `null` for plain text. */
  syntax: SyntaxKind | null;
  /** The analysis range covering this piece. Pieces of one range share it by
   *  identity, which is how the renderer keeps a range in one element. */
  analysis: AnalysisRange | null;
  /** The Find match covering this piece, shared by identity the same way. */
  search: TextRange | null;
};

/// Every non-empty match of the Find matcher in one line. The matcher is the
/// shared global regex, so its cursor is reset first: a previous row may have
/// left it mid-string.
export function searchRanges(text: string, matcher: RegExp | null): TextRange[] {
  if (!matcher) return [];
  const ranges: TextRange[] = [];
  matcher.lastIndex = 0;
  let match: RegExpExecArray | null;
  while ((match = matcher.exec(text)) !== null) {
    if (match[0].length === 0) {
      matcher.lastIndex++; // never stall on an empty match
      continue;
    }
    ranges.push({ start: match.index, end: match.index + match[0].length });
  }
  return ranges;
}

type Token = { start: number; end: number; kind: SyntaxKind };

/// Offsets of the non-plain tokens. Spans describe the line left to right, so
/// their lengths accumulate into positions; a tokenizer that stops short just
/// leaves the remainder plain.
function syntaxTokens(length: number, spans: readonly SyntaxSpan[] | null): Token[] {
  const tokens: Token[] = [];
  if (!spans) return tokens;
  let offset = 0;
  for (const span of spans) {
    if (offset >= length) break;
    const end = Math.min(length, offset + span.text.length);
    if (span.kind !== "plain" && end > offset) tokens.push({ start: offset, end, kind: span.kind });
    offset = end;
  }
  return tokens;
}

/// Cut `[0, length)` at every boundary of the three layers. The segments are
/// contiguous, ordered, and cover the whole line; ranges reaching past the end
/// of the line are clipped to it.
export function composeRowSegments(
  length: number,
  syntax: readonly SyntaxSpan[] | null,
  analysis: readonly AnalysisRange[],
  search: readonly TextRange[],
): RowSegment[] {
  if (length <= 0) return [];
  const tokens = syntaxTokens(length, syntax);
  const cuts = new Set<number>([0, length]);
  const cut = (position: number) => {
    if (position > 0 && position < length) cuts.add(position);
  };
  for (const layer of [tokens, analysis, search]) {
    for (const range of layer) {
      cut(range.start);
      cut(range.end);
    }
  }
  const points = [...cuts].sort((a, b) => a - b);
  const segments: RowSegment[] = [];
  let token = 0;
  let rule = 0;
  let match = 0;
  for (let index = 0; index + 1 < points.length; index++) {
    const start = points[index];
    const end = points[index + 1];
    // Each layer is sorted and non-overlapping, so one forward cursor per
    // layer finds the range (if any) that contains this piece.
    while (token < tokens.length && tokens[token].end <= start) token++;
    while (rule < analysis.length && analysis[rule].end <= start) rule++;
    while (match < search.length && search[match].end <= start) match++;
    segments.push({
      start,
      end,
      syntax: token < tokens.length && tokens[token].start <= start ? tokens[token].kind : null,
      analysis: rule < analysis.length && analysis[rule].start <= start ? analysis[rule] : null,
      search: match < search.length && search[match].start <= start ? search[match] : null,
    });
  }
  return segments;
}
