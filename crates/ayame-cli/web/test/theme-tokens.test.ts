import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { FONT_STACKS } from "../src/state.ts";
import { readCssSource } from "./css-source.js";

const webRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const css = readCssSource();
const themes = JSON.parse(readFileSync(path.join(webRoot, "themes.json"), "utf8"));

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
describe("Ayame design tokens", () => {
  it("keeps built-in theme definitions only in themes.json (#126)", () => {
    const settingsSource = readFileSync(path.join(webRoot, "src/settings.ts"), "utf8");
    expect(Object.keys(themes)).toEqual([
      "iris-light",
      "iris-mist",
      "iris-dawn",
      "sumi-light",
      "mono-paper",
      "dark",
      "black",
    ]);
    expect(settingsSource).not.toContain('"iris-light": {');
    expect(css).not.toContain("html[data-theme=");
  });

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
    for (const name of ["dark", "black"]) {
      const theme = themes[name];
      expect(theme.color.accent).toBe("#9b82d8");
      expect(theme.color.accent2).toBe("#b49de6");
      expect(theme.acrylic.tint).toBeTruthy();
      expect(theme.ui.statusForeground).toBe(theme.color.ink);
    }
  });

  it("gives every named theme its own syntax palette and keeps Mono Paper monochrome (#154)", () => {
    const names = ["iris-mist", "iris-dawn", "sumi-light", "mono-paper", "dark", "black"];
    for (const name of names) {
      for (const token of ["string", "number", "literal", "function", "link"]) {
        expect(themes[name].ui.syntax[token]).toBeTruthy();
      }
    }
    const colors = Object.values(themes["mono-paper"].ui.syntax).map((color) =>
      String(color).replace("#", ""),
    );
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
    for (const [name, theme] of Object.entries<any>(themes)) {
      expect(
        contrast(theme.ui.foregroundFaint, theme.color.paper),
        `${name} --fg-faint on --bg`,
      ).toBeGreaterThanOrEqual(4.5);
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

    for (const [name, theme] of Object.entries<any>(themes)) {
      const gutter = theme.color.paper;
      const editor = theme.ui.edit;
      const accent = theme.color.accent;
      const foreground = theme.color.ink;
      const gutterForeground = theme.color.inkFaint;
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
    for (const theme of Object.values<any>(themes)) {
      expect(theme.ui.focusRing || theme.color.accent2).toBeTruthy();
    }
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
    // The progressive analysis strip and bounded minimap add positioned
    // surfaces while reusing the existing semantic layers.
    expect(layers).toHaveLength(18);
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
