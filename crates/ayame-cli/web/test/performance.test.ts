import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  caretX,
  colFromX,
  pool,
  renderSearchTicks,
  resetGeometryMeasurements,
} from "../src/editor.js";
import {
  flushSettings,
  persistBackgroundImage,
  saveSettings,
  SETTINGS_SAVE_DELAY_MS,
} from "../src/settings.js";
import {
  DEFAULT_SETTINGS,
  SETTINGS_BG_IMAGE_KEY,
  SETTINGS_KEY,
  state,
} from "../src/state.js";

describe("bounded editor rendering (#128)", () => {
  beforeEach(() => {
    document.body.innerHTML = '<div id="content"></div><div id="vticks"></div>';
    pool.splice(0);
    resetGeometryMeasurements(true);
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    pool.splice(0);
  });

  it("installs a line once while hit-testing and caches the gutter for the frame", () => {
    const row = document.createElement("div");
    const gutter = document.createElement("span");
    row.append(gutter, document.createElement("span"));
    pool.push(row);

    const text = "x".repeat(1024);
    state.view.cache = { start: 0, lines: [{ number: 0, text }] };
    state.view.total = 10_000;

    const gutterRead = vi.spyOn(gutter, "getBoundingClientRect").mockReturnValue({
      top: 0,
      left: 0,
      right: 40,
      bottom: 18,
      width: 40,
      height: 18,
      x: 0,
      y: 0,
      toJSON: () => ({}),
    });
    let endOffset = 0;
    const rangeRead = vi.fn(() => {
      const width = 40 + endOffset;
      return {
        top: 0,
        left: 0,
        right: width,
        bottom: 18,
        width,
        height: 18,
        x: 0,
        y: 0,
        toJSON: () => ({}),
      };
    });
    vi.spyOn(document, "createRange").mockReturnValue({
      setStart: vi.fn(),
      setEnd: vi.fn((_node, offset) => {
        endOffset = offset;
      }),
      getBoundingClientRect: rangeRead,
    } as unknown as Range);
    const dataWrite = vi.spyOn(CharacterData.prototype, "data", "set");

    expect(colFromX(0, 512)).toBe(472);
    expect(caretX(0, 472)).toBe(512);
    expect(gutterRead).toHaveBeenCalledOnce();
    expect(dataWrite).toHaveBeenCalledOnce();
    expect(rangeRead.mock.calls.length).toBeLessThan(20);
  });

  it("reuses search tick nodes and only moves the current marker", () => {
    state.search.query = "x";
    state.search.hits = Array.from({ length: 700 }, (_, line) => ({
      line,
      byte: line,
      len: 1,
    }));
    state.search.lastMatch = state.search.hits[0];
    state.analysis.status = null;
    state.markers.changeHistoryOverview = null;

    renderSearchTicks(600);
    const firstNodes = [...document.querySelectorAll(".vtick")];
    expect(firstNodes).toHaveLength(700);
    expect(firstNodes[0].classList.contains("current")).toBe(true);

    const createElement = vi.spyOn(document, "createElement");
    state.search.lastMatch = state.search.hits[699];
    renderSearchTicks(600);
    const secondNodes = [...document.querySelectorAll(".vtick")];

    expect(createElement).not.toHaveBeenCalled();
    expect(secondNodes[0]).toBe(firstNodes[0]);
    expect(secondNodes[699]).toBe(firstNodes[699]);
    expect(secondNodes[0].classList.contains("current")).toBe(false);
    expect(secondNodes[699].classList.contains("current")).toBe(true);
  });
});

describe("debounced settings persistence (#128)", () => {
  afterEach(() => {
    flushSettings();
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("coalesces slider updates and excludes the wallpaper from settings JSON", () => {
    vi.useFakeTimers();
    const setItem = vi.fn();
    const storageWrites: number[] = [];
    vi.stubGlobal("localStorage", {
      setItem: vi.fn((key: string, value: string) => {
        setItem(key, value);
        storageWrites.push(value.length);
      }),
      removeItem: vi.fn(),
    });
    const settings = {
      ...DEFAULT_SETTINGS,
      bgMode: "image",
      bgImage: `data:image/png;base64,${"A".repeat(1024 * 1024)}`,
    };
    for (let i = 0; i < 10; i++) saveSettings({ ...settings, fontSize: 13 + i });

    expect(setItem).not.toHaveBeenCalled();
    vi.advanceTimersByTime(SETTINGS_SAVE_DELAY_MS - 1);
    expect(setItem).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);

    expect(setItem).toHaveBeenCalledOnce();
    expect(setItem.mock.calls[0][0]).toBe(SETTINGS_KEY);
    expect(JSON.parse(setItem.mock.calls[0][1])).toMatchObject({ fontSize: 22 });
    expect(JSON.parse(setItem.mock.calls[0][1])).not.toHaveProperty("bgImage");
    expect(storageWrites[0]).toBeLessThan(10_000);
  });

  it("persists the wallpaper once under its dedicated key", () => {
    const setItem = vi.fn();
    vi.stubGlobal("localStorage", {
      setItem,
      removeItem: vi.fn(),
    });
    const image = `data:image/png;base64,${"A".repeat(1024 * 1024)}`;
    const settings = { ...DEFAULT_SETTINGS, bgMode: "image", bgImage: image };

    expect(persistBackgroundImage(image)).toBe(true);
    expect(flushSettings(settings)).toBe(true);

    expect(setItem).toHaveBeenCalledTimes(2);
    expect(setItem).toHaveBeenNthCalledWith(1, SETTINGS_BG_IMAGE_KEY, image);
    expect(JSON.parse(setItem.mock.calls[1][1])).not.toHaveProperty("bgImage");
  });
});
