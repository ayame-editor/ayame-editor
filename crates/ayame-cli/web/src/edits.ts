// Ayame Editor — edits module. Type-stripped to JS at build time (build.rs, oxc).
import { $, commas } from "./dom.js";
import { MAX_COPY_LINES, OVERSCAN, PAD, state } from "./state.js";
import { t } from "./i18n.js";
import { api, apiPost, type BatchEditResponse, type LinesResponse } from "./api.js";
import {
  cacheLineResponse,
  cachedLine,
  focusEditor,
  loadSparseFoldedData,
  maxFirst,
  refreshChangeHistoryOverview,
  render,
  revealCaret,
  revealLine,
  rowsVisible,
  setActiveLine,
  setCaret,
  setFirst,
  setSearchHits,
  setSelection,
} from "./editor.js";
import { activeFoldMap, clearActiveFoldsForEdit } from "./fold-state.js";
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
  selectionLineCount,
} from "./selection-model.js";
import { selectedTextForRange } from "./selection-text.js";
import { flashCount } from "./notifications.js";
import { charLenOf } from "./text.js";
import type { ReplaceRangeRequest, ReplaceRectRequest, TailPollResponse } from "./types/api.js";
import {
  emptyPairRange,
  isPairCloser,
  newlineIndent,
  pairCloser,
  shouldAutoClose,
  shouldSkipCloser,
} from "./input-assist.js";
import { resolveSyntaxScheme, schemeDefinition } from "./syntax.js";

type ReplaceEditResponse = {
  stats: { total_lines: number };
  caret_line: number;
  caret_col: number;
};

// ---- the serialized edit queue --------------------------------------------

export let editChain = Promise.resolve();
let analysisService = {
  invalidateForEdit: () => Promise.resolve(),
  handleFileChanged: () => Promise.resolve(),
  refreshTail: () => Promise.resolve(),
};
let refreshStat = () => Promise.resolve();
let savingCount = () => 0;
let waitForSavingDone = () => Promise.resolve();

export function setEditAnalysisService(service) {
  analysisService = service;
}

export function setEditSaveService(service) {
  refreshStat = service.refreshStat;
  savingCount = service.savingCount;
  waitForSavingDone = service.waitForSavingDone;
}

export function editContext() {
  return { docGen: state.doc.generation };
}

export function sameEditContext(ctx) {
  return !!state.doc.stat?.open && state.doc.generation === ctx.docGen;
}

export async function settleEditQueue() {
  await editChain;
}

export function enqueueEdit(fn) {
  const ctx = editContext();
  editChain = editChain
    .then(async () => {
      if (!sameEditContext(ctx)) return null;
      clearActiveFoldsForEdit();
      if (savingCount() > 0) {
        flashCount(t("editor.savingWaitInput"));
        await waitForSavingDone();
        if (!sameEditContext(ctx)) return null;
      }
      const result = await fn();
      // Any edit shifts byte offsets, so a search anchor captured against the
      // pre-edit layout is stale: F3 would resume from the wrong byte (skipping
      // a shifted match or landing mid-character) and the count/ticks would lag.
      // Drop them so the next search re-anchors from the caret (issue #74).
      state.search.lastMatch = null;
      setSearchHits(null);
      state.search.truncated = false;
      if (result != null) {
        void analysisService.invalidateForEdit();
      }
      return result;
    })
    .catch((e) => {
      flashCount(t("editor.editError"));
      console.error(e);
    });
  return editChain;
}

// Re-fetch the padded window around state.view.first into the cache in one shot, so
// the text never blinks to the "⋯" pending placeholder between keystrokes.
export async function reloadViewport() {
  if (activeFoldMap().size) {
    await Promise.all([
      loadSparseFoldedData(state.view.first, rowsVisible() + OVERSCAN, true),
      refreshChangeHistoryOverview().catch((error) => {
        state.markers.changeHistoryOverview = null;
        console.error("change-history overview fetch failed", error);
      }),
    ]);
    return;
  }
  const start = Math.max(0, state.view.first - PAD);
  const count = rowsVisible() + OVERSCAN + 2 * PAD;
  const [res] = await Promise.all([
    api<LinesResponse>(`/api/lines?start=${start}&count=${count}`),
    refreshChangeHistoryOverview().catch((error) => {
      // Never leave position-pane ticks from an older marker revision beside
      // a successfully refreshed gutter. A later viewport refresh retries.
      state.markers.changeHistoryOverview = null;
      console.error("change-history overview fetch failed", error);
    }),
  ]);
  cacheLineResponse(start, res);
  state.view.loadToken++; // cancel any in-flight ensureData for the old contents
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
  return state.view.first >= maxFirst() - 2;
}

export function updateTailUI() {
  const btn = $("st-tail");
  if (btn) btn.classList.toggle("on", state.doc.followTail);
  const item = $("menu-toggle-tail");
  if (item) {
    item.classList.toggle("checked", state.doc.followTail);
    item.setAttribute("aria-checked", String(state.doc.followTail));
  }
}

export function setFollowTail(on) {
  on = !!on && !!state.doc.stat?.open;
  const was = state.doc.followTail;
  state.doc.followTail = on;
  if (state.doc.tailTimer) {
    clearInterval(state.doc.tailTimer);
    state.doc.tailTimer = null;
  }
  if (on) {
    state.doc.tailTimer = setInterval(pollTail, TAIL_POLL_MS);
    setFirst(maxFirst()); // jump to the tail so following starts from the end
    flashCount(t("status.followingTail"));
    pollTail(); // don't wait a whole interval for the first check
  } else if (was) {
    flashCount(t("status.followStopped"));
  }
  updateTailUI();
}

export async function pollTail() {
  if (!state.doc.followTail || !state.doc.stat?.open) return;
  if (savingCount() > 0) return; // never poll mid-save
  let resp;
  try {
    resp = await apiPost<TailPollResponse>("/api/tail/poll");
  } catch {
    return; // transient (e.g. a racing reload); try again next tick
  }
  if (!state.doc.followTail) return; // toggled off during the round-trip
  if (!resp.open) {
    setFollowTail(false);
    return;
  }
  if (resp.changed) {
    // Truncated / rotated / replaced under us: stop and let the user reopen.
    setFollowTail(false);
    flashCount(t("status.tailFileChanged"), "error");
    void analysisService.handleFileChanged();
    return;
  }
  // resp.pending_edits: growth seen but not followed (unsaved edits) — pause
  // silently and resume once the overlay is clear. resp.grew false: nothing new.
  if (resp.pending_edits || !resp.grew) return;
  // Auto-scroll only if we were already at the bottom; otherwise adopt the new
  // total so the scrollbar grows but the user's position is left untouched.
  const stick = tailAtBottom();
  state.view.total = resp.lines;
  await analysisService.refreshTail();
  if (stick) state.view.first = maxFirst();
  try {
    await reloadViewport();
    await refreshStat();
  } catch {
    return;
  }
  if (!state.doc.followTail) return;
  if (stick) state.view.first = maxFirst();
  render();
}

// The range the next text insertion replaces: the selection, or the caret.
export function replaceTarget() {
  const r = selRange();
  if (r) return { l0: r.start.line, c0: r.start.col, l1: r.end.line, c1: r.end.col };
  return {
    l0: state.caret.position.line,
    c0: state.caret.position.col,
    l1: state.caret.position.line,
    c1: state.caret.position.col,
  };
}

type EditCommitResponse = {
  stats?: { total_lines: number };
};

type EditCommitOptions<T> = {
  editGeneration: number;
  applyCurrent?: (response: T) => void;
  applyStale?: (response: T) => void;
  afterRefresh?: (response: T) => void;
};

// Commit the common tail of every edit response. The document-generation
// check, authoritative line count, caret-generation guard, refresh error
// handling, reveal, and render order live here so a new edit path cannot omit
// one of them (#124).
async function commitEdit<T extends EditCommitResponse>(
  ctx,
  response: T,
  opts: EditCommitOptions<T>,
) {
  if (!sameEditContext(ctx)) return false;
  if (response.stats) state.view.total = response.stats.total_lines;

  const current = state.caret.editGeneration === opts.editGeneration;
  if (current) {
    opts.applyCurrent?.(response);
  } else if (opts.applyStale) {
    opts.applyStale(response);
  } else {
    // User navigation won the race: preserve it, only clamping the line to the
    // authoritative document length returned by the edit.
    const line = Math.min(state.caret.position.line, Math.max(0, state.view.total - 1));
    state.caret.position = { line, col: state.caret.position.col };
    setActiveLine(line);
  }

  revealCaret(); // ensure the reload below covers the chosen caret line
  try {
    await reloadViewport();
    await refreshStat();
  } catch (error) {
    console.error("post-edit refresh failed", error);
    flashCount(t("editor.reloadError"));
  }
  if (!sameEditContext(ctx)) return false;
  // A navigation that happened during refresh must also win over the edit's
  // deferred caret/selection placement.
  if (state.caret.editGeneration === opts.editGeneration) {
    opts.afterRefresh?.(response);
  }
  revealCaret();
  render();
  return true;
}

// The one primitive every edit funnels through. The backend returns the
// authoritative post-edit caret (already column-clamped against the real
// document), so we commit it — and the new line count — to local state
// immediately, before any await that could reject. That keeps the caret/cache
// from going stale while the document advanced, and lets the next queued edit
// resolve its range against a correct caret even if the refresh below fails.
export async function applyRange(l0, c0, l1, c1, text) {
  const ctx = editContext();
  const gen = state.caret.editGeneration;
  const body: ReplaceRangeRequest = { l0, c0, l1, c1, text };
  const res = await apiPost<ReplaceEditResponse, ReplaceRangeRequest>(
    "/api/edit/replace_range",
    body,
  );
  await commitEdit(ctx, res, {
    editGeneration: gen,
    applyCurrent(response) {
      // No user navigation happened during the round-trip: honor the edit caret.
      const line = Math.min(response.caret_line, Math.max(0, state.view.total - 1));
      setSelection(null);
      state.caret.position = { line, col: response.caret_col };
      setActiveLine(line);
      state.caret.goalCol = response.caret_col;
    },
  });
}

export async function applyRect(l0, l1, c0, c1, text) {
  const ctx = editContext();
  const gen = state.caret.editGeneration;
  const body: ReplaceRectRequest = { l0, l1, c0, c1, text };
  const res = await apiPost<ReplaceEditResponse, ReplaceRectRequest>(
    "/api/edit/replace_rect",
    body,
  );
  await commitEdit(ctx, res, {
    editGeneration: gen,
    applyCurrent(response) {
      const line = Math.min(response.caret_line, Math.max(0, state.view.total - 1));
      setSelection(null);
      state.caret.position = { line, col: response.caret_col };
      setActiveLine(line);
      state.caret.goalCol = response.caret_col;
    },
  });
}

// Multi-cursor edits: send every cursor's replacement as ONE batch (the server
// records it as a single undo step) and adopt the returned carets. `cursors`
// is the sorted allCursors() list; `editOf[i]` is the index of cursor i's edit
// in `edits`, or -1 when the cursor contributed no edit (only possible at the
// document origin, whose position an edit batch cannot move).
export async function applyBatch(
  edits,
  cursors,
  editOf,
  options: { caretBackByEdit?: number[] } = {},
) {
  const ctx = editContext();
  const gen = state.caret.editGeneration;
  const res = await apiPost<BatchEditResponse, { edits: unknown[] }>("/api/edit/replace_batch", {
    edits,
  });
  await commitEdit(ctx, res, {
    editGeneration: gen,
    applyCurrent(response) {
      const clampLine = (line) => Math.min(line, Math.max(0, state.view.total - 1));
      const next = cursors.map((cursor, index) => {
        const editIndex = editOf[index];
        const position =
          editIndex >= 0 && response.carets?.[editIndex] ? response.carets[editIndex] : cursor;
        const caretBack = editIndex >= 0 ? options.caretBackByEdit?.[editIndex] || 0 : 0;
        return {
          line: clampLine(position.line),
          col: Math.max(0, position.col - caretBack),
          primary: cursor.primary,
        };
      });
      const primary = next.find((cursor) => cursor.primary) || next[0];
      setSelection(null);
      state.caret.position = { line: primary.line, col: primary.col };
      setActiveLine(primary.line);
      state.caret.goalCol = primary.col;
      state.caret.extraCursors = next
        .filter((cursor) => cursor !== primary)
        .map((cursor) => ({ line: cursor.line, col: cursor.col }));
    },
    applyStale(response) {
      // Remap only cursors owned by this batch; user-added/removed cursors win.
      const clampLine = (line) => Math.min(line, Math.max(0, state.view.total - 1));
      const line = clampLine(state.caret.position.line);
      state.caret.position = { line, col: state.caret.position.col };
      setActiveLine(line);
      if (!state.caret.extraCursors.length) return;
      const moved = new Map();
      cursors.forEach((cursor, index) => {
        const editIndex = editOf[index];
        const position =
          editIndex >= 0 && response.carets?.[editIndex] ? response.carets[editIndex] : cursor;
        const caretBack = editIndex >= 0 ? options.caretBackByEdit?.[editIndex] || 0 : 0;
        moved.set(`${cursor.line}:${cursor.col}`, {
          line: clampLine(position.line),
          col: Math.max(0, position.col - caretBack),
        });
      });
      const seen = new Set();
      state.caret.extraCursors = state.caret.extraCursors.flatMap((cursor) => {
        const next = moved.get(`${cursor.line}:${cursor.col}`) || {
          line: clampLine(cursor.line),
          col: cursor.col,
        };
        const key = `${next.line}:${next.col}`;
        if (seen.has(key)) return [];
        seen.add(key);
        return [next];
      });
    },
  });
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

function adoptCursorPositions(cursors, positions) {
  const next = cursors.map((cursor, index) => ({
    ...(positions[index] || cursor),
    primary: cursor.primary,
  }));
  const primary = next.find((cursor) => cursor.primary) || next[0];
  setSelection(null);
  state.caret.position = { line: primary.line, col: primary.col };
  setActiveLine(primary.line);
  state.caret.goalCol = primary.col;
  state.caret.extraCursors = next
    .filter((cursor) => cursor !== primary)
    .map((cursor) => ({ line: cursor.line, col: cursor.col }));
  state.caret.editGeneration++;
  revealCaret();
  render();
}

async function cursorLineTexts(cursors) {
  const lines = [...new Set(cursors.map((cursor) => cursor.line))];
  const entries = await Promise.all(
    lines.map(async (line) => [line, await oneLineText(line)] as const),
  );
  return new Map(entries);
}

function activeStructureProvider() {
  const path = state.doc.stat?.path || "";
  const selection = state.syntax.overrides[path] || "auto";
  const scheme = resolveSyntaxScheme(path, selection, state.syntax.mappings);
  return scheme ? schemeDefinition(scheme).structure || null : null;
}

async function encloseRectSelection(opener: string, closer: string) {
  const rect = rectRange();
  if (!rect) return null;
  const texts = await lineTextsFor(rect.l0, rect.l1 - rect.l0 + 1);
  const edits = [];
  const cursors = [];
  const editOf = [];
  const caretBackByEdit = [];
  for (let offset = 0; offset < texts.length; offset++) {
    const line = rect.l0 + offset;
    const length = Array.from(texts[offset]).length;
    const start = Math.min(rect.c0, length);
    const end = Math.min(rect.c1, length);
    cursors.push({ line, col: end, primary: line === state.caret.position.line });
    if (start === end) {
      edits.push({ l0: line, c0: start, l1: line, c1: start, text: opener + closer });
      editOf.push(edits.length - 1);
      caretBackByEdit.push(1);
    } else {
      edits.push({ l0: line, c0: start, l1: line, c1: start, text: opener });
      caretBackByEdit.push(0);
      edits.push({ l0: line, c0: end, l1: line, c1: end, text: closer });
      editOf.push(edits.length - 1);
      caretBackByEdit.push(1);
    }
  }
  if (!cursors.some((cursor) => cursor.primary) && cursors.length) {
    cursors[cursors.length - 1].primary = true;
  }
  return edits.length ? applyBatch(edits, cursors, editOf, { caretBackByEdit }) : null;
}

async function assistedTypeStep(text: string) {
  const openerCloser = pairCloser(text);
  if (!openerCloser && !isPairCloser(text)) return plainTypeStep(text);
  const rect = rectRange();
  if (
    rect &&
    openerCloser &&
    state.settings.selectionEnclosure !== false &&
    (rect.c0 !== rect.c1 || state.settings.closePairs !== false)
  ) {
    return encloseRectSelection(text, openerCloser);
  }
  const cursors = allCursors();
  const hasEnclosure =
    !!openerCloser &&
    state.settings.selectionEnclosure !== false &&
    cursors.some((cursor) => cursorSelectionRange(cursor));
  if (hasEnclosure) {
    const edits = [];
    const editOf = [];
    const caretBackByEdit = [];
    for (const cursor of cursors) {
      const range = cursorSelectionRange(cursor);
      if (range) {
        edits.push({
          l0: range.start.line,
          c0: range.start.col,
          l1: range.start.line,
          c1: range.start.col,
          text,
        });
        caretBackByEdit.push(0);
        edits.push({
          l0: range.end.line,
          c0: range.end.col,
          l1: range.end.line,
          c1: range.end.col,
          text: openerCloser,
        });
        editOf.push(edits.length - 1);
        caretBackByEdit.push(1);
      } else {
        const autoClose = state.settings.closePairs !== false;
        edits.push({
          ...cursorReplaceRange(cursor),
          text: autoClose ? text + openerCloser : text,
        });
        editOf.push(edits.length - 1);
        caretBackByEdit.push(autoClose ? 1 : 0);
      }
    }
    return applyBatch(edits, cursors, editOf, { caretBackByEdit });
  }
  if (state.settings.closePairs === false) return plainTypeStep(text);

  const lines = await cursorLineTexts(cursors);
  const edits = [];
  const editOf = [];
  const caretBackByEdit = [];
  const fallbackPositions = cursors.map((cursor) => ({ line: cursor.line, col: cursor.col }));
  let skipped = 0;
  for (let index = 0; index < cursors.length; index++) {
    const cursor = cursors[index];
    const line = lines.get(cursor.line) || "";
    if (!cursorSelectionRange(cursor) && shouldSkipCloser(line, cursor.col, text)) {
      // When every caret skips we can move locally. In a mixed batch, this
      // no-op replacement lets the backend map the skipped caret through edits
      // above it without recording another undo generation.
      edits.push({ l0: cursor.line, c0: cursor.col, l1: cursor.line, c1: cursor.col + 1, text });
      editOf.push(edits.length - 1);
      caretBackByEdit.push(0);
      fallbackPositions[index] = { line: cursor.line, col: cursor.col + 1 };
      skipped++;
      continue;
    }
    const autoClose =
      !!openerCloser && !cursorSelectionRange(cursor) && shouldAutoClose(line, cursor.col, text);
    edits.push({ ...cursorReplaceRange(cursor), text: autoClose ? text + openerCloser : text });
    editOf.push(edits.length - 1);
    caretBackByEdit.push(autoClose ? 1 : 0);
  }
  if (skipped === cursors.length) {
    adoptCursorPositions(cursors, fallbackPositions);
    return null;
  }
  return applyBatch(edits, cursors, editOf, { caretBackByEdit });
}

function plainTypeStep(text: string) {
  if (state.caret.extraCursors.length) {
    const cursors = allCursors();
    return hasCursorSelections()
      ? multiReplace(cursors, () => text)
      : multiInsert(cursors, () => text);
  }
  const rr = rectRange();
  if (rr) return applyRect(rr.l0, rr.l1, rr.c0, rr.c1, text);
  const target = replaceTarget();
  return applyRange(target.l0, target.c0, target.l1, target.c1, text);
}

// Insert (or replace the selection with) `text`, which may contain newlines.
// The target range is resolved *inside* the queued step, so a burst of
// keystrokes each sees the caret left by the previous edit (never a stale one).
// A 0-line document (an empty file) is editable too: replaceTarget yields the
// (0,0)..(0,0) origin range, which the backend accepts to seed the first line.
export function typeText(text) {
  if (!state.doc.stat?.open) return;
  return enqueueEdit(() => plainTypeStep(text));
}

export function typeAssistedText(text: string) {
  if (!state.doc.stat?.open) return Promise.resolve(null);
  if (Array.from(text).length !== 1) return typeText(text);
  return enqueueEdit(() => assistedTypeStep(text));
}

export function insertNewline() {
  if (!state.doc.stat?.open) return Promise.resolve(null);
  if (state.settings.autoIndent === false) return typeText("\n");
  return enqueueEdit(async () => {
    if (rectRange()) return plainTypeStep("\n");
    const cursors = allCursors();
    const lines = await cursorLineTexts(cursors);
    const provider = activeStructureProvider();
    const edits = cursors.map((cursor) => {
      const range = cursorReplaceRange(cursor);
      return {
        ...range,
        text: `\n${newlineIndent(lines.get(range.l0) || "", range.c0, provider)}`,
      };
    });
    return applyBatch(
      edits,
      cursors,
      cursors.map((_, index) => index),
    );
  });
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
    if (l < 0 || l >= state.view.total || out.has(l) || missing.has(l)) continue;
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
  if (state.caret.extraCursors.length && hasCursorSelections()) {
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
  const gen = state.caret.editGeneration;
  const c = rr.c0;
  const lines = [];
  for (let l = rr.l0; l <= rr.l1; l++) lines.push(l);
  const lens = await lineLensFor(lines);
  const pairTexts =
    !forward && state.settings.closePairs !== false
      ? await cursorLineTexts(lines.map((line) => ({ line })))
      : null;
  const edits = [];
  for (const l of lines) {
    if (!lens.has(l)) continue;
    const len = lens.get(l);
    if (forward) {
      if (c < len) edits.push({ l0: l, c0: c, l1: l, c1: c + 1, text: "" });
    } else if (c > 0 && c <= len) {
      const pair = emptyPairRange(pairTexts?.get(l) || "", c);
      edits.push({
        l0: l,
        c0: pair?.start ?? c - 1,
        l1: l,
        c1: pair?.end ?? c,
        text: "",
      });
    }
  }
  if (!edits.length) return null; // nothing to delete on any line — no request
  const ctx = editContext();
  const response = await apiPost<BatchEditResponse, { edits: unknown[] }>(
    "/api/edit/replace_batch",
    { edits },
  );
  await commitEdit(ctx, response, {
    editGeneration: gen,
    afterRefresh() {
      // Keep the column caret alive at the new column so a held
      // Backspace/Delete keeps acting on every line.
      const newCol = forward ? c : Math.max(0, c - 1);
      const l0 = Math.min(rr.l0, Math.max(0, state.view.total - 1));
      const l1 = Math.min(rr.l1, Math.max(0, state.view.total - 1));
      setSelection({
        anchor: { line: l0, col: newCol },
        head: { line: l1, col: newCol },
        rect: true,
      });
      state.caret.extraCursors = [];
      setCaret(l1, newCol);
    },
  });
}

export function backspace() {
  enqueueEdit(async () => {
    const rr = rectRange();
    if (rr && rr.c0 === rr.c1) return deleteZeroWidthRect(rr, false);
    const del = deleteSelectionEdit();
    if (del) return del;
    if (state.caret.extraCursors.length) {
      // Per cursor: delete one char before the caret (line-join at col 0).
      // allCursors() dedupes positions, so ranges may touch but never overlap;
      // a cursor at the document origin contributes no edit. Join edges need
      // the previous line's REAL length, which may live outside the cache.
      const cursors = allCursors();
      const lines =
        state.settings.closePairs !== false ? await cursorLineTexts(cursors) : new Map();
      const lens = await lineLensFor(
        cursors.filter((c) => c.col === 0 && c.line > 0).map((c) => c.line - 1),
      );
      const edits = [];
      const editOf = cursors.map((c) => {
        if (c.col > 0) {
          const pair = emptyPairRange(lines.get(c.line) || "", c.col);
          edits.push({
            l0: c.line,
            c0: pair?.start ?? c.col - 1,
            l1: c.line,
            c1: pair?.end ?? c.col,
            text: "",
          });
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
    const { line, col } = state.caret.position;
    if (col > 0) {
      const pair =
        state.settings.closePairs !== false ? emptyPairRange(await oneLineText(line), col) : null;
      return applyRange(line, pair?.start ?? col - 1, line, pair?.end ?? col, "");
    }
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
    if (state.caret.extraCursors.length) {
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
        } else if (c.line < state.view.total - 1) {
          edits.push({ l0: c.line, c0: c.col, l1: c.line + 1, c1: 0, text: "" });
        } else {
          return -1;
        }
        return edits.length - 1;
      });
      if (!edits.length) return null;
      return applyBatch(edits, cursors, editOf);
    }
    const { line, col } = state.caret.position;
    const lens = await lineLensFor([line]);
    if (!lens.has(line)) return null;
    if (col < lens.get(line)) return applyRange(line, col, line, col + 1, "");
    if (line < state.view.total - 1) return applyRange(line, col, line + 1, 0, "");
    return null;
  });
}

export function pasteText(raw) {
  const text = raw.replace(/\r\n?/g, "\n");
  if (!state.caret.extraCursors.length) {
    typeText(text);
    return;
  }
  if (!state.doc.stat?.open) return;
  enqueueEdit(() => {
    if (!state.caret.extraCursors.length) {
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
    setSelection(null);
    state.caret.extraCursors = []; // a multi-cursor batch undoes as one step

    await refreshStat();
    await reloadViewport();
    setCaret(state.caret.position.line, state.caret.position.col); // re-clamp into the new bounds
    revealCaret();
    render();
  });
}

export async function redoEdit() {
  enqueueEdit(async () => {
    await apiPost("/api/edit/redo", {});
    setSelection(null);
    state.caret.extraCursors = []; // a multi-cursor batch redoes as one step

    await refreshStat();
    await reloadViewport();
    setCaret(state.caret.position.line, state.caret.position.col);
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
  if (!state.doc.stat?.open) return;
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
  const gen = state.caret.editGeneration;
  const res = await apiPost<BatchEditResponse, { edits: unknown[] }>("/api/edit/replace_batch", {
    edits,
  });
  await commitEdit(ctx, res, {
    editGeneration: gen,
    applyCurrent() {
      setSelection(null);
      state.caret.extraCursors = [];
    },
    afterRefresh() {
      setCaret(
        Math.min(state.caret.position.line, Math.max(0, state.view.total - 1)),
        state.caret.position.col,
      );
    },
  });
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
  return { l0: state.caret.position.line, l1: state.caret.position.line };
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
  const gen = state.caret.editGeneration;
  const res = await apiPost<BatchEditResponse, { edits: unknown[] }>("/api/edit/replace_batch", {
    edits,
  });
  await commitEdit(ctx, res, {
    editGeneration: gen,
    applyCurrent() {
      state.caret.extraCursors = []; // line ops are single-caret
    },
    afterRefresh() {
      const last = Math.max(0, state.view.total - 1);
      const place = (position) => {
        const line = Math.min(Math.max(0, position.line), last);
        const cached = cachedLine(line);
        const col = cached
          ? Math.min(position.col, charLenOf(cached.text ?? ""))
          : Math.max(0, position.col);
        return { line, col };
      };
      const nextCaret = place(caret);
      state.caret.position = nextCaret;
      setActiveLine(nextCaret.line);
      state.caret.goalCol = nextCaret.col;
      setSelection(sel ? { anchor: place(sel.anchor), head: place(sel.head) } : null);
    },
  });
}

// 行を複製: duplicate the covered line block just below itself.
export function duplicateLines() {
  if (!state.doc.stat?.open || state.view.total === 0) return;
  enqueueEdit(async () => {
    const { l0, l1 } = lineOpSpan();
    if (l1 - l0 + 1 > MAX_COPY_LINES) {
      flashCount(t("editor.duplicateCapped", { max: commas(MAX_COPY_LINES) }), "error");
      return null;
    }
    const caret = { ...state.caret.position }; // the copy lands below; the caret stays put
    const sel = cloneSelection(
      state.caret.selection && !state.caret.selection.rect ? state.caret.selection : null,
    );
    const texts = await lineTextsFor(l0, l1 - l0 + 1);
    const endCol = charLenOf(texts[texts.length - 1]);
    const edit = { l0: l1, c0: endCol, l1, c1: endCol, text: "\n" + texts.join("\n") };
    return applyLineEdit([edit], caret, sel);
  });
}

// 行を上へ / 下へ移動: swap the covered block with its neighbouring line.
export function moveLines(dir) {
  if (!state.doc.stat?.open || state.view.total === 0) return;
  enqueueEdit(async () => {
    const { l0, l1 } = lineOpSpan();
    if (dir < 0 ? l0 === 0 : l1 >= state.view.total - 1) return null; // already at the edge
    if (l1 - l0 + 1 > MAX_COPY_LINES) {
      flashCount(t("editor.moveCapped", { max: commas(MAX_COPY_LINES) }), "error");
      return null;
    }
    const caret = { line: state.caret.position.line + dir, col: state.caret.position.col };
    const sel = shiftLineSelection(
      cloneSelection(
        state.caret.selection && !state.caret.selection.rect ? state.caret.selection : null,
      ),
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
  if (!state.doc.stat?.open || state.view.total === 0) return;
  enqueueEdit(async () => {
    const { l0, l1 } = lineOpSpan();
    let edit;
    let caret;
    if (l1 < state.view.total - 1) {
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
  const line = Math.min(v - 1, Math.max(0, state.view.total - 1));
  setSelection(null);
  setCaret(line, 0);
  revealLine(line);
  focusEditor();
}

// Native/CLI deep link. Coordinate validation, EOF resolution and clamping
// live on the server so every encoding and huge logical line number follows
// one conversion path. Only resolved editor-native coordinates cross back.
export async function gotoLaunchPosition(position) {
  if (!position || !Number.isSafeInteger(position.line) || !Number.isSafeInteger(position.column)) {
    return;
  }
  const resolved = await apiPost<
    { line: number; column: number; truncated: boolean },
    { line: number; column: number }
  >("/api/position/resolve", position);
  const page = await api<LinesResponse>(`/api/lines?start=${resolved.line}&count=1`);
  cacheLineResponse(resolved.line, page);
  const available = charLenOf(page.lines[0]?.text ?? "");
  setSelection(null);
  setCaret(resolved.line, resolved.column, available);
  revealLine(resolved.line);
  render();
  focusEditor();
}
