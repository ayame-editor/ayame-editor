import { beforeEach, describe, expect, it, vi } from "vitest";

// anyModalOpen (and the two search helpers other graph modules pull from input)
// touch many elements this minimal DOM lacks; stub them so the menubar logic
// under test runs in isolation.
vi.mock("../src/input.js", () => ({
  anyModalOpen: () => false,
  isWordChar: () => false,
  setQueryFromInput: () => {},
}));

import { focusMenubar, initMenuBar } from "../src/menus.js";

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
    document.body.innerHTML = `
      <div id="app">
        <nav id="menubar" role="menubar">
          ${shell("file", item("newFile", "New") + item("openFile", "Open"))}
          ${shell("edit", item("undo", "Undo") + item("redo", "Redo"))}
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
    initMenuBar();
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
});
