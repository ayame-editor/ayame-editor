import { describe, expect, it, vi } from "vitest";

import {
  clampFontSize,
  filterSettings,
  FONT_SIZE_MAX,
  FONT_SIZE_MIN,
  freshDefaultSettings,
  loadSettings,
  migratedFontSize,
  normalizeSettingsSearch,
} from "../src/settings.js";
import { DEFAULT_SETTINGS, SETTINGS_BG_IMAGE_KEY, SETTINGS_KEY } from "../src/state.js";

describe("editor font size (#170)", () => {
  it("clamps the single effective pixel value", () => {
    expect(clampFontSize(FONT_SIZE_MIN - 1)).toBe(FONT_SIZE_MIN);
    expect(clampFontSize(17.6)).toBe(18);
    expect(clampFontSize(FONT_SIZE_MAX + 1)).toBe(FONT_SIZE_MAX);
    expect(clampFontSize("not-a-number")).toBe(DEFAULT_SETTINGS.fontSize);
  });

  it("collapses legacy fontSize × zoom settings to their visible pixel size", () => {
    expect(migratedFontSize({ fontSize: 16, zoom: 125 })).toBe(20);
    expect(migratedFontSize({ fontSize: 22, zoom: 300 })).toBe(FONT_SIZE_MAX);
    expect(migratedFontSize({ fontSize: 13, zoom: 50 })).toBe(7);
  });

  it("keeps already-unified font sizes unchanged", () => {
    expect(migratedFontSize({ fontSize: 31 })).toBe(31);
    expect(DEFAULT_SETTINGS).not.toHaveProperty("zoom");
  });

  it("rewrites legacy storage once without the zoom field", () => {
    const setItem = vi.fn();
    vi.stubGlobal("localStorage", {
      getItem: vi.fn((key: string) =>
        key === SETTINGS_KEY ? JSON.stringify({ fontSize: 16, zoom: 125 }) : null,
      ),
      setItem,
    });
    try {
      expect(loadSettings().fontSize).toBe(20);
      expect(setItem).toHaveBeenCalledOnce();
      const [key, json] = setItem.mock.calls[0];
      expect(key).toBe(SETTINGS_KEY);
      expect(JSON.parse(json)).toMatchObject({ fontSize: 20 });
      expect(JSON.parse(json)).not.toHaveProperty("zoom");
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it("migrates a legacy wallpaper out of the settings JSON", () => {
    const image = "data:image/png;base64,legacy";
    const setItem = vi.fn();
    vi.stubGlobal("localStorage", {
      getItem: vi.fn((key: string) =>
        key === SETTINGS_KEY
          ? JSON.stringify({ bgMode: "image", bgImage: image, bgImageName: "iris.png" })
          : null,
      ),
      setItem,
    });
    try {
      expect(loadSettings().bgImage).toBe(image);
      expect(setItem).toHaveBeenCalledTimes(2);
      expect(setItem.mock.calls[0][0]).toBe(SETTINGS_KEY);
      expect(JSON.parse(setItem.mock.calls[0][1])).not.toHaveProperty("bgImage");
      expect(setItem).toHaveBeenNthCalledWith(2, SETTINGS_BG_IMAGE_KEY, image);
    } finally {
      vi.unstubAllGlobals();
    }
  });
});

describe("organized settings (#165)", () => {
  it("builds fresh defaults while preserving user-authored visual assets", () => {
    const customThemes = { Plum: { name: "Plum" } };
    const defaults = freshDefaultSettings({
      theme: "dark",
      keymap: { saveFile: "Alt+S" },
      customThemes,
      bgImage: "data:image/png;base64,test",
      bgImageName: "plum.png",
    });

    expect(defaults).toMatchObject({
      theme: DEFAULT_SETTINGS.theme,
      keymap: {},
      customThemes,
      bgImage: "data:image/png;base64,test",
      bgImageName: "plum.png",
    });
    expect(defaults.customThemes).not.toBe(customThemes);
    expect(defaults.keymap).not.toBe(DEFAULT_SETTINGS.keymap);
  });

  it("normalizes width, case, and surrounding whitespace for incremental search", () => {
    expect(normalizeSettingsSearch("  ＦＯＮＴ  ")).toBe("font");
  });

  it("filters individual settings and keeps a matching group fully visible", () => {
    document.body.innerHTML = `
      <span id="settings-search-status"></span>
      <p id="settings-empty" class="hidden"></p>
      <section id="appearance" class="settings-group">
        <h3 class="settings-group-title">Appearance</h3>
        <div class="settings-group-body">
          <label id="theme-row" class="set-row">Theme</label>
          <label id="font-row" class="set-row">Font family</label>
          <label id="image-row" class="set-row hidden">Choose image</label>
        </div>
      </section>
      <section id="editor" class="settings-group">
        <h3 class="settings-group-title">Editor</h3>
        <div class="settings-group-body">
          <label id="wrap-row" class="set-row">Word wrap</label>
        </div>
      </section>`;

    expect(filterSettings("font")).toBe(1);
    expect(document.getElementById("theme-row")!.classList.contains("settings-filtered")).toBe(
      true,
    );
    expect(document.getElementById("font-row")!.classList.contains("settings-filtered")).toBe(
      false,
    );
    expect(document.getElementById("editor")!.classList.contains("settings-filtered")).toBe(true);

    expect(filterSettings("appearance")).toBe(2);
    expect(document.getElementById("theme-row")!.classList.contains("settings-filtered")).toBe(
      false,
    );
    expect(document.getElementById("font-row")!.classList.contains("settings-filtered")).toBe(
      false,
    );

    expect(filterSettings("missing")).toBe(0);
    expect(document.getElementById("settings-empty")!.classList.contains("hidden")).toBe(false);
  });
});
