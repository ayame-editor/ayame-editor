import { describe, expect, it, vi } from "vitest";

import {
  clampFontSize,
  FONT_SIZE_MAX,
  FONT_SIZE_MIN,
  loadSettings,
  migratedFontSize,
} from "../src/settings.js";
import { DEFAULT_SETTINGS, SETTINGS_KEY } from "../src/state.js";

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
      getItem: vi.fn(() => JSON.stringify({ fontSize: 16, zoom: 125 })),
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
});
