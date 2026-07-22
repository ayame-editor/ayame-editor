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

  it("keeps standard clipboard actions in Edit and exposes Paste (#159)", () => {
    const doc = new DOMParser().parseFromString(read("index.html"), "text/html");
    const actions = (selector: string) =>
      [...doc.querySelectorAll(`${selector} [data-menu-action]`)].map((el) =>
        el.getAttribute("data-menu-action"),
      );

    const editActions = actions("#edit-menu");
    expect(editActions.slice(2, 6)).toEqual(["cut", "copy", "paste", "selectAll"]);
    expect(actions("#selection-menu")).not.toEqual(
      expect.arrayContaining(["cut", "copy", "paste", "selectAll"]),
    );
    expect(
      doc.querySelector('#edit-menu [data-menu-action="paste"] [data-i18n="menu.paste"]'),
    ).not.toBeNull();
    expect(read("src/menus.ts")).toContain("paste: { run: pasteFromClipboard, editorOnly: true }");
  });

  it("exposes tab close, Save All, and recent files from the File menu (#167)", () => {
    const doc = new DOMParser().parseFromString(read("index.html"), "text/html");
    const file = doc.querySelector("#file-menu");
    const actions = [...(file?.querySelectorAll("[data-menu-action]") || [])].map((el) =>
      el.getAttribute("data-menu-action"),
    );

    expect(actions).toEqual(expect.arrayContaining(["saveAll", "closeTab"]));
    expect(file?.querySelector('[data-key-action="closeTab"]')).not.toBeNull();
    expect(file?.querySelector("#file-menu-recents[role=group]")).not.toBeNull();
    expect(read("src/save.ts")).toContain("export async function saveAllTabs()");
    expect(read("src/menus.ts")).toContain("export function renderFileMenuRecentFiles()");
  });

  it("groups, searches, and restores defaults from the Settings dialog (#165)", () => {
    const doc = new DOMParser().parseFromString(read("index.html"), "text/html");
    const settings = doc.querySelector("#settings");
    const groups = [...(settings?.querySelectorAll(".settings-group") || [])].map((group) =>
      group.getAttribute("aria-labelledby"),
    );

    expect(groups).toEqual([
      "settings-group-appearance",
      "settings-group-editor",
      "settings-group-app",
      "settings-group-advanced",
    ]);
    expect(settings?.querySelector('#settings-search[type="search"]')).not.toBeNull();
    expect(settings?.querySelector("#settings-search-status[role=status]")).not.toBeNull();
    expect(settings?.querySelector("#settings-reset")).not.toBeNull();
    for (const id of [
      "set-theme",
      "set-fontsize",
      "set-word-wrap",
      "set-language",
      "set-confirm-last-tab-close",
      "keymap-open",
    ]) {
      expect(settings?.querySelector(`#${id}`)?.closest(".settings-group"), id).not.toBeNull();
    }
    expect(read("src/settings.ts")).toContain("export function filterSettings(");
    expect(read("src/settings.ts")).toContain("export function resetSettingsToDefaults()");
  });

  it("keeps overflowed tabs reachable and supports same-window ordering (#166)", () => {
    const doc = new DOMParser().parseFromString(read("index.html"), "text/html");
    const tabbar = doc.querySelector("#tabbar");
    const allTabs = tabbar?.querySelector("#tab-list");
    const css = read("style.css");
    const workspace = read("src/workspace.ts");

    expect(tabbar?.querySelector("#tabs")).not.toBeNull();
    expect(tabbar?.querySelector("#new-tab")).not.toBeNull();
    expect(allTabs?.getAttribute("data-i18n-title")).toBe("tab.allTabs");
    expect(allTabs?.getAttribute("data-i18n-aria-label")).toBe("tab.allTabs");
    expect(css).toMatch(
      /#tabs\s*\{[^}]*flex:\s*1 1 auto[^}]*min-width:\s*0[^}]*overflow-x:\s*auto/s,
    );
    expect(css).toMatch(
      /\.tab \.tab-x\s*\{[^}]*width:\s*var\(--space-6\)[^}]*height:\s*var\(--space-6\)/s,
    );
    expect(workspace).toContain('"/api/tabs/reorder"');
    expect(workspace).toContain("export function ensureActiveTabVisible(");
  });

  it("keeps menubar dropdowns aligned with APP_MENUS and tools in the toolbar (#168)", () => {
    const doc = new DOMParser().parseFromString(read("index.html"), "text/html");
    const topLevelIds = [...doc.querySelectorAll("#menubar > .menu-shell > .menubar-button")].map(
      (button) => button.id.replace(/-menu-button$/, ""),
    );

    expect(topLevelIds).toEqual(["file", "edit", "selection", "view", "help"]);
    expect(doc.querySelector("#menubar > #settings-menu-button")).toBeNull();
    expect(doc.querySelector('#edit-menu [data-menu-action="settings"]')).not.toBeNull();
    expect(doc.querySelector("#toolbar #tools-menu-button")).not.toBeNull();

    const menus = read("src/menus.ts");
    expect(menus).toContain(
      'export const APP_MENUS = ["file", "edit", "selection", "view", "help"]',
    );
    expect(menus).toContain('const DROPDOWN_MENUS = [...APP_MENUS, "tools"]');
  });

  it("places the menubar and compact toolbar in one fixed-height chrome row (#148)", () => {
    const css = read("style.css").replace(/\/\*[\s\S]*?\*\//g, "");
    const block = (selector: string) =>
      css.match(new RegExp(`^${selector.replaceAll(".", "\\.")}\\s*\\{([^}]*)\\}`, "m"))?.[1] ?? "";

    const app = block("#app");
    expect(css).toContain("--chrome-top-h: 38px");
    expect(app).toContain('"menubar toolbar"');
    expect(app).toContain('"tabbar tabbar"');
    expect(app).toContain("grid-template-rows: var(--chrome-top-h) auto minmax(0, 1fr) auto");

    const menubar = block("#menubar");
    const toolbar = block("#toolbar");
    expect(menubar).toContain("grid-area: menubar");
    expect(menubar).toContain("height: 100%");
    expect(toolbar).toContain("grid-area: toolbar");
    expect(toolbar).toContain("height: 100%");
    expect(toolbar).toContain("flex-wrap: nowrap");
    expect(toolbar).toMatch(
      /padding:\s*var\(--space-0\)\s+var\(--space-2-5\)\s+var\(--space-0\)\s+var\(--space-0-5\)/,
    );
  });

  it("exposes search toggles, status values, and palette selection to assistive technology (#171)", () => {
    const doc = new DOMParser().parseFromString(read("index.html"), "text/html");
    for (const id of ["opt-case", "opt-word", "opt-regex"]) {
      const button = doc.querySelector(`#${id}`);
      expect(button?.getAttribute("aria-label")).toBeTruthy();
      expect(button?.getAttribute("aria-pressed")).toBe("false");
    }
    for (const id of ["st-enc", "st-eol", "st-fontsize"]) {
      expect(doc.querySelector(`#${id}`)?.getAttribute("aria-label")).toBeTruthy();
    }
    const input = doc.querySelector("#palette-input");
    expect(input?.getAttribute("role")).toBe("combobox");
    expect(input?.getAttribute("aria-controls")).toBe("palette-list");

    const menus = read("src/menus.ts");
    expect(menus).toContain('setAttribute("aria-pressed", String(pressed))');
    expect(menus).toContain('setAttribute("aria-activedescendant", active.id)');
  });

  it("exposes one effective font-size value instead of multiplying font size and zoom (#170)", () => {
    const doc = new DOMParser().parseFromString(read("index.html"), "text/html");
    const range = doc.querySelector<HTMLInputElement>("#set-fontsize");
    const number = doc.querySelector<HTMLInputElement>("#set-fontsize-number");
    const status = doc.querySelector("#st-fontsize");

    expect(range?.min).toBe("6");
    expect(range?.max).toBe("48");
    expect(number?.type).toBe("number");
    expect(number?.min).toBe(range?.min);
    expect(number?.max).toBe(range?.max);
    expect(status?.textContent).toBe("13px");
    expect(status?.getAttribute("data-i18n-title")).toBe("status.fontSizeTitle");
    expect(doc.querySelector("#st-zoom")).toBeNull();
  });

  it("separates transient notifications from persistent status values (#177)", () => {
    const doc = new DOMParser().parseFromString(read("index.html"), "text/html");
    const notifications = doc.querySelector("#notifications");

    expect(notifications?.getAttribute("role")).toBe("region");
    expect(notifications?.getAttribute("data-i18n-aria-label")).toBe("notification.region");
    expect(doc.querySelector("#statusbar #st-msg")).toBeNull();
    expect(doc.querySelector("#statusbar #st-saving")).not.toBeNull();
    expect(doc.querySelector("#statusbar #st-pos")).not.toBeNull();
  });

  it("exposes opener rows as keyboard-navigable listbox options (#185)", () => {
    const doc = new DOMParser().parseFromString(read("index.html"), "text/html");
    expect(doc.querySelector("#opener-input")?.getAttribute("aria-controls")).toBe(
      "opener-recent opener-list",
    );
    for (const id of ["opener-recent", "opener-list"]) {
      const list = doc.querySelector(`#${id}`);
      expect(list?.getAttribute("role")).toBe("listbox");
      expect(list?.getAttribute("tabindex")).toBe("0");
    }

    const workspace = read("src/workspace.ts");
    expect(workspace).toContain('row.setAttribute("role", "option")');
    expect(workspace).toContain('owner?.setAttribute("aria-activedescendant", active.id)');
    expect(workspace).toContain(
      '$("opener-input").addEventListener("keydown", onOpenerInputKeydown)',
    );
  });

  it("gives controls stable names and hides decorative icons from assistive technology (#182)", () => {
    const doc = new DOMParser().parseFromString(read("index.html"), "text/html");

    for (const id of ["find", "replace-input", "palette-input"]) {
      const input = doc.querySelector(`#${id}`);
      expect(input?.getAttribute("aria-label"), id).toBeTruthy();
      expect(input?.getAttribute("data-i18n-aria-label"), id).toBeTruthy();
    }
    expect(doc.querySelector('label[for="opener-input"]')?.id).toBe("opener-input-label");
    expect(doc.querySelector('label[for="prompt-input"]')?.id).toBe("prompt-label");

    const findCount = doc.querySelector("#find-count");
    expect(findCount?.getAttribute("role")).toBe("status");
    expect(findCount?.getAttribute("aria-live")).toBe("polite");
    expect(findCount?.getAttribute("aria-atomic")).toBe("true");

    for (const icon of doc.querySelectorAll("svg.ay-icon")) {
      expect(icon.getAttribute("aria-hidden")).toBe("true");
      expect(icon.getAttribute("focusable")).toBe("false");
    }
    expect(doc.querySelector("#tools-menu-button")?.hasAttribute("role")).toBe(false);
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
    const findGroup = css.match(/\.find-group\s*{([^}]*)}/s)?.[1] ?? "";
    const overlay = css.match(/\.overlay\s*{([^}]*)}/s)?.[1] ?? "";
    const tokenValue = (name: string) => Number(css.match(new RegExp(`${name}:\\s*(\\d+)`))?.[1]);
    const findZ = tokenValue("--z-find");
    const overlayZ = tokenValue("--z-progress");

    expect(findGroup).toContain("z-index: var(--z-find)");
    expect(overlay).toContain("z-index: var(--z-progress)");
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

  it("uses shared motion and state treatments for interactive feedback (#193)", () => {
    const css = read("style.css");
    const html = new DOMParser().parseFromString(read("index.html"), "text/html");
    const search = read("src/search.ts");
    const block = (selector: string) =>
      css.match(new RegExp(`^${selector.replaceAll(".", "\\.")}\\s*\\{([^}]*)\\}`, "m"))?.[1] ?? "";

    expect(css).toContain("--motion-fast: 120ms");
    expect(css).toContain("--motion-ease:");
    expect(css).toContain("button.toggle.on:hover");
    expect(css).toContain(".keymap-row.conflict .keymap-input");
    expect(css).toContain(".opener-msg:not(.busy):not(:empty)");
    expect(css).toContain(":is(mark, .grep-match)");

    const replace = block(".find-group .replace-row");
    expect(replace).not.toContain("display: none");
    expect(replace).toContain("height: 0");
    expect(replace).toContain("opacity: 0");
    expect(block("html.replace-open .find-group .replace-row")).toContain("height: 30px");

    const replaceRow = html.querySelector("#replace-row");
    expect(replaceRow?.getAttribute("aria-hidden")).toBe("true");
    expect(replaceRow?.hasAttribute("inert")).toBe(true);
    expect(search).toContain('row.setAttribute("aria-hidden", open ? "false" : "true")');
    expect(search).toContain("row.inert = !open");
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

  it("uses one text-entry component across dialogs with mono limited to paths (#150)", () => {
    const css = read("style.css").replace(/\/\*[\s\S]*?\*\//g, "");
    const html = new DOMParser().parseFromString(read("index.html"), "text/html");
    const block = (selector: string) => css.split(`${selector} {`, 2)[1]?.split("}", 1)[0] ?? "";

    const inputControl = block(".input-control");
    expect(inputControl).toContain("height: var(--control-h)");
    expect(inputControl).toContain("font-family: var(--ui)");
    expect(inputControl).toContain("font-size: var(--fs-ui)");
    expect(inputControl).toContain("border-radius: var(--radius-control)");
    expect(block(".input-control:focus")).toContain("box-shadow:");
    expect(block(".input-control--mono")).toContain("font-family: var(--mono)");

    for (const id of [
      "opener-input",
      "set-theme",
      "set-bg",
      "set-language",
      "set-font",
      "set-memo-name",
      "convert-enc",
      "convert-eol",
      "palette-input",
      "prompt-input",
    ]) {
      expect(html.querySelector(`#${id}`)?.classList.contains("input-control"), id).toBe(true);
    }
    expect(html.querySelector("#opener-input")?.classList.contains("input-control--mono")).toBe(
      true,
    );

    const dialogs = read("src/dialogs.ts");
    const menus = read("src/menus.ts");
    expect(dialogs).toContain('input.className = "input-control input-control--mono"');
    expect(dialogs).toContain('sel.className = "input-control"');
    expect(dialogs).toContain('input.className = "input-control"');
    expect(menus).toContain('input.className = "keymap-input input-control"');

    for (const selector of [
      ".opener-path input",
      ".set-row select",
      '.set-row input[type="text"]',
      ".keymap-input",
      ".palette-input",
      "#prompt-input",
      '.form-row input[type="text"]',
      ".form-row select",
    ]) {
      expect(block(selector), selector).not.toMatch(/height:|font-family:|font-size:/);
    }
  });

  it("shares exact, opaque gutter geometry and spacing (#194, #251-#253)", () => {
    const css = read("style.css");
    const padding =
      /padding:\s*var\(--space-0\)\s+var\(--gutter-pad-end\)\s+var\(--space-0\)\s+var\(--gutter-pad-start\)/;
    const block = (selector: string) =>
      css.match(new RegExp(`^${selector.replaceAll(".", "\\.")}\\s*\\{([^}]*)\\}`, "m"))?.[1] ?? "";

    expect(block(".ln")).toMatch(padding);
    expect(block(".grep-ln")).toMatch(padding);
    expect(block(".ln")).toContain("width: calc(");
    expect(block(".ln")).toContain("var(--gutter-ch, 1ch)");
    expect(block(".ln")).toContain("var(--gutter-border-width)");
    expect(block(".ln")).toContain("background: var(--gutter-surface)");
    expect(block(".ln")).toContain(
      "border-right: var(--gutter-border-width) solid var(--gutter-border)",
    );
    expect(block(".grep-ln")).toContain("background: var(--gutter-surface)");
    expect(block(".grep-ln")).toContain(
      "border-right: var(--gutter-border-width) solid var(--gutter-border)",
    );
    expect(block(".grep-hit")).toContain("var(--gutter-ch, 1ch)");
    expect(block(".grep-hit")).toContain("var(--gutter-border-width)");
    expect(css).not.toContain("7ch");
    expect(block(".ln")).not.toContain("transparent");
    expect(block(".grep-ln")).not.toContain("transparent");
    expect(block(".replace-btn")).not.toMatch(/border\s*:/);
  });

  it("centralizes change-history visuals and exposes every requested toggle (#243)", () => {
    const css = read("style.css");
    const doc = new DOMParser().parseFromString(read("index.html"), "text/html");
    const gutterStart = css.indexOf(".row:is(.change-saved, .change-unsaved)");
    const tickStart = css.indexOf(".vtick.change-vtick");
    const gutterRules = css.slice(
      gutterStart,
      css.indexOf(".tx {", gutterStart),
    );
    const tickRules = css.slice(tickStart, css.indexOf("#vthumb", tickStart));

    for (const token of [
      "--change-saved",
      "--change-unsaved",
      "--change-marker-w",
      "--change-marker-offset",
      "--change-deleted-size",
      "--change-tick-saved-w",
      "--change-tick-unsaved-w",
    ]) {
      expect(css).toContain(`${token}:`);
      expect(`${gutterRules}\n${tickRules}`).toContain(`var(${token})`);
    }
    expect(`${gutterRules}\n${tickRules}`).not.toMatch(/#[0-9a-f]{3,8}/i);
    expect(doc.querySelector('#view-menu [data-menu-action="toggleChangeHistory"]')).not.toBeNull();
    expect(doc.querySelector('#settings input[id="set-change-history"]')).not.toBeNull();
  });

  it("uses consistent modal edges and body padding with visible status actions (#157)", () => {
    const css = read("style.css");
    const block = (selector: string) => css.split(`${selector} {`, 2)[1]?.split("}", 1)[0] ?? "";

    for (const selector of [
      ".modal-panel",
      "#opener .modal-panel",
      ".settings-panel",
      ".keymap-panel",
      ".palette-panel",
      ".grep-panel",
      ".analysis-panel",
      ".confirm-panel",
      ".prompt-panel",
      ".form-panel",
    ]) {
      expect(block(selector), selector).toContain("var(--modal-edge)");
    }
    expect(css.match(/padding:\s*var\(--modal-body-padding\)/g)).toHaveLength(1);
    for (const legacyPadding of ["12px 28px 24px", "14px 16px 16px", "14px 16px 4px"]) {
      expect(css).not.toContain(legacyPadding);
    }
    expect(block("#statusbar button.seg-btn")).toMatch(/border:\s*1px solid/);
  });

  it("keeps inactive bookmark-list states visually hidden (#241)", () => {
    const css = read("style.css");
    expect(css).toMatch(
      /\.bookmark-empty\.hidden,\s*#bookmark-more\.hidden\s*\{\s*display:\s*none;/,
    );
  });
});
