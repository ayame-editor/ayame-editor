import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  applySettings,
  clampFontSize,
  filterSettings,
  flushSettings,
  FONT_SIZE_MAX,
  FONT_SIZE_MIN,
  freshDefaultSettings,
  loadSettings,
  migratedFontSize,
  normalizeSettingsSearch,
  persistBackgroundImage,
  resetSettingsToDefaults,
  saveSettings,
  SETTINGS_SAVE_DELAY_MS,
} from "../src/settings.js";
import {
  DEFAULT_SETTINGS,
  LINE_HEIGHT,
  setLineHeight,
  SETTINGS_BG_IMAGE_KEY,
  SETTINGS_KEY,
  state,
} from "../src/state.js";

const webRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const indexHtml = readFileSync(path.join(webRoot, "index.html"), "utf8");

function memoryStorage(initial: Record<string, string> = {}): Storage {
  const values = new Map(Object.entries(initial));
  return {
    get length() {
      return values.size;
    },
    clear: () => values.clear(),
    getItem: (key) => values.get(key) ?? null,
    key: (index) => [...values.keys()][index] ?? null,
    removeItem: (key) => values.delete(key),
    setItem: (key, value) => values.set(key, String(value)),
  };
}

function installApplicationDom() {
  const parsed = new DOMParser().parseFromString(indexHtml, "text/html");
  document.head.innerHTML = parsed.head.innerHTML;
  document.body.innerHTML = parsed.body.innerHTML;
}

function stubBrowserSettingsEnvironment() {
  vi.stubGlobal(
    "requestAnimationFrame",
    vi.fn(() => 1),
  );
  vi.stubGlobal(
    "matchMedia",
    vi.fn((query: string) => ({
      matches: false,
      media: query,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })),
  );
}

afterEach(() => {
  flushSettings();
  setLineHeight(18);
  state.settings = {
    ...DEFAULT_SETTINGS,
    keymap: {},
    customThemes: {},
  };
  vi.useRealTimers();
  vi.unstubAllGlobals();
  document.documentElement.removeAttribute("data-theme");
  document.documentElement.removeAttribute("data-bg");
  document.documentElement.removeAttribute("class");
  document.documentElement.removeAttribute("style");
  document.head.innerHTML = "";
  document.body.innerHTML = "";
});

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

describe("settings execution paths (#188)", () => {
  it("round-trips user settings and the separately stored wallpaper", () => {
    vi.useFakeTimers();
    vi.stubGlobal("localStorage", memoryStorage());
    const wallpaper = "data:image/png;base64,roundtrip";
    const settings = {
      ...DEFAULT_SETTINGS,
      theme: "dark",
      fontSize: 21,
      wordWrap: true,
      restoreSession: false,
      bgMode: "image",
      bgImage: wallpaper,
      bgImageName: "roundtrip.png",
      keymap: { saveFile: "Alt+S" },
      customThemes: { Plum: { name: "Plum" } },
    };

    expect(persistBackgroundImage(wallpaper)).toBe(true);
    saveSettings(settings);
    vi.advanceTimersByTime(SETTINGS_SAVE_DELAY_MS);

    expect(loadSettings()).toMatchObject(settings);
    expect(JSON.parse(localStorage.getItem(SETTINGS_KEY)!)).not.toHaveProperty("bgImage");
    expect(localStorage.getItem(SETTINGS_BG_IMAGE_KEY)).toBe(wallpaper);
  });

  it("returns independent defaults for malformed storage", () => {
    vi.stubGlobal("localStorage", memoryStorage({ [SETTINGS_KEY]: "{" }));

    const loaded = loadSettings();
    loaded.keymap.saveFile = "Alt+S";
    loaded.customThemes.Plum = { name: "Plum" };

    expect(loaded).toMatchObject(DEFAULT_SETTINGS);
    expect(DEFAULT_SETTINGS.keymap).toEqual({});
    expect(DEFAULT_SETTINGS.customThemes).toEqual({});
  });

  it("applies clamped font, wrapping, minimap, and theme state to the live DOM", () => {
    document.head.innerHTML = '<meta name="theme-color" content="#fff">';
    document.body.innerHTML = `
      <main id="viewport"><div id="content"></div></main>
      <button id="st-fontsize"></button>`;
    stubBrowserSettingsEnvironment();

    applySettings({
      ...DEFAULT_SETTINGS,
      theme: "dark",
      font: "system",
      fontSize: FONT_SIZE_MAX + 20,
      wordWrap: true,
      minimap: false,
      zenkakuUnderline: true,
    });

    const root = document.documentElement;
    expect(root.dataset.theme).toBe("dark");
    expect(root.classList.contains("zenkaku-underline")).toBe(true);
    expect(root.style.getPropertyValue("--fs-editor")).toBe(`${FONT_SIZE_MAX}px`);
    expect(root.style.getPropertyValue("--lh-editor")).toBe(`${FONT_SIZE_MAX + 6}px`);
    expect(LINE_HEIGHT).toBe(FONT_SIZE_MAX + 6);
    expect(document.getElementById("content")!.classList.contains("wrap")).toBe(true);
    expect(document.getElementById("viewport")!.classList.contains("has-minimap")).toBe(false);
    expect(document.getElementById("st-fontsize")!.textContent).toBe(`${FONT_SIZE_MAX}px`);
  });

  it("restores defaults through the real dialog path while preserving authored assets", () => {
    vi.useFakeTimers();
    vi.stubGlobal("localStorage", memoryStorage());
    stubBrowserSettingsEnvironment();
    installApplicationDom();
    const wallpaper = "data:image/png;base64,preserved";
    state.settings = {
      ...DEFAULT_SETTINGS,
      theme: "dark",
      fontSize: 32,
      wordWrap: true,
      bgMode: "image",
      bgImage: wallpaper,
      bgImageName: "preserved.png",
      keymap: { saveFile: "Alt+S" },
      customThemes: { Plum: { name: "Plum", color: {} } },
    };
    persistBackgroundImage(wallpaper);

    resetSettingsToDefaults();
    flushSettings();

    expect(state.settings).toMatchObject({
      theme: DEFAULT_SETTINGS.theme,
      fontSize: DEFAULT_SETTINGS.fontSize,
      wordWrap: DEFAULT_SETTINGS.wordWrap,
      keymap: {},
      customThemes: { Plum: { name: "Plum", color: {} } },
      bgImage: wallpaper,
      bgImageName: "preserved.png",
    });
    expect(loadSettings()).toMatchObject(state.settings);
    expect((document.getElementById("set-fontsize-number") as HTMLInputElement).value).toBe(
      String(DEFAULT_SETTINGS.fontSize),
    );
    expect((document.getElementById("set-word-wrap") as HTMLInputElement).checked).toBe(false);
  });
});
