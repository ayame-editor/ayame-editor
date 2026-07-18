import { describe, expect, it } from "vitest";

import {
  displayShortcut,
  eventShortcut,
  matchesShortcut,
  normalizeShortcut,
  sanitizeKeymap,
} from "../src/keys.js";
import { state } from "../src/state.js";

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

  it("indexes global actions until a keymap change triggers a rebuild", async () => {
    const { rebuildGlobalShortcutActions, shortcutActionFromEvent } = await import(
      "../src/menus.js"
    );
    const previous = state.settings;
    const oldEvent = new KeyboardEvent("keydown", { key: "s", ctrlKey: true, altKey: true });
    const newEvent = new KeyboardEvent("keydown", {
      key: "s",
      ctrlKey: true,
      altKey: true,
      shiftKey: true,
    });
    try {
      state.settings = { ...previous, keymap: { saveFile: "Ctrl+Alt+S" } };
      rebuildGlobalShortcutActions();
      expect(shortcutActionFromEvent(oldEvent)).toBe("saveFile");

      state.settings = { ...previous, keymap: { saveFile: "Ctrl+Alt+Shift+S" } };
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
