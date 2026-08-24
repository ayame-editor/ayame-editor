import { readFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { webRoot } from "./css-source.js";

describe("bounded character inspector UI (#247)", () => {
  it("exposes a labeled modal and keeps raw bytes as a distinct copy action", () => {
    const html = readFileSync(path.join(webRoot, "index.html"), "utf8");
    const doc = new DOMParser().parseFromString(html, "text/html");
    const modal = doc.querySelector("#inspect-modal");

    expect(modal?.getAttribute("aria-hidden")).toBe("true");
    expect(modal?.querySelector('[role="dialog"][aria-labelledby="inspect-title"]')).not.toBeNull();
    expect(modal?.querySelector("#inspect-summary[aria-live]")).not.toBeNull();
    expect(modal?.querySelector("#inspect-copy-raw")).not.toBeNull();
    expect(modal?.querySelector("#inspect-copy-encoded")).not.toBeNull();
    expect(modal?.querySelector("#inspect-copy-byte-escapes")).not.toBeNull();
    expect(doc.querySelector('[data-menu-action="inspectCharacter"]')).not.toBeNull();
  });
});
