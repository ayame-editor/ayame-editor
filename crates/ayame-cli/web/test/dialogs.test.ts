import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../src/editor.js", () => ({ focusEditor: vi.fn() }));
vi.mock("../src/i18n.js", () => ({ t: (key: string) => key }));
vi.mock("../src/api.js", () => ({
  api: vi.fn(async () => ({
    kind: "sort",
    percent: 1,
    processed_lines: 1,
    total_lines: 10,
    canceled: false,
  })),
  apiPost: vi.fn(async () => ({})),
}));

import { apiPost } from "../src/api.js";
import { askConfirm, hideLoading, showLoading } from "../src/dialogs.js";

function keydownEnter() {
  document
    .getElementById("confirm")!
    .dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
}

describe("confirmation dialog keyboard actions", () => {
  beforeEach(() => {
    document.body.innerHTML = `
      <div id="editor-hi" tabindex="-1"></div>
      <div id="confirm" class="modal hidden" aria-hidden="true">
        <div id="confirm-title"></div>
        <div id="confirm-message"></div>
        <button id="confirm-close"></button>
        <button id="confirm-cancel"></button>
        <button id="confirm-ok"></button>
      </div>`;
  });

  it("resolves false when Cancel has focus and Enter is pressed", async () => {
    const result = askConfirm("title", "message");
    await Promise.resolve();
    document.getElementById("confirm-cancel")!.focus();
    keydownEnter();
    await expect(result).resolves.toBe(false);
  });

  it("resolves true when OK has focus and Enter is pressed", async () => {
    const result = askConfirm("title", "message");
    await Promise.resolve();
    document.getElementById("confirm-ok")!.focus();
    keydownEnter();
    await expect(result).resolves.toBe(true);
  });

  it("keeps OK as the default when neither action has focus", async () => {
    const result = askConfirm("title", "message");
    await Promise.resolve();
    (document.activeElement as HTMLElement | null)?.blur();
    keydownEnter();
    await expect(result).resolves.toBe(true);
  });
});

describe("operation overlay accessibility", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    document.body.innerHTML = `
      <main id="viewport"><button id="behind">Behind</button>
        <div id="overlay" class="overlay hidden" role="dialog" aria-modal="true"
             aria-hidden="true" aria-labelledby="overlay-text" aria-describedby="overlay-detail"></div>
      </main>`;
  });

  it("isolates the editor, focuses Cancel, and cancels with Escape", async () => {
    const behind = document.getElementById("behind") as HTMLButtonElement;
    behind.focus();
    showLoading("Working", { opId: "sort:test", cancel: true });
    await Promise.resolve();

    const overlay = document.getElementById("overlay")!;
    const cancel = document.getElementById("overlay-cancel") as HTMLButtonElement;
    expect(overlay.getAttribute("aria-hidden")).toBe("false");
    expect(behind.inert).toBe(true);
    expect(document.getElementById("overlay-detail")?.getAttribute("aria-live")).toBe("polite");
    expect(document.activeElement).toBe(cancel);

    cancel.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    expect(apiPost).toHaveBeenCalledWith("/api/ops/cancel", { id: "sort:test" });

    hideLoading();
    expect(behind.inert).toBe(false);
    expect(document.activeElement).toBe(behind);
  });
});
