import { describe, expect, it, vi } from "vitest";

import {
  button,
  commas,
  displayPath,
  el,
  humanBytes,
  isAbsolutePath,
  isUntitled,
  joinPath,
  pathCrumbs,
} from "../src/dom.js";
import { state } from "../src/state.js";

describe("localized number helpers (#176)", () => {
  it("formats grouped numbers with the active UI locale", () => {
    const originalLanguage = state.settings.language;
    const localeSpy = vi.spyOn(Number.prototype, "toLocaleString");
    try {
      state.settings.language = "ja";
      commas(1_234);
      expect(localeSpy).toHaveBeenLastCalledWith("ja");
    } finally {
      localeSpy.mockRestore();
      state.settings.language = originalLanguage;
    }
  });
});

describe("DOM construction helpers (#126)", () => {
  it("creates consistently initialized elements and buttons", () => {
    expect(el("span", "label", "Ayame").outerHTML).toBe(
      '<span class="label">Ayame</span>',
    );
    const clicked = vi.fn();
    const action = button("cmd primary", "Save", clicked);
    expect(action.type).toBe("button");
    expect(action.className).toBe("cmd primary");
    action.click();
    expect(clicked).toHaveBeenCalledOnce();
  });
});

describe("path helpers", () => {
  it("normalizes Windows verbatim paths for display", () => {
    expect(displayPath("\\\\?\\C:\\logs\\app.txt")).toBe("C:\\logs\\app.txt");
    expect(displayPath("\\\\?\\UNC\\server\\share\\app.txt")).toBe("\\\\server\\share\\app.txt");
  });

  it("joins relative paths without changing absolute inputs", () => {
    expect(joinPath("/var/log", "app.log")).toBe("/var/log/app.log");
    expect(joinPath("C:\\logs", "app.log")).toBe("C:\\logs\\app.log");
    expect(joinPath("/var/log", "/tmp/app.log")).toBe("/tmp/app.log");
  });

  it("recognizes absolute Unix, drive, and UNC paths", () => {
    expect(isAbsolutePath("/tmp/a")).toBe(true);
    expect(isAbsolutePath("C:\\tmp\\a")).toBe(true);
    expect(isAbsolutePath("\\\\server\\share\\a")).toBe(true);
    expect(isAbsolutePath("tmp/a")).toBe(false);
  });

  it("builds clickable crumbs for Windows drives", () => {
    expect(pathCrumbs("C:\\Users\\me").map((c) => c.label)).toEqual(["C:", "Users", "me"]);
  });

  it("formats byte sizes with the locale's decimal separator (#189)", () => {
    // Small sizes stay integer bytes, no separator either way.
    expect(humanBytes(512, "en-US")).toBe("512 B");
    // Scaled sizes carry two fraction digits in the selected locale's format.
    expect(humanBytes(1536, "en-US")).toBe("1.50 KiB");
    expect(humanBytes(1536, "de-DE")).toBe("1,50 KiB");
    expect(humanBytes(5 * 1024 * 1024, "en-US")).toBe("5.00 MiB");
  });

  it("recognizes untitled scratch buffers in both dir-name generations", () => {
    // Current server scratch dirs ("srv-untitled") and the pre-rename form.
    expect(
      isUntitled(
        "C:\\Users\\x\\AppData\\Local\\Temp\\ayame-srv-untitled-55c647d-0-0\\untitled.txt",
      ),
    ).toBe(true);
    expect(isUntitled("/tmp/ayame-untitled-1234/untitled.txt")).toBe(true);
    expect(isUntitled("E:\\note\\untitled.txt")).toBe(false);
    expect(isUntitled("")).toBe(false);
  });
});
