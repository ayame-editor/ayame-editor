import { beforeEach, describe, expect, it, vi } from "vitest";

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
  diffVisible: vi.fn(() => false),
  findStep: vi.fn(),
  flashCount: vi.fn(),
  grepVisible: vi.fn(() => false),
  hideDiff: vi.fn(),
  hideFind: vi.fn(),
  hideGrep: vi.fn(),
  replaceAll: vi.fn(),
  replaceCurrent: vi.fn(),
  selectNextOccurrence: vi.fn(),
  setReplaceRow: vi.fn(),
  showSearchHistory: vi.fn(() => false),
  updateCount: vi.fn(),
}));
vi.mock("../src/dialogs.js", () => ({
  confirmVisible: vi.fn(() => false),
  formVisible: vi.fn(() => false),
  promptVisible: vi.fn(() => false),
  loadingVisible: vi.fn(() => false),
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
import { onCompEnd, onEditKey } from "../src/input.js";
import { hideFind } from "../src/search.js";
import { state } from "../src/state.js";

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
