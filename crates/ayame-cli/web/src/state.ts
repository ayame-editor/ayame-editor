// Ayame Editor — state module. Type-stripped to JS at build time (build.rs, oxc).
import type { ChangeHistoryResponse, LineRecord, SearchHit, StatResponse } from "./api.js";
import type { MessageKey } from "./i18n.js";
import { setLanguagePreferenceReader } from "./locale-preference.js";
import type { AnalysisMatcher } from "./analysis-model.js";
import type {
  AnalysisHit,
  AnalysisProfile,
  AnalysisStatus,
  BrowseEntry,
  TabInfo,
} from "./types/api.js";

export let LINE_HEIGHT = 18;
// tracks --lh-editor; updated by Settings (font size)
// LINE_HEIGHT is reassigned only through this setter so other modules can
// import it as a read-only live binding (Settings owns the write).
export function setLineHeight(v) {
  LINE_HEIGHT = v;
}

export const OVERSCAN = 6;

export const PAD = 400;
// extra lines fetched around the viewport and cached
export const SEARCH_HISTORY_KEY = "ayame.searchHistory.v1";

export const REPLACE_HISTORY_KEY = "ayame.replaceHistory.v1";

export const SETTINGS_KEY = "ayame.settings.v1";

export const SETTINGS_BG_IMAGE_KEY = "ayame.settings.bgImage.v1";

export const BROWSE_KEY = "ayame.browseDir.v1";

export const RECENT_KEY = "ayame.recentFiles.v1";

export const RECENT_MAX = 12;
// cap on 最近使ったファイル entries
export const ANALYSIS_PROFILES_KEY = "ayame.analysisProfiles.v1";

export const MAX_COPY_LINES = 20000;

// Creating a multi-selection allocates one small range/caret per bookmark.
// Keep that explicitly bounded; larger sets remain navigable/listable and can
// be streamed to a file without constructing DOM or selection objects.
export const MAX_BOOKMARK_SELECTIONS = 1000;

export const FONT_STACKS = {
  mono: '"SFMono-Regular","Menlo","Consolas","DejaVu Sans Mono","Noto Sans Mono CJK JP","MS Gothic",monospace',
  "mono-jp": '"Consolas","Menlo","Noto Sans Mono CJK JP","MS Gothic",monospace',
  system: '"Segoe UI","Hiragino Kaku Gothic ProN","Noto Sans JP",system-ui,sans-serif',
};

export type ThemeConfig = Record<string, any>;

export interface Settings {
  theme: string;
  font: string;
  fontSize: number;
  ruler: boolean;
  lineNumberCommas: boolean;
  confirmLastTabClose: boolean;
  restoreSession: boolean;
  updateCheckOnStartup: boolean;
  showWhitespace: boolean;
  syntaxHighlight: boolean;
  showChangeHistory: boolean;
  minimap: boolean;
  zenkakuUnderline: boolean;
  wordWrap: boolean;
  bgMode: string;
  bgImage: string | null;
  bgImageName: string;
  illus: number | null;
  language: string;
  keymap: Record<string, string | string[]>;
  customThemes: Record<string, ThemeConfig>;
  memoName: string;
  lastSaveDir: string;
}

export const DEFAULT_SETTINGS: Settings = {
  // "auto" follows the OS light/dark preference until the user picks a theme
  // explicitly (#153); any concrete id (iris-light, dark, …) then wins.
  theme: "auto",
  font: "mono",
  fontSize: 13,
  ruler: true,
  lineNumberCommas: true,
  confirmLastTabClose: true,
  restoreSession: true,
  updateCheckOnStartup: true,
  showWhitespace: false,
  syntaxHighlight: true,
  showChangeHistory: true,
  minimap: true,
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
export const KEYMAP_ACTIONS: [string, MessageKey, string | string[]][] = [
  ["newFile", "menu.newFile", "Ctrl+N"],
  ["newWindow", "menu.newWindow", "Ctrl+Shift+N"],
  ["openFile", "menu.open", "Ctrl+O"],
  ["saveFile", "menu.save", "Ctrl+S"],
  ["saveAs", "menu.saveAs", "Ctrl+Shift+S"],
  // Alt+W is the whole-word search toggle (see `searchWord`); keep it off the
  // tab-close default so the find-bar shortcut is reachable (issue #70).
  ["closeTab", "tab.close", "Ctrl+W"],
  ["closeTabsToRight", "tab.closeToRight", ""],
  ["closeAllTabs", "tab.closeAll", ""],
  ["closeSavedTabs", "tab.closeSaved", ""],
  ["reopenClosedTab", "tab.reopenClosed", "Ctrl+Shift+T"],
  // Ctrl+Tab is unreliable across WebViews, so page-based defaults (issue #79).
  ["nextTab", "keymap.nextTab", "Ctrl+PageDown"],
  ["prevTab", "keymap.prevTab", "Ctrl+PageUp"],
  ["commandPalette", "menu.commandPalette", "Ctrl+Shift+P"],
  ["find", "menu.find", "Ctrl+F"],
  ["replace", "menu.replace", "Ctrl+H"],
  ["findNext", "find.next", "F3"],
  ["findPrev", "find.prev", "Shift+F3"],
  ["gotoLine", "menu.gotoLine", "Ctrl+G"],
  ["toggleBookmark", "bookmark.toggle", "Ctrl+F2"],
  ["nextBookmark", "bookmark.next", "F2"],
  ["previousBookmark", "bookmark.previous", "Shift+F2"],
  ["showBookmarks", "bookmark.showList", "Ctrl+Shift+F2"],
  ["bookmarkMatches", "bookmark.addMatches", ""],
  ["saveBookmarks", "bookmark.save", ""],
  ["selectBookmarks", "bookmark.selectAll", ""],
  ["clearBookmarks", "bookmark.clear", "Alt+F2"],
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
  ["paste", "menu.paste", "Ctrl+V"],
  // Which physical key types "+" or "-" depends on the layout, so each zoom
  // step lists the spellings a keyboard can produce; matching also ignores the
  // Shift a layout needs for them (see `eventShortcuts`).
  ["zoomIn", "keymap.zoomIn", ["Ctrl++", "Ctrl+="]],
  ["zoomOut", "keymap.zoomOut", ["Ctrl+-", "Ctrl+_"]],
  ["zoomReset", "keymap.zoomReset", "Ctrl+0"],
  ["searchCase", "keymap.searchCase", "Alt+C"],
  ["searchWord", "keymap.searchWord", "Alt+W"],
  ["searchRegex", "keymap.searchRegex", "Alt+R"],
  ["sortSave", "menu.sort", "Ctrl+Alt+S"],
  ["splitFile", "menu.split", "Ctrl+Alt+P"],
  ["grepFolder", "menu.grep", "Ctrl+Shift+F"],
  ["grepSave", "menu.grepSave", "Ctrl+Alt+G"],
  ["analysisRules", "analysis.title", "Ctrl+Shift+L"],
  ["analysisNext", "analysis.next", ""],
  ["analysisPrevious", "analysis.previous", ""],
  ["analysisCancel", "analysis.cancel", ""],
  ["caseUpper", "menu.caseUpper", ""],
  ["caseLower", "menu.caseLower", ""],
  ["caseCamel", "menu.caseCamel", ""],
  ["casePascal", "menu.casePascal", ""],
  ["caseSnake", "menu.caseSnake", ""],
  ["caseKebab", "menu.caseKebab", ""],
  ["caseConstant", "menu.caseConstant", ""],
  ["settings", "menu.settings", "Ctrl+,"],
  ["keymap", "keymap.title", ""],
];

export const DEFAULT_KEYMAP = Object.fromEntries(
  KEYMAP_ACTIONS.map(([id, _label, shortcut]) => [id, shortcut]),
);

export interface Point {
  line: number;
  col: number;
}

export interface Selection {
  anchor: Point;
  head: Point;
  rect?: boolean;
}

export interface ExtraCursor extends Point {
  sel?: Selection | null;
}

export interface DocumentStat {
  open: boolean;
  path?: string;
  bytes?: number;
  lines?: number;
  view_lines: number;
  encoding?: string;
  eol?: string;
  bom_bytes?: number;
  stride?: number;
  checkpoints?: number;
  index_bytes?: number;
  index_ms?: number;
  from_cache?: boolean;
  dirty: boolean;
  revision: number;
  inserted_lines: number;
  replaced_lines: number;
  deleted_lines: number;
  can_undo: boolean;
  can_redo: boolean;
  recoverable?: number;
  wal_error?: string;
}

export type OpenerMode = "open" | "save" | "file" | "folder";

export interface AppState {
  runtime: {
    windowId: string;
  };
  view: {
    total: number;
    first: number;
    fracAcc: number;
    cache: { start: number; lines: LineRecord[] };
    loadToken: number;
  };
  search: {
    query: string;
    regex: boolean;
    caseInsensitive: boolean;
    word: boolean;
    matcher: RegExp | null;
    regexError: boolean;
    matcherWordFallback: boolean;
    lastMatch: { byte: number; len: number } | null;
    hits: SearchHit[] | null;
    truncated: boolean;
    findOpen: boolean;
    replaceOpen: boolean;
    /// Replace-all writes only inside the current selection (#173).
    inSelection: boolean;
    history: string[];
    historyIndex: number;
    replaceHistory: string[];
    replaceHistoryIndex: number;
  };
  analysis: {
    profiles: AnalysisProfile[];
    activeProfile: string | null;
    operationId: string | null;
    status: AnalysisStatus | null;
    matchers: AnalysisMatcher[];
    visibleRuleIds: Set<string>;
    selectedRule: string | null;
    lastHits: Map<string, AnalysisHit>;
  };
  markers: {
    bookmarks: Set<number>;
    bookmarkCount: number;
    changeSaved: Set<number>;
    changeUnsaved: Set<number>;
    changeDeleted: Set<number>;
    changeHistoryOverview: ChangeHistoryResponse | null;
  };
  settings: Settings;
  doc: {
    stat: DocumentStat | null;
    tabs: TabInfo[];
    followTail: boolean;
    tailTimer: ReturnType<typeof setInterval> | null;
    generation: number;
  };
  opener: {
    dir: string | null;
    entries: BrowseEntry[];
  };
  caret: {
    position: Point;
    activeLine: number;
    goalCol: number;
    editGeneration: number;
    composing: boolean;
    focused: boolean;
    selection: Selection | null;
    extraCursors: ExtraCursor[];
    dragging: boolean;
    dragMoved: boolean;
    dragAnchor: Point | null;
    dragRect: boolean;
  };
}

export function createInitialState(): AppState {
  return {
    runtime: {
      windowId:
        typeof crypto !== "undefined" && typeof crypto.randomUUID === "function"
          ? crypto.randomUUID()
          : String(Date.now()) + "-" + Math.random().toString(36).slice(2),
    },
    view: {
      total: 0,
      first: 0, // top visible line (0-based)
      fracAcc: 0, // sub-line wheel accumulator
      cache: { start: 0, lines: [] },
      loadToken: 0,
    },
    search: {
      query: "",
      regex: false,
      caseInsensitive: false,
      word: false,
      matcher: null,
      regexError: false,
      matcherWordFallback: false,
      lastMatch: null, // { byte, len }
      hits: null,
      truncated: false,
      findOpen: false,
      replaceOpen: false,
      inSelection: false,
      history: [],
      historyIndex: -1,
      replaceHistory: [],
      replaceHistoryIndex: -1,
    },
    // Bounded log-analysis state. Profiles persist separately from find history;
    // status contains only fixed histograms and capped sparse positions.
    analysis: {
      profiles: [],
      activeProfile: null,
      operationId: null,
      status: null,
      matchers: [],
      visibleRuleIds: new Set<string>(),
      selectedRule: null,
      lastHits: new Map<string, AnalysisHit>(),
    },
    // Sparse marker cache for the same range as `view.cache.lines`. It is
    // replaced, not accumulated, on each viewport fetch so edits can never leave
    // stale line-number markers behind.
    markers: {
      bookmarks: new Set<number>(),
      bookmarkCount: 0,
      changeSaved: new Set<number>(),
      changeUnsaved: new Set<number>(),
      changeDeleted: new Set<number>(),
      changeHistoryOverview: null,
    },
    settings: {
      ...DEFAULT_SETTINGS,
      keymap: { ...DEFAULT_SETTINGS.keymap },
      customThemes: { ...DEFAULT_SETTINGS.customThemes },
    },
    doc: {
      stat: null as StatResponse | null,
      tabs: [], // open tabs from /api/tabs
      followTail: false, // 末尾に追従 (tail -f): poll for appended data and auto-scroll
      tailTimer: null, // setInterval handle while following; cleared when off
      generation: 0, // bumps whenever the active document/tab changes; cancels stale queued edits
    },
    opener: {
      dir: null, // directory currently shown in the open dialog
      entries: [],
    },
    // ---- caret-based (Notepad-style) editing ----
    caret: {
      position: { line: 0, col: 0 }, // Unicode scalar coordinates, like the backend
      activeLine: -1,
      goalCol: 0, // remembered column for vertical caret motion
      editGeneration: 0, // bumps on every user caret move; protects in-flight edits
      composing: false, // an IME composition is in progress
      focused: false, // the hidden text input holds focus (draw the caret)
      selection: null,
      extraCursors: [], // additional carets; primary is `caret.position`
      dragging: false,
      dragMoved: false,
      dragAnchor: null, // mouse-down caret, promoted to a selection once it moves
      dragRect: false,
    },
  };
}

export const state: AppState = createInitialState();
setLanguagePreferenceReader(() => state.settings.language);
