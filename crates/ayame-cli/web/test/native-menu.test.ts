import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { buildNativeMenuItems, syncNativeMenu } from "../src/native-menu.js";
import { state } from "../src/state.js";

let previousLanguage = state.settings.language;

beforeEach(() => {
  previousLanguage = state.settings.language;
  state.settings = { ...state.settings, language: "en" };
});

afterEach(() => {
  state.settings = { ...state.settings, language: previousLanguage };
  delete window.ipc;
});

describe("macOS native menu configuration (#141)", () => {
  it("derives labels from i18n and accelerators from the current keymap", () => {
    const items = buildNativeMenuItems((action) =>
      action === "find" ? "Ctrl+Shift+F" : action === "settings" ? "Ctrl+," : "",
    );

    expect(items.find((item) => item.id === "find")).toEqual({
      id: "find",
      label: "Find",
      shortcut: "Ctrl+Shift+F",
    });
    expect(items.find((item) => item.id === "settings")).toEqual({
      id: "settings",
      label: "Settings",
      shortcut: "Ctrl+,",
    });
    expect(items.find((item) => item.id === "section.file")?.label).toBe("File");
    expect(new Set(items.map((item) => item.id)).size).toBe(items.length);
  });

  it("sends one typed JSON menu_config envelope to the native shell", () => {
    const postMessage = vi.fn();
    window.ipc = { postMessage };

    syncNativeMenu((action) => (action === "find" ? "Ctrl+Shift+F" : ""));

    expect(postMessage).toHaveBeenCalledOnce();
    const payload = JSON.parse(postMessage.mock.calls[0][0]);
    expect(payload.type).toBe("menu_config");
    expect(payload.items.find((item) => item.id === "find")).toEqual({
      id: "find",
      label: "Find",
      shortcut: "Ctrl+Shift+F",
    });
  });
});
