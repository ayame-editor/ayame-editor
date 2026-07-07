// Ayame Editor — input module. Type-stripped to JS at build time (build.rs, oxc).
import { $ } from "./dom.js";
import { LINE_HEIGHT, state } from "./state.js";
import { t } from "./i18n.js";
import {
  convertSave,
  convertVisible,
  hideConvert,
  reopenWithEncoding,
  saveCopy,
  saveFile,
  savingCount,
  showConvert,
  syncConvertBom,
} from "./save.js";
import {
  focusEditor,
  lineChars,
  lineLen,
  moveCaret,
  positionCaret,
  rowsVisible,
  scheduleRender,
  setFirst,
} from "./editor.js";
import { addCursorAbove, addCursorBelow, clearExtraCursors, caretToDocEnd } from "./selection.js";
import {
  commandPaletteVisible,
  ctxMenuVisible,
  fileMenuVisible,
  hideCommandPalette,
  hideCtxMenu,
  hideFileMenu,
  hideKeymap,
  keymapVisible,
  matchesShortcut,
  refreshFindOptButtons,
  runAction,
  shortcutActionFromEvent,
  toggleOpt,
} from "./menus.js";
import {
  applyRange,
  backspace,
  deleteLines,
  deleteSelectionEdit,
  duplicateLines,
  enqueueEdit,
  forwardDelete,
  insertNewline,
  moveLines,
  pasteText,
  redoEdit,
  setFollowTail,
  typeText,
  undoEdit,
} from "./edits.js";
import {
  buildMatcher,
  diffVisible,
  findStep,
  flashCount,
  grepVisible,
  hideDiff,
  hideFind,
  hideGrep,
  replaceAll,
  replaceCurrent,
  selectNextOccurrence,
  setReplaceRow,
  showSearchHistory,
  updateCount,
} from "./search.js";
import { confirmVisible, formVisible, loadingVisible, promptVisible } from "./dialogs.js";
import { hideOpener, openerVisible } from "./workspace.js";
import {
  adjustZoom,
  applyKeymapFromBuffer,
  applyThemeFromBuffer,
  hideSettings,
  settingsVisible,
  setZoom,
  ZOOM_STEP,
} from "./settings.js";

export function anyModalOpen() {
  return (
    promptVisible() ||
    formVisible() ||
    confirmVisible() ||
    settingsVisible() ||
    keymapVisible() ||
    commandPaletteVisible() ||
    diffVisible() ||
    grepVisible() ||
    openerVisible() ||
    convertVisible() ||
    loadingVisible()
  );
}

const ESCAPE_CLOSE_HANDLERS: [() => boolean, () => void][] = [
  [ctxMenuVisible, hideCtxMenu],
  [fileMenuVisible, () => hideFileMenu(true)],
  [keymapVisible, hideKeymap],
  [commandPaletteVisible, hideCommandPalette],
  [diffVisible, hideDiff],
  [grepVisible, hideGrep],
  [settingsVisible, hideSettings],
  [convertVisible, hideConvert],
  [openerVisible, hideOpener],
  [
    () => state.findOpen,
    () => {
      hideFind();
      focusEditor();
    },
  ],
];

// ---- input wiring ----------------------------------------------------------

export function setQueryFromInput() {
  state.query = $("find").value;
  state.lastMatch = null;
  state.searchHits = null;
  state.searchTruncated = false;
  buildMatcher();
  $("find-count").textContent = state.regexError ? t("find.regexError") : "";
  scheduleRender();
}

export function initEvents() {
  const vp = $("viewport");

  vp.addEventListener(
    "wheel",
    (e) => {
      if (e.ctrlKey || e.metaKey) {
        // Ctrl/Cmd + wheel zooms instead of scrolling.
        e.preventDefault();
        adjustZoom(e.deltaY < 0 ? ZOOM_STEP : -ZOOM_STEP);
        return;
      }
      e.preventDefault();
      let dy = e.deltaY;
      if (e.deltaMode === 1) dy *= LINE_HEIGHT;
      else if (e.deltaMode === 2) dy *= vp.clientHeight;
      state.fracAcc += dy / LINE_HEIGHT;
      const whole = Math.trunc(state.fracAcc);
      state.fracAcc -= whole;
      if (whole !== 0) setFirst(state.first + whole);
    },
    { passive: false },
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
  $("find-expand").addEventListener("click", () => setReplaceRow(!state.replaceOpen));
  $("replace-one").addEventListener("click", () => replaceCurrent());
  $("replace-all").addEventListener("click", () => replaceAll());
  $("replace-input").addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      replaceCurrent();
    } else if (e.key === "Escape") {
      hideFind();
      focusEditor();
    }
  });
  $("find-next").addEventListener("click", () => findStep("next"));
  $("find-prev").addEventListener("click", () => findStep("prev"));
  $("opt-case").addEventListener("click", () => toggleOpt("ci", "opt-case"));
  $("opt-word").addEventListener("click", () => toggleOpt("word", "opt-word"));
  $("opt-regex").addEventListener("click", () => toggleOpt("regex", "opt-regex"));
  // Reflect the initial search-option state (Match Case is lit when ci is off).
  refreshFindOptButtons();
  $("save-file").addEventListener("click", () => {
    hideFileMenu();
    saveFile();
  });
  $("save-copy").addEventListener("click", () => {
    hideFileMenu();
    saveCopy();
  });
  $("convert-save-item").addEventListener("click", showConvert);
  $("convert-close").addEventListener("click", hideConvert);
  $("convert-cancel").addEventListener("click", hideConvert);
  $("convert-enc").addEventListener("change", syncConvertBom);
  $("convert-go").addEventListener("click", () => {
    const encoding = $("convert-enc").value;
    const eolVal = $("convert-eol").value;
    const bom = ["utf-8", "utf-16le", "utf-16be"].includes(encoding) && $("convert-bom").checked;
    hideConvert();
    convertSave(encoding, eolVal, bom);
  });
  $("reopen-go").addEventListener("click", () => {
    const encoding = $("convert-enc").value;
    hideConvert();
    reopenWithEncoding(encoding);
  });
  $("convert-modal").addEventListener("click", (e) => {
    if (e.target === $("convert-modal")) hideConvert();
  });
  $("st-enc").addEventListener("click", showConvert);
  $("st-eol").addEventListener("click", showConvert);
  $("st-zoom").addEventListener("click", () => setZoom(100));
  $("st-tail").addEventListener("click", () => setFollowTail(!state.followTail));
  $("apply-theme").addEventListener("click", applyThemeFromBuffer);
  $("apply-keymap").addEventListener("click", applyKeymapFromBuffer);
  $("undo-edit").addEventListener("click", undoEdit);
  $("redo-edit").addEventListener("click", redoEdit);
  $("diff-close").addEventListener("click", hideDiff);
  $("diff-modal").addEventListener("click", (e) => {
    if (e.target === $("diff-modal")) hideDiff();
  });
  $("grep-close").addEventListener("click", hideGrep);
  $("grep-modal").addEventListener("click", (e) => {
    if (e.target === $("grep-modal")) hideGrep();
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

// App-level shortcuts. Caret motion and text editing live in onEditKey (bound
// to the hidden input); those keys never reach here because onEditKey stops
// their propagation. `inField` is true only for the real text inputs (find /
// opener / prompt / settings), never the editor's hidden textarea.
export function onGlobalKey(e) {
  const inField = e.target.tagName === "INPUT";
  if (promptVisible() || formVisible() || confirmVisible()) return;
  if (e.key === "Escape") {
    const handler = ESCAPE_CLOSE_HANDLERS.find(([visible]) => visible());
    if (handler) {
      e.preventDefault();
      handler[1]();
      return;
    }
  }
  // A modal owns the keyboard: never run editor/clipboard/history/nav commands
  // against the hidden document behind Settings / the Opener / a prompt.
  if (anyModalOpen()) return;

  const action = shortcutActionFromEvent(e, inField);
  if (action) {
    e.preventDefault();
    hideFileMenu();
    runAction(action);
  }
}

// ---- editor keyboard: caret motion + structural edits ----------------------

export const isWordChar = (ch) => /[\p{L}\p{N}_]/u.test(ch || "");

export function wordLeft(line, col) {
  const cs = lineChars(line);
  if (col === 0) return line > 0 ? [line - 1, lineLen(line - 1)] : [line, 0];
  let i = col;
  while (i > 0 && !isWordChar(cs[i - 1])) i--;
  while (i > 0 && isWordChar(cs[i - 1])) i--;
  return [line, i];
}

export function wordRight(line, col) {
  const cs = lineChars(line);
  const len = cs.length;
  if (col >= len) return line < state.total - 1 ? [line + 1, 0] : [line, len];
  let i = col;
  while (i < len && !isWordChar(cs[i])) i++;
  while (i < len && isWordChar(cs[i])) i++;
  return [line, i];
}

export function deleteWordBack() {
  enqueueEdit(() => {
    const del = deleteSelectionEdit();
    if (del) return del;
    clearExtraCursors(); // word-delete is single-cursor: collapse to the primary
    const c = state.caret;
    const [l, col] = wordLeft(c.line, c.col);
    if (l === c.line && col === c.col) return null;
    return applyRange(l, col, c.line, c.col, "");
  });
}

export function deleteWordFwd() {
  enqueueEdit(() => {
    const del = deleteSelectionEdit();
    if (del) return del;
    clearExtraCursors(); // word-delete is single-cursor: collapse to the primary
    const c = state.caret;
    const [l, col] = wordRight(c.line, c.col);
    if (l === c.line && col === c.col) return null;
    return applyRange(c.line, c.col, l, col, "");
  });
}

export function onEditKey(e) {
  if (state.composing || e.isComposing) return; // IME owns the keyboard
  if (anyModalOpen()) return; // a dialog is up; don't edit behind it
  if (savingCount > 0) {
    // Edits are blocked while a save is in flight; swallow the key so the
    // hidden textarea can't buffer text that would never be applied.
    e.preventDefault();
    flashCount(t("editor.savingWait"));
    return;
  }
  const mod = e.ctrlKey || e.metaKey;
  const shift = e.shiftKey;
  const c = state.caret;
  const take = () => {
    e.preventDefault();
    e.stopPropagation();
  };
  // Zoom: Ctrl/Cmd with +/-/0. Checked before the switch so nothing else claims
  // them (and the browser/webview page zoom is suppressed).
  if (mod && !e.altKey) {
    if (e.key === "+" || e.key === "=") {
      take();
      adjustZoom(ZOOM_STEP);
      return;
    }
    if (e.key === "-" || e.key === "_") {
      take();
      adjustZoom(-ZOOM_STEP);
      return;
    }
    if (e.key === "0") {
      take();
      setZoom(100);
      return;
    }
  }
  // Multi-cursor: add a caret above/below (default Ctrl+Alt+ArrowUp/Down).
  // Checked before the switch so the plain-arrow cases never swallow them.
  if (matchesShortcut(e, "addCursorAbove")) {
    take();
    addCursorAbove();
    return;
  }
  if (matchesShortcut(e, "addCursorBelow")) {
    take();
    addCursorBelow();
    return;
  }
  if (matchesShortcut(e, "selectNextOccurrence")) {
    take();
    selectNextOccurrence();
    return;
  }
  // Whole-line ops: checked before the switch so the plain-arrow cases never
  // swallow 行を上へ/下へ移動 (default Alt+ArrowUp/Down).
  if (matchesShortcut(e, "duplicateLine")) {
    take();
    duplicateLines();
    return;
  }
  if (matchesShortcut(e, "moveLineUp")) {
    take();
    moveLines(-1);
    return;
  }
  if (matchesShortcut(e, "moveLineDown")) {
    take();
    moveLines(1);
    return;
  }
  if (matchesShortcut(e, "deleteLine")) {
    take();
    deleteLines();
    return;
  }
  switch (e.key) {
    case "ArrowLeft":
      take();
      if (mod) {
        const [l, col] = wordLeft(c.line, c.col);
        moveCaret(l, col, shift);
      } else if (c.col > 0) moveCaret(c.line, c.col - 1, shift);
      else if (c.line > 0) moveCaret(c.line - 1, lineLen(c.line - 1), shift);
      state.goalCol = state.caret.col;
      return;
    case "ArrowRight":
      take();
      if (mod) {
        const [l, col] = wordRight(c.line, c.col);
        moveCaret(l, col, shift);
      } else if (c.col < lineLen(c.line)) moveCaret(c.line, c.col + 1, shift);
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
      if (mod) {
        caretToDocEnd(shift); // async: resolves the uncached last line's length
      } else {
        moveCaret(c.line, lineLen(c.line), shift);
        state.goalCol = state.caret.col;
      }
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
      if (mod) deleteWordBack();
      else backspace();
      return;
    case "Delete":
      take();
      if (mod) deleteWordFwd();
      else forwardDelete();
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
      // (modals never reach this handler — see the guards above).
      if (state.extraCursors.length) {
        take();
        clearExtraCursors();
        return;
      }
      if (state.sel) {
        take();
        state.sel = null;
        scheduleRender();
      }
      if (state.findOpen) {
        take();
        hideFind();
        focusEditor();
      }
      return;
    default:
      return; // printable input flows through beforeinput / composition
  }
}

export function onBeforeInput(e) {
  if (state.composing) return; // composition text is committed on compositionend
  if (anyModalOpen()) {
    e.preventDefault();
    return;
  }
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

export function onPaste(e) {
  const text = (e.clipboardData || window.clipboardData)?.getData("text") ?? "";
  e.preventDefault();
  if (text) pasteText(text);
}

export function onCompStart() {
  state.composing = true;
  $("hidden-input").classList.add("composing");
  positionCaret();
}

export function onCompUpdate() {
  positionCaret(); // the textarea itself renders the composing string
}

export function onCompEnd(e) {
  state.composing = false;
  const hi = $("hidden-input");
  hi.classList.remove("composing");
  const data = e.data || "";
  hi.value = "";
  // Don't commit composed text behind a modal or the busy overlay — a long
  // Replace All must not have IME input spliced into its edit queue (#72).
  if (anyModalOpen()) {
    scheduleRender();
    return;
  }
  if (data) typeText(data);
  else scheduleRender();
}

export function initEditor() {
  const hi = $("hidden-input");
  hi.addEventListener("keydown", onEditKey);
  hi.addEventListener("beforeinput", onBeforeInput);
  hi.addEventListener("input", () => {
    if (!state.composing) hi.value = "";
  });
  hi.addEventListener("paste", onPaste);
  hi.addEventListener("compositionstart", onCompStart);
  hi.addEventListener("compositionupdate", onCompUpdate);
  hi.addEventListener("compositionend", onCompEnd);
  hi.addEventListener("focus", () => {
    state.focused = true;
    scheduleRender();
  });
  hi.addEventListener("blur", () => {
    state.focused = false;
    scheduleRender();
  });
  // Keep the caret glued to its cell during horizontal scroll.
  $("content").addEventListener("scroll", positionCaret);
}
