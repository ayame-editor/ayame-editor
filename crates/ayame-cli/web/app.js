// Ayame Editor front-end.
//
// Design rule: the browser never holds more than the visible window. Lines are
// fetched on demand from the local server; vertical position is tracked as a
// *line number* (not pixels), so navigation is exact for any file size — ten
// lines or Ayame Editor's minimum ten-billion-line scale. A custom scrollbar maps line
// position to a thumb, side-stepping the browser's ~33M-pixel element-height
// ceiling entirely.

const $ = (id) => document.getElementById(id);
let LINE_HEIGHT = 18; // tracks --line-height; updated by Settings (font size)
const OVERSCAN = 6;
const PAD = 400; // extra lines fetched around the viewport and cached
const SEARCH_HISTORY_KEY = "ayame.searchHistory.v1";
const SETTINGS_KEY = "ayame.settings.v1";
const TREE_KEY = "ayame.treeRoot.v1";
const MAX_COPY_LINES = 20000; // clipboard cap: copy warns, cut refuses beyond this

const FONT_STACKS = {
  mono: '"SFMono-Regular","Menlo","Consolas","DejaVu Sans Mono",monospace',
  "mono-jp": '"Consolas","Menlo","Noto Sans Mono CJK JP","MS Gothic",monospace',
  system: '"Segoe UI","Hiragino Kaku Gothic ProN","Noto Sans JP",system-ui,sans-serif',
};
const DEFAULT_SETTINGS = {
  theme: "iris-light",
  font: "mono",
  fontSize: 13,
  sidebar: false,
  sidebarSide: "left",
  ruler: true,
  bgMode: "watercolor",
  illus: null,
  keymap: {},
  customThemes: {},
};

const KEYMAP_ACTIONS = [
  ["newFile", "新規テキスト", "Ctrl+N"],
  ["openFile", "開く", "Ctrl+O"],
  ["saveFile", "保存", "Ctrl+S"],
  ["saveAs", "別名で保存", "Ctrl+Shift+S"],
  ["closeTab", "タブを閉じる", "Ctrl+W"],
  ["toggleSidebar", "エクスプローラー表示", "Ctrl+B"],
  ["find", "検索", "Ctrl+F"],
  ["findNext", "次の一致", "F3"],
  ["findPrev", "前の一致", "Shift+F3"],
  ["gotoLine", "行へ移動", "Ctrl+G"],
  ["undo", "元に戻す", "Ctrl+Z"],
  ["redo", "やり直す", ["Ctrl+Y", "Ctrl+Shift+Z"]],
  ["selectAll", "すべて選択", "Ctrl+A"],
  ["addCursorAbove", "カーソルを上に追加", "Ctrl+Alt+ArrowUp"],
  ["addCursorBelow", "カーソルを下に追加", "Ctrl+Alt+ArrowDown"],
  ["copy", "コピー", "Ctrl+C"],
  ["cut", "切り取り", "Ctrl+X"],
  ["searchCase", "検索: 大文字小文字", "Alt+C"],
  ["searchWord", "検索: 単語単位", "Alt+W"],
  ["searchRegex", "検索: 正規表現", "Alt+R"],
  ["sortSave", "ソート", ""],
  ["replaceSave", "置換して保存", ""],
  ["diffFile", "2ファイル差分", ""],
];
const DEFAULT_KEYMAP = Object.fromEntries(KEYMAP_ACTIONS.map(([id, _label, shortcut]) => [id, shortcut]));

const state = {
  total: 0,
  first: 0, // top visible line (0-based)
  fracAcc: 0, // sub-line wheel accumulator
  cache: { start: 0, lines: [] },
  loadToken: 0,
  stat: null,
  // search
  query: "",
  regex: false,
  ci: false,
  word: false,
  matcher: null,
  regexError: false,
  activeLine: -1,
  lastMatch: null, // { byte, len }
  searchHits: null,
  searchTruncated: false,
  findOpen: false,
  history: [],
  historyIndex: -1,
  settings: { ...DEFAULT_SETTINGS },
  tabs: [], // open tabs from /api/tabs
  treeParent: null, // parent of the current tree root (for the "up" button)
  treeLoaded: false,
  openerDir: null, // directory currently shown in the open dialog
  openerMode: "open", // "open" | "save"
  openerEntries: [],
  openerResolve: null,
  // ---- caret-based (Notepad-style) editing ----
  caret: { line: 0, col: 0 }, // (line, col) in Unicode scalars, like the backend
  goalCol: 0, // remembered column for vertical caret motion
  editGen: 0, // bumps on every user caret move; lets an in-flight edit detect it
  docGen: 0, // bumps whenever the active document/tab changes; cancels stale queued edits
  composing: false, // an IME composition is in progress
  focused: false, // the hidden text input holds focus (draw the caret)
  sel: null, // selection: { anchor: {line,col}, head: {line,col}, rect?: bool } or null
  extraCursors: [], // multi-cursor: additional carets [{line,col}]; primary is state.caret
  dragging: false,
  dragMoved: false,
  dragAnchor: null, // caret at mouse-down, promoted to a selection once it moves
  dragRect: false,
};

const pool = [];
let renderQueued = false;
let lastNativeTitle = "";

// ---- tiny helpers -----------------------------------------------------------

async function api(path) {
  const r = await fetch(path);
  if (!r.ok) throw new Error((await r.text()) || r.statusText);
  return r.json();
}

async function apiPost(path, body = {}) {
  const r = await fetch(path, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!r.ok) throw new Error((await r.text()) || r.statusText);
  return r.json();
}

function commas(n) {
  return n.toLocaleString("en-US");
}

function humanBytes(n) {
  const u = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
  let v = n,
    i = 0;
  while (v >= 1024 && i < u.length - 1) {
    v /= 1024;
    i++;
  }
  return i === 0 ? `${n} B` : `${v.toFixed(2)} ${u[i]}`;
}

function escapeRegExp(s) {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

// Show/hide one modal element, keeping the .hidden class and aria-hidden in
// step (every modal in the app pairs the two).
function setModalOpen(modal, open) {
  modal.classList.toggle("hidden", !open);
  modal.setAttribute("aria-hidden", open ? "false" : "true");
}

const APP_MENUS = ["file", "edit", "selection", "view", "tools"];

function fileMenuVisible() {
  return APP_MENUS.some((id) => !$(`${id}-menu`).classList.contains("hidden"));
}

function showAppMenu(id) {
  hideFileMenu();
  $(`${id}-menu`).classList.remove("hidden");
  $(`${id}-menu-button`).classList.add("on");
  $(`${id}-menu-button`).setAttribute("aria-expanded", "true");
}

function hideFileMenu(focusButton = false) {
  let focused = false;
  for (const id of APP_MENUS) {
    const menu = $(`${id}-menu`);
    const button = $(`${id}-menu-button`);
    const wasOpen = !menu.classList.contains("hidden");
    menu.classList.add("hidden");
    button.classList.remove("on");
    button.setAttribute("aria-expanded", "false");
    if (focusButton && wasOpen && !focused) {
      button.focus();
      focused = true;
    }
  }
}

function normalizeShortcut(raw) {
  if (!raw) return "";
  const parts = String(raw).split("+").map((p) => p.trim()).filter(Boolean);
  const mods = { Ctrl: false, Shift: false, Alt: false };
  let key = "";
  for (const part of parts) {
    const low = part.toLowerCase();
    if (low === "ctrl" || low === "control" || low === "cmd" || low === "command" || low === "meta") mods.Ctrl = true;
    else if (low === "shift") mods.Shift = true;
    else if (low === "alt" || low === "option") mods.Alt = true;
    else key = part.length === 1 ? part.toUpperCase() : part[0].toUpperCase() + part.slice(1);
  }
  if (!key || ["Ctrl", "Shift", "Alt"].includes(key)) return "";
  return [mods.Ctrl && "Ctrl", mods.Shift && "Shift", mods.Alt && "Alt", key].filter(Boolean).join("+");
}

function isBindableShortcut(shortcut) {
  if (!shortcut) return true;
  const parts = shortcut.split("+");
  const key = parts[parts.length - 1];
  return parts.includes("Ctrl") || parts.includes("Alt") || /^F\d+$/i.test(key);
}

function sanitizeKeymap(raw) {
  const src = raw && typeof raw === "object" ? raw : {};
  const clean = {};
  for (const [action] of KEYMAP_ACTIONS) {
    if (!Object.prototype.hasOwnProperty.call(src, action)) continue;
    if (Array.isArray(src[action])) {
      clean[action] = src[action].map(normalizeShortcut).filter((v) => v && isBindableShortcut(v));
    } else {
      const v = normalizeShortcut(src[action]);
      clean[action] = isBindableShortcut(v) ? v : "";
    }
  }
  return clean;
}

function eventShortcut(e) {
  if (["Control", "Shift", "Alt", "Meta"].includes(e.key)) return "";
  let key = e.key;
  if (key.length === 1) key = key.toUpperCase();
  else if (/^f\d+$/i.test(key)) key = key.toUpperCase();
  else key = key[0].toUpperCase() + key.slice(1);
  return [
    (e.ctrlKey || e.metaKey) && "Ctrl",
    e.shiftKey && "Shift",
    e.altKey && "Alt",
    key,
  ].filter(Boolean).join("+");
}

function shortcutList(action) {
  const custom = state.settings.keymap && Object.prototype.hasOwnProperty.call(state.settings.keymap, action)
    ? state.settings.keymap[action]
    : DEFAULT_KEYMAP[action];
  const list = Array.isArray(custom) ? custom : [custom];
  return list.map(normalizeShortcut).filter(Boolean);
}

function shortcutFor(action) {
  return shortcutList(action)[0] || "";
}

function matchesShortcut(e, action) {
  const ev = eventShortcut(e);
  return !!ev && shortcutList(action).includes(ev);
}

function postNativeMessage(msg) {
  try {
    if (window.ipc && typeof window.ipc.postMessage === "function") {
      window.ipc.postMessage(msg);
    }
  } catch {
    // The web build has no native IPC; title/close still work in the browser.
  }
}

function setAppTitle(title) {
  const next = title || "Ayame Editor";
  document.title = next;
  if (lastNativeTitle !== next) {
    lastNativeTitle = next;
    postNativeMessage(`ayame:title:${next}`);
  }
}

function dirtyTabNames() {
  const names = [];
  for (const t of state.tabs || []) {
    if (t.dirty && t.name) names.push(t.name);
  }
  if (state.stat?.dirty && names.length === 0) names.push(displayName(state.stat.path));
  return [...new Set(names)].filter(Boolean);
}

function hasDirtyDocuments() {
  return !!state.stat?.dirty || dirtyTabNames().length > 0;
}

function confirmCloseWorkspace() {
  const dirty = dirtyTabNames();
  if (!hasDirtyDocuments()) return true;
  const shown = dirty.slice(0, 5).join(", ");
  const more = dirty.length > 5 ? ` ほか ${dirty.length - 5} 件` : "";
  const suffix = shown ? `\n\n${shown}${more}` : "";
  return confirm(`未保存の編集があります。保存せず終了しますか?${suffix}`);
}

// Never let the native window kill the process while a save is in flight; the
// close request is answered "cancel" and retried once the save settles.
// While saving: key edits are blocked (onEditKey) and IME/beforeinput commits
// wait inside enqueueEdit so confirmed text is delayed, not lost.
let savingCount = 0;
let savingWaiters = [];
function setSavingUI() {
  const on = savingCount > 0;
  document.documentElement.classList.toggle("saving", on);
  $("st-saving")?.classList.toggle("hidden", !on);
  if (!on && savingWaiters.length) {
    const waiters = savingWaiters;
    savingWaiters = [];
    for (const resolve of waiters) resolve();
  }
}
function waitForSavingDone() {
  if (savingCount === 0) return Promise.resolve();
  return new Promise((resolve) => savingWaiters.push(resolve));
}
let pendingNativeClose = false;
window.__ayameNativeCloseRequested = () => {
  if (savingCount > 0) {
    pendingNativeClose = true;
    flashCount("保存処理中です。完了後に閉じます…");
    postNativeMessage("ayame:close-cancel");
    return;
  }
  postNativeMessage(confirmCloseWorkspace() ? "ayame:close-ok" : "ayame:close-cancel");
};

function retryPendingNativeClose() {
  if (pendingNativeClose && savingCount === 0) {
    pendingNativeClose = false;
    window.__ayameNativeCloseRequested();
  }
}

window.addEventListener("beforeunload", (e) => {
  if (!hasDirtyDocuments()) return;
  e.preventDefault();
  e.returnValue = "";
});

function setKeymap(action, shortcut) {
  const normalized = normalizeShortcut(shortcut);
  if (normalized && !isBindableShortcut(normalized)) {
    flashCount("文字入力と衝突するキーは使えません");
    return;
  }
  state.settings = {
    ...state.settings,
    keymap: { ...(state.settings.keymap || {}), [action]: normalized },
  };
  saveSettings(state.settings);
  updateKeyHints();
  renderKeymapRows();
}

function resetKeymap() {
  state.settings = { ...state.settings, keymap: {} };
  saveSettings(state.settings);
  updateKeyHints();
  renderKeymapRows();
}

function updateKeyHints() {
  document.querySelectorAll("[data-key-action]").forEach((el) => {
    el.textContent = shortcutFor(el.dataset.keyAction);
  });
  const hint = (label, action) => {
    const key = shortcutFor(action);
    return key ? `${label} (${key})` : label;
  };
  $("toggle-sidebar").title = hint("エクスプローラー", "toggleSidebar");
  $("undo-edit").title = hint("元に戻す", "undo");
  $("redo-edit").title = hint("やり直す", "redo");
  $("find").placeholder = hint("検索", "find");
  $("find-prev").title = hint("前の一致", "findPrev");
  $("find-next").title = hint("次の一致", "findNext");
  $("opt-case").title = hint("大文字小文字を区別", "searchCase");
  $("opt-word").title = hint("単語単位", "searchWord");
  $("opt-regex").title = hint("正規表現", "searchRegex");
  $("new-tab").title = hint("新規タブ", "newFile");
}

function keymapVisible() {
  return !$("keymap-modal").classList.contains("hidden");
}

function showKeymap() {
  hideSettings();
  renderKeymapRows();
  setModalOpen($("keymap-modal"), true);
  queueMicrotask(() => $("keymap-list").querySelector("input")?.focus());
}

function hideKeymap() {
  setModalOpen($("keymap-modal"), false);
  focusEditor();
}

function renderKeymapRows() {
  const list = $("keymap-list");
  if (!list) return;
  const used = new Map();
  for (const [action] of KEYMAP_ACTIONS) {
    for (const key of shortcutList(action)) used.set(key, (used.get(key) || 0) + 1);
  }
  list.textContent = "";
  const frag = document.createDocumentFragment();
  for (const [action, label] of KEYMAP_ACTIONS) {
    const row = document.createElement("label");
    const shortcut = shortcutFor(action);
    row.className = "keymap-row";
    if (shortcut && used.get(shortcut) > 1) row.classList.add("conflict");
    const name = document.createElement("span");
    name.className = "keymap-label";
    name.textContent = label;
    const input = document.createElement("input");
    input.className = "keymap-input";
    input.readOnly = true;
    input.value = shortcut;
    input.placeholder = "未設定";
    input.dataset.action = action;
    input.addEventListener("keydown", (e) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape") { hideKeymap(); return; }
      if (e.key === "Backspace" || e.key === "Delete") { setKeymap(action, ""); return; }
      const shortcut = eventShortcut(e);
      if (shortcut) setKeymap(action, shortcut);
    });
    row.append(name, input);
    frag.append(row);
  }
  list.append(frag);
}

function rowsVisible() {
  const h = $("viewport").clientHeight - (state.settings && state.settings.ruler ? 18 : 0);
  return Math.max(1, Math.ceil(h / LINE_HEIGHT));
}

// ---- column ruler ----------------------------------------------------------

let _rulerKey = "";
function buildRuler() {
  const vp = $("viewport");
  if (!state.settings.ruler) {
    vp.classList.remove("has-ruler");
    return;
  }
  vp.classList.add("has-ruler");
  // Gutter width, measured from a visible row so ticks line up with the text.
  let gutterPx = 0;
  for (const row of pool) {
    if (row.style.display !== "none") {
      gutterPx = row.firstChild.getBoundingClientRect().width;
      break;
    }
  }
  const cw = charWidth();
  const inner = $("ruler-inner");
  const key = `${Math.round(gutterPx)}|${cw.toFixed(2)}`;
  if (key !== _rulerKey && gutterPx > 0) {
    _rulerKey = key;
    $("ruler-corner").style.width = `${gutterPx}px`;
    inner.textContent = "";
    for (let c = 10; c <= 500; c += 10) {
      const t = document.createElement("span");
      t.className = "rtick";
      t.style.left = `${gutterPx + c * cw}px`;
      t.textContent = String(c);
      inner.append(t);
    }
  }
  inner.style.transform = `translateX(${-$("content").scrollLeft}px)`;
}

function maxFirst() {
  return Math.max(0, state.total - rowsVisible());
}

// ---- data ------------------------------------------------------------------

function cachedLine(line) {
  const c = state.cache;
  const i = line - c.start;
  return i >= 0 && i < c.lines.length ? c.lines[i] : null;
}

function ensureData(start, count) {
  const need0 = start;
  const need1 = Math.min(state.total, start + count);
  const c = state.cache;
  if (c.lines.length && need0 >= c.start && need1 <= c.start + c.lines.length) {
    return; // already cached
  }
  const fstart = Math.max(0, start - PAD);
  const fcount = count + 2 * PAD;
  const token = ++state.loadToken;
  api(`/api/lines?start=${fstart}&count=${fcount}`)
    .then((res) => {
      if (token !== state.loadToken) return; // a newer request superseded us
      state.cache = { start: fstart, lines: res.lines };
      state.total = res.total;
      render();
    })
    .catch((e) => console.error("lines fetch failed", e));
}

async function lineByte(line) {
  try {
    const r = await api(`/api/linebyte?line=${Math.max(0, line)}`);
    return r.byte ?? 0;
  } catch {
    return 0;
  }
}

// ---- rendering -------------------------------------------------------------

function ensurePool(count) {
  const content = $("content");
  while (pool.length < count) {
    const row = document.createElement("div");
    row.className = "row";
    const ln = document.createElement("span");
    ln.className = "ln";
    const tx = document.createElement("span");
    tx.className = "tx";
    row.append(ln, tx);
    // Mouse selection/caret is handled at the #content level (see initSelection),
    // so it works uniformly across the pooled rows and beyond the viewport.
    content.append(row);
    pool.push(row);
  }
}

function fillRow(row, line, rec, gutterWidth) {
  const ln = row.firstChild;
  const tx = row.lastChild;
  row.className = "row";
  row.dataset.line = String(line);
  ln.textContent = String(line + 1).padStart(gutterWidth, " ");
  tx.textContent = "";
  tx.classList.remove("pending");
  row.classList.toggle("inserted", !!rec?.inserted);
  if (rec == null) {
    tx.classList.add("pending");
    tx.textContent = "⋯";
  } else if (state.matcher) {
    appendHighlighted(tx, rec.text);
  } else {
    tx.textContent = rec.text;
  }
  // Hide the current-line highlight while a selection exists — the two
  // washes stack otherwise and the selection becomes hard to read.
  row.classList.toggle("active", line === state.activeLine && !hasSelection());
}

function fillEofRow(row) {
  row.className = "row eof";
  row.dataset.line = "-1";
  row.firstChild.textContent = "";
  const tx = row.lastChild;
  tx.className = "tx";
  tx.textContent = "[EOF]";
}

// Character width of the monospace content font, for a rough fallback only
// (real caret/selection geometry is measured from the actual glyphs below, so
// CJK, tabs and proportional fallbacks all line up).
let _charW = 0;
function charWidth() {
  if (_charW) return _charW;
  _charW = measureTextWidth("0".repeat(100)) / 100 || 8;
  return _charW;
}

// Measure the rendered pixel width of `str` in the content font. One reused,
// hidden probe kept inside #content so it inherits the exact font metrics.
let _measSpan = null;
function measureTextWidth(str) {
  if (!str) return 0;
  if (!_measSpan) {
    _measSpan = document.createElement("span");
    _measSpan.style.cssText =
      "position:absolute;visibility:hidden;white-space:pre;top:-9999px;left:0;pointer-events:none;";
    $("content").appendChild(_measSpan);
  }
  _measSpan.textContent = str;
  return _measSpan.getBoundingClientRect().width;
}

// Unicode-scalar view of a line's text (matches the backend's char columns).
function lineChars(line) {
  return Array.from(cachedLine(line)?.text ?? "");
}
function lineLen(line) {
  return lineChars(line).length;
}

// Pixel x (in #content coordinates, gutter included) of column `col` on `line`.
function caretX(line, col) {
  const cs = lineChars(line);
  const head = cs.slice(0, Math.max(0, Math.min(col, cs.length))).join("");
  return gutterPixels() + measureTextWidth(head);
}

// Inverse of caretX: nearest column boundary to pixel x (content coordinates).
function colFromX(line, x) {
  const cs = lineChars(line);
  const local = x - gutterPixels();
  if (local <= 0) return 0;
  const full = measureTextWidth(cs.join(""));
  if (local >= full) return cs.length;
  let lo = 0,
    hi = cs.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (measureTextWidth(cs.slice(0, mid).join("")) < local) lo = mid + 1;
    else hi = mid;
  }
  const wLo = measureTextWidth(cs.slice(0, lo).join(""));
  const wPrev = lo > 0 ? measureTextWidth(cs.slice(0, lo - 1).join("")) : 0;
  return local - wPrev < wLo - local ? lo - 1 : lo;
}

// ---- selection (multi-line, coordinate-based) ------------------------------

// Width in px of the line-number gutter, measured from a visible row.
function gutterPixels() {
  for (const row of pool) {
    if (row.style.display !== "none" && row.firstChild) {
      return row.firstChild.getBoundingClientRect().width;
    }
  }
  return 7 * charWidth() + 29; // fallback: 8 + 20 padding + 1 border
}

// Map a mouse event to a {line, col} position in the document.
function coordsFromEvent(e) {
  const content = $("content");
  const rect = content.getBoundingClientRect();
  const rowInView = Math.floor((e.clientY - rect.top) / LINE_HEIGHT);
  let line = state.first + Math.max(0, rowInView);
  line = Math.max(0, Math.min(line, Math.max(0, state.total - 1)));
  const x = e.clientX - rect.left + content.scrollLeft; // #content coordinates
  return { line, col: colFromX(line, x) };
}

// Normalized selection: { start, end } with start <= end, or null.
function selRange() {
  if (!state.sel) return null;
  const { anchor: a, head: h } = state.sel;
  const forward = a.line < h.line || (a.line === h.line && a.col <= h.col);
  const r = forward ? { start: a, end: h } : { start: h, end: a };
  r.rect = !!state.sel.rect;
  return r;
}

function rectRange() {
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

function hasSelection() {
  const rr = rectRange();
  if (rr) return rr.l0 !== rr.l1 || rr.c0 !== rr.c1;
  const r = selRange();
  return !!r && !(r.start.line === r.end.line && r.start.col === r.end.col);
}

// Like hasSelection(), but a zero-width rect (c0 == c1 across several lines)
// counts as empty: it selects no characters, so text-producing actions
// (copy / cut / save-selection) treat it as "no selection".
function hasTextSelection() {
  const rr = rectRange();
  if (rr) return rr.c0 !== rr.c1;
  return hasSelection();
}

function renderSelection() {
  const layer = $("sel-layer");
  layer.textContent = "";
  const rr = rectRange();
  if (rr) {
    const vis = rowsVisible() + OVERSCAN;
    const from = Math.max(rr.l0, state.first);
    const to = Math.min(rr.l1, state.first + vis);
    for (let line = from; line <= to; line++) {
      const left = caretX(line, rr.c0);
      const width = Math.max(2, caretX(line, rr.c1) - left);
      const rect = document.createElement("div");
      rect.className = "selrect";
      rect.style.left = `${left}px`;
      rect.style.top = `${(line - state.first) * LINE_HEIGHT}px`;
      rect.style.width = `${width}px`;
      layer.append(rect);
    }
    return;
  }
  const r = selRange();
  if (!r || (r.start.line === r.end.line && r.start.col === r.end.col)) return;
  const cw = charWidth();
  const vis = rowsVisible() + OVERSCAN;
  const from = Math.max(r.start.line, state.first);
  const to = Math.min(r.end.line, state.first + vis);
  for (let line = from; line <= to; line++) {
    const startCol = line === r.start.line ? r.start.col : 0;
    const len = lineLen(line);
    // A line selected through its end extends a hair past the text so the
    // trailing newline reads as selected, like a normal editor.
    const endCol = line === r.end.line ? Math.min(r.end.col, len) : len;
    const trail = line === r.end.line ? 0 : cw * 0.6;
    const left = caretX(line, startCol);
    const width = caretX(line, endCol) - left + trail;
    const rect = document.createElement("div");
    rect.className = "selrect";
    rect.style.left = `${left}px`;
    rect.style.top = `${(line - state.first) * LINE_HEIGHT}px`;
    rect.style.width = `${Math.max(2, width)}px`;
    layer.append(rect);
  }
}

function initSelection() {
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
function selectLineAt(line) {
  if (state.total === 0) return;
  const l = Math.max(0, Math.min(line, state.total - 1));
  const hasNext = l + 1 < state.total;
  const head = hasNext ? { line: l + 1, col: 0 } : { line: l, col: lineLen(l) };
  state.sel = { anchor: { line: l, col: 0 }, head };
  setCaret(head.line, head.col);
  focusEditor();
  scheduleRender();
}

// ---- editor context menu ----------------------------------------------------

function ctxMenuVisible() {
  return !$("ctx-menu").classList.contains("hidden");
}

function hideCtxMenu() {
  $("ctx-menu").classList.add("hidden");
}

function posInsideSelection(p) {
  const rr = rectRange();
  if (rr) return p.line >= rr.l0 && p.line <= rr.l1 && p.col >= rr.c0 && p.col <= rr.c1;
  const r = selRange();
  if (!r) return false;
  if (p.line < r.start.line || p.line > r.end.line) return false;
  if (p.line === r.start.line && p.col < r.start.col) return false;
  if (p.line === r.end.line && p.col > r.end.col) return false;
  return true;
}

async function pasteFromClipboard() {
  try {
    const text = await navigator.clipboard.readText();
    if (text) pasteText(text);
  } catch {
    // Clipboard read needs a permission some webviews withhold; the keyboard
    // path (paste event on the hidden textarea) always works.
    flashCount("ここからは貼り付けできません — Ctrl+V を使ってください", "error");
  }
  focusEditor();
}

// Save the selected lines to a file server-side: streamed in batches, so the
// clipboard cap does not apply. Output matches what copy would produce.
async function saveSelectionToFile() {
  const rr = rectRange();
  const r = selRange();
  if ((!rr && !r) || !hasTextSelection()) {
    // A zero-width rect selects no characters — nothing to write out.
    flashCount("選択がありません", "error");
    return;
  }
  const total = rr ? rr.l1 - rr.l0 + 1 : r.end.line - r.start.line + 1;
  const base = state.stat?.path || "selection";
  const f = await askForm("選択箇所をファイルに保存", [
    { id: "path", type: "text", label: "保存先パス", value: `${base}.selection.txt` },
    { id: "_hint", type: "hint",
      label: `選択中の ${commas(total)} 行を UTF-8 / LF で書き出します。コピーの行数上限 (${commas(MAX_COPY_LINES)} 行) はかかりません。` },
  ], "保存");
  if (!f || !f.path.trim()) return;
  const body = rr
    ? { path: f.path.trim(), rect: true, l0: rr.l0, c0: rr.c0, l1: rr.l1, c1: rr.c1 }
    : { path: f.path.trim(), rect: false, l0: r.start.line, c0: r.start.col, l1: r.end.line, c1: r.end.col };
  showLoading("選択を書き出し中…");
  try {
    const res = await apiPost("/api/selection/save", body);
    flashCount(`選択 ${commas(res.lines)} 行を保存しました: ${res.path}`);
  } catch (e) {
    if (String(e.message || "").includes("既に存在")) {
      if (confirm(`${f.path.trim()} は既に存在します。上書きしますか?`)) {
        try {
          const res = await apiPost("/api/selection/save", { ...body, overwrite: true });
          flashCount(`選択 ${commas(res.lines)} 行を保存しました: ${res.path}`);
        } catch (e2) {
          flashCount("選択の保存エラー", "error");
          alert(e2.message);
        }
      }
    } else {
      flashCount("選択の保存エラー", "error");
      alert(e.message);
    }
  } finally {
    hideLoading();
  }
}

function runCtxAction(action) {
  hideCtxMenu();
  // Only the two context-menu-specific actions live here; everything else
  // (cut / copy / selectAll / find / sortSave / replaceSave / diffFile /
  // splitFile) shares the menu dispatcher.
  let out;
  if (action === "paste") out = pasteFromClipboard();
  else if (action === "saveSelection") out = saveSelectionToFile();
  else out = runMenuAction(action);
  // A context-menu click leaves focus on the (now hidden) menu item, killing
  // keyboard input after cut/copy etc. Put focus back in the editor once the
  // action settles — unless it opened its own focus target (a modal, or the
  // find bar).
  return Promise.resolve(out).finally(() => {
    if (!anyModalOpen() && !state.findOpen) focusEditor();
  });
}

function initContextMenu() {
  const menu = $("ctx-menu");
  $("viewport").addEventListener("contextmenu", (e) => {
    e.preventDefault();
    if (!state.stat?.open || anyModalOpen()) return;
    // Right-click inside the selection keeps it as the action target;
    // outside it, the caret moves to the click point first (editor standard).
    const p = coordsFromEvent(e);
    if (!posInsideSelection(p)) {
      state.sel = null;
      setCaret(p.line, p.col);
      scheduleRender();
    }
    // Zero-width rect selections count as empty for the text actions.
    const hasSel = hasTextSelection();
    menu.querySelectorAll("[data-ctx]").forEach((el) => {
      const a = el.dataset.ctx;
      el.disabled = (a === "cut" || a === "copy" || a === "saveSelection") && !hasSel;
    });
    menu.classList.remove("hidden");
    const mw = menu.offsetWidth;
    const mh = menu.offsetHeight;
    menu.style.left = `${Math.max(4, Math.min(e.clientX, window.innerWidth - mw - 8))}px`;
    menu.style.top = `${Math.max(4, Math.min(e.clientY, window.innerHeight - mh - 8))}px`;
  });
  menu.querySelectorAll("[data-ctx]").forEach((el) => {
    el.addEventListener("click", () => runCtxAction(el.dataset.ctx));
  });
  document.addEventListener("pointerdown", (e) => {
    if (ctxMenuVisible() && !e.target.closest("#ctx-menu")) hideCtxMenu();
  });
}

function selectAll() {
  if (state.total === 0) return;
  const last = state.total - 1;
  state.sel = {
    anchor: { line: 0, col: 0 },
    head: { line: last, col: lineLen(last) },
  };
  setCaret(last, lineLen(last));
  focusEditor();
}

function selectionLineCount(r) {
  const rr = rectRange();
  if (rr) return rr.l1 - rr.l0 + 1;
  return r.end.line - r.start.line + 1;
}

// Fetch the selected text (bounded) and join with newlines.
async function selectedText(r) {
  const rr = rectRange();
  if (rr) {
    const count = Math.min(rr.l1 - rr.l0 + 1, MAX_COPY_LINES);
    const res = await api(`/api/lines?start=${rr.l0}&count=${count}`);
    return res.lines.map((x) => {
      const chars = Array.from(x.text ?? "");
      return chars.slice(rr.c0, rr.c1).join("");
    }).join("\n");
  }
  const count = Math.min(r.end.line - r.start.line + 1, MAX_COPY_LINES);
  const res = await api(`/api/lines?start=${r.start.line}&count=${count}`);
  // Columns are Unicode scalar counts (the server contract); slicing UTF-16
  // units here would split surrogate pairs (emoji etc.).
  const L = res.lines.map((x) => Array.from(x.text ?? ""));
  if (L.length === 1) return L[0].slice(r.start.col, r.end.col).join("");
  const out = [L[0].slice(r.start.col).join("")];
  for (let i = 1; i < L.length - 1; i++) out.push(L[i].join(""));
  out.push(L[L.length - 1].slice(0, r.end.col).join(""));
  return out.join("\n");
}

async function copyToClipboard(text) {
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

async function copySelection() {
  const r = selRange();
  if (!r) return;
  try {
    const total = selectionLineCount(r);
    await copyToClipboard(await selectedText(r));
    if (total > MAX_COPY_LINES) {
      flashCount(
        `コピーは先頭 ${commas(MAX_COPY_LINES)} 行まで — 残り ${commas(total - MAX_COPY_LINES)} 行はコピーされていません。全体は右クリック→「選択箇所をファイルに保存」で書き出せます`,
        "error"
      );
    } else {
      flashCount("コピーしました");
    }
  } catch (e) {
    flashCount("コピーエラー", "error");
    console.error(e);
  }
}

function deleteSelection() {
  if (!hasSelection()) return;
  typeText(""); // replace the selection with nothing
}

async function cutSelection() {
  const r = selRange();
  if (!r || !hasSelection()) return;
  // Never delete more than what reached the clipboard: a capped copy followed
  // by a full delete would silently destroy data.
  const total = selectionLineCount(r);
  if (total > MAX_COPY_LINES) {
    flashCount(
      `切り取りは ${commas(MAX_COPY_LINES)} 行まで (選択は ${commas(total)} 行)。全体を残すなら右クリック→「選択箇所をファイルに保存」、削除だけなら Delete キー`,
      "error"
    );
    return;
  }
  await copyToClipboard(await selectedText(r));
  deleteSelection();
}

function appendHighlighted(container, text) {
  const re = state.matcher;
  re.lastIndex = 0;
  let last = 0;
  let m;
  while ((m = re.exec(text)) !== null) {
    if (m.index > last) {
      container.appendChild(document.createTextNode(text.slice(last, m.index)));
    }
    const mk = document.createElement("mark");
    mk.textContent = m[0];
    container.appendChild(mk);
    last = m.index + m[0].length;
    if (m[0].length === 0) re.lastIndex++; // never stall on empty matches
  }
  if (last < text.length) {
    container.appendChild(document.createTextNode(text.slice(last)));
  }
}

function render() {
  renderQueued = false;
  const vis = rowsVisible();
  const count = vis + OVERSCAN;
  ensurePool(count);
  ensureData(state.first, count);

  const gutterWidth = Math.max(4, String(state.total).length);
  for (let r = 0; r < pool.length; r++) {
    const row = pool[r];
    const line = state.first + r;
    if (r >= count || line > state.total) {
      row.style.display = "none";
      continue;
    }
    row.style.display = "";
    if (line === state.total) {
      fillEofRow(row); // one marker row just past the last line
    } else {
      fillRow(row, line, cachedLine(line), gutterWidth);
    }
  }
  buildRuler();
  renderSelection();
  positionCaret();
  updateScrollbar();
  updateStatusPos();
}

function scheduleRender() {
  if (renderQueued) return;
  renderQueued = true;
  requestAnimationFrame(render);
}

function setFirst(line) {
  state.first = Math.min(Math.max(0, Math.round(line)), maxFirst());
  scheduleRender();
}

// ---- custom scrollbar ------------------------------------------------------

function updateScrollbar() {
  const vh = $("viewport").clientHeight;
  const thumb = $("vthumb");
  const vis = rowsVisible();
  const ratio = state.total > 0 ? Math.min(1, vis / state.total) : 1;
  const thumbH = Math.max(24, vh * ratio);
  const mf = maxFirst();
  const top = mf > 0 ? (vh - thumbH) * (state.first / mf) : 0;
  thumb.style.height = `${thumbH}px`;
  thumb.style.transform = `translateY(${top}px)`;
  renderSearchTicks(vh);
}

function renderSearchTicks(vh) {
  const ticks = $("vticks");
  if (!ticks) return;
  ticks.textContent = "";
  if (!state.query || !state.searchHits || state.searchHits.length === 0 || state.total <= 1) return;
  const frag = document.createDocumentFragment();
  const maxTicks = 700;
  const step = Math.max(1, Math.ceil(state.searchHits.length / maxTicks));
  const denom = Math.max(1, state.total - 1);
  for (let i = 0; i < state.searchHits.length; i += step) {
    const h = state.searchHits[i];
    if (typeof h.line !== "number") continue;
    const t = document.createElement("div");
    t.className = "vtick";
    if (state.lastMatch && h.byte === state.lastMatch.byte) t.classList.add("current");
    const y = Math.max(0, Math.min(vh - 3, (h.line / denom) * (vh - 3)));
    t.style.transform = `translateY(${y}px)`;
    frag.append(t);
  }
  ticks.append(frag);
}

function initScrollbar() {
  const bar = $("vscrollbar");
  const thumb = $("vthumb");
  let dragging = false;
  let startY = 0;
  let startFirst = 0;

  thumb.addEventListener("mousedown", (e) => {
    dragging = true;
    startY = e.clientY;
    startFirst = state.first;
    thumb.classList.add("drag");
    e.preventDefault();
    e.stopPropagation();
  });
  window.addEventListener("mousemove", (e) => {
    if (!dragging) return;
    const vh = $("viewport").clientHeight;
    const thumbH = thumb.getBoundingClientRect().height;
    const usable = Math.max(1, vh - thumbH);
    const dr = (e.clientY - startY) / usable;
    setFirst(startFirst + dr * maxFirst());
  });
  window.addEventListener("mouseup", () => {
    dragging = false;
    thumb.classList.remove("drag");
  });
  // Click on the track pages toward the click.
  bar.addEventListener("mousedown", (e) => {
    if (e.target === thumb) return;
    const rect = bar.getBoundingClientRect();
    const above = e.clientY < thumb.getBoundingClientRect().top;
    setFirst(state.first + (above ? -1 : 1) * rowsVisible());
    void rect;
  });
}

// ---- status bar ------------------------------------------------------------

function updateStatusMeta() {
  const s = state.stat;
  if (!s) {
    setAppTitle("Ayame Editor");
    return;
  }
  if (!s.open) {
    for (const id of ["st-enc", "st-eol", "st-edit", "st-index"]) {
      $(id).textContent = "—";
    }
    $("st-edit").title = "";
    $("st-index").title = "";
    $("st-pos").textContent = "行 0";
    $("undo-edit").disabled = true;
    $("redo-edit").disabled = true;
    $("apply-theme").classList.add("hidden");
    $("apply-keymap").classList.add("hidden");
    setAppTitle("Ayame Editor");
    return;
  }
  const name = displayName(s.path);
  const dirtyMark = s.dirty ? "* " : "";
  setAppTitle(`${dirtyMark}${name} - Ayame Editor`);
  $("apply-theme").classList.toggle("hidden", !isThemeDoc(s.path));
  $("apply-keymap").classList.toggle("hidden", !isKeymapDoc(s.path));
  const lines = s.view_lines ?? s.lines;
  $("st-enc").textContent = s.bom_bytes > 0 ? `${enc(s.encoding)} (BOM)` : enc(s.encoding);
  $("st-eol").textContent = eol(s.eol);
  // Deliberately terse: the bar shows state, the tooltip carries the numbers.
  $("st-edit").textContent = s.dirty ? "未保存" : "保存済";
  $("st-edit").title = s.dirty
    ? `未保存の編集: +${commas(s.inserted_lines)} 行追加 / ~${commas(s.replaced_lines)} 行変更 / -${commas(s.deleted_lines)} 行削除`
    : "すべての編集は保存済みです";
  $("undo-edit").disabled = !s.can_undo;
  $("redo-edit").disabled = !s.can_redo;
  $("st-index").textContent = "索引OK";
  $("st-index").title =
    `${commas(lines)} 行 / ${humanBytes(s.bytes)} / 索引 ${commas(s.checkpoints)} 点 (${humanBytes(s.index_bytes)}, ${s.index_ms} ms)`;
  // Keep the active tab's unsaved-dot (and the tabs model behind
  // beforeunload / close confirmations) in sync as you type.
  const at = $("tabs").querySelector(".tab.active");
  if (at) at.classList.toggle("dirty", !!s.dirty);
  const activeTab = (state.tabs || []).find((t) => t.active);
  if (activeTab) activeTab.dirty = !!s.dirty;
}

function isUntitled(path) {
  return !!path && path.includes("ayame-untitled-");
}

function untitledName(path) {
  const base = pathBaseName(path);
  return base && base !== "untitled.txt" ? base : "untitled";
}

// Show a short, friendly name in the toolbar (basename, or "untitled").
function displayName(path) {
  if (!path) return "—";
  if (isUntitled(path)) return untitledName(path);
  const parts = path.replace(/\\/g, "/").split("/");
  return parts[parts.length - 1] || path;
}

function pathBaseName(path) {
  if (!path) return "";
  const clean = String(path).replace(/^\\\\\?\\/, "");
  const parts = clean.replace(/\\/g, "/").split("/");
  return parts[parts.length - 1] || clean;
}

function pathDirName(path) {
  if (!path) return null;
  const clean = String(path).replace(/^\\\\\?\\/, "");
  const i = Math.max(clean.lastIndexOf("/"), clean.lastIndexOf("\\"));
  if (i < 0) return null;
  if (i === 0) return clean.slice(0, 1);
  return clean.slice(0, i);
}

function isAbsolutePath(path) {
  return /^(?:[A-Za-z]:[\\/]|\/|\\\\)/.test(String(path || ""));
}

function joinPath(dir, name) {
  const n = String(name || "").trim();
  if (!n) return "";
  if (isAbsolutePath(n)) return n;
  const d = String(dir || "").replace(/[\\/]+$/, "");
  if (!d) return n;
  const sep = d.includes("\\") && !d.includes("/") ? "\\" : "/";
  return `${d}${sep}${n}`;
}

function pathCrumbs(path) {
  const clean = String(path || "").replace(/^\\\\\?\\/, "");
  if (!clean) return [];
  const winDrive = clean.match(/^([A-Za-z]:)[\\/](.*)$/);
  if (winDrive) {
    const sep = "\\";
    let acc = `${winDrive[1]}${sep}`;
    const out = [{ label: winDrive[1], path: acc }];
    for (const part of winDrive[2].split(/[\\/]+/).filter(Boolean)) {
      acc = acc.endsWith(sep) ? `${acc}${part}` : `${acc}${sep}${part}`;
      out.push({ label: part, path: acc });
    }
    return out;
  }
  if (clean.startsWith("\\\\")) {
    const parts = clean.split(/[\\/]+/).filter(Boolean);
    if (parts.length < 2) return [{ label: clean, path: clean }];
    let acc = `\\\\${parts[0]}\\${parts[1]}`;
    const out = [{ label: `\\\\${parts[0]}\\${parts[1]}`, path: acc }];
    for (const part of parts.slice(2)) {
      acc = `${acc}\\${part}`;
      out.push({ label: part, path: acc });
    }
    return out;
  }
  if (clean.startsWith("/")) {
    let acc = "";
    const out = [{ label: "/", path: "/" }];
    for (const part of clean.split("/").filter(Boolean)) {
      acc += `/${part}`;
      out.push({ label: part, path: acc });
    }
    return out;
  }
  let acc = "";
  return clean.split(/[\\/]+/).filter(Boolean).map((part) => {
    acc = acc ? `${acc}/${part}` : part;
    return { label: part, path: acc };
  });
}

function enc(e) {
  return (
    { "utf-8": "UTF-8", "shift-jis": "Shift_JIS", "euc-jp": "EUC-JP", ascii: "ASCII" }[e] ||
    String(e)
  );
}
function eol(e) {
  return { lf: "LF", crlf: "CRLF", cr: "CR", mixed: "Mixed", none: "None" }[e] || String(e);
}

function updateStatusPos() {
  if (state.total === 0) {
    $("st-pos").textContent = "行 0";
    return;
  }
  const pos = `行 ${commas(state.caret.line + 1)}, 列 ${commas(state.caret.col + 1)}`;
  const n = state.extraCursors.length;
  $("st-pos").textContent = n ? `${pos} · ${n + 1} カーソル` : pos;
}

// ---- search ----------------------------------------------------------------

function showFind() {
  state.findOpen = true;
  document.documentElement.classList.add("find-open");
  const f = $("find");
  queueMicrotask(() => {
    f.focus();
    f.select();
  });
}

function hideFind() {
  state.findOpen = false;
  document.documentElement.classList.remove("find-open");
}

function buildMatcher() {
  state.regexError = false;
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
  } catch {
    state.regexError = true;
    state.matcher = null; // invalid regex while typing — just don't highlight
    $("find").parentElement.classList.add("error");
  }
}

function qs() {
  return `q=${encodeURIComponent(state.query)}&regex=${state.regex}&ci=${state.ci}&word=${state.word}`;
}

async function findStep(dir) {
  if (!state.query) return;
  buildMatcher();
  if (state.regexError) {
    flashCount("正規表現エラー", "error");
    return;
  }
  saveSearchHistory(state.query);
  let from;
  if (dir === "next") {
    from = state.lastMatch ? state.lastMatch.byte + Math.max(1, state.lastMatch.len) : await lineByte(state.first);
  } else {
    from = state.lastMatch ? state.lastMatch.byte : await lineByte(Math.min(state.total, state.first + rowsVisible()));
  }
  try {
    const res = await api(`/api/find?dir=${dir}&from=${from}&${qs()}`);
    if (!res.hit) {
      flashCount("一致なし");
      return;
    }
    const h = res.hit;
    state.lastMatch = { byte: h.byte, len: h.byte_len };
    state.sel = null;
    setCaret(h.line, 0);
    revealLine(h.line);
    updateCount();
  } catch (e) {
    flashCount("エラー");
    console.error(e);
  }
}

async function updateCount() {
  if (!state.query) {
    $("find-count").textContent = "";
    state.searchHits = null;
    state.searchTruncated = false;
    return;
  }
  try {
    const res = await api(`/api/search?${qs()}&start=0&max=2000`);
    state.searchHits = res.hits;
    state.searchTruncated = res.truncated;
    updateFindCountLabel();
    scheduleRender();
  } catch (e) {
    $("find-count").textContent = "正規表現エラー";
    $("find").parentElement.classList.add("error");
    scheduleRender();
  }
}

function updateFindCountLabel() {
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
  $("find-count").textContent = `${total} 件`;
}

// Operation feedback goes to the always-visible status bar (aria-live), and is
// mirrored into the find bar when that is open. Errors stay a little longer.
let stMsgTimer = 0;
function flashCount(msg, kind = "") {
  const isError = kind === "error";
  const el = $("st-msg");
  if (el) {
    el.textContent = msg || "";
    el.classList.toggle("error", isError);
    clearTimeout(stMsgTimer);
    if (msg) {
      stMsgTimer = setTimeout(() => {
        el.textContent = "";
        el.classList.remove("error");
      }, isError ? 10000 : 6000);
    }
  }
  if (state.findOpen) $("find-count").textContent = msg;
}

function loadSearchHistory() {
  try {
    const raw = JSON.parse(localStorage.getItem(SEARCH_HISTORY_KEY) || "[]");
    return Array.isArray(raw) ? raw.filter((x) => typeof x === "string").slice(0, 50) : [];
  } catch {
    return [];
  }
}

function saveSearchHistory(q) {
  const value = q.trim();
  if (!value) return;
  state.history = [value, ...state.history.filter((x) => x !== value)].slice(0, 50);
  state.historyIndex = -1;
  try {
    localStorage.setItem(SEARCH_HISTORY_KEY, JSON.stringify(state.history));
  } catch {
    // Ignore private-mode quota errors; search still works.
  }
}

function showSearchHistory(delta) {
  if (!state.history.length) return false;
  if (state.historyIndex < 0) {
    state.historyIndex = delta < 0 ? 0 : state.history.length - 1;
  } else {
    state.historyIndex = Math.min(
      state.history.length - 1,
      Math.max(0, state.historyIndex + delta)
    );
  }
  $("find").value = state.history[state.historyIndex];
  setQueryFromInput();
  return true;
}

function revealLine(line) {
  const vis = rowsVisible();
  if (line < state.first || line >= state.first + vis) {
    setFirst(line - Math.floor(vis / 3));
  } else {
    scheduleRender();
  }
  updateStatusPos();
}

async function refreshStat() {
  state.stat = await api("/api/stat");
  state.total = state.stat.view_lines ?? state.stat.lines;
  updateStatusMeta();
}

function clearLineCache() {
  state.cache = { start: 0, lines: [] };
  state.loadToken++;
}

// ===========================================================================
//  Caret-based editing (Notepad / Sakura Editor style)
//
//  There is a single fluid caret (state.caret) you can place anywhere and type
//  across lines. Every mutation is expressed as one range replacement and sent
//  to the backend's /api/edit/replace_range, which records it as a single undo
//  step and returns the resulting caret. Edits are serialized through a small
//  queue so fast typing/IME stays in order; the visible window is re-fetched
//  after each edit (cheap over loopback) rather than mirrored optimistically.
// ===========================================================================

// Move the caret without touching the selection model wholesale (callers set
// state.sel themselves). Keeps the active-line highlight on the caret line.
function setCaret(line, col) {
  line = Math.max(0, Math.min(line, Math.max(0, state.total - 1)));
  col = Math.max(0, Math.min(col, lineLen(line)));
  state.caret = { line, col };
  state.activeLine = line;
  state.extraCursors = []; // any explicit caret placement collapses multi-cursor
  state.editGen++; // user-driven caret placement (click, search, open, …)
}

// Caret motion for the keyboard: `extend` grows the selection from its anchor.
function moveCaret(line, col, extend) {
  line = Math.max(0, Math.min(line, Math.max(0, state.total - 1)));
  col = Math.max(0, Math.min(col, lineLen(line)));
  if (extend) {
    const anchor = state.sel ? state.sel.anchor : { ...state.caret };
    state.sel = { anchor, head: { line, col } };
    state.extraCursors = []; // entering a selection collapses multi-cursor
  } else {
    state.sel = null;
  }
  state.caret = { line, col };
  state.activeLine = line;
  state.editGen++; // user-driven caret motion (arrows, Home/End, PageUp/Down)
  revealCaret();
  scheduleRender();
}

// ---- multi-cursor (MVP: extra insertion carets, no per-cursor selections) ---

// Primary caret plus the extra cursors, deduped and in document order. The
// entry carrying `primary: true` mirrors state.caret.
function allCursors() {
  const out = [];
  const seen = new Set();
  const push = (c, primary) => {
    const k = `${c.line}:${c.col}`;
    if (seen.has(k)) return;
    seen.add(k);
    out.push({ line: c.line, col: c.col, primary });
  };
  push(state.caret, true);
  for (const c of state.extraCursors) push(c, false);
  out.sort((a, b) => a.line - b.line || a.col - b.col);
  return out;
}

function clearExtraCursors() {
  if (state.extraCursors.length) {
    state.extraCursors = [];
    scheduleRender();
  }
}

// Ctrl+Click: add a caret; clicking an existing extra caret removes it; the
// primary caret is left alone.
function toggleExtraCursorAt(line, col) {
  if (line === state.caret.line && col === state.caret.col) return;
  const i = state.extraCursors.findIndex((c) => c.line === line && c.col === col);
  if (i >= 0) state.extraCursors.splice(i, 1);
  else {
    state.sel = null; // extra cursors and range selections are exclusive
    state.extraCursors.push({ line, col });
  }
  state.editGen++; // user cursor action: an in-flight edit must not clobber it
  scheduleRender();
}

function addExtraCursorAt(line, col) {
  if (line === state.caret.line && col === state.caret.col) return;
  if (state.extraCursors.some((c) => c.line === line && c.col === col)) return;
  state.sel = null;
  state.extraCursors.push({ line, col });
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
async function addCursorAbove() {
  if (!state.stat?.open || state.total === 0) return;
  const top = allCursors()[0];
  if (top.line <= 0) return;
  const line = top.line - 1;
  const lens = await lineLensFor([line]);
  addExtraCursorAt(line, Math.min(top.col, lens.get(line) ?? 0));
}

async function addCursorBelow() {
  if (!state.stat?.open || state.total === 0) return;
  const cs = allCursors();
  const bottom = cs[cs.length - 1];
  if (bottom.line >= state.total - 1) return;
  const line = bottom.line + 1;
  const lens = await lineLensFor([line]);
  addExtraCursorAt(line, Math.min(bottom.col, lens.get(line) ?? 0));
}

function focusEditor() {
  const hi = $("hidden-input");
  if (hi && document.activeElement !== hi) hi.focus({ preventScroll: true });
  state.focused = true;
  scheduleRender();
}

// Bring the caret into view: scroll vertically (whole lines) and horizontally
// (#content is the horizontal scroll container).
function revealCaret() {
  const vis = rowsVisible();
  if (state.caret.line < state.first) {
    state.first = Math.min(state.caret.line, maxFirst());
  } else if (state.caret.line >= state.first + vis) {
    state.first = Math.min(Math.max(0, state.caret.line - vis + 1), maxFirst());
  }
  const content = $("content");
  const x = caretX(state.caret.line, state.caret.col);
  const view = content.clientWidth;
  const margin = 24;
  if (x - margin < content.scrollLeft) {
    content.scrollLeft = Math.max(0, x - margin);
  } else if (x + margin > content.scrollLeft + view) {
    content.scrollLeft = x + margin - view;
  }
}

// Position the caret element and the hidden IME input at the caret pixel.
function positionCaret() {
  const caretEl = $("caret");
  const hi = $("hidden-input");
  if (!caretEl || !hi) return;
  const vis = rowsVisible();
  const focusVisible = state.focused && !anyModalOpen() && !state.composing;
  positionExtraCarets(vis, focusVisible);
  const onScreen =
    !!state.stat?.open &&
    state.caret.line >= state.first &&
    state.caret.line < state.first + vis;
  const show = onScreen && state.focused && !anyModalOpen();
  caretEl.classList.toggle("on", show && !state.composing);
  if (!onScreen) return;
  const x = caretX(state.caret.line, state.caret.col);
  const y = (state.caret.line - state.first) * LINE_HEIGHT;
  caretEl.style.transform = `translate(${x}px, ${y}px)`;
  hi.style.transform = `translate(${x}px, ${y}px)`;
}

// Mirror #caret for every extra cursor: same transform math and the same
// visibility rules (focus, modal open, IME composition, offscreen). The divs
// live in a small pool inside #content and are trimmed when cursors go away.
const extraCaretPool = [];
function positionExtraCarets(vis, focusVisible) {
  const cursors = state.extraCursors;
  while (extraCaretPool.length < cursors.length) {
    const el = document.createElement("div");
    el.className = "caret extra";
    el.setAttribute("aria-hidden", "true");
    $("content").append(el);
    extraCaretPool.push(el);
  }
  while (extraCaretPool.length > cursors.length) extraCaretPool.pop().remove();
  for (let i = 0; i < cursors.length; i++) {
    const c = cursors[i];
    const el = extraCaretPool[i];
    const onScreen =
      !!state.stat?.open && c.line >= state.first && c.line < state.first + vis;
    el.classList.toggle("on", onScreen && focusVisible);
    if (onScreen) {
      const x = caretX(c.line, c.col);
      const y = (c.line - state.first) * LINE_HEIGHT;
      el.style.transform = `translate(${x}px, ${y}px)`;
    }
  }
}

function anyModalOpen() {
  return promptVisible() || formVisible() || settingsVisible() || keymapVisible() || diffVisible() || openerVisible();
}

// ---- the serialized edit queue --------------------------------------------

let editChain = Promise.resolve();
function editContext() {
  return { docGen: state.docGen };
}
function sameEditContext(ctx) {
  return !!state.stat?.open && state.docGen === ctx.docGen;
}
async function settleEditQueue() {
  await editChain;
}
function enqueueEdit(fn) {
  const ctx = editContext();
  editChain = editChain.then(async () => {
    if (!sameEditContext(ctx)) return null;
    if (savingCount > 0) {
      flashCount("保存中です — 完了後に入力します");
      await waitForSavingDone();
      if (!sameEditContext(ctx)) return null;
    }
    return fn();
  }).catch((e) => {
    flashCount("編集エラー");
    console.error(e);
  });
  return editChain;
}

// Re-fetch the padded window around state.first into the cache in one shot, so
// the text never blinks to the "⋯" pending placeholder between keystrokes.
async function reloadViewport() {
  const start = Math.max(0, state.first - PAD);
  const count = rowsVisible() + OVERSCAN + 2 * PAD;
  const res = await api(`/api/lines?start=${start}&count=${count}`);
  state.cache = { start, lines: res.lines };
  state.total = res.total;
  state.loadToken++; // cancel any in-flight ensureData for the old contents
}

// The range the next text insertion replaces: the selection, or the caret.
function replaceTarget() {
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
async function applyRange(l0, c0, l1, c1, text) {
  const ctx = editContext();
  const gen = state.editGen;
  const res = await apiPost("/api/edit/replace_range", { l0, c0, l1, c1, text });
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
    flashCount("再読込エラー");
  }
  if (!sameEditContext(ctx)) return;
  revealCaret();
  render();
}

async function applyRect(l0, l1, c0, c1, text) {
  const ctx = editContext();
  const gen = state.editGen;
  const res = await apiPost("/api/edit/replace_rect", { l0, l1, c0, c1, text });
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
    flashCount("再読込エラー");
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
async function applyBatch(edits, cursors, editOf) {
  const ctx = editContext();
  const gen = state.editGen;
  const res = await apiPost("/api/edit/replace_batch", { edits });
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
    flashCount("再読込エラー");
  }
  if (!sameEditContext(ctx)) return;
  revealCaret();
  render();
}

// One same-shaped insertion per cursor. `textFor(i)` is the string inserted at
// cursor i (document order) — a constant for typing, per-line for paste.
function multiInsert(cursors, textFor) {
  const edits = cursors.map((c, i) => ({
    l0: c.line, c0: c.col, l1: c.line, c1: c.col, text: textFor(i),
  }));
  return applyBatch(edits, cursors, cursors.map((_, i) => i));
}

// Insert (or replace the selection with) `text`, which may contain newlines.
// The target range is resolved *inside* the queued step, so a burst of
// keystrokes each sees the caret left by the previous edit (never a stale one).
// A 0-line document (an empty file) is editable too: replaceTarget yields the
// (0,0)..(0,0) origin range, which the backend accepts to seed the first line.
function typeText(text) {
  if (!state.stat?.open) return;
  enqueueEdit(() => {
    if (state.extraCursors.length) {
      // Multi-cursor: the same text goes in at every caret, as one undo step.
      return multiInsert(allCursors(), () => text);
    }
    const rr = rectRange();
    if (rr) {
      return applyRect(rr.l0, rr.l1, rr.c0, rr.c1, text);
    }
    const t = replaceTarget();
    return applyRange(t.l0, t.c0, t.l1, t.c1, text);
  });
}

function insertNewline() {
  typeText("\n");
}

// Decoded length (in Unicode scalars) of each requested line, as a Map. Lines
// inside the local cache are read from it; anything else is fetched, because
// lineLen() silently reads 0 for uncached lines — and multi-cursor edits can
// reference lines far outside the viewport±PAD cache window, where a guessed 0
// would turn a delete edge into "delete the whole line". Lines whose length
// cannot be resolved are absent from the map; callers must skip those edits.
async function lineLensFor(lineNumbers) {
  const out = new Map();
  const missing = new Set();
  for (const l of lineNumbers) {
    if (l < 0 || l >= state.total || out.has(l) || missing.has(l)) continue;
    const rec = cachedLine(l);
    if (rec != null) out.set(l, Array.from(rec.text ?? "").length);
    else missing.add(l);
  }
  await Promise.all([...missing].map(async (l) => {
    try {
      const res = await api(`/api/lines?start=${l}&count=1`);
      const text = res.lines?.[0]?.text;
      if (text != null) out.set(l, Array.from(text).length);
    } catch {
      // Leave the line out: the caller drops that cursor's edit, never guesses.
    }
  }));
  return out;
}

// The shared "a selection is active" arm of every delete command: remove the
// rect or range selection as one edit. Returns null when nothing is selected
// (callers then handle their caret-relative case). Call inside enqueueEdit.
function deleteSelectionEdit() {
  if (!hasSelection()) return null;
  const rr = rectRange();
  if (rr) return applyRect(rr.l0, rr.l1, rr.c0, rr.c1, "");
  const t = replaceTarget();
  return applyRange(t.l0, t.c0, t.l1, t.c1, "");
}

function backspace() {
  enqueueEdit(async () => {
    if (state.extraCursors.length) {
      // Per cursor: delete one char before the caret (line-join at col 0).
      // allCursors() dedupes positions, so ranges may touch but never overlap;
      // a cursor at the document origin contributes no edit. Join edges need
      // the previous line's REAL length, which may live outside the cache.
      const cursors = allCursors();
      const lens = await lineLensFor(
        cursors.filter((c) => c.col === 0 && c.line > 0).map((c) => c.line - 1)
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
    const del = deleteSelectionEdit();
    if (del) return del;
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

function forwardDelete() {
  enqueueEdit(async () => {
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
    const del = deleteSelectionEdit();
    if (del) return del;
    const { line, col } = state.caret;
    const lens = await lineLensFor([line]);
    if (!lens.has(line)) return null;
    if (col < lens.get(line)) return applyRange(line, col, line, col + 1, "");
    if (line < state.total - 1) return applyRange(line, col, line + 1, 0, "");
    return null;
  });
}

function pasteText(raw) {
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
    return multiInsert(cursors, (i) => (perCursor ? perCursor[i] : text));
  });
}

async function saveCopy() {
  if (savingCount > 0) {
    flashCount("保存中です — 完了までお待ちください");
    return;
  }
  await settleEditQueue();
  const target = await showSaveDialog("別名で保存", suggestedSaveAsPath());
  if (!target) return;
  savingCount++;
  setSavingUI();
  try {
    const res = await apiPost("/api/edit/save", target);
    const stat = await apiPost("/api/open", { path: res.path });
    onDocumentOpened(stat);
    flashCount(`保存しました: ${res.path}`);
  } catch (e) {
    flashCount("保存エラー", "error");
    alert(e.message);
  } finally {
    savingCount--;
    setSavingUI();
    retryPendingNativeClose();
  }
}

function suggestedSaveAsPath() {
  const p = state.stat?.path || "";
  if (!p) return "untitled.txt";
  if (isUntitled(p)) return pathBaseName(p) || "untitled.txt";
  return `${p}.edited`;
}

async function saveFile() {
  if (!state.stat?.open) return;
  if (savingCount > 0) {
    flashCount("保存中です — 完了までお待ちください");
    return;
  }
  await settleEditQueue();
  if (isUntitled(state.stat.path)) {
    await saveCopy();
    return;
  }
  savingCount++;
  setSavingUI();
  try {
    const res = await apiPost("/api/edit/save", { overwrite: true });
    clearLineCache();
    await refreshStat();
    await reloadViewport();
    render();
    flashCount(`保存しました: ${res.path}`);
  } catch (e) {
    flashCount("保存エラー", "error");
    alert(e.message);
  } finally {
    savingCount--;
    setSavingUI();
    retryPendingNativeClose();
  }
}

// ファイルメニュー「保存時の状態に戻す」: discard every unsaved edit and go
// back to the document as it exists on disk (/api/edit/revert reloads it).
async function revertEdits() {
  if (!state.stat?.dirty) {
    flashCount("未保存の編集はありません");
    return;
  }
  if (!confirm("未保存の編集をすべて破棄して、保存時の状態に戻しますか?")) return;
  try {
    await apiPost("/api/edit/revert", {});
    clearLineCache();
    state.sel = null;
    state.extraCursors = [];
    await refreshStat();
    await reloadViewport();
    // Re-clamp the caret against the reverted document's real line count.
    setCaret(Math.min(state.caret.line, Math.max(0, state.total - 1)), 0);
    render();
    flashCount("保存時の状態に戻しました");
  } catch (e) {
    flashCount("元に戻せません", "error");
    alert(e.message);
  }
}

async function undoEdit() {
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

async function redoEdit() {
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

// ソート: sorts the current tab in place — unsaved edits included — and
// overwrites the original file on disk. All options sit in one form.
async function sortSave() {
  if (!state.stat?.open) return;
  const f = await askForm("ソート", [
    { id: "key", type: "text", label: "キー列 (1始まり)", placeholder: "空なら行全体で比較",
      title: "空欄: 行全体を文字列として比較 / 数字: 区切り文字で分けたその列をキーとして比較" },
    { id: "delim", type: "text", label: "区切り文字", value: ",", placeholder: ",",
      title: "キー列を使うときの列の区切り (例: , やタブ)" },
    { id: "numeric", type: "check", label: "数値として比較する", value: false,
      title: "10 と 9 を文字列でなく数値の大小で並べます" },
    { id: "order", type: "select", label: "並び順",
      options: [["asc", "昇順 (A→Z, 小→大)"], ["desc", "降順 (Z→A, 大→小)"]] },
    { id: "_hint", type: "hint", label: "現在のファイルを並び替えて上書きします。未保存の編集も含めて並び替えます。この操作は元に戻せません。" },
  ], "ソート");
  if (!f) return;
  const keyText = String(f.key || "").trim();
  const key = keyText === "" ? null : Number(keyText);
  if (keyText !== "" && (!Number.isInteger(key) || key < 1)) {
    flashCount("キー列は 1 以上の整数で指定してください", "error");
    return;
  }
  showLoading("ソート実行中…");
  try {
    await apiPost("/api/sort/save", {
      in_place: true,
      key,
      numeric: !!f.numeric,
      reverse: f.order === "desc",
      delim: key != null && f.delim ? f.delim : null,
    });
    state.sel = null;
    state.extraCursors = [];
    setCaret(0, 0);
    clearLineCache();
    await refreshStat();
    await reloadViewport();
    render();
    flashCount("ソートして上書きしました");
  } catch (e) {
    flashCount("ソートエラー", "error");
    alert(e.message);
  } finally {
    hideLoading();
  }
}

// ファイル分割: writes the current document (unsaved edits included) out as
// multiple files of at most N lines each; the original file is untouched.
async function splitFile() {
  if (!state.stat?.open) return;
  const f = await askForm("ファイルを分割", [
    { id: "lines", type: "text", label: "1ファイルあたりの行数", value: "1000000" },
    { id: "dir", type: "text", label: "出力先フォルダ", value: "",
      placeholder: "空なら元ファイルと同じ場所" },
    { id: "_hint", type: "hint", label: "現在のファイルを指定行数ごとに分割して書き出します。未保存の編集も含まれます。元のファイルは変更されません。" },
  ], "分割");
  if (!f) return;
  const lines = Number(String(f.lines || "").trim());
  if (!Number.isInteger(lines) || lines < 1) {
    flashCount("行数は 1 以上の整数で指定してください", "error");
    return;
  }
  showLoading("分割実行中…");
  try {
    const dir = String(f.dir || "").trim();
    const res = await apiPost("/api/split/save", { lines, dir: dir || null });
    flashCount(`${res.count} 個に分割しました: 最初のファイル ${res.files[0]}`);
  } catch (e) {
    flashCount("分割エラー", "error");
    alert(e.message);
  } finally {
    hideLoading();
  }
}

async function replaceSave() {
  if (!state.stat?.open) return;
  const base = state.stat?.path || "ayame";
  const f = await askForm("置換して保存", [
    { id: "find", type: "text", label: "置換前", value: $("find").value || state.query || "" },
    { id: "replacement", type: "text", label: "置換後", value: "" },
    { id: "regex", type: "check", label: "正規表現として解釈する", value: state.regex },
    { id: "ci", type: "check", label: "大文字小文字を区別しない", value: state.ci },
    { id: "path", type: "text", label: "保存先パス", value: `${base}.replaced` },
  ]);
  if (!f) return;
  if (!f.find) {
    flashCount("置換前の文字列を入力してください", "error");
    return;
  }
  showLoading("置換実行中…");
  try {
    const res = await apiPost("/api/replace/save", {
      path: f.path,
      find: f.find,
      replacement: f.replacement,
      regex: !!f.regex,
      ci: !!f.ci,
    });
    flashCount(`置換して保存しました: ${res.path}`);
  } catch (e) {
    flashCount("置換エラー", "error");
    alert(e.message);
  } finally {
    hideLoading();
  }
}

function diffVisible() {
  return !$("diff-modal").classList.contains("hidden");
}

function hideDiff() {
  setModalOpen($("diff-modal"), false);
  focusEditor();
}

function showDiff(res) {
  $("diff-summary").textContent =
    `${commas(res.hunk_count)} hunk / +${commas(res.added)}  -${commas(res.deleted)}  ~${commas(res.modified)}`
    + (res.current_dirty ? " / 未保存編集込み" : "")
    + (res.omitted_hunks ? ` / ${commas(res.omitted_hunks)} hunk omitted` : "");
  $("diff-old-path").textContent = (res.old_path || "現在のファイル") + (res.current_dirty ? " *" : "");
  $("diff-new-path").textContent = res.new_path || "比較先";
  renderDiffView(res);
  setModalOpen($("diff-modal"), true);
}

function diffKindLabel(kind) {
  if (kind === "insert") return "追加";
  if (kind === "delete") return "削除";
  return "変更";
}

const INLINE_DIFF_MAX_CHARS = 2000;
const INLINE_DIFF_MAX_TOKENS = 260;

function inlineTokens(text) {
  const tokens = [];
  const re = /(\s+|[\p{Letter}\p{Number}_]+|[^\s\p{Letter}\p{Number}_]+)/gu;
  for (const m of String(text || "").matchAll(re)) tokens.push(m[0]);
  return tokens;
}

function pushDiffPart(parts, text, changed) {
  if (!text) return;
  const last = parts[parts.length - 1];
  if (last && last.changed === changed) last.text += text;
  else parts.push({ text, changed });
}

function inlineWordDiff(oldText, newText) {
  oldText = String(oldText || "");
  newText = String(newText || "");
  if (oldText === newText) return null;
  if (oldText.length + newText.length > INLINE_DIFF_MAX_CHARS) return null;
  const oldTokens = inlineTokens(oldText);
  const newTokens = inlineTokens(newText);
  if (oldTokens.length + newTokens.length > INLINE_DIFF_MAX_TOKENS) return null;

  const m = oldTokens.length;
  const n = newTokens.length;
  const dp = Array.from({ length: m + 1 }, () => new Uint16Array(n + 1));
  for (let i = m - 1; i >= 0; i--) {
    for (let j = n - 1; j >= 0; j--) {
      dp[i][j] = oldTokens[i] === newTokens[j]
        ? dp[i + 1][j + 1] + 1
        : Math.max(dp[i + 1][j], dp[i][j + 1]);
    }
  }

  const oldParts = [];
  const newParts = [];
  let i = 0;
  let j = 0;
  while (i < m || j < n) {
    if (i < m && j < n && oldTokens[i] === newTokens[j]) {
      pushDiffPart(oldParts, oldTokens[i], false);
      pushDiffPart(newParts, newTokens[j], false);
      i++;
      j++;
    } else if (j >= n || (i < m && dp[i + 1][j] >= dp[i][j + 1])) {
      pushDiffPart(oldParts, oldTokens[i], true);
      i++;
    } else {
      pushDiffPart(newParts, newTokens[j], true);
      j++;
    }
  }
  return { oldParts, newParts };
}

function appendDiffText(el, line, parts) {
  if (!line) return;
  if (!parts) {
    el.textContent = line.text;
    return;
  }
  for (const part of parts) {
    const span = document.createElement("span");
    span.className = part.changed ? "diff-word changed" : "diff-word";
    span.textContent = part.text;
    el.append(span);
  }
}

function diffCell(line, cls, parts = null) {
  const cell = document.createElement("div");
  cell.className = "diff-cell " + (cls || "");
  const ln = document.createElement("span");
  ln.className = "diff-ln";
  ln.textContent = line ? String(line.number + 1) : "";
  const tx = document.createElement("span");
  tx.className = "diff-tx";
  appendDiffText(tx, line, parts);
  cell.append(ln, tx);
  return cell;
}

function renderDiffView(res) {
  const view = $("diff-view");
  view.textContent = "";
  const frag = document.createDocumentFragment();
  for (const h of res.hunks || []) {
    const hunk = document.createElement("section");
    hunk.className = "diff-hunk";
    const title = document.createElement("div");
    title.className = "diff-hunk-title";
    title.textContent =
      `${diffKindLabel(h.kind)}  現在:${commas(h.old_start + 1)} (${commas(h.old_len)}行)  `
      + `比較先:${commas(h.new_start + 1)} (${commas(h.new_len)}行)`;
    hunk.append(title);
    const oldRows = h.old_preview || [];
    const newRows = h.new_preview || [];
    const max = Math.max(oldRows.length, newRows.length, 1);
    for (let i = 0; i < max; i++) {
      const row = document.createElement("div");
      row.className = "diff-row";
      const oldLine = oldRows[i] || null;
      const newLine = newRows[i] || null;
      const oldCls = h.kind === "insert" ? "blank" : h.kind === "delete" ? "del" : oldLine ? "mod" : "blank";
      const newCls = h.kind === "delete" ? "blank" : h.kind === "insert" ? "add" : newLine ? "mod" : "blank";
      const wordDiff = h.kind === "replace" && oldLine && newLine
        ? inlineWordDiff(oldLine.text, newLine.text)
        : null;
      row.append(
        diffCell(oldLine, oldCls, wordDiff?.oldParts),
        diffCell(newLine, newCls, wordDiff?.newParts)
      );
      hunk.append(row);
    }
    if (h.old_truncated || h.new_truncated) {
      const tr = document.createElement("div");
      tr.className = "diff-truncated";
      tr.textContent = `このhunkは先頭 ${commas(res.max_lines_per_hunk || 80)} 行だけ表示しています`;
      hunk.append(tr);
    }
    frag.append(hunk);
  }
  if (!res.hunks || res.hunks.length === 0) {
    const empty = document.createElement("div");
    empty.className = "diff-truncated";
    empty.textContent = "差分はありません";
    frag.append(empty);
  }
  view.append(frag);
}

async function diffFile() {
  const base = state.stat?.path || "";
  const path = await askPrompt("2ファイル差分", "比較先ファイルパス", base);
  if (path == null || path.trim() === "") return;
  showLoading("差分を計算中…");
  try {
    const res = await api(`/api/diff?path=${encodeURIComponent(path.trim())}&max_hunks=200&max_lines=80&window=128`);
    flashCount(`差分: ${commas(res.hunk_count)} hunk`);
    showDiff(res);
  } catch (e) {
    flashCount("差分エラー", "error");
    alert(e.message);
  } finally {
    hideLoading();
  }
}

async function caseSave(mode) {
  if (!state.stat?.open) return;
  const base = state.stat?.path || "ayame";
  const suffix = mode === "upper" ? "upper" : "lower";
  const f = await askForm(`${mode === "upper" ? "大文字化" : "小文字化"}して保存`, [
    { id: "path", type: "text", label: "保存先パス", value: `${base}.${suffix}` },
    { id: "_hint", type: "hint", label: "ASCII 英字を変換した内容を別ファイルへ書き出します。元のファイルは変更されません。" },
  ]);
  if (!f || !f.path) return;
  showLoading("変換実行中…");
  try {
    const res = await apiPost("/api/case/save", { path: f.path, mode });
    flashCount(`保存しました: ${res.path}`);
  } catch (e) {
    flashCount("変換エラー", "error");
    alert(e.message);
  } finally {
    hideLoading();
  }
}

// ---- input wiring ----------------------------------------------------------

function setQueryFromInput() {
  state.query = $("find").value;
  state.lastMatch = null;
  state.searchHits = null;
  state.searchTruncated = false;
  buildMatcher();
  $("find-count").textContent = state.regexError ? "正規表現エラー" : "";
  scheduleRender();
}

function initEvents() {
  const vp = $("viewport");

  vp.addEventListener(
    "wheel",
    (e) => {
      e.preventDefault();
      let dy = e.deltaY;
      if (e.deltaMode === 1) dy *= LINE_HEIGHT;
      else if (e.deltaMode === 2) dy *= vp.clientHeight;
      state.fracAcc += dy / LINE_HEIGHT;
      const whole = Math.trunc(state.fracAcc);
      state.fracAcc -= whole;
      if (whole !== 0) setFirst(state.first + whole);
    },
    { passive: false }
  );

  const find = $("find");
  find.addEventListener("input", setQueryFromInput);
  find.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      updateCount();
      findStep(e.shiftKey ? "prev" : "next");
    } else if (e.key === "ArrowUp" || e.key === "ArrowDown") {
      if (showSearchHistory(e.key === "ArrowUp" ? -1 : 1)) {
        e.preventDefault();
      }
    } else if (e.key === "Escape") {
      hideFind();
      focusEditor();
    }
  });

  $("find-close").addEventListener("click", () => {
    hideFind();
    focusEditor();
  });
  $("find-next").addEventListener("click", () => findStep("next"));
  $("find-prev").addEventListener("click", () => findStep("prev"));
  $("opt-case").addEventListener("click", () => toggleOpt("ci", "opt-case"));
  $("opt-word").addEventListener("click", () => toggleOpt("word", "opt-word"));
  $("opt-regex").addEventListener("click", () => toggleOpt("regex", "opt-regex"));
  $("save-file").addEventListener("click", () => {
    hideFileMenu();
    saveFile();
  });
  $("save-copy").addEventListener("click", () => {
    hideFileMenu();
    saveCopy();
  });
  $("apply-theme").addEventListener("click", applyThemeFromBuffer);
  $("apply-keymap").addEventListener("click", applyKeymapFromBuffer);
  $("undo-edit").addEventListener("click", undoEdit);
  $("redo-edit").addEventListener("click", redoEdit);
  $("diff-close").addEventListener("click", hideDiff);
  $("diff-modal").addEventListener("click", (e) => {
    if (e.target === $("diff-modal")) hideDiff();
  });

  // Keep the column ruler aligned as the text scrolls horizontally.
  $("content").addEventListener("scroll", () => {
    if (state.settings.ruler) {
      $("ruler-inner").style.transform = `translateX(${-$("content").scrollLeft}px)`;
    }
  });

  document.addEventListener("keydown", onGlobalKey);
  window.addEventListener("resize", scheduleRender);
}

function toggleOpt(key, id) {
  state[key] = !state[key];
  $(id).classList.toggle("on", state[key]);
  state.lastMatch = null;
  state.searchHits = null;
  state.searchTruncated = false;
  buildMatcher();
  scheduleRender();
  if (state.query) updateCount();
}

// ---- generic input prompt (replaces the browser's window.prompt) ---------
function promptVisible() { return !$("prompt").classList.contains("hidden"); }
function askPrompt(title, label, value = "") {
  return new Promise((resolve) => {
    const modal = $("prompt");
    $("prompt-title").textContent = title || "入力";
    $("prompt-label").textContent = label || "";
    const input = $("prompt-input");
    input.value = value;
    setModalOpen(modal, true);
    setTimeout(() => { input.focus(); input.select(); }, 0);
    const finish = (val) => {
      setModalOpen(modal, false);
      input.removeEventListener("keydown", onKey);
      $("prompt-ok").removeEventListener("click", onOk);
      $("prompt-cancel").removeEventListener("click", onCancel);
      $("prompt-close").removeEventListener("click", onCancel);
      modal.removeEventListener("mousedown", onBackdrop);
      focusEditor();
      resolve(val);
    };
    const onOk = () => finish(input.value);
    const onCancel = () => finish(null);
    const onKey = (ev) => {
      ev.stopPropagation();
      if (ev.key === "Enter") { ev.preventDefault(); finish(input.value); }
      else if (ev.key === "Escape") { ev.preventDefault(); finish(null); }
    };
    const onBackdrop = (ev) => { if (ev.target === modal) finish(null); };
    input.addEventListener("keydown", onKey);
    $("prompt-ok").addEventListener("click", onOk);
    $("prompt-cancel").addEventListener("click", onCancel);
    $("prompt-close").addEventListener("click", onCancel);
    modal.addEventListener("mousedown", onBackdrop);
  });
}

// ---- generic small form dialog (sort / replace / case options) ------------
function formVisible() { return !$("form-modal").classList.contains("hidden"); }

// fields: {id, type: "text"|"check"|"select"|"hint", label, value, placeholder,
// title, options}. Resolves to {id: value} or null on cancel.
function askForm(title, fields, okLabel = "実行") {
  return new Promise((resolve) => {
    const modal = $("form-modal");
    const body = $("form-body");
    $("form-title").textContent = title || "オプション";
    $("form-ok").textContent = okLabel;
    body.textContent = "";
    const readers = {};
    for (const f of fields) {
      if (f.type === "hint") {
        const hint = document.createElement("div");
        hint.className = "form-hint";
        hint.textContent = f.label;
        body.append(hint);
        continue;
      }
      if (f.type === "check") {
        const lab = document.createElement("label");
        lab.className = "form-check";
        if (f.title) lab.title = f.title;
        const cb = document.createElement("input");
        cb.type = "checkbox";
        cb.checked = !!f.value;
        lab.append(cb, document.createTextNode(f.label));
        body.append(lab);
        readers[f.id] = () => cb.checked;
        continue;
      }
      const row = document.createElement("label");
      row.className = "form-row";
      const span = document.createElement("span");
      span.textContent = f.label;
      row.append(span);
      if (f.type === "select") {
        const sel = document.createElement("select");
        for (const [v, text] of f.options || []) {
          const o = document.createElement("option");
          o.value = v;
          o.textContent = text;
          sel.append(o);
        }
        if (f.value != null) sel.value = f.value;
        row.append(sel);
        readers[f.id] = () => sel.value;
      } else {
        const input = document.createElement("input");
        input.type = "text";
        input.value = f.value ?? "";
        input.placeholder = f.placeholder ?? "";
        if (f.title) input.title = f.title;
        row.append(input);
        readers[f.id] = () => input.value;
      }
      body.append(row);
    }
    setModalOpen(modal, true);
    queueMicrotask(() => body.querySelector("input, select")?.focus());
    const finish = (val) => {
      setModalOpen(modal, false);
      $("form-ok").removeEventListener("click", onOk);
      $("form-cancel").removeEventListener("click", onCancel);
      $("form-close").removeEventListener("click", onCancel);
      modal.removeEventListener("mousedown", onBackdrop);
      modal.removeEventListener("keydown", onKey);
      focusEditor();
      resolve(val);
    };
    const collect = () =>
      Object.fromEntries(Object.entries(readers).map(([k, read]) => [k, read()]));
    const onOk = () => finish(collect());
    const onCancel = () => finish(null);
    const onKey = (ev) => {
      ev.stopPropagation();
      if (ev.key === "Enter" && ev.target.tagName !== "SELECT") { ev.preventDefault(); finish(collect()); }
      else if (ev.key === "Escape") { ev.preventDefault(); finish(null); }
    };
    const onBackdrop = (ev) => { if (ev.target === modal) finish(null); };
    $("form-ok").addEventListener("click", onOk);
    $("form-cancel").addEventListener("click", onCancel);
    $("form-close").addEventListener("click", onCancel);
    modal.addEventListener("mousedown", onBackdrop);
    modal.addEventListener("keydown", onKey);
  });
}

// ---- loading overlay ------------------------------------------------------
function showLoading(text) {
  const o = $("overlay");
  o.textContent = text || "読み込み中…";
  o.classList.remove("hidden");
}
function hideLoading() { $("overlay").classList.add("hidden"); }

// Jump the caret to a 1-based line number.
function gotoLine(n) {
  const v = parseInt(String(n).replace(/[^0-9]/g, ""), 10);
  if (!Number.isFinite(v) || v < 1) return;
  const line = Math.min(v - 1, Math.max(0, state.total - 1));
  state.sel = null;
  setCaret(line, 0);
  revealLine(line);
  focusEditor();
}

// App-level shortcuts. Caret motion and text editing live in onEditKey (bound
// to the hidden input); those keys never reach here because onEditKey stops
// their propagation. `inField` is true only for the real text inputs (find /
// opener / prompt / settings), never the editor's hidden textarea.
function onGlobalKey(e) {
  const inField = e.target.tagName === "INPUT";
  if (promptVisible() || formVisible()) return;
  if (e.key === "Escape" && ctxMenuVisible()) { e.preventDefault(); hideCtxMenu(); return; }
  if (e.key === "Escape" && fileMenuVisible()) { e.preventDefault(); hideFileMenu(true); return; }
  if (e.key === "Escape" && keymapVisible()) { e.preventDefault(); hideKeymap(); return; }
  if (e.key === "Escape" && diffVisible()) { e.preventDefault(); hideDiff(); return; }
  if (e.key === "Escape" && settingsVisible()) { e.preventDefault(); hideSettings(); return; }
  if (e.key === "Escape" && openerVisible()) { e.preventDefault(); hideOpener(); return; }
  // A modal owns the keyboard: never run editor/clipboard/history/nav commands
  // against the hidden document behind Settings / the Opener / a prompt.
  if (anyModalOpen()) return;
  if (matchesShortcut(e, "openFile")) { e.preventDefault(); hideFileMenu(); showOpener(); return; }
  if (matchesShortcut(e, "toggleSidebar")) { e.preventDefault(); setSidebar(!sidebarOpen()); return; }
  if (matchesShortcut(e, "newFile")) { e.preventDefault(); hideFileMenu(); newUntitled(); return; }
  if (matchesShortcut(e, "gotoLine")) {
    e.preventDefault();
    askPrompt("行へ移動", "行番号").then((v) => { if (v != null) gotoLine(v); });
    return;
  }
  if (matchesShortcut(e, "closeTab")) {
    e.preventDefault();
    const active = state.tabs.find((t) => t.active);
    if (active) closeTab(active.id);
    return;
  }
  if (matchesShortcut(e, "find")) { e.preventDefault(); showFind(); return; }
  if (matchesShortcut(e, "saveAs")) { e.preventDefault(); hideFileMenu(); saveCopy(); return; }
  if (matchesShortcut(e, "saveFile")) { e.preventDefault(); hideFileMenu(); saveFile(); return; }
  if (matchesShortcut(e, "findPrev")) { e.preventDefault(); findStep("prev"); return; }
  if (matchesShortcut(e, "findNext")) { e.preventDefault(); findStep("next"); return; }
  if (matchesShortcut(e, "searchCase")) { e.preventDefault(); toggleOpt("ci", "opt-case"); return; }
  if (matchesShortcut(e, "searchRegex")) { e.preventDefault(); toggleOpt("regex", "opt-regex"); return; }
  if (matchesShortcut(e, "searchWord")) { e.preventDefault(); toggleOpt("word", "opt-word"); return; }
  if (matchesShortcut(e, "sortSave")) { e.preventDefault(); sortSave(); return; }
  if (matchesShortcut(e, "replaceSave")) { e.preventDefault(); replaceSave(); return; }
  if (matchesShortcut(e, "diffFile")) { e.preventDefault(); diffFile(); return; }
  // Editor clipboard / history — not while typing in a search or dialog field.
  if (inField) return;
  if (matchesShortcut(e, "selectAll")) { e.preventDefault(); selectAll(); return; }
  if (matchesShortcut(e, "copy")) { e.preventDefault(); copySelection(); return; }
  if (matchesShortcut(e, "cut")) { e.preventDefault(); cutSelection(); return; }
  if (matchesShortcut(e, "redo")) { e.preventDefault(); redoEdit(); return; }
  if (matchesShortcut(e, "undo")) { e.preventDefault(); undoEdit(); return; }
}

// ---- editor keyboard: caret motion + structural edits ----------------------

const isWordChar = (ch) => /[\p{L}\p{N}_]/u.test(ch || "");

function wordLeft(line, col) {
  const cs = lineChars(line);
  if (col === 0) return line > 0 ? [line - 1, lineLen(line - 1)] : [line, 0];
  let i = col;
  while (i > 0 && !isWordChar(cs[i - 1])) i--;
  while (i > 0 && isWordChar(cs[i - 1])) i--;
  return [line, i];
}

function wordRight(line, col) {
  const cs = lineChars(line);
  const len = cs.length;
  if (col >= len) return line < state.total - 1 ? [line + 1, 0] : [line, len];
  let i = col;
  while (i < len && !isWordChar(cs[i])) i++;
  while (i < len && isWordChar(cs[i])) i++;
  return [line, i];
}

function deleteWordBack() {
  enqueueEdit(() => {
    clearExtraCursors(); // word-delete is single-cursor: collapse to the primary
    const del = deleteSelectionEdit();
    if (del) return del;
    const c = state.caret;
    const [l, col] = wordLeft(c.line, c.col);
    if (l === c.line && col === c.col) return null;
    return applyRange(l, col, c.line, c.col, "");
  });
}

function deleteWordFwd() {
  enqueueEdit(() => {
    clearExtraCursors(); // word-delete is single-cursor: collapse to the primary
    const del = deleteSelectionEdit();
    if (del) return del;
    const c = state.caret;
    const [l, col] = wordRight(c.line, c.col);
    if (l === c.line && col === c.col) return null;
    return applyRange(c.line, c.col, l, col, "");
  });
}

function onEditKey(e) {
  if (state.composing || e.isComposing) return; // IME owns the keyboard
  if (anyModalOpen()) return; // a dialog is up; don't edit behind it
  if (savingCount > 0) {
    // Edits are blocked while a save is in flight; swallow the key so the
    // hidden textarea can't buffer text that would never be applied.
    e.preventDefault();
    flashCount("保存中です — 完了までお待ちください");
    return;
  }
  const mod = e.ctrlKey || e.metaKey;
  const shift = e.shiftKey;
  const c = state.caret;
  const take = () => { e.preventDefault(); e.stopPropagation(); };
  // Multi-cursor: add a caret above/below (default Ctrl+Alt+ArrowUp/Down).
  // Checked before the switch so the plain-arrow cases never swallow them.
  if (matchesShortcut(e, "addCursorAbove")) { take(); addCursorAbove(); return; }
  if (matchesShortcut(e, "addCursorBelow")) { take(); addCursorBelow(); return; }
  switch (e.key) {
    case "ArrowLeft":
      take();
      if (mod) { const [l, col] = wordLeft(c.line, c.col); moveCaret(l, col, shift); }
      else if (c.col > 0) moveCaret(c.line, c.col - 1, shift);
      else if (c.line > 0) moveCaret(c.line - 1, lineLen(c.line - 1), shift);
      state.goalCol = state.caret.col;
      return;
    case "ArrowRight":
      take();
      if (mod) { const [l, col] = wordRight(c.line, c.col); moveCaret(l, col, shift); }
      else if (c.col < lineLen(c.line)) moveCaret(c.line, c.col + 1, shift);
      else if (c.line < state.total - 1) moveCaret(c.line + 1, 0, shift);
      state.goalCol = state.caret.col;
      return;
    case "ArrowUp":
      take();
      if (mod) setFirst(state.first - 1);
      else if (c.line > 0) moveCaret(c.line - 1, state.goalCol, shift);
      return;
    case "ArrowDown":
      take();
      if (mod) setFirst(state.first + 1);
      else if (c.line < state.total - 1) moveCaret(c.line + 1, state.goalCol, shift);
      return;
    case "Home":
      take();
      moveCaret(mod ? 0 : c.line, 0, shift);
      state.goalCol = state.caret.col;
      return;
    case "End":
      take();
      if (mod) { const last = state.total - 1; moveCaret(last, lineLen(last), shift); }
      else moveCaret(c.line, lineLen(c.line), shift);
      state.goalCol = state.caret.col;
      return;
    case "PageUp":
      take();
      moveCaret(c.line - rowsVisible(), state.goalCol, shift);
      return;
    case "PageDown":
      take();
      moveCaret(c.line + rowsVisible(), state.goalCol, shift);
      return;
    case "Backspace":
      take();
      mod ? deleteWordBack() : backspace();
      return;
    case "Delete":
      take();
      mod ? deleteWordFwd() : forwardDelete();
      return;
    case "Enter":
      take();
      insertNewline();
      return;
    case "Tab":
      if (mod) return; // don't trap window focus-cycling combos
      take();
      typeText("\t");
      return;
    case "Escape":
      // Collapsing multi-cursor wins over every other Escape meaning here
      // (modals/find never reach this handler — see the guards above).
      if (state.extraCursors.length) { take(); clearExtraCursors(); return; }
      if (state.sel) { take(); state.sel = null; scheduleRender(); }
      return;
    default:
      return; // printable input flows through beforeinput / composition
  }
}

function onBeforeInput(e) {
  if (state.composing) return; // composition text is committed on compositionend
  if (anyModalOpen()) { e.preventDefault(); return; }
  switch (e.inputType) {
    case "insertText":
      e.preventDefault();
      if (e.data != null) typeText(e.data);
      break;
    case "insertLineBreak":
    case "insertParagraph":
      e.preventDefault();
      insertNewline();
      break;
    case "deleteContentBackward":
    case "deleteSoftLineBackward":
      e.preventDefault();
      backspace();
      break;
    case "deleteWordBackward":
      e.preventDefault();
      deleteWordBack();
      break;
    case "deleteContentForward":
    case "deleteSoftLineForward":
      e.preventDefault();
      forwardDelete();
      break;
    case "deleteWordForward":
      e.preventDefault();
      deleteWordFwd();
      break;
    case "insertFromPaste":
      e.preventDefault(); // the paste event carries the clipboard text
      break;
    default:
      break;
  }
}

function onPaste(e) {
  const text = (e.clipboardData || window.clipboardData)?.getData("text") ?? "";
  e.preventDefault();
  if (text) pasteText(text);
}

function onCompStart() {
  state.composing = true;
  $("hidden-input").classList.add("composing");
  positionCaret();
}
function onCompUpdate() {
  positionCaret(); // the textarea itself renders the composing string
}
function onCompEnd(e) {
  state.composing = false;
  const hi = $("hidden-input");
  hi.classList.remove("composing");
  const data = e.data || "";
  hi.value = "";
  if (data) typeText(data);
  else scheduleRender();
}

function initEditor() {
  const hi = $("hidden-input");
  hi.addEventListener("keydown", onEditKey);
  hi.addEventListener("beforeinput", onBeforeInput);
  hi.addEventListener("input", () => { if (!state.composing) hi.value = ""; });
  hi.addEventListener("paste", onPaste);
  hi.addEventListener("compositionstart", onCompStart);
  hi.addEventListener("compositionupdate", onCompUpdate);
  hi.addEventListener("compositionend", onCompEnd);
  hi.addEventListener("focus", () => { state.focused = true; scheduleRender(); });
  hi.addEventListener("blur", () => { state.focused = false; scheduleRender(); });
  // Keep the caret glued to its cell during horizontal scroll.
  $("content").addEventListener("scroll", positionCaret);
}

// ---- workspace: open / browse / drag&drop ----------------------------------

function openerVisible() {
  return !$("opener").classList.contains("hidden");
}

function showOpener() {
  configureOpener("open");
  setModalOpen($("opener"), true);
  browse(null);
  const inp = $("opener-input");
  inp.value = "";
  queueMicrotask(() => inp.focus());
}

function showSaveDialog(title, suggestedPath) {
  return new Promise((resolve) => {
    configureOpener("save", title);
    state.openerResolve = resolve;
    const inp = $("opener-input");
    const dir = pathDirName(suggestedPath) || localStorage.getItem(TREE_KEY) || ".";
    inp.value = pathBaseName(suggestedPath) || "untitled.txt";
    setModalOpen($("opener"), true);
    browse(dir);
    queueMicrotask(() => {
      inp.focus();
      inp.select();
    });
  });
}

function configureOpener(mode, title) {
  state.openerMode = mode;
  const save = mode === "save";
  const m = $("opener");
  m.classList.toggle("save-mode", save);
  $("opener-title").textContent = title || (save ? "別名で保存" : "ファイルを開く");
  $("opener-input-label").textContent = save ? "ファイル名" : "パス";
  $("opener-input").placeholder = save
    ? "保存するファイル名、またはフルパス"
    : "ファイルのパスを入力… (例: /var/log/huge.log)";
  $("opener-open").textContent = save ? "保存" : "開く";
  $("opener-folder").textContent = save ? "場所" : "フォルダ";
  $("opener-folder").title = save ? "表示中のフォルダをエクスプローラーに表示" : "表示中のフォルダをツリーに開く";
  $("opener-hint").textContent = save
    ? "フォルダを選び、保存するファイル名を入力します。既存ファイルを選ぶと上書き確認します。"
    : "ここへファイルをドラッグ＆ドロップしても開けます。大きなファイルはパス指定の方が高速です。";
  openerMsg("");
}

function hideOpener() {
  if (state.openerMode === "save") {
    finishSaveDialog(null);
    return;
  }
  // The opener doubles as the welcome screen: don't let it close while there is
  // no document to fall back to.
  if (!state.stat?.open) return;
  setModalOpen($("opener"), false);
  focusEditor();
}

function finishSaveDialog(value) {
  const resolve = state.openerResolve;
  state.openerResolve = null;
  state.openerMode = "open";
  setModalOpen($("opener"), false);
  configureOpener("open");
  focusEditor();
  if (resolve) resolve(value);
}

function openerMsg(text, busy = false) {
  const el = $("opener-msg");
  el.textContent = text || "";
  el.classList.toggle("busy", !!text && busy);
}

async function browse(dir) {
  openerMsg("読み込み中…", true);
  try {
    const q = dir == null ? "" : `?dir=${encodeURIComponent(dir)}`;
    const res = await api(`/api/browse${q}`);
    renderBrowse(res);
    openerMsg("");
  } catch (e) {
    openerMsg("ディレクトリを開けません: " + e.message);
  }
}

function renderBrowse(res) {
  state.openerDir = res.dir;
  state.openerEntries = res.entries || [];
  renderCwdCrumbs(res.dir);
  const list = $("opener-list");
  list.textContent = "";
  if (res.parent) {
    list.append(browseRow({ name: "..", path: res.parent, is_dir: true }, true));
  }
  for (const ent of res.entries) list.append(browseRow(ent, false));
  list.scrollTop = 0;
}

function renderCwdCrumbs(path) {
  const cwd = $("opener-cwd");
  const clean = String(path || "").replace(/^\\\\\?\\/, "");
  cwd.textContent = "";
  cwd.title = clean;
  for (const [i, crumb] of pathCrumbs(clean).entries()) {
    if (i > 0) {
      const sep = document.createElement("span");
      sep.className = "cwd-sep";
      sep.textContent = "›";
      cwd.append(sep);
    }
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "cwd-crumb";
    btn.textContent = crumb.label;
    btn.title = crumb.path;
    btn.addEventListener("click", () => browse(crumb.path));
    cwd.append(btn);
  }
}

function browseRow(ent, isUp) {
  const row = document.createElement("button");
  row.className = "opener-row" + (ent.is_dir ? " dir" : "") + (isUp ? " up" : "");
  row.type = "button";
  const ic = document.createElement("span");
  ic.className = "ic";
  ic.textContent = isUp ? "上へ" : ent.is_dir ? "フォルダ" : "ファイル";
  const nm = document.createElement("span");
  nm.className = "nm";
  nm.textContent = isUp ? "上の階層へ" : ent.name;
  const sz = document.createElement("span");
  sz.className = "sz";
  sz.textContent = ent.is_dir ? "" : humanBytes(ent.size);
  row.append(ic, nm, sz);
  row.addEventListener("click", () => {
    if (ent.is_dir) browse(ent.path);
    else if (state.openerMode === "save") {
      $("opener-input").value = ent.name;
      markPickedFile(ent.name);
      $("opener-input").focus();
    }
    else openPath(ent.path);
  });
  row.addEventListener("dblclick", () => {
    if (!ent.is_dir && state.openerMode === "save") commitOpener();
  });
  return row;
}

function markPickedFile(name) {
  for (const row of $("opener-list").querySelectorAll(".opener-row")) {
    row.classList.toggle("picked", row.querySelector(".nm")?.textContent === name);
  }
}

function saveDialogTarget() {
  const raw = $("opener-input").value.trim();
  if (!raw) {
    openerMsg("保存するファイル名を入力してください");
    return null;
  }
  const path = isAbsolutePath(raw) ? raw : joinPath(state.openerDir, raw);
  const base = pathBaseName(path);
  const existing = state.openerEntries.find((e) => !e.is_dir && e.name === base);
  const overwrite = !!existing;
  if (overwrite && !confirm(`${base} は既に存在します。上書きしますか?`)) return null;
  return { path, overwrite };
}

function commitOpener() {
  if (state.openerMode === "save") {
    const target = saveDialogTarget();
    if (target) finishSaveDialog(target);
    return;
  }
  openPath($("opener-input").value);
}

function confirmDiscardIfDirty() {
  if (!state.stat?.dirty) return true;
  return confirm("未保存の編集があります。別のファイルを開くと破棄されます。開きますか?");
}

async function openPath(path) {
  const p = (path || "").trim();
  if (!p) return;
  await settleEditQueue();
  if (!confirmDiscardIfDirty()) return;
  openerMsg("開いています…", true);
  try {
    const stat = await apiPost("/api/open", { path: p });
    onDocumentOpened(stat);
  } catch (e) {
    reportOpenError("開けません: " + e.message);
  }
}

async function uploadFile(file) {
  await settleEditQueue();
  if (!confirmDiscardIfDirty()) return;
  openerMsg(`読み込み中… (${file.name})`, true);
  showLoading(`読み込み中… ${file.name}`);
  try {
    const r = await fetch(`/api/upload?name=${encodeURIComponent(file.name)}`, {
      method: "POST",
      body: file,
    });
    if (!r.ok) throw new Error((await r.text()) || r.statusText);
    onDocumentOpened(await r.json());
  } catch (e) {
    reportOpenError("読み込みエラー: " + e.message);
  } finally {
    hideLoading();
  }
}

// Surface an open/upload failure where the user is looking: inside the opener if
// it's up, otherwise in the toolbar (and an alert if a doc is already open).
function reportOpenError(msg) {
  if (openerVisible()) {
    openerMsg(msg);
  } else if (state.stat?.open) {
    flashCount("読み込みエラー");
    alert(msg);
  } else {
    showOpener();
    openerMsg(msg);
  }
}

function onDocumentOpened(stat) {
  state.docGen++;
  state.editGen++; // stale in-flight edit responses must not reposition this tab
  state.stat = stat;
  state.total = stat.view_lines ?? stat.lines ?? 0;
  // Fresh document: reset navigation, search, and caret state.
  state.first = 0;
  state.caret = { line: 0, col: 0 };
  state.goalCol = 0;
  state.activeLine = 0;
  state.sel = null;
  state.extraCursors = [];
  state.lastMatch = null;
  state.searchHits = null;
  state.searchTruncated = false;
  $("find-count").textContent = "";
  clearLineCache();
  setModalOpen($("opener"), false);
  updateStatusMeta();
  render();
  refreshTabs();
  updateTreeActive();
  focusEditor();
}

function hasFiles(e) {
  const t = e.dataTransfer;
  return !!t && Array.from(t.types || []).includes("Files");
}

function initDropZone() {
  const dz = $("dropzone");
  let depth = 0;
  window.addEventListener("dragenter", (e) => {
    if (!hasFiles(e)) return;
    e.preventDefault();
    depth++;
    dz.classList.remove("hidden");
  });
  window.addEventListener("dragover", (e) => {
    if (!hasFiles(e)) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = "copy";
  });
  window.addEventListener("dragleave", (e) => {
    if (!hasFiles(e)) return;
    depth = Math.max(0, depth - 1);
    if (depth === 0) dz.classList.add("hidden");
  });
  window.addEventListener("drop", (e) => {
    if (!hasFiles(e)) return;
    e.preventDefault();
    depth = 0;
    dz.classList.add("hidden");
    const file = e.dataTransfer.files[0];
    if (file) uploadFile(file);
  });
}

// ---- tabs ------------------------------------------------------------------

async function refreshTabs() {
  try {
    const r = await api("/api/tabs");
    renderTabs(r.tabs);
  } catch {
    // non-fatal: the tab bar just won't update
  }
}

function renderTabs(list) {
  state.tabs = list;
  const c = $("tabs");
  c.setAttribute("role", "tablist");
  c.textContent = "";
  for (const t of list) {
    const el = document.createElement("div");
    el.className = "tab" + (t.active ? " active" : "") + (t.dirty ? " dirty" : "");
    el.dataset.id = String(t.id);
    el.title = t.path;
    el.setAttribute("role", "tab");
    el.setAttribute("aria-selected", t.active ? "true" : "false");
    el.tabIndex = 0;
    const dot = document.createElement("span");
    dot.className = "tab-dot";
    const nm = document.createElement("span");
    nm.className = "tab-name";
    nm.textContent = t.name;
    const x = document.createElement("button");
    x.type = "button";
    x.className = "tab-x";
    x.textContent = "✕";
    x.title = "閉じる";
    x.setAttribute("aria-label", `${t.name} を閉じる`);
    el.append(dot, nm, x);
    el.addEventListener("click", () => {
      if (!t.active) selectTab(t.id);
    });
    el.addEventListener("keydown", (e) => {
      if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        if (!t.active) selectTab(t.id);
      } else if (e.key === "Delete") {
        e.preventDefault();
        closeTab(t.id);
      }
    });
    el.addEventListener("mousedown", (e) => {
      if (e.button === 1) {
        e.preventDefault();
        closeTab(t.id); // middle-click closes
      }
    });
    x.addEventListener("click", (e) => {
      e.stopPropagation();
      closeTab(t.id);
    });
    c.append(el);
  }
}

async function selectTab(id) {
  try {
    await settleEditQueue();
    onDocumentOpened(await apiPost("/api/tabs/select", { id }));
  } catch (e) {
    flashCount("タブ切替エラー");
    console.error(e);
  }
}

async function closeTab(id) {
  await settleEditQueue();
  const t = state.tabs.find((x) => x.id === id);
  if (t && t.dirty && !confirm(`${t.name} の未保存の編集を破棄して閉じますか?`)) return;
  try {
    const stat = await apiPost("/api/tabs/close", { id });
    if (!stat.open) {
      await newUntitled(); // closed the last tab → open a fresh page
    } else {
      onDocumentOpened(stat);
    }
  } catch (e) {
    flashCount("タブを閉じられません");
    console.error(e);
  }
}

// ---- sidebar file tree ------------------------------------------------------

function sidebarOpen() {
  return !$("sidebar").classList.contains("hidden");
}

function setSidebar(open) {
  $("sidebar").classList.toggle("hidden", !open);
  $("toggle-sidebar").classList.toggle("on", open);
  state.settings = { ...state.settings, sidebar: open };
  saveSettings(state.settings);
  if (open && !state.treeLoaded) {
    state.treeLoaded = true;
    treeSetRoot(localStorage.getItem(TREE_KEY) || null);
  }
  scheduleRender(); // viewport width changed
}

// Load `dir` (or the server default when null) as the tree root.
async function treeSetRoot(dir) {
  try {
    const q = dir ? `?dir=${encodeURIComponent(dir)}` : "";
    const res = await api(`/api/browse${q}`);
    state.treeParent = res.parent;
    $("sb-root").textContent = res.dir;
    $("sb-root").title = res.dir;
    try {
      localStorage.setItem(TREE_KEY, res.dir);
    } catch {
      // ignore quota
    }
    const tree = $("tree");
    tree.textContent = "";
    tree.append(renderTreeEntries(res.entries, 0));
  } catch (e) {
    // A stale saved root: fall back to the server default once.
    if (dir) {
      treeSetRoot(null);
    } else {
      $("tree").textContent = "";
    }
  }
}

function renderTreeEntries(entries, depth) {
  const frag = document.createDocumentFragment();
  for (const ent of entries) frag.append(renderTreeNode(ent, depth));
  return frag;
}

function renderTreeNode(ent, depth) {
  const row = document.createElement("div");
  row.className = "tnode " + (ent.is_dir ? "dir" : "file");
  row.dataset.path = ent.path;
  row.style.setProperty("--depth", String(depth));
  if (!ent.is_dir && ent.path === state.stat?.path) row.classList.add("active");
  const indent = document.createElement("span");
  indent.className = "tindent";
  for (let i = 0; i < depth; i++) {
    const guide = document.createElement("span");
    guide.className = "tguide";
    indent.append(guide);
  }
  const chev = document.createElement("span");
  chev.className = "chev";
  chev.setAttribute("aria-hidden", "true");
  const icon = document.createElement("span");
  icon.className = "ticon " + (ent.is_dir ? "folder" : `file ${treeFileClass(ent.name)}`);
  icon.setAttribute("aria-hidden", "true");
  const nm = document.createElement("span");
  nm.className = "tname";
  nm.textContent = ent.name;
  row.append(indent, chev, icon, nm);

  if (!ent.is_dir) {
    if (typeof ent.size === "number") {
      const meta = document.createElement("span");
      meta.className = "tmeta";
      meta.textContent = humanBytes(ent.size);
      row.append(meta);
    }
    row.title = ent.path;
    row.addEventListener("click", (e) => {
      e.stopPropagation();
      openPath(ent.path); // opens in a new tab
    });
    return row;
  }

  // Folder: lazily load children on first expand.
  const kids = document.createElement("div");
  kids.className = "tkids";
  kids.style.display = "none";
  let loaded = false;
  row.addEventListener("click", async (e) => {
    e.stopPropagation();
    const opening = kids.style.display === "none";
    row.classList.toggle("open", opening);
    if (opening && !loaded) {
      loaded = true;
      try {
        const res = await api(`/api/browse?dir=${encodeURIComponent(ent.path)}`);
        kids.append(renderTreeEntries(res.entries, depth + 1));
      } catch {
        loaded = false;
      }
    }
    kids.style.display = opening ? "block" : "none";
  });
  const frag = document.createDocumentFragment();
  frag.append(row, kids);
  return frag;
}

function treeFileClass(name) {
  const ext = String(name || "").split(".").pop()?.toLowerCase() || "";
  if (ext === "md" || ext === "markdown") return "md";
  if (ext === "py") return "py";
  if (ext === "json") return "json";
  if (ext === "csv" || ext === "tsv" || ext === "xlsx") return "data";
  return "text";
}

function updateTreeActive() {
  const path = state.stat?.path || "";
  document.querySelectorAll("#tree .tnode.file").forEach((row) => {
    row.classList.toggle("active", !!path && row.dataset.path === path);
  });
}

function initTree() {
  $("toggle-sidebar").addEventListener("click", () => setSidebar(!sidebarOpen()));
  $("sb-close").addEventListener("click", () => setSidebar(false));
  $("sb-up").addEventListener("click", () => {
    if (state.treeParent) treeSetRoot(state.treeParent);
  });
  $("opener-folder").addEventListener("click", () => {
    if (!state.openerDir) return;
    if (!sidebarOpen()) setSidebar(true);
    state.treeLoaded = true;
    treeSetRoot(state.openerDir);
    if (state.openerMode === "save") {
      openerMsg("現在のフォルダをエクスプローラーに表示しました");
      return;
    }
    hideOpener();
  });
  // Apply persisted visibility.
  if (state.settings.sidebar) setSidebar(true);
}

// Start a fresh empty "untitled" buffer with a blank editable first line, so
// the app opens to a usable page (like Notepad) instead of a dialog.
async function newUntitled() {
  try {
    await settleEditQueue();
    onDocumentOpened(await apiPost("/api/new", {}));
    // The buffer already has one empty line; drop the caret in, Notepad-style.
    setCaret(0, 0);
    focusEditor();
  } catch (e) {
    showOpener();
    openerMsg("新規バッファを作成できません: " + e.message);
  }
}

function runMenuAction(action) {
  hideFileMenu();
  // A modal owns the UI. Every menu action either opens a dialog or acts on
  // the document hidden behind the modal, and the native macOS menu can fire
  // at any time — so ALL actions are ignored while a modal is open. (In-page
  // menus are unreachable then; this guards the native path.)
  if (anyModalOpen()) return;
  if (action === "undo") return undoEdit();
  if (action === "redo") return redoEdit();
  if (action === "find") return showFind();
  if (action === "gotoLine") {
    askPrompt("行へ移動", "行番号").then((v) => { if (v != null) gotoLine(v); });
    return;
  }
  if (action === "selectAll") return selectAll();
  if (action === "addCursorAbove") return addCursorAbove();
  if (action === "addCursorBelow") return addCursorBelow();
  if (action === "copy") return copySelection();
  if (action === "cut") return cutSelection();
  if (action === "toggleSidebar") return setSidebar(!sidebarOpen());
  if (action === "settings") return showSettings();
  if (action === "sortSave") return sortSave();
  if (action === "replaceSave") return replaceSave();
  if (action === "diffFile") return diffFile();
  if (action === "splitFile") return splitFile();
  if (action === "caseUpper") return caseSave("upper");
  if (action === "caseLower") return caseSave("lower");
  if (action === "keymap") return showKeymap();
  if (action === "revert") return revertEdits();
  if (action === "newFile") return newUntitled();
  if (action === "openFile") return showOpener();
  if (action === "saveFile") return saveFile();
  if (action === "saveAs") return saveCopy();
  if (action === "closeTab") {
    const active = state.tabs.find((t) => t.active);
    if (active) closeTab(active.id);
  }
}

// Native menu dispatcher: the macOS (Rust) side calls this via evaluate_script
// with the same action ids the in-page menus use.
window.__ayameMenu = runMenuAction;

function initMenuBar() {
  for (const id of APP_MENUS) {
    const button = $(`${id}-menu-button`);
    button.addEventListener("click", (e) => {
      e.stopPropagation();
      const open = !$(`${id}-menu`).classList.contains("hidden");
      open ? hideFileMenu() : showAppMenu(id);
    });
    button.addEventListener("pointerenter", () => {
      if (fileMenuVisible()) showAppMenu(id);
    });
  }
  document.querySelectorAll("[data-menu-action]").forEach((item) => {
    item.addEventListener("click", () => runMenuAction(item.dataset.menuAction));
  });
}

function initWorkspace() {
  initMenuBar();
  document.addEventListener("pointerdown", (e) => {
    if (fileMenuVisible() && !e.target.closest(".menu-shell")) hideFileMenu();
  });
  $("new-file").addEventListener("click", () => {
    hideFileMenu();
    newUntitled();
  });
  $("open-file").addEventListener("click", () => {
    hideFileMenu();
    showOpener();
  });
  $("opener-close").addEventListener("click", hideOpener);
  $("opener-open").addEventListener("click", commitOpener);
  $("opener-input").addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      commitOpener();
    } else if (e.key === "Escape") {
      e.preventDefault();
      hideOpener();
    }
  });
  // Click on the dim backdrop (outside the panel) closes the dialog.
  $("opener").addEventListener("click", (e) => {
    if (e.target === $("opener")) hideOpener();
  });
  $("new-tab").addEventListener("click", () => newUntitled());
  initDropZone();
}

// ---- settings (theme / font) -----------------------------------------------

function loadSettings() {
  try {
    const raw = JSON.parse(localStorage.getItem(SETTINGS_KEY) || "{}");
    const merged = { ...DEFAULT_SETTINGS, ...(raw && typeof raw === "object" ? raw : {}) };
    merged.sidebarSide = merged.sidebarSide === "right" ? "right" : "left";
    merged.keymap = sanitizeKeymap(merged.keymap);
    return merged;
  } catch {
    return { ...DEFAULT_SETTINGS };
  }
}

function saveSettings(s) {
  try {
    localStorage.setItem(SETTINGS_KEY, JSON.stringify(s));
  } catch {
    // ignore private-mode quota errors
  }
}

// Built-in themes are also defined as CSS `html[data-theme=...]` blocks in
// style.css; these JSON mirrors let the Settings JSON editor show/export them
// and act as a base for custom themes. Custom themes apply at runtime by
// setting the same CSS variables the built-ins use.
const THEME_PRESETS = {
  "iris-light": {"name":"Iris Light","type":"light","radius":10,
    "color":{"paper":"#FBF8F1","paper2":"#FDFCF8","ink":"#2A2140","inkDim":"#6E6383","inkFaint":"#A99DBC","accent":"#7A5CC0","accent2":"#6A4CB0","gold":"#C79A2E","edge":"#E7E0D3","err":"#C0506A","markBg":"#FBEBB0","markFg":"#6B5510","markCur":"#E8B84B","markCurFg":"#2A2205"},
    "acrylic":{"tint":"rgba(255,253,248,0.72)","blur":20},"background":{"mode":"watercolor","solid":"#FBF8F1"},"illustration":0.18,
    "watercolor":[{"x":"12%","y":"84%","r":"46vh","color":"rgba(122,92,192,0.12)"},{"x":"88%","y":"14%","r":"42vh","color":"rgba(185,139,214,0.10)"},{"x":"70%","y":"96%","r":"30vh","color":"rgba(231,197,107,0.08)"}]},
  "iris-mist": {"name":"Iris Mist","type":"light","radius":12,
    "color":{"paper":"#F7F9FC","paper2":"#FDFEFF","ink":"#26314A","inkDim":"#5E6E8A","inkFaint":"#9DAAC0","accent":"#5B79C9","accent2":"#4A68B8","gold":"#C9A24E","edge":"#DCE4EF","err":"#C05C74","markBg":"#E3ECFB","markFg":"#2C3E6B","markCur":"#7EC7C0","markCurFg":"#0F2A28"},
    "acrylic":{"tint":"rgba(250,252,255,0.68)","blur":24},"background":{"mode":"watercolor","solid":"#F7F9FC"},"illustration":0.22,
    "watercolor":[{"x":"14%","y":"82%","r":"44vh","color":"rgba(91,121,201,0.12)"},{"x":"86%","y":"16%","r":"42vh","color":"rgba(143,182,224,0.10)"},{"x":"74%","y":"96%","r":"30vh","color":"rgba(126,199,192,0.08)"}]},
  "iris-dawn": {"name":"Iris Dawn","type":"light","radius":10,
    "color":{"paper":"#FDF6EE","paper2":"#FFFBF7","ink":"#3A2438","inkDim":"#7A5A6E","inkFaint":"#B79AA6","accent":"#A65CB0","accent2":"#944EA0","gold":"#E0A94E","edge":"#EFE0D6","err":"#D96A86","markBg":"#FBE7C8","markFg":"#7A4A16","markCur":"#F0B85A","markCurFg":"#3A2205"},
    "acrylic":{"tint":"rgba(255,250,244,0.70)","blur":20},"background":{"mode":"watercolor","solid":"#FDF6EE"},"illustration":0.22,
    "watercolor":[{"x":"12%","y":"84%","r":"46vh","color":"rgba(166,92,176,0.13)"},{"x":"84%","y":"16%","r":"42vh","color":"rgba(224,169,78,0.11)"},{"x":"70%","y":"96%","r":"30vh","color":"rgba(227,154,176,0.10)"}]},
  "sumi-light": {"name":"Sumi Light","type":"light","radius":10,
    "color":{"paper":"#FAFAF8","paper2":"#FFFFFF","ink":"#222024","inkDim":"#63616A","inkFaint":"#A7A4AE","accent":"#7A5CC0","accent2":"#6A4CB0","gold":"#B7912F","edge":"#E6E4DE","err":"#B24A5E","markBg":"#ECE6FA","markFg":"#3E2E63","markCur":"#7A5CC0","markCurFg":"#FFFFFF"},
    "acrylic":{"tint":"rgba(252,252,250,0.74)","blur":22},"background":{"mode":"watercolor","solid":"#FAFAF8"},"illustration":0.16,
    "watercolor":[{"x":"16%","y":"82%","r":"40vh","color":"rgba(122,92,192,0.07)"},{"x":"84%","y":"20%","r":"34vh","color":"rgba(40,36,48,0.03)"}]},
  "mono-paper": {"name":"Mono Paper (単色)","type":"light","radius":10,
    "color":{"paper":"#F5F3ED","paper2":"#FBFAF5","ink":"#24231F","inkDim":"#6C6A63","inkFaint":"#A9A69D","accent":"#6F6B79","accent2":"#605C6C","gold":"#7A7568","edge":"#E2DFD6","err":"#9A6A6A","markBg":"#E7E4EC","markFg":"#3A3745","markCur":"#6F6B79","markCurFg":"#FFFFFF"},
    "acrylic":{"tint":"rgba(245,243,237,0.92)","blur":8},"background":{"mode":"solid","solid":"#F4F2EC"},"illustration":0,"watercolor":[]},
};

// CSS variables a custom/JSON theme drives (cleared when switching back to a
// built-in data-theme so its CSS block wins).
const THEME_VARS = [
  "--bg","--bg-elevated","--bg-toolbar","--bg-active-line","--gutter-bg","--edit-bg",
  "--fg","--fg-dim","--fg-faint","--border","--accent","--accent-bright","--status",
  "--status-fg","--gutter-fg","--mark-bg","--mark-fg","--mark-active-bg","--mark-active-fg",
  "--danger","--gold","--desk","--illus","--radius","--acrylic-blur",
];
function clearCustomVars() {
  const r = document.documentElement.style;
  THEME_VARS.forEach((v) => r.removeProperty(v));
}
function deskFrom(t) {
  const bg = t.background || { mode: "watercolor" };
  if (bg.mode === "solid") return bg.solid || t.color.paper2 || t.color.paper;
  const layers = (t.watercolor || []).map(
    (b) => `radial-gradient(${b.r} ${b.r} at ${b.x} ${b.y}, ${b.color}, transparent 62%)`
  );
  layers.push(t.color.paper);
  return layers.join(", ");
}
function applyCustomVars(t) {
  const r = document.documentElement.style, c = t.color || {};
  const S = (k, v) => v != null && r.setProperty(k, v);
  S("--bg", c.paper); S("--bg-elevated", c.paper2 || c.paper); S("--bg-toolbar", (t.acrylic && t.acrylic.tint) || c.paper);
  S("--bg-active-line", `color-mix(in srgb, ${c.accent} 14%, ${c.paper})`);
  S("--gutter-bg", c.paper); S("--edit-bg", c.paper2 || c.paper);
  S("--fg", c.ink); S("--fg-dim", c.inkDim); S("--fg-faint", c.inkFaint); S("--border", c.edge);
  S("--accent", c.accent); S("--accent-bright", c.accent2 || c.accent);
  S("--status", (t.acrylic && t.acrylic.tint) || c.paper); S("--status-fg", c.inkDim);
  S("--gutter-fg", c.inkFaint); S("--mark-bg", c.markBg); S("--mark-fg", c.markFg);
  S("--mark-active-bg", c.markCur); S("--mark-active-fg", c.markCurFg); S("--danger", c.err);
  S("--gold", c.gold); S("--radius", (t.radius || 10) + "px");
  S("--acrylic-blur", ((t.acrylic && t.acrylic.blur) ?? 20) + "px");
  S("--desk", deskFrom(t)); S("--illus", String(t.illustration ?? 0.2));
}

function applySettings(s) {
  const root = document.documentElement;
  // ---- theme (built-in CSS block, or a custom JSON theme at runtime) ----
  clearCustomVars();
  if (s.theme && s.theme.startsWith("custom:")) {
    const t = (s.customThemes || {})[s.theme.slice(7)];
    root.dataset.theme = "custom";
    if (t) applyCustomVars(t);
  } else {
    root.dataset.theme = s.theme || "iris-light"; // iris-* | dark | black (unknown → :root)
  }
  root.dataset.sidebarSide = s.sidebarSide === "right" ? "right" : "left";
  // ---- background mode + illustration (user overrides on top of the theme) ----
  if (s.bgMode === "solid") {
    const flat = getComputedStyle(root).getPropertyValue("--bg").trim() || "#FBF8F1";
    root.style.setProperty("--desk", flat);
  }
  if (typeof s.illus === "number") root.style.setProperty("--illus", String(s.illus));
  // ---- font / size ----
  root.style.setProperty("--mono", FONT_STACKS[s.font] || FONT_STACKS.mono);
  const fs = Math.max(11, Math.min(22, Number(s.fontSize) || 13));
  root.style.setProperty("--font-size", `${fs}px`);
  const lh = fs + 6;
  root.style.setProperty("--line-height", `${lh}px`);
  LINE_HEIGHT = lh; // keep virtualization math in sync with the CSS
  _charW = 0; // font metrics changed → remeasure on next click
  _rulerKey = ""; // force the ruler to rebuild against the new metrics
  scheduleRender();
}

function updateSetting(key, value) {
  state.settings = { ...state.settings, [key]: value };
  applySettings(state.settings);
  saveSettings(state.settings);
  if (key === "sidebarSide") updateSidebarSideButtons();
}

function settingsVisible() {
  return !$("settings").classList.contains("hidden");
}
function showSettings() {
  setModalOpen($("settings"), true);
}
function hideSettings() {
  setModalOpen($("settings"), false);
  focusEditor();
}

// ---- theme JSON editor (in Settings) --------------------------------------

function themeJSONFor(id) {
  if (id && id.startsWith("custom:")) return (state.settings.customThemes || {})[id.slice(7)] || null;
  return THEME_PRESETS[id] || null;
}
function themeIllusPct(id) {
  const t = themeJSONFor(id);
  return Math.round(((t && t.illustration) ?? 0) * 100);
}
function populateThemeSelect() {
  const sel = $("set-theme");
  [...sel.querySelectorAll("option[data-custom]")].forEach((o) => o.remove());
  for (const name of Object.keys(state.settings.customThemes || {})) {
    const o = document.createElement("option");
    o.value = "custom:" + name; o.textContent = "★ " + name; o.dataset.custom = "1";
    sel.appendChild(o);
  }
}
function persistCustomTheme(t) {
  const customs = { ...(state.settings.customThemes || {}) };
  customs[t.name] = t;
  state.settings = {
    ...state.settings, customThemes: customs, theme: "custom:" + t.name,
    illus: null, bgMode: (t.background && t.background.mode) || "watercolor",
  };
  saveSettings(state.settings);
  populateThemeSelect();
  if ($("set-theme")) $("set-theme").value = "custom:" + t.name;
}

// Open the current theme's JSON as an ordinary editor tab, so it can be edited
// like any text file (edit / undo / Ctrl+S), then applied with テーマ適用.
async function openThemeJsonDoc() {
  const id = state.settings.theme;
  const t = themeJSONFor(id) || THEME_PRESETS["iris-light"];
  const jsonText = JSON.stringify(t, null, 2);
  const base = (id ? id.replace(/^custom:/, "") : "theme") || "theme";
  hideSettings();
  try {
    await settleEditQueue();
    const r = await fetch("/api/upload?name=" + encodeURIComponent(base + ".ayame-theme.json"),
                          { method: "POST", body: jsonText });
    if (!r.ok) throw new Error(await r.text());
    onDocumentOpened(await r.json());
  } catch (e) {
    flashCount("テーマを開けません");
    console.error(e);
  }
}

// Apply the theme JSON in the active buffer (a *.ayame-theme.json tab).
async function applyThemeFromBuffer() {
  try {
    const count = Math.min(state.total, MAX_COPY_LINES);
    const r = await api(`/api/lines?start=0&count=${count}`);
    const text = r.lines.map((l) => l.text).join("\n");
    const t = JSON.parse(text);
    if (!t.color) return flashCount("color がありません");
    document.documentElement.dataset.theme = "custom";
    clearCustomVars();
    applyCustomVars(t);
    if (t.name) persistCustomTheme(t);
    flashCount(`テーマ適用${t.name ? `: ${t.name}` : ""}`);
  } catch (e) {
    flashCount("テーマ JSON エラー");
    console.error(e);
  }
}
function isThemeDoc(path) {
  return !!path && /\.ayame-theme\.json$/i.test(path);
}

function keymapJSONForEditor() {
  const out = {};
  for (const [action] of KEYMAP_ACTIONS) {
    out[action] = Object.prototype.hasOwnProperty.call(state.settings.keymap || {}, action)
      ? state.settings.keymap[action]
      : DEFAULT_KEYMAP[action];
  }
  return out;
}

async function openKeymapJsonDoc() {
  hideKeymap();
  try {
    await settleEditQueue();
    const r = await fetch("/api/upload?name=" + encodeURIComponent("keymap.ayame-keys.json"), {
      method: "POST",
      body: JSON.stringify(keymapJSONForEditor(), null, 2),
    });
    if (!r.ok) throw new Error(await r.text());
    onDocumentOpened(await r.json());
  } catch (e) {
    flashCount("キー設定を開けません");
    console.error(e);
  }
}

async function applyKeymapFromBuffer() {
  try {
    const count = Math.min(state.total, MAX_COPY_LINES);
    const r = await api(`/api/lines?start=0&count=${count}`);
    const text = r.lines.map((l) => l.text).join("\n");
    const parsed = JSON.parse(text);
    const clean = sanitizeKeymap(parsed);
    state.settings = { ...state.settings, keymap: clean };
    saveSettings(state.settings);
    updateKeyHints();
    renderKeymapRows();
    flashCount("キー設定適用");
  } catch (e) {
    flashCount("キー設定 JSON エラー");
    console.error(e);
  }
}

function isKeymapDoc(path) {
  return !!path && /\.ayame-keys\.json$/i.test(path);
}

function updateSidebarSideButtons() {
  const side = state.settings.sidebarSide === "right" ? "right" : "left";
  document.querySelectorAll("button[data-sidebar-side]").forEach((btn) => {
    const on = btn.dataset.sidebarSide === side;
    btn.classList.toggle("on", on);
    btn.setAttribute("aria-pressed", on ? "true" : "false");
  });
}

function initSettings() {
  state.settings = loadSettings();
  applySettings(state.settings);
  updateKeyHints();
  populateThemeSelect();
  $("set-theme").value = state.settings.theme;
  $("set-bg").value = state.settings.bgMode || "watercolor";
  const illusPct = state.settings.illus == null ? themeIllusPct(state.settings.theme) : Math.round(state.settings.illus * 100);
  $("set-illus").value = illusPct; $("set-illus-val").textContent = illusPct + "%";
  $("set-font").value = state.settings.font;
  $("set-fontsize").value = state.settings.fontSize;
  $("set-fontsize-val").textContent = `${state.settings.fontSize}px`;

  $("set-theme").addEventListener("change", () => {
    const id = $("set-theme").value;
    state.settings = { ...state.settings, theme: id, illus: null };
    saveSettings(state.settings); applySettings(state.settings);
    const pct = themeIllusPct(id); $("set-illus").value = pct; $("set-illus-val").textContent = pct + "%";
  });
  $("set-bg").addEventListener("change", () => updateSetting("bgMode", $("set-bg").value));
  $("set-illus").addEventListener("input", () => {
    const v = Number($("set-illus").value);
    $("set-illus-val").textContent = v + "%";
    updateSetting("illus", v / 100);
  });
  $("set-font").addEventListener("change", () => updateSetting("font", $("set-font").value));
  $("set-fontsize").addEventListener("input", () => {
    const v = Number($("set-fontsize").value);
    $("set-fontsize-val").textContent = `${v}px`;
    updateSetting("fontSize", v);
  });
  $("set-ruler").checked = !!state.settings.ruler;
  $("set-ruler").addEventListener("change", () => updateSetting("ruler", $("set-ruler").checked));
  updateSidebarSideButtons();
  document.querySelectorAll("button[data-sidebar-side]").forEach((btn) => {
    btn.addEventListener("click", () => updateSetting("sidebarSide", btn.dataset.sidebarSide));
  });
  $("theme-json-edit").addEventListener("click", openThemeJsonDoc);
  $("keymap-open").addEventListener("click", showKeymap);
  $("keymap-close").addEventListener("click", hideKeymap);
  $("keymap-done").addEventListener("click", hideKeymap);
  $("keymap-reset").addEventListener("click", resetKeymap);
  $("keymap-json-edit").addEventListener("click", openKeymapJsonDoc);
  $("keymap-modal").addEventListener("click", (e) => {
    if (e.target === $("keymap-modal")) hideKeymap();
  });

  $("settings-close").addEventListener("click", hideSettings);
  $("settings").addEventListener("click", (e) => {
    if (e.target === $("settings")) hideSettings();
  });
}

// ---- boot ------------------------------------------------------------------

// Native window: open files dropped onto the window (real paths, no copy).
window.__ayameOpenNativePaths = (paths) => {
  if (!Array.isArray(paths)) return;
  (async () => {
    for (const p of paths) {
      if (typeof p !== "string" || !p) continue;
      try {
        await openPath(p);
      } catch (e) {
        flashCount(`開けません: ${p}`, "error");
        console.error(e);
      }
    }
  })();
};

async function boot() {
  state.history = loadSearchHistory();
  initSettings();
  initScrollbar();
  initEvents();
  initEditor();
  initSelection();
  initWorkspace();
  initTree();
  initContextMenu();
  try {
    await refreshStat();
  } catch (e) {
    $("overlay").classList.remove("hidden");
    $("overlay").textContent = "サーバに接続できません: " + e.message;
    postNativeMessage("ayame:ready"); // still show the window so the error is visible
    return;
  }
  updateStatusMeta();
  // Native launch with a FILE argument: the window appears immediately and the
  // (possibly long) first-index happens behind this progress overlay.
  const pending = typeof window.__ayamePendingOpen === "string" ? window.__ayamePendingOpen : "";
  if (!state.stat.open && pending) {
    showLoading(`開いています: ${displayName(pending)} …`);
    postNativeMessage("ayame:ready");
    try {
      onDocumentOpened(await apiPost("/api/open", { path: pending }));
    } catch (e) {
      flashCount(`開けません: ${pending}`, "error");
      console.error(e);
      await newUntitled();
    } finally {
      hideLoading();
    }
    return;
  }
  if (!state.stat.open) {
    await newUntitled(); // open to a blank untitled page, not the file dialog
  } else {
    focusEditor();
    render();
    refreshTabs();
  }
  postNativeMessage("ayame:ready");
}

boot();
