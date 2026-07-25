import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../src/save.js", () => ({
  convertSave: vi.fn(),
  convertVisible: vi.fn(() => false),
  hideConvert: vi.fn(),
  reopenWithEncoding: vi.fn(),
  saveCopy: vi.fn(),
  saveFile: vi.fn(),
  savingCount: 0,
  showConvert: vi.fn(),
  syncConvertBom: vi.fn(),
}));
vi.mock("../src/editor.js", () => ({
  focusEditor: vi.fn(),
  lineChars: vi.fn(() => []),
  lineLen: vi.fn(() => 0),
  moveCaret: vi.fn(),
  positionCaret: vi.fn(),
  rowsVisible: vi.fn(() => 10),
  scheduleRender: vi.fn(),
  setFirst: vi.fn(),
}));
vi.mock("../src/selection.js", () => ({
  addCursorAbove: vi.fn(),
  addCursorBelow: vi.fn(),
  caretToDocEnd: vi.fn(),
  clearExtraCursors: vi.fn(),
}));
vi.mock("../src/menus.js", () => ({
  commandPaletteVisible: vi.fn(() => false),
  ctxMenuVisible: vi.fn(() => false),
  fileMenuVisible: vi.fn(() => false),
  hideCommandPalette: vi.fn(),
  hideCtxMenu: vi.fn(),
  hideFileMenu: vi.fn(),
  hideKeymap: vi.fn(),
  keymapVisible: vi.fn(() => false),
  matchesShortcut: vi.fn(() => false),
  runAction: vi.fn(),
  shortcutActionFromEvent: vi.fn(() => null),
  toggleOpt: vi.fn(),
}));
vi.mock("../src/edits.js", () => ({
  applyRange: vi.fn(),
  backspace: vi.fn(),
  deleteLines: vi.fn(),
  deleteSelectionEdit: vi.fn(),
  duplicateLines: vi.fn(),
  enqueueEdit: vi.fn(),
  forwardDelete: vi.fn(),
  insertNewline: vi.fn(),
  moveLines: vi.fn(),
  pasteText: vi.fn(),
  redoEdit: vi.fn(),
  setFollowTail: vi.fn(),
  typeText: vi.fn(),
  undoEdit: vi.fn(),
}));
vi.mock("../src/search.js", () => ({
  buildMatcher: vi.fn(),
  findStep: vi.fn(),
  grepVisible: vi.fn(() => false),
  hideFind: vi.fn(),
  hideGrep: vi.fn(),
  replaceAll: vi.fn(),
  replaceCurrent: vi.fn(),
  selectNextOccurrence: vi.fn(),
  setQueryFromInput: vi.fn(),
  setReplaceRow: vi.fn(),
  showSearchHistory: vi.fn(() => false),
  updateCount: vi.fn(),
}));
vi.mock("../src/notifications.js", () => ({ flashCount: vi.fn() }));
vi.mock("../src/dialogs.js", () => ({
  confirmVisible: vi.fn(() => false),
  formVisible: vi.fn(() => false),
  promptVisible: vi.fn(() => false),
  loadingVisible: vi.fn(() => false),
  loadingCancelable: vi.fn(() => false),
  cancelLoading: vi.fn(() => {}),
}));
vi.mock("../src/workspace.js", () => ({
  hideOpener: vi.fn(),
  openerVisible: vi.fn(() => false),
}));
vi.mock("../src/settings.js", () => ({
  applyKeymapFromBuffer: vi.fn(),
  applyThemeFromBuffer: vi.fn(),
  hideSettings: vi.fn(),
  settingsVisible: vi.fn(() => false),
}));

import { focusEditor } from "../src/editor.js";
import { insertNewline } from "../src/edits.js";
import {
  anyModalOpen,
  onCompEnd,
  onConvertModalKey,
  onEditKey,
  onGlobalKey,
} from "../src/input.js";
import { hideFind } from "../src/search.js";
import { state } from "../src/state.js";

describe("bookmark modal keyboard ownership (#241)", () => {
  beforeEach(() => {
    document.body.innerHTML = `
      <div id="bookmark-modal">
        <button id="bookmark-close"></button>
      </div>`;
    vi.clearAllMocks();
  });

  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("blocks editor shortcuts and closes from the global Escape path", () => {
    expect(anyModalOpen()).toBe(true);
    const close = vi.fn();
    document.getElementById("bookmark-close")!.addEventListener("click", close);
    const event = {
      key: "Escape",
      target: document.body,
      preventDefault: vi.fn(),
    };

    onGlobalKey(event);

    expect(event.preventDefault).toHaveBeenCalledOnce();
    expect(close).toHaveBeenCalledOnce();
  });
});

describe("editor Escape handling", () => {
  beforeEach(() => {
    state.composing = false;
    state.findOpen = true;
    state.sel = null;
    state.extraCursors = [];
    state.stat = { open: true };
    vi.clearAllMocks();
  });

  it("closes the find bar even when the hidden editor input has focus", () => {
    const event = {
      key: "Escape",
      isComposing: false,
      ctrlKey: false,
      metaKey: false,
      altKey: false,
      shiftKey: false,
      preventDefault: vi.fn(),
      stopPropagation: vi.fn(),
    };

    onEditKey(event);

    expect(event.preventDefault).toHaveBeenCalledOnce();
    expect(event.stopPropagation).toHaveBeenCalledOnce();
    expect(hideFind).toHaveBeenCalledOnce();
    expect(focusEditor).toHaveBeenCalledOnce();
  });
});

describe("IME confirm-Enter (WebKit) handling", () => {
  beforeEach(() => {
    document.body.innerHTML = '<textarea id="hidden-input"></textarea>';
    state.composing = false;
    state.findOpen = false;
    state.sel = null;
    state.extraCursors = [];
    state.stat = { open: true };
    state.caret = { line: 0, col: 0 };
    vi.clearAllMocks();
  });

  const enterEvent = (timeStamp) => ({
    key: "Enter",
    isComposing: false,
    ctrlKey: false,
    metaKey: false,
    altKey: false,
    shiftKey: false,
    timeStamp,
    preventDefault: vi.fn(),
    stopPropagation: vi.fn(),
  });

  it("swallows the Enter that arrives right after compositionend", () => {
    onCompEnd({ data: "あ", timeStamp: 1000 });
    const enter = enterEvent(1050); // 50ms later — the WebKit artifact
    onEditKey(enter);
    expect(enter.preventDefault).toHaveBeenCalledOnce();
    expect(insertNewline).not.toHaveBeenCalled();
  });

  it("keeps a deliberate Enter pressed well after the composition", () => {
    onCompEnd({ data: "あ", timeStamp: 1000 });
    onEditKey(enterEvent(2000)); // 1s later — a real newline
    expect(insertNewline).toHaveBeenCalledOnce();
  });

  it("only swallows one Enter per composition", () => {
    onCompEnd({ data: "あ", timeStamp: 1000 });
    onEditKey(enterEvent(1010)); // consumed
    onEditKey(enterEvent(1020)); // next Enter is a real newline
    expect(insertNewline).toHaveBeenCalledOnce();
  });
});

describe("encoding dialog Enter-to-confirm (#169)", () => {
  beforeEach(() => {
    document.body.innerHTML = `
      <div id="convert-modal">
        <select id="convert-enc"><option value="utf-8">UTF-8</option></select>
        <button id="reopen-go"></button>
        <button id="convert-go" class="primary"></button>
      </div>`;
  });

  it("confirms Convert & Save when Enter is pressed from the encoding select", () => {
    const convert = vi.fn();
    const reopen = vi.fn();
    document.getElementById("convert-go")!.addEventListener("click", convert);
    document.getElementById("reopen-go")!.addEventListener("click", reopen);
    document.getElementById("convert-enc")!.focus();
    const e = { key: "Enter", preventDefault: vi.fn() };
    onConvertModalKey(e);
    expect(e.preventDefault).toHaveBeenCalledOnce();
    expect(convert).toHaveBeenCalledOnce();
    expect(reopen).not.toHaveBeenCalled();
  });

  it("honors a focused Reopen button instead of the primary action", () => {
    const convert = vi.fn();
    const reopen = vi.fn();
    document.getElementById("convert-go")!.addEventListener("click", convert);
    document.getElementById("reopen-go")!.addEventListener("click", reopen);
    document.getElementById("reopen-go")!.focus();
    onConvertModalKey({ key: "Enter", preventDefault: vi.fn() });
    expect(reopen).toHaveBeenCalledOnce();
    expect(convert).not.toHaveBeenCalled();
  });

  it("ignores non-Enter keys", () => {
    const convert = vi.fn();
    document.getElementById("convert-go")!.addEventListener("click", convert);
    onConvertModalKey({ key: "a", preventDefault: vi.fn() });
    expect(convert).not.toHaveBeenCalled();
  });
});
