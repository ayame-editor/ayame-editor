import { describe, expect, it } from "vitest";

import {
  findMatchingBrace,
  findSiblingBlock,
  findStructureBlock,
  type StructureLineSource,
} from "../src/structure.js";

function source(lines: string[]): StructureLineSource {
  return {
    total: lines.length,
    async get(line) {
      return lines[line] ?? null;
    },
  };
}

describe("lazy structure providers (#245)", () => {
  it("finds the enclosing JSON block and ignores braces in strings and comments", async () => {
    const input = source([
      "{",
      '  "text": "not } a brace",',
      '  "child": {',
      "    /* } ignored",
      "       { ignored too */",
      '    "ok": true',
      "  }",
      "}",
    ]);
    await expect(findStructureBlock("brace", input, 4)).resolves.toEqual({
      start: 2,
      end: 6,
      level: 2,
    });
  });

  it("finds Python/YAML indentation blocks and sibling blocks", async () => {
    const input = source([
      "root:",
      "  first:",
      "    value: 1",
      "  second:",
      "    value: 2",
      "tail: true",
    ]);
    const first = await findStructureBlock("indent", input, 2);
    expect(first).toEqual({ start: 1, end: 2, level: 2 });
    await expect(findSiblingBlock("indent", input, first!, 1)).resolves.toEqual({
      start: 3,
      end: 4,
      level: 2,
    });
  });

  it("finds markup and multi-line log events", async () => {
    const markup = source(["<root>", "  <child>", "    text", "  </child>", "</root>"]);
    await expect(findStructureBlock("markup", markup, 2)).resolves.toEqual({
      start: 1,
      end: 3,
      level: 2,
    });
    const log = source([
      "2026-01-01 ERROR failed",
      "  at one",
      "  at two",
      "2026-01-01 INFO recovered",
    ]);
    await expect(findStructureBlock("log", log, 1)).resolves.toEqual({
      start: 0,
      end: 2,
      level: 0,
    });
  });

  it("matches braces in both directions", async () => {
    const input = source(["const value = {", "  items: [1, 2]", "};"]);
    await expect(findMatchingBrace(input, 0, 14)).resolves.toEqual({ line: 2, col: 0 });
    await expect(findMatchingBrace(input, 2, 0)).resolves.toEqual({ line: 0, col: 14 });
  });
});
