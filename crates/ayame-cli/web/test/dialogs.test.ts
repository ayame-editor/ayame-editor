import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../src/editor.js", () => ({ focusEditor: vi.fn() }));
vi.mock("../src/i18n.js", () => ({
  serverMessage: (error: unknown) => String(error),
  t: (key: string) => key,
}));

import { askConfirm, askForm, CONFIRM_ALT } from "../src/dialogs.js";

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

describe("form directory browser (#172)", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    document.body.innerHTML = `
      <div id="editor-hi" tabindex="-1"></div>
      <div id="form-modal" class="modal hidden" aria-hidden="true">
        <div id="form-title"></div>
        <button id="form-close"></button>
        <div id="form-body"></div>
        <button id="form-cancel"></button>
        <button id="form-ok"></button>
      </div>`;
  });

  it("chooses a folder inside the active form without opening another modal", async () => {
    const fetch = vi
      .fn()
      .mockResolvedValueOnce({
        ok: true,
        json: async () => ({
          dir: "/tmp",
          parent: "/",
          entries: [
            { name: "notes.txt", path: "/tmp/notes.txt", is_dir: false, size: 10 },
            { name: "logs", path: "/tmp/logs", is_dir: true, size: 0 },
          ],
        }),
      })
      .mockResolvedValueOnce({
        ok: true,
        json: async () => ({ dir: "/tmp/logs", parent: "/tmp", entries: [] }),
      });
    vi.stubGlobal("fetch", fetch);

    const result = askForm<{ dir: string }>("grep", [
      {
        id: "dir",
        type: "path",
        label: "directory",
        value: "/tmp",
        browseDirectories: true,
      },
    ]);
    const browse = document.querySelector<HTMLButtonElement>(".form-path .cmd")!;
    browse.click();

    await vi.waitFor(() =>
      expect(document.querySelector('[data-path="/tmp/logs"]')).not.toBeNull(),
    );
    const browser = document.querySelector<HTMLElement>(".form-path-browser")!;
    expect(browser.classList.contains("hidden")).toBe(false);
    expect(document.querySelectorAll(".modal:not(.hidden)")).toHaveLength(1);
    expect(document.querySelector('[data-path="/tmp/notes.txt"]')).toBeNull();

    const logs = document.querySelector<HTMLButtonElement>('[data-path="/tmp/logs"]')!;
    logs.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    expect(document.getElementById("form-modal")!.classList.contains("hidden")).toBe(false);
    logs.click();
    await vi.waitFor(() =>
      expect(document.querySelector<HTMLInputElement>(".form-path input")!.value).toBe("/tmp/logs"),
    );
    expect(fetch).toHaveBeenLastCalledWith("/api/browse?dir=%2Ftmp%2Flogs", undefined);

    document.getElementById("form-cancel")!.click();
    await expect(result).resolves.toBeNull();
  });
});
