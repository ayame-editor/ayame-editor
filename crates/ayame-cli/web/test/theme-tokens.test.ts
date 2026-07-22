import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { FONT_STACKS } from "../src/state.ts";

const webRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const css = readFileSync(path.join(webRoot, "style.css"), "utf8");

const compactStack = (value: string) => value.replace(/[\s"]/g, "");

// --- WCAG relative-luminance contrast, for the hint/placeholder legibility
// guarantee (#152). Mirrors the sRGB formula in WCAG 2.1 SC 1.4.3.
function channel(v: number): number {
  const c = v / 255;
  return c <= 0.03928 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4);
}
function luminance(hex: string): number {
  const h = hex.replace("#", "");
  const r = parseInt(h.slice(0, 2), 16);
  const g = parseInt(h.slice(2, 4), 16);
  const b = parseInt(h.slice(4, 6), 16);
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
}
function contrast(a: string, b: string): number {
  const hi = Math.max(luminance(a), luminance(b));
  const lo = Math.min(luminance(a), luminance(b));
  return (hi + 0.05) / (lo + 0.05);
}
function mixHex(foreground: string, background: string, share: number): string {
  const channelAt = (hex: string, offset: number) =>
    parseInt(hex.replace("#", "").slice(offset, offset + 2), 16);
  const mixed = [0, 2, 4].map((offset) =>
    Math.round(channelAt(foreground, offset) * share + channelAt(background, offset) * (1 - share)),
  );
  return `#${mixed.map((value) => value.toString(16).padStart(2, "0")).join("")}`;
}
// Read a `--token: #hex` value from a CSS block body.
function token(block: string, name: string): string | undefined {
  return block.match(new RegExp(`${name}:\\s*(#[0-9a-fA-F]{6})`))?.[1];
}

describe("Ayame design tokens", () => {
  it("uses one CJK-capable mono stack in CSS and runtime settings", () => {
    const cssMono = css.match(/--mono:\s*([^;]+);/s)?.[1];
    expect(cssMono).toBeTruthy();
    expect(compactStack(cssMono!)).toBe(compactStack(FONT_STACKS.mono));
    expect(FONT_STACKS.mono).toContain("Noto Sans Mono CJK JP");
    expect(FONT_STACKS.mono).toContain("MS Gothic");
  });

  it("defines the chrome type scale without hardcoded component font sizes", () => {
    for (const declaration of [
      "--fs-editor: 13px",
      "--fs-ui: 13px",
      "--fs-ui-sm: 12px",
      "--fs-hint: 11px",
      "--fs-ui-lg: 14px",
      "--fs-title: 16px",
      "--fs-ruler: 10px",
    ]) {
      expect(css).toContain(declaration);
    }
    const fontSizes = [...css.matchAll(/(?<!-)font-size:\s*([^;]+);/g)].map((match) =>
      match[1].trim(),
    );
    expect(fontSizes.length).toBeGreaterThanOrEqual(40);
    for (const fontSize of fontSizes) {
      expect(fontSize).toMatch(/^var\(--fs-[\w-]+\)$/);
    }
  });

  it("completes dark theme chrome tokens", () => {
    for (const theme of ["dark", "black"]) {
      const block = css.match(
        new RegExp(`html\\[data-theme="${theme}"\\]\\s*\\{([^}]+)\\}`, "s"),
      )?.[1];
      expect(block).toBeTruthy();
      expect(block).toContain("--accent: #9b82d8");
      expect(block).toContain("--accent-bright: #b49de6");
      expect(block).toContain("--status:");
      expect(block).toContain("--status-fg:");
    }
  });

  it("gives every named theme its own syntax palette and keeps Mono Paper monochrome (#154)", () => {
    const names = ["iris-mist", "iris-dawn", "sumi-light", "mono-paper", "dark", "black"];
    for (const name of names) {
      const block = css.match(
        new RegExp(`html\\[data-theme="${name}"\\]\\s*\\{([^}]+)\\}`, "s"),
      )?.[1];
      expect(block).toBeTruthy();
      for (const token of ["string", "number", "literal", "function", "link"]) {
        expect(block).toContain(`--syn-${token}:`);
      }
    }
    const mono = css.match(/html\[data-theme="mono-paper"\]\s*\{([^}]+)\}/s)?.[1] || "";
    const colors = [...mono.matchAll(/--syn-[\w-]+:\s*#([\da-f]{6})/gi)].map((match) => match[1]);
    expect(colors).toHaveLength(5);
    for (const color of colors) {
      expect(color.slice(0, 2)).toBe(color.slice(2, 4));
      expect(color.slice(2, 4)).toBe(color.slice(4, 6));
    }
  });

  it("uses semantic tokens for syntax colors", () => {
    expect(css).not.toMatch(
      /\.syn-(?:string|number|literal|function|link)[^{]*\{[^}]*#[\da-f]{3,8}/is,
    );
  });

  it("routes component corners, shadows, and foregrounds through tokens", () => {
    expect(css).toContain("--radius-control: 6px");
    expect(css).toContain("--shadow:");
    expect(css).toContain("--on-accent:");
    expect(css).not.toMatch(/border-radius:\s*(?:[1-9]|1[02])px/);
    expect(css).not.toContain("color: #fff");
    expect(css).not.toMatch(/font-weight:\s*(?:560|650)/);
  });

  it("keeps --fg-faint legible on --bg (WCAG AA >= 4.5:1) in every theme (#152)", () => {
    // Default theme lives in :root; named themes in their html[data-theme] block.
    const root = css.match(/:root\s*\{([\s\S]*?)\n\}/)?.[1] || "";
    const rootBg = token(root, "--bg");
    const rootFaint = token(root, "--fg-faint");
    expect(rootBg).toBeTruthy();
    expect(rootFaint).toBeTruthy();
    expect(contrast(rootFaint!, rootBg!)).toBeGreaterThanOrEqual(4.5);

    for (const theme of ["iris-mist", "iris-dawn", "sumi-light", "mono-paper", "dark", "black"]) {
      const block = css.match(
        new RegExp(`html\\[data-theme="${theme}"\\]\\s*\\{([^}]+)\\}`, "s"),
      )?.[1];
      expect(block, theme).toBeTruthy();
      const bg = token(block!, "--bg");
      const faint = token(block!, "--fg-faint");
      expect(bg, theme).toBeTruthy();
      expect(faint, theme).toBeTruthy();
      expect(contrast(faint!, bg!), `${theme} --fg-faint on --bg`).toBeGreaterThanOrEqual(4.5);
    }
  });

  it("derives an opaque, separated gutter palette with accessible text (#251, #253)", () => {
    const root = css.match(/:root\s*\{([\s\S]*?)\n\}/)?.[1] || "";
    for (const declaration of [
      "--gutter-surface: color-mix(in srgb, var(--accent) 12%, var(--gutter-bg))",
      "--gutter-border: var(--accent)",
      "--gutter-text: color-mix(in srgb, var(--fg) 55%, var(--gutter-fg))",
    ]) {
      expect(root).toContain(declaration);
    }

    const themes: [string, string][] = [
      ["iris-light", root],
      ...["iris-mist", "iris-dawn", "sumi-light", "mono-paper", "dark", "black"].map(
        (name) =>
          [
            name,
            css.match(new RegExp(`html\\[data-theme="${name}"\\]\\s*\\{([^}]+)\\}`, "s"))?.[1] ||
              "",
          ] as [string, string],
      ),
    ];

    for (const [name, block] of themes) {
      const required = (variable: string) => token(block, variable) || token(root, variable);
      const gutter = required("--gutter-bg");
      const editor = required("--edit-bg");
      const accent = required("--accent");
      const foreground = required("--fg");
      const gutterForeground = required("--gutter-fg");
      for (const value of [gutter, editor, accent, foreground, gutterForeground]) {
        expect(value, name).toBeTruthy();
      }

      const surface = mixHex(accent!, gutter!, 0.12);
      const text = mixHex(foreground!, gutterForeground!, 0.55);
      expect(surface.toLowerCase(), `${name} gutter surface`).not.toBe(editor!.toLowerCase());
      expect(contrast(accent!, editor!), `${name} gutter divider/editor`).toBeGreaterThanOrEqual(3);
      expect(contrast(accent!, surface), `${name} gutter divider/surface`).toBeGreaterThanOrEqual(
        3,
      );
      expect(contrast(text, surface), `${name} gutter text`).toBeGreaterThanOrEqual(4.5);
    }
  });

  it("never double-dims disabled buttons with both color and opacity (#152)", () => {
    // The base disabled rule must not set opacity alongside the faint color.
    const rule = css.match(/button:disabled\s*\{([^}]*)\}/)?.[1];
    expect(rule).toBeTruthy();
    expect(rule).not.toMatch(/opacity/);
  });

  it("uses a theme-aware focus ring for controls and the editor viewport (#183)", () => {
    expect(css).toContain("--focus-ring:");
    expect(css).toMatch(/button:focus-visible,[\s\S]*outline: 2px solid var\(--focus-ring\)/);
    expect(css).toMatch(
      /#viewport:focus-visible\s*\{[^}]*box-shadow: inset 0 0 0 2px var\(--focus-ring\)/s,
    );
  });

  it("routes every layer through an ordered semantic z-index scale (#178)", () => {
    for (const declaration of [
      "--z-base: 0",
      "--z-sticky: 10",
      "--z-content: 20",
      "--z-scrollbar: 30",
      "--z-caret: 40",
      "--z-ime: 50",
      "--z-find: 60",
      "--z-progress: 70",
      "--z-chrome: 80",
      "--z-menu: 90",
      "--z-menubar: 100",
      "--z-notification: 110",
      "--z-modal: 120",
      "--z-dropzone: 130",
      "--z-context-menu: 140",
    ]) {
      expect(css).toContain(declaration);
    }

    const layers = [...css.matchAll(/(?<!-)z-index:\s*([^;]+);/g)].map((match) => match[1].trim());
    // The progressive analysis strip adds one positioned surface while reusing
    // the existing semantic --z-find layer.
    expect(layers).toHaveLength(17);
    for (const layer of layers) {
      expect(layer).toMatch(/^var\(--z-[\w-]+\)$/);
    }
  });

  it("separates editor typography from chrome and tokenizes UI leading (#178)", () => {
    expect(css).toContain("Settings updates these two values live");
    expect(css).toContain("--fs-ui is not an alias for the user-controlled");
    expect(css).toContain("--fs-editor: 13px");
    expect(css).toContain("--lh-editor: 18px");
    expect(css).toContain("--lh-icon: 1");
    expect(css).toContain("--lh-tight: 1.4");
    expect(css).toContain("--lh-body: 1.5");

    const lineHeights = [...css.matchAll(/(?<!-)line-height:\s*([^;]+);/g)].map((match) =>
      match[1].trim(),
    );
    expect(lineHeights.length).toBeGreaterThanOrEqual(9);
    for (const lineHeight of lineHeights) {
      expect(lineHeight).toMatch(/^var\(--lh-[\w-]+\)$/);
    }
  });

  it("names and documents intentional subpixel optical geometry (#178)", () => {
    expect(css).toContain("Fractional values preserve the intended visual weight");
    for (const declaration of [
      "--stroke-icon: 1.8",
      "--stroke-folder: 1.5px",
      "--offset-pressed: 0.5px",
      "--offset-glyph-underline: 0.12em",
    ]) {
      expect(css).toContain(declaration);
    }
    expect(css).not.toMatch(/(?<!-)stroke-width:\s*1\.8/);
    expect(css).not.toMatch(/border:\s*1\.5px/);
    expect(css).not.toMatch(/box-shadow:[^;]*-1\.5px/s);
    expect(css).not.toMatch(/bottom:\s*0\.12em/);
    expect(css).not.toMatch(/translateY\(0\.5px\)/);
  });

  it("keeps readable fallbacks for color mixing and acrylic chrome (#178)", () => {
    expect(css).toContain("@supports not (color: color-mix(in srgb, black, white))");
    expect(css).toContain(
      "@supports not ((backdrop-filter: blur(1px)) or (-webkit-backdrop-filter: blur(1px)))",
    );
    expect(css).toMatch(
      /@supports not \(color: color-mix[\s\S]*\.modal-panel[\s\S]*background: var\(--bg-elevated\)/,
    );
    expect(css).toMatch(
      /@supports not \(\(backdrop-filter:[\s\S]*\.file-menu,[\s\S]*backdrop-filter: none/,
    );
    expect(css).toMatch(
      /@supports not \(\(backdrop-filter:[\s\S]*#statusbar\s*\{\s*background: var\(--bg-elevated\)/,
    );

    const standard = css.match(/(?<!-)backdrop-filter:/g) ?? [];
    const webkit = css.match(/-webkit-backdrop-filter:/g) ?? [];
    expect(standard).toHaveLength(webkit.length);
  });
});
