import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../src/editor.js", () => ({
  cachedLine: vi.fn(),
  caretX: vi.fn(() => 48),
}));
vi.mock("../src/edits.js", () => ({ typeText: vi.fn(() => Promise.resolve()) }));
vi.mock("../src/fold-state.js", () => ({ visibleIndexForLine: vi.fn((line) => line) }));

import { cachedLine } from "../src/editor.js";
import { typeText } from "../src/edits.js";
import {
  handleCompletionKey,
  hideCompletion,
  initCompletion,
  showAutomaticCompletion,
  showCompletion,
} from "../src/completion.js";
import { COMPLETION_MAX_DOM_ROWS } from "../src/completion-model.js";
import { DEFAULT_SETTINGS, state } from "../src/state.js";

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

describe("completion popup and scan boundary (#246)", () => {
  beforeAll(() => {
    document.body.innerHTML = `
      <div id="content"><textarea id="hidden-input"></textarea></div>
      <div id="completion-popup" class="completion-popup hidden" role="listbox"></div>`;
    initCompletion();
  });

  beforeEach(() => {
    hideCompletion();
    vi.clearAllMocks();
    state.settings = { ...DEFAULT_SETTINGS };
    state.doc.stat = { open: true, path: "/tmp/example.txt" };
    state.doc.generation = 7;
    state.caret.composing = false;
    state.caret.position = { line: 2, col: 2 };
    state.caret.selection = null;
    state.caret.extraCursors = [];
    state.view.first = 0;
    state.view.cache = {
      start: 0,
      lines: [
        {
          number: 1,
          text: "alpha alpine",
          edited: false,
          inserted: false,
          original_line: 1,
          truncated: false,
        },
        {
          number: 2,
          text: "al",
          edited: false,
          inserted: false,
          original_line: 2,
          truncated: false,
        },
      ],
    };
    state.view.sparseCache = new Map();
    vi.mocked(cachedLine).mockImplementation(
      (line) => state.view.cache.lines.find((record) => record.number === line) || null,
    );
    vi.stubGlobal("fetch", vi.fn());
  });

  afterEach(() => vi.unstubAllGlobals());

  it("uses only resident text for automatic completion and exposes listbox state", () => {
    showAutomaticCompletion();

    expect(fetch).not.toHaveBeenCalled();
    expect(document.querySelectorAll('[role="option"]')).toHaveLength(2);
    expect(document.getElementById("hidden-input")?.getAttribute("aria-expanded")).toBe("true");
    expect(document.getElementById("hidden-input")?.getAttribute("aria-activedescendant")).toBe(
      "completion-option-0",
    );
  });

  it("accepts a keyboard-selected suffix without replacing the committed prefix", () => {
    showAutomaticCompletion();
    const down = {
      key: "ArrowDown",
      ctrlKey: false,
      metaKey: false,
      altKey: false,
      preventDefault: vi.fn(),
      stopPropagation: vi.fn(),
    };
    handleCompletionKey(down);
    handleCompletionKey({ ...down, key: "Enter" });

    expect(typeText).toHaveBeenCalledWith("pine");
    expect(document.getElementById("completion-popup")?.classList.contains("hidden")).toBe(true);
  });

  it("calls the word-only endpoint only on explicit completion and bounds DOM rows", async () => {
    vi.mocked(fetch).mockResolvedValue(
      jsonResponse({
        candidates: Array.from({ length: 100 }, (_, index) => `al${index}`),
        scanned_lines: 500,
        scanned_bytes: 65536,
        complete: false,
        timed_out: true,
        truncated: true,
        revision: 1,
      }),
    );

    await showCompletion();

    expect(fetch).toHaveBeenCalledWith(
      "/api/completion",
      expect.objectContaining({ method: "POST", signal: expect.any(AbortSignal) }),
    );
    expect(document.querySelectorAll('[role="option"]').length).toBeLessThanOrEqual(
      COMPLETION_MAX_DOM_ROWS,
    );
    expect(document.querySelector(".completion-status")?.getAttribute("role")).toBe("status");
  });

  it("drops a scan response after its caret generation becomes stale", async () => {
    let resolve!: (response: Response) => void;
    vi.mocked(fetch).mockImplementation(() => new Promise<Response>((done) => (resolve = done)));

    const pending = showCompletion();
    await Promise.resolve();
    state.caret.position = { line: 2, col: 1 };
    resolve(
      jsonResponse({
        candidates: ["alpha"],
        scanned_lines: 2,
        scanned_bytes: 16,
        complete: true,
        timed_out: false,
        truncated: false,
        revision: 1,
      }),
    );
    await pending;

    expect(document.getElementById("completion-popup")?.classList.contains("hidden")).toBe(true);
  });
});
