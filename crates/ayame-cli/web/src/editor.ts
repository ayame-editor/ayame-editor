// Ayame Editor — editor module. Type-stripped to JS at build time (build.rs, oxc).
import { $ } from "./dom.js";
import { LINE_HEIGHT, OVERSCAN, PAD, state } from "./state.js";
import { api } from "./api.js";
import { t } from "./i18n.js";
import { hasSelection, renderSelection } from "./selection.js";
import { updateStatusPos } from "./menus.js";
import { anyModalOpen } from "./input.js";

export const pool = [];

export let renderQueued = false;

export function rowsVisible() {
  const h = $("viewport").clientHeight - (state.settings && state.settings.ruler ? 18 : 0);
  return Math.max(1, Math.ceil(h / LINE_HEIGHT));
}

// ---- column ruler ----------------------------------------------------------

export let _rulerKey = "";

export function buildRuler() {
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

export function maxFirst() {
  return Math.max(0, state.total - rowsVisible());
}

// ---- data ------------------------------------------------------------------

export function cachedLine(line) {
  const c = state.cache;
  const i = line - c.start;
  return i >= 0 && i < c.lines.length ? c.lines[i] : null;
}

export function ensureData(start, count) {
  const need0 = start;
  const need1 = Math.min(state.total, start + count);
  const c = state.cache;
  if (c.lines.length && need0 >= c.start && need1 <= c.start + c.lines.length) {
    return; // already cached
  }
  const fstart = Math.max(0, start - PAD);
  const fcount = count + 2 * PAD;
  const token = ++state.loadToken;
  const url = `/api/lines?start=${fstart}&count=${fcount}`;
  api(url)
    .catch(async (firstError) => {
      if (token !== state.loadToken) throw firstError;
      return api(url);
    })
    .then((res) => {
      if (token !== state.loadToken) return; // a newer request superseded us
      state.cache = { start: fstart, lines: res.lines };
      state.total = res.total;
      render();
    })
    .catch((e) => {
      if (token !== state.loadToken) return;
      console.error("lines fetch failed", e);
      import("./search.js")
        .then((m) => m.flashCount(t("editor.reloadError"), "error"))
        .catch(() => {});
    });
}

export async function lineByte(line, col = null) {
  try {
    const q = col == null ? "" : `&col=${Math.max(0, col)}`;
    const r = await api(`/api/linebyte?line=${Math.max(0, line)}${q}`);
    return r.byte ?? 0;
  } catch {
    return 0;
  }
}

// ---- rendering -------------------------------------------------------------

export function ensurePool(count) {
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

export function fillRow(row, line, rec, gutterWidth) {
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
  } else if (state.settings.showWhitespace) {
    appendText(tx, rec.text, true);
  } else {
    tx.textContent = rec.text;
  }
  if (rec != null && state.settings.showWhitespace) appendEol(tx);
  // Hide the current-line highlight while a selection exists — the two
  // washes stack otherwise and the selection becomes hard to read.
  row.classList.toggle("active", line === state.activeLine && !hasSelection());
}

export function fillEofRow(row) {
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
export let _charW = 0;

export function charWidth() {
  if (_charW) return _charW;
  _charW = measureTextWidth("0".repeat(100)) / 100 || 8;
  return _charW;
}

// Called by Settings when the font metrics change: drop the cached char width
// and force the ruler to rebuild. Grouped here so both cached metrics
// (_charW / _rulerKey) stay owned by this module.
export function invalidateFontMetrics() {
  _charW = 0;
  _rulerKey = "";
}

// Measure the rendered pixel width of `str` in the content font. One reused,
// hidden probe kept inside #content so it inherits the exact font metrics.
export let _measSpan = null;

export function measureTextWidth(str) {
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
export function lineChars(line) {
  return Array.from(cachedLine(line)?.text ?? "");
}

export function lineLen(line) {
  return lineChars(line).length;
}

// Pixel x (in #content coordinates, gutter included) of column `col` on `line`.
export function caretX(line, col) {
  const cs = lineChars(line);
  const head = cs.slice(0, Math.max(0, Math.min(col, cs.length))).join("");
  return gutterPixels() + measureTextWidth(head);
}

// Inverse of caretX: nearest column boundary to pixel x (content coordinates).
export function colFromX(line, x) {
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
export function gutterPixels() {
  for (const row of pool) {
    if (row.style.display !== "none" && row.firstChild) {
      return row.firstChild.getBoundingClientRect().width;
    }
  }
  return 7 * charWidth() + 29; // fallback: 8 + 20 padding + 1 border
}

// Map a mouse event to a {line, col} position in the document.
export function coordsFromEvent(e) {
  const content = $("content");
  const rect = content.getBoundingClientRect();
  const rowInView = Math.floor((e.clientY - rect.top) / LINE_HEIGHT);
  let line = state.first + Math.max(0, rowInView);
  line = Math.max(0, Math.min(line, Math.max(0, state.total - 1)));
  const x = e.clientX - rect.left + content.scrollLeft; // #content coordinates
  return { line, col: colFromX(line, x) };
}

// Append plain text to a row, rendering tabs as a faint arrow glyph when
// "空白・改行を表示" is on. The real \t stays in the DOM (the arrow is an
// absolutely-positioned ::before), so glyph widths — and therefore caret and
// selection geometry, which are measured from the logical text — never shift.
// `endsLine` marks the final piece of a line so its trailing ASCII spaces get
// a middle-dot overlay (only meaningful at a real line end).
export function appendText(container, str, endsLine?) {
  // Count the run of trailing half-width spaces so a line made purely of ASCII
  // spaces still gets dots (and so the fast path below is skipped when needed).
  let trail = 0;
  if (endsLine && state.settings.showWhitespace) {
    while (trail < str.length && str.charCodeAt(str.length - 1 - trail) === 0x20) trail++;
  }
  if (!state.settings.showWhitespace || (!/[\t　]/.test(str) && trail === 0)) {
    if (str) container.appendChild(document.createTextNode(str));
    return;
  }
  const body = trail ? str.slice(0, str.length - trail) : str;
  // Split keeping tabs and full-width (zenkaku) spaces as their own pieces so
  // each can be wrapped in a width-preserving overlay span.
  for (const p of body.split(/(\t|　)/)) {
    if (p === "") continue;
    if (p === "\t") {
      const t = document.createElement("span");
      t.className = "ws-tab";
      t.textContent = "\t";
      container.appendChild(t);
    } else if (p === "　") {
      const s = document.createElement("span");
      s.className = "ws-zsp";
      s.textContent = "　";
      container.appendChild(s);
    } else {
      container.appendChild(document.createTextNode(p));
    }
  }
  // Trailing spaces before the line end: one dot overlay per space, real space
  // kept so caret columns are unchanged.
  for (let i = 0; i < trail; i++) {
    const s = document.createElement("span");
    s.className = "ws-trail";
    s.textContent = " ";
    container.appendChild(s);
  }
}

// A faint end-of-line marker (↵) drawn after the text. It sits past every
// column, so it adds no width before any caret position.
export function appendEol(container) {
  const el = document.createElement("span");
  el.className = "ws-eol";
  el.textContent = "↵";
  container.appendChild(el);
}

export function appendHighlighted(container, text) {
  const re = state.matcher;
  re.lastIndex = 0;
  let last = 0;
  let m;
  while ((m = re.exec(text)) !== null) {
    if (m.index > last) {
      appendText(container, text.slice(last, m.index));
    }
    const mk = document.createElement("mark");
    appendText(mk, m[0]);
    container.appendChild(mk);
    last = m.index + m[0].length;
    if (m[0].length === 0) re.lastIndex++; // never stall on empty matches
  }
  if (last < text.length) {
    appendText(container, text.slice(last), true);
  }
}

export function render() {
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

export function scheduleRender() {
  if (renderQueued) return;
  renderQueued = true;
  requestAnimationFrame(render);
}

export function setFirst(line) {
  state.first = Math.min(Math.max(0, Math.round(line)), maxFirst());
  scheduleRender();
}

// ---- custom scrollbar ------------------------------------------------------

export function updateScrollbar() {
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

export function renderSearchTicks(vh) {
  const ticks = $("vticks");
  if (!ticks) return;
  ticks.textContent = "";
  if (!state.query || !state.searchHits || state.searchHits.length === 0 || state.total <= 1)
    return;
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

export function initScrollbar() {
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

export function revealLine(line) {
  const vis = rowsVisible();
  if (line < state.first || line >= state.first + vis) {
    setFirst(line - Math.floor(vis / 3));
  } else {
    scheduleRender();
  }
  updateStatusPos();
}

export function clearLineCache() {
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
// `knownLen` bypasses the lineLen() clamp for a column the caller resolved
// from the server — lineLen() guesses 0 outside the cache window.
export function setCaret(line, col, knownLen = null) {
  line = Math.max(0, Math.min(line, Math.max(0, state.total - 1)));
  col = Math.max(0, Math.min(col, knownLen ?? lineLen(line)));
  state.caret = { line, col };
  state.activeLine = line;
  state.extraCursors = []; // any explicit caret placement collapses multi-cursor
  state.editGen++; // user-driven caret placement (click, search, open, …)
}

// Caret motion for the keyboard: `extend` grows the selection from its anchor.
// `knownLen` as in setCaret: a server-resolved length for an uncached line.
export function moveCaret(line, col, extend, knownLen = null) {
  line = Math.max(0, Math.min(line, Math.max(0, state.total - 1)));
  col = Math.max(0, Math.min(col, knownLen ?? lineLen(line)));
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

export function focusEditor() {
  const hi = $("hidden-input");
  if (hi && document.activeElement !== hi) hi.focus({ preventScroll: true });
  state.focused = true;
  scheduleRender();
}

// Bring the caret into view: scroll vertically (whole lines) and horizontally
// (#content is the horizontal scroll container).
export function revealCaret() {
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
export function positionCaret() {
  const caretEl = $("caret");
  const hi = $("hidden-input");
  if (!caretEl || !hi) return;
  const vis = rowsVisible();
  const focusVisible = state.focused && !anyModalOpen() && !state.composing;
  positionExtraCarets(vis, focusVisible);
  const onScreen =
    !!state.stat?.open && state.caret.line >= state.first && state.caret.line < state.first + vis;
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
export const extraCaretPool = [];

export function positionExtraCarets(vis, focusVisible) {
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
    const onScreen = !!state.stat?.open && c.line >= state.first && c.line < state.first + vis;
    el.classList.toggle("on", onScreen && focusVisible);
    if (onScreen) {
      const x = caretX(c.line, c.col);
      const y = (c.line - state.first) * LINE_HEIGHT;
      el.style.transform = `translate(${x}px, ${y}px)`;
    }
  }
}
