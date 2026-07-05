import { describe, expect, it } from "vitest";

import { normalizeShortcut, sanitizeKeymap } from "../src/shortcuts.js";

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
