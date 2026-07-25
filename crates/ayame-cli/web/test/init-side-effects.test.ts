import { afterEach, describe, expect, it, vi } from "vitest";

afterEach(() => {
  delete window.__ayameSaveDialogDone;
  delete window.__ayameOpenDialogDone;
  delete window.__ayameNativeCloseRequested;
  delete window.__ayameMenu;
  vi.restoreAllMocks();
});

describe("explicit application initialization (#129)", () => {
  it("keeps native dialog callbacks unregistered until initApp()", async () => {
    vi.resetModules();
    const app = await import("../src/app.js");

    expect(window.__ayameSaveDialogDone).toBeUndefined();
    expect(window.__ayameOpenDialogDone).toBeUndefined();

    app.initApp();
    const saveCallback = window.__ayameSaveDialogDone;
    const openCallback = window.__ayameOpenDialogDone;
    expect(saveCallback).toBeTypeOf("function");
    expect(openCallback).toBeTypeOf("function");

    app.initApp();
    expect(window.__ayameSaveDialogDone).toBe(saveCallback);
    expect(window.__ayameOpenDialogDone).toBe(openCallback);
  });

  it("registers save lifecycle hooks once from initSave()", async () => {
    vi.resetModules();
    const addEventListener = vi.spyOn(window, "addEventListener");
    const save = await import("../src/save.js");

    expect(window.__ayameNativeCloseRequested).toBeUndefined();
    expect(addEventListener.mock.calls.filter(([type]) => type === "pagehide")).toHaveLength(0);
    expect(addEventListener.mock.calls.filter(([type]) => type === "beforeunload")).toHaveLength(0);

    save.initSave();
    const closeCallback = window.__ayameNativeCloseRequested;
    save.initSave();

    expect(closeCallback).toBeTypeOf("function");
    expect(window.__ayameNativeCloseRequested).toBe(closeCallback);
    expect(addEventListener.mock.calls.filter(([type]) => type === "pagehide")).toHaveLength(1);
    expect(addEventListener.mock.calls.filter(([type]) => type === "beforeunload")).toHaveLength(1);
  });

  it("keeps the native menu dispatcher unregistered until initMenus()", async () => {
    vi.resetModules();
    const menus = await import("../src/menus.js");

    expect(window.__ayameMenu).toBeUndefined();
    menus.initMenus();
    const menuCallback = window.__ayameMenu;
    menus.initMenus();

    expect(menuCallback).toBeTypeOf("function");
    expect(window.__ayameMenu).toBe(menuCallback);
  });
});
