import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { readCssSource } from "./css-source.js";

import {
  ANALYSIS_COLOR_TOKENS,
  analysisProfileForPath,
  analysisRanges,
  compileAnalysisMatchers,
  defaultAnalysisProfile,
  normalizeAnalysisProfiles,
} from "../src/analysis-model.js";

const webRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const html = readFileSync(path.join(webRoot, "index.html"), "utf8");
const css = readCssSource();

describe("progressive multi-rule log analysis (#242)", () => {
  it("ships a useful three-rule literal/regex profile", () => {
    const profile = defaultAnalysisProfile();
    expect(profile.rules.length).toBeGreaterThanOrEqual(3);
    expect(profile.rules.some((rule) => rule.regex)).toBe(true);
    expect(profile.rules.some((rule) => !rule.regex)).toBe(true);
    expect(new Set(profile.rules.map((rule) => rule.color)).size).toBeGreaterThanOrEqual(3);
  });

  it("uses first-rule background priority and marks overlaps explicitly", () => {
    const profile = defaultAnalysisProfile();
    profile.rules = [
      { ...profile.rules[0], id: "broad", pattern: "ERROR", color: "danger" },
      {
        ...profile.rules[1],
        id: "narrow",
        pattern: "ERR",
        color: "warn",
        whole_word: false,
      },
    ];
    const ranges = analysisRanges("ERROR", compileAnalysisMatchers(profile));
    expect(ranges).toEqual([
      {
        start: 0,
        end: 3,
        color: "danger",
        overlap: true,
        ruleIds: ["broad", "narrow"],
      },
      {
        start: 3,
        end: 5,
        color: "danger",
        overlap: false,
        ruleIds: ["broad"],
      },
    ]);
  });

  it("filters visible rules without changing the saved profile", () => {
    const profile = defaultAnalysisProfile();
    const matchers = compileAnalysisMatchers(profile);
    const visible = new Set(["warning"]);
    const ranges = analysisRanges("ERROR WARN", matchers, visible);
    expect(ranges).toHaveLength(1);
    expect(ranges[0].ruleIds).toEqual(["warning"]);
  });

  it("normalizes profile bounds and accepts semantic colors only", () => {
    const source = defaultAnalysisProfile();
    source.rules = Array.from({ length: 20 }, (_, index) => ({
      ...source.rules[0],
      id: `rule-${index}`,
      color: index === 0 ? "#f00" : ANALYSIS_COLOR_TOKENS[index % ANALYSIS_COLOR_TOKENS.length],
    }));
    const [profile] = normalizeAnalysisProfiles([source]);
    expect(profile.rules).toHaveLength(12);
    expect(profile.rules[0].color).toBe("accent");
    expect(profile.rules.every((rule) => ANALYSIS_COLOR_TOKENS.includes(rule.color))).toBe(true);

    const profiles = normalizeAnalysisProfiles(
      Array.from({ length: 40 }, (_, index) => ({
        ...defaultAnalysisProfile(),
        id: `profile-${index}`,
      })),
    );
    expect(profiles).toHaveLength(32);
  });

  it("checks whole-word boundaries by Unicode code point", () => {
    const profile = defaultAnalysisProfile();
    profile.rules = [{ ...profile.rules[0], pattern: "A", whole_word: true }];
    const matchers = compileAnalysisMatchers(profile);
    expect(analysisRanges("𐐀A", matchers)).toEqual([]);
    expect(analysisRanges("😀A", matchers)).toHaveLength(1);
  });

  it("selects profiles by basename or full-path glob", () => {
    const profile = defaultAnalysisProfile();
    expect(analysisProfileForPath([profile], "/var/log/system.log")?.id).toBe(profile.id);
    expect(analysisProfileForPath([profile], "/var/log/system.txt")).toBeUndefined();
  });

  it("exposes keyboard/a11y entry points and tokenized component styling", () => {
    expect(html).toContain('data-menu-action="analysisRules"');
    expect(html).toContain('id="analysis-modal" class="modal hidden" aria-hidden="true"');
    expect(html).toContain('id="analysis-progress"');
    expect(html).toContain('role="status" aria-live="polite"');
    expect(css).toContain("--analysis-panel-w:");
    expect(css).toContain('[data-analysis-color="danger"]');

    const componentCss = css.slice(css.indexOf("progressive multi-rule log analysis"));
    for (const match of componentCss.matchAll(/font-size\s*:\s*([^;]+);/g)) {
      expect(match[1].trim()).toMatch(/^var\(--fs-/);
    }
    for (const match of componentCss.matchAll(/(?:padding|margin|gap)\s*:\s*([^;]+);/g)) {
      expect(match[1], match[0]).not.toMatch(/\d+(?:px|rem|em)/);
    }
  });
});
