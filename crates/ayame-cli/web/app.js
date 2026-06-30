// Ayame viewer front-end.
//
// Design rule: the browser never holds more than the visible window. Lines are
// fetched on demand from the local server; vertical position is tracked as a
// *line number* (not pixels), so navigation is exact for any file size — ten
// lines or ten billion. A custom scrollbar maps line position to a thumb, side-
// stepping the browser's ~33M-pixel element-height ceiling entirely.

const $ = (id) => document.getElementById(id);
const LINE_HEIGHT = 18;
const OVERSCAN = 6;
const PAD = 400; // extra lines fetched around the viewport and cached
const SEARCH_HISTORY_KEY = "ayame.searchHistory.v1";

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
};

const pool = [];
let renderQueued = false;

// ---- tiny helpers -----------------------------------------------------------

async function api(path) {
  const r = await fetch(path);
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
    content.append(row);
    pool.push(row);
  }
}

function fillRow(row, line, rec, gutterWidth) {
  const ln = row.firstChild;
  const tx = row.lastChild;
  ln.textContent = String(line + 1).padStart(gutterWidth, " ");
  tx.textContent = "";
  tx.classList.remove("pending");
  if (rec == null) {
    tx.classList.add("pending");
    tx.textContent = "⋯";
  } else if (state.matcher) {
    appendHighlighted(tx, rec.text);
  } else {
    tx.textContent = rec.text;
  }
  row.classList.toggle("active", line === state.activeLine);
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
    if (r >= count || line >= state.total) {
      row.style.display = "none";
      continue;
    }
    row.style.display = "";
    fillRow(row, line, cachedLine(line), gutterWidth);
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
  $("filename").textContent = s.path;
  $("filename").title = s.path;
  $("st-lines").textContent = `${commas(s.lines)} 行`;
  $("st-size").textContent = humanBytes(s.bytes);
  $("st-enc").textContent = s.bom_bytes > 0 ? `${enc(s.encoding)} (BOM)` : enc(s.encoding);
  $("st-eol").textContent = eol(s.eol);
  $("st-index").textContent =
    `索引 ${commas(s.checkpoints)} 点 / ${humanBytes(s.index_bytes)} / ${s.index_ms} ms`;
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
  // Shortcuts that work even from inputs:
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
    case "Escape":
      state.activeLine = -1;
      scheduleRender();
      break;
  }
}

// ---- boot ------------------------------------------------------------------

async function boot() {
  state.history = loadSearchHistory();
  initScrollbar();
  initEvents();
  try {
    state.stat = await api("/api/stat");
    state.total = state.stat.lines;
  } catch (e) {
    $("overlay").classList.remove("hidden");
    $("overlay").textContent = "サーバに接続できません: " + e.message;
    return;
  }
  updateStatusMeta();
  $("viewport").focus();
  render();
}

boot();
