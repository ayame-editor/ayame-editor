import { describe, expect, it, vi } from "vitest";

import {
  commas,
  displayPath,
  humanBytes,
  isAbsolutePath,
  isUntitled,
  joinPath,
  pathCrumbs,
} from "../src/dom.js";
import { state } from "../src/state.js";

describe("localized number helpers (#176, #189)", () => {
  it("formats grouped numbers and byte decimals with the active UI locale", () => {
    const originalLanguage = state.settings.language;
    const localeSpy = vi.spyOn(Number.prototype, "toLocaleString");
    try {
      state.settings.language = "ja";
      commas(1_234);
      expect(localeSpy).toHaveBeenLastCalledWith("ja");

      state.settings.language = "en";
      humanBytes(1_536);
      expect(localeSpy).toHaveBeenLastCalledWith("en", {
        minimumFractionDigits: 2,
        maximumFractionDigits: 2,
      });
    } finally {
      localeSpy.mockRestore();
      state.settings.language = originalLanguage;
    }
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
