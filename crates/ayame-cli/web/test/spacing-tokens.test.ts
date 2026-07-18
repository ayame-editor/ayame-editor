import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const webRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const css = readFileSync(path.join(webRoot, "style.css"), "utf8");

const spacingDeclarations = [
  ...css.matchAll(
    /(?<![-\w])((?:padding|margin)(?:-[a-z]+)*|gap|row-gap|column-gap)\s*:\s*([^;]+);/g,
  ),
];

describe("Ayame spacing tokens (#149)", () => {
  it("defines a 4px scale with compact 2px half-steps", () => {
    for (const declaration of [
      "--space-0: 0",
      "--space-0-5: 2px",
      "--space-1: 4px",
      "--space-1-5: 6px",
      "--space-2: 8px",
      "--space-2-5: 10px",
      "--space-3: 12px",
      "--space-3-5: 14px",
      "--space-4: 16px",
      "--space-5: 20px",
      "--space-5-5: 22px",
      "--space-6: 24px",
      "--space-7: 28px",
      "--space-8: 32px",
    ]) {
      expect(css).toContain(declaration);
    }
    expect(css).toContain("--gutter-pad-start: var(--space-2)");
    expect(css).toContain("--gutter-pad-end: var(--space-5)");
    expect(css).toMatch(
      /--modal-body-padding:\s*var\(--space-3\)\s+var\(--space-3-5\)\s+var\(--space-3-5\)/,
    );
  });

  it("routes every component padding, margin, and gap through tokens", () => {
    expect(spacingDeclarations.length).toBeGreaterThan(70);

    for (const [, property, value] of spacingDeclarations) {
      expect(value, `${property}: ${value}`).not.toMatch(/-?(?:\d*\.)?\d+(?:px|rem|em|%)/);
      expect(value, `${property}: ${value}`).not.toMatch(/(?:^|\s)0(?:\s|$)/);
      expect(value, `${property}: ${value}`).toMatch(/var\(--(?:space|gutter|modal)-|^auto$/);
    }
  });

  it("keeps the 1px optical exception documented and isolated from layout", () => {
    expect(css).toContain(
      "/* Optical-only nudge for inline glyph ink; never use this for layout. */",
    );
    expect(css.match(/var\(--space-optical\)/g)).toHaveLength(2);
    expect(css).toMatch(/mark\s*\{[^}]*Optical exception:[^}]*var\(--space-optical\)/s);
    expect(css).toMatch(
      /\.opener-cwd::before\s*\{[^}]*Optical exception:[^}]*var\(--space-optical\)/s,
    );
  });
});
