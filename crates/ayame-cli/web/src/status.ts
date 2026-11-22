// Ayame Editor — status bar and window-title rendering.
import { $, commas, displayName, humanBytes } from "./dom.js";
import { state } from "./state.js";
import { currentLocale, t } from "./i18n.js";
import { setAppTitle } from "./app.js";
import { isKeymapDoc, isThemeDoc } from "./document-kind.js";
import { encodingLabel } from "./encodings.js";
import {
  hasTextSelection,
  rectRange,
  selectionLineCount,
  selectionRanges,
} from "./selection-model.js";
import { resolveSyntaxScheme, schemeDefinition } from "./syntax.js";

export function enc(encoding) {
  return encodingLabel(encoding);
}

export function eol(lineEnding) {
  // LF/CRLF/CR are universal; "混在"/"なし" (Mixed/None) are words, so localize.
  return (
    {
      lf: "LF",
      crlf: "CRLF",
      cr: "CR",
      mixed: t("status.eolMixed"),
      none: t("status.eolNone"),
    }[lineEnding] || String(lineEnding)
  );
}

export function syntaxStatusValue(path: string) {
  const selection = state.syntax.overrides[path] || "auto";
  const resolved = resolveSyntaxScheme(path, selection, state.syntax.mappings);
  const scheme = t(schemeDefinition(resolved || "plain").labelKey);
  return selection === "auto" ? t("syntax.autoValue", { scheme }) : scheme;
}

export function updateStatusMeta() {
  const stat = state.doc.stat;
  if (!stat) {
    setAppTitle("Ayame Editor");
    return;
  }
  if (!stat.open) {
    for (const id of ["st-syntax", "st-enc", "st-eol", "st-edit", "st-index"]) {
      $(id).textContent = "—";
    }
    $("st-edit").title = "";
    $("st-index").title = "";
    $("st-enc").setAttribute("aria-label", t("status.encodingValue", { value: "—" }));
    $("st-eol").setAttribute("aria-label", t("status.eolValue", { value: "—" }));
    $("st-syntax").setAttribute("aria-label", t("syntax.statusValue", { value: "—" }));
    $("st-pos").textContent = t("status.line0");
    $("undo-edit").disabled = true;
    $("redo-edit").disabled = true;
    $("apply-theme").classList.add("hidden");
    $("apply-keymap").classList.add("hidden");
    setAppTitle("Ayame Editor");
    return;
  }
  const name = displayName(stat.path);
  const dirtyMark = stat.dirty ? "* " : "";
  setAppTitle(`${dirtyMark}${name} - Ayame Editor`);
  $("apply-theme").classList.toggle("hidden", !isThemeDoc(stat.path));
  $("apply-keymap").classList.toggle("hidden", !isKeymapDoc(stat.path));
  const lines = stat.view_lines ?? stat.lines;
  const encoding =
    stat.bom_bytes > 0 ? t("status.encWithBom", { enc: enc(stat.encoding) }) : enc(stat.encoding);
  const lineEnding = eol(stat.eol);
  const syntax = syntaxStatusValue(stat.path);
  $("st-syntax").textContent = syntax;
  $("st-syntax").setAttribute("aria-label", t("syntax.statusValue", { value: syntax }));
  $("st-enc").textContent = encoding;
  $("st-enc").setAttribute("aria-label", t("status.encodingValue", { value: encoding }));
  $("st-eol").textContent = lineEnding;
  $("st-eol").setAttribute("aria-label", t("status.eolValue", { value: lineEnding }));
  $("st-edit").textContent = stat.dirty ? t("status.unsaved") : t("status.saved");
  $("st-edit").title = stat.dirty
    ? t("status.unsavedDetail", {
        added: commas(stat.inserted_lines),
        changed: commas(stat.replaced_lines),
        deleted: commas(stat.deleted_lines),
      })
    : t("status.allSaved");
  $("undo-edit").disabled = !stat.can_undo;
  $("redo-edit").disabled = !stat.can_redo;
  $("st-index").textContent = t("status.indexOk");
  $("st-index").title = t("status.indexDetail", {
    lines: commas(lines),
    bytes: humanBytes(stat.bytes, currentLocale()),
    checkpoints: commas(stat.checkpoints),
    indexBytes: humanBytes(stat.index_bytes, currentLocale()),
    indexMs: stat.index_ms,
  });
  const tab = $("tabs").querySelector(".tab.active");
  if (tab) tab.classList.toggle("dirty", !!stat.dirty);
  const activeTab = (state.doc.tabs || []).find((item) => item.active);
  if (activeTab) activeTab.dirty = !!stat.dirty;
}

export function updateStatusPos() {
  if (state.view.total === 0) {
    $("st-pos").textContent = t("status.line0");
    return;
  }
  const pos = t("status.pos", {
    line: commas(state.caret.position.line + 1),
    col: commas(state.caret.position.col + 1),
  });
  const count = state.caret.extraCursors.length;
  const text = count ? t("status.posCursors", { pos, n: count + 1 }) : pos;
  if (!hasTextSelection()) {
    $("st-pos").textContent = text;
    return;
  }
  // Selection size next to the caret: how much a copy would carry. Character
  // counts need the selected lines' text, which is resident for anything on
  // screen; a selection reaching into un-fetched lines keeps the line count
  // rather than triggering a fetch on every caret move.
  const lines = commas(selectionLineCount());
  const chars = selectionCharCount();
  $("st-pos").textContent =
    chars == null
      ? t("status.selectionPosLines", { pos: text, lines })
      : t("status.selectionPos", { pos: text, lines, chars: commas(chars) });
}

/// Text of one line from the viewport caches, or `null` when it is not loaded.
function cachedLineText(line) {
  const cache = state.view.cache;
  const index = line - cache.start;
  if (index >= 0 && index < cache.lines.length) return cache.lines[index].text ?? "";
  const record = state.view.sparseCache.get(line);
  return record ? (record.text ?? "") : null;
}

// Selected scalars across every range, counting the newline between lines so
// the number matches the text a copy would produce. `null` if any selected
// line is outside the cache.
function selectionCharCount() {
  let total = 0;
  const rect = rectRange();
  if (rect) {
    for (let line = rect.l0; line <= rect.l1; line++) {
      const text = cachedLineText(line);
      if (text == null) return null;
      total += Array.from(text).slice(rect.c0, rect.c1).length;
    }
    return total;
  }
  for (const range of selectionRanges()) {
    const count = rangeCharCount(range);
    if (count == null) return null;
    total += count;
  }
  return total;
}

function rangeCharCount(range) {
  const first = cachedLineText(range.start.line);
  if (first == null) return null;
  if (range.start.line === range.end.line) {
    return Array.from(first).slice(range.start.col, range.end.col).length;
  }
  let total = Array.from(first).slice(range.start.col).length;
  for (let line = range.start.line + 1; line <= range.end.line; line++) {
    const text = cachedLineText(line);
    if (text == null) return null;
    const chars = Array.from(text);
    total += 1 + (line === range.end.line ? chars.slice(0, range.end.col).length : chars.length);
  }
  return total;
}
