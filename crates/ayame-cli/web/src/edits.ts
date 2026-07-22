// Ayame Editor — edits module. Type-stripped to JS at build time (build.rs, oxc).
import { $, commas } from "./dom.js";
import { MAX_COPY_LINES, OVERSCAN, PAD, state } from "./state.js";
import { t } from "./i18n.js";
import { api, apiPost, type BatchEditResponse, type LinesResponse } from "./api.js";
import { refreshStat, savingCount, waitForSavingDone } from "./save.js";
import {
  cacheLineResponse,
  cachedLine,
  focusEditor,
  maxFirst,
  refreshChangeHistoryOverview,
  render,
  revealCaret,
  revealLine,
  rowsVisible,
  setCaret,
  setFirst,
} from "./editor.js";
import {
  allCursors,
  cloneSelection,
  cursorSelectionRange,
  hasCursorSelections,
  hasSelection,
  hasTextSelection,
  rangeEmpty,
  rectRange,
  selRange,
  selectedTextForRange,
  selectionLineCount,
} from "./selection.js";
import { charLenOf, flashCount } from "./search.js";
import type { ReplaceRangeRequest, ReplaceRectRequest } from "./types/api.js";

type ReplaceEditResponse = {
  stats: { total_lines: number };
  caret_line: number;
  caret_col: number;
};

// ---- the serialized edit queue --------------------------------------------

export let editChain = Promise.resolve();

export function editContext() {
  return { docGen: state.docGen };
}

export function sameEditContext(ctx) {
  return !!state.stat?.open && state.docGen === ctx.docGen;
}

export async function settleEditQueue() {
  await editChain;
}

export function enqueueEdit(fn) {
  const ctx = editContext();
  editChain = editChain
    .then(async () => {
      if (!sameEditContext(ctx)) return null;
      if (savingCount > 0) {
        flashCount(t("editor.savingWaitInput"));
        await waitForSavingDone();
        if (!sameEditContext(ctx)) return null;
      }
      const result = await fn();
      // Any edit shifts byte offsets, so a search anchor captured against the
      // pre-edit layout is stale: F3 would resume from the wrong byte (skipping
      // a shifted match or landing mid-character) and the count/ticks would lag.
      // Drop them so the next search re-anchors from the caret (issue #74).
      state.lastMatch = null;
      state.searchHits = null;
      state.searchTruncated = false;
      if (result != null) {
        void import("./analysis.js").then(({ invalidateAnalysisForEdit }) =>
          invalidateAnalysisForEdit(),
        );
      }
      return result;
    })
    .catch((e) => {
      flashCount(t("editor.editError"));
      console.error(e);
    });
  return editChain;
}

// Re-fetch the padded window around state.first into the cache in one shot, so
// the text never blinks to the "⋯" pending placeholder between keystrokes.
export async function reloadViewport() {
  const start = Math.max(0, state.first - PAD);
  const count = rowsVisible() + OVERSCAN + 2 * PAD;
  const [res] = await Promise.all([
    api<LinesResponse>(`/api/lines?start=${start}&count=${count}`),
    refreshChangeHistoryOverview().catch((error) => {
      // Never leave position-pane ticks from an older marker revision beside
      // a successfully refreshed gutter. A later viewport refresh retries.
      state.changeHistoryOverview = null;
      console.error("change-history overview fetch failed", error);
    }),
  ]);
  cacheLineResponse(start, res);
  state.loadToken++; // cancel any in-flight ensureData for the old contents
}

// ---- 末尾に追従 (tail -f) ----------------------------------------------------
//
// While following, poll /api/tail/poll on an interval. On growth the server has
// already extended the line index in place over just the appended bytes, so the
// client only re-fetches the visible window. If the viewport was sitting at the
// bottom we auto-scroll to the new bottom (follow); if the user had scrolled up
// we keep their position and just let the scrollbar grow. Following pauses while
// the session has unsaved edits (the overlay would not line up) and stops on an
// external truncation/rotation, prompting the user to reopen.

export const TAIL_POLL_MS = 1000;

// "At the bottom" = the last line sits within the current view; decides whether
// a growth should follow (auto-scroll) or merely extend the scrollbar in place.
export function tailAtBottom() {
  return state.first >= maxFirst() - 2;
}

export function updateTailUI() {
  const btn = $("st-tail");
  if (btn) btn.classList.toggle("on", state.followTail);
  const item = $("menu-toggle-tail");
  if (item) {
    item.classList.toggle("checked", state.followTail);
    item.setAttribute("aria-checked", String(state.followTail));
  }
}

export function setFollowTail(on) {
  on = !!on && !!state.stat?.open;
  const was = state.followTail;
  state.followTail = on;
  if (state.tailTimer) {
    clearInterval(state.tailTimer);
    state.tailTimer = null;
  }
  if (on) {
    state.tailTimer = setInterval(pollTail, TAIL_POLL_MS);
    setFirst(maxFirst()); // jump to the tail so following starts from the end
    flashCount(t("status.followingTail"));
    pollTail(); // don't wait a whole interval for the first check
  } else if (was) {
    flashCount(t("status.followStopped"));
  }
  updateTailUI();
}

export async function pollTail() {
  if (!state.followTail || !state.stat?.open) return;
  if (savingCount > 0) return; // never poll mid-save
  let resp;
  try {
    resp = await apiPost("/api/tail/poll");
  } catch {
    return; // transient (e.g. a racing reload); try again next tick
  }
  if (!state.followTail) return; // toggled off during the round-trip
  if (!resp.open) {
    setFollowTail(false);
    return;
  }
  if (resp.changed) {
    // Truncated / rotated / replaced under us: stop and let the user reopen.
    setFollowTail(false);
    flashCount(t("status.tailFileChanged"), "error");
    void import("./analysis.js").then(({ handleAnalysisFileChanged }) =>
      handleAnalysisFileChanged(),
    );
    return;
  }
  // resp.pending_edits: growth seen but not followed (unsaved edits) — pause
  // silently and resume once the overlay is clear. resp.grew false: nothing new.
  if (resp.pending_edits || !resp.grew) return;
  // Auto-scroll only if we were already at the bottom; otherwise adopt the new
  // total so the scrollbar grows but the user's position is left untouched.
  const stick = tailAtBottom();
  state.total = resp.lines;
  await import("./analysis.js").then(({ refreshAnalysisTail }) => refreshAnalysisTail());
  if (stick) state.first = maxFirst();
  try {
    await reloadViewport();
    await refreshStat();
  } catch {
    return;
  }
  if (!state.followTail) return;
  if (stick) state.first = maxFirst();
  render();
}

// The range the next text insertion replaces: the selection, or the caret.
export function replaceTarget() {
  const r = selRange();
  if (r) return { l0: r.start.line, c0: r.start.col, l1: r.end.line, c1: r.end.col };
  return { l0: state.caret.line, c0: state.caret.col, l1: state.caret.line, c1: state.caret.col };
}

// The one primitive every edit funnels through. The backend returns the
// authoritative post-edit caret (already column-clamped against the real
// document), so we commit it — and the new line count — to local state
// immediately, before any await that could reject. That keeps the caret/cache
// from going stale while the document advanced, and lets the next queued edit
// resolve its range against a correct caret even if the refresh below fails.
export async function applyRange(l0, c0, l1, c1, text) {
  const ctx = editContext();
  const gen = state.editGen;
  const body: ReplaceRangeRequest = { l0, c0, l1, c1, text };
  const res = await apiPost<ReplaceEditResponse, ReplaceRangeRequest>(
    "/api/edit/replace_range",
    body,
  );
  if (!sameEditContext(ctx)) return;
  state.total = res.stats.total_lines;
  if (state.editGen === gen) {
    // No user navigation happened during the round-trip: honor the edit caret.
    const line = Math.min(res.caret_line, Math.max(0, state.total - 1));
    state.sel = null;
    state.caret = { line, col: res.caret_col };
    state.activeLine = line;
    state.goalCol = res.caret_col;
  } else {
    // The user moved the caret mid-edit — keep their position, re-clamped to
    // the new line count (don't clobber it with the edit's caret).
    const line = Math.min(state.caret.line, Math.max(0, state.total - 1));
    state.caret = { line, col: state.caret.col };
    state.activeLine = line;
  }
  revealCaret(); // scroll so the caret line is covered by the reload below
  try {
    await reloadViewport();
    await refreshStat();
  } catch (e) {
    console.error("post-edit refresh failed", e);
    flashCount(t("editor.reloadError"));
  }
  if (!sameEditContext(ctx)) return;
  revealCaret();
  render();
}

export async function applyRect(l0, l1, c0, c1, text) {
  const ctx = editContext();
  const gen = state.editGen;
  const body: ReplaceRectRequest = { l0, l1, c0, c1, text };
  const res = await apiPost<ReplaceEditResponse, ReplaceRectRequest>(
    "/api/edit/replace_rect",
    body,
  );
  if (!sameEditContext(ctx)) return;
  state.total = res.stats.total_lines;
  if (state.editGen === gen) {
    const line = Math.min(res.caret_line, Math.max(0, state.total - 1));
    state.sel = null;
    state.caret = { line, col: res.caret_col };
    state.activeLine = line;
    state.goalCol = res.caret_col;
  }
  revealCaret();
  try {
    await reloadViewport();
    await refreshStat();
  } catch (e) {
    console.error("post-rect-edit refresh failed", e);
    flashCount(t("editor.reloadError"));
  }
  if (!sameEditContext(ctx)) return;
  revealCaret();
  render();
}

// Multi-cursor edits: send every cursor's replacement as ONE batch (the server
// records it as a single undo step) and adopt the returned carets. `cursors`
// is the sorted allCursors() list; `editOf[i]` is the index of cursor i's edit
// in `edits`, or -1 when the cursor contributed no edit (only possible at the
// document origin, whose position an edit batch cannot move).
export async function applyBatch(edits, cursors, editOf) {
  const ctx = editContext();
  const gen = state.editGen;
  const res = await apiPost<BatchEditResponse, { edits: unknown[] }>("/api/edit/replace_batch", {
    edits,
  });
  if (!sameEditContext(ctx)) return;
  state.total = res.stats.total_lines;
  if (state.editGen === gen) {
    const clampLine = (l) => Math.min(l, Math.max(0, state.total - 1));
    const next = cursors.map((c, i) => {
      const k = editOf[i];
      const p = k >= 0 && res.carets?.[k] ? res.carets[k] : c;
      return { line: clampLine(p.line), col: p.col, primary: c.primary };
    });
    const primary = next.find((c) => c.primary) || next[0];
    state.sel = null;
    state.caret = { line: primary.line, col: primary.col };
    state.activeLine = primary.line;
    state.goalCol = primary.col;
    state.extraCursors = next
      .filter((c) => c !== primary)
      .map((c) => ({ line: c.line, col: c.col }));
  } else {
    // The user moved the caret mid-edit: keep their position, re-clamped to
    // the new line count (don't clobber it with the edit's caret).
    const line = Math.min(state.caret.line, Math.max(0, state.total - 1));
    state.caret = { line, col: state.caret.col };
    state.activeLine = line;
    if (state.extraCursors.length) {
      // Plain caret motion / cursor-adds keep the extras alive mid-flight.
      // Remap ONLY the cursors this batch owned (matched by their batch-start
      // position) onto their post-edit positions; cursors the user added or
      // removed while the batch was in flight are left exactly as they are.
      const clampLine = (l) => Math.min(l, Math.max(0, state.total - 1));
      const moved = new Map();
      cursors.forEach((c, i) => {
        const k = editOf[i];
        const p = k >= 0 && res.carets?.[k] ? res.carets[k] : c;
        moved.set(`${c.line}:${c.col}`, { line: clampLine(p.line), col: p.col });
      });
      const seen = new Set();
      state.extraCursors = state.extraCursors.flatMap((c) => {
        const next = moved.get(`${c.line}:${c.col}`) || { line: clampLine(c.line), col: c.col };
        const key = `${next.line}:${next.col}`;
        if (seen.has(key)) return []; // two cursors landing together collapse
        seen.add(key);
        return [next];
      });
    }
  }
  revealCaret();
  try {
    await reloadViewport();
    await refreshStat();
  } catch (e) {
    console.error("post-batch-edit refresh failed", e);
    flashCount(t("editor.reloadError"));
  }
  if (!sameEditContext(ctx)) return;
  revealCaret();
  render();
}

// One same-shaped insertion per cursor. `textFor(i)` is the string inserted at
// cursor i (document order) — a constant for typing, per-line for paste.
export function multiInsert(cursors, textFor) {
  const edits = cursors.map((c, i) => ({
    l0: c.line,
    c0: c.col,
    l1: c.line,
    c1: c.col,
    text: textFor(i),
  }));
  return applyBatch(
    edits,
    cursors,
    cursors.map((_, i) => i),
  );
}

export function cursorReplaceRange(c) {
  const r = cursorSelectionRange(c);
  if (r) return { l0: r.start.line, c0: r.start.col, l1: r.end.line, c1: r.end.col };
  return { l0: c.line, c0: c.col, l1: c.line, c1: c.col };
}

export function multiReplace(cursors, textFor) {
  const edits = cursors.map((c, i) => ({ ...cursorReplaceRange(c), text: textFor(i) }));
  return applyBatch(
    edits,
    cursors,
    cursors.map((_, i) => i),
  );
}

// Insert (or replace the selection with) `text`, which may contain newlines.
// The target range is resolved *inside* the queued step, so a burst of
// keystrokes each sees the caret left by the previous edit (never a stale one).
// A 0-line document (an empty file) is editable too: replaceTarget yields the
// (0,0)..(0,0) origin range, which the backend accepts to seed the first line.
export function typeText(text) {
  if (!state.stat?.open) return;
  enqueueEdit(() => {
    if (state.extraCursors.length) {
      // Multi-cursor: the same text goes in at every caret, or replaces each
      // cursor's selection, as one undo step.
      const cursors = allCursors();
      return hasCursorSelections()
        ? multiReplace(cursors, () => text)
        : multiInsert(cursors, () => text);
    }
    const rr = rectRange();
    if (rr) {
      return applyRect(rr.l0, rr.l1, rr.c0, rr.c1, text);
    }
    const t = replaceTarget();
    return applyRange(t.l0, t.c0, t.l1, t.c1, text);
  });
}

export function insertNewline() {
  typeText("\n");
}

// Decoded length (in Unicode scalars) of each requested line, as a Map. Lines
// inside the local cache are read from it; anything else is fetched, because
// lineLen() silently reads 0 for uncached lines — and multi-cursor edits can
// reference lines far outside the viewport±PAD cache window, where a guessed 0
// would turn a delete edge into "delete the whole line". Lines whose length
// cannot be resolved are absent from the map; callers must skip those edits.
export async function lineLensFor(lineNumbers) {
  const out = new Map();
  const missing = new Set();
  for (const l of lineNumbers) {
    if (l < 0 || l >= state.total || out.has(l) || missing.has(l)) continue;
    const rec = cachedLine(l);
    if (rec != null) out.set(l, Array.from(rec.text ?? "").length);
    else missing.add(l);
  }
  await Promise.all(
    [...missing].map(async (l) => {
      try {
        const res = await api<LinesResponse>(`/api/lines?start=${l}&count=1`);
        const text = res.lines?.[0]?.text;
        if (text != null) out.set(l, Array.from(text).length);
      } catch {
        // Leave the line out: the caller drops that cursor's edit, never guesses.
      }
    }),
  );
  return out;
}

// The shared "a selection is active" arm of every delete command: remove the
// rect or range selection as one edit. Returns null when nothing is selected
// (callers then handle their caret-relative case). Call inside enqueueEdit.
export function deleteSelectionEdit() {
  if (!hasSelection()) return null;
  if (state.extraCursors.length && hasCursorSelections()) {
    return multiReplace(allCursors(), () => "");
  }
  const rr = rectRange();
  if (rr) return applyRect(rr.l0, rr.l1, rr.c0, rr.c1, "");
  const t = replaceTarget();
  return applyRange(t.l0, t.c0, t.l1, t.c1, "");
}

// A zero-width rectangular selection (c0 === c1 across several lines) is the
// normal way to start column editing, but it still reports as a selection
// (l0 !== l1). Backspace/Delete must remove one char per covered line — before
// the caret column for Backspace, after it for Delete — instead of issuing an
// empty-range rect edit that deletes nothing and drops the selection (#74).
async function deleteZeroWidthRect(rr, forward) {
  const gen = state.editGen;
  const c = rr.c0;
  const lines = [];
  for (let l = rr.l0; l <= rr.l1; l++) lines.push(l);
  const lens = await lineLensFor(lines);
  const edits = [];
  for (const l of lines) {
    if (!lens.has(l)) continue;
    const len = lens.get(l);
    if (forward) {
      if (c < len) edits.push({ l0: l, c0: c, l1: l, c1: c + 1, text: "" });
    } else if (c > 0 && c <= len) {
      edits.push({ l0: l, c0: c - 1, l1: l, c1: c, text: "" });
    }
  }
  if (!edits.length) return null; // nothing to delete on any line — no request
  const ctx = editContext();
  await apiPost("/api/edit/replace_batch", { edits });
  if (!sameEditContext(ctx)) return;
  try {
    await reloadViewport();
    await refreshStat();
  } catch (e) {
    console.error("post-rect-delete refresh failed", e);
    flashCount(t("editor.reloadError"));
  }
  if (!sameEditContext(ctx)) return;
  if (state.editGen !== gen) return;
  // Keep the column caret alive at the new column so a held Backspace/Delete
  // keeps acting on every line.
  const newCol = forward ? c : Math.max(0, c - 1);
  const l0 = Math.min(rr.l0, Math.max(0, state.total - 1));
  const l1 = Math.min(rr.l1, Math.max(0, state.total - 1));
  state.sel = { anchor: { line: l0, col: newCol }, head: { line: l1, col: newCol }, rect: true };
  state.extraCursors = [];
  setCaret(l1, newCol);
  revealCaret();
  render();
}

export function backspace() {
  enqueueEdit(async () => {
    const rr = rectRange();
    if (rr && rr.c0 === rr.c1) return deleteZeroWidthRect(rr, false);
    const del = deleteSelectionEdit();
    if (del) return del;
    if (state.extraCursors.length) {
      // Per cursor: delete one char before the caret (line-join at col 0).
      // allCursors() dedupes positions, so ranges may touch but never overlap;
      // a cursor at the document origin contributes no edit. Join edges need
      // the previous line's REAL length, which may live outside the cache.
      const cursors = allCursors();
      const lens = await lineLensFor(
        cursors.filter((c) => c.col === 0 && c.line > 0).map((c) => c.line - 1),
      );
      const edits = [];
      const editOf = cursors.map((c) => {
        if (c.col > 0) {
          edits.push({ l0: c.line, c0: c.col - 1, l1: c.line, c1: c.col, text: "" });
        } else if (c.line > 0 && lens.has(c.line - 1)) {
          edits.push({ l0: c.line - 1, c0: lens.get(c.line - 1), l1: c.line, c1: 0, text: "" });
        } else {
          return -1; // document origin, or an unresolvable line length
        }
        return edits.length - 1;
      });
      if (!edits.length) return null;
      return applyBatch(edits, cursors, editOf);
    }
    const { line, col } = state.caret;
    if (col > 0) return applyRange(line, col - 1, line, col, "");
    if (line > 0) {
      const lens = await lineLensFor([line - 1]);
      if (!lens.has(line - 1)) return null;
      return applyRange(line - 1, lens.get(line - 1), line, 0, "");
    }
    return null;
  });
}

export function forwardDelete() {
  enqueueEdit(async () => {
    const rr = rectRange();
    if (rr && rr.c0 === rr.c1) return deleteZeroWidthRect(rr, true);
    const del = deleteSelectionEdit();
    if (del) return del;
    if (state.extraCursors.length) {
      // Per cursor: delete one char after the caret (line-join at EOL). Same
      // dedupe rule as backspace; the very end of the document yields no edit.
      // The char-vs-join decision needs each cursor line's REAL length.
      const cursors = allCursors();
      const lens = await lineLensFor(cursors.map((c) => c.line));
      const edits = [];
      const editOf = cursors.map((c) => {
        if (!lens.has(c.line)) return -1; // unresolvable length: never guess 0
        if (c.col < lens.get(c.line)) {
          edits.push({ l0: c.line, c0: c.col, l1: c.line, c1: c.col + 1, text: "" });
        } else if (c.line < state.total - 1) {
          edits.push({ l0: c.line, c0: c.col, l1: c.line + 1, c1: 0, text: "" });
        } else {
          return -1;
        }
        return edits.length - 1;
      });
      if (!edits.length) return null;
      return applyBatch(edits, cursors, editOf);
    }
    const { line, col } = state.caret;
    const lens = await lineLensFor([line]);
    if (!lens.has(line)) return null;
    if (col < lens.get(line)) return applyRange(line, col, line, col + 1, "");
    if (line < state.total - 1) return applyRange(line, col, line + 1, 0, "");
    return null;
  });
}

export function pasteText(raw) {
  const text = raw.replace(/\r\n?/g, "\n");
  if (!state.extraCursors.length) {
    typeText(text);
    return;
  }
  if (!state.stat?.open) return;
  enqueueEdit(() => {
    if (!state.extraCursors.length) {
      // Collapsed while the paste was queued: normal single-caret insert.
      const t = replaceTarget();
      return applyRange(t.l0, t.c0, t.l1, t.c1, text);
    }
    // VS Code rule: N clipboard lines onto N cursors paste line i at cursor i
    // (document order); any other shape inserts the whole text at every caret.
    const cursors = allCursors();
    const lines = text.split("\n");
    const perCursor = lines.length === cursors.length ? lines : null;
    const textFor = (i) => (perCursor ? perCursor[i] : text);
    return hasCursorSelections() ? multiReplace(cursors, textFor) : multiInsert(cursors, textFor);
  });
}

export async function undoEdit() {
  enqueueEdit(async () => {
    await apiPost("/api/edit/undo", {});
    state.sel = null;
    state.extraCursors = []; // a multi-cursor batch undoes as one step

    await refreshStat();
    await reloadViewport();
    setCaret(state.caret.line, state.caret.col); // re-clamp into the new bounds
    revealCaret();
    render();
  });
}

export async function redoEdit() {
  enqueueEdit(async () => {
    await apiPost("/api/edit/redo", {});
    state.sel = null;
    state.extraCursors = []; // a multi-cursor batch redoes as one step

    await refreshStat();
    await reloadViewport();
    setCaret(state.caret.line, state.caret.col);
    revealCaret();
    render();
  });
}

// ツールメニュー「大文字に変換 / snake_case に変換 …」: split an identifier
// run into words (on _ / - and camelCase boundaries; acronyms stay together)
// and re-join in the requested style. Only ASCII identifier runs are touched —
// 日本語 text, spacing, and punctuation pass through unchanged. Mirrors
// push_converted_run in ayame-core/src/transform.rs.
export function applyCaseMode(text, mode) {
  const s = String(text);
  if (mode === "upper") return s.toUpperCase();
  if (mode === "lower") return s.toLowerCase();
  return s.replace(/[A-Za-z0-9]+(?:[_-][A-Za-z0-9]+)*/g, (run) => {
    const words = run
      .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
      .replace(/([A-Z]+)([A-Z][a-z])/g, "$1 $2")
      .split(/[\s_-]+/)
      .filter(Boolean)
      .map((w) => w.toLowerCase());
    if (!words.length) return run;
    const cap = (w) => w.charAt(0).toUpperCase() + w.slice(1);
    switch (mode) {
      case "camel":
        return words[0] + words.slice(1).map(cap).join("");
      case "pascal":
        return words.map(cap).join("");
      case "snake":
        return words.join("_");
      case "kebab":
        return words.join("-");
      case "constant":
        return words.map((w) => w.toUpperCase()).join("_");
      default:
        return run;
    }
  });
}

// ツールメニューのケース変換: transform the selection in the editor as one
// undoable edit — nothing is written to disk until 保存.
export async function transformSelection(mode) {
  if (!state.stat?.open) return;
  const fn = (s) => applyCaseMode(s, mode);
  if (!hasTextSelection()) {
    flashCount(t("editor.selectRangeFirst"), "error");
    return;
  }
  if (selectionLineCount() > MAX_COPY_LINES) {
    flashCount(t("editor.transformCapped", { max: commas(MAX_COPY_LINES) }), "error");
    return;
  }
  const rr = rectRange();
  enqueueEdit(async () => {
    if (rr) {
      const total = rr.l1 - rr.l0 + 1;
      const res = await api<LinesResponse>(`/api/lines?start=${rr.l0}&count=${total}`);
      const edits = [];
      res.lines.forEach((rec, i) => {
        const chars = Array.from(rec.text ?? "");
        const c0 = Math.min(rr.c0, chars.length);
        const c1 = Math.min(rr.c1, chars.length);
        const piece = chars.slice(c0, c1).join("");
        const next = fn(piece);
        if (c1 > c0 && next !== piece) {
          edits.push({ l0: rr.l0 + i, c0, l1: rr.l0 + i, c1, text: next });
        }
      });
      return edits.length ? applyBatchPlain(edits) : null;
    }
    // Normal / multi-cursor selections: one edit per cursor, one undo step.
    const cursors = allCursors();
    const texts = [];
    for (const c of cursors) {
      const r = cursorSelectionRange(c);
      texts.push(r ? fn(await selectedTextForRange(r)) : null);
    }
    const edits = [];
    const editOf = cursors.map((c, i) => {
      if (texts[i] == null) return -1;
      edits.push({ ...cursorReplaceRange(c), text: texts[i] });
      return edits.length - 1;
    });
    if (!edits.length) return null;
    return applyBatch(edits, cursors, editOf);
  });
}

// Apply a prepared edit batch that is not tied to the cursors (rect case
// transform, replace-all) and refresh the view around the existing caret.
export async function applyBatchPlain(edits) {
  const ctx = editContext();
  await apiPost("/api/edit/replace_batch", { edits });
  if (!sameEditContext(ctx)) return;
  state.sel = null;
  state.extraCursors = [];
  try {
    await reloadViewport();
    await refreshStat();
  } catch (e) {
    console.error("post-batch refresh failed", e);
    flashCount(t("editor.reloadError"));
  }
  if (!sameEditContext(ctx)) return;
  setCaret(Math.min(state.caret.line, Math.max(0, state.total - 1)), state.caret.col);
  revealCaret();
  render();
}

// ---- whole-line operations (行を複製 / 移動 / 削除) -------------------------
// Each command edits whole lines as ONE batch edit (a single undo step) built
// against the pre-edit view, then commits through applyLineEdit so the caret —
// and, for a selection, the covered block — follows the lines to their new home.
// A multi-line selection acts on every line it covers; multi-cursor collapses to
// the primary caret, like the word-delete commands.

// The whole-line span a line op acts on: the lines the selection covers, or the
// caret's line when there is no selection. A selection ending exactly at column
// 0 of a line does not pull that trailing line in.
export function lineOpSpan() {
  const r = selRange();
  if (r && !rangeEmpty(r)) {
    let l0 = r.start.line;
    let l1 = r.end.line;
    if (!r.rect && l1 > l0 && r.end.col === 0) l1 -= 1;
    return { l0, l1 };
  }
  return { l0: state.caret.line, l1: state.caret.line };
}

// Decoded text of one line — from the cache when resident, else a targeted
// fetch (a selection can reach past the cached window).
export async function oneLineText(line) {
  const c = cachedLine(line);
  if (c != null) return c.text ?? "";
  const res = await api<LinesResponse>(`/api/lines?start=${line}&count=1`);
  return res.lines[0]?.text ?? "";
}

export async function lineLenAt(line) {
  return charLenOf(await oneLineText(line));
}

// Decoded text of the lines [start, start+count) as a plain string array.
export async function lineTextsFor(start, count) {
  const res = await api<LinesResponse>(`/api/lines?start=${start}&count=${count}`);
  return res.lines.map((r) => r.text ?? "");
}

// Shift a (non-rect) selection's endpoints by `delta` whole lines so it keeps
// hugging the block it covered after a move.
export function shiftLineSelection(sel, delta) {
  if (!sel) return null;
  return {
    anchor: { line: sel.anchor.line + delta, col: sel.anchor.col },
    head: { line: sel.head.line + delta, col: sel.head.col },
  };
}

// Commit a line op's batch edit, refresh the view, then place the caret and
// (optionally) restore the selection. Mirrors applyBatchPlain but positions the
// caret/selection deliberately instead of collapsing to the pre-edit caret.
export async function applyLineEdit(edits, caret, sel) {
  const ctx = editContext();
  await apiPost("/api/edit/replace_batch", { edits });
  if (!sameEditContext(ctx)) return;
  state.extraCursors = []; // line ops are single-caret
  try {
    await reloadViewport();
    await refreshStat();
  } catch (e) {
    console.error("post-line-edit refresh failed", e);
    flashCount(t("editor.reloadError"));
  }
  if (!sameEditContext(ctx)) return;
  const last = Math.max(0, state.total - 1);
  const place = (p) => {
    const line = Math.min(Math.max(0, p.line), last);
    const cached = cachedLine(line);
    const col = cached ? Math.min(p.col, charLenOf(cached.text ?? "")) : Math.max(0, p.col);
    return { line, col };
  };
  const c = place(caret);
  state.caret = c;
  state.activeLine = c.line;
  state.goalCol = c.col;
  state.sel = sel ? { anchor: place(sel.anchor), head: place(sel.head) } : null;
  revealCaret();
  render();
}

// 行を複製: duplicate the covered line block just below itself.
export function duplicateLines() {
  if (!state.stat?.open || state.total === 0) return;
  enqueueEdit(async () => {
    const { l0, l1 } = lineOpSpan();
    if (l1 - l0 + 1 > MAX_COPY_LINES) {
      flashCount(t("editor.duplicateCapped", { max: commas(MAX_COPY_LINES) }), "error");
      return null;
    }
    const caret = { ...state.caret }; // the copy lands below; the caret stays put
    const sel = cloneSelection(state.sel && !state.sel.rect ? state.sel : null);
    const texts = await lineTextsFor(l0, l1 - l0 + 1);
    const endCol = charLenOf(texts[texts.length - 1]);
    const edit = { l0: l1, c0: endCol, l1, c1: endCol, text: "\n" + texts.join("\n") };
    return applyLineEdit([edit], caret, sel);
  });
}

// 行を上へ / 下へ移動: swap the covered block with its neighbouring line.
export function moveLines(dir) {
  if (!state.stat?.open || state.total === 0) return;
  enqueueEdit(async () => {
    const { l0, l1 } = lineOpSpan();
    if (dir < 0 ? l0 === 0 : l1 >= state.total - 1) return null; // already at the edge
    if (l1 - l0 + 1 > MAX_COPY_LINES) {
      flashCount(t("editor.moveCapped", { max: commas(MAX_COPY_LINES) }), "error");
      return null;
    }
    const caret = { line: state.caret.line + dir, col: state.caret.col };
    const sel = shiftLineSelection(
      cloneSelection(state.sel && !state.sel.rect ? state.sel : null),
      dir,
    );
    const block = await lineTextsFor(l0, l1 - l0 + 1);
    if (dir < 0) {
      // Up: line (l0-1) drops below the block.
      const above = await oneLineText(l0 - 1);
      const edit = {
        l0: l0 - 1,
        c0: 0,
        l1,
        c1: charLenOf(block[block.length - 1]),
        text: block.join("\n") + "\n" + above,
      };
      return applyLineEdit([edit], caret, sel);
    }
    // Down: line (l1+1) rises above the block.
    const below = await oneLineText(l1 + 1);
    const edit = {
      l0,
      c0: 0,
      l1: l1 + 1,
      c1: charLenOf(below),
      text: below + "\n" + block.join("\n"),
    };
    return applyLineEdit([edit], caret, sel);
  });
}

// 行を削除: drop the covered line block entirely.
export function deleteLines() {
  if (!state.stat?.open || state.total === 0) return;
  enqueueEdit(async () => {
    const { l0, l1 } = lineOpSpan();
    let edit;
    let caret;
    if (l1 < state.total - 1) {
      // Lines survive below: drop the block; the next line slides up to l0.
      edit = { l0, c0: 0, l1: l1 + 1, c1: 0, text: "" };
      caret = { line: l0, col: 0 };
    } else if (l0 === 0) {
      // The whole document: collapse to a single empty line.
      edit = { l0: 0, c0: 0, l1, c1: await lineLenAt(l1), text: "" };
      caret = { line: 0, col: 0 };
    } else {
      // The block runs to EOF: fold it into the previous line's tail.
      const prevLen = await lineLenAt(l0 - 1);
      edit = { l0: l0 - 1, c0: prevLen, l1, c1: await lineLenAt(l1), text: "" };
      caret = { line: l0 - 1, col: prevLen };
    }
    return applyLineEdit([edit], caret, null);
  });
}

// Jump the caret to a 1-based line number.
export function gotoLine(n) {
  const v = parseInt(String(n).replace(/[^0-9]/g, ""), 10);
  if (!Number.isFinite(v) || v < 1) return;
  const line = Math.min(v - 1, Math.max(0, state.total - 1));
  state.sel = null;
  setCaret(line, 0);
  revealLine(line);
  focusEditor();
}
