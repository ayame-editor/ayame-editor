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

  it("does not override the widened settings popup with the old width", () => {
    const widths = [
      ...read("style.css").matchAll(/\.settings-panel\s*{\s*width:\s*min\((\d+)px/g),
    ].map((match) => Number(match[1]));
    expect(widths).toEqual([840]);
  });

  it("labels search, opener, prompt, and command-palette controls", () => {
    const doc = new DOMParser().parseFromString(read("index.html"), "text/html");
    for (const id of ["find", "replace-input", "palette-input"]) {
      expect(doc.getElementById(id)?.getAttribute("aria-label"), id).toBeTruthy();
    }
    for (const id of ["opt-case", "opt-word", "opt-regex"]) {
      const button = doc.getElementById(id);
      expect(button?.getAttribute("aria-label"), id).toBeTruthy();
      expect(button?.getAttribute("aria-pressed"), id).toBe("false");
    }
    expect(doc.querySelector('label[for="opener-input"]')).not.toBeNull();
    expect(doc.querySelector('label[for="prompt-input"]')).not.toBeNull();
    expect(doc.getElementById("find-count")?.getAttribute("aria-live")).toBe("polite");
  });

  it("keeps keyboard focus visible and honors reduced motion", () => {
    const css = read("style.css");
    expect(css).toContain("button:focus-visible");
    expect(css).toContain("#viewport:focus-visible");
    expect(css).toContain("@media (prefers-reduced-motion: reduce)");
    expect(css).toContain("animation: none !important");
  });

  it("dispatches the encoding menu once and deduplicates palette actions (#181)", () => {
    const html = read("index.html");
    const input = read("src/input.ts");
    const menus = read("src/menus.ts");
    expect(html).toContain('id="convert-save-item" class="menu-item" data-menu-action="encoding"');
    expect(input).not.toContain('$("convert-save-item").addEventListener');
    expect(menus).toContain("if (!action || seen.has(action)) return");
    expect(menus).toContain("seen.add(action)");
  });

  it("reserves the JSON document action slot and syncs selection menu state (#186)", () => {
    const html = read("index.html");
    const css = read("style.css");
    const menus = read("src/menus.ts");
    expect(html).toContain('class="document-actions"');
    expect(css).toMatch(/\.document-actions\s*\{[^}]*width:\s*132px/s);
    expect(css).toMatch(/\.document-actions > button\.hidden\s*\{[^}]*visibility:\s*hidden/s);
    expect(menus).toContain('if (id === "edit")');
    expect(menus).toContain("item.disabled = !hasSelection");
  });

  it("uses conventional edit-menu clipboard placement and ARIA menubar keys", () => {
    const doc = new DOMParser().parseFromString(read("index.html"), "text/html");
    const menus = read("src/menus.ts");
    for (const action of ["cut", "copy", "paste"]) {
      expect(doc.querySelector(`#edit-menu [data-menu-action="${action}"]`), action).not.toBeNull();
    }
    expect(doc.querySelector('#selection-menu [data-menu-action="copy"]')).toBeNull();
    expect(doc.querySelector('#selection-menu [data-menu-action="cut"]')).toBeNull();
    expect(menus).toContain(
      'export const APP_MENUS = ["file", "edit", "selection", "view", "help"]',
    );
    expect(menus).toContain("onMenubarKeydown");
    for (const key of [
      "ArrowLeft",
      "ArrowRight",
      "ArrowUp",
      "ArrowDown",
      "Home",
      "End",
      "Escape",
    ]) {
      expect(menus).toContain(`"${key}"`);
    }
  });

  it("uses one responsive modal limit/body padding and marks status actions", () => {
    const css = read("style.css");
    expect(css).toContain("--modal-inline-limit: 94vw");
    expect(css).toContain("--modal-body-padding: 14px 16px 16px");
    expect(css.match(/width:\s*min\([^;]+,\s*(?:92|96)vw\)/g)).toBeNull();
    expect(css).toMatch(/#statusbar button\.seg-btn\s*\{[^}]*text-decoration:\s*underline dotted/s);
  });

  it("keeps component chrome in its owning rules without dead Iris overrides (#158)", () => {
    const css = read("style.css");
    expect(css).not.toContain("Iris skin");
    expect(css).not.toContain(".menu-button");
    expect(css.match(/(?:^|\n)button\s*\{[^}]*border-radius:/gs)).toHaveLength(1);
    expect(css.match(/\.field\s*\{[^}]*border-radius:/gs)).toHaveLength(1);
    expect(css.match(/\.field:focus-within\s*\{/g)).toHaveLength(1);
    expect(css.match(/\.modal-panel\s*\{[^}]*border-radius:/gs)).toHaveLength(1);
    expect(css.indexOf(".set-hint {")).toBeGreaterThan(css.indexOf(".settings-body"));

    const doc = new DOMParser().parseFromString(read("index.html"), "text/html");
    const menuButtons = [...doc.querySelectorAll("#menubar > .menu-shell > .menubar-button")];
    expect(menuButtons.length).toBeGreaterThan(0);
    expect(menuButtons.every((button) => Boolean(button.textContent?.trim()))).toBe(true);
  });

  it("uses one input and command-button size across dialogs (#150, #151)", () => {
    const css = read("style.css");
    const controls = [
      ".opener-path input",
      ".set-row select",
      '.set-row input[type="text"]',
      ".keymap-input",
      ".palette-input",
      "#prompt-input",
      '.form-row input[type="text"]',
      ".form-row select",
    ];
    const sharedStart = css.indexOf(`${controls.join(",\n")} {`);
    const focusStart = css.indexOf(
      `${controls.map((selector) => `${selector}:focus`).join(",\n")} {`,
    );
    expect(sharedStart).toBeGreaterThan(0);
    expect(css.slice(sharedStart, css.indexOf("}", sharedStart))).toContain(
      "height: var(--control-h)",
    );
    expect(focusStart).toBeGreaterThan(sharedStart);
    expect(css.slice(focusStart, css.indexOf("}", focusStart))).toContain(
      "box-shadow: var(--control-focus-ring)",
    );

    expect(
      css.match(/(?:\.opener-path|\.[\w-]+-actions) \.cmd\s*\{[^}]*(?:height|min-width):/gs),
    ).toBeNull();
    expect(css.match(/(?:^|\n)button\s*\{[^}]*border-radius:/gs)).toHaveLength(1);
  });
});
