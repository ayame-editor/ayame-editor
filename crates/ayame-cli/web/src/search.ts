// Ayame Editor — search module. Type-stripped to JS at build time (build.rs, oxc).
import { $, commas, displayPath, escapeRegExp, pathDirName, setModalOpen } from "./dom.js";
import { BROWSE_KEY, state } from "./state.js";
import { serverMessage, t } from "./i18n.js";
import { api, apiPost, type FindResponse, type LinesResponse, type SearchResponse } from "./api.js";
import {
  focusEditor,
  lineByte,
  lineChars,
  revealLine,
  rowsVisible,
  scheduleRender,
  setCaret,
} from "./editor.js";
import {
  clonePoint,
  cloneSelection,
  cursorSelectionRange,
  normalizedRange,
  rangeEmpty,
  rangeKey,
  rectRange,
  selectedTextForRange,
  selectionRanges,
} from "./selection.js";
import { applyBatchPlain, applyRange, enqueueEdit, gotoLine } from "./edits.js";
import { askForm, hideLoading, showLoading, showMessage } from "./dialogs.js";
import { anyModalOpen, isWordChar, setQueryFromInput } from "./input.js";
import { openPath, showFolderDialog } from "./workspace.js";
import { loadSearchHistoryShared, saveSearchHistoryShared } from "./persistence.js";
import type { GrepRequest } from "./types/api.js";

type GrepResponse = {
  hits: { path: string; line: number; col: number; text: string }[];
  truncated: boolean;
  files_scanned: number;
  files_truncated: boolean;
};

// ---- search ----------------------------------------------------------------

export function showFind(withReplace = false) {
  state.findOpen = true;
  document.documentElement.classList.add("find-open");
  if (withReplace) setReplaceRow(true);
  const f = withReplace && state.query ? $("replace-input") : $("find");
  queueMicrotask(() => {
    f.focus();
    f.select();
  });
}

export function hideFind() {
  state.findOpen = false;
  document.documentElement.classList.remove("find-open");
  setReplaceRow(false);
}

export function setReplaceRow(open) {
  state.replaceOpen = open;
  document.documentElement.classList.toggle("replace-open", open);
  $("find-expand").setAttribute("aria-expanded", open ? "true" : "false");
}

export function buildMatcher() {
  state.regexError = false;
  state.matcherWordFallback = false;
  $("find").parentElement.classList.remove("error");
  if (!state.query) {
    state.matcher = null;
    return;
  }
  const src = state.regex ? state.query : escapeRegExp(state.query);
  const flags = "g" + (state.ci ? "i" : "");
  try {
    // Mirror the server's whole-word rule so the highlight matches the count.
    state.matcher = state.word
      ? new RegExp(`(?<![\\p{L}\\p{N}_])(?:${src})(?![\\p{L}\\p{N}_])`, flags + "u")
      : new RegExp(src, flags);
    return;
  } catch {
    // The word/unicode wrapper can reject patterns the plain form accepts.
  }
  try {
    state.matcher = new RegExp(src, flags); // fall back: highlight the superset
    state.matcherWordFallback = !!state.word;
  } catch {
    state.regexError = true;
    state.matcher = null; // invalid regex while typing — just don't highlight
    $("find").parentElement.classList.add("error");
  }
}

export function qs() {
  return `q=${encodeURIComponent(state.query)}&regex=${state.regex}&ci=${state.ci}&word=${state.word}`;
}

export async function findStep(dir) {
  if (!state.query) return;
  buildMatcher();
  if (state.regexError) {
    flashCount(t("find.regexError"), "error");
    return;
  }
  saveSearchHistory(state.query);
  let from;
  if (dir === "next") {
    from = state.lastMatch
      ? state.lastMatch.byte + Math.max(1, state.lastMatch.len)
      : await lineByte(state.first);
  } else {
    from = state.lastMatch
      ? state.lastMatch.byte
      : await lineByte(Math.min(state.total, state.first + rowsVisible()));
  }
  try {
    let res = await api<FindResponse>(`/api/find?dir=${dir}&from=${from}&${qs()}`);
    let wrapped = false;
    if (!res.hit) {
      // Wrap around: search again from the opposite end so "next" past the last
      // match rolls to the first, and "prev" past the first rolls to the last.
      const wrapFrom = dir === "next" ? 0 : await lineByte(state.total);
      res = await api<FindResponse>(`/api/find?dir=${dir}&from=${wrapFrom}&${qs()}`);
      wrapped = true;
    }
    if (!res.hit) {
      flashCount(t("find.noMatch"));
      return;
    }
    const h = res.hit;
    state.lastMatch = { byte: h.byte, len: h.byte_len };
    state.sel = null;
    setCaret(h.line, 0);
    revealLine(h.line);
    if (wrapped) flashCount(dir === "next" ? t("find.wrapTop") : t("find.wrapBottom"));
    updateCount();
  } catch (e) {
    flashCount(t("common.error"));
    console.error(e);
  }
}

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
  state.sel = { anchor: clonePoint(r.start), head: clonePoint(r.end) };
  state.caret = clonePoint(r.end);
  state.activeLine = state.caret.line;
  state.goalCol = state.caret.col;
  state.editGen++;
  revealLine(state.caret.line);
  focusEditor();
  scheduleRender();
}

export function promoteSelectionRange(r) {
  const nextKey = rangeKey(r);
  const old =
    state.sel && !state.sel.rect ? normalizedRange(state.sel.anchor, state.sel.head) : null;
  if (old && !rangeEmpty(old) && rangeKey(old) !== nextKey) {
    const exists = state.extraCursors.some((c) => {
      const cr = cursorSelectionRange(c);
      return cr && rangeKey(cr) === rangeKey(old);
    });
    if (!exists) {
      state.extraCursors.push({
        line: state.sel.head.line,
        col: state.sel.head.col,
        sel: cloneSelection(state.sel),
      });
    }
  }
  state.extraCursors = state.extraCursors.filter((c) => {
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
  if (!state.stat?.open) return;
  if (rectRange()) {
    flashCount(t("find.rectNoCtrlD"), "error");
    return;
  }
  let ranges = selectionRanges();
  if (!ranges.length) {
    const r = wordRangeAt(state.caret);
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

let countRequestToken = 0;
let countRequestController: AbortController | null = null;
let countDebounceTimer: ReturnType<typeof setTimeout> | null = null;

export function scheduleCountUpdate(delay = 150) {
  if (countDebounceTimer != null) clearTimeout(countDebounceTimer);
  countDebounceTimer = null;
  countRequestToken++;
  countRequestController?.abort();
  countRequestController = null;
  if (!state.query) {
    $("find-count").textContent = "";
    state.searchHits = null;
    state.searchTruncated = false;
    return;
  }
  if (state.regexError) return;
  countDebounceTimer = setTimeout(() => {
    countDebounceTimer = null;
    void updateCount();
  }, delay);
}

export async function updateCount() {
  if (countDebounceTimer != null) clearTimeout(countDebounceTimer);
  countDebounceTimer = null;
  const token = ++countRequestToken;
  countRequestController?.abort();
  countRequestController = null;
  if (!state.query) {
    $("find-count").textContent = "";
    state.searchHits = null;
    state.searchTruncated = false;
    return;
  }
  const controller = new AbortController();
  countRequestController = controller;
  try {
    const res = await api<SearchResponse>(`/api/search?${qs()}&start=0&max=2000`, {
      signal: controller.signal,
    });
    if (token !== countRequestToken || controller.signal.aborted) return;
    state.searchHits = res.hits;
    state.searchTruncated = res.truncated;
    updateFindCountLabel();
    scheduleRender();
  } catch {
    if (token !== countRequestToken || controller.signal.aborted) return;
    $("find-count").textContent = t("find.searchError");
    flashCount(t("find.searchError"), "error");
    scheduleRender();
  } finally {
    if (token === countRequestToken) countRequestController = null;
  }
}

export function updateFindCountLabel() {
  const hits = state.searchHits;
  if (!hits || !state.query) {
    $("find-count").textContent = "";
    return;
  }
  const total = state.searchTruncated ? `${commas(hits.length)}+` : commas(hits.length);
  if (state.lastMatch) {
    const idx = hits.findIndex((h) => h.byte === state.lastMatch.byte);
    if (idx >= 0) {
      $("find-count").textContent = `${commas(idx + 1)} / ${total}`;
      return;
    }
  }
  $("find-count").textContent = t("find.matchCount", { total });
}

// Operation feedback goes to the always-visible status bar (aria-live), and is
// mirrored into the find bar when that is open. Errors stay a little longer.
// `msg` arrives already localized — callers pass t("key", vars) results.
export let stMsgTimer = 0;

export function flashCount(msg, kind = "") {
  msg = msg || "";
  const isError = kind === "error";
  const el = $("st-msg");
  if (el) {
    el.textContent = msg || "";
    el.classList.toggle("error", isError);
    clearTimeout(stMsgTimer);
    if (msg) {
      stMsgTimer = setTimeout(
        () => {
          el.textContent = "";
          el.classList.remove("error");
          if (state.findOpen) updateFindCountLabel();
        },
        isError ? 10000 : 6000,
      );
    }
  }
  if (state.findOpen) $("find-count").textContent = msg;
}

export function loadSearchHistory() {
  return loadSearchHistoryShared();
}

export function saveSearchHistory(q) {
  const value = q.trim();
  if (!value) return;
  state.history = [value, ...state.history.filter((x) => x !== value)].slice(0, 50);
  state.historyIndex = -1;
  saveSearchHistoryShared(state.history);
}

export function showSearchHistory(delta) {
  if (!state.history.length) return false;
  if (state.historyIndex < 0) {
    state.historyIndex = delta < 0 ? 0 : state.history.length - 1;
  } else {
    state.historyIndex = Math.min(
      state.history.length - 1,
      Math.max(0, state.historyIndex + delta),
    );
  }
  $("find").value = state.history[state.historyIndex];
  setQueryFromInput();
  return true;
}

// ---- in-editor replace (the find bar's replace row) -------------------------
//
// Replacements are ordinary edit-session batches (/api/edit/replace_batch), so
// they show up in the view immediately and undo like any other edit — no
// separate output file. Matching lines come from the server (the same engine
// the counter uses); the replacement text per line is computed with the same
// JS matcher that drives the highlights, so regex group references ($1, $&)
// work in regex mode. In literal mode the replacement is inserted verbatim.

export const REPLACE_ALL_MAX = 20000;
// hits per pass; the message says when to rerun

export function charLenOf(str) {
  return Array.from(str).length;
}

export function utf8ByteLength(str) {
  return new TextEncoder().encode(str).length;
}

// UTF-16 index of Unicode-scalar column `col` in `text` (surrogate-safe).
export function utf16IndexOfCol(text, col) {
  let idx = 0;
  let c = 0;
  for (const ch of text) {
    if (c >= col) break;
    idx += ch.length;
    c++;
  }
  return idx;
}

// The replacement string sent to the document for one concrete match.
export function replacementFor(matchText, replacement) {
  if (!state.regex) return replacement;
  const single = new RegExp(state.matcher.source, state.matcher.flags.replace("g", ""));
  return matchText.replace(single, replacement);
}

// In literal mode "$" has no special meaning; escape it for String.replace.
export function literalReplacement(replacement) {
  return replacement.replace(/\$/g, "$$$$");
}

export function replaceReady() {
  if (!state.stat?.open) return false;
  if (!state.query) {
    flashCount(t("find.enterQuery"), "error");
    return false;
  }
  buildMatcher();
  if (state.regexError || !state.matcher) {
    flashCount(t("find.regexError"), "error");
    return false;
  }
  if (state.matcherWordFallback) {
    flashCount(t("find.regexError"), "error");
    return false;
  }
  return true;
}

// 置換: replace the current match, then move to the next one. Without a
// current match this just selects the first one (Notepad-style two-step).
export async function replaceCurrent() {
  if (!replaceReady()) return;
  const replacement = $("replace-input").value;
  if (!state.lastMatch) {
    await findStep("next");
    return;
  }
  try {
    const res = await api<FindResponse>(`/api/find?dir=next&from=${state.lastMatch.byte}&${qs()}`);
    const h = res.hit;
    if (!h || h.byte !== state.lastMatch.byte) {
      await findStep("next");
      return;
    }
    const lr = await api<LinesResponse>(`/api/lines?start=${h.line}&count=1`);
    const text = lr.lines?.[0]?.text ?? "";
    const u16 = utf16IndexOfCol(text, h.column);
    const re = new RegExp(state.matcher.source, state.matcher.flags);
    re.lastIndex = u16;
    const m = re.exec(text);
    if (!m || m.index !== u16) {
      flashCount(t("find.cannotIdentifyMatch"), "error");
      return;
    }
    const rep = replacementFor(m[0], replacement);
    const c0 = h.column;
    const c1 = h.column + charLenOf(m[0]);
    await enqueueEdit(() => applyRange(h.line, c0, h.line, c1, rep));
    // Resume the scan just past the inserted text so a replacement that
    // contains the query can never loop.
    state.lastMatch = { byte: h.byte, len: Math.max(1, utf8ByteLength(rep)) };
    await updateCount();
    await findStep("next");
  } catch (e) {
    flashCount(t("find.replaceError"), "error");
    console.error(e);
  }
}

// すべて置換: one whole-line edit per matching line, flushed in batches. Line
// numbers never change (line-based matches cannot introduce newlines), so
// every batch keeps referring to valid coordinates.
export async function replaceAll() {
  if (!replaceReady()) return;
  const replacement = $("replace-input").value;
  const literal = literalReplacement(replacement);
  showLoading(t("find.replacing"));
  try {
    const lineSet = new Set<number>();
    let totalHits = 0;
    let start = 0;
    for (let pass = 0; pass < 10000; pass++) {
      const res = await api<SearchResponse>(
        `/api/search?${qs()}&start=${start}&max=${REPLACE_ALL_MAX}`,
      );
      const hits = res.hits || [];
      for (const h of hits) lineSet.add(h.line);
      totalHits += hits.length;
      if (!hits.length || !res.truncated) break;
      const last = hits[hits.length - 1];
      const next = last.byte + Math.max(1, last.byte_len || 0);
      if (next <= start) break;
      start = next;
      flashCount(t("find.matchCount", { total: `${commas(totalHits)}+` }));
    }
    if (!lineSet.size) {
      flashCount(t("find.noMatch"));
      return;
    }
    const lines: number[] = [...lineSet].sort((a, b) => a - b);
    // Fetch the affected lines in contiguous chunks (≤2000 lines per request).
    const texts = new Map();
    for (let i = 0; i < lines.length; ) {
      let j = i;
      while (j + 1 < lines.length && lines[j + 1] - lines[i] < 2000) j++;
      const start = lines[i];
      const count = lines[j] - lines[i] + 1;
      const r = await api<LinesResponse>(`/api/lines?start=${start}&count=${count}`);
      r.lines.forEach((rec, k) => texts.set(start + k, rec.text ?? ""));
      i = j + 1;
    }
    let replaced = 0;
    let edits = [];
    let pendingBytes = 0;
    const flush = async () => {
      if (!edits.length) return;
      const batch = edits;
      edits = [];
      pendingBytes = 0;
      await enqueueEdit(() => applyBatchPlain(batch));
    };
    for (const line of lines) {
      const text = texts.get(line);
      if (text == null) continue;
      const re = new RegExp(state.matcher.source, state.matcher.flags);
      const count = [...text.matchAll(re)].length;
      if (!count) continue;
      const next = text.replace(re, state.regex ? replacement : literal);
      if (next === text) continue;
      replaced += count;
      edits.push({ l0: line, c0: 0, l1: line, c1: charLenOf(text), text: next });
      pendingBytes += next.length;
      if (edits.length >= 2000 || pendingBytes > 512 * 1024) await flush();
    }
    await flush();
    state.lastMatch = null;
    await updateCount();
    flashCount(replaced ? t("find.replacedCount", { n: commas(replaced) }) : t("find.noMatch"));
  } catch (e) {
    flashCount(t("find.replaceError"), "error");
    console.error(e);
  } finally {
    hideLoading();
  }
}

// ---- フォルダ内検索 (Grep): recursive multi-file search -------------------
// Prompts for a query + options, streams the hits from /api/grep, and shows
// them in a results panel modeled on the diff view. Clicking a hit opens that
// file and jumps to the line.
export let lastGrep = { query: "", dir: "", glob: "", ci: false, word: false, regex: false };

export function grepVisible() {
  return !$("grep-modal").classList.contains("hidden");
}

export function hideGrep() {
  setModalOpen($("grep-modal"), false);
  focusEditor();
}

export async function grepFolder() {
  if (anyModalOpen()) return;
  const base =
    lastGrep.dir || localStorage.getItem(BROWSE_KEY) || pathDirName(state.stat?.path || "") || "";
  const form = await askForm(
    t("menu.grep"),
    [
      {
        id: "query",
        type: "text",
        label: t("dialog.grep.query"),
        value: lastGrep.query,
        placeholder: t("dialog.grep.queryPlaceholder"),
      },
      {
        id: "dir",
        type: "path",
        label: t("dialog.grep.dir"),
        value: base,
        placeholder: t("dialog.grep.dirPlaceholder"),
        onBrowse: (cur) =>
          showFolderDialog(t("dialog.open.chooseFolder"), (cur || base || "").trim()),
      },
      {
        id: "glob",
        type: "text",
        label: t("dialog.grep.glob"),
        value: lastGrep.glob,
        placeholder: t("dialog.grep.globPlaceholder"),
      },
      { id: "ci", type: "check", label: t("dialog.grep.ignoreCase"), value: lastGrep.ci },
      { id: "word", type: "check", label: t("find.wholeWord"), value: lastGrep.word },
      { id: "regex", type: "check", label: t("find.regex"), value: lastGrep.regex },
    ],
    t("menu.find"),
  );
  if (!form) return;
  const query = (form.query || "").trim();
  if (!query) return;
  lastGrep = {
    query: form.query,
    dir: (form.dir || "").trim(),
    glob: form.glob || "",
    ci: !!form.ci,
    word: !!form.word,
    regex: !!form.regex,
  };
  showLoading(t("dialog.grep.searching"));
  try {
    const res = await apiPost<GrepResponse, GrepRequest>("/api/grep", {
      query,
      dir: lastGrep.dir || null,
      glob: (form.glob || "").trim(),
      ci: lastGrep.ci,
      word: lastGrep.word,
      regex: lastGrep.regex,
      max: 2000,
    });
    flashCount(t("dialog.grep.flash", { n: commas(res.hits.length) }));
    showGrep(res, query, lastGrep.regex);
  } catch (e) {
    flashCount(t("dialog.grep.error"), "error");
    showMessage(t("dialog.grep.error"), serverMessage(e));
  } finally {
    hideLoading();
  }
}

export function showGrep(res, query, regex) {
  const files = new Set(res.hits.map((h) => h.path)).size;
  $("grep-summary").textContent =
    t("dialog.grep.summary", { hits: commas(res.hits.length), files: commas(files) }) +
    (res.truncated ? t("dialog.grep.summaryTruncated", { max: commas(res.hits.length) }) : "") +
    (res.files_truncated ? t("dialog.grep.summaryFiles") : "");
  renderGrepResults(res, query, regex);
  setModalOpen($("grep-modal"), true);
}

// Highlight the literal match inside a preview line ([col, col+queryChars]).
// Regex matches have a variable span we don't return, so those aren't marked.
export function appendGrepText(el, text, col, query, regex) {
  const chars = Array.from(text);
  const qlen = regex ? 0 : Array.from(query).length;
  if (!qlen || col < 0 || col > chars.length) {
    el.textContent = text;
    return;
  }
  const before = chars.slice(0, col).join("");
  const mid = chars.slice(col, col + qlen).join("");
  const after = chars.slice(col + qlen).join("");
  if (before) el.append(document.createTextNode(before));
  const mark = document.createElement("span");
  mark.className = "grep-match";
  mark.textContent = mid;
  el.append(mark);
  if (after) el.append(document.createTextNode(after));
}

export function renderGrepResults(res, query, regex) {
  const view = $("grep-results");
  view.textContent = "";
  const hits = res.hits || [];
  if (hits.length === 0) {
    const empty = document.createElement("div");
    empty.className = "grep-empty";
    empty.textContent = t("dialog.grep.noMatches");
    view.append(empty);
    return;
  }
  const frag = document.createDocumentFragment();
  let group = null;
  let currentPath = null;
  for (const h of hits) {
    if (h.path !== currentPath) {
      currentPath = h.path;
      group = document.createElement("section");
      group.className = "grep-file";
      const head = document.createElement("div");
      head.className = "grep-file-head";
      head.textContent = displayPath(h.path);
      head.title = displayPath(h.path);
      group.append(head);
      frag.append(group);
    }
    const row = document.createElement("button");
    row.className = "grep-hit";
    row.type = "button";
    const ln = document.createElement("span");
    ln.className = "grep-ln";
    ln.textContent = commas(h.line + 1);
    const tx = document.createElement("span");
    tx.className = "grep-tx";
    appendGrepText(tx, h.text, h.col, query, regex);
    row.append(ln, tx);
    row.addEventListener("click", () => openGrepHit(h.path, h.line));
    group.append(row);
  }
  view.append(frag);
}

export async function openGrepHit(path, line) {
  hideGrep();
  await openPath(path);
  gotoLine(line + 1);
}
