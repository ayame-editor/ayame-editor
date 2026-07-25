import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it, vi } from "vitest";

vi.mock("../src/editor.js", () => ({
  cachedLine: vi.fn(() => null),
  maxFirst: vi.fn(() => 0),
  rowsVisible: vi.fn(() => 40),
  setFirst: vi.fn(),
  setMinimapRenderer: vi.fn(),
}));

import {
  MINIMAP_ROW,
  MINIMAP_WIDTH,
  lineAtMinimapY,
  minimapCapacity,
  minimapStart,
  scrubTargetFirst,
} from "../src/minimap.js";

const webRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

describe("bounded minimap (#144)", () => {
  it("uses the fixed 88px strip and three-pixel line rows", () => {
    expect(MINIMAP_WIDTH).toBe(88);
    expect(MINIMAP_ROW).toBe(3);
    expect(minimapCapacity(300)).toBe(100);
    expect(minimapCapacity(0)).toBe(1);
    expect(readFileSync(path.join(webRoot, "style.css"), "utf8")).toContain(
      "--minimap-width: 88px",
    );
  });

  it("shows the whole document from line zero when it fits", () => {
    expect(minimapStart(0, 50, 100, 20)).toBe(0);
    expect(minimapStart(20, 50, 100, 20)).toBe(0);
  });

  it("slides proportionally over larger documents", () => {
    const total = 1_000_000;
    const capacity = 250;
    const maxFirstLine = total + 1 - 30;
    expect(minimapStart(0, total, capacity, maxFirstLine)).toBe(0);
    expect(minimapStart(maxFirstLine, total, capacity, maxFirstLine)).toBe(
      total + 1 - capacity,
    );
    const middle = minimapStart(Math.round(maxFirstLine / 2), total, capacity, maxFirstLine);
    expect(Math.abs(middle - (total + 1 - capacity) / 2)).toBeLessThan(2);
  });

  it("keeps the viewport inside the window at ten billion lines", () => {
    const total = 10_000_000_000;
    const capacity = 260;
    const visible = 33;
    const maxFirstLine = total + 1 - visible;
    for (const fraction of [0, 0.25, 0.5, 0.75, 0.999, 1]) {
      const first = Math.round(fraction * maxFirstLine);
      const start = minimapStart(first, total, capacity, maxFirstLine);
      expect(start).toBeLessThanOrEqual(first);
      expect(start + capacity).toBeGreaterThanOrEqual(
        Math.min(first + visible, total + 1) - 2,
      );
    }
  });

  it("maps pixels to lines and centers scrub targets", () => {
    expect(lineAtMinimapY(0, 500)).toBe(500);
    expect(lineAtMinimapY(MINIMAP_ROW, 500)).toBe(501);
    expect(lineAtMinimapY(-4, 500)).toBe(500);
    expect(scrubTargetFirst(100 * MINIMAP_ROW, 500, 40)).toBe(580);
  });

  it("is wired between content and the custom scrollbar with two toggles", () => {
    const html = new DOMParser().parseFromString(
      readFileSync(path.join(webRoot, "index.html"), "utf8"),
      "text/html",
    );
    const viewport = html.querySelector("#viewport");
    const children = [...(viewport?.children || [])];
    expect(children.indexOf(html.querySelector("#content")!)).toBeLessThan(
      children.indexOf(html.querySelector("#minimap")!),
    );
    expect(children.indexOf(html.querySelector("#minimap")!)).toBeLessThan(
      children.indexOf(html.querySelector("#vscrollbar")!),
    );
    expect(html.querySelector('#menu-toggle-minimap[aria-checked="true"]')).not.toBeNull();
    expect(html.querySelector('#set-minimap[type="checkbox"]')).not.toBeNull();
  });

  it("depends only on cached editor lines and never starts an API fetch", () => {
    const source = readFileSync(path.join(webRoot, "src/minimap.ts"), "utf8");
    expect(source).toContain("cachedLine(line)");
    expect(source).not.toMatch(/\b(?:api|ensureData)\s*\(/);
  });
});
