import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const webRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const read = (file: string) => readFileSync(path.join(webRoot, file), "utf8");

describe("application chrome", () => {
  it("removes the unfinished Explorer UI from PR #90", () => {
    const html = read("index.html");
    const sources = ["state.ts", "workspace.ts", "settings.ts", "menus.ts", "main.ts"]
      .map((file) => read(path.join("src", file)))
      .join("\n");

    expect(html).not.toMatch(/id="(?:sidebar|toggle-sidebar|sb-[^"]+)"/);
    expect(html).not.toContain('data-menu-action="toggleSidebar"');
    expect(html).not.toContain("data-sidebar-side");
    expect(sources).not.toMatch(/toggleSidebar|sidebarOpen|treeSetRoot|initTree/);
  });

  it("exposes Help as a top-level menu", () => {
    const doc = new DOMParser().parseFromString(read("index.html"), "text/html");
    const button = doc.querySelector("#menubar > .menu-shell > #help-menu-button");
    const menu = doc.querySelector("#help-menu");

    expect(button?.getAttribute("aria-haspopup")).toBe("true");
    expect(menu?.querySelector('[data-menu-action="help"]')).not.toBeNull();
    expect(menu?.querySelector('[data-menu-action="keymap"]')).not.toBeNull();
    expect(menu?.querySelector('[data-menu-action="about"]')).not.toBeNull();
  });

  it("binds conversion once and keeps shared actions in one canonical menu (#181)", () => {
    const html = read("index.html");
    const input = read("src/input.ts");
    expect(html.match(/data-menu-action="find"/g)).toHaveLength(1);
    expect(html.match(/data-menu-action="commandPalette"/g)).toHaveLength(1);
    expect(html.match(/data-menu-action="encoding"/g)).toHaveLength(1);
    expect(input).not.toContain('$("convert-save-item").addEventListener');
  });

  it("exposes search toggles, status values, and palette selection to assistive technology (#171)", () => {
    const doc = new DOMParser().parseFromString(read("index.html"), "text/html");
    for (const id of ["opt-case", "opt-word", "opt-regex"]) {
      const button = doc.querySelector(`#${id}`);
      expect(button?.getAttribute("aria-label")).toBeTruthy();
      expect(button?.getAttribute("aria-pressed")).toBe("false");
    }
    for (const id of ["st-enc", "st-eol", "st-zoom"]) {
      expect(doc.querySelector(`#${id}`)?.getAttribute("aria-label")).toBeTruthy();
    }
    const input = doc.querySelector("#palette-input");
    expect(input?.getAttribute("role")).toBe("combobox");
    expect(input?.getAttribute("aria-controls")).toBe("palette-list");

    const menus = read("src/menus.ts");
    expect(menus).toContain('setAttribute("aria-pressed", String(pressed))');
    expect(menus).toContain('setAttribute("aria-activedescendant", active.id)');
  });

  it("removes the editor-owned diff UI while keeping folder search standalone (#104)", () => {
    const html = read("index.html");
    const css = read("style.css");
    const sources = ["api.ts", "i18n.ts", "input.ts", "menus.ts", "search.ts", "state.ts"]
      .map((file) => read(path.join("src", file)))
      .join("\n");

    expect(html).not.toMatch(/diff-modal|diffFile|menu\.diff/);
    expect(css).not.toMatch(/\.diff-|--(?:add|del|chg)-bg|--word-(?:add|del)/);
    expect(sources).not.toMatch(/diffFile|dialog\.diff|\/api\/diff|DiffResponse/);
    expect(html).toContain('id="grep-modal"');
    expect(css).toContain(".grep-panel");
    expect(css).toContain(".grep-results");
  });

  it("sorts to a temporary result and opens a new tab", () => {
    const save = read("src/save.ts");
    expect(save).toContain("in_place: false");
    expect(save).toContain("await openPath(res.path)");
    expect(save).not.toContain('id: "mode"');
  });

  it("keeps the busy overlay above the floating find popup", () => {
    const css = read("style.css");
    const findZ = Number(css.match(/\.find-group\s*{[^}]*z-index:\s*(\d+)/s)?.[1]);
    const overlayZ = Number(css.match(/\.overlay\s*{[^}]*z-index:\s*(\d+)/s)?.[1]);

    expect(findZ).toBeGreaterThan(0);
    expect(overlayZ).toBeGreaterThan(findZ);
  });

  it("stops persistent motion when the OS requests reduced motion (#156)", () => {
    const css = read("style.css");
    const reduced = css.match(/@media \(prefers-reduced-motion: reduce\)\s*\{([\s\S]+)\}\s*$/)?.[1];
    expect(reduced).toBeTruthy();
    expect(reduced).toContain("#caret.on");
    expect(reduced).toContain(".caret.extra.on");
    expect(reduced).toContain("#statusbar .saving-seg::before");
    expect(reduced).toContain("animation: none");
  });

  it("does not override the widened settings popup with the old width", () => {
    const widths = [
      ...read("style.css").matchAll(/\.settings-panel\s*{\s*width:\s*min\((\d+)px/g),
    ].map((match) => Number(match[1]));
    expect(widths).toEqual([840]);
  });

  it("keeps component styles canonical and targets the real mobile menu class (#158)", () => {
    const css = read("style.css");

    for (const selector of ["button", ".field", ".field:focus-within", ".modal-panel"]) {
      const escaped = selector.replaceAll(".", "\\.");
      const declarations = css.match(new RegExp(`^${escaped}\\s*\\{`, "gm"));
      expect(declarations, selector).toHaveLength(1);
    }
    expect(css).not.toContain(".menu-button");
    expect(css).toContain(".menubar-button .btn-label");
  });

  it("uses one size definition for command buttons in every container (#151)", () => {
    const css = read("style.css").replace(/\/\*[\s\S]*?\*\//g, "");
    const dimensionalSelectors = [...css.matchAll(/([^{}]+)\{([^{}]*)\}/g)]
      .filter((match) => /\.cmd\b/.test(match[1]) && /(?:height|min-width)\s*:/.test(match[2]))
      .map((match) => match[1].trim());

    expect(dimensionalSelectors).toEqual(["button.cmd"]);
    expect(css).toContain("height: var(--control-h)");
    expect(css).toContain("min-width: var(--cmd-min-width)");
  });
});
