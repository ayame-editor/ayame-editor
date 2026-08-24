import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../src/editor.js", () => ({ focusEditor: vi.fn() }));
vi.mock("../src/i18n.js", () => ({ t: (key: string) => key }));

import { askConfirm, CONFIRM_ALT } from "../src/dialogs.js";

function keydownEnter() {
  document
    .getElementById("confirm")!
    .dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
}

function keydown(key: string) {
  document
    .getElementById("confirm")!
    .dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true }));
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
        <button id="confirm-alt" class="hidden"></button>
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

  it("leaves the third button out unless a label asks for it", async () => {
    const result = askConfirm("title", "message");
    await Promise.resolve();
    expect(document.getElementById("confirm-alt")!.classList.contains("hidden")).toBe(true);
    document.getElementById("confirm-cancel")!.focus();
    keydownEnter();
    await expect(result).resolves.toBe(false);
  });

  // The reload-vs-overwrite question an external file change raises (#163) has
  // two real answers plus cancel, which is what altLabel exists for.
  it("resolves the alt answer when the third button is clicked", async () => {
    const result = askConfirm("title", "message", { altLabel: "reload" });
    await Promise.resolve();
    const alt = document.getElementById("confirm-alt")!;
    expect(alt.classList.contains("hidden")).toBe(false);
    expect(alt.textContent).toBe("reload");
    alt.click();
    await expect(result).resolves.toBe(CONFIRM_ALT);
  });

  it("walks all three buttons with the arrow keys", async () => {
    const result = askConfirm("title", "message", { altLabel: "reload" });
    await Promise.resolve();
    document.getElementById("confirm-ok")!.focus();
    keydown("ArrowLeft");
    expect(document.activeElement!.id).toBe("confirm-alt");
    keydown("ArrowLeft");
    expect(document.activeElement!.id).toBe("confirm-cancel");
    keydown("ArrowRight");
    expect(document.activeElement!.id).toBe("confirm-alt");
    keydownEnter();
    await expect(result).resolves.toBe(CONFIRM_ALT);
  });

  it("uses the shared modal close registration for Escape and backdrop dismissal", async () => {
    const escaped = askConfirm("title", "message");
    keydown("Escape");
    await expect(escaped).resolves.toBe(false);

    const backdrop = askConfirm("title", "message");
    document
      .getElementById("confirm")!
      .dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    await expect(backdrop).resolves.toBe(false);
  });
});
