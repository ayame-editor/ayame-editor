// Ayame editor front-end.
//
// Design rule: the browser never holds more than the visible window. Lines are
// fetched on demand from the local server; vertical position is tracked as a
// *line number* (not pixels), so navigation is exact for any file size — ten
// lines or Ayame's minimum ten-billion-line scale. A custom scrollbar maps line
// position to a thumb, side-stepping the browser's ~33M-pixel element-height
// ceiling entirely.

const $ = (id) => document.getElementById(id);
let LINE_HEIGHT = 18; // tracks --line-height; updated by Settings (font size)
const OVERSCAN = 6;
const PAD = 400; // extra lines fetched around the viewport and cached
const SEARCH_HISTORY_KEY = "ayame.searchHistory.v1";
const SETTINGS_KEY = "ayame.settings.v1";

const FONT_STACKS = {
  mono: '"SFMono-Regular","Menlo","Consolas","DejaVu Sans Mono",monospace',
  "mono-jp": '"Consolas","Menlo","Noto Sans Mono CJK JP","MS Gothic",monospace',
  system: '"Segoe UI","Hiragino Kaku Gothic ProN","Noto Sans JP",system-ui,sans-serif',
};
const DEFAULT_SETTINGS = { theme: "light", font: "mono", fontSize: 13 };

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
  history: [],
  historyIndex: -1,
  editingLine: -1,
  editCaret: null, // caret column to place when the next line editor opens
  settings: { ...DEFAULT_SETTINGS },
  tabs: [], // open tabs from /api/tabs
};

const pool = [];
let renderQueued = false;

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

function rowsVisible() {
  return Math.max(1, Math.ceil($("viewport").clientHeight / LINE_HEIGHT));
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
    // Single click puts a caret on the line and starts editing it, like a
    // normal text editor. The gutter and the [EOF] marker are not editable.
    row.addEventListener("mousedown", (e) => {
      if (row.classList.contains("eof")) return;
      if (e.target.closest(".line-edit")) return; // clicking inside the editor
      const line = Number(row.dataset.line);
      if (!Number.isFinite(line) || line < 0 || line >= state.total) return;
      beginEdit(line, caretFromEvent(row, e));
    });
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
  } else if (state.editingLine === line) {
    renderLineEditor(tx, line, rec.text);
  } else if (state.matcher) {
    appendHighlighted(tx, rec.text);
  } else {
    tx.textContent = rec.text;
  }
  row.classList.toggle("active", line === state.activeLine);
}

function fillEofRow(row) {
  row.className = "row eof";
  row.dataset.line = "-1";
  row.firstChild.textContent = "";
  const tx = row.lastChild;
  tx.className = "tx";
  tx.textContent = "[EOF]";
}

function renderLineEditor(container, line, text) {
  const input = document.createElement("input");
  input.className = "line-edit";
  input.value = text;
  input.spellcheck = false;
  const stop = (e) => e.stopPropagation();
  input.addEventListener("mousedown", stop);
  input.addEventListener("click", stop);
  input.addEventListener("dblclick", stop);
  input.addEventListener("keydown", (e) => {
    const val = input.value;
    const pos = input.selectionStart;
    const sel = input.selectionEnd - input.selectionStart;
    if (e.key === "Enter") {
      e.preventDefault();
      splitLine(line, val, pos); // Enter splits the line at the caret
    } else if (e.key === "Escape") {
      e.preventDefault();
      state.editingLine = -1;
      scheduleRender();
      $("viewport").focus();
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      moveEdit(line, val, pos, -1);
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      moveEdit(line, val, pos, +1);
    } else if (e.key === "Backspace" && pos === 0 && sel === 0 && line > 0) {
      e.preventDefault();
      joinWithPrev(line, val); // Backspace at col 0 joins the previous line
    } else if (e.key === "Delete" && pos === val.length && sel === 0 && line < state.total - 1) {
      e.preventDefault();
      joinWithNext(line, val); // Delete at end pulls the next line up
    }
  });
  input.addEventListener("blur", () => {
    if (state.editingLine === line) commitEdit(line, input.value);
  });
  container.append(input);
  const caret = state.editCaret;
  state.editCaret = null;
  queueMicrotask(() => {
    input.focus();
    if (caret == null) {
      input.select();
    } else {
      const p = Math.max(0, Math.min(caret, input.value.length));
      input.setSelectionRange(p, p);
    }
  });
}

// Character width of the monospace content font, for click→caret mapping.
let _charW = 0;
function charWidth() {
  if (_charW) return _charW;
  const cs = getComputedStyle($("content"));
  const probe = document.createElement("span");
  probe.style.cssText = "position:absolute;visibility:hidden;white-space:pre;";
  probe.style.fontFamily = cs.fontFamily;
  probe.style.fontSize = cs.fontSize;
  probe.textContent = "0".repeat(100);
  document.body.append(probe);
  _charW = probe.getBoundingClientRect().width / 100 || 8;
  probe.remove();
  return _charW;
}

function caretFromEvent(row, e) {
  const tx = row.lastChild;
  const rect = tx.getBoundingClientRect();
  return Math.max(0, Math.round((e.clientX - rect.left) / charWidth()));
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
  if (!s) return;
  if (!s.open) {
    $("filename").textContent = "ファイル未選択";
    $("filename").title = "";
    for (const id of ["st-lines", "st-size", "st-enc", "st-eol", "st-edit", "st-index"]) {
      $(id).textContent = "—";
    }
    $("st-pos").textContent = "行 0";
    $("undo-edit").disabled = true;
    $("redo-edit").disabled = true;
    return;
  }
  $("filename").textContent = `${s.dirty ? "* " : ""}${displayName(s.path)}`;
  $("filename").title = isUntitled(s.path) ? "untitled" : s.path;
  const lines = s.view_lines ?? s.lines;
  $("st-lines").textContent = `${commas(lines)} 行`;
  $("st-size").textContent = humanBytes(s.bytes);
  $("st-enc").textContent = s.bom_bytes > 0 ? `${enc(s.encoding)} (BOM)` : enc(s.encoding);
  $("st-eol").textContent = eol(s.eol);
  $("st-edit").textContent = s.dirty
    ? `編集 +${commas(s.inserted_lines)} ~${commas(s.replaced_lines)} -${commas(s.deleted_lines)}`
    : "未編集";
  $("undo-edit").disabled = !s.can_undo;
  $("redo-edit").disabled = !s.can_redo;
  $("st-index").textContent =
    `索引 ${commas(s.checkpoints)} 点 / ${humanBytes(s.index_bytes)} / ${s.index_ms} ms`;
  // Keep the active tab's unsaved-dot in sync as you type (no full refetch).
  const at = $("tabs").querySelector(".tab.active");
  if (at) at.classList.toggle("dirty", !!s.dirty);
}

function isUntitled(path) {
  return !!path && path.includes("ayame-untitled-");
}

// Show a short, friendly name in the toolbar (basename, or "untitled").
function displayName(path) {
  if (!path) return "—";
  if (isUntitled(path)) return "untitled";
  const parts = path.replace(/\\/g, "/").split("/");
  return parts[parts.length - 1] || path;
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
  const cur = state.activeLine >= 0 ? state.activeLine : state.first;
  $("st-pos").textContent = `行 ${commas(cur + 1)}`;
}

// ---- search ----------------------------------------------------------------

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
    state.matcher = new RegExp(src, flags);
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
    flashCount("正規表現エラー");
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
    state.activeLine = h.line;
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
  } catch (e) {
    $("find-count").textContent = "正規表現エラー";
    $("find").parentElement.classList.add("error");
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

function flashCount(msg) {
  const el = $("find-count");
  el.textContent = msg;
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

function beginEdit(line, caret = null) {
  if (line < 0 || line >= state.total) return;
  state.activeLine = line;
  state.editingLine = line;
  state.editCaret = caret; // null → select whole line
  scheduleRender();
}

async function commitEdit(line, text) {
  state.editingLine = -1;
  try {
    if (await commitIfChanged(line, text)) {
      clearLineCache();
      state.activeLine = line;
      await refreshStat();
    }
    render();
  } catch (e) {
    flashCount("編集エラー");
    console.error(e);
  }
  $("viewport").focus();
}

// Only persist a line when its text actually changed, so merely moving the
// caret between lines never dirties the document.
async function commitIfChanged(line, text) {
  const rec = cachedLine(line);
  if (rec && rec.text === text) return false;
  await apiPost("/api/edit/line", { line, text });
  return true;
}

// Move the caret to the adjacent line (ArrowUp/Down), committing the current
// line first and keeping the column.
async function moveEdit(line, text, col, dir) {
  const target = line + dir;
  if (target < 0 || target >= state.total) return;
  state.editingLine = -1;
  try {
    if (await commitIfChanged(line, text)) {
      clearLineCache();
      await refreshStat();
    }
    state.activeLine = target;
    state.editingLine = target;
    state.editCaret = col;
    revealLine(target);
    render();
  } catch (e) {
    flashCount("編集エラー");
    console.error(e);
  }
}

// Enter: split the line at the caret into two lines.
async function splitLine(line, text, pos) {
  state.editingLine = -1;
  try {
    await apiPost("/api/edit/line", { line, text: text.slice(0, pos) });
    await apiPost("/api/edit/insert", { line: line + 1, text: text.slice(pos) });
    clearLineCache();
    await refreshStat();
    state.activeLine = line + 1;
    state.editingLine = line + 1;
    state.editCaret = 0;
    revealLine(line + 1);
    render();
  } catch (e) {
    flashCount("編集エラー");
    console.error(e);
  }
}

// Backspace at column 0: merge this line onto the end of the previous one.
async function joinWithPrev(line, text) {
  const prevText = cachedLine(line - 1)?.text ?? "";
  state.editingLine = -1;
  try {
    await apiPost("/api/edit/line", { line: line - 1, text: prevText + text });
    await apiPost("/api/edit/delete", { line });
    clearLineCache();
    await refreshStat();
    state.activeLine = line - 1;
    state.editingLine = line - 1;
    state.editCaret = prevText.length;
    revealLine(line - 1);
    render();
  } catch (e) {
    flashCount("編集エラー");
    console.error(e);
  }
}

// Delete at end of line: pull the next line up onto this one.
async function joinWithNext(line, text) {
  const nextText = cachedLine(line + 1)?.text ?? "";
  state.editingLine = -1;
  try {
    await apiPost("/api/edit/line", { line, text: text + nextText });
    await apiPost("/api/edit/delete", { line: line + 1 });
    clearLineCache();
    await refreshStat();
    state.activeLine = line;
    state.editingLine = line;
    state.editCaret = text.length;
    revealLine(line);
    render();
  } catch (e) {
    flashCount("編集エラー");
    console.error(e);
  }
}

async function insertLineBelow() {
  const base = state.activeLine >= 0 ? state.activeLine : state.first;
  const line = Math.min(state.total, base + 1);
  try {
    await apiPost("/api/edit/insert", { line, text: "" });
    clearLineCache();
    await refreshStat();
    state.activeLine = line;
    state.editingLine = line;
    revealLine(line);
  } catch (e) {
    flashCount("挿入エラー");
    console.error(e);
  }
}

async function deleteActiveLine() {
  const line = state.activeLine >= 0 ? state.activeLine : state.first;
  if (line < 0 || line >= state.total) return;
  try {
    await apiPost("/api/edit/delete", { line });
    clearLineCache();
    await refreshStat();
    if (state.total === 0) {
      state.activeLine = -1;
      state.editingLine = -1;
      render();
    } else {
      state.activeLine = Math.min(line, state.total - 1);
      revealLine(state.activeLine);
    }
  } catch (e) {
    flashCount("削除エラー");
    console.error(e);
  }
}

async function saveCopy() {
  const suggested = `${state.stat?.path || "ayame"}.edited`;
  const path = prompt("保存先パス", suggested);
  if (path == null) return;
  try {
    const res = await apiPost("/api/edit/save", { path });
    flashCount(`保存: ${res.path}`);
  } catch (e) {
    flashCount("保存エラー");
    alert(e.message);
  }
}

async function revertEdits() {
  if (!state.stat?.dirty) return;
  if (!confirm("未保存の編集を破棄しますか?")) return;
  await apiPost("/api/edit/revert", {});
  clearLineCache();
  state.editingLine = -1;
  await refreshStat();
  render();
}

async function undoEdit() {
  try {
    await apiPost("/api/edit/undo", {});
    clearLineCache();
    await refreshStat();
    render();
  } catch (e) {
    flashCount("Undo エラー");
    console.error(e);
  }
}

async function redoEdit() {
  try {
    await apiPost("/api/edit/redo", {});
    clearLineCache();
    await refreshStat();
    render();
  } catch (e) {
    flashCount("Redo エラー");
    console.error(e);
  }
}

async function sortSave() {
  const base = state.stat?.path || "ayame";
  const path = prompt("ソート保存先パス", `${base}.sorted`);
  if (path == null) return;
  const keyText = prompt("キー列 (空なら行全体)", "");
  if (keyText == null) return;
  const key = keyText.trim() === "" ? null : Number(keyText.trim());
  if (keyText.trim() !== "" && (!Number.isInteger(key) || key < 1)) {
    flashCount("キー列エラー");
    return;
  }
  const numeric = confirm("数値ソートにしますか?");
  const reverse = confirm("降順にしますか?");
  try {
    const res = await apiPost("/api/sort/save", { path, key, numeric, reverse });
    flashCount(`ソート保存: ${res.path}`);
  } catch (e) {
    flashCount("ソートエラー");
    alert(e.message);
  }
}

async function replaceSave() {
  const base = state.stat?.path || "ayame";
  const find = prompt("置換前", $("find").value || state.query || "");
  if (find == null || find === "") return;
  const replacement = prompt("置換後", "");
  if (replacement == null) return;
  const path = prompt("置換保存先パス", `${base}.replaced`);
  if (path == null) return;
  try {
    const res = await apiPost("/api/replace/save", {
      path,
      find,
      replacement,
      regex: state.regex,
      ci: state.ci,
    });
    flashCount(`置換保存: ${res.path}`);
  } catch (e) {
    flashCount("置換エラー");
    alert(e.message);
  }
}

async function caseSave(mode) {
  const base = state.stat?.path || "ayame";
  const suffix = mode === "upper" ? "upper" : "lower";
  const path = prompt(`${mode === "upper" ? "大文字" : "小文字"}保存先パス`, `${base}.${suffix}`);
  if (path == null) return;
  try {
    const res = await apiPost("/api/case/save", { path, mode });
    flashCount(`保存: ${res.path}`);
  } catch (e) {
    flashCount("変換エラー");
    alert(e.message);
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

function gotoFromInput() {
  const v = parseInt($("goto").value.replace(/[^0-9]/g, ""), 10);
  if (!Number.isFinite(v) || v < 1) return;
  const line = Math.min(v - 1, Math.max(0, state.total - 1));
  state.activeLine = line;
  revealLine(line);
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

  $("goto").addEventListener("keydown", (e) => {
    if (e.key === "Enter") gotoFromInput();
    if (e.key === "Escape") $("viewport").focus();
  });

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
      $("viewport").focus();
    }
  });

  $("find-next").addEventListener("click", () => findStep("next"));
  $("find-prev").addEventListener("click", () => findStep("prev"));
  $("opt-case").addEventListener("click", () => toggleOpt("ci", "opt-case"));
  $("opt-word").addEventListener("click", () => toggleOpt("word", "opt-word"));
  $("opt-regex").addEventListener("click", () => toggleOpt("regex", "opt-regex"));
  $("save-copy").addEventListener("click", saveCopy);
  $("undo-edit").addEventListener("click", undoEdit);
  $("redo-edit").addEventListener("click", redoEdit);
  $("insert-line").addEventListener("click", insertLineBelow);
  $("delete-line").addEventListener("click", deleteActiveLine);
  $("revert-edit").addEventListener("click", revertEdits);
  $("sort-save").addEventListener("click", sortSave);
  $("replace-save").addEventListener("click", replaceSave);
  $("upper-save").addEventListener("click", () => caseSave("upper"));
  $("lower-save").addEventListener("click", () => caseSave("lower"));

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

function onGlobalKey(e) {
  const inInput = e.target.tagName === "INPUT";
  const inLineEditor = inInput && e.target.classList.contains("line-edit");
  // Modals own Escape while they're up.
  if (e.key === "Escape" && settingsVisible()) {
    e.preventDefault();
    hideSettings();
    return;
  }
  if (e.key === "Escape" && openerVisible()) {
    e.preventDefault();
    hideOpener();
    return;
  }
  // Shortcuts that work even from inputs:
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "o") {
    e.preventDefault();
    showOpener();
    return;
  }
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "n") {
    e.preventDefault();
    newUntitled(); // new tab
    return;
  }
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "w") {
    e.preventDefault();
    const active = state.tabs.find((t) => t.active);
    if (active) closeTab(active.id);
    return;
  }
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "g") {
    e.preventDefault();
    const g = $("goto");
    g.focus();
    g.select();
    return;
  }
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "f") {
    e.preventDefault();
    const f = $("find");
    f.focus();
    f.select();
    return;
  }
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
    e.preventDefault();
    saveCopy();
    return;
  }
  if (!inLineEditor && (e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "z") {
    e.preventDefault();
    if (e.shiftKey) redoEdit();
    else undoEdit();
    return;
  }
  if (!inLineEditor && (e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "y") {
    e.preventDefault();
    redoEdit();
    return;
  }
  if (e.key === "F3") {
    e.preventDefault();
    findStep(e.shiftKey ? "prev" : "next");
    return;
  }
  if (e.altKey && e.key.toLowerCase() === "c") {
    toggleOpt("ci", "opt-case");
    return;
  }
  if (e.altKey && e.key.toLowerCase() === "r") {
    toggleOpt("regex", "opt-regex");
    return;
  }
  if (e.altKey && e.key.toLowerCase() === "w") {
    toggleOpt("word", "opt-word");
    return;
  }
  if (inInput) return; // leave normal text editing alone

  const vis = rowsVisible();
  switch (e.key) {
    case "ArrowDown": setFirst(state.first + 1); e.preventDefault(); break;
    case "ArrowUp": setFirst(state.first - 1); e.preventDefault(); break;
    case "PageDown": setFirst(state.first + vis); e.preventDefault(); break;
    case "PageUp": setFirst(state.first - vis); e.preventDefault(); break;
    case " ": setFirst(state.first + (e.shiftKey ? -vis : vis)); e.preventDefault(); break;
    case "Home":
      if (e.ctrlKey || e.metaKey) { setFirst(0); e.preventDefault(); }
      break;
    case "End":
      if (e.ctrlKey || e.metaKey) { setFirst(maxFirst()); e.preventDefault(); }
      break;
    case "F2":
    case "Enter":
      beginEdit(state.activeLine >= 0 ? state.activeLine : state.first);
      e.preventDefault();
      break;
    case "Insert":
      insertLineBelow();
      e.preventDefault();
      break;
    case "Delete":
      deleteActiveLine();
      e.preventDefault();
      break;
    case "Escape":
      state.activeLine = -1;
      state.editingLine = -1;
      scheduleRender();
      break;
  }
}

// ---- workspace: open / browse / drag&drop ----------------------------------

function openerVisible() {
  return !$("opener").classList.contains("hidden");
}

function showOpener() {
  const m = $("opener");
  m.classList.remove("hidden");
  m.setAttribute("aria-hidden", "false");
  browse(null);
  const inp = $("opener-input");
  inp.value = "";
  queueMicrotask(() => inp.focus());
}

function hideOpener() {
  // The opener doubles as the welcome screen: don't let it close while there is
  // no document to fall back to.
  if (!state.stat?.open) return;
  const m = $("opener");
  m.classList.add("hidden");
  m.setAttribute("aria-hidden", "true");
  $("viewport").focus();
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
  $("opener-cwd").textContent = res.dir;
  $("opener-cwd").title = res.dir;
  const list = $("opener-list");
  list.textContent = "";
  if (res.parent) {
    list.append(browseRow({ name: "..", path: res.parent, is_dir: true }, true));
  }
  for (const ent of res.entries) list.append(browseRow(ent, false));
  list.scrollTop = 0;
}

function browseRow(ent, isUp) {
  const row = document.createElement("button");
  row.className = "opener-row" + (ent.is_dir ? " dir" : "");
  const ic = document.createElement("span");
  ic.className = "ic";
  ic.textContent = ent.is_dir ? (isUp ? "↰" : "▸") : "·";
  const nm = document.createElement("span");
  nm.className = "nm";
  nm.textContent = isUp ? ".. (親フォルダ)" : ent.name;
  const sz = document.createElement("span");
  sz.className = "sz";
  sz.textContent = ent.is_dir ? "" : humanBytes(ent.size);
  row.append(ic, nm, sz);
  row.addEventListener("click", () => {
    if (ent.is_dir) browse(ent.path);
    else openPath(ent.path);
  });
  return row;
}

function confirmDiscardIfDirty() {
  if (!state.stat?.dirty) return true;
  return confirm("未保存の編集があります。別のファイルを開くと破棄されます。開きますか?");
}

async function openPath(path) {
  const p = (path || "").trim();
  if (!p) return;
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
  if (!confirmDiscardIfDirty()) return;
  openerMsg(`読み込み中… (${file.name})`, true);
  flashCount(`読み込み中: ${file.name}`);
  try {
    const r = await fetch(`/api/upload?name=${encodeURIComponent(file.name)}`, {
      method: "POST",
      body: file,
    });
    if (!r.ok) throw new Error((await r.text()) || r.statusText);
    onDocumentOpened(await r.json());
  } catch (e) {
    reportOpenError("読み込みエラー: " + e.message);
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
  state.stat = stat;
  state.total = stat.view_lines ?? stat.lines ?? 0;
  // Fresh document: reset navigation, search, and edit view state.
  state.first = 0;
  state.activeLine = -1;
  state.editingLine = -1;
  state.lastMatch = null;
  state.searchHits = null;
  state.searchTruncated = false;
  $("find-count").textContent = "";
  clearLineCache();
  const m = $("opener");
  m.classList.add("hidden");
  m.setAttribute("aria-hidden", "true");
  updateStatusMeta();
  render();
  refreshTabs();
  $("viewport").focus();
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
  c.textContent = "";
  for (const t of list) {
    const el = document.createElement("div");
    el.className = "tab" + (t.active ? " active" : "") + (t.dirty ? " dirty" : "");
    el.dataset.id = String(t.id);
    el.title = t.path;
    const dot = document.createElement("span");
    dot.className = "tab-dot";
    const nm = document.createElement("span");
    nm.className = "tab-name";
    nm.textContent = t.name;
    const x = document.createElement("span");
    x.className = "tab-x";
    x.textContent = "✕";
    x.title = "閉じる";
    el.append(dot, nm, x);
    el.addEventListener("click", () => {
      if (!t.active) selectTab(t.id);
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
    onDocumentOpened(await apiPost("/api/tabs/select", { id }));
  } catch (e) {
    flashCount("タブ切替エラー");
    console.error(e);
  }
}

async function closeTab(id) {
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

// Start a fresh empty "untitled" buffer with a blank editable first line, so
// the app opens to a usable page (like Notepad) instead of a dialog.
async function newUntitled() {
  try {
    onDocumentOpened(await apiPost("/api/new", {}));
    // The buffer already has one empty line; drop the caret in, Notepad-style.
    if (state.total >= 1) beginEdit(0, 0);
  } catch (e) {
    showOpener();
    openerMsg("新規バッファを作成できません: " + e.message);
  }
}

function initWorkspace() {
  $("open-file").addEventListener("click", showOpener);
  $("opener-close").addEventListener("click", hideOpener);
  $("opener-open").addEventListener("click", () => openPath($("opener-input").value));
  $("opener-input").addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      openPath($("opener-input").value);
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
    return { ...DEFAULT_SETTINGS, ...(raw && typeof raw === "object" ? raw : {}) };
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

function applySettings(s) {
  const root = document.documentElement;
  root.dataset.theme = s.theme; // light | dark | black
  root.style.setProperty("--mono", FONT_STACKS[s.font] || FONT_STACKS.mono);
  const fs = Math.max(11, Math.min(22, Number(s.fontSize) || 13));
  root.style.setProperty("--font-size", `${fs}px`);
  const lh = fs + 6;
  root.style.setProperty("--line-height", `${lh}px`);
  LINE_HEIGHT = lh; // keep virtualization math in sync with the CSS
  _charW = 0; // font metrics changed → remeasure on next click
  scheduleRender();
}

function updateSetting(key, value) {
  state.settings = { ...state.settings, [key]: value };
  applySettings(state.settings);
  saveSettings(state.settings);
}

function settingsVisible() {
  return !$("settings").classList.contains("hidden");
}
function showSettings() {
  const m = $("settings");
  m.classList.remove("hidden");
  m.setAttribute("aria-hidden", "false");
}
function hideSettings() {
  const m = $("settings");
  m.classList.add("hidden");
  m.setAttribute("aria-hidden", "true");
  $("viewport").focus();
}

function initSettings() {
  state.settings = loadSettings();
  applySettings(state.settings);
  $("set-theme").value = state.settings.theme;
  $("set-font").value = state.settings.font;
  $("set-fontsize").value = state.settings.fontSize;
  $("set-fontsize-val").textContent = `${state.settings.fontSize}px`;

  $("set-theme").addEventListener("change", () => updateSetting("theme", $("set-theme").value));
  $("set-font").addEventListener("change", () => updateSetting("font", $("set-font").value));
  $("set-fontsize").addEventListener("input", () => {
    const v = Number($("set-fontsize").value);
    $("set-fontsize-val").textContent = `${v}px`;
    updateSetting("fontSize", v);
  });

  $("open-settings").addEventListener("click", showSettings);
  $("settings-close").addEventListener("click", hideSettings);
  $("settings").addEventListener("click", (e) => {
    if (e.target === $("settings")) hideSettings();
  });
}

// ---- boot ------------------------------------------------------------------

async function boot() {
  state.history = loadSearchHistory();
  initSettings();
  initScrollbar();
  initEvents();
  initWorkspace();
  try {
    await refreshStat();
  } catch (e) {
    $("overlay").classList.remove("hidden");
    $("overlay").textContent = "サーバに接続できません: " + e.message;
    return;
  }
  updateStatusMeta();
  if (!state.stat.open) {
    await newUntitled(); // open to a blank untitled page, not the file dialog
  } else {
    $("viewport").focus();
    render();
    refreshTabs();
  }
}

boot();
