import { afterEach, describe, expect, it, vi } from "vitest";

afterEach(() => {
  delete window.ipc;
  delete window.__ayameSaveDialogDone;
  delete window.__ayameOpenDialogDone;
  vi.restoreAllMocks();
});

describe("typed native IPC (#114)", () => {
  it("serializes one JSON envelope for the native bridge", async () => {
    vi.resetModules();
    const postMessage = vi.fn();
    window.ipc = { postMessage };
    const { postNativeMessage } = await import("../src/app.js");

    postNativeMessage({ type: "update_check_startup", enabled: false });

    expect(postMessage).toHaveBeenCalledOnce();
    expect(JSON.parse(postMessage.mock.calls[0][0])).toEqual({
      type: "update_check_startup",
      enabled: false,
    });
  });

  it("preserves path delimiters in new-window requests", async () => {
    vi.resetModules();
    const postMessage = vi.fn();
    window.ipc = { postMessage };
    const { openNewWindow } = await import("../src/app.js");

    openNewWindow("C:\\tmp:a.txt", true);

    expect(JSON.parse(postMessage.mock.calls[0][0])).toEqual({
      type: "new_window_path",
      path: "C:\\tmp:a.txt",
      recover: true,
    });
  });

  it("sends required save-dialog fields in the typed payload", async () => {
    vi.resetModules();
    const postMessage = vi.fn();
    window.ipc = { postMessage };
    const { initApp, nativeSaveDialog } = await import("../src/app.js");
    initApp();

    const pending = nativeSaveDialog("/tmp:a", "draft.json");
    expect(JSON.parse(postMessage.mock.calls[0][0])).toEqual({
      type: "pick_save",
      dir: "/tmp:a",
      name: "draft.json",
    });

    window.__ayameSaveDialogDone?.(null);
    await expect(pending).resolves.toBeNull();
  });
});
