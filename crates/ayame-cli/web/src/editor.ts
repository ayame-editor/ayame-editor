// Ayame Editor — editor module. Type-stripped to JS at build time (build.rs, oxc).
import { $, commas } from "./dom.js";
import { LINE_HEIGHT, OVERSCAN, PAD, state, type Selection } from "./state.js";
import {
  api,
  type ChangeHistoryResponse,
  type LineByteResponse,
  type LinesResponse,
  type SearchHit,
} from "./api.js";
import { t } from "./i18n.js";
import { highlightSpans } from "./syntax.js";
import { analysisRanges } from "./analysis-model.js";
import { flashCount } from "./notifications.js";

export const pool = [];
const MAX_CHANGE_TICKS = 512;

export let renderQueued = false;

// ---- per-row render memo (#142) ---------------------------------------------
//
// `render()` runs on every caret move, selection change, scroll tick and edit,
// and used to rebuild every visible row unconditionally: `textContent = ""`
// then a fresh span tree per line, re-tokenized. Arrow-key repeat and drag
// selection meant ~60 rows torn down and rebuilt per frame for a document that
// had not changed.
//
// A row's rendered output is a function of (line, active flag) plus a handful
// of shared inputs. Almost all of those are REPLACED rather than mutated — the
// line cache, the marker sets, the settings object, the search matcher, the
// analysis matchers — so identity comparison is exact and one epoch counter
// stands in for the lot. Keys parallel to `pool` then say whether a row
// already shows exactly what it should.
let rowEpoch = 0;
let rowEpochInputs: unknown[] = [];
// Keyed by the row element rather than its pool index: a pool that was emptied
// and refilled (tab teardown, tests) then starts with no keys at all instead of
// inheriting the previous rows'.
const renderedRows = new WeakMap<Element, string>();

/// Bump the epoch when anything shared by all rows has changed.
function refreshRowEpoch() {
  const inputs = [
    state.view.cache,
    state.view.total,
    state.settings,
    state.markers.bookmarks,
    state.markers.changeSaved,
    state.markers.changeUnsaved,
    state.markers.changeDeleted,
    state.search.matcher,
    state.analysis.matchers,
    // Rule visibility is toggled in place on the same Set, so this one input
    // is compared by value rather than by identity.
    visibleAnalysisRulesKey(),
    // The document's path selects the syntax language, and its identity
    // changes whenever the tab or the file behind it does.
    state.doc.stat,
  ];
  if (inputs.some((input, i) => input !== rowEpochInputs[i])) {
    rowEpochInputs = inputs;
    rowEpoch++;
  }
  return rowEpoch;
}

/// Forget what every pooled row is showing. Every key embeds the epoch, so
/// forcing the next epoch bump invalidates all of them at once. For callers
/// that write into rows behind `render()`'s back.
export function invalidateRenderedRows() {
  rowEpochInputs = [];
}

/// What a row is currently believed to show; `undefined` means "needs filling".
/// Exported for the render-reuse tests.
export function renderedRowKey(row: Element) {
  return renderedRows.get(row);
}

let minimapRenderer = () => {};
let selectionRenderer = () => {};
let selectionPresent = () => false;
let modalOpenProvider = () => false;
let statusPositionRenderer = () => {};

export function setMinimapRenderer(renderer) {
  minimapRenderer = renderer;
}

export function setSelectionRenderer(renderer, hasSelection) {
  selectionRenderer = renderer;
  selectionPresent = hasSelection;
}

export function setModalOpenProvider(provider) {
  modalOpenProvider = provider;
}

export function setStatusPositionRenderer(renderer) {
  statusPositionRenderer = renderer;
}

export function setSelection(selection: Selection | null) {
  state.caret.selection = selection;
  scheduleRender();
}

export function setSearchHits(hits: SearchHit[] | null) {
  state.search.hits = hits;
  scheduleRender();
}

export function setActiveLine(line: number) {
  state.caret.activeLine = line;
  scheduleRender();
}

// Horizontal-scroll preservation. A not-yet-loaded row renders as a narrow "⋯"
// placeholder (see fillRow); when a vertical scroll lands on such rows every
// visible row collapses to that width, #content's scroll width shrinks, and the
// browser clamps scrollLeft back to 0 — so the horizontal position is lost on
// every scroll into un-fetched lines. A hidden zero-height spacer holds the last
// measured content width while data loads, then we re-measure once it's in.
let hkeeper: HTMLDivElement | null = null;
let contentWidth = 0;

function ensureHKeeper(content: HTMLElement) {
  if (hkeeper) return;
  hkeeper = document.createElement("div");
  hkeeper.className = "hkeeper";
  hkeeper.setAttribute("aria-hidden", "true");
  content.append(hkeeper);
}

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
  const gutterPx = gutterPixels();
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
    // Sakura-style fine graduations: a short tick every column and a taller
    // one every 5, painted as two repeating gradients (periods in fractional
    // px of one character cell) so no DOM node exists per column. The
    // numbered full-height lines every 10 stay the .rtick spans above. The
    // background needs a real box to paint into, hence the explicit width —
    // sized to the labeled range.
    const tick = (period, color) =>
      `repeating-linear-gradient(to right, ${color} 0 1px, transparent 1px ${period.toFixed(3)}px)`;
    inner.style.width = `${gutterPx + 500 * cw}px`;
    inner.style.backgroundImage =
      tick(5 * cw, "var(--border)") +
      ", " +
      tick(cw, "color-mix(in srgb, var(--border) 55%, transparent)");
    inner.style.backgroundSize = "auto 9px, auto 5px";
    inner.style.backgroundPosition = `${gutterPx}px bottom, ${gutterPx}px bottom`;
    inner.style.backgroundRepeat = "no-repeat";
  }
  inner.style.transform = `translateX(${-$("content").scrollLeft}px)`;
}

// Rows that fit the viewport in full. rowsVisible() rounds *up* so render()
// also fills the partially clipped bottom row; clamping the scroll range with
// that count left the last line (and the [EOF] marker) permanently cut off at
// the bottom — "全部表示できてない" on huge files.
export function rowsFullyVisible() {
  const h = $("viewport").clientHeight - (state.settings && state.settings.ruler ? 18 : 0);
  return Math.max(1, Math.floor(h / LINE_HEIGHT));
}

export function maxFirst() {
  // state.view.total + 1: the [EOF] marker is the document's final row, so at the
  // bottom of the range the last line and the marker are both shown whole.
  return Math.max(0, state.view.total + 1 - rowsFullyVisible());
}

// ---- data ------------------------------------------------------------------

export function cachedLine(line) {
  const c = state.view.cache;
  const i = line - c.start;
  return i >= 0 && i < c.lines.length ? c.lines[i] : null;
}

export function cacheLineResponse(start: number, response: LinesResponse) {
  state.view.cache = { start, lines: response.lines };
  const markerLines = (kind) =>
    new Set(
      (response.markers || [])
        .filter((marker) => marker.kind === kind)
        .map((marker) => marker.line),
    );
  state.markers.bookmarks = markerLines("bookmark");
  state.markers.changeSaved = markerLines("change-saved");
  state.markers.changeUnsaved = markerLines("change-unsaved");
  state.markers.changeDeleted = markerLines("change-deleted");
  state.view.total = response.total;
}

export async function refreshChangeHistoryOverview() {
  state.markers.changeHistoryOverview = await api<ChangeHistoryResponse>("/api/change-history");
}

export function ensureData(start, count) {
  const need0 = start;
  const need1 = Math.min(state.view.total, start + count);
  const c = state.view.cache;
  if (c.lines.length && need0 >= c.start && need1 <= c.start + c.lines.length) {
    return; // already cached
  }
  const fstart = Math.max(0, start - PAD);
  const fcount = count + 2 * PAD;
  const token = ++state.view.loadToken;
  const url = `/api/lines?start=${fstart}&count=${fcount}`;
  api<LinesResponse>(url)
    .catch(async (firstError) => {
      if (token !== state.view.loadToken) throw firstError;
      return api<LinesResponse>(url);
    })
    .then((res) => {
      if (token !== state.view.loadToken) return; // a newer request superseded us
      cacheLineResponse(fstart, res);
      render();
    })
    .catch((e) => {
      if (token !== state.view.loadToken) return;
      console.error("lines fetch failed", e);
      flashCount(t("editor.reloadError"), "error");
    });
}

export async function lineByte(line, col = null) {
  try {
    const q = col == null ? "" : `&col=${Math.max(0, col)}`;
    const r = await api<LineByteResponse>(`/api/linebyte?line=${Math.max(0, line)}${q}`);
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

// The line number as shown in the gutter: grouped with commas by default
// (17,586,323) so long numbers stay readable, unless the setting is off.
// Right-alignment and width come from CSS (`.ln`), not string padding.
export function formatLineNo(n) {
  return state.settings.lineNumberCommas === false ? String(n) : commas(n);
}

// Width of the largest displayed line number. Keep this as a character count
// so the editor and auxiliary result panes can share the same CSS geometry
// without baking a pixel width (or a generous fixed digit floor) into either.
export function lineNumberChars(maxLine) {
  return formatLineNo(Math.max(0, maxLine)).length;
}

function changeStateForLine(line) {
  if (state.settings.showChangeHistory === false) return null;
  const status = state.markers.changeUnsaved.has(line)
    ? "unsaved"
    : state.markers.changeSaved.has(line)
      ? "saved"
      : null;
  if (!status) return null;
  return { status, deleted: state.markers.changeDeleted.has(line) };
}

function changeStateLabel(change) {
  if (!change) return "";
  const status = t(change.status === "unsaved" ? "changeHistory.unsaved" : "changeHistory.saved");
  return change.deleted
    ? t("changeHistory.deletedState", { state: status, deleted: t("changeHistory.deleted") })
    : status;
}

function applyRowChangeState(row, line) {
  const change = changeStateForLine(line);
  row.classList.toggle("change-unsaved", change?.status === "unsaved");
  row.classList.toggle("change-saved", change?.status === "saved");
  row.classList.toggle("change-deleted", !!change?.deleted);
  return change;
}

export function fillRow(row, line, rec) {
  const ln = row.firstChild;
  const tx = row.lastChild;
  row.className = "row";
  row.dataset.line = String(line);
  ln.textContent = formatLineNo(line + 1);
  const bookmarked = state.markers.bookmarks.has(line);
  const change = applyRowChangeState(row, line);
  row.classList.toggle("bookmarked", bookmarked);
  ln.setAttribute("role", "button");
  ln.setAttribute("tabindex", change ? "0" : "-1");
  const bookmarkLabel = t(bookmarked ? "bookmark.gutterRemove" : "bookmark.gutterAdd", {
    line: formatLineNo(line + 1),
  });
  const changeLabel = changeStateLabel(change);
  ln.setAttribute(
    "aria-label",
    changeLabel
      ? t("changeHistory.gutterLabel", { bookmark: bookmarkLabel, state: changeLabel })
      : bookmarkLabel,
  );
  ln.title = changeLabel ? `${bookmarkLabel}\n${changeLabel}` : bookmarkLabel;
  tx.textContent = "";
  tx.classList.remove("pending");
  row.classList.toggle("inserted", !!rec?.inserted);
  if (rec == null) {
    tx.classList.add("pending");
    tx.textContent = "⋯";
  } else if (state.search.matcher || state.analysis.matchers.length) {
    appendLayeredHighlighted(tx, rec.text);
  } else if (state.settings.syntaxHighlight !== false && appendSyntaxHighlighted(tx, rec.text)) {
    // rendered by appendSyntaxHighlighted
  } else if (state.settings.showWhitespace) {
    appendText(tx, rec.text, true);
  } else {
    tx.textContent = rec.text;
  }
  if (rec != null && state.settings.showWhitespace) appendEol(tx);
  // Hide the current-line highlight while a selection exists — the two
  // washes stack otherwise and the selection becomes hard to read.
  row.classList.toggle("active", line === state.caret.activeLine && !selectionPresent());
}

export function fillEofRow(row) {
  row.className = "row eof";
  row.dataset.line = "-1";
  const ln = row.firstChild;
  ln.textContent = "";
  const change = applyRowChangeState(row, state.view.total);
  if (change) {
    const label = t("changeHistory.eofLabel", { state: changeStateLabel(change) });
    ln.setAttribute("role", "img");
    ln.setAttribute("tabindex", "0");
    ln.setAttribute("aria-label", label);
    ln.title = label;
  } else {
    ln.removeAttribute("role");
    ln.removeAttribute("tabindex");
    ln.removeAttribute("aria-label");
    ln.removeAttribute("title");
  }
  const tx = row.lastChild;
  tx.className = "tx";
  tx.textContent = t("editor.eofMarker");
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
  resetGeometryMeasurements(true);
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

// Probe that replicates a row's horizontal geometry: a gutter-width
// inline-block spacer followed by the text. CSS resolves TAB stops relative
// to the ROW's content edge — which includes the line-number gutter — so a
// bare-text measurement puts every stop past a tab off by the gutter width
// (the caret drifts from where characters actually land, e.g. in TSV files).
// Measuring behind the same spacer makes both geometries identical; the
// returned width IS the x coordinate in #content space.
export let _rowProbe = null;

export let _rowProbeSpacer = null;

export let _rowProbeText = null;

let _rowProbeRange: Range | null = null;
let _rowProbeSource: string | null = null;
let _rowProbeGutter = -1;
let _gutterPixelsCache: number | null = null;
const _rowPrefixWidths = new Map<string, Map<number, number>>();
const _rowGeometry = new Map<string, { chars: string[]; offsets: number[] }>();

// Geometry is shared by selection, caret and ruler rendering. Clear the
// frame-local memo before a new render (or pointer hit-test) so a changed
// gutter can never leak into the next layout pass.
export function resetGeometryMeasurements(clearProbe = false) {
  _gutterPixelsCache = null;
  _rowPrefixWidths.clear();
  _rowGeometry.clear();
  if (clearProbe) {
    _rowProbeSource = null;
    _rowProbeGutter = -1;
  }
}

function ensureRowProbe() {
  if (!_rowProbe) {
    _rowProbe = document.createElement("span");
    _rowProbe.style.cssText =
      "position:absolute;visibility:hidden;white-space:pre;top:-9999px;left:0;pointer-events:none;";
    _rowProbeSpacer = document.createElement("span");
    _rowProbeSpacer.style.cssText = "display:inline-block;height:1px;";
    _rowProbeText = document.createTextNode("");
    _rowProbe.append(_rowProbeSpacer, _rowProbeText);
    $("content").appendChild(_rowProbe);
    _rowProbeRange = document.createRange();
  }
}

function prepareRowProbe(str) {
  ensureRowProbe();
  const gutter = gutterPixels();
  if (_rowProbeGutter !== gutter) {
    _rowProbeSpacer.style.width = `${gutter}px`;
    _rowProbeGutter = gutter;
    _rowProbeSource = null;
  }
  if (_rowProbeSource !== str) {
    _rowProbeText.data = str;
    _rowProbeSource = str;
  }
}

function scalarOffsets(chars: string[]) {
  const offsets = [0];
  for (const ch of chars) offsets.push(offsets[offsets.length - 1] + ch.length);
  return offsets;
}

function geometryForText(str: string) {
  let geometry = _rowGeometry.get(str);
  if (!geometry) {
    const chars = Array.from(str);
    geometry = { chars, offsets: scalarOffsets(chars) };
    _rowGeometry.set(str, geometry);
  }
  return geometry;
}

function measureLinePrefix(str: string, chars: string[], offsets: number[], col: number) {
  const clamped = Math.max(0, Math.min(col, chars.length));
  let widths = _rowPrefixWidths.get(str);
  if (!widths) {
    widths = new Map();
    _rowPrefixWidths.set(str, widths);
  }
  const cached = widths.get(clamped);
  if (cached != null) return cached;

  prepareRowProbe(str);
  let width;
  if (_rowProbeRange && typeof _rowProbeRange.getBoundingClientRect === "function") {
    // Install the full line once, then move only the Range boundary. The first
    // read resolves layout; the binary-search reads that follow do not
    // alternate DOM writes with synchronous layout reads.
    _rowProbeRange.setStart(_rowProbe, 0);
    _rowProbeRange.setEnd(_rowProbeText, offsets[clamped]);
    width = _rowProbeRange.getBoundingClientRect().width;
  } else {
    // jsdom and older embedded engines may not expose Range geometry.
    _rowProbeText.data = chars.slice(0, clamped).join("");
    width = _rowProbe.getBoundingClientRect().width;
    _rowProbeText.data = str;
  }
  widths.set(clamped, width);
  return width;
}

export function measureRowPrefix(str) {
  const { chars, offsets } = geometryForText(str);
  return measureLinePrefix(str, chars, offsets, chars.length);
}

// Pixel x (in #content coordinates, gutter included) of column `col` on `line`.
export function caretX(line, col) {
  const text = cachedLine(line)?.text ?? "";
  const { chars, offsets } = geometryForText(text);
  return measureLinePrefix(text, chars, offsets, col);
}

// Inverse of caretX: nearest column boundary to pixel x (content coordinates).
export function colFromX(line, x) {
  const text = cachedLine(line)?.text ?? "";
  const { chars, offsets } = geometryForText(text);
  if (x <= gutterPixels()) return 0;
  const full = measureLinePrefix(text, chars, offsets, chars.length);
  if (x >= full) return chars.length;
  let lo = 0,
    hi = chars.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (measureLinePrefix(text, chars, offsets, mid) < x) lo = mid + 1;
    else hi = mid;
  }
  const wLo = measureLinePrefix(text, chars, offsets, lo);
  const wPrev = lo > 0 ? measureLinePrefix(text, chars, offsets, lo - 1) : gutterPixels();
  return x - wPrev < wLo - x ? lo - 1 : lo;
}

// ---- selection (multi-line, coordinate-based) ------------------------------

// Width in px of the line-number gutter, measured from a visible row.
export function gutterPixels() {
  if (_gutterPixelsCache != null) return _gutterPixelsCache;
  for (const row of pool) {
    if (row.style.display !== "none" && row.firstChild) {
      _gutterPixelsCache = row.firstChild.getBoundingClientRect().width;
      return _gutterPixelsCache;
    }
  }
  const rootStyle = getComputedStyle(document.documentElement);
  const tokenPixels = (name) => Number.parseFloat(rootStyle.getPropertyValue(name).trim()) || 0;
  _gutterPixelsCache =
    lineNumberChars(state.view.total) * charWidth() +
    tokenPixels("--gutter-pad-start") +
    tokenPixels("--gutter-pad-end") +
    tokenPixels("--gutter-border-width");
  return _gutterPixelsCache;
}

// Map a mouse event to a {line, col} position in the document.
export function coordsFromEvent(e) {
  resetGeometryMeasurements();
  const content = $("content");
  const rect = content.getBoundingClientRect();
  const rowInView = Math.floor((e.clientY - rect.top) / LINE_HEIGHT);
  let line = state.view.first + Math.max(0, rowInView);
  line = Math.max(0, Math.min(line, Math.max(0, state.view.total - 1)));
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
  const re = state.search.matcher;
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

/// Compose normal Find highlighting above analysis backgrounds. Rule priority
/// is resolved by `analysisRanges`; the ordinary `<mark>` stays innermost so
/// the active Find query remains the strongest visual signal.
export function appendLayeredHighlighted(container, text) {
  const analysis = analysisRanges(text, state.analysis.matchers, state.analysis.visibleRuleIds);
  const search = [];
  if (state.search.matcher) {
    state.search.matcher.lastIndex = 0;
    let match;
    while ((match = state.search.matcher.exec(text)) !== null) {
      if (match[0].length === 0) {
        state.search.matcher.lastIndex++;
        continue;
      }
      search.push({ start: match.index, end: match.index + match[0].length });
    }
  }
  if (!analysis.length && !search.length) {
    appendText(container, text, true);
    return;
  }
  const boundaries = [
    ...new Set([
      0,
      text.length,
      ...analysis.flatMap((range) => [range.start, range.end]),
      ...search.flatMap((range) => [range.start, range.end]),
    ]),
  ].sort((a, b) => a - b);
  for (let index = 0; index + 1 < boundaries.length; index++) {
    const start = boundaries[index];
    const end = boundaries[index + 1];
    if (end <= start) continue;
    const analysisRange = analysis.find((range) => range.start < end && range.end > start);
    const searchRange = search.find((range) => range.start < end && range.end > start);
    let target = container;
    if (analysisRange) {
      const span = document.createElement("span");
      span.className = `analysis-mark${analysisRange.overlap ? " overlap" : ""}`;
      span.dataset.analysisColor = analysisRange.color;
      span.dataset.analysisRules = analysisRange.ruleIds.join(" ");
      target.append(span);
      target = span;
    }
    if (searchRange) {
      const mark = document.createElement("mark");
      target.append(mark);
      target = mark;
    }
    appendText(target, text.slice(start, end), end === text.length);
  }
}

export function appendSyntaxHighlighted(container, text) {
  const spans = highlightSpans(text, state.doc.stat?.path || "");
  if (!spans) return false;
  for (let i = 0; i < spans.length; i++) {
    const span = spans[i];
    if (span.kind === "plain") {
      appendText(container, span.text, i === spans.length - 1);
      continue;
    }
    const el = document.createElement("span");
    el.className = `syn syn-${span.kind}`;
    appendText(el, span.text, i === spans.length - 1);
    container.appendChild(el);
  }
  return true;
}

export function render() {
  renderQueued = false;
  resetGeometryMeasurements();
  const content = $("content");
  const vis = rowsVisible();
  const count = vis + OVERSCAN;
  ensurePool(count);
  ensureData(state.view.first, count);
  ensureHKeeper(content);

  // Size the gutter to the widest visible line number (commas included). Every
  // `.ln` reads this through its tokenized width calculation, so normal rows
  // and the empty [EOF] gutter share one width and the numbers right-align.
  const gutterCh = lineNumberChars(state.view.total);
  content.style.setProperty("--gutter-ch", `${gutterCh}ch`);
  // One epoch for the whole pass, so rows that already show the right thing
  // are left untouched instead of torn down and rebuilt (#142).
  const epoch = refreshRowEpoch();
  const selecting = selectionPresent();
  let loading = false;
  for (let r = 0; r < pool.length; r++) {
    const row = pool[r];
    const line = state.view.first + r;
    if (r >= count || line > state.view.total) {
      if (renderedRows.get(row) !== "off") {
        row.style.display = "none";
        renderedRows.set(row, "off");
      }
      continue;
    }
    if (line === state.view.total) {
      const key = `eof:${epoch}`;
      if (renderedRows.get(row) === key) continue;
      renderedRows.set(row, key);
      row.style.display = "";
      fillEofRow(row); // one marker row just past the last line
      continue;
    }
    const rec = cachedLine(line);
    if (rec == null) loading = true;
    // Within one epoch the line cache is fixed, so `line` determines `rec` —
    // the caret's line is the only other thing a row's output turns on.
    const active = line === state.caret.activeLine && !selecting;
    const key = `${epoch}:${line}:${active ? 1 : 0}`;
    if (renderedRows.get(row) === key) continue;
    renderedRows.set(row, key);
    row.style.display = "";
    fillRow(row, line, rec);
  }
  // Keep the horizontal scroll position stable while rows are still loading: the
  // "⋯" placeholders would otherwise collapse the scroll width and snap
  // scrollLeft to 0. Hold the last good width with the spacer during the load,
  // then re-measure once real rows are in. This runs before buildRuler(), which
  // reads scrollLeft — so the value it mirrors onto the ruler is the preserved
  // one, not a clamped 0.
  if (loading) {
    hkeeper!.style.width = `${contentWidth}px`;
  } else {
    hkeeper!.style.width = "0px";
    contentWidth = content.scrollWidth;
  }
  buildRuler();
  selectionRenderer();
  positionCaret();
  updateScrollbar();
  minimapRenderer();
  statusPositionRenderer();
}

export function scheduleRender() {
  if (renderQueued) return;
  renderQueued = true;
  requestAnimationFrame(render);
}

export function setFirst(line) {
  state.view.first = Math.min(Math.max(0, Math.round(line)), maxFirst());
  scheduleRender();
}

// ---- custom scrollbar ------------------------------------------------------

export function updateScrollbar() {
  const vh = $("viewport").clientHeight;
  const thumb = $("vthumb");
  const vis = rowsVisible();
  const ratio = state.view.total > 0 ? Math.min(1, vis / state.view.total) : 1;
  const thumbH = Math.max(24, vh * ratio);
  const mf = maxFirst();
  const top = mf > 0 ? (vh - thumbH) * (state.view.first / mf) : 0;
  thumb.style.height = `${thumbH}px`;
  thumb.style.transform = `translateY(${top}px)`;
  renderSearchTicks(vh);
}

let tickRenderCache = null;
let searchTickNodes: HTMLElement[] = [];

function visibleAnalysisRulesKey() {
  return [...state.analysis.visibleRuleIds].sort().join("\u0000");
}

function updateCurrentSearchTick() {
  const current = state.search.lastMatch ? String(state.search.lastMatch.byte) : null;
  for (const tick of searchTickNodes) {
    tick.classList.toggle("current", current != null && tick.dataset.searchByte === current);
  }
}

export function renderSearchTicks(vh) {
  const ticks = $("vticks");
  if (!ticks) return;
  const nextCache = {
    ticks,
    vh,
    total: state.view.total,
    queryActive: !!state.search.query,
    hits: state.search.hits,
    analysisStatus: state.analysis.status,
    visibleAnalysisRules: visibleAnalysisRulesKey(),
    changeHistory:
      state.settings.showChangeHistory === false ? null : state.markers.changeHistoryOverview,
    language: state.settings.language,
  };
  if (
    tickRenderCache &&
    Object.keys(nextCache).every((key) => tickRenderCache[key] === nextCache[key])
  ) {
    updateCurrentSearchTick();
    return;
  }
  tickRenderCache = nextCache;
  searchTickNodes = [];
  ticks.textContent = "";
  const frag = document.createDocumentFragment();
  const maxTicks = 700;
  if (state.search.query && state.search.hits?.length && state.view.total > 1) {
    const step = Math.max(1, Math.ceil(state.search.hits.length / maxTicks));
    const denom = Math.max(1, state.view.total - 1);
    for (let i = 0; i < state.search.hits.length; i += step) {
      const h = state.search.hits[i];
      if (typeof h.line !== "number") continue;
      const tick = document.createElement("div");
      tick.className = "vtick";
      tick.dataset.searchByte = String(h.byte);
      const y = Math.max(0, Math.min(vh - 3, (h.line / denom) * (vh - 3)));
      tick.style.transform = `translateY(${y}px)`;
      frag.append(tick);
      searchTickNodes.push(tick);
    }
  }
  const analysisRules = (state.analysis.status?.rules || []).filter(
    (rule) =>
      rule.enabled &&
      state.analysis.visibleRuleIds.has(rule.id) &&
      rule.histogram.some((count) => count > 0),
  );
  const occupied = analysisRules.reduce(
    (total, rule) => total + rule.histogram.reduce((n, count) => n + (count > 0 ? 1 : 0), 0),
    0,
  );
  const analysisStep = Math.max(1, Math.ceil(occupied / maxTicks));
  let seen = 0;
  for (const rule of analysisRules) {
    const denominator = Math.max(1, rule.histogram.length - 1);
    rule.histogram.forEach((count, bin) => {
      if (!count || seen++ % analysisStep !== 0) return;
      const tick = document.createElement("div");
      tick.className = "vtick analysis-vtick";
      tick.dataset.analysisColor = rule.color;
      const y = Math.max(0, Math.min(vh - 3, (bin / denominator) * (vh - 3)));
      tick.style.transform = `translateY(${y}px)`;
      frag.append(tick);
    });
  }

  const change =
    state.settings.showChangeHistory === false ? null : state.markers.changeHistoryOverview;
  if (change) {
    const groups = [
      ["saved", change.saved],
      ["unsaved", change.unsaved],
    ] as const;
    const occupiedChanges = groups.reduce(
      (sum, [, group]) => sum + group.histogram.reduce((n, value) => n + (value > 0 ? 1 : 0), 0),
      0,
    );
    const changeStep = Math.max(1, Math.ceil(occupiedChanges / MAX_CHANGE_TICKS));
    let changeSeen = 0;
    for (const [status, group] of groups) {
      const denominator = Math.max(1, group.histogram.length - 1);
      group.histogram.forEach((value, bin) => {
        if (!value || changeSeen++ % changeStep !== 0) return;
        const tick = document.createElement("div");
        tick.className = `vtick change-vtick change-${status}-vtick`;
        if (change.deleted.histogram[bin]) tick.classList.add("change-deleted-vtick");
        const y = Math.max(0, Math.min(vh - 3, (bin / denominator) * (vh - 3)));
        tick.style.transform = `translateY(${y}px)`;
        tick.setAttribute("aria-hidden", "true");
        frag.append(tick);
      });
    }
    if (change.saved.count || change.unsaved.count) {
      ticks.setAttribute("role", "img");
      const summary = t("changeHistory.overviewLabel", {
        saved: commas(change.saved.count),
        unsaved: commas(change.unsaved.count),
      });
      ticks.setAttribute(
        "aria-label",
        change.limit_reached ? `${summary} ${t("changeHistory.limited")}` : summary,
      );
    } else {
      ticks.removeAttribute("role");
      ticks.removeAttribute("aria-label");
    }
  } else {
    ticks.removeAttribute("role");
    ticks.removeAttribute("aria-label");
  }
  ticks.append(frag);
  updateCurrentSearchTick();
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
    startFirst = state.view.first;
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
    setFirst(state.view.first + (above ? -1 : 1) * rowsVisible());
    void rect;
  });
}

export function revealLine(line) {
  const vis = rowsVisible();
  if (line < state.view.first || line >= state.view.first + vis) {
    setFirst(line - Math.floor(vis / 3));
  } else {
    scheduleRender();
  }
  statusPositionRenderer();
}

export function clearLineCache() {
  state.view.cache = { start: 0, lines: [] };
  state.markers.bookmarks = new Set();
  state.markers.bookmarkCount = 0;
  state.markers.changeSaved = new Set();
  state.markers.changeUnsaved = new Set();
  state.markers.changeDeleted = new Set();
  state.markers.changeHistoryOverview = null;
  state.view.loadToken++;
}

// ===========================================================================
//  Caret-based editing (Notepad / Sakura Editor style)
//
//  There is a single fluid caret (state.caret.position) you can place anywhere and type
//  across lines. Every mutation is expressed as one range replacement and sent
//  to the backend's /api/edit/replace_range, which records it as a single undo
//  step and returns the resulting caret. Edits are serialized through a small
//  queue so fast typing/IME stays in order; the visible window is re-fetched
//  after each edit (cheap over loopback) rather than mirrored optimistically.
// ===========================================================================

// Move the caret without touching the selection model wholesale (callers set
// state.caret.selection themselves). Keeps the active-line highlight on the caret line.
// `knownLen` bypasses the lineLen() clamp for a column the caller resolved
// from the server — lineLen() guesses 0 outside the cache window.
export function setCaret(line, col, knownLen = null) {
  line = Math.max(0, Math.min(line, Math.max(0, state.view.total - 1)));
  col = Math.max(0, Math.min(col, knownLen ?? lineLen(line)));
  state.caret.position = { line, col };
  setActiveLine(line);
  state.caret.extraCursors = []; // any explicit caret placement collapses multi-cursor
  state.caret.editGeneration++; // user-driven caret placement (click, search, open, …)
}

// Caret motion for the keyboard: `extend` grows the selection from its anchor.
// `knownLen` as in setCaret: a server-resolved length for an uncached line.
export function moveCaret(line, col, extend, knownLen = null) {
  line = Math.max(0, Math.min(line, Math.max(0, state.view.total - 1)));
  col = Math.max(0, Math.min(col, knownLen ?? lineLen(line)));
  if (extend) {
    const anchor = state.caret.selection
      ? state.caret.selection.anchor
      : { ...state.caret.position };
    setSelection({ anchor, head: { line, col } });
    state.caret.extraCursors = []; // entering a selection collapses multi-cursor
  } else {
    setSelection(null);
  }
  state.caret.position = { line, col };
  setActiveLine(line);
  state.caret.editGeneration++; // user-driven caret motion (arrows, Home/End, PageUp/Down)
  revealCaret();
  scheduleRender();
}

export function focusEditor() {
  const hi = $("hidden-input");
  if (hi && document.activeElement !== hi) hi.focus({ preventScroll: true });
  state.caret.focused = true;
  scheduleRender();
}

// Bring the caret into view: scroll vertically (whole lines) and horizontally
// (#content is the horizontal scroll container).
export function revealCaret() {
  // Clamp against the *fully* visible row count: landing the caret on the
  // half-clipped bottom row would leave the line it is on unreadable.
  const vis = rowsFullyVisible();
  if (state.caret.position.line < state.view.first) {
    state.view.first = Math.min(state.caret.position.line, maxFirst());
  } else if (state.caret.position.line >= state.view.first + vis) {
    state.view.first = Math.min(Math.max(0, state.caret.position.line - vis + 1), maxFirst());
  }
  const content = $("content");
  const x = caretX(state.caret.position.line, state.caret.position.col);
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
  const focusVisible = state.caret.focused && !modalOpenProvider() && !state.caret.composing;
  positionExtraCarets(vis, focusVisible);
  const onScreen =
    !!state.doc.stat?.open &&
    state.caret.position.line >= state.view.first &&
    state.caret.position.line < state.view.first + vis;
  const show = onScreen && state.caret.focused && !modalOpenProvider();
  caretEl.classList.toggle("on", show && !state.caret.composing);
  if (!onScreen) return;
  const x = caretX(state.caret.position.line, state.caret.position.col);
  const y = (state.caret.position.line - state.view.first) * LINE_HEIGHT;
  caretEl.style.transform = `translate(${x}px, ${y}px)`;
  hi.style.transform = `translate(${x}px, ${y}px)`;
}

// Mirror #caret for every extra cursor: same transform math and the same
// visibility rules (focus, modal open, IME composition, offscreen). The divs
// live in a small pool inside #content and are trimmed when cursors go away.
export const extraCaretPool = [];

export function positionExtraCarets(vis, focusVisible) {
  const cursors = state.caret.extraCursors;
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
      !!state.doc.stat?.open && c.line >= state.view.first && c.line < state.view.first + vis;
    el.classList.toggle("on", onScreen && focusVisible);
    if (onScreen) {
      const x = caretX(c.line, c.col);
      const y = (c.line - state.view.first) * LINE_HEIGHT;
      el.style.transform = `translate(${x}px, ${y}px)`;
    }
  }
}
