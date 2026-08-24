// Ayame Editor — selection module. Type-stripped to JS at build time (build.rs, oxc).
import { $, commas, displayPath } from "./dom.js";
import { LINE_HEIGHT, MAX_COPY_LINES, OVERSCAN, state } from "./state.js";
import { serverMessage, t } from "./i18n.js";
import { api, apiPost, type FindResponse, type LinesResponse } from "./api.js";
import {
  caretX,
  charWidth,
  coordsFromEvent,
  focusEditor,
  lineByte,
  lineChars,
  lineLen,
  moveCaret,
  revealLine,
  rowsVisible,
  scheduleRender,
  scrollVisibleRows,
  setCaret,
  setFirst,
  setActiveLine,
  setSelection,
  setSelectionRenderer,
} from "./editor.js";
import {
  expandFoldsForLine,
  logicalLineAtVisible,
  visibleIndexForLine,
  visibleLinesFrom,
} from "./fold-state.js";
import { lineLensFor, pasteText, typeText } from "./edits.js";
import { flashCount } from "./notifications.js";
import { askForm, hideLoading, showLoading, showMessage } from "./dialogs.js";
import {
  allCursors,
  clonePoint,
  cloneSelection,
  cursorSelectionRange,
  hasCursorSelections,
  hasSelection,
  hasTextSelection,
  normalizedRange,
  rangeEmpty,
  rangeKey,
  rangeLineCount,
  rectRange,
  selRange,
  selectionLineCount,
  selectionRanges,
} from "./selection-model.js";
import { selectedTextForRange } from "./selection-text.js";
import { updateInSelectionUI } from "./findbar.js";
import { isWordChar } from "./text.js";
import { withOverwriteRetry } from "./saveflow.js";
import type { SelectionSaveRequest, SelectionSaveResponse } from "./types/api.js";

export {
  allCursors,
  clonePoint,
  cloneSelection,
  cursorSelectionRange,
  hasCursorSelections,
  hasSelection,
  hasTextSelection,
  normalizedRange,
  rangeEmpty,
  rangeKey,
  rangeLineCount,
  rectRange,
  selRange,
  selectedTextForRange,
  selectionLineCount,
  selectionRanges,
};

// ---- select next occurrence -----------------------------------------------

export function wordRangeAt(p) {
  const cs = lineChars(p.line);
  if (!cs.length) return null;
  let i = Math.min(p.col, cs.length - 1);
  if (!isWordChar(cs[i]) && p.col > 0 && isWordChar(cs[p.col - 1])) i = p.col - 1;
  if (!isWordChar(cs[i])) return null;
  let a = i;
  let b = i + 1;
  while (a > 0 && isWordChar(cs[a - 1])) a--;
  while (b < cs.length && isWordChar(cs[b])) b++;
  return { start: { line: p.line, col: a }, end: { line: p.line, col: b } };
}

export function selectPrimaryRange(r) {
  setSelection({ anchor: clonePoint(r.start), head: clonePoint(r.end) });
  state.caret.position = clonePoint(r.end);
  setActiveLine(state.caret.position.line);
  state.caret.goalCol = state.caret.position.col;
  state.caret.editGeneration++;
  revealLine(state.caret.position.line);
  focusEditor();
  scheduleRender();
}

export function promoteSelectionRange(r) {
  const nextKey = rangeKey(r);
  const old =
    state.caret.selection && !state.caret.selection.rect
      ? normalizedRange(state.caret.selection.anchor, state.caret.selection.head)
      : null;
  if (old && !rangeEmpty(old) && rangeKey(old) !== nextKey) {
    const exists = state.caret.extraCursors.some((c) => {
      const cr = cursorSelectionRange(c);
      return cr && rangeKey(cr) === rangeKey(old);
    });
    if (!exists) {
      state.caret.extraCursors.push({
        line: state.caret.selection.head.line,
        col: state.caret.selection.head.col,
        sel: cloneSelection(state.caret.selection),
      });
    }
  }
  state.caret.extraCursors = state.caret.extraCursors.filter((c) => {
    const cr = cursorSelectionRange(c);
    return !cr || rangeKey(cr) !== nextKey;
  });
  selectPrimaryRange(r);
}

export async function findNextOccurrenceRange(query, fromByte, existing) {
  const selected = new Set(existing.map(rangeKey));
  const charLen = Array.from(query).length;
  let from = fromByte;
  let wrapped = false;
  for (let i = 0; i < existing.length + 3; i++) {
    const params = new URLSearchParams({
      dir: "next",
      from: String(from),
      q: query,
      regex: "false",
      ci: "false",
      word: "false",
    });
    const res = await api<FindResponse>(`/api/find?${params.toString()}`);
    if (!res.hit) {
      if (wrapped) return null;
      from = 0;
      wrapped = true;
      continue;
    }
    const h = res.hit;
    const r = {
      start: { line: h.line, col: h.column },
      end: { line: h.line, col: h.column + charLen },
    };
    if (!selected.has(rangeKey(r))) return r;
    from = h.byte + Math.max(1, h.byte_len);
  }
  return null;
}

export async function selectNextOccurrence() {
  if (!state.doc.stat?.open) return;
  if (rectRange()) {
    flashCount(t("find.rectNoCtrlD"), "error");
    return;
  }
  let ranges = selectionRanges();
  if (!ranges.length) {
    const r = wordRangeAt(state.caret.position);
    if (!r) {
      flashCount(t("find.noWordToSelect"));
      return;
    }
    selectPrimaryRange(r);
    return;
  }
  const query = await selectedTextForRange(ranges[0]);
  if (!query || query.includes("\n")) {
    flashCount(t("find.multiLineNoCtrlD"), "error");
    return;
  }
  ranges = selectionRanges();
  const last = ranges[ranges.length - 1];
  const from = await lineByte(last.end.line, last.end.col);
  try {
    const next = await findNextOccurrenceRange(query, from, ranges);
    if (!next) {
      flashCount(t("find.noNextOccurrence"));
      return;
    }
    promoteSelectionRange(next);
  } catch (e) {
    flashCount(t("find.searchError"), "error");
    console.error(e);
  }
}

export function appendSelectionRect(layer, line, startCol, endCol, trailingNewline = false) {
  const left = caretX(line, startCol);
  const trail = trailingNewline ? charWidth() * 0.6 : 0;
  const width = caretX(line, endCol) - left + trail;
  const rect = document.createElement("div");
  rect.className = "selrect";
  rect.style.left = `${left}px`;
  rect.style.top = `${(visibleIndexForLine(line) - visibleIndexForLine(state.view.first)) * LINE_HEIGHT}px`;
  rect.style.width = `${Math.max(2, width)}px`;
  layer.append(rect);
}

export function renderRangeSelection(layer, r) {
  if (!r || rangeEmpty(r)) return;
  const vis = rowsVisible() + OVERSCAN;
  for (const line of visibleLinesFrom(state.view.first, vis)) {
    if (line < r.start.line || line > r.end.line || line >= state.view.total) continue;
    const startCol = line === r.start.line ? r.start.col : 0;
    const len = lineLen(line);
    // A line selected through its end extends a hair past the text so the
    // trailing newline reads as selected, like a normal editor.
    const endCol = line === r.end.line ? Math.min(r.end.col, len) : len;
    appendSelectionRect(layer, line, startCol, endCol, line !== r.end.line);
  }
}

export function renderSelection() {
  // The replace row's "in selection only" toggle is enabled by whether a
  // selection exists, so it is refreshed from the one place that already runs
  // whenever the selection is redrawn (#173). A no-op while the row is closed.
  updateInSelectionUI();
  const layer = $("sel-layer");
  layer.textContent = "";
  const rr = rectRange();
  if (rr) {
    const vis = rowsVisible() + OVERSCAN;
    for (const line of visibleLinesFrom(state.view.first, vis)) {
      if (line < rr.l0 || line > rr.l1 || line >= state.view.total) continue;
      appendSelectionRect(layer, line, rr.c0, rr.c1);
    }
    return;
  }
  for (const r of selectionRanges()) renderRangeSelection(layer, r);
}

export function initSelection() {
  setSelectionRenderer(renderSelection, hasSelection);
  const content = $("content");
  content.addEventListener("mousedown", (e) => {
    if (e.button !== 0) return;
    e.preventDefault(); // keep focus on the hidden input, not the div
    const p = coordsFromEvent(e);
    if ((e.ctrlKey || e.metaKey) && !e.shiftKey && !e.altKey) {
      // A recognized path/URL gets the platform convention Ctrl+Click. If the
      // bounded recognizer finds nothing, preserve Ayame's existing
      // multi-cursor toggle at the same point.
      if (state.doc.stat?.open) {
        void import("./recognition.js").then(async ({ openRecognizedAt }) => {
          if (!(await openRecognizedAt(p))) toggleExtraCursorAt(p.line, p.col);
          focusEditor();
        });
      }
      return;
    }
    if (e.detail >= 3) {
      // Triple-click: select the whole line (newline included when possible).
      selectLineAt(p.line);
      return;
    }
    if (e.shiftKey) {
      const anchor = state.caret.selection
        ? state.caret.selection.anchor
        : { ...state.caret.position };
      setSelection({ anchor, head: p, rect: e.altKey });
      state.caret.dragAnchor = anchor;
      state.caret.dragMoved = true;
    } else {
      setSelection(null); // a bare click collapses any selection to a caret
      state.caret.dragAnchor = p;
      state.caret.dragMoved = false;
    }
    state.caret.dragRect = e.altKey;
    setCaret(p.line, p.col);
    state.caret.dragging = true;
    focusEditor();
  });

  window.addEventListener("mousemove", (e) => {
    if (!state.caret.dragging) return;
    const p = coordsFromEvent(e);
    const a = state.caret.dragAnchor;
    if (p.line !== a.line || p.col !== a.col) state.caret.dragMoved = true;
    if (state.caret.dragMoved) {
      setSelection({ anchor: a, head: p, rect: state.caret.dragRect });
    }
    setCaret(p.line, p.col);
    // Auto-scroll when dragging past the top/bottom edge.
    const rect = content.getBoundingClientRect();
    if (e.clientY < rect.top + 14) scrollVisibleRows(-2);
    else if (e.clientY > rect.bottom - 14) scrollVisibleRows(2);
    scheduleRender();
  });

  window.addEventListener("mouseup", () => {
    if (!state.caret.dragging) return;
    state.caret.dragging = false;
    state.caret.dragRect = false;
    if (!state.caret.dragMoved) setSelection(null); // plain click → just the caret
    scheduleRender();
  });

  // Double-click selects the run under the caret: a word, or (on symbols /
  // whitespace) the contiguous run of the same class, editor-style.
  content.addEventListener("dblclick", (e) => {
    e.preventDefault();
    const p = coordsFromEvent(e);
    const cs = lineChars(p.line);
    if (cs.length === 0) return;
    const classOf = (ch) => {
      if (ch == null) return null;
      if (/[\p{L}\p{N}_]/u.test(ch)) return "word";
      if (/\s/.test(ch)) return "space";
      return "punct";
    };
    // Prefer the char at the caret, else the one before it (click at run end).
    const pivot = cs[p.col] != null ? p.col : p.col - 1;
    const cls = classOf(cs[pivot]);
    if (cls == null) return;
    let a = pivot,
      b = pivot + 1;
    while (a > 0 && classOf(cs[a - 1]) === cls) a--;
    while (b < cs.length && classOf(cs[b]) === cls) b++;
    setSelection({ anchor: { line: p.line, col: a }, head: { line: p.line, col: b } });
    setCaret(p.line, b);
    focusEditor();
  });
}

// Select one whole line; the newline is included by anchoring the head at the
// start of the next line (matches VS Code's triple-click).
export function selectLineAt(line) {
  if (state.view.total === 0) return;
  const l = Math.max(0, Math.min(line, state.view.total - 1));
  const hasNext = l + 1 < state.view.total;
  const head = hasNext ? { line: l + 1, col: 0 } : { line: l, col: lineLen(l) };
  setSelection({ anchor: { line: l, col: 0 }, head });
  setCaret(head.line, head.col);
  focusEditor();
  scheduleRender();
}

export function posInsideSelection(p) {
  const rr = rectRange();
  if (rr) return p.line >= rr.l0 && p.line <= rr.l1 && p.col >= rr.c0 && p.col <= rr.c1;
  return selectionRanges().some((r) => {
    if (p.line < r.start.line || p.line > r.end.line) return false;
    if (p.line === r.start.line && p.col < r.start.col) return false;
    if (p.line === r.end.line && p.col > r.end.col) return false;
    return true;
  });
}

export async function pasteFromClipboard() {
  try {
    const text = await navigator.clipboard.readText();
    if (text) pasteText(text);
  } catch {
    // Clipboard read needs a permission some webviews withhold; the keyboard
    // path (paste event on the hidden textarea) always works.
    flashCount(t("editor.pasteBlocked"), "error");
  }
  focusEditor();
}

// Save the selected lines to a file server-side: streamed in batches, so the
// clipboard cap does not apply. Output matches what copy would produce.
export async function saveSelectionToFile() {
  const rr = rectRange();
  const ranges = selectionRanges();
  const r = selRange() || (ranges.length === 1 ? ranges[0] : null);
  if ((!rr && !r) || !hasTextSelection()) {
    // A zero-width rect selects no characters — nothing to write out.
    flashCount(t("editor.noSelection"), "error");
    return;
  }
  if (!rr && ranges.length > 1) {
    flashCount(t("editor.multiSelUseCopy"), "error");
    return;
  }
  const total = rr ? rr.l1 - rr.l0 + 1 : r.end.line - r.start.line + 1;
  const base = state.doc.stat?.path || "selection";
  const f = await askForm<{ path: string }>(
    t("dialog.saveSel.title"),
    [
      { id: "path", type: "text", label: t("dialog.saveSel.path"), value: `${base}.selection.txt` },
      {
        id: "_hint",
        type: "hint",
        label: t("dialog.saveSel.hint", { lines: commas(total), max: commas(MAX_COPY_LINES) }),
      },
    ],
    t("menu.save"),
  );
  if (!f || !f.path.trim()) return;
  const body: SelectionSaveRequest = rr
    ? {
        path: f.path.trim(),
        overwrite: false,
        rect: true,
        l0: rr.l0,
        c0: rr.c0,
        l1: rr.l1,
        c1: rr.c1,
      }
    : {
        path: f.path.trim(),
        overwrite: false,
        rect: false,
        l0: r.start.line,
        c0: r.start.col,
        l1: r.end.line,
        c1: r.end.col,
      };
  try {
    const res = await withOverwriteRetry(f.path.trim(), async (overwrite) => {
      showLoading(t("dialog.saveSel.writing"));
      try {
        return await apiPost<SelectionSaveResponse, SelectionSaveRequest>("/api/selection/save", {
          ...body,
          overwrite,
        });
      } finally {
        hideLoading();
      }
    });
    if (!res) return;
    flashCount(t("dialog.saveSel.done", { lines: commas(res.lines), path: displayPath(res.path) }));
  } catch (e) {
    flashCount(t("dialog.saveSel.error"), "error");
    showMessage(t("dialog.saveSel.error"), serverMessage(e));
  }
}

export async function selectAll() {
  if (state.view.total === 0) return;
  const last = state.view.total - 1;
  // lineLen() guesses 0 for lines outside the cache window; a guessed head of
  // (last, 0) would make "select all → delete/type" silently spare the last
  // line's text. Resolve the real end-of-document column first, and select
  // nothing rather than almost-everything if it cannot be resolved.
  const len = (await lineLensFor([last])).get(last);
  if (len == null) return;
  setSelection({
    anchor: { line: 0, col: 0 },
    head: { line: last, col: len },
  });
  setCaret(last, len, len);
  focusEditor();
  scheduleRender();
}

// Ctrl+End: the caret target is the end of the (usually uncached) last line —
// resolve its real length like selectAll does, never lineLen()'s guessed 0.
export async function caretToDocEnd(extend) {
  if (state.view.total === 0) return;
  const last = state.view.total - 1;
  const len = (await lineLensFor([last])).get(last);
  if (len == null) return;
  moveCaret(last, len, extend, len);
  state.caret.goalCol = state.caret.position.col;
}

// Fetch the selected text (bounded) and join with newlines.
export async function selectedText(r = null) {
  const rr = rectRange();
  if (rr) {
    const count = Math.min(rr.l1 - rr.l0 + 1, MAX_COPY_LINES);
    const res = await api<LinesResponse>(`/api/lines?start=${rr.l0}&count=${count}`);
    return res.lines
      .map((x) => {
        const chars = Array.from(x.text ?? "");
        return chars.slice(rr.c0, rr.c1).join("");
      })
      .join("\n");
  }
  const ranges = r ? [r] : selectionRanges();
  const out = [];
  let remaining = MAX_COPY_LINES;
  for (const range of ranges) {
    if (remaining <= 0) break;
    out.push(await selectedTextForRange(range, remaining));
    remaining -= Math.min(rangeLineCount(range), remaining);
  }
  return out.join("\n");
}

export async function copyToClipboard(text) {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    // Fallback for webviews without the async clipboard API.
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.style.cssText = "position:fixed;opacity:0;";
    document.body.append(ta);
    ta.select();
    try {
      document.execCommand("copy");
    } catch {
      /* give up silently */
    }
    ta.remove();
  }
}

export async function copySelection() {
  if (!hasTextSelection()) return;
  try {
    const total = selectionLineCount();
    await copyToClipboard(await selectedText());
    if (total > MAX_COPY_LINES) {
      const multi = selectionRanges().length > 1;
      const vars = { max: commas(MAX_COPY_LINES), rest: commas(total - MAX_COPY_LINES) };
      flashCount(multi ? t("editor.copyCapped", vars) : t("editor.copyCappedHint", vars), "error");
    } else {
      flashCount(t("editor.copied"));
    }
  } catch (e) {
    flashCount(t("editor.copyError"), "error");
    console.error(e);
  }
}

export function deleteSelection() {
  if (!hasSelection()) return;
  typeText(""); // replace the selection with nothing
}

export async function cutSelection() {
  if (!hasTextSelection()) return;
  // Never delete more than what reached the clipboard: a capped copy followed
  // by a full delete would silently destroy data.
  const total = selectionLineCount();
  if (total > MAX_COPY_LINES) {
    const multi = selectionRanges().length > 1;
    const vars = { max: commas(MAX_COPY_LINES), total: commas(total) };
    flashCount(multi ? t("editor.cutCapped", vars) : t("editor.cutCappedHint", vars), "error");
    return;
  }
  await copyToClipboard(await selectedText());
  deleteSelection();
}

// ---- multi-cursor -----------------------------------------------------------

export function clearExtraCursors() {
  if (state.caret.extraCursors.length) {
    state.caret.extraCursors = [];
    scheduleRender();
  }
}

export function clearExtraSelections() {
  for (const c of state.caret.extraCursors) c.sel = null;
}

// Ctrl+Click: add a caret; clicking an existing extra caret removes it; the
// primary caret is left alone.
export function toggleExtraCursorAt(line, col) {
  if (line === state.caret.position.line && col === state.caret.position.col) return;
  const i = state.caret.extraCursors.findIndex((c) => c.line === line && c.col === col);
  if (i >= 0) state.caret.extraCursors.splice(i, 1);
  else {
    setSelection(null);
    clearExtraSelections();
    state.caret.extraCursors.push({ line, col, sel: null });
  }
  state.caret.editGeneration++; // user cursor action: an in-flight edit must not clobber it
  scheduleRender();
}

export function addExtraCursorAt(line, col) {
  if (line === state.caret.position.line && col === state.caret.position.col) return;
  if (state.caret.extraCursors.some((c) => c.line === line && c.col === col)) return;
  setSelection(null);
  clearExtraSelections();
  state.caret.extraCursors.push({ line, col, sel: null });
  expandFoldsForLine(line);
  state.caret.editGeneration++; // user cursor action: an in-flight edit must not clobber it
  // Keep the newest cursor visible, like revealCaret does for the primary.
  const vis = rowsVisible();
  const row = visibleIndexForLine(line) - visibleIndexForLine(state.view.first);
  if (row < 0) setFirst(line);
  else if (row >= vis) setFirst(logicalLineAtVisible(visibleIndexForLine(line) - vis + 1));
  focusEditor();
  scheduleRender();
}

// Ctrl+Alt+ArrowUp / ArrowDown: grow the cursor column one line beyond the
// topmost / bottommost cursor, preserving its column — clamped to the target
// line's REAL length, which may need a fetch when it is outside the cache.
export async function addCursorAbove() {
  if (!state.doc.stat?.open || state.view.total === 0) return;
  const top = allCursors()[0];
  if (top.line <= 0) return;
  const line = top.line - 1;
  const lens = await lineLensFor([line]);
  addExtraCursorAt(line, Math.min(top.col, lens.get(line) ?? 0));
}

export async function addCursorBelow() {
  if (!state.doc.stat?.open || state.view.total === 0) return;
  const cs = allCursors();
  const bottom = cs[cs.length - 1];
  if (bottom.line >= state.view.total - 1) return;
  const line = bottom.line + 1;
  const lens = await lineLensFor([line]);
  addExtraCursorAt(line, Math.min(bottom.col, lens.get(line) ?? 0));
}
