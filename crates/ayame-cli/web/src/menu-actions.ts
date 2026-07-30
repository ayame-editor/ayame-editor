// Ayame Editor — action registry and global shortcut dispatch.
import { $ } from "./dom.js";
import { state } from "./state.js";
import { t } from "./i18n.js";
import { openNewWindow } from "./app.js";
import {
  bookmarksToFile,
  grepToFile,
  saveAllTabs,
  saveCopy,
  saveFile,
  showConvert,
  sortSave,
  splitFile,
} from "./save.js";
import { scheduleRender, setSearchHits } from "./editor.js";
import {
  addCursorAbove,
  addCursorBelow,
  copySelection,
  cutSelection,
  pasteFromClipboard,
  selectAll,
  selectNextOccurrence,
} from "./selection.js";
import {
  deleteLines,
  duplicateLines,
  gotoLine,
  moveLines,
  redoEdit,
  setFollowTail,
  transformSelection,
  undoEdit,
} from "./edits.js";
import { buildMatcher, findStep, showFind, updateCount } from "./findbar.js";
import { grepFolder } from "./grep.js";
import { askPrompt, showMessage } from "./dialogs.js";
import { anyModalOpen } from "./modal-state.js";
import { eventShortcut } from "./keys.js";
import { closeTab, newUntitled, openFileDialog, selectRelativeTab } from "./workspace.js";
import { setSettingsMenuService, showSettings, updateSetting } from "./settings.js";
import {
  bookmarkSearchMatches,
  clearBookmarks,
  nextBookmark,
  previousBookmark,
  selectBookmarkedLines,
  showBookmarkList,
  toggleBookmark,
} from "./bookmarks.js";
import { hideFileMenu } from "./menu-surface.js";
import {
  hideKeymap,
  renderKeymapRows,
  resetKeymap,
  setShortcutMapRebuilder,
  shortcutList,
  showKeymap,
  updateKeyHints,
} from "./keymap-menu.js";
import { setPaletteActionRunner, showCommandPalette } from "./palette.js";
import { applyLocale } from "./menu-ui.js";

const closeActiveTab = () => {
  const active = state.doc.tabs.find((tab) => tab.active);
  if (active) closeTab(active.id);
};

const promptGotoLine = () => {
  askPrompt(t("menu.gotoLine"), t("dialog.gotoLine.label")).then((value) => {
    if (value != null) gotoLine(value);
  });
};

const showHelp = () => showMessage(t("help.title"), t("help.body"));
const showAbout = () => showMessage(t("help.about"), t("help.aboutBody"));

const SEARCH_OPTION_KEYS = {
  ci: "caseInsensitive",
  word: "word",
  regex: "regex",
} as const;

type SearchOptionKey = keyof typeof SEARCH_OPTION_KEYS;

function optButtonLit(key: SearchOptionKey): boolean {
  const value = state.search[SEARCH_OPTION_KEYS[key]];
  return key === "ci" ? !value : value;
}

export function refreshFindOptButtons() {
  for (const [id, key] of [
    ["opt-case", "ci"],
    ["opt-word", "word"],
    ["opt-regex", "regex"],
  ] as const) {
    const pressed = optButtonLit(key);
    $(id).classList.toggle("on", pressed);
    $(id).setAttribute("aria-pressed", String(pressed));
  }
}

export function toggleOpt(key: SearchOptionKey, id: string) {
  const property = SEARCH_OPTION_KEYS[key];
  state.search[property] = !state.search[property];
  const pressed = optButtonLit(key);
  $(id).classList.toggle("on", pressed);
  $(id).setAttribute("aria-pressed", String(pressed));
  state.search.lastMatch = null;
  setSearchHits(null);
  state.search.truncated = false;
  buildMatcher();
  scheduleRender();
  if (state.search.query) updateCount();
}

export const ACTIONS: Record<
  string,
  { run: () => any; globalShortcut?: boolean; editorOnly?: boolean }
> = {
  commandPalette: { run: showCommandPalette, globalShortcut: true },
  undo: { run: undoEdit, globalShortcut: true, editorOnly: true },
  redo: { run: redoEdit, globalShortcut: true, editorOnly: true },
  find: { run: () => showFind(), globalShortcut: true },
  replace: { run: () => showFind(true), globalShortcut: true },
  gotoLine: { run: promptGotoLine, globalShortcut: true },
  toggleBookmark: { run: () => toggleBookmark(), globalShortcut: true, editorOnly: true },
  nextBookmark: { run: nextBookmark, globalShortcut: true, editorOnly: true },
  previousBookmark: { run: previousBookmark, globalShortcut: true, editorOnly: true },
  showBookmarks: { run: showBookmarkList, globalShortcut: true, editorOnly: true },
  bookmarkMatches: { run: bookmarkSearchMatches, globalShortcut: true, editorOnly: true },
  saveBookmarks: { run: bookmarksToFile, globalShortcut: true, editorOnly: true },
  selectBookmarks: { run: selectBookmarkedLines, globalShortcut: true, editorOnly: true },
  clearBookmarks: { run: clearBookmarks, globalShortcut: true, editorOnly: true },
  selectAll: { run: selectAll, globalShortcut: true, editorOnly: true },
  selectNextOccurrence: { run: selectNextOccurrence },
  addCursorAbove: { run: addCursorAbove },
  addCursorBelow: { run: addCursorBelow },
  duplicateLine: { run: duplicateLines },
  moveLineUp: { run: () => moveLines(-1) },
  moveLineDown: { run: () => moveLines(1) },
  deleteLine: { run: deleteLines },
  copy: { run: copySelection, globalShortcut: true, editorOnly: true },
  cut: { run: cutSelection, globalShortcut: true, editorOnly: true },
  paste: { run: pasteFromClipboard, editorOnly: true },
  toggleWhitespace: {
    run: () => updateSetting("showWhitespace", !state.settings.showWhitespace),
  },
  toggleSyntaxHighlight: {
    run: () => updateSetting("syntaxHighlight", state.settings.syntaxHighlight === false),
  },
  toggleChangeHistory: {
    run: () => updateSetting("showChangeHistory", state.settings.showChangeHistory === false),
  },
  toggleMinimap: {
    run: () => updateSetting("minimap", state.settings.minimap === false),
  },
  toggleZenkakuUnderline: {
    run: () => updateSetting("zenkakuUnderline", !state.settings.zenkakuUnderline),
  },
  toggleWordWrap: { run: () => updateSetting("wordWrap", !state.settings.wordWrap) },
  toggleFollowTail: { run: () => setFollowTail(!state.doc.followTail) },
  nextTab: { run: () => selectRelativeTab(1), globalShortcut: true },
  prevTab: { run: () => selectRelativeTab(-1), globalShortcut: true },
  settings: { run: showSettings, globalShortcut: true },
  help: { run: showHelp, globalShortcut: true },
  about: { run: showAbout, globalShortcut: true },
  sortSave: { run: sortSave, globalShortcut: true },
  splitFile: { run: splitFile, globalShortcut: true },
  grepFolder: { run: grepFolder, globalShortcut: true },
  grepSave: { run: grepToFile, globalShortcut: true },
  analysisRules: {
    run: () => import("./analysis.js").then(({ openAnalysis }) => openAnalysis()),
    globalShortcut: true,
  },
  analysisNext: {
    run: () => import("./analysis.js").then(({ nextAnalysisMatch }) => nextAnalysisMatch()),
    globalShortcut: true,
    editorOnly: true,
  },
  analysisPrevious: {
    run: () => import("./analysis.js").then(({ previousAnalysisMatch }) => previousAnalysisMatch()),
    globalShortcut: true,
    editorOnly: true,
  },
  analysisCancel: {
    run: () => import("./analysis.js").then(({ cancelAnalysis }) => cancelAnalysis()),
    globalShortcut: true,
  },
  caseUpper: { run: () => transformSelection("upper"), globalShortcut: true, editorOnly: true },
  caseLower: { run: () => transformSelection("lower"), globalShortcut: true, editorOnly: true },
  caseCamel: { run: () => transformSelection("camel"), globalShortcut: true, editorOnly: true },
  casePascal: { run: () => transformSelection("pascal"), globalShortcut: true, editorOnly: true },
  caseSnake: { run: () => transformSelection("snake"), globalShortcut: true, editorOnly: true },
  caseKebab: { run: () => transformSelection("kebab"), globalShortcut: true, editorOnly: true },
  caseConstant: {
    run: () => transformSelection("constant"),
    globalShortcut: true,
    editorOnly: true,
  },
  keymap: { run: showKeymap, globalShortcut: true },
  newFile: { run: newUntitled, globalShortcut: true },
  newWindow: { run: openNewWindow, globalShortcut: true },
  openFile: { run: openFileDialog, globalShortcut: true },
  saveFile: { run: saveFile, globalShortcut: true },
  saveAs: { run: saveCopy, globalShortcut: true },
  saveAll: { run: saveAllTabs, globalShortcut: true },
  encoding: { run: showConvert },
  closeTab: { run: closeActiveTab, globalShortcut: true },
  findPrev: { run: () => findStep("prev"), globalShortcut: true },
  findNext: { run: () => findStep("next"), globalShortcut: true },
  searchCase: { run: () => toggleOpt("ci", "opt-case"), globalShortcut: true },
  searchRegex: { run: () => toggleOpt("regex", "opt-regex"), globalShortcut: true },
  searchWord: { run: () => toggleOpt("word", "opt-word"), globalShortcut: true },
};

export function runAction(action) {
  return ACTIONS[action]?.run();
}

const GLOBAL_SHORTCUT_ACTIONS = [
  "commandPalette",
  "openFile",
  "newFile",
  "newWindow",
  "gotoLine",
  "toggleBookmark",
  "nextBookmark",
  "previousBookmark",
  "showBookmarks",
  "bookmarkMatches",
  "saveBookmarks",
  "selectBookmarks",
  "clearBookmarks",
  "closeTab",
  "nextTab",
  "prevTab",
  "find",
  "replace",
  "saveAs",
  "saveFile",
  "findPrev",
  "findNext",
  "searchCase",
  "searchRegex",
  "searchWord",
  "sortSave",
  "splitFile",
  "grepFolder",
  "grepSave",
  "analysisRules",
  "analysisNext",
  "analysisPrevious",
  "analysisCancel",
  "settings",
  "keymap",
  "selectAll",
  "copy",
  "cut",
  "caseUpper",
  "caseLower",
  "caseCamel",
  "casePascal",
  "caseSnake",
  "caseKebab",
  "caseConstant",
  "redo",
  "undo",
];

let globalShortcutActionsByKey = new Map<string, string[]>();

export function rebuildGlobalShortcutActions() {
  const next = new Map<string, string[]>();
  for (const action of GLOBAL_SHORTCUT_ACTIONS) {
    for (const shortcut of shortcutList(action)) {
      const actions = next.get(shortcut) || [];
      if (!actions.includes(action)) actions.push(action);
      next.set(shortcut, actions);
    }
  }
  globalShortcutActionsByKey = next;
}

export function shortcutActionFromEvent(e, inField = false) {
  const shortcut = eventShortcut(e);
  if (!shortcut) return null;
  if (!globalShortcutActionsByKey.size) rebuildGlobalShortcutActions();
  for (const action of globalShortcutActionsByKey.get(shortcut) || []) {
    const entry = ACTIONS[action];
    if (!entry?.globalShortcut || (inField && entry.editorOnly)) continue;
    return action;
  }
  return null;
}

export function runMenuAction(action) {
  hideFileMenu();
  if (anyModalOpen()) return;
  return runAction(action);
}

let menusInitialized = false;

export function initMenus() {
  if (menusInitialized) return;
  menusInitialized = true;
  setShortcutMapRebuilder(rebuildGlobalShortcutActions);
  setPaletteActionRunner(runMenuAction);
  window.__ayameMenu = runMenuAction;
  setSettingsMenuService({
    applyLocale,
    hideKeymap,
    renderKeymapRows,
    resetKeymap,
    showKeymap,
    updateKeyHints,
  });
}
