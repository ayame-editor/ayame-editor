import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { FONT_STACKS } from "../src/state.ts";

const webRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const css = readFileSync(path.join(webRoot, "style.css"), "utf8");

const compactStack = (value: string) => value.replace(/[\s"]/g, "");

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
    }
  });

  it("uses semantic tokens for syntax and diff colors", () => {
    expect(css).toContain("--add-bg:");
    expect(css).toContain("--word-add:");
    expect(css).toContain("--word-del:");
    expect(css).toContain(".diff-cell.old .diff-word.changed { background: var(--word-del); }");
    expect(css).toContain(".diff-cell.new .diff-word.changed { background: var(--word-add); }");
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
});
