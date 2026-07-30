// Ayame Editor — find bar UI, matcher, navigation, count, and history.
import { $, commas, escapeRegExp } from "./dom.js";
import { state } from "./state.js";
import { t } from "./i18n.js";
import { api, type FindResponse, type SearchResponse } from "./api.js";
import {
  lineByte,
  revealLine,
  rowsVisible,
  scheduleRender,
  setCaret,
  setSearchHits,
  setSelection,
} from "./editor.js";
import { loadSearchHistoryShared, saveSearchHistoryShared } from "./persistence.js";
import { flashCount } from "./notifications.js";

export function showFind(withReplace = false) {
  state.search.findOpen = true;
  document.documentElement.classList.add("find-open");
  if (withReplace) setReplaceRow(true);
  const f = withReplace && state.search.query ? $("replace-input") : $("find");
  queueMicrotask(() => {
    f.focus();
    f.select();
  });
}

export function hideFind() {
  state.search.findOpen = false;
  document.documentElement.classList.remove("find-open");
  setReplaceRow(false);
}

export function setReplaceRow(open) {
  state.search.replaceOpen = open;
  document.documentElement.classList.toggle("replace-open", open);
  $("find-expand").setAttribute("aria-expanded", open ? "true" : "false");
  const row = $("replace-row");
  row.setAttribute("aria-hidden", open ? "false" : "true");
  row.inert = !open;
}

export function buildMatcher() {
  state.search.regexError = false;
  state.search.matcherWordFallback = false;
  $("find").parentElement.classList.remove("error");
  if (!state.search.query) {
    state.search.matcher = null;
    return;
  }
  const src = state.search.regex ? state.search.query : escapeRegExp(state.search.query);
  const flags = "g" + (state.search.caseInsensitive ? "i" : "");
  let plain;
  try {
    // Validate the user's pattern on its own before embedding it in the word
    // wrapper. An unmatched "[" can otherwise consume syntax from the wrapper
    // and accidentally turn an invalid query into a valid expression.
    plain = new RegExp(src, flags);
  } catch {
    state.search.regexError = true;
    state.search.matcher = null; // invalid regex while typing — just don't highlight
    $("find").parentElement.classList.add("error");
    return;
  }
  if (!state.search.word) {
    state.search.matcher = plain;
    return;
  }
  try {
    // Mirror the server's whole-word rule so the highlight matches the count.
    state.search.matcher = new RegExp(
      `(?<![\\p{L}\\p{N}_])(?:${src})(?![\\p{L}\\p{N}_])`,
      flags + "u",
    );
  } catch {
    // The Unicode wrapper can reject patterns the plain form accepts.
    state.search.matcher = plain; // fall back: highlight the superset
    state.search.matcherWordFallback = true;
  }
}

export function setQueryFromInput() {
  state.search.query = $("find").value;
  state.search.lastMatch = null;
  setSearchHits(null);
  state.search.truncated = false;
  buildMatcher();
  $("find-count").textContent = state.search.regexError ? t("find.regexError") : "";
  scheduleCount(); // keep the "N / total" label in sync with the live highlights
  scheduleRender();
}

export function qs() {
  return `q=${encodeURIComponent(state.search.query)}&regex=${state.search.regex}&ci=${state.search.caseInsensitive}&word=${state.search.word}`;
}

export async function findStep(dir) {
  if (!state.search.query) return;
  buildMatcher();
  if (state.search.regexError) {
    flashCount(t("find.regexError"), "error");
    return;
  }
  saveSearchHistory(state.search.query);
  let from;
  if (dir === "next") {
    from = state.search.lastMatch
      ? state.search.lastMatch.byte + Math.max(1, state.search.lastMatch.len)
      : await lineByte(state.view.first);
  } else {
    from = state.search.lastMatch
      ? state.search.lastMatch.byte
      : await lineByte(Math.min(state.view.total, state.view.first + rowsVisible()));
  }
  try {
    let res = await api<FindResponse>(`/api/find?dir=${dir}&from=${from}&${qs()}`);
    let wrapped = false;
    if (!res.hit) {
      // Wrap around: search again from the opposite end so "next" past the last
      // match rolls to the first, and "prev" past the first rolls to the last.
      const wrapFrom = dir === "next" ? 0 : await lineByte(state.view.total);
      res = await api<FindResponse>(`/api/find?dir=${dir}&from=${wrapFrom}&${qs()}`);
      wrapped = true;
    }
    if (!res.hit) {
      flashCount(t("find.noMatch"));
      return;
    }
    const h = res.hit;
    state.search.lastMatch = { byte: h.byte, len: h.byte_len };
    setSelection(null);
    setCaret(h.line, 0);
    revealLine(h.line);
    if (wrapped) flashCount(dir === "next" ? t("find.wrapTop") : t("find.wrapBottom"));
    updateCount();
  } catch (e) {
    flashCount(t("common.error"));
    console.error(e);
  }
}

let countRequestToken = 0;
let countRequestController: AbortController | null = null;
let countDebounceTimer = 0;

// Live incremental-search feedback is debounced so a fast typist doesn't fire a
// /api/search per keystroke; 120ms keeps the count feeling immediate.
export const COUNT_DEBOUNCE_MS = 120;

// Invalidate any in-flight count request so its late response is ignored.
function abortPendingCount() {
  countRequestToken++;
  countRequestController?.abort();
  countRequestController = null;
}

// Refresh the match count as the query changes while typing, coalescing bursts
// of keystrokes into one request. An empty or invalid-regex query has nothing
// to count, so we only drop any pending request and leave the label as
// setQueryFromInput set it (empty, or the regex-error message).
export function scheduleCount() {
  clearTimeout(countDebounceTimer);
  countDebounceTimer = 0;
  if (!state.search.query || state.search.regexError) {
    abortPendingCount();
    return;
  }
  countDebounceTimer = setTimeout(() => {
    countDebounceTimer = 0;
    updateCount();
  }, COUNT_DEBOUNCE_MS);
}

export async function updateCount() {
  clearTimeout(countDebounceTimer);
  countDebounceTimer = 0;
  abortPendingCount();
  const token = countRequestToken;
  if (!state.search.query) {
    $("find-count").textContent = "";
    setSearchHits(null);
    state.search.truncated = false;
    return;
  }
  const controller = new AbortController();
  countRequestController = controller;
  try {
    const res = await api<SearchResponse>(`/api/search?${qs()}&start=0&max=2000`, {
      signal: controller.signal,
    });
    if (token !== countRequestToken || controller.signal.aborted) return;
    setSearchHits(res.hits);
    state.search.truncated = res.truncated;
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
  const hits = state.search.hits;
  if (!hits || !state.search.query) {
    $("find-count").textContent = "";
    return;
  }
  const total = state.search.truncated ? `${commas(hits.length)}+` : commas(hits.length);
  if (state.search.lastMatch) {
    const idx = hits.findIndex((h) => h.byte === state.search.lastMatch.byte);
    if (idx >= 0) {
      $("find-count").textContent = `${commas(idx + 1)} / ${total}`;
      return;
    }
  }
  $("find-count").textContent = t("find.matchCount", { total });
}

export function loadSearchHistory() {
  return loadSearchHistoryShared();
}

export function saveSearchHistory(q) {
  const value = q.trim();
  if (!value) return;
  state.search.history = [value, ...state.search.history.filter((x) => x !== value)].slice(0, 50);
  state.search.historyIndex = -1;
  saveSearchHistoryShared(state.search.history);
}

export function showSearchHistory(delta) {
  if (!state.search.history.length) return false;
  if (state.search.historyIndex < 0) {
    state.search.historyIndex = delta < 0 ? 0 : state.search.history.length - 1;
  } else {
    state.search.historyIndex = Math.min(
      state.search.history.length - 1,
      Math.max(0, state.search.historyIndex + delta),
    );
  }
  $("find").value = state.search.history[state.search.historyIndex];
  setQueryFromInput();
  return true;
}
