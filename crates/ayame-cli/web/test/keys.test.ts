import { describe, expect, it } from "vitest";

import {
  displayShortcut,
  eventShortcut,
  eventShortcuts,
  matchesShortcut,
  normalizeShortcut,
  sanitizeKeymap,
} from "../src/keys.js";
import { DEFAULT_KEYMAP, state } from "../src/state.js";

it("assigns distinct defaults to the major tools (#172)", () => {
  expect([
    DEFAULT_KEYMAP.sortSave,
    DEFAULT_KEYMAP.splitFile,
    DEFAULT_KEYMAP.grepFolder,
    DEFAULT_KEYMAP.grepSave,
  ]).toEqual(["Ctrl+Alt+S", "Ctrl+Alt+P", "Ctrl+Shift+F", "Ctrl+Alt+G"]);
});

describe("shortcut normalization", () => {
  it("keeps known multi-character key casing compatible with KeyboardEvent.key", () => {
    expect(normalizeShortcut("ctrl+alt+arrowup")).toBe("Ctrl+Alt+ArrowUp");
    expect(normalizeShortcut("shift+pageup")).toBe("Shift+PageUp");
    expect(normalizeShortcut("ctrl+escape")).toBe("Ctrl+Escape");
  });

  it("normalizes function keys and aliases", () => {
    expect(normalizeShortcut("cmd+shift+f3")).toBe("Ctrl+Shift+F3");
    expect(normalizeShortcut("option+x")).toBe("Alt+X");
  });

  // "+" is both the separator and the key zoom-in wants, so it has to survive
  // parsing rather than dissolve into empty segments (#175).
  it("keeps a bound plus sign as the key", () => {
    expect(normalizeShortcut("Ctrl++")).toBe("Ctrl++");
    expect(normalizeShortcut("ctrl+shift++")).toBe("Ctrl+Shift++");
    expect(normalizeShortcut("Ctrl+-")).toBe("Ctrl+-");
    expect(normalizeShortcut("+")).toBe("+");
  });

  it("still refuses a bare plus as an unmodified binding", () => {
    expect(sanitizeKeymap({ zoomIn: "+" }).zoomIn).toBe("");
    expect(sanitizeKeymap({ zoomIn: "Ctrl++" }).zoomIn).toBe("Ctrl++");
  });

  it("drops unbindable custom shortcuts while preserving valid ones", () => {
    const clean = sanitizeKeymap({
      closeTab: "alt+w",
      saveFile: "s",
      addCursorAbove: "ctrl+alt+arrowup",
    });
    expect(clean.closeTab).toBe("Alt+W");
    expect(clean.saveFile).toBe("");
    expect(clean.addCursorAbove).toBe("Ctrl+Alt+ArrowUp");
  });
});

describe("KeyboardEvent shortcut conversion", () => {
  it("uses the same spelling and matcher as normalized known keys", () => {
    const ev = new KeyboardEvent("keydown", {
      key: "ArrowUp",
      ctrlKey: true,
      altKey: true,
    });
    expect(eventShortcut(ev)).toBe(normalizeShortcut("ctrl+alt+arrowup"));
    expect(matchesShortcut(ev, ["Ctrl+Alt+ArrowUp"])).toBe(true);
  });

  // Zoom in is Ctrl+Shift+= on a US keyboard and Ctrl+Shift+; on a Japanese
  // one; both report the key "+". Requiring the recorded Shift would tie the
  // binding to one keyboard (#175).
  it("lets a punctuation key answer to its chord with and without Shift", () => {
    const shifted = new KeyboardEvent("keydown", { key: "+", ctrlKey: true, shiftKey: true });
    expect(eventShortcuts(shifted)).toEqual(["Ctrl+Shift++", "Ctrl++"]);
    expect(matchesShortcut(shifted, ["Ctrl++"])).toBe(true);
    expect(matchesShortcut(shifted, ["Ctrl+Shift++"])).toBe(true);
  });

  it("keeps Shift significant for letters and named keys", () => {
    const letter = new KeyboardEvent("keydown", { key: "T", ctrlKey: true, shiftKey: true });
    expect(eventShortcuts(letter)).toEqual(["Ctrl+Shift+T"]);
    expect(matchesShortcut(letter, ["Ctrl+T"])).toBe(false);

    const named = new KeyboardEvent("keydown", { key: "F3", shiftKey: true });
    expect(eventShortcuts(named)).toEqual(["Shift+F3"]);
    expect(matchesShortcut(named, ["F3"])).toBe(false);
  });

  // Ctrl+V has to keep reaching the browser's own clipboard event: it carries
  // the text and needs no clipboard-read permission. Rebinding paste is what
  // routes it through the action instead (#175).
  it("leaves paste to the browser on its native chord but not once rebound", async () => {
    const { rebuildGlobalShortcutActions, shortcutActionFromEvent } = await import(
      "../src/menus.js"
    );
    const previous = state.settings;
    try {
      state.settings = { ...previous, keymap: {} };
      rebuildGlobalShortcutActions();
      const ctrlV = new KeyboardEvent("keydown", { key: "v", ctrlKey: true });
      expect(shortcutActionFromEvent(ctrlV)).toBeNull();

      state.settings = { ...previous, keymap: { paste: "Ctrl+Shift+V" } };
      rebuildGlobalShortcutActions();
      const rebound = new KeyboardEvent("keydown", { key: "V", ctrlKey: true, shiftKey: true });
      expect(shortcutActionFromEvent(rebound)).toBe("paste");
    } finally {
      state.settings = previous;
      rebuildGlobalShortcutActions();
    }
  });

  it("dispatches the zoom actions its defaults describe", async () => {
    const { rebuildGlobalShortcutActions, shortcutActionFromEvent } = await import(
      "../src/menus.js"
    );
    const previous = state.settings;
    try {
      state.settings = { ...previous, keymap: {} };
      rebuildGlobalShortcutActions();
      const press = (key: string, shiftKey = false) =>
        shortcutActionFromEvent(new KeyboardEvent("keydown", { key, ctrlKey: true, shiftKey }));
      expect(press("+", true)).toBe("zoomIn");
      expect(press("=")).toBe("zoomIn");
      expect(press("-")).toBe("zoomOut");
      expect(press("_", true)).toBe("zoomOut");
      expect(press("0")).toBe("zoomReset");
    } finally {
      state.settings = previous;
      rebuildGlobalShortcutActions();
    }
  });

  it("indexes global actions until a keymap change triggers a rebuild", async () => {
    const { rebuildGlobalShortcutActions, shortcutActionFromEvent } = await import(
      "../src/menus.js"
    );
    const previous = state.settings;
    const oldEvent = new KeyboardEvent("keydown", { key: "j", ctrlKey: true, altKey: true });
    const newEvent = new KeyboardEvent("keydown", {
      key: "j",
      ctrlKey: true,
      altKey: true,
      shiftKey: true,
    });
    try {
      state.settings = { ...previous, keymap: { saveFile: "Ctrl+Alt+J" } };
      rebuildGlobalShortcutActions();
      expect(shortcutActionFromEvent(oldEvent)).toBe("saveFile");

      state.settings = { ...previous, keymap: { saveFile: "Ctrl+Alt+Shift+J" } };
      expect(shortcutActionFromEvent(oldEvent)).toBe("saveFile");
      expect(shortcutActionFromEvent(newEvent)).toBeNull();

      rebuildGlobalShortcutActions();
      expect(shortcutActionFromEvent(oldEvent)).toBeNull();
      expect(shortcutActionFromEvent(newEvent)).toBe("saveFile");
    } finally {
      state.settings = previous;
      rebuildGlobalShortcutActions();
    }
  });
});

describe("shortcut display (#164)", () => {
  it("uses native compact modifier glyphs on macOS", () => {
    expect(displayShortcut("Ctrl+S", "MacIntel")).toBe("⌘S");
    expect(displayShortcut("Ctrl+Alt+Shift+ArrowUp", "MacIntel")).toBe("⌘⌥⇧↑");
    expect(displayShortcut("Ctrl++", "MacIntel")).toBe("⌘+");
  });

  it("keeps the existing labels on Windows and Linux", () => {
    expect(displayShortcut("Ctrl+Alt+Shift+ArrowUp", "Win32")).toBe("Ctrl+Alt+Shift+↑");
    expect(displayShortcut("Ctrl+S", "Linux x86_64")).toBe("Ctrl+S");
  });
});
