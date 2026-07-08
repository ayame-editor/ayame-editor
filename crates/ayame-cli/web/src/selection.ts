// Ayame Editor — selection module. Type-stripped to JS at build time (build.rs, oxc).
import { $, commas, displayPath } from "./dom.js";
import { LINE_HEIGHT, MAX_COPY_LINES, OVERSCAN, state } from "./state.js";
import { isExistsError, serverMessage, t } from "./i18n.js";
import { api, apiPost, type LinesResponse } from "./api.js";
import {
  caretX,
  charWidth,
  coordsFromEvent,
  focusEditor,
  lineChars,
  lineLen,
  moveCaret,
  rowsVisible,
  scheduleRender,
  setCaret,
  setFirst,
} from "./editor.js";
import { lineLensFor, pasteText, typeText } from "./edits.js";
import { flashCount } from "./search.js";
import { askConfirm, askForm, hideLoading, showLoading, showMessage } from "./dialogs.js";
import type { SelectionSaveRequest, SelectionSaveResponse } from "./types/api.js";

// Normalized selection: { start, end } with start <= end, or null.
export function selRange() {
  if (!state.sel) return null;
  const { anchor: a, head: h } = state.sel;
  const forward = a.line < h.line || (a.line === h.line && a.col <= h.col);
  const r: any = forward ? { start: a, end: h } : { start: h, end: a };
  r.rect = !!state.sel.rect;
  return r;
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

export function hasSelection() {
  const rr = rectRange();
  if (rr) return rr.l0 !== rr.l1 || rr.c0 !== rr.c1;
  const r = selRange();
  return (!!r && !rangeEmpty(r)) || selectionRanges().length > 0;
}

// Like hasSelection(), but a zero-width rect (c0 == c1 across several lines)
// counts as empty: it selects no characters, so text-producing actions
// (copy / cut / save-selection) treat it as "no selection".
export function hasTextSelection() {
  const rr = rectRange();
  if (rr) return rr.c0 !== rr.c1;
  return selectionRanges().length > 0;
}

export function appendSelectionRect(layer, line, startCol, endCol, trailingNewline = false) {
  const left = caretX(line, startCol);
  const trail = trailingNewline ? charWidth() * 0.6 : 0;
  const width = caretX(line, endCol) - left + trail;
  const rect = document.createElement("div");
  rect.className = "selrect";
  rect.style.left = `${left}px`;
  rect.style.top = `${(line - state.first) * LINE_HEIGHT}px`;
  rect.style.width = `${Math.max(2, width)}px`;
  layer.append(rect);
}

export function renderRangeSelection(layer, r) {
  if (!r || rangeEmpty(r)) return;
  const vis = rowsVisible() + OVERSCAN;
  const from = Math.max(r.start.line, state.first);
  const to = Math.min(r.end.line, state.first + vis);
  for (let line = from; line <= to; line++) {
    const startCol = line === r.start.line ? r.start.col : 0;
    const len = lineLen(line);
    // A line selected through its end extends a hair past the text so the
    // trailing newline reads as selected, like a normal editor.
    const endCol = line === r.end.line ? Math.min(r.end.col, len) : len;
    appendSelectionRect(layer, line, startCol, endCol, line !== r.end.line);
  }
}

export function renderSelection() {
  const layer = $("sel-layer");
  layer.textContent = "";
  const rr = rectRange();
  if (rr) {
    const vis = rowsVisible() + OVERSCAN;
    const from = Math.max(rr.l0, state.first);
    const to = Math.min(rr.l1, state.first + vis);
    for (let line = from; line <= to; line++) {
      appendSelectionRect(layer, line, rr.c0, rr.c1);
    }
    return;
  }
  for (const r of selectionRanges()) renderRangeSelection(layer, r);
}

export function initSelection() {
  const content = $("content");
  content.addEventListener("mousedown", (e) => {
    if (e.button !== 0) return;
    e.preventDefault(); // keep focus on the hidden input, not the div
    const p = coordsFromEvent(e);
    if ((e.ctrlKey || e.metaKey) && !e.shiftKey && !e.altKey) {
      // Ctrl+Click (Cmd+Click on mac): toggle an extra cursor at the point.
      if (state.stat?.open) toggleExtraCursorAt(p.line, p.col);
      focusEditor();
      return;
    }
    if (e.detail >= 3) {
      // Triple-click: select the whole line (newline included when possible).
      selectLineAt(p.line);
      return;
    }
    if (e.shiftKey) {
      const anchor = state.sel ? state.sel.anchor : { ...state.caret };
      state.sel = { anchor, head: p, rect: e.altKey };
      state.dragAnchor = anchor;
      state.dragMoved = true;
    } else {
      state.sel = null; // a bare click collapses any selection to a caret
      state.dragAnchor = p;
      state.dragMoved = false;
    }
    state.dragRect = e.altKey;
    setCaret(p.line, p.col);
    state.dragging = true;
    focusEditor();
  });

  window.addEventListener("mousemove", (e) => {
    if (!state.dragging) return;
    const p = coordsFromEvent(e);
    const a = state.dragAnchor;
    if (p.line !== a.line || p.col !== a.col) state.dragMoved = true;
    if (state.dragMoved) state.sel = { anchor: a, head: p, rect: state.dragRect };
    setCaret(p.line, p.col);
    // Auto-scroll when dragging past the top/bottom edge.
    const rect = content.getBoundingClientRect();
    if (e.clientY < rect.top + 14) setFirst(state.first - 2);
    else if (e.clientY > rect.bottom - 14) setFirst(state.first + 2);
    scheduleRender();
  });

  window.addEventListener("mouseup", () => {
    if (!state.dragging) return;
    state.dragging = false;
    state.dragRect = false;
    if (!state.dragMoved) state.sel = null; // plain click → just the caret
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
    state.sel = { anchor: { line: p.line, col: a }, head: { line: p.line, col: b } };
    setCaret(p.line, b);
    focusEditor();
  });
}

// Select one whole line; the newline is included by anchoring the head at the
// start of the next line (matches VS Code's triple-click).
export function selectLineAt(line) {
  if (state.total === 0) return;
  const l = Math.max(0, Math.min(line, state.total - 1));
  const hasNext = l + 1 < state.total;
  const head = hasNext ? { line: l + 1, col: 0 } : { line: l, col: lineLen(l) };
  state.sel = { anchor: { line: l, col: 0 }, head };
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
  const base = state.stat?.path || "selection";
  const f = await askForm(
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
  showLoading(t("dialog.saveSel.writing"));
  try {
    const res = await apiPost<SelectionSaveResponse, SelectionSaveRequest>(
      "/api/selection/save",
      body,
    );
    flashCount(t("dialog.saveSel.done", { lines: commas(res.lines), path: displayPath(res.path) }));
  } catch (e) {
    hideLoading();
    // Server-boundary string match: the save endpoint reports an existing
    // target as a Japanese message (no error codes yet).
    if (isExistsError(e)) {
      const overwrite = await askConfirm(
        t("dialog.overwrite.title"),
        t("dialog.overwrite.ask", { name: displayPath(f.path.trim()) }),
        { okLabel: t("dialog.overwrite.ok"), danger: true },
      );
      if (overwrite) {
        showLoading(t("dialog.saveSel.writing"));
        try {
          const res = await apiPost<SelectionSaveResponse, SelectionSaveRequest>(
            "/api/selection/save",
            { ...body, overwrite: true },
          );
          flashCount(
            t("dialog.saveSel.done", { lines: commas(res.lines), path: displayPath(res.path) }),
          );
        } catch (e2) {
          flashCount(t("dialog.saveSel.error"), "error");
          showMessage(t("dialog.saveSel.error"), serverMessage(e2));
        }
      }
    } else {
      flashCount(t("dialog.saveSel.error"), "error");
      showMessage(t("dialog.saveSel.error"), serverMessage(e));
    }
  } finally {
    hideLoading();
  }
}

export async function selectAll() {
  if (state.total === 0) return;
  const last = state.total - 1;
  // lineLen() guesses 0 for lines outside the cache window; a guessed head of
  // (last, 0) would make "select all → delete/type" silently spare the last
  // line's text. Resolve the real end-of-document column first, and select
  // nothing rather than almost-everything if it cannot be resolved.
  const len = (await lineLensFor([last])).get(last);
  if (len == null) return;
  state.sel = {
    anchor: { line: 0, col: 0 },
    head: { line: last, col: len },
  };
  setCaret(last, len, len);
  focusEditor();
  scheduleRender();
}

// Ctrl+End: the caret target is the end of the (usually uncached) last line —
// resolve its real length like selectAll does, never lineLen()'s guessed 0.
export async function caretToDocEnd(extend) {
  if (state.total === 0) return;
  const last = state.total - 1;
  const len = (await lineLensFor([last])).get(last);
  if (len == null) return;
  moveCaret(last, len, extend, len);
  state.goalCol = state.caret.col;
}

export function rangeLineCount(r) {
  return r.end.line - r.start.line + 1;
}

export function selectionLineCount(r = null) {
  const rr = rectRange();
  if (rr) return rr.l1 - rr.l0 + 1;
  if (r) return rangeLineCount(r);
  return selectionRanges().reduce((n, range) => n + rangeLineCount(range), 0);
}

export async function selectedTextForRange(r, maxLines = MAX_COPY_LINES) {
  const count = Math.min(rangeLineCount(r), maxLines);
  const res = await api<LinesResponse>(`/api/lines?start=${r.start.line}&count=${count}`);
  // Columns are Unicode scalar counts (the server contract); slicing UTF-16
  // units here would split surrogate pairs (emoji etc.).
  const L = res.lines.map((x) => Array.from(x.text ?? ""));
  if (!L.length) return "";
  const complete = count >= rangeLineCount(r);
  if (L.length === 1) {
    const endCol = complete && r.start.line === r.end.line ? r.end.col : L[0].length;
    return L[0].slice(r.start.col, endCol).join("");
  }
  const out = [L[0].slice(r.start.col).join("")];
  for (let i = 1; i < L.length - 1; i++) out.push(L[i].join(""));
  if (L.length > 1) {
    const last = L[L.length - 1];
    out.push(last.slice(0, complete ? r.end.col : last.length).join(""));
  }
  return out.join("\n");
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

export function clonePoint(p) {
  return { line: p.line, col: p.col };
}

export function cloneSelection(sel) {
  return sel
    ? { anchor: clonePoint(sel.anchor), head: clonePoint(sel.head), rect: !!sel.rect }
    : null;
}

export function normalizedRange(anchor, head) {
  const forward = anchor.line < head.line || (anchor.line === head.line && anchor.col <= head.col);
  return forward
    ? { start: clonePoint(anchor), end: clonePoint(head) }
    : { start: clonePoint(head), end: clonePoint(anchor) };
}

export function rangeEmpty(r) {
  return r.start.line === r.end.line && r.start.col === r.end.col;
}

export function rangeKey(r) {
  return `${r.start.line}:${r.start.col}:${r.end.line}:${r.end.col}`;
}

// Primary caret plus the extra cursors, deduped and in document order. The
// entry carrying `primary: true` mirrors state.caret.
export function allCursors() {
  const out = [];
  const seen = new Set();
  const push = (c, primary) => {
    const k = `${c.line}:${c.col}`;
    if (seen.has(k)) return;
    seen.add(k);
    out.push({
      line: c.line,
      col: c.col,
      primary,
      sel: primary
        ? cloneSelection(state.sel && !state.sel.rect ? state.sel : null)
        : cloneSelection(c.sel),
    });
  };
  push(state.caret, true);
  for (const c of state.extraCursors) push(c, false);
  out.sort((a, b) => a.line - b.line || a.col - b.col);
  return out;
}

export function cursorSelectionRange(c) {
  const sel = c.primary ? state.sel : c.sel;
  if (!sel || sel.rect) return null;
  const r = normalizedRange(sel.anchor, sel.head);
  return rangeEmpty(r) ? null : r;
}

export function selectionRanges() {
  const ranges = [];
  const seen = new Set();
  const add = (r, primary = false) => {
    if (!r || rangeEmpty(r)) return;
    const key = rangeKey(r);
    if (seen.has(key)) return;
    seen.add(key);
    ranges.push({ ...r, primary });
  };
  const rr = rectRange();
  if (rr) return ranges;
  add(selRange(), true);
  for (const c of state.extraCursors) add(cursorSelectionRange(c), false);
  ranges.sort((a, b) => a.start.line - b.start.line || a.start.col - b.start.col);
  return ranges;
}

export function hasCursorSelections() {
  if (state.sel && !state.sel.rect && selRange() && !rangeEmpty(selRange())) return true;
  return state.extraCursors.some((c) => {
    const r = cursorSelectionRange(c);
    return r && !rangeEmpty(r);
  });
}

export function clearExtraCursors() {
  if (state.extraCursors.length) {
    state.extraCursors = [];
    scheduleRender();
  }
}

export function clearExtraSelections() {
  for (const c of state.extraCursors) c.sel = null;
}

// Ctrl+Click: add a caret; clicking an existing extra caret removes it; the
// primary caret is left alone.
export function toggleExtraCursorAt(line, col) {
  if (line === state.caret.line && col === state.caret.col) return;
  const i = state.extraCursors.findIndex((c) => c.line === line && c.col === col);
  if (i >= 0) state.extraCursors.splice(i, 1);
  else {
    state.sel = null;
    clearExtraSelections();
    state.extraCursors.push({ line, col, sel: null });
  }
  state.editGen++; // user cursor action: an in-flight edit must not clobber it
  scheduleRender();
}

export function addExtraCursorAt(line, col) {
  if (line === state.caret.line && col === state.caret.col) return;
  if (state.extraCursors.some((c) => c.line === line && c.col === col)) return;
  state.sel = null;
  clearExtraSelections();
  state.extraCursors.push({ line, col, sel: null });
  state.editGen++; // user cursor action: an in-flight edit must not clobber it
  // Keep the newest cursor visible, like revealCaret does for the primary.
  const vis = rowsVisible();
  if (line < state.first) setFirst(line);
  else if (line >= state.first + vis) setFirst(line - vis + 1);
  focusEditor();
  scheduleRender();
}

// Ctrl+Alt+ArrowUp / ArrowDown: grow the cursor column one line beyond the
// topmost / bottommost cursor, preserving its column — clamped to the target
// line's REAL length, which may need a fetch when it is outside the cache.
export async function addCursorAbove() {
  if (!state.stat?.open || state.total === 0) return;
  const top = allCursors()[0];
  if (top.line <= 0) return;
  const line = top.line - 1;
  const lens = await lineLensFor([line]);
  addExtraCursorAt(line, Math.min(top.col, lens.get(line) ?? 0));
}

export async function addCursorBelow() {
  if (!state.stat?.open || state.total === 0) return;
  const cs = allCursors();
  const bottom = cs[cs.length - 1];
  if (bottom.line >= state.total - 1) return;
  const line = bottom.line + 1;
  const lens = await lineLensFor([line]);
  addExtraCursorAt(line, Math.min(bottom.col, lens.get(line) ?? 0));
}
