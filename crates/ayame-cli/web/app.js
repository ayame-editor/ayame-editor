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
const TREE_KEY = "ayame.treeRoot.v1";
const MAX_COPY_LINES = 20000; // cap for copy/cut/delete of a selection

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
  ruler: true,
  bgMode: "watercolor",
  illus: null,
  customThemes: {},
};

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
  treeParent: null, // parent of the current tree root (for the "up" button)
  treeLoaded: false,
  openerDir: null, // directory currently shown in the open dialog
  sel: null, // multi-line selection: { anchor: {line,col}, head: {line,col} }
  dragging: false,
  dragMoved: false,
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
      jumpEdit(line, val, line - 1, pos); // same column, line above
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      jumpEdit(line, val, line + 1, pos); // same column, line below
    } else if (e.key === "ArrowLeft" && pos === 0 && sel === 0 && line > 0) {
      e.preventDefault();
      jumpEdit(line, val, line - 1, (cachedLine(line - 1)?.text ?? "").length);
    } else if (e.key === "ArrowRight" && pos === val.length && sel === 0 && line < state.total - 1) {
      e.preventDefault();
      jumpEdit(line, val, line + 1, 0);
    } else if (e.key === "Backspace" && pos === 0 && sel === 0 && line > 0) {
      e.preventDefault();
      joinWithPrev(line, val); // Backspace at col 0 joins the previous line
    } else if (e.key === "Delete" && pos === val.length && sel === 0 && line < state.total - 1) {
      e.preventDefault();
      joinWithNext(line, val); // Delete at end pulls the next line up
    }
  });
  // Pasting text that contains newlines becomes multiple lines (like Notepad),
  // instead of collapsing into this single-line input.
  input.addEventListener("paste", (e) => {
    const text = (e.clipboardData || window.clipboardData)?.getData("text") ?? "";
    if (!/[\r\n]/.test(text)) return; // single line → let the input handle it
    e.preventDefault();
    pasteMultiline(line, input.value, input.selectionStart, input.selectionEnd, text);
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

// ---- selection (multi-line, coordinate-based) ------------------------------

// Width in px of the line-number gutter, measured from a visible row.
function gutterPixels() {
  for (const row of pool) {
    if (row.style.display !== "none" && row.firstChild) {
      return row.firstChild.getBoundingClientRect().width;
    }
  }
  return 7 * charWidth() + 20;
}

// Map a mouse event to a {line, col} position in the document.
function coordsFromEvent(e) {
  const content = $("content");
  const rect = content.getBoundingClientRect();
  const rowInView = Math.floor((e.clientY - rect.top) / LINE_HEIGHT);
  let line = state.first + Math.max(0, rowInView);
  line = Math.max(0, Math.min(line, Math.max(0, state.total - 1)));
  const x = e.clientX - rect.left - gutterPixels() + content.scrollLeft;
  let col = Math.max(0, Math.round(x / charWidth()));
  const len = cachedLine(line)?.text.length ?? 0;
  return { line, col: Math.min(col, len) };
}

// Normalized selection: { start, end } with start <= end, or null.
function selRange() {
  if (!state.sel) return null;
  const { anchor: a, head: h } = state.sel;
  const forward = a.line < h.line || (a.line === h.line && a.col <= h.col);
  return forward ? { start: a, end: h } : { start: h, end: a };
}

function hasSelection() {
  const r = selRange();
  return !!r && !(r.start.line === r.end.line && r.start.col === r.end.col);
}

function clearSelection() {
  if (state.sel) {
    state.sel = null;
    scheduleRender();
  }
}

function renderSelection() {
  const layer = $("sel-layer");
  layer.textContent = "";
  const r = selRange();
  if (!r || (r.start.line === r.end.line && r.start.col === r.end.col)) return;
  const gpx = gutterPixels();
  const cw = charWidth();
  const vis = rowsVisible() + OVERSCAN;
  const from = Math.max(r.start.line, state.first);
  const to = Math.min(r.end.line, state.first + vis);
  for (let line = from; line <= to; line++) {
    const startCol = line === r.start.line ? r.start.col : 0;
    const len = cachedLine(line)?.text.length ?? 0;
    // End of a fully/partly selected line extends a bit past its text so the
    // trailing newline reads as selected, like a normal editor.
    let endCol = line === r.end.line ? Math.min(r.end.col, len) : len + 1;
    if (endCol < startCol) endCol = startCol;
    const rect = document.createElement("div");
    rect.className = "selrect";
    rect.style.left = `${gpx + startCol * cw}px`;
    rect.style.top = `${(line - state.first) * LINE_HEIGHT}px`;
    rect.style.width = `${Math.max(2, (endCol - startCol) * cw)}px`;
    layer.append(rect);
  }
}

function initSelection() {
  const content = $("content");
  content.addEventListener("mousedown", async (e) => {
    if (e.button !== 0 || e.target.closest(".line-edit")) return;
    // Commit any in-progress line edit before selecting.
    if (state.editingLine >= 0) {
      const inp = content.querySelector(".line-edit");
      const l = state.editingLine;
      state.editingLine = -1;
      if (inp) {
        try {
          if (await commitIfChanged(l, inp.value)) {
            clearLineCache();
            await refreshStat();
          }
        } catch {
          /* ignore */
        }
      }
    }
    const p = coordsFromEvent(e);
    if (e.shiftKey && state.sel) {
      state.sel.head = p;
      state.dragMoved = true;
    } else {
      state.sel = { anchor: p, head: p };
      state.dragMoved = false;
    }
    state.dragging = true;
    $("viewport").focus();
    scheduleRender();
  });

  window.addEventListener("mousemove", (e) => {
    if (!state.dragging) return;
    const p = coordsFromEvent(e);
    const a = state.sel.anchor;
    if (p.line !== a.line || p.col !== a.col) state.dragMoved = true;
    state.sel.head = p;
    // Auto-scroll when dragging past the top/bottom edge.
    const rect = $("content").getBoundingClientRect();
    if (e.clientY < rect.top + 14) setFirst(state.first - 2);
    else if (e.clientY > rect.bottom - 14) setFirst(state.first + 2);
    scheduleRender();
  });

  window.addEventListener("mouseup", () => {
    if (!state.dragging) return;
    state.dragging = false;
    if (!state.dragMoved) {
      // A plain click: place the caret and start editing that line.
      const p = state.sel.head;
      state.sel = null;
      beginEdit(p.line, p.col);
    } else {
      scheduleRender();
    }
  });
}

function selectAll() {
  if (state.total === 0) return;
  const last = state.total - 1;
  state.sel = {
    anchor: { line: 0, col: 0 },
    head: { line: last, col: cachedLine(last)?.text.length ?? Number.MAX_SAFE_INTEGER },
  };
  state.editingLine = -1;
  $("viewport").focus();
  scheduleRender();
}

// Fetch the selected text (bounded) and join with newlines.
async function selectedText(r) {
  const count = Math.min(r.end.line - r.start.line + 1, MAX_COPY_LINES);
  const res = await api(`/api/lines?start=${r.start.line}&count=${count}`);
  const L = res.lines.map((x) => x.text);
  if (L.length === 1) return (L[0] ?? "").slice(r.start.col, r.end.col);
  const out = [(L[0] ?? "").slice(r.start.col)];
  for (let i = 1; i < L.length - 1; i++) out.push(L[i]);
  out.push((L[L.length - 1] ?? "").slice(0, r.end.col));
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
    await copyToClipboard(await selectedText(r));
    flashCount("コピーしました");
  } catch (e) {
    flashCount("コピーエラー");
    console.error(e);
  }
}

async function deleteSelection() {
  const r = selRange();
  if (!r || !hasSelection()) return;
  const count = Math.min(r.end.line - r.start.line + 1, MAX_COPY_LINES);
  try {
    const res = await api(`/api/lines?start=${r.start.line}&count=${count}`);
    const L = res.lines.map((x) => x.text);
    const merged = (L[0] ?? "").slice(0, r.start.col) + (L[L.length - 1] ?? "").slice(r.end.col);
    await apiPost("/api/edit/line", { line: r.start.line, text: merged });
    for (let i = r.start.line + L.length - 1; i > r.start.line; i--) {
      await apiPost("/api/edit/delete", { line: i });
    }
    state.sel = null;
    clearLineCache();
    await refreshStat();
    beginEdit(r.start.line, r.start.col);
  } catch (e) {
    flashCount("削除エラー");
    console.error(e);
  }
}

async function cutSelection() {
  const r = selRange();
  if (!r || !hasSelection()) return;
  await copyToClipboard(await selectedText(r));
  await deleteSelection();
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
  $("apply-theme").classList.toggle("hidden", !isThemeDoc(s.path));
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
  state.sel = null; // editing a line clears any range selection
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

// Move the caret to another line (arrow keys), committing the current line
// first if it changed, and placing the caret at `targetCol`.
async function jumpEdit(line, text, targetLine, targetCol) {
  if (targetLine < 0 || targetLine >= state.total) return;
  state.editingLine = -1;
  try {
    if (await commitIfChanged(line, text)) {
      clearLineCache();
      await refreshStat();
    }
    state.activeLine = targetLine;
    state.editingLine = targetLine;
    state.editCaret = targetCol;
    revealLine(targetLine);
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

// Paste multi-line text at the caret: the first pasted line joins the text
// before the caret, the last joins the text after it, and any lines between
// become new lines — the Notepad behavior.
async function pasteMultiline(line, val, selStart, selEnd, raw) {
  const parts = raw.replace(/\r\n?/g, "\n").split("\n");
  const before = val.slice(0, selStart);
  const after = val.slice(selEnd);
  state.editingLine = -1;
  try {
    await apiPost("/api/edit/line", { line, text: before + parts[0] });
    let caretLine = line;
    let caretCol = (before + parts[0]).length;
    for (let i = 1; i < parts.length; i++) {
      const last = i === parts.length - 1;
      await apiPost("/api/edit/insert", {
        line: line + i,
        text: last ? parts[i] + after : parts[i],
      });
      caretLine = line + i;
      caretCol = parts[i].length; // caret sits just before the trailing `after`
    }
    clearLineCache();
    await refreshStat();
    state.activeLine = caretLine;
    state.editingLine = caretLine;
    state.editCaret = caretCol;
    revealLine(caretLine);
    render();
  } catch (e) {
    flashCount("貼り付けエラー");
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
  const path = await askPrompt("別名で保存", "保存先パス", suggested);
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
  const path = await askPrompt("ソートして保存", "保存先パス", `${base}.sorted`);
  if (path == null) return;
  const keyText = await askPrompt("ソート", "キー列 (空なら行全体)", "");
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
  const find = await askPrompt("置換", "置換前の文字列", $("find").value || state.query || "");
  if (find == null || find === "") return;
  const replacement = await askPrompt("置換", "置換後の文字列", "");
  if (replacement == null) return;
  const path = await askPrompt("置換して保存", "保存先パス", `${base}.replaced`);
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
  const path = await askPrompt(`${mode === "upper" ? "大文字化" : "小文字化"}して保存`, "保存先パス", `${base}.${suffix}`);
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
  $("apply-theme").addEventListener("click", applyThemeFromBuffer);
  $("undo-edit").addEventListener("click", undoEdit);
  $("redo-edit").addEventListener("click", redoEdit);
  $("sort-save").addEventListener("click", sortSave);
  $("replace-save").addEventListener("click", replaceSave);

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
    modal.classList.remove("hidden");
    modal.setAttribute("aria-hidden", "false");
    setTimeout(() => { input.focus(); input.select(); }, 0);
    const finish = (val) => {
      modal.classList.add("hidden");
      modal.setAttribute("aria-hidden", "true");
      input.removeEventListener("keydown", onKey);
      $("prompt-ok").removeEventListener("click", onOk);
      $("prompt-cancel").removeEventListener("click", onCancel);
      $("prompt-close").removeEventListener("click", onCancel);
      modal.removeEventListener("mousedown", onBackdrop);
      $("viewport").focus();
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

// ---- loading overlay ------------------------------------------------------
function showLoading(text) {
  const o = $("overlay");
  o.textContent = text || "読み込み中…";
  o.classList.remove("hidden");
}
function hideLoading() { $("overlay").classList.add("hidden"); }

// Jump the active line to a 1-based line number.
function gotoLine(n) {
  const v = parseInt(String(n).replace(/[^0-9]/g, ""), 10);
  if (!Number.isFinite(v) || v < 1) return;
  const line = Math.min(v - 1, Math.max(0, state.total - 1));
  state.activeLine = line;
  revealLine(line);
}

function onGlobalKey(e) {
  const inInput = e.target.tagName === "INPUT";
  const inLineEditor = inInput && e.target.classList.contains("line-edit");
  // A prompt modal owns all keys while it is open.
  if (promptVisible()) return;
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
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "b") {
    e.preventDefault();
    setSidebar(!sidebarOpen());
    return;
  }
  if ((e.ctrlKey || e.metaKey) && (e.key.toLowerCase() === "n" || e.key.toLowerCase() === "t")) {
    e.preventDefault();
    newUntitled(); // new tab (Ctrl+N / Ctrl+T)
    return;
  }
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "g") {
    e.preventDefault();
    askPrompt("行へ移動", "行番号").then((v) => { if (v != null) gotoLine(v); });
    return;
  }
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "w") {
    e.preventDefault();
    const active = state.tabs.find((t) => t.active);
    if (active) closeTab(active.id);
    return;
  }
  // Multi-line selection: select-all / copy / cut / delete (when not typing in
  // the single-line editor).
  if (!inLineEditor && (e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "a") {
    e.preventDefault();
    selectAll();
    return;
  }
  if (!inLineEditor && hasSelection() && (e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "c") {
    e.preventDefault();
    copySelection();
    return;
  }
  if (!inLineEditor && hasSelection() && (e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "x") {
    e.preventDefault();
    cutSelection();
    return;
  }
  if (!inInput && hasSelection() && (e.key === "Delete" || e.key === "Backspace")) {
    e.preventDefault();
    deleteSelection();
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
      state.sel = null;
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
  state.openerDir = res.dir;
  $("opener-cwd").textContent = res.dir.replace(/^\\\\\?\\/, "");
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
  row.className = "opener-row" + (ent.is_dir ? " dir" : "") + (isUp ? " up" : "");
  const ic = document.createElement("span");
  ic.className = "ic";
  ic.textContent = isUp ? "↑" : ent.is_dir ? "▸" : "·";
  const nm = document.createElement("span");
  nm.className = "nm";
  nm.textContent = isUp ? "上の階層へ" : ent.name;
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
  state.stat = stat;
  state.total = stat.view_lines ?? stat.lines ?? 0;
  // Fresh document: reset navigation, search, and edit view state.
  state.first = 0;
  state.activeLine = -1;
  state.editingLine = -1;
  state.sel = null;
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
  row.style.paddingLeft = `${8 + depth * 14}px`;
  const chev = document.createElement("span");
  chev.className = "chev";
  chev.textContent = ent.is_dir ? "▸" : "";
  const nm = document.createElement("span");
  nm.className = "tname";
  nm.textContent = ent.name;
  row.append(chev, nm);

  if (!ent.is_dir) {
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
    chev.textContent = opening ? "▾" : "▸";
  });
  const frag = document.createDocumentFragment();
  frag.append(row, kids);
  return frag;
}

function initTree() {
  $("toggle-sidebar").addEventListener("click", () => setSidebar(!sidebarOpen()));
  $("sb-up").addEventListener("click", () => {
    if (state.treeParent) treeSetRoot(state.treeParent);
  });
  $("sb-openfolder").addEventListener("click", () => {
    // Reuse the open dialog to pick a folder for the tree root.
    showOpener();
  });
  $("opener-folder").addEventListener("click", () => {
    if (!state.openerDir) return;
    if (!sidebarOpen()) setSidebar(true);
    state.treeLoaded = true;
    treeSetRoot(state.openerDir);
    hideOpener();
  });
  // Apply persisted visibility.
  if (state.settings.sidebar) setSidebar(true);
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

// Built-in themes are also defined as CSS `html[data-theme=...]` blocks in
// style.css; these JSON mirrors let the Settings JSON editor show/export them
// and act as a base for custom themes. Custom themes apply at runtime by
// setting the same CSS variables the built-ins use.
const THEME_PRESETS = {
  "iris-light": {"name":"Iris Light","type":"light","radius":10,
    "color":{"paper":"#FBF8F1","paper2":"#FDFCF8","ink":"#2A2140","inkDim":"#6E6383","inkFaint":"#A99DBC","accent":"#7A5CC0","accent2":"#6A4CB0","gold":"#C79A2E","edge":"#E7E0D3","err":"#C0506A","markBg":"#FBEBB0","markFg":"#6B5510","markCur":"#E8B84B","markCurFg":"#2A2205"},
    "acrylic":{"tint":"rgba(255,253,248,0.72)","blur":20},"background":{"mode":"watercolor","solid":"#FBF8F1"},"illustration":0.1,
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

function initSettings() {
  state.settings = loadSettings();
  applySettings(state.settings);
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
  $("theme-json-edit").addEventListener("click", openThemeJsonDoc);

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
  initSelection();
  initWorkspace();
  initTree();
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
