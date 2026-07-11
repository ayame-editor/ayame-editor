import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../src/editor.js", () => ({ focusEditor: vi.fn() }));
vi.mock("../src/i18n.js", () => ({ t: (key: string) => key }));

import { askConfirm } from "../src/dialogs.js";

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
