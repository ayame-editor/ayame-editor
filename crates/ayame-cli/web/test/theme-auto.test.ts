import { afterEach, describe, expect, it, vi } from "vitest";

// settings.js pulls the whole editor graph; mock its side-effecting direct
// imports so the pure theme-resolution helper can be exercised in isolation.
vi.mock("../src/editor.js", () => ({
  focusEditor: vi.fn(),
  invalidateFontMetrics: vi.fn(),
  scheduleRender: vi.fn(),
}));
vi.mock("../src/menus.js", () => ({
  applyLocale: vi.fn(),
  hideKeymap: vi.fn(),
  renderKeymapRows: vi.fn(),
  resetKeymap: vi.fn(),
  sanitizeKeymap: vi.fn(),
  showKeymap: vi.fn(),
  updateKeyHints: vi.fn(),
}));
vi.mock("../src/dialogs.js", () => ({ askConfirm: vi.fn() }));
vi.mock("../src/edits.js", () => ({ settleEditQueue: vi.fn() }));
vi.mock("../src/search.js", () => ({ flashCount: vi.fn() }));
vi.mock("../src/workspace.js", () => ({ onDocumentOpened: vi.fn() }));
vi.mock("../src/app.js", () => ({ postNativeMessage: vi.fn() }));
vi.mock("../src/api.js", () => ({ api: vi.fn() }));

import { resolvedThemeId } from "../src/settings.js";

function stubPrefersDark(dark: boolean) {
  vi.stubGlobal(
    "matchMedia",
    vi.fn((query: string) => ({
      matches: dark,
      media: query,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })),
  );
}

describe("OS dark-mode default theme (#153)", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("resolves auto/unset to the dark theme when the OS prefers dark", () => {
    stubPrefersDark(true);
    expect(resolvedThemeId("auto")).toBe("dark");
    expect(resolvedThemeId(undefined)).toBe("dark");
    expect(resolvedThemeId("")).toBe("dark");
  });

  it("resolves auto/unset to the light default when the OS prefers light", () => {
    stubPrefersDark(false);
    expect(resolvedThemeId("auto")).toBe("iris-light");
    expect(resolvedThemeId(undefined)).toBe("iris-light");
  });

  it("lets an explicitly chosen theme win over the OS preference", () => {
    stubPrefersDark(true);
    expect(resolvedThemeId("iris-mist")).toBe("iris-mist");
    expect(resolvedThemeId("black")).toBe("black");
    expect(resolvedThemeId("iris-light")).toBe("iris-light");
  });
});
