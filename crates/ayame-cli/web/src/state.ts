// Ayame Editor — state module. Type-stripped to JS at build time (build.rs, oxc).
export let LINE_HEIGHT = 18;
// tracks --line-height; updated by Settings (font size)
// LINE_HEIGHT is reassigned only through this setter so other modules can
// import it as a read-only live binding (Settings owns the write).
export function setLineHeight(v) {
  LINE_HEIGHT = v;
}

export const OVERSCAN = 6;

export const PAD = 400;
// extra lines fetched around the viewport and cached
export const SEARCH_HISTORY_KEY = "ayame.searchHistory.v1";

export const SETTINGS_KEY = "ayame.settings.v1";

export const BROWSE_KEY = "ayame.browseDir.v1";

export const RECENT_KEY = "ayame.recentFiles.v1";

export const RECENT_MAX = 12;
// cap on 最近使ったファイル entries
export const MAX_COPY_LINES = 20000;

export const FONT_STACKS = {
  mono: '"SFMono-Regular","Menlo","Consolas","DejaVu Sans Mono","Noto Sans Mono CJK JP","MS Gothic",monospace',
  "mono-jp": '"Consolas","Menlo","Noto Sans Mono CJK JP","MS Gothic",monospace',
  system: '"Segoe UI","Hiragino Kaku Gothic ProN","Noto Sans JP",system-ui,sans-serif',
};

export const DEFAULT_SETTINGS = {
  theme: "iris-light",
  font: "mono",
  fontSize: 13,
  zoom: 100,
  ruler: true,
  lineNumberCommas: true,
  confirmLastTabClose: true,
  restoreSession: true,
  updateCheckOnStartup: true,
  showWhitespace: false,
  syntaxHighlight: true,
  zenkakuUnderline: false,
  wordWrap: false,
  bgMode: "watercolor", // "watercolor" (theme default) | "solid" | "image"
  bgImage: null, // data: URL of the custom wallpaper when bgMode === "image"
  bgImageName: "", // its original file name, shown in the settings dialog
  illus: null,
  language: "auto",
  keymap: {},
  customThemes: {},
  // 新規ファイルを保存する際、保存ダイアログに提案する既定のファイル名テンプレート。
  memoName: "untitled-{seq}.txt",
  // 前回の保存先: last save-as folder, suggested for new untitled buffers.
  lastSaveDir: "",
};

// [action id, i18n label key, default shortcut(s)]
export const KEYMAP_ACTIONS: [string, string, string | string[]][] = [
  ["newFile", "menu.newFile", "Ctrl+N"],
  ["newWindow", "menu.newWindow", "Ctrl+Shift+N"],
  ["openFile", "menu.open", "Ctrl+O"],
  ["saveFile", "menu.save", "Ctrl+S"],
  ["saveAs", "menu.saveAs", "Ctrl+Shift+S"],
  // Alt+W is the whole-word search toggle (see `searchWord`); keep it off the
  // tab-close default so the find-bar shortcut is reachable (issue #70).
  ["closeTab", "tab.close", "Ctrl+W"],
  // Ctrl+Tab is unreliable across WebViews, so page-based defaults (issue #79).
  ["nextTab", "keymap.nextTab", "Ctrl+PageDown"],
  ["prevTab", "keymap.prevTab", "Ctrl+PageUp"],
  ["commandPalette", "menu.commandPalette", "Ctrl+Shift+P"],
  ["find", "menu.find", "Ctrl+F"],
  ["replace", "menu.replace", "Ctrl+H"],
  ["findNext", "find.next", "F3"],
  ["findPrev", "find.prev", "Shift+F3"],
  ["gotoLine", "menu.gotoLine", "Ctrl+G"],
  ["undo", "menu.undo", "Ctrl+Z"],
  ["redo", "menu.redo", ["Ctrl+Y", "Ctrl+Shift+Z"]],
  ["selectAll", "menu.selectAll", "Ctrl+A"],
  ["selectNextOccurrence", "menu.selectNextOccurrence", "Ctrl+D"],
  ["addCursorAbove", "menu.addCursorAbove", "Ctrl+Alt+ArrowUp"],
  ["addCursorBelow", "menu.addCursorBelow", "Ctrl+Alt+ArrowDown"],
  ["duplicateLine", "menu.duplicateLine", "Ctrl+Shift+D"],
  ["moveLineUp", "menu.moveLineUp", "Alt+ArrowUp"],
  ["moveLineDown", "menu.moveLineDown", "Alt+ArrowDown"],
  ["deleteLine", "menu.deleteLine", "Ctrl+Shift+K"],
  ["copy", "menu.copy", "Ctrl+C"],
  ["cut", "menu.cut", "Ctrl+X"],
  ["searchCase", "keymap.searchCase", "Alt+C"],
  ["searchWord", "keymap.searchWord", "Alt+W"],
  ["searchRegex", "keymap.searchRegex", "Alt+R"],
  ["sortSave", "menu.sort", ""],
  ["splitFile", "menu.split", ""],
  ["grepFolder", "menu.grep", ""],
  ["grepSave", "menu.grepSave", ""],
  ["caseUpper", "menu.caseUpper", ""],
  ["caseLower", "menu.caseLower", ""],
  ["caseCamel", "menu.caseCamel", ""],
  ["casePascal", "menu.casePascal", ""],
  ["caseSnake", "menu.caseSnake", ""],
  ["caseKebab", "menu.caseKebab", ""],
  ["caseConstant", "menu.caseConstant", ""],
  ["settings", "menu.settings", ""],
  ["keymap", "keymap.title", ""],
];

export const DEFAULT_KEYMAP = Object.fromEntries(
  KEYMAP_ACTIONS.map(([id, _label, shortcut]) => [id, shortcut]),
);

export const state = {
  windowId:
    typeof crypto !== "undefined" && typeof crypto.randomUUID === "function"
      ? crypto.randomUUID()
      : String(Date.now()) + "-" + Math.random().toString(36).slice(2),
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
  matcherWordFallback: false,
  activeLine: -1,
  lastMatch: null, // { byte, len }
  searchHits: null,
  searchTruncated: false,
  findOpen: false,
  replaceOpen: false,
  history: [],
  historyIndex: -1,
  settings: { ...DEFAULT_SETTINGS },
  tabs: [], // open tabs from /api/tabs
  followTail: false, // 末尾に追従 (tail -f): poll for appended data and auto-scroll
  tailTimer: null, // setInterval handle while following; cleared when off
  openerDir: null, // directory currently shown in the open dialog
  openerMode: "open", // "open" | "save" | "file" | "folder"
  openerEntries: [],
  openerResolve: null,
  // ---- caret-based (Notepad-style) editing ----
  caret: { line: 0, col: 0 }, // (line, col) in Unicode scalars, like the backend
  goalCol: 0, // remembered column for vertical caret motion
  editGen: 0, // bumps on every user caret move; lets an in-flight edit detect it
  docGen: 0, // bumps whenever the active document/tab changes; cancels stale queued edits
  composing: false, // an IME composition is in progress
  focused: false, // the hidden text input holds focus (draw the caret)
  sel: null, // selection: { anchor: {line,col}, head: {line,col}, rect?: bool } or null
  extraCursors: [], // multi-cursor: additional carets [{line,col}]; primary is state.caret
  dragging: false,
  dragMoved: false,
  dragAnchor: null, // caret at mouse-down, promoted to a selection once it moves
  dragRect: false,
};
