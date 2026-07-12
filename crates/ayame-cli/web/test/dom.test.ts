import { afterEach, describe, expect, it } from "vitest";

import {
  displayPath,
  displayShortcut,
  isAbsolutePath,
  isUntitled,
  joinPath,
  pathCrumbs,
  setModalOpen,
} from "../src/dom.js";

afterEach(() => {
  document.body.innerHTML = "";
});

describe("path helpers", () => {
  it("renders stored Ctrl shortcuts with macOS command glyphs", () => {
    expect(displayShortcut("Ctrl+Shift+ArrowUp", true)).toBe("⌘⇧↑");
    expect(displayShortcut("Ctrl+Alt+S", true)).toBe("⌘⌥S");
    expect(displayShortcut("Ctrl+S", false)).toBe("Ctrl+S");
  });

  it("normalizes Windows verbatim paths for display", () => {
    expect(displayPath("\\\\?\\C:\\logs\\app.txt")).toBe("C:\\logs\\app.txt");
    expect(displayPath("\\\\?\\UNC\\server\\share\\app.txt")).toBe("\\\\server\\share\\app.txt");
  });

  it("joins relative paths without changing absolute inputs", () => {
    expect(joinPath("/var/log", "app.log")).toBe("/var/log/app.log");
    expect(joinPath("C:\\logs", "app.log")).toBe("C:\\logs\\app.log");
    expect(joinPath("/var/log", "/tmp/app.log")).toBe("/tmp/app.log");
  });

  it("recognizes absolute Unix, drive, and UNC paths", () => {
    expect(isAbsolutePath("/tmp/a")).toBe(true);
    expect(isAbsolutePath("C:\\tmp\\a")).toBe(true);
    expect(isAbsolutePath("\\\\server\\share\\a")).toBe(true);
    expect(isAbsolutePath("tmp/a")).toBe(false);
  });

  it("builds clickable crumbs for Windows drives", () => {
    expect(pathCrumbs("C:\\Users\\me").map((c) => c.label)).toEqual(["C:", "Users", "me"]);
  });

  it("recognizes untitled scratch buffers in both dir-name generations", () => {
    // Current server scratch dirs ("srv-untitled") and the pre-rename form.
    expect(
      isUntitled(
        "C:\\Users\\x\\AppData\\Local\\Temp\\ayame-srv-untitled-55c647d-0-0\\untitled.txt",
      ),
    ).toBe(true);
    expect(isUntitled("/tmp/ayame-untitled-1234/untitled.txt")).toBe(true);
    expect(isUntitled("E:\\note\\untitled.txt")).toBe(false);
    expect(isUntitled("")).toBe(false);
  });
});

describe("modal focus management", () => {
  it("makes the app inert, traps Tab, and restores focus", async () => {
    document.body.innerHTML = `
      <main id="app"><button id="open">Open</button></main>
      <div id="dialog" class="modal hidden" aria-hidden="true">
        <button id="first">First</button><button id="last">Last</button>
      </div>`;
    const opener = document.getElementById("open") as HTMLButtonElement;
    const dialog = document.getElementById("dialog") as HTMLElement;
    const first = document.getElementById("first") as HTMLButtonElement;
    const last = document.getElementById("last") as HTMLButtonElement;
    opener.focus();

    setModalOpen(dialog, true);
    await Promise.resolve();
    expect((document.getElementById("app") as HTMLElement).inert).toBe(true);
    expect(document.getElementById("app")?.getAttribute("aria-hidden")).toBe("true");
    expect(document.activeElement).toBe(first);

    last.focus();
    const tab = new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true });
    last.dispatchEvent(tab);
    expect(tab.defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(first);

    setModalOpen(dialog, false);
    expect((document.getElementById("app") as HTMLElement).inert).toBe(false);
    expect(document.activeElement).toBe(opener);
  });
});
