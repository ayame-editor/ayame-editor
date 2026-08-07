import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../src/i18n.js", () => ({ t: (key: string) => key }));
vi.mock("../src/app.js", () => ({ openNewWindow: vi.fn() }));
vi.mock("../src/save.js", () => ({
  bookmarksToFile: vi.fn(),
  grepToFile: vi.fn(),
  saveAllTabs: vi.fn(),
  saveCopy: vi.fn(),
  saveFile: vi.fn(),
  showConvert: vi.fn(),
  sortSave: vi.fn(),
  splitFile: vi.fn(),
}));
vi.mock("../src/editor.js", () => ({ scheduleRender: vi.fn(), setSearchHits: vi.fn() }));
vi.mock("../src/selection.js", () => ({
  addCursorAbove: vi.fn(),
  addCursorBelow: vi.fn(),
  copySelection: vi.fn(),
  cutSelection: vi.fn(),
  pasteFromClipboard: vi.fn(),
  selectAll: vi.fn(),
  selectNextOccurrence: vi.fn(),
}));
vi.mock("../src/edits.js", () => ({
  deleteLines: vi.fn(),
  duplicateLines: vi.fn(),
  gotoLine: vi.fn(),
  moveLines: vi.fn(),
  redoEdit: vi.fn(),
  setFollowTail: vi.fn(),
  transformSelection: vi.fn(),
  undoEdit: vi.fn(),
}));
vi.mock("../src/findbar.js", () => ({
  buildMatcher: vi.fn(),
  findStep: vi.fn(),
  showFind: vi.fn(),
  updateCount: vi.fn(),
}));
vi.mock("../src/grep.js", () => ({ grepFolder: vi.fn() }));
vi.mock("../src/dialogs.js", () => ({ askPrompt: vi.fn(), showMessage: vi.fn() }));
vi.mock("../src/modal-state.js", () => ({ anyModalOpen: vi.fn(() => false) }));
vi.mock("../src/keys.js", () => ({ eventShortcuts: vi.fn() }));
vi.mock("../src/workspace.js", () => ({
  closeAllTabs: vi.fn(),
  closeSavedTabs: vi.fn(),
  closeTab: vi.fn(),
  closeTabsToRight: vi.fn(),
  newUntitled: vi.fn(),
  openFileDialog: vi.fn(),
  reopenClosedTab: vi.fn(),
  selectRelativeTab: vi.fn(),
}));
vi.mock("../src/settings.js", () => ({
  adjustFontSize: vi.fn(),
  FONT_SIZE_STEP: 1,
  setFontSize: vi.fn(),
  setSettingsMenuService: vi.fn(),
  showSettings: vi.fn(),
  updateSetting: vi.fn(),
}));
vi.mock("../src/bookmarks.js", () => ({
  bookmarkSearchMatches: vi.fn(),
  clearBookmarks: vi.fn(),
  nextBookmark: vi.fn(),
  previousBookmark: vi.fn(),
  selectBookmarkedLines: vi.fn(),
  showBookmarkList: vi.fn(),
  toggleBookmark: vi.fn(),
}));
vi.mock("../src/keymap-menu.js", () => ({
  hideKeymap: vi.fn(),
  renderKeymapRows: vi.fn(),
  resetKeymap: vi.fn(),
  setShortcutMapRebuilder: vi.fn(),
  shortcutList: vi.fn(() => []),
  showKeymap: vi.fn(),
  updateKeyHints: vi.fn(),
}));
vi.mock("../src/palette.js", () => ({
  setPaletteActionRunner: vi.fn(),
  showCommandPalette: vi.fn(),
}));
vi.mock("../src/menu-ui.js", () => ({ applyLocale: vi.fn(), showAppMenu: vi.fn() }));

import { moveLines, transformSelection } from "../src/edits.js";
import { eventShortcuts } from "../src/keys.js";
import { shortcutList } from "../src/keymap-menu.js";
import {
  rebuildGlobalShortcutActions,
  runAction,
  runMenuAction,
  shortcutActionFromEvent,
} from "../src/menu-actions.js";
import { APP_MENUS, DROPDOWN_MENUS } from "../src/menu-surface.js";
import { initMenuBar } from "../src/menubar.js";
import { saveFile } from "../src/save.js";
import { anyModalOpen } from "../src/modal-state.js";
import { newUntitled, selectRelativeTab } from "../src/workspace.js";

function menuShell(id: string, actions: string[] = []) {
  return `
    <button id="${id}-menu-button" aria-expanded="false">${id}</button>
    <div id="${id}-menu" class="hidden">
      ${actions
        .map(
          (action) =>
            `<button id="action-${action}" class="menu-item" data-menu-action="${action}">${action}</button>`,
        )
        .join("")}
    </div>`;
}

function installMenuDom(actions: string[] = []) {
  const grouped = new Map(DROPDOWN_MENUS.map((id) => [id, [] as string[]]));
  actions.forEach((action, index) =>
    grouped.get(DROPDOWN_MENUS[index % DROPDOWN_MENUS.length])!.push(action),
  );
  document.body.innerHTML = `
    <nav id="menubar">${APP_MENUS.map((id) => menuShell(id, grouped.get(id))).join("")}</nav>
    ${menuShell("tools", grouped.get("tools"))}`;
}

describe("menu action dispatch (#188)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(anyModalOpen).mockReturnValue(false);
    document.body.innerHTML = "";
  });

  it("dispatches data-menu-action values through the real action registry", () => {
    installMenuDom(["newFile", "saveFile", "moveLineUp", "caseSnake", "nextTab"]);
    initMenuBar();

    for (const action of ["newFile", "saveFile", "moveLineUp", "caseSnake", "nextTab"]) {
      document.getElementById(`action-${action}`)!.click();
    }

    expect(newUntitled).toHaveBeenCalledOnce();
    expect(saveFile).toHaveBeenCalledOnce();
    expect(moveLines).toHaveBeenCalledOnce();
    expect(moveLines).toHaveBeenCalledWith(-1);
    expect(transformSelection).toHaveBeenCalledOnce();
    expect(transformSelection).toHaveBeenCalledWith("snake");
    expect(selectRelativeTab).toHaveBeenCalledOnce();
    expect(selectRelativeTab).toHaveBeenCalledWith(1);
  });

  it("closes dropdowns but does not run actions while a modal is open", () => {
    installMenuDom(["newFile"]);
    for (const id of DROPDOWN_MENUS) {
      document.getElementById(`${id}-menu`)!.classList.remove("hidden");
    }
    vi.mocked(anyModalOpen).mockReturnValue(true);

    expect(runMenuAction("newFile")).toBeUndefined();

    expect(newUntitled).not.toHaveBeenCalled();
    expect(
      DROPDOWN_MENUS.every((id) =>
        document.getElementById(`${id}-menu`)!.classList.contains("hidden"),
      ),
    ).toBe(true);
  });

  it.each([
    ["moveLineDown", moveLines, 1],
    ["caseConstant", transformSelection, "constant"],
    ["prevTab", selectRelativeTab, -1],
  ])("maps %s to its parameterized operation", (action, operation, argument) => {
    runAction(action);
    expect(operation).toHaveBeenCalledWith(argument);
  });

  it("honors editor-only shortcuts without blocking global field shortcuts", () => {
    vi.mocked(shortcutList).mockImplementation((action) => {
      if (action === "copy") return ["Ctrl+C"];
      if (action === "saveFile") return ["Ctrl+S"];
      return [];
    });
    rebuildGlobalShortcutActions();

    vi.mocked(eventShortcuts).mockReturnValue(["Ctrl+C"]);
    expect(shortcutActionFromEvent(new KeyboardEvent("keydown"))).toBe("copy");
    expect(shortcutActionFromEvent(new KeyboardEvent("keydown"), true)).toBeNull();

    vi.mocked(eventShortcuts).mockReturnValue(["Ctrl+S"]);
    expect(shortcutActionFromEvent(new KeyboardEvent("keydown"), true)).toBe("saveFile");
  });
});
