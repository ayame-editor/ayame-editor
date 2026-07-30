// Ayame Editor — bounded document minimap.
//
// The browser still holds only the editor's existing PAD-sized cache. This
// strip never requests lines: uncached positions are faint stubs and become
// detailed only after ordinary scrolling brings them into the cache.
import { $ } from "./dom.js";
import { state } from "./state.js";
import { cachedLine, maxFirst, rowsVisible, setFirst, setMinimapRenderer } from "./editor.js";

export const MINIMAP_ROW = 3;
export const MINIMAP_WIDTH = 88;

export function minimapCapacity(heightPx) {
  return Math.max(1, Math.floor(heightPx / MINIMAP_ROW));
}

// Slide a capacity-sized window proportionally over the whole document. The
// formula also keeps the editor viewport inside the map window at both ends.
export function minimapStart(first, total, capacity, maxFirstLine) {
  if (total + 1 <= capacity || maxFirstLine <= 0) return 0;
  const fraction = Math.max(0, Math.min(1, first / maxFirstLine));
  return Math.round(fraction * (total + 1 - capacity));
}

export function lineAtMinimapY(y, mapStart) {
  return mapStart + Math.floor(Math.max(0, y) / MINIMAP_ROW);
}

export function scrubTargetFirst(y, mapStart, visibleRows) {
  return lineAtMinimapY(y, mapStart) - Math.floor(visibleRows / 2);
}

let lastMapStart = 0;

function themeColor(styles, name, fallback) {
  return styles.getPropertyValue(name).trim() || fallback;
}

function indentOf(text) {
  let cells = 0;
  let chars = 0;
  for (; chars < text.length; chars++) {
    const ch = text[chars];
    if (ch === " ") cells += 1;
    else if (ch === "\t") cells += 4;
    else if (ch === "　") cells += 2;
    else break;
  }
  return { cells, chars };
}

export function updateMinimap() {
  const canvas = $("minimap") as HTMLCanvasElement | null;
  if (!canvas || state.settings.minimap === false) return;
  const context = canvas.getContext?.("2d");
  if (!context) return;
  const cssWidth = canvas.clientWidth;
  const cssHeight = canvas.clientHeight;
  if (cssWidth <= 0 || cssHeight <= 0) return;

  const dpr = window.devicePixelRatio || 1;
  const pixelWidth = Math.round(cssWidth * dpr);
  const pixelHeight = Math.round(cssHeight * dpr);
  if (canvas.width !== pixelWidth || canvas.height !== pixelHeight) {
    canvas.width = pixelWidth;
    canvas.height = pixelHeight;
  }
  context.setTransform(dpr, 0, 0, dpr, 0, 0);
  context.clearRect(0, 0, cssWidth, cssHeight);

  canvas.setAttribute("aria-valuemin", "1");
  canvas.setAttribute("aria-valuemax", String(Math.max(1, state.view.total + 1)));
  canvas.setAttribute(
    "aria-valuenow",
    String(Math.min(state.view.first + 1, state.view.total + 1)),
  );
  if (state.view.total <= 0) return;

  const styles = getComputedStyle(document.documentElement);
  const foreground = themeColor(styles, "--fg", "#333");
  const accent = themeColor(styles, "--accent", "#8859b1");
  const capacity = minimapCapacity(cssHeight);
  const mapStart = minimapStart(state.view.first, state.view.total, capacity, maxFirst());
  lastMapStart = mapStart;
  const usableWidth = cssWidth - 4;

  for (let index = 0; index < capacity; index++) {
    const line = mapStart + index;
    if (line >= state.view.total) break;
    const y = index * MINIMAP_ROW;
    const record = cachedLine(line);
    if (record == null) {
      context.globalAlpha = 0.07;
      context.fillStyle = foreground;
      context.fillRect(2, y, usableWidth * 0.3, 2);
      continue;
    }
    const text = record.text || "";
    if (!text) continue;
    const { cells, chars } = indentOf(text);
    const x = 2 + Math.min(cells, 24);
    const width = Math.max(1, Math.min(text.length - chars, usableWidth - (x - 2)));
    context.globalAlpha = record.inserted ? 0.55 : 0.3;
    context.fillStyle = record.inserted ? accent : foreground;
    context.fillRect(x, y, width, 2);
  }

  if (state.caret.selection) {
    const first = Math.min(state.caret.selection.anchor.line, state.caret.selection.head.line);
    const last = Math.max(state.caret.selection.anchor.line, state.caret.selection.head.line);
    const from = Math.max(first, mapStart);
    const to = Math.min(last, mapStart + capacity - 1);
    if (to >= from) {
      context.globalAlpha = 0.16;
      context.fillStyle = accent;
      context.fillRect(0, (from - mapStart) * MINIMAP_ROW, cssWidth, (to - from + 1) * MINIMAP_ROW);
    }
  }

  if (state.search.query && state.search.hits?.length) {
    context.globalAlpha = 0.6;
    context.fillStyle = accent;
    for (const hit of state.search.hits) {
      const index = hit.line - mapStart;
      if (index >= 0 && index < capacity) {
        context.fillRect(0, index * MINIMAP_ROW, cssWidth, 2);
      }
    }
  }

  const caretIndex = state.caret.position.line - mapStart;
  if (state.doc.stat?.open && caretIndex >= 0 && caretIndex < capacity) {
    context.globalAlpha = 0.85;
    context.fillStyle = accent;
    context.fillRect(0, caretIndex * MINIMAP_ROW, cssWidth, 2);
  }

  const visibleRows = rowsVisible();
  const top = Math.max(0, (state.view.first - mapStart) * MINIMAP_ROW);
  const height = Math.min(cssHeight - top, Math.max(MINIMAP_ROW, visibleRows * MINIMAP_ROW));
  context.globalAlpha = 0.1;
  context.fillStyle = foreground;
  context.fillRect(0, top, cssWidth, height);
  context.globalAlpha = 0.3;
  context.strokeStyle = foreground;
  context.lineWidth = 1;
  context.strokeRect(0.5, top + 0.5, cssWidth - 1, Math.max(0, height - 1));
  context.globalAlpha = 1;
}

export function initMinimap() {
  const canvas = $("minimap");
  if (!canvas) return;
  setMinimapRenderer(updateMinimap);

  let activePointer: number | null = null;
  const jump = (clientY) => {
    const rect = canvas.getBoundingClientRect();
    setFirst(scrubTargetFirst(clientY - rect.top, lastMapStart, rowsVisible()));
  };
  canvas.addEventListener("pointerdown", (event: PointerEvent) => {
    activePointer = event.pointerId;
    canvas.setPointerCapture?.(event.pointerId);
    canvas.classList.add("drag");
    jump(event.clientY);
    event.preventDefault();
    event.stopPropagation();
  });
  canvas.addEventListener("pointermove", (event: PointerEvent) => {
    if (event.pointerId === activePointer) jump(event.clientY);
  });
  const stop = (event: PointerEvent) => {
    if (event.pointerId !== activePointer) return;
    activePointer = null;
    canvas.classList.remove("drag");
  };
  canvas.addEventListener("pointerup", stop);
  canvas.addEventListener("pointercancel", stop);
  canvas.addEventListener("keydown", (event: KeyboardEvent) => {
    const visible = rowsVisible();
    let next = state.view.first;
    if (event.key === "ArrowUp") next -= 1;
    else if (event.key === "ArrowDown") next += 1;
    else if (event.key === "PageUp") next -= visible;
    else if (event.key === "PageDown") next += visible;
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = maxFirst();
    else return;
    setFirst(next);
    event.preventDefault();
  });
}
