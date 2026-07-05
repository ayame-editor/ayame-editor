import { describe, expect, it } from "vitest";

import { displayPath, isAbsolutePath, joinPath, pathCrumbs } from "../src/dom.js";

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
});
