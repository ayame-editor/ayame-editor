import { describe, expect, it, vi } from "vitest";

import {
  expandNameTemplate,
  freeMemoName,
  isExistsError,
  parseSortKeys,
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
