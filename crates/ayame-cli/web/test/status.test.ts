import { beforeEach, describe, expect, it } from "vitest";
import { state } from "../src/state.js";
import { t } from "../src/i18n.js";
import { updateStatusPos } from "../src/status.js";

function setup(lines: string[], total = lines.length) {
  document.body.innerHTML = '<span id="st-pos"></span>';
  state.view.total = total;
  state.view.cache = {
    start: 0,
    lines: lines.map((text) => ({ text }) as never),
  };
  state.view.sparseCache = new Map();
  state.caret.position = { line: 0, col: 0 };
  state.caret.extraCursors = [];
  state.caret.selection = null;
}

function statusText() {
  return document.getElementById("st-pos")?.textContent ?? "";
}

function caret(line: number, col: number) {
  return { line, col };
}

describe("status bar position and selection", () => {
  beforeEach(() => {
    setup(["abc", "de😀", "fghi"]);
  });

  it("shows the caret position when nothing is selected", () => {
    state.caret.position = caret(1, 2);
    updateStatusPos();
    expect(statusText()).toBe(t("status.pos", { line: "2", col: "3" }));
  });

  it("counts selected scalars and lines on one line", () => {
    state.caret.selection = { anchor: caret(0, 1), head: caret(0, 3), rect: false };
    updateStatusPos();
    const pos = t("status.pos", { line: "1", col: "1" });
    expect(statusText()).toBe(
      t("status.selectionPos", { pos, lines: "1", chars: "2" }),
    );
  });

  it("counts newlines and surrogate pairs across lines", () => {
    state.caret.selection = { anchor: caret(0, 1), head: caret(2, 2), rect: false };
    updateStatusPos();
    const pos = t("status.pos", { line: "1", col: "1" });
    // "bc" + "\n" + "de😀" + "\n" + "fg"
    expect(statusText()).toBe(
      t("status.selectionPos", { pos, lines: "3", chars: "9" }),
    );
  });

  it("counts a rectangular selection column by column", () => {
    state.caret.selection = { anchor: caret(0, 0), head: caret(1, 2), rect: true };
    updateStatusPos();
    const pos = t("status.pos", { line: "1", col: "1" });
    expect(statusText()).toBe(
      t("status.selectionPos", { pos, lines: "2", chars: "4" }),
    );
  });

  it("keeps the line count when a selected line is not cached", () => {
    setup(["abc", "de😀", "fghi"], 4);
    state.caret.selection = { anchor: caret(0, 0), head: caret(3, 1), rect: false };
    updateStatusPos();
    const pos = t("status.pos", { line: "1", col: "1" });
    expect(statusText()).toBe(
      t("status.selectionPosLines", { pos, lines: "4" }),
    );
  });
});
