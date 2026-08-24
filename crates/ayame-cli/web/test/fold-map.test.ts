import { describe, expect, it } from "vitest";

import { FoldMap, type FoldInterval } from "../src/fold-map.js";

function bruteVisible(total: number, intervals: readonly FoldInterval[]) {
  const hidden = new Set<number>();
  for (const interval of intervals) {
    for (let line = interval.start + 1; line <= Math.min(interval.end, total - 1); line++) {
      hidden.add(line);
    }
  }
  return Array.from({ length: total }, (_, line) => line).filter((line) => !hidden.has(line));
}

describe("sparse FoldMap (#245)", () => {
  it("normalizes overlap and nesting but preserves adjacent visible headers", () => {
    const map = new FoldMap([
      { start: 10, end: 20 },
      { start: 12, end: 14 },
      { start: 18, end: 24 },
      { start: 25, end: 30 },
      { start: 0, end: 3 },
    ]);
    expect(map.intervals()).toEqual([
      { start: 0, end: 3 },
      { start: 10, end: 24 },
      { start: 25, end: 30 },
    ]);
  });

  it("maps logical lines, folded headers, EOF, and visible rows exactly", () => {
    const map = new FoldMap([
      { start: 0, end: 3 },
      { start: 7, end: 9 },
    ]);
    const visible = [0, 4, 5, 6, 7, 10, 11];
    expect(map.visibleLineCount(12)).toBe(visible.length);
    expect(visible.map((_, row) => map.logicalAtVisible(row, 12))).toEqual(visible);
    expect(map.logicalAtVisible(visible.length, 12)).toBe(12);
    expect(map.visibleIndex(2, 12)).toBe(0);
    expect(map.visibleIndex(8, 12)).toBe(4);
    expect(map.visibleIndex(12, 12)).toBe(visible.length);
  });

  it("matches a brute-force model across randomized overlap, adjacency, nesting, and EOF", () => {
    let seed = 0x245;
    const random = () => {
      seed = (seed * 1664525 + 1013904223) >>> 0;
      return seed / 0x1_0000_0000;
    };
    for (let pass = 0; pass < 300; pass++) {
      const total = 1 + Math.floor(random() * 80);
      const input = Array.from({ length: Math.floor(random() * 14) }, () => {
        const left = Math.floor(random() * total);
        const right = Math.floor(random() * total);
        return { start: Math.min(left, right), end: Math.max(left, right) };
      });
      const map = new FoldMap(input);
      const expected = bruteVisible(total, map.intervals());
      expect(map.visibleLineCount(total)).toBe(expected.length);
      expected.forEach((line, visible) => {
        expect(map.logicalAtVisible(visible, total)).toBe(line);
        expect(map.visibleIndex(line, total)).toBe(visible);
      });
      expect(map.logicalAtVisible(expected.length, total)).toBe(total);
    }
  });

  it("keeps memory proportional to collapsed intervals at ten billion lines", () => {
    const map = new FoldMap([
      { start: 10, end: 1_000_000_000 },
      { start: 9_999_999_990, end: 9_999_999_999 },
    ]);
    expect(map.size).toBe(2);
    expect(map.visibleLineCount(10_000_000_000)).toBe(9_000_000_001);
    expect(map.logicalAtVisible(map.visibleLineCount(10_000_000_000), 10_000_000_000)).toBe(
      10_000_000_000,
    );
  });
});
