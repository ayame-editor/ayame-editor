import { readFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { readCssSource, webRoot } from "./css-source.js";

const read = (file: string) => readFileSync(path.join(webRoot, file), "utf8");

describe("folding UI (#245)", () => {
  it("exposes menu and keyboard-remappable actions", () => {
    const doc = new DOMParser().parseFromString(read("index.html"), "text/html");
    for (const action of [
      "toggleFold",
      "foldCurrentLevel",
      "unfoldCurrentLevel",
      "unfoldAll",
      "foldToLevel",
      "goBlockStart",
      "goBlockEnd",
      "previousSiblingBlock",
      "nextSiblingBlock",
      "matchingBrace",
      "selectMatchingBrace",
    ]) {
      expect(doc.querySelector(`#view-menu [data-menu-action="${action}"]`), action).not.toBeNull();
      expect(read("src/state.ts")).toContain(`["${action}",`);
    }
  });

  it("uses an accessible native fold control and semantic visual tokens", () => {
    const editor = read("src/editor.ts");
    const actions = read("src/fold-actions.ts");
    expect(editor).toContain('fold.type = "button"');
    expect(editor).toContain('fold.setAttribute("aria-expanded"');
    expect(editor).toMatch(/fold\.setAttribute\(\s*"aria-label"/);
    expect(actions).toContain('showLoading(t("fold.scanning")');
    expect(actions).toContain("setLoadingDetail(");
    expect(actions).toContain("currentGeneration: () => state.doc.generation");
    expect(actions).toContain("onCancel: () => controller.abort()");

    const css = readCssSource();
    for (const token of [
      "--fold-control-size",
      "--fold-control-offset",
      "--fold-guide-stroke",
      "--fold-badge-radius",
    ]) {
      expect(css).toContain(`${token}:`);
      expect(css).toContain(`var(${token})`);
    }
    const rules = css.slice(css.indexOf(".fold-toggle"), css.indexOf(".tx {"));
    expect(rules).not.toMatch(/#[0-9a-f]{3,8}/i);
  });
});
