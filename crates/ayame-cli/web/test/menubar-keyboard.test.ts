import { beforeEach, describe, expect, it, vi } from "vitest";

// anyModalOpen (and the two search helpers other graph modules pull from input)
// touch many elements this minimal DOM lacks; stub them so the menubar logic
// under test runs in isolation.
vi.mock("../src/input.js", () => ({
  anyModalOpen: () => false,
  isWordChar: () => false,
  setQueryFromInput: () => {},
}));

import { focusMenubar, initMenuBar, renderFileMenuRecentFiles } from "../src/menus.js";
import { state } from "../src/state.js";

function shell(id: string, items: string) {
  return `
    <div class="menu-shell">
      <button id="${id}-menu-button" class="menubar-button" role="menuitem"
              aria-haspopup="true" aria-expanded="false">${id}</button>
      <div id="${id}-menu" class="file-menu hidden" role="menu">${items}</div>
    </div>`;
}
const item = (action: string, label: string) =>
  `<button class="menu-item" data-menu-action="${action}" role="menuitem">${label}</button>`;

function key(el: Element | null, k: string) {
  (el || document).dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true }));
}

describe("menubar keyboard navigation (#161)", () => {
  beforeEach(() => {
    const values = new Map<string, string>();
    vi.stubGlobal("localStorage", {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => values.set(key, value),
      removeItem: (key: string) => values.delete(key),
      clear: () => values.clear(),
      key: (index: number) => [...values.keys()][index] ?? null,
      get length() {
        return values.size;
      },
    } satisfies Storage);
    document.body.innerHTML = `
      <div id="app">
        <nav id="menubar" role="menubar">
          ${shell(
            "file",
            item("newFile", "New") +
              item("openFile", "Open") +
              `<div id="file-menu-recent-section" class="hidden">
                <div id="file-menu-recent-label">Recent Files</div>
                <div id="file-menu-recents" role="group"
                     aria-labelledby="file-menu-recent-label"></div>
              </div>`,
          )}
          ${shell(
            "edit",
            item("undo", "Undo") + item("redo", "Redo") + item("cut", "Cut") + item("copy", "Copy"),
          )}
          ${shell("selection", item("selectAll", "All"))}
          ${shell(
            "view",
            '<button class="menu-item" id="menu-toggle-ws" role="menuitemcheckbox">WS</button>' +
              '<button class="menu-item" id="menu-toggle-syntax" role="menuitemcheckbox">Syn</button>' +
              '<button class="menu-item" id="menu-toggle-zsp-underline" role="menuitemcheckbox">Z</button>' +
              '<button class="menu-item" id="menu-toggle-wrap" role="menuitemcheckbox">Wrap</button>' +
              '<button class="menu-item" id="menu-toggle-tail" role="menuitemcheckbox">Tail</button>',
          )}
          ${shell("help", item("help", "Help"))}
        </nav>
      </div>
      ${shell("tools", item("sortSave", "Sort"))}`;
    state.sel = null;
    state.extraCursors = [];
    initMenuBar();
  });

  const editItem = (action: string) =>
    document.querySelector<HTMLButtonElement>(`#edit-menu [data-menu-action="${action}"]`)!;

  it("disables Cut/Copy in the edit menu when there is no selection (#186)", () => {
    state.sel = null;
    const edit = document.getElementById("edit-menu-button")!;
    edit.focus();
    key(edit, "ArrowDown"); // opens the edit menu → showAppMenu("edit")
    expect(editItem("cut").disabled).toBe(true);
    expect(editItem("copy").disabled).toBe(true);
  });

  it("enables Cut/Copy in the edit menu when text is selected (#186)", () => {
    state.sel = { anchor: { line: 0, col: 0 }, head: { line: 0, col: 4 } } as never;
    const edit = document.getElementById("edit-menu-button")!;
    edit.focus();
    key(edit, "ArrowDown");
    expect(editItem("cut").disabled).toBe(false);
    expect(editItem("copy").disabled).toBe(false);
  });

  it("starts with a single roving Tab stop on the first trigger", () => {
    expect(document.getElementById("file-menu-button")!.getAttribute("tabindex")).toBe("0");
    expect(document.getElementById("edit-menu-button")!.getAttribute("tabindex")).toBe("-1");
  });

  it("moves focus and the Tab stop with ArrowRight/ArrowLeft, wrapping", () => {
    const file = document.getElementById("file-menu-button")!;
    file.focus();
    key(file, "ArrowRight");
    expect(document.activeElement!.id).toBe("edit-menu-button");
    expect(document.getElementById("edit-menu-button")!.getAttribute("tabindex")).toBe("0");
    expect(file.getAttribute("tabindex")).toBe("-1");
    // ArrowLeft from the first wraps to the last menubar trigger (help).
    file.focus();
    key(file, "ArrowLeft");
    expect(document.activeElement!.id).toBe("help-menu-button");
  });

  it("opens the menu and focuses the first item on ArrowDown", () => {
    const file = document.getElementById("file-menu-button")!;
    file.focus();
    key(file, "ArrowDown");
    expect(document.getElementById("file-menu")!.classList.contains("hidden")).toBe(false);
    expect(file.getAttribute("aria-expanded")).toBe("true");
    expect(document.activeElement!.textContent).toBe("New");
  });

  it("ArrowUp on a trigger opens the menu at its last item", () => {
    const file = document.getElementById("file-menu-button")!;
    file.focus();
    key(file, "ArrowUp");
    expect(document.activeElement!.textContent).toBe("Open");
  });

  it("cycles items with ArrowDown/ArrowUp inside an open menu", () => {
    const file = document.getElementById("file-menu-button")!;
    file.focus();
    key(file, "ArrowDown"); // focus "New"
    const menu = document.getElementById("file-menu")!;
    key(document.activeElement, "ArrowDown");
    expect(document.activeElement!.textContent).toBe("Open");
    key(document.activeElement, "ArrowDown"); // wrap
    expect(document.activeElement!.textContent).toBe("New");
    key(document.activeElement, "ArrowUp"); // wrap back up
    expect(document.activeElement!.textContent).toBe("Open");
    expect(menu).toBeTruthy();
  });

  it("closes the menu and restores the trigger on Escape", () => {
    const file = document.getElementById("file-menu-button")!;
    file.focus();
    key(file, "ArrowDown");
    key(document.activeElement, "Escape");
    expect(document.getElementById("file-menu")!.classList.contains("hidden")).toBe(true);
    expect(document.activeElement!.id).toBe("file-menu-button");
  });

  it("ArrowRight inside an open menu opens the adjacent menu", () => {
    const file = document.getElementById("file-menu-button")!;
    file.focus();
    key(file, "ArrowDown"); // open file, focus New
    key(document.activeElement, "ArrowRight");
    expect(document.getElementById("file-menu")!.classList.contains("hidden")).toBe(true);
    expect(document.getElementById("edit-menu")!.classList.contains("hidden")).toBe(false);
    expect(document.activeElement!.textContent).toBe("Undo");
  });

  it("F10 moves focus to the menubar", () => {
    document.getElementById("file-menu-button")!.blur();
    key(document.body, "F10");
    expect(document.activeElement!.id).toBe("file-menu-button");
  });

  it("renders recent paths as keyboard-navigable File menu items (#167)", () => {
    localStorage.setItem(
      "ayame.recentFiles.v1",
      JSON.stringify(["/work/alpha.txt", "/work/nested/beta.txt"]),
    );

    renderFileMenuRecentFiles();

    const section = document.getElementById("file-menu-recent-section")!;
    const rows = [...section.querySelectorAll<HTMLButtonElement>(".menu-item")];
    expect(section.classList.contains("hidden")).toBe(false);
    expect(rows.map((row) => row.querySelector(".menu-label")?.textContent)).toEqual([
      "alpha.txt",
      "beta.txt",
    ]);
    expect(rows[0].querySelector(".menu-recent-path")?.textContent).toBe("/work");
    expect(rows[0].title).toBe("/work/alpha.txt");
    expect(rows.every((row) => row.getAttribute("tabindex") === "-1")).toBe(true);
  });
});
