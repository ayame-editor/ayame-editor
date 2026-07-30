// Ayame Editor — status bar and window-title rendering.
import { $, commas, displayName, humanBytes } from "./dom.js";
import { state } from "./state.js";
import { currentLocale, t } from "./i18n.js";
import { setAppTitle } from "./app.js";
import { isKeymapDoc, isThemeDoc } from "./document-kind.js";

export function enc(encoding) {
  // Keys match the core Encoding enum's kebab-case serialization (Utf8 → "utf8").
  return (
    {
      utf8: "UTF-8",
      "utf-8": "UTF-8",
      "utf-16le": "UTF-16 LE",
      "utf-16be": "UTF-16 BE",
      "shift-jis": "Shift_JIS",
      "euc-jp": "EUC-JP",
      ascii: "ASCII",
    }[encoding] || String(encoding)
  );
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

export function updateStatusMeta() {
  const stat = state.doc.stat;
  if (!stat) {
    setAppTitle("Ayame Editor");
    return;
  }
  if (!stat.open) {
    for (const id of ["st-enc", "st-eol", "st-edit", "st-index"]) {
      $(id).textContent = "—";
    }
    $("st-edit").title = "";
    $("st-index").title = "";
    $("st-enc").setAttribute("aria-label", t("status.encodingValue", { value: "—" }));
    $("st-eol").setAttribute("aria-label", t("status.eolValue", { value: "—" }));
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
  $("st-pos").textContent = count ? t("status.posCursors", { pos, n: count + 1 }) : pos;
}
