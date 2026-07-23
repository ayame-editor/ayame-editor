// Ayame Editor — sparse line bookmarks.
//
// The server owns the ordered MarkerSet and edit-history coordinate mapping.
// The browser caches only markers returned with the current line window; every
// navigation/list operation is an O(log M) server query rather than a scan of
// the document or a document-sized client array.

import {
  api,
  apiPost,
  type MarkerBulkResponse,
  type MarkerListResponse,
  type MarkerMutationResponse,
  type MarkerNavigateResponse,
  type MarkerPreviewResponse,
  type SearchResponse,
} from "./api.js";
import { $, commas, modalVisible, setModalOpen } from "./dom.js";
import { focusEditor, render, revealCaret, scheduleRender, setCaret } from "./editor.js";
import { lineLensFor, reloadViewport, settleEditQueue } from "./edits.js";
import { t } from "./i18n.js";
import { showPopupMenu } from "./popup-menu.js";
import { flashCount, qs } from "./search.js";
import { MAX_BOOKMARK_SELECTIONS, state } from "./state.js";
import { askConfirm, hideLoading, showLoading } from "./dialogs.js";

const KIND = "bookmark";
const PREVIEW_PAGE = 200;
const SEARCH_PAGE = 100_000;
const MAX_SEARCH_PAGES = 10;

function markerQuery(path, params) {
  const query = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) query.set(key, String(value));
  return `${path}?${query}`;
}

function updateCachedBookmark(line, marked) {
  const next = new Set(state.bookmarks);
  if (marked) next.add(line);
  else next.delete(line);
  state.bookmarks = next;
}

export async function toggleBookmark(line = state.caret.line) {
  if (!state.stat?.open || state.total === 0) return;
  try {
    await settleEditQueue();
    const res = await apiPost<MarkerMutationResponse, { kind: string; line: number }>(
      "/api/markers/toggle",
      { kind: KIND, line },
    );
    updateCachedBookmark(line, res.marked);
    state.bookmarkCount = res.count;
    scheduleRender();
    flashCount(
      t(res.marked ? "bookmark.added" : "bookmark.removed", {
        line: commas(line + 1),
        count: commas(res.count),
      }),
    );
  } catch (error) {
    flashCount(t("bookmark.error", { msg: error.message }), "error");
  }
}

async function goToBookmark(direction) {
  if (!state.stat?.open || state.total === 0) return;
  try {
    await settleEditQueue();
    const res = await api<MarkerNavigateResponse>(
      markerQuery("/api/markers/navigate", {
        kind: KIND,
        from: state.caret.line,
        direction,
        wrap: true,
      }),
    );
    state.bookmarkCount = res.count;
    if (res.line == null) {
      flashCount(t("bookmark.none"), "error");
      return;
    }
    setCaret(res.line, 0, 0);
    revealCaret();
    await reloadViewport();
    revealCaret();
    render();
    focusEditor();
    if (res.wrapped) flashCount(t("bookmark.wrapped"));
  } catch (error) {
    flashCount(t("bookmark.error", { msg: error.message }), "error");
  }
}

export function nextBookmark() {
  return goToBookmark("next");
}

export function previousBookmark() {
  return goToBookmark("previous");
}

export async function clearBookmarks() {
  if (!state.stat?.open) return;
  let count = state.bookmarkCount;
  try {
    await settleEditQueue();
    if (!count) {
      const list = await api<MarkerListResponse>(
        markerQuery("/api/markers", { kind: KIND, start: 0, limit: 1 }),
      );
      count = list.total;
    }
    if (!count) {
      flashCount(t("bookmark.none"), "error");
      return;
    }
    const confirmed = await askConfirm(
      t("bookmark.clear"),
      t("bookmark.clearConfirm", { count: commas(count) }),
      { okLabel: t("bookmark.clear"), danger: true },
    );
    if (!confirmed) return;
    await apiPost<MarkerMutationResponse, { kind: string }>("/api/markers/clear", { kind: KIND });
    state.bookmarks = new Set();
    state.bookmarkCount = 0;
    scheduleRender();
    flashCount(t("bookmark.cleared", { count: commas(count) }));
  } catch (error) {
    flashCount(t("bookmark.error", { msg: error.message }), "error");
  }
}

export async function bookmarkSearchMatches() {
  if (!state.stat?.open) return;
  if (!state.query || state.regexError) {
    flashCount(t(state.regexError ? "find.regexError" : "bookmark.noSearch"), "error");
    return;
  }
  showLoading(t("bookmark.markingMatches"));
  let start = 0;
  let added = 0;
  let count = state.bookmarkCount;
  let limited = false;
  let sawMatch = false;
  try {
    await settleEditQueue();
    for (let page = 0; page < MAX_SEARCH_PAGES; page++) {
      const result = await api<SearchResponse>(
        `/api/search?${qs()}&start=${start}&max=${SEARCH_PAGE}`,
      );
      if (!result.hits.length) break;
      sawMatch = true;
      const lines = [...new Set(result.hits.map((hit) => hit.line))];
      const mutation = await apiPost<MarkerBulkResponse, { kind: string; lines: number[] }>(
        "/api/markers/add",
        { kind: KIND, lines },
      );
      added += mutation.added;
      count = mutation.count;
      if (mutation.limit_reached) {
        limited = true;
        break;
      }
      if (!result.truncated) break;
      const last = result.hits[result.hits.length - 1];
      const next = last.byte + Math.max(1, last.byte_len);
      if (!Number.isSafeInteger(next) || next <= start) {
        limited = true;
        break;
      }
      start = next;
      limited = page + 1 === MAX_SEARCH_PAGES;
    }
    state.bookmarkCount = count;
    await reloadViewport();
    render();
    if (!sawMatch) {
      flashCount(t("bookmark.noMatches"));
    } else if (limited) {
      flashCount(
        t("bookmark.matchesLimited", {
          added: commas(added),
          count: commas(count),
          max: commas(SEARCH_PAGE * MAX_SEARCH_PAGES),
        }),
      );
    } else {
      flashCount(t("bookmark.matchesAdded", { added: commas(added), count: commas(count) }));
    }
  } catch (error) {
    flashCount(t("bookmark.error", { msg: error.message }), "error");
  } finally {
    hideLoading();
  }
}

function wholeLineSelection(line, lastLength) {
  const start = { line, col: 0 };
  const end = line + 1 < state.total ? { line: line + 1, col: 0 } : { line, col: lastLength ?? 0 };
  // Put the caret at the line start while keeping the trailing newline in the
  // range. This makes the active line and each extra caret stay on the marked
  // line, including for adjacent bookmarks.
  return { anchor: end, head: start };
}

export async function selectBookmarkedLines() {
  if (!state.stat?.open || state.total === 0) return;
  try {
    await settleEditQueue();
    const res = await api<MarkerListResponse>(
      markerQuery("/api/markers", {
        kind: KIND,
        start: 0,
        limit: MAX_BOOKMARK_SELECTIONS + 1,
      }),
    );
    state.bookmarkCount = res.total;
    if (!res.total) {
      flashCount(t("bookmark.none"), "error");
      return;
    }
    if (res.total > MAX_BOOKMARK_SELECTIONS || res.truncated) {
      flashCount(
        t("bookmark.selectLimit", {
          count: commas(res.total),
          max: commas(MAX_BOOKMARK_SELECTIONS),
        }),
        "error",
      );
      return;
    }

    const last = res.lines.includes(state.total - 1) ? state.total - 1 : null;
    const lengths = last == null ? new Map() : await lineLensFor([last]);
    const selections = res.lines.map((line) => ({
      line,
      sel: wholeLineSelection(line, lengths.get(line)),
    }));
    let primary = 0;
    let distance = Number.POSITIVE_INFINITY;
    selections.forEach((entry, index) => {
      const next = Math.abs(entry.line - state.caret.line);
      if (next < distance) {
        primary = index;
        distance = next;
      }
    });
    const chosen = selections[primary];
    state.caret = { line: chosen.line, col: 0 };
    state.activeLine = chosen.line;
    state.sel = chosen.sel;
    state.extraCursors = selections
      .filter((_, index) => index !== primary)
      .map((entry) => ({
        line: entry.line,
        col: 0,
        sel: entry.sel,
      }));
    state.editGen++;
    revealCaret();
    scheduleRender();
    focusEditor();
    flashCount(t("bookmark.selected", { count: commas(res.total) }));
  } catch (error) {
    flashCount(t("bookmark.error", { msg: error.message }), "error");
  }
}

export function hideBookmarkList() {
  setModalOpen($("bookmark-modal"), false);
  focusEditor();
}

async function jumpFromList(line) {
  hideBookmarkList();
  setCaret(line, 0, 0);
  revealCaret();
  await reloadViewport();
  revealCaret();
  render();
  focusEditor();
}

function renderPreviewEntries(entries, append) {
  const list = $("bookmark-list");
  if (!append) list.textContent = "";
  const fragment = document.createDocumentFragment();
  for (const entry of entries) {
    const row = document.createElement("button");
    row.type = "button";
    row.className = "bookmark-list-row";
    row.dataset.line = String(entry.line);
    row.setAttribute("role", "option");
    row.setAttribute(
      "aria-label",
      t("bookmark.previewLabel", { line: commas(entry.line + 1), text: entry.text }),
    );
    const number = document.createElement("span");
    number.className = "bookmark-list-line";
    number.textContent = commas(entry.line + 1);
    const text = document.createElement("span");
    text.className = "bookmark-list-text";
    text.textContent = `${entry.text}${entry.truncated ? "…" : ""}`;
    row.append(number, text);
    row.addEventListener("click", () => void jumpFromList(entry.line));
    fragment.append(row);
  }
  list.append(fragment);
}

async function loadBookmarkPreviews(start = 0, append = false) {
  const res = await api<MarkerPreviewResponse>(
    markerQuery("/api/markers/previews", {
      kind: KIND,
      start,
      limit: PREVIEW_PAGE,
    }),
  );
  state.bookmarkCount = res.total;
  renderPreviewEntries(res.entries, append);
  $("bookmark-summary").textContent = t("bookmark.summary", { count: commas(res.total) });
  const more = $("bookmark-more") as HTMLButtonElement;
  more.classList.toggle("hidden", !res.truncated || !res.entries.length);
  more.dataset.start = res.entries.length
    ? String(res.entries[res.entries.length - 1].line + 1)
    : "";
  $("bookmark-empty").classList.toggle("hidden", res.total !== 0);
}

export async function showBookmarkList() {
  if (!state.stat?.open) return;
  await settleEditQueue();
  setModalOpen($("bookmark-modal"), true);
  $("bookmark-list").textContent = "";
  $("bookmark-summary").textContent = t("bookmark.loading");
  $("bookmark-empty").classList.add("hidden");
  $("bookmark-more").classList.add("hidden");
  try {
    await loadBookmarkPreviews();
    queueMicrotask(() => $("bookmark-list").querySelector("button")?.focus());
  } catch (error) {
    $("bookmark-summary").textContent = t("bookmark.error", { msg: error.message });
  }
}

function gutterLine(target) {
  const gutter = (target as HTMLElement | null)?.closest?.(".ln");
  const row = gutter?.closest?.(".row") as HTMLElement | null;
  if (!gutter || !row) return null;
  const line = Number(row.dataset.line);
  return Number.isSafeInteger(line) && line >= 0 ? line : null;
}

function showGutterMenu(event, line) {
  const marked = state.bookmarks.has(line);
  showPopupMenu(event.clientX, event.clientY, [
    {
      label: t(marked ? "bookmark.remove" : "bookmark.add"),
      action: () => void toggleBookmark(line),
    },
    { separator: true },
    { label: t("bookmark.next"), action: () => void nextBookmark() },
    { label: t("bookmark.previous"), action: () => void previousBookmark() },
    { label: t("bookmark.showList"), action: () => void showBookmarkList() },
    { label: t("bookmark.addMatches"), action: () => void bookmarkSearchMatches() },
    { label: t("bookmark.selectAll"), action: () => void selectBookmarkedLines() },
    { separator: true },
    {
      label: t("bookmark.clear"),
      disabled: state.bookmarkCount === 0 && state.bookmarks.size === 0,
      action: () => void clearBookmarks(),
    },
  ]);
}

export function initBookmarks() {
  const content = $("content");
  // Capture before the editor's selection mousedown handler: clicking the
  // gutter is a marker command, never a caret drag.
  content.addEventListener(
    "mousedown",
    (event) => {
      if (gutterLine(event.target) == null) return;
      event.preventDefault();
      event.stopPropagation();
    },
    true,
  );
  content.addEventListener("click", (event) => {
    const line = gutterLine(event.target);
    if (line == null) return;
    event.preventDefault();
    event.stopPropagation();
    void toggleBookmark(line);
  });
  content.addEventListener("keydown", (event) => {
    if (event.key !== "Enter" && event.key !== " ") return;
    const line = gutterLine(event.target);
    if (line == null) return;
    event.preventDefault();
    event.stopPropagation();
    void toggleBookmark(line);
  });
  content.addEventListener(
    "contextmenu",
    (event) => {
      const line = gutterLine(event.target);
      if (line == null) return;
      event.preventDefault();
      event.stopPropagation();
      showGutterMenu(event, line);
    },
    true,
  );

  $("bookmark-close").addEventListener("click", hideBookmarkList);
  $("bookmark-modal").addEventListener("click", (event) => {
    if (event.target === $("bookmark-modal")) hideBookmarkList();
  });
  $("bookmark-modal").addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      event.preventDefault();
      hideBookmarkList();
    }
  });
  $("bookmark-more").addEventListener("click", () => {
    const start = Number(($("bookmark-more") as HTMLElement).dataset.start);
    if (Number.isSafeInteger(start) && start >= 0) void loadBookmarkPreviews(start, true);
  });

  // A tab/document switch clears the viewport cache in editor.ts; the modal
  // itself never survives that transition as an active editor operation.
  if (modalVisible("bookmark-modal")) hideBookmarkList();
}
