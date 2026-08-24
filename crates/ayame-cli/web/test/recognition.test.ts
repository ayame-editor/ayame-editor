import { beforeEach, describe, expect, it, vi } from "vitest";

const { lineChars } = vi.hoisted(() => ({ lineChars: vi.fn() }));

vi.mock("../src/editor.js", () => ({ lineChars }));
vi.mock("../src/api.js", () => ({ apiPost: vi.fn() }));
vi.mock("../src/dialogs.js", () => ({ askConfirm: vi.fn(), showMessage: vi.fn() }));
vi.mock("../src/workspace.js", () => ({ openPath: vi.fn() }));
vi.mock("../src/edits.js", () => ({ gotoLaunchPosition: vi.fn() }));
vi.mock("../src/app.js", () => ({
  isNativeApp: vi.fn(() => false),
  postNativeMessage: vi.fn(),
}));

import { candidateAt } from "../src/recognition.js";
import { state } from "../src/state.js";

describe("bounded selected target candidates (#248)", () => {
  beforeEach(() => {
    state.caret.selection = null;
    state.caret.extraCursors = [];
    lineChars.mockReset();
  });

  it("extracts the URL-like token under the caret", () => {
    const text = "see https://example.test/a:b?q=1 next";
    lineChars.mockReturnValue(Array.from(text));

    expect(candidateAt({ line: 0, col: text.indexOf("example") })).toBe(
      "https://example.test/a:b?q=1",
    );
  });

  it("prefers one selected path so spaces remain part of the candidate", () => {
    const text = "open ./logs/app 2026.log now";
    const start = text.indexOf("./logs");
    const end = text.indexOf(" now");
    lineChars.mockReturnValue(Array.from(text));
    state.caret.selection = {
      anchor: { line: 0, col: start },
      head: { line: 0, col: end },
    };

    expect(candidateAt({ line: 0, col: start + 4 })).toBe("./logs/app 2026.log");
  });

  it("does not materialize a multi-line selection as a candidate", () => {
    lineChars.mockReturnValue(Array.from("plain-token"));
    state.caret.selection = {
      anchor: { line: 0, col: 0 },
      head: { line: 1, col: 4 },
    };

    // It falls back to the bounded local token at the clicked point.
    expect(candidateAt({ line: 0, col: 2 })).toBe("plain-token");
  });
});
