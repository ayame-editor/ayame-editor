// Ayame Editor — side-effect-free selection and multi-cursor model helpers.
import { state } from "./state.js";

// Normalized selection: { start, end } with start <= end, or null.
export function selRange() {
  if (!state.sel) return null;
  const { anchor: a, head: h } = state.sel;
  const forward = a.line < h.line || (a.line === h.line && a.col <= h.col);
  const range: any = forward ? { start: a, end: h } : { start: h, end: a };
  range.rect = !!state.sel.rect;
  return range;
}

export function rectRange() {
  if (!state.sel?.rect) return null;
  const a = state.sel.anchor;
  const h = state.sel.head;
  return {
    l0: Math.min(a.line, h.line),
    l1: Math.max(a.line, h.line),
    c0: Math.min(a.col, h.col),
    c1: Math.max(a.col, h.col),
  };
}

export function clonePoint(point) {
  return { line: point.line, col: point.col };
}

export function cloneSelection(selection) {
  return selection
    ? {
        anchor: clonePoint(selection.anchor),
        head: clonePoint(selection.head),
        rect: !!selection.rect,
      }
    : null;
}

export function normalizedRange(anchor, head) {
  const forward = anchor.line < head.line || (anchor.line === head.line && anchor.col <= head.col);
  return forward
    ? { start: clonePoint(anchor), end: clonePoint(head) }
    : { start: clonePoint(head), end: clonePoint(anchor) };
}

export function rangeEmpty(range) {
  return range.start.line === range.end.line && range.start.col === range.end.col;
}

export function rangeKey(range) {
  return `${range.start.line}:${range.start.col}:${range.end.line}:${range.end.col}`;
}

// Primary caret plus the extra cursors, deduped and in document order.
export function allCursors() {
  const cursors = [];
  const seen = new Set();
  const push = (cursor, primary) => {
    const key = `${cursor.line}:${cursor.col}`;
    if (seen.has(key)) return;
    seen.add(key);
    cursors.push({
      line: cursor.line,
      col: cursor.col,
      primary,
      sel: primary
        ? cloneSelection(state.sel && !state.sel.rect ? state.sel : null)
        : cloneSelection(cursor.sel),
    });
  };
  push(state.caret, true);
  for (const cursor of state.extraCursors) push(cursor, false);
  cursors.sort((a, b) => a.line - b.line || a.col - b.col);
  return cursors;
}

export function cursorSelectionRange(cursor) {
  const selection = cursor.primary ? state.sel : cursor.sel;
  if (!selection || selection.rect) return null;
  const range = normalizedRange(selection.anchor, selection.head);
  return rangeEmpty(range) ? null : range;
}

export function selectionRanges() {
  const ranges = [];
  const seen = new Set();
  const add = (range, primary = false) => {
    if (!range || rangeEmpty(range)) return;
    const key = rangeKey(range);
    if (seen.has(key)) return;
    seen.add(key);
    ranges.push({ ...range, primary });
  };
  if (rectRange()) return ranges;
  add(selRange(), true);
  for (const cursor of state.extraCursors) add(cursorSelectionRange(cursor), false);
  ranges.sort((a, b) => a.start.line - b.start.line || a.start.col - b.start.col);
  return ranges;
}

export function hasSelection() {
  const rect = rectRange();
  if (rect) return rect.l0 !== rect.l1 || rect.c0 !== rect.c1;
  const range = selRange();
  return (!!range && !rangeEmpty(range)) || selectionRanges().length > 0;
}

// A zero-width rectangular range selects no text even when it spans lines.
export function hasTextSelection() {
  const rect = rectRange();
  if (rect) return rect.c0 !== rect.c1;
  return selectionRanges().length > 0;
}

export function rangeLineCount(range) {
  return range.end.line - range.start.line + 1;
}

export function selectionLineCount(range = null) {
  const rect = rectRange();
  if (rect) return rect.l1 - rect.l0 + 1;
  if (range) return rangeLineCount(range);
  return selectionRanges().reduce((count, item) => count + rangeLineCount(item), 0);
}

export function hasCursorSelections() {
  const primary = state.sel && !state.sel.rect ? selRange() : null;
  if (primary && !rangeEmpty(primary)) return true;
  return state.extraCursors.some((cursor) => {
    const range = cursorSelectionRange(cursor);
    return range && !rangeEmpty(range);
  });
}
