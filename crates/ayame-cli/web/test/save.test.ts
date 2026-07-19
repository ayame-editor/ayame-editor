import { describe, expect, it, vi } from "vitest";

import {
  expandNameTemplate,
  freeMemoName,
  isExistsError,
  parseSortKeys,
  saveTabsSequentially,
  sortFormatForPath,
} from "../src/save.js";

describe("save-name helpers", () => {
  it("expands date/time and shared sequence tokens", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-07-05T03:04:05Z"));
    try {
      const taken = new Set(["note-20260705-01.txt", "note-20260705-02.txt"]);
      expect(expandNameTemplate("note-{date}-{seq2}.txt", taken)).toBe("note-20260705-03.txt");
    } finally {
      vi.useRealTimers();
    }
  });

  it("adds numeric suffixes for templates without sequence tokens", () => {
    expect(freeMemoName("note.txt", new Set(["note.txt", "note-2.txt"]))).toBe("note-3.txt");
  });

  it("recognizes existing targets only by the structured API code", () => {
    expect(isExistsError(Object.assign(new Error("whatever"), { code: "exists" }))).toBe(true);
    expect(isExistsError(Object.assign(new Error("already exists"), { code: "conflict" }))).toBe(
      false,
    );
    expect(isExistsError(new Error("a.txt は既に存在します"))).toBe(false);
  });

  it("detects CSV/TSV formats and parses ordered sort columns", () => {
    expect(sortFormatForPath("/data/report.CSV")).toBe("csv");
    expect(sortFormatForPath("/data/report.tsv")).toBe("tsv");
    expect(sortFormatForPath("/data/report.tab")).toBe("tsv");
    expect(sortFormatForPath("/data/report.txt")).toBe("text");
    expect(parseSortKeys("3, 1、2")).toEqual([3, 1, 2]);
  });
});

describe("Save All ordering (#167)", () => {
  it("saves only dirty tabs sequentially and restores the original tab", async () => {
    const events: string[] = [];
    const select = vi.fn(async (id: number) => {
      events.push(`select:${id}`);
      return true;
    });
    const save = vi.fn(async (tab: { id: number }) => {
      events.push(`save:${tab.id}`);
      return tab.id === 1;
    });

    const result = await saveTabsSequentially(
      [
        { id: 1, active: true, dirty: true },
        { id: 2, active: false, dirty: false },
        { id: 3, active: false, dirty: true },
      ],
      select,
      save,
    );

    expect(result).toEqual({ saved: 1, total: 2 });
    expect(events).toEqual(["save:1", "select:3", "save:3", "select:1"]);
  });

  it("skips a tab when selection fails instead of saving the wrong document", async () => {
    const save = vi.fn(async () => true);
    const select = vi.fn(async (id: number) => id !== 2);

    const result = await saveTabsSequentially(
      [
        { id: 1, active: true, dirty: false },
        { id: 2, active: false, dirty: true },
        { id: 3, active: false, dirty: true },
      ],
      select,
      save,
    );

    expect(result).toEqual({ saved: 1, total: 2 });
    expect(save).toHaveBeenCalledOnce();
    expect(select.mock.calls.map(([id]) => id)).toEqual([2, 3, 1]);
  });
});
