import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { FONT_STACKS } from "../src/state.ts";
import { resolveTheme } from "../src/settings.ts";

const webRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const css = readFileSync(path.join(webRoot, "style.css"), "utf8");

const compactStack = (value: string) => value.replace(/[\s"]/g, "");

const luminance = (hex: string) => {
  const channels = [1, 3, 5]
    .map((index) => Number.parseInt(hex.slice(index, index + 2), 16) / 255)
    .map((value) => (value <= 0.03928 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4));
  return 0.2126 * channels[0] + 0.7152 * channels[1] + 0.0722 * channels[2];
};

const contrast = (a: string, b: string) => {
  const [lighter, darker] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (lighter + 0.05) / (darker + 0.05);
};

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
      "--fs-ui: 13px",
      "--fs-ui-sm: 12px",
      "--fs-hint: 11px",
      "--fs-ui-lg: 14px",
      "--fs-title: 16px",
      "--fs-ruler: 10px",
    ]) {
      expect(css).toContain(declaration);
    }
    expect(css.match(/(?<!-)font-size:\s*[\d.]+px/g)).toBeNull();
  });

  it("completes dark theme chrome tokens", () => {
    for (const theme of ["dark", "black"]) {
      const block = css.match(new RegExp(`html\\[data-theme="${theme}"\\]\\s*\\{([^}]+)\\}`, "s"))?.[1];
      expect(block).toBeTruthy();
      expect(block).toContain("--accent: #9b82d8");
      expect(block).toContain("--accent-bright: #b49de6");
      expect(block).toContain("--status:");
      expect(block).toContain("--status-fg:");
      expect(block).toContain("--add-bg:");
      expect(block).toContain("--word-add:");
      expect(block).toContain("--word-del:");
    }
  });

  it("follows system color preference and keeps Mono Paper monochrome", () => {
    expect(resolveTheme("system", false)).toBe("iris-light");
    expect(resolveTheme("system", true)).toBe("dark");
    const mono = css.match(/html\[data-theme="mono-paper"\]\s*\{([^}]+)\}/s)?.[1];
    expect(mono).toBeTruthy();
    const syntax = [...mono!.matchAll(/--syn-[\w-]+:\s*([^;]+);/g)].map((match) => match[1].trim());
    expect(new Set(syntax)).toEqual(new Set(["#56534D"]));
  });

  it("uses semantic tokens for syntax and change colors", () => {
    expect(css).toContain("--add-bg:");
    expect(css).toContain("--word-add:");
    expect(css).toContain("--word-del:");
    expect(css).not.toMatch(/\.syn-(?:string|number|literal|function|link)[^{]*\{[^}]*#[\da-f]{3,8}/is);
  });

  it("routes component corners, shadows, and foregrounds through tokens", () => {
    expect(css).toContain("--radius-control: 6px");
    expect(css).toContain("--shadow:");
    expect(css).toContain("--on-accent:");
    expect(css).not.toMatch(/border-radius:\s*(?:[1-9]|1[02])px/);
    expect(css).not.toContain("color: #fff");
    expect(css).not.toMatch(/font-weight:\s*(?:560|650)/);
  });

  it("keeps faint hint text above WCAG AA in every built-in theme", () => {
    for (const [foreground, background] of [
      ["#756A88", "#FFFFFF"],
      ["#65748C", "#FFFFFF"],
      ["#806878", "#FFFFFF"],
      ["#716E78", "#FFFFFF"],
      ["#716E66", "#FBFAF5"],
      ["#9A9A9A", "#1B1B1B"],
      ["#9A9A9A", "#0A0A0A"],
    ]) {
      expect(contrast(foreground, background), `${foreground} on ${background}`).toBeGreaterThanOrEqual(
        4.5,
      );
    }
  });
});
